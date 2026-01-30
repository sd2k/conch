//! Agent sandbox for executing commands in an agent context.
//!
//! [`AgentSandbox`] wraps the [`Shell`] API and provides an agent-aware
//! execution environment with automatic VFS setup, tool registration,
//! and workspace mounts.
//!
//! # Tool Invocation
//!
//! Tools are invoked via callbacks rather than the old exit-code-42 mechanism.
//! When a shell script runs `tool <name> --params`, the configured tool handler
//! is called directly during execution.

use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use eryx_vfs::{DirEntry, DirPerms, FilePerms, InMemoryStorage, Metadata, VfsResult, VfsStorage};

// Tool invocation uses the callback-based ToolHandler trait from crate::executor.
// The continuation module's ToolRequest/ToolResult types are kept for VFS history recording.
use super::history::HistoryProvider;
use super::tools::ToolDefinition;
use super::vfs::AgentVfs;
use crate::executor::{ToolHandler, ToolRequest, ToolResult};
use crate::limits::ResourceLimits;
use crate::policy::{PolicyHandler, PolicyStorage};
use crate::runtime::{ExecutionResult, RuntimeError};
use crate::shell::{Mount, Shell};

/// Wrapper around user-provided tool handler for agent-specific processing.
///
/// This wrapper can add agent-specific behavior like:
/// - Tool validation (checking if tool is in registry)
/// - VFS history recording
/// - Logging/metrics
struct AgentToolHandlerWrapper {
    inner: Arc<dyn ToolHandler>,
}

#[async_trait]
impl ToolHandler for AgentToolHandlerWrapper {
    async fn invoke(&self, request: ToolRequest) -> ToolResult {
        // For now, just delegate to the inner handler.
        // Future: validate tool exists in registry, write to VFS history, etc.
        self.inner.invoke(request).await
    }
}

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
/// let mut sandbox = AgentSandbox::builder("agent-123")
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
    tool_handler: Option<Arc<dyn ToolHandler>>,
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
            .field("has_tool_handler", &self.tool_handler.is_some())
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
            tool_handler: None,
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
    /// let mut sandbox = AgentSandbox::builder("agent-123")
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
    /// let mut sandbox = AgentSandbox::builder("agent-123")
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

    /// Set a tool handler for processing tool invocations from shell scripts.
    ///
    /// When a script runs `tool <name> --param value`, the handler is called
    /// directly during execution. This replaces the old exit-code-42 yield mechanism.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use conch::{ToolRequest, ToolResult};
    ///
    /// let mut sandbox = AgentSandbox::builder("agent-123")
    ///     .tool(ToolDefinition::no_params("web_search", "Search the web"))
    ///     .tool_handler(|req: ToolRequest| async move {
    ///         match req.tool.as_str() {
    ///             "web_search" => ToolResult {
    ///                 success: true,
    ///                 output: format!("Results for: {}", req.params),
    ///             },
    ///             _ => ToolResult {
    ///                 success: false,
    ///                 output: format!("Unknown tool: {}", req.tool),
    ///             },
    ///         }
    ///     })
    ///     .build()
    ///     .await?;
    /// ```
    pub fn tool_handler(mut self, handler: impl ToolHandler + 'static) -> Self {
        self.tool_handler = Some(Arc::new(handler));
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

        // Add tool handler if configured
        if let Some(handler) = self.tool_handler {
            shell_builder = shell_builder.tool_handler(AgentToolHandlerWrapper { inner: handler });
        }

        let shell = shell_builder.build().await?;

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
/// let mut sandbox = AgentSandbox::builder("agent-123")
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
        &mut self,
        script: &str,
        limits: &ResourceLimits,
    ) -> Result<ExecutionResult, RuntimeError> {
        self.shell.execute(script, limits).await
    }
}

#[cfg(test)]
#[cfg(feature = "embedded-shell")]
#[allow(clippy::expect_used, clippy::unwrap_used)]
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
        let mut sandbox = AgentSandbox::builder("agent-123")
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
        let mut sandbox = AgentSandbox::builder("agent-123")
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
        let mut sandbox = AgentSandbox::builder("agent-123")
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
        let mut sandbox = AgentSandbox::builder("agent-123")
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
    async fn test_sandbox_grep_tools() {
        let mut sandbox = AgentSandbox::builder("agent-123")
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
    async fn test_tool_handler_callback() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let call_count = Arc::new(AtomicUsize::new(0));
        let call_count_clone = call_count.clone();

        // Use an async closure directly - ToolHandler has a blanket impl for Fn(ToolRequest) -> Future
        let handler = move |request: crate::executor::ToolRequest| {
            let count = call_count_clone.clone();
            async move {
                count.fetch_add(1, Ordering::SeqCst);
                crate::executor::ToolResult {
                    success: true,
                    output: format!(
                        "Handled tool: {} with params: {}",
                        request.tool, request.params
                    ),
                }
            }
        };

        let mut sandbox = AgentSandbox::builder("agent-123")
            .tool(ToolDefinition::no_params("web_search", "Search the web"))
            .tool_handler(handler)
            .build()
            .await
            .expect("Failed to build sandbox");

        // Invoke the tool - should call our handler
        let result = sandbox
            .execute("tool web_search --query 'rust async'", &default_limits())
            .await
            .expect("Execute failed");

        // The tool command should succeed (exit code 0)
        assert_eq!(
            result.exit_code,
            0,
            "Tool invocation should succeed. stderr: {}",
            String::from_utf8_lossy(&result.stderr)
        );

        // Our handler should have been called
        assert_eq!(
            call_count.load(Ordering::SeqCst),
            1,
            "Handler should be called once"
        );

        // The output should contain our response
        let stdout = String::from_utf8_lossy(&result.stdout);
        assert!(
            stdout.contains("Handled tool: web_search"),
            "stdout: {}",
            stdout
        );
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

        let mut sandbox = AgentSandbox::builder("agent-123")
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

        let mut sandbox = AgentSandbox::builder("agent-123")
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

        let mut sandbox = AgentSandbox::builder("agent-123")
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
            let mut sandbox = AgentSandbox::builder("agent-123")
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
            let mut sandbox = AgentSandbox::builder("agent-123")
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
            let mut sandbox = AgentSandbox::builder("agent-123")
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
            let mut sandbox = AgentSandbox::builder("agent-123")
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

            let mut sandbox = AgentSandbox::builder("agent-123")
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
            let mut sandbox = AgentSandbox::builder("agent-123")
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
            let mut sandbox = AgentSandbox::builder("agent-123")
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
