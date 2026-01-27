//! Conch MCP Server
//!
//! An MCP server that exposes Conch sandboxed shell execution as a tool.
//! This allows AI agents to execute shell commands in a secure WASM sandbox.
//!
//! # Mount Configuration
//!
//! The server can be configured with filesystem mounts that allow the sandboxed
//! shell to access specific host directories:
//!
//! ```rust,ignore
//! use conch_mcp::{ConchServer, MountConfig};
//! use std::path::PathBuf;
//!
//! let mounts = vec![
//!     MountConfig {
//!         guest_path: "/data".to_string(),
//!         host_path: PathBuf::from("/home/user/data"),
//!         readonly: true,
//!     },
//! ];
//!
//! let server = ConchServer::new(4, mounts)?;
//! ```

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use conch::{Mount, ResourceLimits, Shell};
use rmcp::{
    ErrorData as McpError, ServerHandler,
    model::*,
    schemars::{self, JsonSchema},
    service::{RequestContext, RoleServer},
};
use serde::{Deserialize, Serialize};
use tokio::sync::Semaphore;

/// Configuration for a filesystem mount.
#[derive(Debug, Clone)]
pub struct MountConfig {
    /// Path visible inside the shell (e.g., "/data")
    pub guest_path: String,
    /// Real filesystem path to mount
    pub host_path: PathBuf,
    /// Whether the mount is read-only
    pub readonly: bool,
}

/// Parameters for the shell execution tool
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ExecuteParams {
    /// The shell command or script to execute.
    /// This will be interpreted by a bash-compatible shell.
    pub command: String,

    /// Maximum CPU time in milliseconds (default: 5000ms = 5 seconds)
    #[serde(default)]
    pub max_cpu_ms: Option<u64>,

    /// Maximum memory in bytes (default: 64MB)
    #[serde(default)]
    pub max_memory_bytes: Option<u64>,

    /// Wall-clock timeout in milliseconds (default: 30000ms = 30 seconds)
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

/// MCP Server that provides sandboxed shell execution via Conch
pub struct ConchServer {
    shell: Arc<Shell>,
    semaphore: Arc<Semaphore>,
    mount_descriptions: Vec<String>,
}

impl Clone for ConchServer {
    fn clone(&self) -> Self {
        Self {
            shell: Arc::clone(&self.shell),
            semaphore: Arc::clone(&self.semaphore),
            mount_descriptions: self.mount_descriptions.clone(),
        }
    }
}

impl std::fmt::Debug for ConchServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConchServer")
            .field("mounts", &self.mount_descriptions)
            .finish_non_exhaustive()
    }
}

impl ConchServer {
    /// Create a new ConchServer with the embedded WASM shell module.
    ///
    /// # Arguments
    /// * `max_concurrent` - Maximum number of concurrent shell executions
    /// * `mounts` - Filesystem directories to mount into the sandbox
    pub fn new(
        max_concurrent: usize,
        mounts: Vec<MountConfig>,
    ) -> Result<Self, conch::RuntimeError> {
        let mut builder = Shell::builder();

        // Collect mount descriptions for the tool description
        let mut mount_descriptions = Vec::new();

        // Add configured mounts
        for mount in &mounts {
            let perms = if mount.readonly {
                Mount::readonly()
            } else {
                Mount::readwrite()
            };
            builder = builder.mount(&mount.guest_path, &mount.host_path, perms);

            mount_descriptions.push(format!(
                "{} ({})",
                mount.guest_path,
                if mount.readonly {
                    "read-only"
                } else {
                    "read-write"
                }
            ));
        }

        let shell = builder.build()?;

        Ok(Self {
            shell: Arc::new(shell),
            semaphore: Arc::new(Semaphore::new(max_concurrent)),
            mount_descriptions,
        })
    }

    /// Execute a shell command in the Conch sandbox.
    async fn execute_command(&self, params: ExecuteParams) -> Result<CallToolResult, McpError> {
        // Acquire semaphore permit for concurrency limiting
        let _permit = self
            .semaphore
            .acquire()
            .await
            .map_err(|_| McpError::internal_error("Semaphore closed", None))?;

        let limits = ResourceLimits {
            max_cpu_ms: params.max_cpu_ms.unwrap_or(5000),
            max_memory_bytes: params.max_memory_bytes.unwrap_or(64 * 1024 * 1024),
            max_output_bytes: 1024 * 1024, // 1MB output limit
            timeout: Duration::from_millis(params.timeout_ms.unwrap_or(30000)),
        };

        let result = self
            .shell
            .execute(&params.command, &limits)
            .await
            .map_err(|e| McpError::internal_error(format!("Execution error: {}", e), None))?;

        // Format the output nicely for the model
        let stdout = String::from_utf8_lossy(&result.stdout);
        let stderr = String::from_utf8_lossy(&result.stderr);

        let mut output = String::new();

        if !stdout.is_empty() {
            output.push_str(&stdout);
        }

        if !stderr.is_empty() {
            if !output.is_empty() {
                output.push_str("\n--- stderr ---\n");
            }
            output.push_str(&stderr);
        }

        if output.is_empty() {
            output = format!("(no output, exit code: {})", result.exit_code);
        } else if result.exit_code != 0 {
            output.push_str(&format!("\n(exit code: {})", result.exit_code));
        }

        if result.truncated {
            output.push_str("\n[output truncated]");
        }

        Ok(CallToolResult::success(vec![Content::text(output)]))
    }

    fn execute_tool(&self) -> Tool {
        let schema = schemars::schema_for!(ExecuteParams);
        let schema_json = serde_json::to_value(schema).unwrap_or_default();
        let input_schema = match schema_json {
            serde_json::Value::Object(map) => Arc::new(map),
            _ => Arc::new(serde_json::Map::new()),
        };

        // Build description including mount info
        let mut description = String::from(
            "Execute a shell command in a secure WASM sandbox. Supports bash-compatible \
            syntax including pipes, redirects, and common utilities like cat, grep, head, \
            tail, jq, etc.",
        );

        if !self.mount_descriptions.is_empty() {
            description.push_str("\n\nAvailable filesystem mounts:\n");
            for mount_desc in &self.mount_descriptions {
                description.push_str(&format!("  - {}\n", mount_desc));
            }
        } else {
            description.push_str(
                " The sandbox has no filesystem access beyond the virtual /scratch directory.",
            );
        }

        Tool {
            name: "execute".into(),
            title: Some("Execute Shell Command".into()),
            description: Some(description.into()),
            input_schema,
            output_schema: None,
            annotations: None,
            icons: None,
            meta: None,
        }
    }
}

impl ServerHandler for ConchServer {
    fn get_info(&self) -> ServerInfo {
        let mut instructions = String::from(
            "Conch provides sandboxed shell execution in a WASM-based bash-compatible environment. \
            Use the 'execute' tool to run shell commands safely. The sandbox supports common utilities \
            like echo, cat, grep, head, tail, wc, sort, uniq, jq, and more. Commands run with strict \
            resource limits.",
        );

        if !self.mount_descriptions.is_empty() {
            instructions.push_str("\n\nFilesystem mounts available:\n");
            for mount_desc in &self.mount_descriptions {
                instructions.push_str(&format!("  - {}\n", mount_desc));
            }
        }

        ServerInfo {
            protocol_version: ProtocolVersion::LATEST,
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            server_info: Implementation::from_build_env(),
            instructions: Some(instructions.into()),
        }
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParam>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        Ok(ListToolsResult {
            tools: vec![self.execute_tool()],
            next_cursor: None,
            meta: None,
        })
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParam,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        match request.name.as_ref() {
            "execute" => {
                let params: ExecuteParams = match &request.arguments {
                    Some(args) => serde_json::from_value(serde_json::Value::Object(args.clone()))
                        .map_err(|e| {
                        McpError::invalid_params(format!("Invalid parameters: {}", e), None)
                    })?,
                    None => {
                        return Err(McpError::invalid_params(
                            "Missing 'command' parameter",
                            None,
                        ));
                    }
                };
                self.execute_command(params).await
            }
            _ => Err(McpError::invalid_params(
                format!("Unknown tool: {}", request.name),
                None,
            )),
        }
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn test_execute_params_defaults() {
        let json = r#"{"command": "echo hello"}"#;
        let params: ExecuteParams = serde_json::from_str(json).expect("parse failed");
        assert_eq!(params.command, "echo hello");
        assert!(params.max_cpu_ms.is_none());
        assert!(params.max_memory_bytes.is_none());
        assert!(params.timeout_ms.is_none());
    }

    #[test]
    fn test_execute_params_with_limits() {
        let json = r#"{"command": "echo hello", "max_cpu_ms": 1000, "timeout_ms": 5000}"#;
        let params: ExecuteParams = serde_json::from_str(json).expect("parse failed");
        assert_eq!(params.command, "echo hello");
        assert_eq!(params.max_cpu_ms, Some(1000));
        assert_eq!(params.timeout_ms, Some(5000));
    }

    #[test]
    fn test_mount_config() {
        let mount = MountConfig {
            guest_path: "/data".to_string(),
            host_path: PathBuf::from("/home/user/data"),
            readonly: true,
        };
        assert_eq!(mount.guest_path, "/data");
        assert!(mount.readonly);
    }
}
