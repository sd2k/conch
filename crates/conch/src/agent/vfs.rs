//! Agent-aware VFS that provides the agent filesystem structure.
//!
//! [`AgentVfs`] wraps any [`VfsStorage`] implementation and automatically
//! creates the agent directory structure on initialization.

use std::sync::Arc;
use std::time::SystemTime;

use async_trait::async_trait;
use eryx_vfs::{DirEntry, Metadata, VfsResult, VfsStorage};
use serde::{Deserialize, Serialize};

use super::history::{HistoryProvider, generate_history_index};
use super::tools::{ToolDefinition, generate_index_txt};

/// Metadata about the current agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentMetadata {
    /// Unique identifier for this agent.
    pub id: String,
    /// Human-readable name for the agent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// ID of the parent agent (if this is a sub-agent).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    /// When this agent was spawned.
    pub spawned_at: String,
    /// Capabilities granted to this agent.
    #[serde(default)]
    pub capabilities: Vec<String>,
}

/// Builder for creating an [`AgentVfs`] with custom configuration.
///
/// # Example
///
/// ```rust,ignore
/// use serde_json::json;
///
/// let vfs = AgentVfs::builder("agent-123")
///     .name("code-reviewer")
///     .parent("agent-root")
///     .capability("read_code")
///     .tool(ToolDefinition::new("web_search", "Search the web", json!({})))
///     .build(InMemoryStorage::new())
///     .await?;
/// ```
pub struct AgentVfsBuilder {
    agent_id: String,
    name: Option<String>,
    parent_id: Option<String>,
    capabilities: Vec<String>,
    params: Option<serde_json::Value>,
    tools: Vec<ToolDefinition>,
    history: Option<Arc<dyn HistoryProvider>>,
}

impl std::fmt::Debug for AgentVfsBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentVfsBuilder")
            .field("agent_id", &self.agent_id)
            .field("name", &self.name)
            .field("parent_id", &self.parent_id)
            .field("capabilities", &self.capabilities)
            .field("params", &self.params)
            .field("tools", &self.tools)
            .field("has_history", &self.history.is_some())
            .finish()
    }
}

impl AgentVfsBuilder {
    /// Create a new builder for an agent with the given ID.
    pub fn new(agent_id: impl Into<String>) -> Self {
        Self {
            agent_id: agent_id.into(),
            name: None,
            parent_id: None,
            capabilities: Vec::new(),
            params: None,
            tools: Vec::new(),
            history: None,
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

    /// Set the history provider for conversation context.
    ///
    /// The history provider supplies conversation transcripts that are
    /// accessible via `/history/` in the VFS.
    pub fn history(mut self, provider: Arc<dyn HistoryProvider>) -> Self {
        self.history = Some(provider);
        self
    }

    /// Build the [`AgentVfs`] with the given storage backend.
    ///
    /// This creates the agent directory structure and writes initial metadata.
    pub async fn build<S: VfsStorage + 'static>(self, storage: S) -> VfsResult<AgentVfs> {
        let storage = Arc::new(storage);

        // Create directory structure
        create_directory_structure(&*storage).await?;

        // Generate and write metadata
        let metadata = AgentMetadata {
            id: self.agent_id.clone(),
            name: self.name,
            parent_id: self.parent_id,
            spawned_at: chrono_lite_now(),
            capabilities: self.capabilities,
        };

        let metadata_json = serde_json::to_string_pretty(&metadata).map_err(|e| {
            eryx_vfs::VfsError::Storage(format!("Failed to serialize metadata: {e}"))
        })?;
        storage
            .write("/agent/metadata.json", metadata_json.as_bytes())
            .await?;

        // Write params if provided
        if let Some(params) = self.params {
            let params_json = serde_json::to_string_pretty(&params).map_err(|e| {
                eryx_vfs::VfsError::Storage(format!("Failed to serialize params: {e}"))
            })?;
            storage
                .write("/agent/params.json", params_json.as_bytes())
                .await?;
        } else {
            // Write empty object as default params
            storage.write("/agent/params.json", b"{}").await?;
        }

        // Write tools index and definitions
        write_tools(&*storage, &self.tools).await?;

        // Write history if provider is set
        if let Some(history) = &self.history {
            write_history(&*storage, history.as_ref()).await?;
        }

        Ok(AgentVfs {
            storage,
            agent_id: self.agent_id,
            call_counter: std::sync::atomic::AtomicU32::new(0),
        })
    }
}

/// Agent-aware VFS that provides the agent filesystem structure.
///
/// This wraps any [`VfsStorage`] implementation and provides:
/// - Automatic directory structure creation
/// - Agent metadata in `/agent/metadata.json`
/// - Tool call ID generation
///
/// # Filesystem Structure
///
/// ```text
/// /agent/              - Current agent context (read-write)
/// ├── metadata.json    - Agent ID, name, capabilities
/// ├── params.json      - Parameters passed when spawned
/// ├── scratch/         - Temporary working directory
/// └── state/           - Persistent state across tool calls
///
/// /tools/              - Tool definitions and results
/// ├── index.txt        - Quick reference (name + description)
/// ├── available/       - Full tool definitions (JSON schema)
/// ├── pending/         - Tools currently executing
/// └── history/         - Completed tool calls
/// ```
pub struct AgentVfs {
    storage: Arc<dyn VfsStorage>,
    agent_id: String,
    call_counter: std::sync::atomic::AtomicU32,
}

impl std::fmt::Debug for AgentVfs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentVfs")
            .field("agent_id", &self.agent_id)
            .field("call_counter", &self.call_counter)
            .finish_non_exhaustive()
    }
}

impl AgentVfs {
    /// Create a new builder for an agent VFS.
    pub fn builder(agent_id: impl Into<String>) -> AgentVfsBuilder {
        AgentVfsBuilder::new(agent_id)
    }

    /// Get the agent ID.
    pub fn agent_id(&self) -> &str {
        &self.agent_id
    }

    /// Generate the next tool call ID.
    ///
    /// Call IDs are sequential: `call-001`, `call-002`, etc.
    pub fn next_call_id(&self) -> String {
        let n = self
            .call_counter
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
            + 1;
        format!("call-{n:03}")
    }

    /// Get access to the underlying storage.
    pub fn storage(&self) -> &dyn VfsStorage {
        &*self.storage
    }

    /// Get access to the underlying storage as an Arc.
    pub fn storage_arc(&self) -> Arc<dyn VfsStorage> {
        Arc::clone(&self.storage)
    }
}

/// Create the standard agent directory structure.
async fn create_directory_structure(storage: &dyn VfsStorage) -> VfsResult<()> {
    // Agent directories
    mkdir_p(storage, "/agent").await?;
    mkdir_p(storage, "/agent/scratch").await?;
    mkdir_p(storage, "/agent/state").await?;

    // Tools directories
    mkdir_p(storage, "/tools").await?;
    mkdir_p(storage, "/tools/available").await?;
    mkdir_p(storage, "/tools/pending").await?;
    mkdir_p(storage, "/tools/history").await?;

    // History directories
    mkdir_p(storage, "/history").await?;
    mkdir_p(storage, "/history/current").await?;

    Ok(())
}

/// Write tools index and definitions to the filesystem.
async fn write_tools(storage: &dyn VfsStorage, tools: &[ToolDefinition]) -> VfsResult<()> {
    // Generate and write index.txt
    let summaries: Vec<_> = tools.iter().map(ToolDefinition::summary).collect();
    let index_content = generate_index_txt(&summaries);
    storage
        .write("/tools/index.txt", index_content.as_bytes())
        .await?;

    // Write each tool definition as JSON
    for tool in tools {
        let path = format!("/tools/available/{}.json", tool.name);
        let json = serde_json::to_string_pretty(tool).map_err(|e| {
            eryx_vfs::VfsError::Storage(format!("Failed to serialize tool {}: {e}", tool.name))
        })?;
        storage.write(&path, json.as_bytes()).await?;
    }

    Ok(())
}

/// Write conversation history to the filesystem.
async fn write_history(storage: &dyn VfsStorage, history: &dyn HistoryProvider) -> VfsResult<()> {
    // Write index.txt with conversation list
    let conversations = history.list_conversations();
    let index_content = generate_history_index(&conversations);
    storage
        .write("/history/index.txt", index_content.as_bytes())
        .await?;

    // Write current conversation if available
    if let Some(transcript) = history.current_transcript() {
        storage
            .write("/history/current/transcript.md", transcript.as_bytes())
            .await?;
    }

    if let Some(metadata) = history.current_metadata() {
        let metadata_json = serde_json::to_string_pretty(&metadata).map_err(|e| {
            eryx_vfs::VfsError::Storage(format!("Failed to serialize history metadata: {e}"))
        })?;
        storage
            .write("/history/current/metadata.json", metadata_json.as_bytes())
            .await?;
    }

    // Write historical conversations
    for conv in &conversations {
        if conv.is_current {
            continue; // Already written above
        }

        let conv_dir = format!("/history/{}", conv.id);
        mkdir_p(storage, &conv_dir).await?;

        if let Some(transcript) = history.get_transcript(&conv.id) {
            storage
                .write(&format!("{conv_dir}/transcript.md"), transcript.as_bytes())
                .await?;
        }

        if let Some(metadata) = history.get_metadata(&conv.id) {
            let metadata_json = serde_json::to_string_pretty(&metadata).map_err(|e| {
                eryx_vfs::VfsError::Storage(format!("Failed to serialize history metadata: {e}"))
            })?;
            storage
                .write(
                    &format!("{conv_dir}/metadata.json"),
                    metadata_json.as_bytes(),
                )
                .await?;
        }
    }

    Ok(())
}

/// Create a directory if it doesn't exist (mkdir -p style).
async fn mkdir_p(storage: &dyn VfsStorage, path: &str) -> VfsResult<()> {
    // Build up path components and create each directory
    let mut current = String::new();
    for component in path.split('/').filter(|s| !s.is_empty()) {
        current = format!("{current}/{component}");
        if !storage.exists(&current).await? {
            storage.mkdir(&current).await?;
        }
    }
    Ok(())
}

/// Get current time in RFC3339 format without external chrono dependency.
fn chrono_lite_now() -> String {
    // Get current time and format as ISO 8601 / RFC 3339
    let now = SystemTime::now();
    let duration = now
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();

    // Simple formatting - for production, consider using chrono crate
    let secs = duration.as_secs();

    // Calculate date/time components (simplified, doesn't handle leap seconds)
    let days_since_epoch = secs / 86400;
    let time_of_day = secs % 86400;

    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;

    // Simplified year/month/day calculation (approximate, good enough for logging)
    let (year, month, day) = days_to_ymd(days_since_epoch);

    format!("{year:04}-{month:02}-{day:02}T{hours:02}:{minutes:02}:{seconds:02}Z")
}

/// Convert days since Unix epoch to year/month/day.
fn days_to_ymd(days: u64) -> (u32, u32, u32) {
    // Simplified calculation - approximation for reasonable dates
    let days = days as i64;

    // Days from 1970 to 2000
    const DAYS_1970_TO_2000: i64 = 10957;

    let days_from_2000 = days - DAYS_1970_TO_2000;

    // Approximate year (365.25 days average)
    let mut year = 2000 + (days_from_2000 / 365) as u32;

    // Adjust for leap years
    let mut day_of_year = days_from_2000 % 365;
    if day_of_year < 0 {
        year -= 1;
        day_of_year += 365;
    }

    let is_leap = year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
    let month_days: [u32; 12] = if is_leap {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };

    let mut month = 1u32;
    let mut remaining = day_of_year as u32;
    for &days_in_month in &month_days {
        if remaining < days_in_month {
            break;
        }
        remaining -= days_in_month;
        month += 1;
    }

    let day = remaining + 1;

    (year, month.min(12), day.clamp(1, 31))
}

// Implement VfsStorage by delegating to the inner storage
#[async_trait]
impl VfsStorage for AgentVfs {
    async fn read(&self, path: &str) -> VfsResult<Vec<u8>> {
        self.storage.read(path).await
    }

    async fn read_at(&self, path: &str, offset: u64, len: u64) -> VfsResult<Vec<u8>> {
        self.storage.read_at(path, offset, len).await
    }

    async fn write(&self, path: &str, data: &[u8]) -> VfsResult<()> {
        self.storage.write(path, data).await
    }

    async fn write_at(&self, path: &str, offset: u64, data: &[u8]) -> VfsResult<()> {
        self.storage.write_at(path, offset, data).await
    }

    async fn set_size(&self, path: &str, size: u64) -> VfsResult<()> {
        self.storage.set_size(path, size).await
    }

    async fn delete(&self, path: &str) -> VfsResult<()> {
        self.storage.delete(path).await
    }

    async fn exists(&self, path: &str) -> VfsResult<bool> {
        self.storage.exists(path).await
    }

    async fn list(&self, path: &str) -> VfsResult<Vec<DirEntry>> {
        self.storage.list(path).await
    }

    async fn stat(&self, path: &str) -> VfsResult<Metadata> {
        self.storage.stat(path).await
    }

    async fn mkdir(&self, path: &str) -> VfsResult<()> {
        self.storage.mkdir(path).await
    }

    async fn rmdir(&self, path: &str) -> VfsResult<()> {
        self.storage.rmdir(path).await
    }

    async fn rename(&self, from: &str, to: &str) -> VfsResult<()> {
        self.storage.rename(from, to).await
    }

    fn mkdir_sync(&self, path: &str) -> VfsResult<()> {
        self.storage.mkdir_sync(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eryx_vfs::InMemoryStorage;

    #[tokio::test]
    async fn test_agent_vfs_creation() {
        let vfs = AgentVfs::builder("agent-123")
            .build(InMemoryStorage::new())
            .await
            .expect("Failed to build AgentVfs");

        assert_eq!(vfs.agent_id(), "agent-123");
    }

    #[tokio::test]
    async fn test_directory_structure_created() {
        let vfs = AgentVfs::builder("agent-123")
            .build(InMemoryStorage::new())
            .await
            .expect("Failed to build AgentVfs");

        // Check agent directories exist
        assert!(vfs.exists("/agent").await.expect("exists check failed"));
        assert!(
            vfs.exists("/agent/scratch")
                .await
                .expect("exists check failed")
        );
        assert!(
            vfs.exists("/agent/state")
                .await
                .expect("exists check failed")
        );

        // Check tools directories exist
        assert!(vfs.exists("/tools").await.expect("exists check failed"));
        assert!(
            vfs.exists("/tools/available")
                .await
                .expect("exists check failed")
        );
        assert!(
            vfs.exists("/tools/pending")
                .await
                .expect("exists check failed")
        );
        assert!(
            vfs.exists("/tools/history")
                .await
                .expect("exists check failed")
        );

        // Check index file exists
        assert!(
            vfs.exists("/tools/index.txt")
                .await
                .expect("exists check failed")
        );
    }

    #[tokio::test]
    async fn test_metadata_written() {
        let vfs = AgentVfs::builder("agent-123")
            .name("test-agent")
            .parent("parent-456")
            .capability("read_code")
            .capability("execute_tools")
            .build(InMemoryStorage::new())
            .await
            .expect("Failed to build AgentVfs");

        let metadata_bytes = vfs
            .read("/agent/metadata.json")
            .await
            .expect("read metadata");
        let metadata: AgentMetadata =
            serde_json::from_slice(&metadata_bytes).expect("parse metadata");

        assert_eq!(metadata.id, "agent-123");
        assert_eq!(metadata.name.as_deref(), Some("test-agent"));
        assert_eq!(metadata.parent_id.as_deref(), Some("parent-456"));
        assert_eq!(metadata.capabilities, vec!["read_code", "execute_tools"]);
    }

    #[tokio::test]
    async fn test_params_written() {
        let params = serde_json::json!({
            "task": "review code",
            "files": ["src/main.rs"]
        });

        let vfs = AgentVfs::builder("agent-123")
            .params(params.clone())
            .build(InMemoryStorage::new())
            .await
            .expect("Failed to build AgentVfs");

        let params_bytes = vfs.read("/agent/params.json").await.expect("read params");
        let read_params: serde_json::Value =
            serde_json::from_slice(&params_bytes).expect("parse params");

        assert_eq!(read_params, params);
    }

    #[tokio::test]
    async fn test_default_params() {
        let vfs = AgentVfs::builder("agent-123")
            .build(InMemoryStorage::new())
            .await
            .expect("Failed to build AgentVfs");

        let params_bytes = vfs.read("/agent/params.json").await.expect("read params");
        let params: serde_json::Value =
            serde_json::from_slice(&params_bytes).expect("parse params");

        assert_eq!(params, serde_json::json!({}));
    }

    #[tokio::test]
    async fn test_call_id_generation() {
        let vfs = AgentVfs::builder("agent-123")
            .build(InMemoryStorage::new())
            .await
            .expect("Failed to build AgentVfs");

        assert_eq!(vfs.next_call_id(), "call-001");
        assert_eq!(vfs.next_call_id(), "call-002");
        assert_eq!(vfs.next_call_id(), "call-003");
    }

    #[tokio::test]
    async fn test_vfs_storage_delegation() {
        let vfs = AgentVfs::builder("agent-123")
            .build(InMemoryStorage::new())
            .await
            .expect("Failed to build AgentVfs");

        // Write to scratch
        vfs.write("/agent/scratch/test.txt", b"hello world")
            .await
            .expect("write failed");

        // Read back
        let content = vfs
            .read("/agent/scratch/test.txt")
            .await
            .expect("read failed");
        assert_eq!(content, b"hello world");

        // Delete
        vfs.delete("/agent/scratch/test.txt")
            .await
            .expect("delete failed");
        assert!(
            !vfs.exists("/agent/scratch/test.txt")
                .await
                .expect("exists check")
        );
    }

    #[tokio::test]
    async fn test_chrono_lite_now_format() {
        let timestamp = chrono_lite_now();

        // Should be in format YYYY-MM-DDTHH:MM:SSZ
        assert!(
            timestamp.len() == 20,
            "timestamp should be 20 chars: {}",
            timestamp
        );
        assert!(
            timestamp.ends_with('Z'),
            "timestamp should end with Z: {}",
            timestamp
        );
        assert!(
            timestamp.contains('T'),
            "timestamp should contain T: {}",
            timestamp
        );

        // Basic validation of year
        let year: u32 = timestamp[0..4].parse().expect("parse year");
        assert!(
            year >= 2020 && year <= 2100,
            "year should be reasonable: {}",
            year
        );
    }

    #[tokio::test]
    async fn test_tools_written_to_vfs() {
        let tools = vec![
            ToolDefinition::new(
                "web_search",
                "Search the web for information",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "query": { "type": "string" }
                    },
                    "required": ["query"]
                }),
            ),
            ToolDefinition::no_params("list_files", "List files in a directory"),
        ];

        let vfs = AgentVfs::builder("agent-123")
            .tools(tools)
            .build(InMemoryStorage::new())
            .await
            .expect("Failed to build AgentVfs");

        // Check index.txt contains tool names
        let index = vfs.read("/tools/index.txt").await.expect("read index");
        let index_str = String::from_utf8_lossy(&index);
        assert!(
            index_str.contains("web_search"),
            "index should contain web_search"
        );
        assert!(
            index_str.contains("list_files"),
            "index should contain list_files"
        );
        assert!(
            index_str.contains("Search the web"),
            "index should contain description"
        );

        // Check individual tool definitions exist
        assert!(
            vfs.exists("/tools/available/web_search.json")
                .await
                .expect("exists check")
        );
        assert!(
            vfs.exists("/tools/available/list_files.json")
                .await
                .expect("exists check")
        );

        // Check tool definition content
        let tool_json = vfs
            .read("/tools/available/web_search.json")
            .await
            .expect("read tool");
        let tool: ToolDefinition = serde_json::from_slice(&tool_json).expect("parse tool");
        assert_eq!(tool.name, "web_search");
        assert_eq!(tool.description, "Search the web for information");
    }

    #[tokio::test]
    async fn test_no_tools_empty_index() {
        let vfs = AgentVfs::builder("agent-123")
            .build(InMemoryStorage::new())
            .await
            .expect("Failed to build AgentVfs");

        // Index should exist but be empty
        let index = vfs.read("/tools/index.txt").await.expect("read index");
        assert!(index.is_empty(), "index should be empty when no tools");

        // No tool files should exist
        let entries = vfs.list("/tools/available").await.expect("list available");
        assert!(entries.is_empty(), "available should be empty");
    }

    #[tokio::test]
    async fn test_single_tool() {
        let vfs = AgentVfs::builder("agent-123")
            .tool(ToolDefinition::no_params("my_tool", "A single tool"))
            .build(InMemoryStorage::new())
            .await
            .expect("Failed to build AgentVfs");

        let index = vfs.read("/tools/index.txt").await.expect("read index");
        let index_str = String::from_utf8_lossy(&index);
        assert!(index_str.contains("my_tool"));

        let entries = vfs.list("/tools/available").await.expect("list available");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "my_tool.json");
    }
}
