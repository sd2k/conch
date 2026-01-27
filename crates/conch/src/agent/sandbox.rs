//! Agent sandbox for executing commands in an agent context.
//!
//! [`AgentSandbox`] wraps the [`Shell`] API and provides an agent-aware
//! execution environment with automatic VFS setup, tool registration,
//! and workspace mounts.

use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use eryx_vfs::{DirEntry, DirPerms, FilePerms, InMemoryStorage, Metadata, VfsResult, VfsStorage};

use super::continuation::{
    ExecutionOutcome, TOOL_REQUEST_EXIT_CODE, ToolResult, write_tool_result,
};
use super::history::HistoryProvider;
use super::tools::ToolDefinition;
use super::vfs::AgentVfs;
use crate::limits::ResourceLimits;
use crate::policy::{PolicyHandler, PolicyStorage};
use crate::runtime::{ExecutionResult, RuntimeError};
use crate::shell::{Mount, Shell};

/// A sized wrapper around `Arc<dyn VfsStorage>` that implements `VfsStorage`.
///
/// This allows us to use dynamic dispatch for VFS storage while still being
/// compatible with APIs that require `Sized` types.
#[derive(Clone)]
struct DynStorage(Arc<dyn VfsStorage>);

#[async_trait]
impl VfsStorage for DynStorage {
    async fn read(&self, path: &str) -> VfsResult<Vec<u8>> {
        self.0.read(path).await
    }

    async fn read_at(&self, path: &str, offset: u64, len: u64) -> VfsResult<Vec<u8>> {
        self.0.read_at(path, offset, len).await
    }

    async fn write(&self, path: &str, data: &[u8]) -> VfsResult<()> {
        self.0.write(path, data).await
    }

    async fn write_at(&self, path: &str, offset: u64, data: &[u8]) -> VfsResult<()> {
        self.0.write_at(path, offset, data).await
    }

    async fn set_size(&self, path: &str, size: u64) -> VfsResult<()> {
        self.0.set_size(path, size).await
    }

    async fn delete(&self, path: &str) -> VfsResult<()> {
        self.0.delete(path).await
    }

    async fn exists(&self, path: &str) -> VfsResult<bool> {
        self.0.exists(path).await
    }

    async fn list(&self, path: &str) -> VfsResult<Vec<DirEntry>> {
        self.0.list(path).await
    }

    async fn stat(&self, path: &str) -> VfsResult<Metadata> {
        self.0.stat(path).await
    }

    async fn mkdir(&self, path: &str) -> VfsResult<()> {
        self.0.mkdir(path).await
    }

    async fn rmdir(&self, path: &str) -> VfsResult<()> {
        self.0.rmdir(path).await
    }

    async fn rename(&self, from: &str, to: &str) -> VfsResult<()> {
        self.0.rename(from, to).await
    }

    fn mkdir_sync(&self, path: &str) -> VfsResult<()> {
        self.0.mkdir_sync(path)
    }
}

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
pub struct AgentSandboxBuilder {
    agent_id: String,
    name: Option<String>,
    parent_id: Option<String>,
    capabilities: Vec<String>,
    params: Option<serde_json::Value>,
    tools: Vec<ToolDefinition>,
    workspace_mounts: Vec<WorkspaceMount>,
    history: Option<Arc<dyn HistoryProvider>>,
    policy: Option<Arc<dyn PolicyHandler>>,
}

impl std::fmt::Debug for AgentSandboxBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentSandboxBuilder")
            .field("agent_id", &self.agent_id)
            .field("name", &self.name)
            .field("parent_id", &self.parent_id)
            .field("capabilities", &self.capabilities)
            .field("params", &self.params)
            .field("tools", &self.tools)
            .field("workspace_mounts", &self.workspace_mounts)
            .field("has_history", &self.history.is_some())
            .field("has_policy", &self.policy.is_some())
            .finish()
    }
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
            history: None,
            policy: None,
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

    /// Set the history provider for conversation context.
    ///
    /// The history provider supplies conversation transcripts accessible
    /// via `/history/` in the VFS.
    pub fn history(mut self, provider: Arc<dyn HistoryProvider>) -> Self {
        self.history = Some(provider);
        self
    }

    /// Set a policy handler for filesystem access control.
    ///
    /// The policy enforces security boundaries on what the agent can read/write,
    /// regardless of the script being executed. This provides defense-in-depth:
    /// even if a script is malicious, it can only access what the policy allows.
    ///
    /// If no policy is set, all operations are allowed (backward compatible).
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use conch::policy::{PolicyBuilder, agent_sandbox_policy};
    ///
    /// // Use the standard agent sandbox policy
    /// let sandbox = AgentSandbox::builder("agent-123")
    ///     .policy(agent_sandbox_policy())
    ///     .build()
    ///     .await?;
    ///
    /// // Or create a custom policy
    /// let policy = PolicyBuilder::new()
    ///     .allow_read("/agent/**")
    ///     .allow_read("/tools/**")
    ///     .allow_write("/agent/scratch/**")
    ///     .build();
    ///
    /// let sandbox = AgentSandbox::builder("agent-123")
    ///     .policy(policy)
    ///     .build()
    ///     .await?;
    /// ```
    pub fn policy(mut self, policy: impl PolicyHandler + 'static) -> Self {
        self.policy = Some(Arc::new(policy));
        self
    }

    /// Set a policy handler from an Arc.
    ///
    /// Use this when you want to share a policy across multiple sandboxes.
    pub fn policy_arc(mut self, policy: Arc<dyn PolicyHandler>) -> Self {
        self.policy = Some(policy);
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
        // Convert to Arc<dyn VfsStorage> for dynamic dispatch
        let storage: Arc<dyn VfsStorage> = Arc::new(storage);

        // Build the AgentVfs with all configuration FIRST (before applying policy)
        // This allows the initialization to write metadata, tools, etc.
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

        if let Some(history) = self.history {
            vfs_builder = vfs_builder.history(history);
        }

        let agent_vfs = vfs_builder
            .build(DynStorage(Arc::clone(&storage)))
            .await
            .map_err(|e| RuntimeError::Vfs(e.to_string()))?;

        let agent_vfs = Arc::new(agent_vfs);

        // Now wrap with policy if configured
        // The policy applies to the storage that will be used for shell execution,
        // not for the initial VFS setup which has already completed.
        let shell_storage: Arc<dyn VfsStorage> = match self.policy {
            Some(policy) => {
                // PolicyStorage wraps the storage with policy enforcement
                Arc::new(PolicyStorage::new(storage, policy))
            }
            None => {
                // No policy - use storage directly
                storage
            }
        };

        // Build the Shell with the (optionally policy-wrapped) storage
        let mut shell_builder = Shell::builder()
            .vfs_arc(shell_storage)
            // Mount all agent-related paths
            .vfs_path("/agent", DirPerms::all(), FilePerms::all())
            // /tools is mostly read-only, but /tools/pending needs write access for tool requests
            .vfs_path("/tools", DirPerms::all(), FilePerms::all())
            // /history is read-only for agents
            .vfs_path("/history", DirPerms::READ, FilePerms::READ);

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
    /// - `/tools/` - Tool definitions
    /// - `/history/` - Conversation history (if configured)
    /// - Workspace mounts configured via `mount()`
    ///
    /// If a policy was configured via [`AgentSandboxBuilder::policy`], it enforces
    /// access control on all filesystem operations. The default `agent_sandbox_policy()`
    /// allows reads to `/agent/**`, `/tools/**`, `/history/**` and writes only to
    /// `/agent/scratch/**`. Operations that violate the policy will fail with a
    /// permission error.
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

    /// Execute a shell script, handling tool invocation requests.
    ///
    /// If the script invokes a tool via the `tool` builtin, this method returns
    /// [`ExecutionOutcome::ToolRequest`] with the tool details. The orchestrator
    /// should execute the tool and call [`write_tool_result`] to record the result.
    ///
    /// Like [`execute`](Self::execute), filesystem access is controlled by any
    /// policy configured via [`AgentSandboxBuilder::policy`].
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let outcome = sandbox.execute_with_tools("tool web_search --query test", &limits).await?;
    /// match outcome {
    ///     ExecutionOutcome::Completed(result) => {
    ///         println!("Script completed with exit code {}", result.exit_code);
    ///     }
    ///     ExecutionOutcome::ToolRequest(request) => {
    ///         println!("Tool {} requested with params {:?}", request.tool, request.params);
    ///         // Execute tool externally, then write result:
    ///         // sandbox.write_tool_result(&request.call_id, ToolResult::success(value)).await?;
    ///     }
    /// }
    /// ```
    pub async fn execute_with_tools(
        &self,
        script: &str,
        limits: &ResourceLimits,
    ) -> Result<ExecutionOutcome, RuntimeError> {
        use super::continuation::ToolRequest;

        let result = self.shell.execute(script, limits).await?;

        // Check if the script yielded for a tool request
        if result.exit_code == TOOL_REQUEST_EXIT_CODE {
            // Parse stdout as tool request JSON
            // The tool builtin outputs: {"tool": "name", "params": {...}, "stdin": "..."}
            let stdout = String::from_utf8_lossy(&result.stdout);

            #[derive(serde::Deserialize)]
            struct RawToolRequest {
                tool: String,
                params: serde_json::Value,
                #[serde(default)]
                stdin: Option<String>,
                #[serde(default)]
                stdin_bytes: Option<usize>,
            }

            let raw: RawToolRequest = serde_json::from_str(stdout.trim()).map_err(|e| {
                RuntimeError::Vfs(format!("Failed to parse tool request from stdout: {e}"))
            })?;

            // Generate a call_id
            let call_id = self.next_call_id();

            // Create the full request
            let request = ToolRequest {
                call_id: call_id.clone(),
                tool: raw.tool,
                params: raw.params,
                stdin: raw.stdin,
                stdin_bytes: raw.stdin_bytes,
            };

            // Write to pending directory
            let pending_dir = format!("/tools/pending/{call_id}");
            self.vfs
                .mkdir(&pending_dir)
                .await
                .map_err(|e| RuntimeError::Vfs(e.to_string()))?;

            let request_json = serde_json::to_string_pretty(&request)
                .map_err(|e| RuntimeError::Vfs(format!("Failed to serialize request: {e}")))?;
            self.vfs
                .write(
                    &format!("{pending_dir}/request.json"),
                    request_json.as_bytes(),
                )
                .await
                .map_err(|e| RuntimeError::Vfs(e.to_string()))?;

            return Ok(ExecutionOutcome::ToolRequest(request));
        }

        Ok(ExecutionOutcome::Completed(result))
    }

    /// Write a tool result to the VFS.
    ///
    /// After a tool request is fulfilled by the orchestrator, call this method
    /// to record the result. The result is written to:
    /// - `/tools/history/<call_id>/response.json` or `/tools/history/<call_id>/error.json`
    /// - `/tools/last_result.json` (for easy script access)
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let outcome = sandbox.execute_with_tools("tool web_search --query test", &limits).await?;
    /// if let ExecutionOutcome::ToolRequest(request) = outcome {
    ///     // Execute tool externally...
    ///     let result = ToolResult::success(serde_json::json!({"results": [...]}));
    ///     sandbox.write_tool_result(&request.call_id, result).await?;
    /// }
    /// ```
    pub async fn write_tool_result(
        &self,
        call_id: &str,
        result: ToolResult,
    ) -> Result<(), RuntimeError> {
        write_tool_result(self.vfs.storage(), call_id, result).await
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

    #[tokio::test]
    async fn test_tool_invocation_yields() {
        let sandbox = AgentSandbox::builder("agent-123")
            .tool(ToolDefinition::no_params("web_search", "Search the web"))
            .build()
            .await
            .expect("Failed to build sandbox");

        // Invoke a tool - should yield with ToolRequest
        let outcome = sandbox
            .execute_with_tools("tool web_search --query 'rust async'", &default_limits())
            .await
            .expect("Execute failed");

        match outcome {
            ExecutionOutcome::ToolRequest(request) => {
                assert_eq!(request.tool, "web_search");
                assert_eq!(request.params["query"], "rust async");
                assert_eq!(request.call_id, "call-001");
            }
            ExecutionOutcome::Completed(result) => {
                panic!(
                    "Expected ToolRequest, got Completed with exit code {}",
                    result.exit_code
                );
            }
        }
    }

    #[tokio::test]
    async fn test_tool_request_written_to_pending() {
        let sandbox = AgentSandbox::builder("agent-123")
            .tool(ToolDefinition::no_params("analyze", "Analyze data"))
            .build()
            .await
            .expect("Failed to build sandbox");

        // Invoke a tool
        let outcome = sandbox
            .execute_with_tools("tool analyze --format json", &default_limits())
            .await
            .expect("Execute failed");

        let request = match outcome {
            ExecutionOutcome::ToolRequest(r) => r,
            _ => panic!("Expected ToolRequest"),
        };

        // Verify request was written to pending directory
        let pending_path = format!("/tools/pending/{}/request.json", request.call_id);
        let pending_data = sandbox
            .vfs()
            .read(&pending_path)
            .await
            .expect("Should be able to read pending request");

        let pending_json: serde_json::Value =
            serde_json::from_slice(&pending_data).expect("Should parse as JSON");
        assert_eq!(pending_json["tool"], "analyze");
        assert_eq!(pending_json["params"]["format"], "json");
    }

    #[tokio::test]
    async fn test_write_tool_result_success() {
        let sandbox = AgentSandbox::builder("agent-123")
            .tool(ToolDefinition::no_params("test_tool", "A test tool"))
            .build()
            .await
            .expect("Failed to build sandbox");

        // Invoke a tool
        let outcome = sandbox
            .execute_with_tools("tool test_tool --arg value", &default_limits())
            .await
            .expect("Execute failed");

        let request = match outcome {
            ExecutionOutcome::ToolRequest(r) => r,
            _ => panic!("Expected ToolRequest"),
        };

        // Write successful result
        let result_value = serde_json::json!({"status": "success", "data": [1, 2, 3]});
        sandbox
            .write_tool_result(&request.call_id, ToolResult::success(result_value.clone()))
            .await
            .expect("write_tool_result failed");

        // Verify result was written to history
        let history_path = format!("/tools/history/{}/response.json", request.call_id);
        let history_data = sandbox
            .vfs()
            .read(&history_path)
            .await
            .expect("Should be able to read history response");

        let history_json: serde_json::Value =
            serde_json::from_slice(&history_data).expect("Should parse as JSON");
        assert_eq!(history_json, result_value);

        // Verify last_result.json was updated
        let last_result = sandbox
            .vfs()
            .read("/tools/last_result.json")
            .await
            .expect("Should be able to read last_result");
        let last_json: serde_json::Value =
            serde_json::from_slice(&last_result).expect("Should parse as JSON");
        assert_eq!(last_json, result_value);

        // Verify pending was cleaned up
        let pending_path = format!("/tools/pending/{}/request.json", request.call_id);
        assert!(
            sandbox.vfs().read(&pending_path).await.is_err(),
            "Pending request should be deleted"
        );
    }

    #[tokio::test]
    async fn test_write_tool_result_error() {
        let sandbox = AgentSandbox::builder("agent-123")
            .tool(ToolDefinition::no_params(
                "failing_tool",
                "A tool that fails",
            ))
            .build()
            .await
            .expect("Failed to build sandbox");

        // Invoke a tool
        let outcome = sandbox
            .execute_with_tools("tool failing_tool", &default_limits())
            .await
            .expect("Execute failed");

        let request = match outcome {
            ExecutionOutcome::ToolRequest(r) => r,
            _ => panic!("Expected ToolRequest"),
        };

        // Write error result
        sandbox
            .write_tool_result(&request.call_id, ToolResult::error("Tool execution failed"))
            .await
            .expect("write_tool_result failed");

        // Verify error was written to history
        let error_path = format!("/tools/history/{}/error.json", request.call_id);
        let error_data = sandbox
            .vfs()
            .read(&error_path)
            .await
            .expect("Should be able to read history error");

        let error_json: serde_json::Value =
            serde_json::from_slice(&error_data).expect("Should parse as JSON");
        assert_eq!(error_json["error"], "Tool execution failed");

        // Verify metadata shows failure
        let metadata_path = format!("/tools/history/{}/metadata.json", request.call_id);
        let metadata_data = sandbox
            .vfs()
            .read(&metadata_path)
            .await
            .expect("Should be able to read metadata");
        let metadata_json: serde_json::Value =
            serde_json::from_slice(&metadata_data).expect("Should parse as JSON");
        assert_eq!(metadata_json["success"], false);
    }

    #[tokio::test]
    async fn test_full_tool_cycle() {
        let sandbox = AgentSandbox::builder("agent-123")
            .tool(ToolDefinition::no_params(
                "calculator",
                "Perform calculations",
            ))
            .build()
            .await
            .expect("Failed to build sandbox");

        // Step 1: Agent invokes tool
        let outcome = sandbox
            .execute_with_tools(
                "tool calculator --operation add --a 5 --b 3",
                &default_limits(),
            )
            .await
            .expect("Execute failed");

        let request = match outcome {
            ExecutionOutcome::ToolRequest(r) => r,
            _ => panic!("Expected ToolRequest"),
        };

        assert_eq!(request.tool, "calculator");
        assert_eq!(request.params["operation"], "add");
        assert_eq!(request.params["a"], 5);
        assert_eq!(request.params["b"], 3);

        // Step 2: Orchestrator executes tool (simulated)
        let tool_result = serde_json::json!({"result": 8});

        // Step 3: Orchestrator writes result
        sandbox
            .write_tool_result(&request.call_id, ToolResult::success(tool_result))
            .await
            .expect("write_tool_result failed");

        // Step 4: Agent can read result (in a new script execution)
        let result = sandbox
            .execute("cat /tools/last_result.json", &default_limits())
            .await
            .expect("Execute failed");

        assert_eq!(result.exit_code, 0);
        let stdout = String::from_utf8_lossy(&result.stdout);
        assert!(stdout.contains("\"result\": 8"), "stdout: {}", stdout);
    }

    #[tokio::test]
    async fn test_normal_execution_completes() {
        let sandbox = AgentSandbox::builder("agent-123")
            .build()
            .await
            .expect("Failed to build sandbox");

        // Execute a normal command (no tool invocation)
        let outcome = sandbox
            .execute_with_tools("echo 'hello world'", &default_limits())
            .await
            .expect("Execute failed");

        match outcome {
            ExecutionOutcome::Completed(result) => {
                assert_eq!(result.exit_code, 0);
                assert!(String::from_utf8_lossy(&result.stdout).contains("hello world"));
            }
            ExecutionOutcome::ToolRequest(_) => {
                panic!("Expected Completed, got ToolRequest");
            }
        }
    }

    #[tokio::test]
    async fn test_tool_with_json_params() {
        let sandbox = AgentSandbox::builder("agent-123")
            .tool(ToolDefinition::no_params("code_edit", "Edit code"))
            .build()
            .await
            .expect("Failed to build sandbox");

        // Invoke tool with --json parameter
        let outcome = sandbox
            .execute_with_tools(
                r#"tool code_edit --json '{"file": "src/main.rs", "changes": [{"line": 10, "text": "// fixed"}]}'"#,
                &default_limits(),
            )
            .await
            .expect("Execute failed");

        match outcome {
            ExecutionOutcome::ToolRequest(request) => {
                assert_eq!(request.tool, "code_edit");
                assert_eq!(request.params["file"], "src/main.rs");
                assert!(request.params["changes"].is_array());
                assert_eq!(request.params["changes"][0]["line"], 10);
            }
            _ => panic!("Expected ToolRequest"),
        }
    }

    #[tokio::test]
    async fn test_sandbox_with_history() {
        use crate::agent::history::{ConversationMetadata, SimpleHistoryProvider};

        let history = SimpleHistoryProvider::new()
            .with_transcript("U> Hello, can you help me?\n\nA> Of course! What do you need?")
            .with_metadata(ConversationMetadata {
                id: "current".to_string(),
                title: Some("Help request".to_string()),
                started_at: "2024-01-01T00:00:00Z".to_string(),
                updated_at: None,
                user_message_count: 1,
                assistant_message_count: 1,
                tool_call_count: 0,
            });

        let sandbox = AgentSandbox::builder("agent-123")
            .history(Arc::new(history))
            .build()
            .await
            .expect("Failed to build sandbox");

        // Read the transcript via shell command
        let result = sandbox
            .execute("cat /history/current/transcript.md", &default_limits())
            .await
            .expect("Execute failed");

        assert_eq!(result.exit_code, 0);
        let stdout = String::from_utf8_lossy(&result.stdout);
        assert!(stdout.contains("U>"), "Should contain user marker");
        assert!(stdout.contains("Hello"), "Should contain user message");
        assert!(stdout.contains("A>"), "Should contain assistant marker");
    }

    #[tokio::test]
    async fn test_sandbox_history_index() {
        use crate::agent::history::{
            ConversationMetadata, ConversationSummary, SimpleHistoryProvider,
        };

        let history = SimpleHistoryProvider::new()
            .with_transcript("U> Current message")
            .with_metadata(ConversationMetadata {
                id: "current".to_string(),
                title: Some("Current task".to_string()),
                started_at: "2024-01-02T00:00:00Z".to_string(),
                updated_at: None,
                user_message_count: 1,
                assistant_message_count: 0,
                tool_call_count: 0,
            })
            .with_conversation(
                ConversationSummary {
                    id: "old-001".to_string(),
                    title: "Previous task".to_string(),
                    started_at: "2024-01-01T00:00:00Z".to_string(),
                    message_count: 5,
                    is_current: false,
                },
                "U> Old message\n\nA> Old response",
                None,
            );

        let sandbox = AgentSandbox::builder("agent-123")
            .history(Arc::new(history))
            .build()
            .await
            .expect("Failed to build sandbox");

        // Read the index
        let result = sandbox
            .execute("cat /history/index.txt", &default_limits())
            .await
            .expect("Execute failed");

        assert_eq!(result.exit_code, 0);
        let stdout = String::from_utf8_lossy(&result.stdout);
        assert!(
            stdout.contains("current (current)"),
            "Should list current conversation"
        );
        assert!(
            stdout.contains("old-001"),
            "Should list historical conversation"
        );
    }

    #[tokio::test]
    async fn test_sandbox_history_grep() {
        use crate::agent::history::SimpleHistoryProvider;

        let transcript = r#"U> Can you search for information about Rust async patterns?

A> I'll search for that information.

T[web_search] {"query": "Rust async patterns"}
R> {"results": [{"title": "Async in Rust", "url": "https://example.com"}]}

A> Based on the search results, here's what I found about Rust async patterns..."#;

        let history = SimpleHistoryProvider::new().with_transcript(transcript);

        let sandbox = AgentSandbox::builder("agent-123")
            .history(Arc::new(history))
            .build()
            .await
            .expect("Failed to build sandbox");

        // Grep for tool calls in the transcript
        let result = sandbox
            .execute(
                "grep 'T\\[' /history/current/transcript.md",
                &default_limits(),
            )
            .await
            .expect("Execute failed");

        assert_eq!(result.exit_code, 0);
        let stdout = String::from_utf8_lossy(&result.stdout);
        assert!(stdout.contains("web_search"), "Should find tool call");
    }

    // Policy integration tests
    mod policy_tests {
        use super::*;
        use crate::policy::{PolicyBuilder, agent_sandbox_policy};

        #[tokio::test]
        async fn test_sandbox_with_policy_allows_agent_reads() {
            // Use the standard agent sandbox policy
            let sandbox = AgentSandbox::builder("agent-123")
                .policy(agent_sandbox_policy())
                .build()
                .await
                .expect("Failed to build sandbox");

            // Should be able to read /agent/metadata.json
            let result = sandbox
                .execute("cat /agent/metadata.json", &default_limits())
                .await
                .expect("Execute failed");

            assert_eq!(
                result.exit_code,
                0,
                "Should allow reading /agent/metadata.json. stderr: {}",
                String::from_utf8_lossy(&result.stderr)
            );
        }

        #[tokio::test]
        async fn test_sandbox_with_policy_allows_tools_reads() {
            let sandbox = AgentSandbox::builder("agent-123")
                .tool(ToolDefinition::no_params("test_tool", "A test tool"))
                .policy(agent_sandbox_policy())
                .build()
                .await
                .expect("Failed to build sandbox");

            // Should be able to read /tools/index.txt
            let result = sandbox
                .execute("cat /tools/index.txt", &default_limits())
                .await
                .expect("Execute failed");

            assert_eq!(
                result.exit_code,
                0,
                "Should allow reading /tools/index.txt. stderr: {}",
                String::from_utf8_lossy(&result.stderr)
            );
            assert!(String::from_utf8_lossy(&result.stdout).contains("test_tool"));
        }

        #[tokio::test]
        async fn test_sandbox_with_policy_allows_scratch_writes() {
            let sandbox = AgentSandbox::builder("agent-123")
                .policy(agent_sandbox_policy())
                .build()
                .await
                .expect("Failed to build sandbox");

            // Should be able to write to /agent/scratch/
            let result = sandbox
                .execute(
                    "echo 'test data' > /agent/scratch/output.txt",
                    &default_limits(),
                )
                .await
                .expect("Execute failed");

            assert_eq!(
                result.exit_code,
                0,
                "Should allow writing to /agent/scratch/. stderr: {}",
                String::from_utf8_lossy(&result.stderr)
            );

            // Verify the write worked
            let result = sandbox
                .execute("cat /agent/scratch/output.txt", &default_limits())
                .await
                .expect("Execute failed");
            assert!(String::from_utf8_lossy(&result.stdout).contains("test data"));
        }

        #[tokio::test]
        async fn test_sandbox_with_policy_denies_write_to_agent_root() {
            let sandbox = AgentSandbox::builder("agent-123")
                .policy(agent_sandbox_policy())
                .build()
                .await
                .expect("Failed to build sandbox");

            // Should NOT be able to write to /agent/metadata.json (outside scratch)
            let result = sandbox
                .execute("echo 'malicious' > /agent/metadata.json", &default_limits())
                .await
                .expect("Execute failed");

            // The command should fail (non-zero exit code) because write is denied
            assert_ne!(
                result.exit_code, 0,
                "Should deny writing to /agent/metadata.json"
            );
        }

        #[tokio::test]
        async fn test_sandbox_with_custom_policy() {
            // Create a custom policy that only allows reading /agent/params.json
            let policy = PolicyBuilder::new()
                .allow_read("/agent/params.json")
                .allow_read("/agent")
                .build();

            let sandbox = AgentSandbox::builder("agent-123")
                .params(serde_json::json!({"task": "test"}))
                .policy(policy)
                .build()
                .await
                .expect("Failed to build sandbox");

            // Should be able to read /agent/params.json
            let result = sandbox
                .execute("cat /agent/params.json", &default_limits())
                .await
                .expect("Execute failed");

            assert_eq!(
                result.exit_code, 0,
                "Should allow reading /agent/params.json"
            );

            // Should NOT be able to read /agent/metadata.json (not in policy)
            let result = sandbox
                .execute("cat /agent/metadata.json", &default_limits())
                .await
                .expect("Execute failed");

            assert_ne!(
                result.exit_code, 0,
                "Should deny reading /agent/metadata.json"
            );
        }

        #[tokio::test]
        async fn test_sandbox_without_policy_allows_everything() {
            // No policy = allow all (backward compatible)
            let sandbox = AgentSandbox::builder("agent-123")
                .build()
                .await
                .expect("Failed to build sandbox");

            // Should be able to write anywhere
            let result = sandbox
                .execute("echo 'data' > /agent/metadata.json", &default_limits())
                .await
                .expect("Execute failed");

            // Without policy, this should succeed
            assert_eq!(
                result.exit_code, 0,
                "Without policy, should allow all writes"
            );
        }

        #[tokio::test]
        async fn test_policy_shared_across_invocations() {
            let sandbox = AgentSandbox::builder("agent-123")
                .policy(agent_sandbox_policy())
                .build()
                .await
                .expect("Failed to build sandbox");

            // Multiple executions should all be governed by the same policy
            for i in 0..3 {
                let result = sandbox
                    .execute("cat /agent/params.json", &default_limits())
                    .await
                    .expect("Execute failed");

                assert_eq!(
                    result.exit_code, 0,
                    "Execution {} should succeed with policy",
                    i
                );
            }
        }
    }
}
