//! Agent sandbox for executing commands in an agent context.
//!
//! [`AgentSandbox`] wraps the [`Shell`] API and provides an agent-aware
//! execution environment with automatic VFS setup, tool registration,
//! and workspace mounts.

use std::path::Path;
use std::sync::Arc;

use eryx_vfs::{DirPerms, FilePerms, InMemoryStorage, VfsStorage};

use super::tools::ToolDefinition;
use super::vfs::AgentVfs;
use crate::limits::ResourceLimits;
use crate::runtime::{ExecutionResult, RuntimeError};
use crate::shell::{Mount, Shell};

/// Configuration for a workspace mount.
#[derive(Debug, Clone)]
struct WorkspaceMount {
    guest_path: String,
    host_path: std::path::PathBuf,
    mount: Mount,
}

/// Builder for creating an [`AgentSandbox`] with custom configuration.
///
/// # Example
///
/// ```rust,ignore
/// use conch::agent::{AgentSandbox, ToolDefinition};
/// use serde_json::json;
///
/// let sandbox = AgentSandbox::builder("agent-123")
///     .name("code-reviewer")
///     .parent("agent-root")
///     .params(json!({"task": "review PR #42"}))
///     .tool(ToolDefinition::new("web_search", "Search the web", json!({})))
///     .mount("/workspace", "/home/user/code", Mount::readonly())
///     .build()
///     .await?;
///
/// let result = sandbox.execute("cat /agent/params.json", &limits).await?;
/// ```
#[derive(Debug)]
pub struct AgentSandboxBuilder {
    agent_id: String,
    name: Option<String>,
    parent_id: Option<String>,
    capabilities: Vec<String>,
    params: Option<serde_json::Value>,
    tools: Vec<ToolDefinition>,
    workspace_mounts: Vec<WorkspaceMount>,
}

impl AgentSandboxBuilder {
    /// Create a new builder for an agent sandbox.
    pub fn new(agent_id: impl Into<String>) -> Self {
        Self {
            agent_id: agent_id.into(),
            name: None,
            parent_id: None,
            capabilities: Vec::new(),
            params: None,
            tools: Vec::new(),
            workspace_mounts: Vec::new(),
        }
    }

    /// Set a human-readable name for the agent.
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Set the parent agent ID (for sub-agents).
    pub fn parent(mut self, parent_id: impl Into<String>) -> Self {
        self.parent_id = Some(parent_id.into());
        self
    }

    /// Add a capability to the agent.
    pub fn capability(mut self, capability: impl Into<String>) -> Self {
        self.capabilities.push(capability.into());
        self
    }

    /// Add multiple capabilities to the agent.
    pub fn capabilities(
        mut self,
        capabilities: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        self.capabilities
            .extend(capabilities.into_iter().map(Into::into));
        self
    }

    /// Set the parameters passed when this agent was spawned.
    pub fn params(mut self, params: serde_json::Value) -> Self {
        self.params = Some(params);
        self
    }

    /// Add a tool to the agent's available tools.
    pub fn tool(mut self, tool: ToolDefinition) -> Self {
        self.tools.push(tool);
        self
    }

    /// Add multiple tools to the agent's available tools.
    pub fn tools(mut self, tools: impl IntoIterator<Item = ToolDefinition>) -> Self {
        self.tools.extend(tools);
        self
    }

    /// Add a workspace mount (real filesystem accessible to the agent).
    ///
    /// The `host_path` directory will be accessible at `guest_path` inside the sandbox.
    pub fn mount(
        mut self,
        guest_path: impl Into<String>,
        host_path: impl AsRef<Path>,
        mount: Mount,
    ) -> Self {
        self.workspace_mounts.push(WorkspaceMount {
            guest_path: guest_path.into(),
            host_path: host_path.as_ref().to_path_buf(),
            mount,
        });
        self
    }

    /// Build the sandbox with a custom VFS storage backend.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - VFS initialization fails
    /// - Shell creation fails
    /// - A workspace mount path doesn't exist
    pub async fn build_with_storage<S: VfsStorage + 'static>(
        self,
        storage: S,
    ) -> Result<AgentSandbox, RuntimeError> {
        // Build the AgentVfs with all configuration
        let mut vfs_builder = AgentVfs::builder(&self.agent_id);

        if let Some(name) = self.name {
            vfs_builder = vfs_builder.name(name);
        }
        if let Some(parent_id) = self.parent_id {
            vfs_builder = vfs_builder.parent(parent_id);
        }
        if let Some(params) = self.params {
            vfs_builder = vfs_builder.params(params);
        }

        vfs_builder = vfs_builder.capabilities(self.capabilities);
        vfs_builder = vfs_builder.tools(self.tools);

        let agent_vfs = vfs_builder
            .build(storage)
            .await
            .map_err(|e| RuntimeError::Vfs(e.to_string()))?;

        let agent_vfs = Arc::new(agent_vfs);

        // Build the Shell with the agent VFS
        let mut shell_builder = Shell::builder()
            .vfs_arc(agent_vfs.storage_arc())
            // Mount all agent-related paths
            .vfs_path("/agent", DirPerms::all(), FilePerms::all())
            .vfs_path("/tools", DirPerms::READ, FilePerms::READ);

        // Add workspace mounts
        for wm in self.workspace_mounts {
            shell_builder = shell_builder.mount(&wm.guest_path, &wm.host_path, wm.mount);
        }

        let shell = shell_builder.build()?;

        Ok(AgentSandbox {
            shell,
            agent_id: self.agent_id,
            vfs: agent_vfs,
        })
    }

    /// Build the sandbox with default in-memory storage.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Shell creation fails
    /// - A workspace mount path doesn't exist
    pub async fn build(self) -> Result<AgentSandbox, RuntimeError> {
        self.build_with_storage(InMemoryStorage::new()).await
    }
}

/// A sandboxed execution environment for an agent.
///
/// `AgentSandbox` provides:
/// - Isolated filesystem with agent metadata and tools
/// - Shell command execution with resource limits
/// - Access to workspace mounts
///
/// # Filesystem Layout
///
/// ```text
/// /agent/              - Agent metadata and scratch space (read-write)
/// /tools/              - Tool definitions (read-only)
/// /workspace/          - Custom mounts (configurable permissions)
/// ```
///
/// # Example
///
/// ```rust,ignore
/// let sandbox = AgentSandbox::builder("agent-123")
///     .params(json!({"task": "analyze code"}))
///     .build()
///     .await?;
///
/// // Read agent parameters
/// let result = sandbox.execute("cat /agent/params.json", &limits).await?;
///
/// // Write to scratch space
/// sandbox.execute("echo 'notes' > /agent/scratch/notes.txt", &limits).await?;
/// ```
pub struct AgentSandbox {
    shell: Shell,
    agent_id: String,
    vfs: Arc<AgentVfs>,
}

impl std::fmt::Debug for AgentSandbox {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentSandbox")
            .field("agent_id", &self.agent_id)
            .field("vfs", &self.vfs)
            .finish_non_exhaustive()
    }
}

impl AgentSandbox {
    /// Create a new builder for an agent sandbox.
    pub fn builder(agent_id: impl Into<String>) -> AgentSandboxBuilder {
        AgentSandboxBuilder::new(agent_id)
    }

    /// Get the agent ID.
    pub fn agent_id(&self) -> &str {
        &self.agent_id
    }

    /// Get access to the agent's VFS.
    ///
    /// This allows direct read/write to the agent's virtual filesystem.
    pub fn vfs(&self) -> &AgentVfs {
        &self.vfs
    }

    /// Execute a shell script in the sandbox.
    ///
    /// The script has access to:
    /// - `/agent/` - Agent metadata and scratch space
    /// - `/tools/` - Tool definitions (read-only)
    /// - Workspace mounts configured via `mount()`
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let result = sandbox.execute("cat /agent/metadata.json | jq .id", &limits).await?;
    /// assert_eq!(result.exit_code, 0);
    /// ```
    pub async fn execute(
        &self,
        script: &str,
        limits: &ResourceLimits,
    ) -> Result<ExecutionResult, RuntimeError> {
        self.shell.execute(script, limits).await
    }

    /// Generate the next tool call ID.
    ///
    /// Call IDs are sequential: `call-001`, `call-002`, etc.
    pub fn next_call_id(&self) -> String {
        self.vfs.next_call_id()
    }
}

#[cfg(test)]
#[cfg(feature = "embedded-shell")]
mod tests {
    use super::*;

    fn default_limits() -> ResourceLimits {
        ResourceLimits::default()
    }

    #[tokio::test]
    async fn test_sandbox_creation() {
        let sandbox = AgentSandbox::builder("agent-123")
            .build()
            .await
            .expect("Failed to build sandbox");

        assert_eq!(sandbox.agent_id(), "agent-123");
    }

    #[tokio::test]
    async fn test_sandbox_with_name_and_parent() {
        let sandbox = AgentSandbox::builder("agent-123")
            .name("test-agent")
            .parent("parent-456")
            .build()
            .await
            .expect("Failed to build sandbox");

        // Verify via VFS read
        let metadata = sandbox
            .vfs()
            .read("/agent/metadata.json")
            .await
            .expect("read metadata");
        let metadata_str = String::from_utf8_lossy(&metadata);
        assert!(metadata_str.contains("\"id\": \"agent-123\""));
        assert!(metadata_str.contains("\"name\": \"test-agent\""));
        assert!(metadata_str.contains("\"parent_id\": \"parent-456\""));
    }

    #[tokio::test]
    async fn test_sandbox_execute_basic() {
        let sandbox = AgentSandbox::builder("agent-123")
            .build()
            .await
            .expect("Failed to build sandbox");

        let result = sandbox
            .execute("echo hello", &default_limits())
            .await
            .expect("Execute failed");

        assert_eq!(result.exit_code, 0);
        assert!(String::from_utf8_lossy(&result.stdout).contains("hello"));
    }

    #[tokio::test]
    async fn test_sandbox_read_metadata() {
        let sandbox = AgentSandbox::builder("agent-123")
            .params(serde_json::json!({"task": "test task"}))
            .build()
            .await
            .expect("Failed to build sandbox");

        // Read params via shell command
        let result = sandbox
            .execute("cat /agent/params.json", &default_limits())
            .await
            .expect("Execute failed");

        assert_eq!(result.exit_code, 0);
        let stdout = String::from_utf8_lossy(&result.stdout);
        assert!(stdout.contains("test task"), "stdout: {}", stdout);
    }

    #[tokio::test]
    async fn test_sandbox_write_to_scratch() {
        let sandbox = AgentSandbox::builder("agent-123")
            .build()
            .await
            .expect("Failed to build sandbox");

        // Write to scratch
        let result = sandbox
            .execute(
                "echo 'hello world' > /agent/scratch/test.txt",
                &default_limits(),
            )
            .await
            .expect("Execute failed");
        assert_eq!(
            result.exit_code,
            0,
            "stderr: {}",
            String::from_utf8_lossy(&result.stderr)
        );

        // Read back
        let result = sandbox
            .execute("cat /agent/scratch/test.txt", &default_limits())
            .await
            .expect("Execute failed");
        assert_eq!(result.exit_code, 0);
        assert!(String::from_utf8_lossy(&result.stdout).contains("hello world"));
    }

    #[tokio::test]
    async fn test_sandbox_tools_readable() {
        let sandbox = AgentSandbox::builder("agent-123")
            .tool(ToolDefinition::new(
                "web_search",
                "Search the web",
                serde_json::json!({"type": "object"}),
            ))
            .build()
            .await
            .expect("Failed to build sandbox");

        // Read tool index
        let result = sandbox
            .execute("cat /tools/index.txt", &default_limits())
            .await
            .expect("Execute failed");
        assert_eq!(result.exit_code, 0);
        assert!(String::from_utf8_lossy(&result.stdout).contains("web_search"));

        // Read tool definition
        let result = sandbox
            .execute("cat /tools/available/web_search.json", &default_limits())
            .await
            .expect("Execute failed");
        assert_eq!(result.exit_code, 0);
        assert!(String::from_utf8_lossy(&result.stdout).contains("Search the web"));
    }

    #[tokio::test]
    async fn test_sandbox_call_id_generation() {
        let sandbox = AgentSandbox::builder("agent-123")
            .build()
            .await
            .expect("Failed to build sandbox");

        assert_eq!(sandbox.next_call_id(), "call-001");
        assert_eq!(sandbox.next_call_id(), "call-002");
        assert_eq!(sandbox.next_call_id(), "call-003");
    }

    #[tokio::test]
    async fn test_sandbox_grep_tools() {
        let sandbox = AgentSandbox::builder("agent-123")
            .tool(ToolDefinition::no_params(
                "web_search",
                "Search the web for information",
            ))
            .tool(ToolDefinition::no_params(
                "code_edit",
                "Edit source code files",
            ))
            .tool(ToolDefinition::no_params("file_read", "Read file contents"))
            .build()
            .await
            .expect("Failed to build sandbox");

        // Grep for tools containing "search" or "read"
        let result = sandbox
            .execute("grep -i search /tools/index.txt", &default_limits())
            .await
            .expect("Execute failed");
        assert_eq!(result.exit_code, 0);
        assert!(String::from_utf8_lossy(&result.stdout).contains("web_search"));

        // Grep should not match code_edit
        let result = sandbox
            .execute("grep -i search /tools/index.txt", &default_limits())
            .await
            .expect("Execute failed");
        assert!(!String::from_utf8_lossy(&result.stdout).contains("code_edit"));
    }
}
