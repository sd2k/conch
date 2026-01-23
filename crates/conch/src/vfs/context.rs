//! Virtual filesystem for agent context

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Errors that can occur during filesystem operations
#[derive(Debug, Error)]
pub enum FsError {
    /// File or directory not found
    #[error("file not found: {0}")]
    NotFound(String),
    /// Permission denied for the operation
    #[error("permission denied: {0}")]
    PermissionDenied(String),
    /// Path is not a directory
    #[error("not a directory: {0}")]
    NotADirectory(String),
    /// Path is not a file
    #[error("not a file: {0}")]
    NotAFile(String),
    /// Filesystem is read-only
    #[error("read-only filesystem")]
    ReadOnly,
    /// Invalid path format
    #[error("invalid path: {0}")]
    InvalidPath(String),
    /// IO error
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    /// Error from the context provider
    #[error("provider error: {0}")]
    Provider(String),
}

/// Directory entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirEntry {
    /// Entry name
    pub name: String,
    /// Whether this entry is a directory
    pub is_dir: bool,
}

/// File/directory metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Metadata {
    /// Whether this is a directory
    pub is_dir: bool,
    /// Whether this is a file
    pub is_file: bool,
    /// File size in bytes
    pub size: u64,
    /// Last modified time as Unix timestamp
    pub modified: Option<u64>,
}

/// Tool call data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    /// Unique identifier for the tool call
    pub id: String,
    /// Name of the tool that was called
    pub tool: String,
    /// Parameters passed to the tool
    pub params: serde_json::Value,
    /// Result returned by the tool
    pub result: serde_json::Value,
    /// ISO 8601 timestamp when the call started
    pub started_at: String,
    /// ISO 8601 timestamp when the call completed
    pub completed_at: String,
    /// Duration of the call in milliseconds
    pub duration_ms: u64,
    /// Whether the call succeeded
    pub success: bool,
    /// Error message if the call failed
    pub error: Option<String>,
}

/// Tool call summary (for listings)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallSummary {
    /// Unique identifier for the tool call
    pub id: String,
    /// Name of the tool that was called
    pub tool: String,
    /// Whether the call succeeded
    pub success: bool,
}

/// Message in conversation history
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    /// Role of the message sender (e.g., "user", "assistant")
    pub role: String,
    /// Message content
    pub content: String,
}

/// Trait for providing context data to the virtual filesystem
#[async_trait]
pub trait ContextProvider: Send + Sync {
    /// Get tool call by ID
    async fn get_tool_call(&self, agent_id: &str, id: &str) -> Result<ToolCall, FsError>;

    /// List tool calls for an agent
    async fn list_tool_calls(&self, agent_id: &str) -> Result<Vec<ToolCallSummary>, FsError>;

    /// Get message by index
    async fn get_message(&self, agent_id: &str, index: usize) -> Result<Message, FsError>;

    /// Get message count
    async fn message_count(&self, agent_id: &str) -> Result<usize, FsError>;

    /// Write to scratch space
    async fn write_scratch(&self, agent_id: &str, path: &str, data: &[u8]) -> Result<(), FsError>;

    /// Read from scratch space
    async fn read_scratch(&self, agent_id: &str, path: &str) -> Result<Vec<u8>, FsError>;

    /// Publish artifact (visible to parent)
    async fn publish_artifact(
        &self,
        agent_id: &str,
        name: &str,
        data: &[u8],
    ) -> Result<(), FsError>;
}

/// Access control policy for the virtual filesystem
#[derive(Debug, Clone)]
pub struct AccessPolicy {
    /// ID of the current agent
    pub agent_id: String,
    /// ID of the parent agent, if any
    pub parent_agent_id: Option<String>,
    /// IDs of child agents
    pub child_agent_ids: Vec<String>,
    /// ID of the shared investigation, if any
    pub investigation_id: Option<String>,
    /// Whether this agent can read parent's tool calls
    pub can_read_parent_tools: bool,
    /// Whether this agent can read parent's messages
    pub can_read_parent_messages: bool,
}

impl Default for AccessPolicy {
    fn default() -> Self {
        Self {
            agent_id: "self".to_string(),
            parent_agent_id: None,
            child_agent_ids: Vec::new(),
            investigation_id: None,
            can_read_parent_tools: true,
            can_read_parent_messages: false,
        }
    }
}

/// Parsed path within the context filesystem
#[derive(Debug, Clone)]
pub enum ContextPath {
    Root,
    Self_,
    SelfMeta,
    SelfTools,
    SelfToolCall { id: String },
    SelfToolCallFile { id: String, file: String },
    SelfToolsByName,
    SelfToolsByNameTool { tool: String },
    SelfMessages,
    SelfMessage { index: usize },
    SelfScratch,
    SelfScratchFile { path: String },
    SelfArtifacts,
    Parent,
    ParentTools,
    ParentToolCall { id: String },
    ParentToolCallFile { id: String, file: String },
    Children,
    Child { id: String },
    Shared,
    SharedInvestigation { id: String },
}

/// Virtual filesystem for agent context
pub struct ContextFs {
    provider: Arc<dyn ContextProvider>,
    policy: AccessPolicy,
    cache: tokio::sync::RwLock<HashMap<(String, String), ToolCall>>,
}

impl std::fmt::Debug for ContextFs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ContextFs")
            .field("policy", &self.policy)
            .field("provider", &"<dyn ContextProvider>")
            .finish()
    }
}

impl ContextFs {
    /// Create a new context filesystem with the given provider and access policy
    pub fn new(provider: Arc<dyn ContextProvider>, policy: AccessPolicy) -> Self {
        Self {
            provider,
            policy,
            cache: tokio::sync::RwLock::new(HashMap::new()),
        }
    }

    /// Parse a path into a ContextPath
    pub fn parse_path(&self, path: &str) -> Result<ContextPath, FsError> {
        let path = path.trim_start_matches('/');
        let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();

        if parts.is_empty() || (parts.len() == 1 && parts[0] == "ctx") {
            return Ok(ContextPath::Root);
        }

        // Skip "ctx" prefix if present
        let parts = if parts.first() == Some(&"ctx") {
            &parts[1..]
        } else {
            &parts[..]
        };

        match parts {
            [] => Ok(ContextPath::Root),
            ["self"] => Ok(ContextPath::Self_),
            ["self", "meta.json"] => Ok(ContextPath::SelfMeta),
            ["self", "tools"] => Ok(ContextPath::SelfTools),
            ["self", "tools", "by-name"] => Ok(ContextPath::SelfToolsByName),
            ["self", "tools", "by-name", tool] => Ok(ContextPath::SelfToolsByNameTool {
                tool: (*tool).to_string(),
            }),
            ["self", "tools", id] => Ok(ContextPath::SelfToolCall {
                id: (*id).to_string(),
            }),
            ["self", "tools", id, file] => Ok(ContextPath::SelfToolCallFile {
                id: (*id).to_string(),
                file: (*file).to_string(),
            }),
            ["self", "messages"] => Ok(ContextPath::SelfMessages),
            ["self", "messages", idx] => {
                let index: usize = idx
                    .parse()
                    .map_err(|_| FsError::InvalidPath(format!("invalid message index: {}", idx)))?;
                Ok(ContextPath::SelfMessage { index })
            }
            ["self", "scratch"] => Ok(ContextPath::SelfScratch),
            ["self", "scratch", rest @ ..] => Ok(ContextPath::SelfScratchFile {
                path: rest.join("/"),
            }),
            ["self", "artifacts"] => Ok(ContextPath::SelfArtifacts),
            ["parent"] => Ok(ContextPath::Parent),
            ["parent", "tools"] => Ok(ContextPath::ParentTools),
            ["parent", "tools", id] => Ok(ContextPath::ParentToolCall {
                id: (*id).to_string(),
            }),
            ["parent", "tools", id, file] => Ok(ContextPath::ParentToolCallFile {
                id: (*id).to_string(),
                file: (*file).to_string(),
            }),
            ["children"] => Ok(ContextPath::Children),
            ["children", id, ..] => Ok(ContextPath::Child {
                id: (*id).to_string(),
            }),
            ["shared"] => Ok(ContextPath::Shared),
            ["shared", id, ..] => Ok(ContextPath::SharedInvestigation {
                id: (*id).to_string(),
            }),
            _ => Err(FsError::NotFound(path.to_string())),
        }
    }

    /// Check if access is allowed for the given path
    pub fn check_access(&self, parsed: &ContextPath) -> Result<(), FsError> {
        match parsed {
            ContextPath::Parent
            | ContextPath::ParentTools
            | ContextPath::ParentToolCall { .. }
            | ContextPath::ParentToolCallFile { .. } => {
                if self.policy.parent_agent_id.is_none() {
                    return Err(FsError::PermissionDenied("no parent agent".to_string()));
                }
                if !self.policy.can_read_parent_tools {
                    return Err(FsError::PermissionDenied(
                        "parent tool access denied".to_string(),
                    ));
                }
                Ok(())
            }
            ContextPath::Child { id } => {
                if !self.policy.child_agent_ids.contains(id) {
                    return Err(FsError::PermissionDenied(format!(
                        "child agent {} not accessible",
                        id
                    )));
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// Read file contents
    pub async fn read(&self, path: &str) -> Result<Vec<u8>, FsError> {
        let parsed = self.parse_path(path)?;
        self.check_access(&parsed)?;

        match parsed {
            ContextPath::SelfMeta => {
                let meta = serde_json::json!({
                    "agent_id": self.policy.agent_id,
                    "parent_agent_id": self.policy.parent_agent_id,
                    "child_agent_ids": self.policy.child_agent_ids,
                });
                serde_json::to_vec_pretty(&meta).map_err(|e| FsError::Provider(e.to_string()))
            }
            ContextPath::SelfToolCallFile { id, file } => {
                let tool_call = self
                    .get_or_fetch_tool_call(&self.policy.agent_id, &id)
                    .await?;
                self.extract_tool_call_file(&tool_call, &file)
            }
            ContextPath::ParentToolCallFile { id, file } => {
                let parent_id = self
                    .policy
                    .parent_agent_id
                    .as_ref()
                    .ok_or_else(|| FsError::PermissionDenied("no parent agent".to_string()))?;
                let tool_call = self.get_or_fetch_tool_call(parent_id, &id).await?;
                self.extract_tool_call_file(&tool_call, &file)
            }
            ContextPath::SelfMessage { index } => {
                let msg = self
                    .provider
                    .get_message(&self.policy.agent_id, index)
                    .await?;
                Ok(msg.content.into_bytes())
            }
            ContextPath::SelfScratchFile { path } => {
                self.provider
                    .read_scratch(&self.policy.agent_id, &path)
                    .await
            }
            _ => Err(FsError::NotAFile(path.to_string())),
        }
    }

    /// List directory entries
    pub async fn read_dir(&self, path: &str) -> Result<Vec<DirEntry>, FsError> {
        let parsed = self.parse_path(path)?;
        self.check_access(&parsed)?;

        match parsed {
            ContextPath::Root => Ok(vec![
                DirEntry {
                    name: "self".to_string(),
                    is_dir: true,
                },
                DirEntry {
                    name: "parent".to_string(),
                    is_dir: true,
                },
                DirEntry {
                    name: "children".to_string(),
                    is_dir: true,
                },
                DirEntry {
                    name: "shared".to_string(),
                    is_dir: true,
                },
            ]),
            ContextPath::Self_ => Ok(vec![
                DirEntry {
                    name: "meta.json".to_string(),
                    is_dir: false,
                },
                DirEntry {
                    name: "tools".to_string(),
                    is_dir: true,
                },
                DirEntry {
                    name: "messages".to_string(),
                    is_dir: true,
                },
                DirEntry {
                    name: "scratch".to_string(),
                    is_dir: true,
                },
                DirEntry {
                    name: "artifacts".to_string(),
                    is_dir: true,
                },
            ]),
            ContextPath::SelfTools => {
                let tool_calls = self.provider.list_tool_calls(&self.policy.agent_id).await?;
                let mut entries: Vec<DirEntry> = tool_calls
                    .into_iter()
                    .map(|tc| DirEntry {
                        name: tc.id,
                        is_dir: true,
                    })
                    .collect();
                entries.push(DirEntry {
                    name: "by-name".to_string(),
                    is_dir: true,
                });
                Ok(entries)
            }
            ContextPath::SelfToolCall { id: _ } => {
                // Each tool call directory has these files
                Ok(vec![
                    DirEntry {
                        name: "request.json".to_string(),
                        is_dir: false,
                    },
                    DirEntry {
                        name: "result.json".to_string(),
                        is_dir: false,
                    },
                    DirEntry {
                        name: "meta.json".to_string(),
                        is_dir: false,
                    },
                ])
            }
            ContextPath::ParentTools => {
                let parent_id = self
                    .policy
                    .parent_agent_id
                    .as_ref()
                    .ok_or_else(|| FsError::PermissionDenied("no parent agent".to_string()))?;
                let tool_calls = self.provider.list_tool_calls(parent_id).await?;
                Ok(tool_calls
                    .into_iter()
                    .map(|tc| DirEntry {
                        name: tc.id,
                        is_dir: true,
                    })
                    .collect())
            }
            ContextPath::Children => Ok(self
                .policy
                .child_agent_ids
                .iter()
                .map(|id| DirEntry {
                    name: id.clone(),
                    is_dir: true,
                })
                .collect()),
            _ => Err(FsError::NotADirectory(path.to_string())),
        }
    }

    /// Get file/directory metadata
    pub async fn stat(&self, path: &str) -> Result<Metadata, FsError> {
        let parsed = self.parse_path(path)?;
        self.check_access(&parsed)?;

        let is_dir = matches!(
            parsed,
            ContextPath::Root
                | ContextPath::Self_
                | ContextPath::SelfTools
                | ContextPath::SelfToolCall { .. }
                | ContextPath::SelfToolsByName
                | ContextPath::SelfToolsByNameTool { .. }
                | ContextPath::SelfMessages
                | ContextPath::SelfScratch
                | ContextPath::SelfArtifacts
                | ContextPath::Parent
                | ContextPath::ParentTools
                | ContextPath::ParentToolCall { .. }
                | ContextPath::Children
                | ContextPath::Child { .. }
                | ContextPath::Shared
                | ContextPath::SharedInvestigation { .. }
        );

        Ok(Metadata {
            is_dir,
            is_file: !is_dir,
            size: 0, // We don't track size
            modified: None,
        })
    }

    /// Check if path exists
    pub async fn exists(&self, path: &str) -> Result<bool, FsError> {
        match self.stat(path).await {
            Ok(_) => Ok(true),
            Err(FsError::NotFound(_)) => Ok(false),
            Err(e) => Err(e),
        }
    }

    async fn get_or_fetch_tool_call(&self, agent_id: &str, id: &str) -> Result<ToolCall, FsError> {
        let key = (agent_id.to_string(), id.to_string());

        // Check cache
        {
            let cache = self.cache.read().await;
            if let Some(tc) = cache.get(&key) {
                return Ok(tc.clone());
            }
        }

        // Fetch from provider
        let tool_call = self.provider.get_tool_call(agent_id, id).await?;

        // Cache it
        {
            let mut cache = self.cache.write().await;
            cache.insert(key, tool_call.clone());
        }

        Ok(tool_call)
    }

    fn extract_tool_call_file(&self, tool_call: &ToolCall, file: &str) -> Result<Vec<u8>, FsError> {
        match file {
            "request.json" => {
                let request = serde_json::json!({
                    "tool": tool_call.tool,
                    "params": tool_call.params,
                });
                serde_json::to_vec_pretty(&request).map_err(|e| FsError::Provider(e.to_string()))
            }
            "result.json" => serde_json::to_vec_pretty(&tool_call.result)
                .map_err(|e| FsError::Provider(e.to_string())),
            "meta.json" => {
                let meta = serde_json::json!({
                    "id": tool_call.id,
                    "tool": tool_call.tool,
                    "started_at": tool_call.started_at,
                    "completed_at": tool_call.completed_at,
                    "duration_ms": tool_call.duration_ms,
                    "success": tool_call.success,
                    "error": tool_call.error,
                });
                serde_json::to_vec_pretty(&meta).map_err(|e| FsError::Provider(e.to_string()))
            }
            _ => Err(FsError::NotFound(file.to_string())),
        }
    }
}

/// Mock context provider for testing
#[allow(dead_code)]
pub struct MockContextProvider {
    tool_calls: HashMap<String, Vec<ToolCall>>,
    messages: HashMap<String, Vec<Message>>,
    scratch: tokio::sync::RwLock<HashMap<(String, String), Vec<u8>>>,
}

#[allow(dead_code)]
impl MockContextProvider {
    pub fn new() -> Self {
        Self {
            tool_calls: HashMap::new(),
            messages: HashMap::new(),
            scratch: tokio::sync::RwLock::new(HashMap::new()),
        }
    }

    pub fn with_tool_calls(mut self, agent_id: &str, calls: Vec<ToolCall>) -> Self {
        self.tool_calls.insert(agent_id.to_string(), calls);
        self
    }

    pub fn with_messages(mut self, agent_id: &str, msgs: Vec<Message>) -> Self {
        self.messages.insert(agent_id.to_string(), msgs);
        self
    }
}

impl Default for MockContextProvider {
    fn default() -> Self {
        Self::new()
    }
}

/// Artifacts storage for MockContextProvider
#[allow(dead_code)]
impl MockContextProvider {
    pub fn artifacts(&self) -> &tokio::sync::RwLock<HashMap<(String, String), Vec<u8>>> {
        // This is a bit of a hack - we use scratch storage for artifacts too
        &self.scratch
    }
}

#[async_trait]
impl ContextProvider for MockContextProvider {
    async fn get_tool_call(&self, agent_id: &str, id: &str) -> Result<ToolCall, FsError> {
        self.tool_calls
            .get(agent_id)
            .and_then(|calls| calls.iter().find(|tc| tc.id == id))
            .cloned()
            .ok_or_else(|| FsError::NotFound(format!("tool call {} not found", id)))
    }

    async fn list_tool_calls(&self, agent_id: &str) -> Result<Vec<ToolCallSummary>, FsError> {
        Ok(self
            .tool_calls
            .get(agent_id)
            .map(|calls| {
                calls
                    .iter()
                    .map(|tc| ToolCallSummary {
                        id: tc.id.clone(),
                        tool: tc.tool.clone(),
                        success: tc.success,
                    })
                    .collect()
            })
            .unwrap_or_default())
    }

    async fn get_message(&self, agent_id: &str, index: usize) -> Result<Message, FsError> {
        self.messages
            .get(agent_id)
            .and_then(|msgs| msgs.get(index))
            .cloned()
            .ok_or_else(|| FsError::NotFound(format!("message {} not found", index)))
    }

    async fn message_count(&self, agent_id: &str) -> Result<usize, FsError> {
        Ok(self.messages.get(agent_id).map(|m| m.len()).unwrap_or(0))
    }

    async fn write_scratch(&self, agent_id: &str, path: &str, data: &[u8]) -> Result<(), FsError> {
        let mut scratch = self.scratch.write().await;
        scratch.insert((agent_id.to_string(), path.to_string()), data.to_vec());
        Ok(())
    }

    async fn read_scratch(&self, agent_id: &str, path: &str) -> Result<Vec<u8>, FsError> {
        let scratch = self.scratch.read().await;
        scratch
            .get(&(agent_id.to_string(), path.to_string()))
            .cloned()
            .ok_or_else(|| FsError::NotFound(format!("scratch file {} not found", path)))
    }

    async fn publish_artifact(
        &self,
        agent_id: &str,
        name: &str,
        data: &[u8],
    ) -> Result<(), FsError> {
        let mut scratch = self.scratch.write().await;
        scratch.insert(
            (agent_id.to_string(), format!("__artifact__{}", name)),
            data.to_vec(),
        );
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    // Helper to create a test provider with sample data
    fn create_test_provider() -> MockContextProvider {
        MockContextProvider::new()
            .with_tool_calls(
                "agent-1",
                vec![
                    ToolCall {
                        id: "tc-001".to_string(),
                        tool: "read_file".to_string(),
                        params: serde_json::json!({"path": "/etc/hosts"}),
                        result: serde_json::json!({"content": "127.0.0.1 localhost"}),
                        started_at: "2024-01-01T00:00:00Z".to_string(),
                        completed_at: "2024-01-01T00:00:01Z".to_string(),
                        duration_ms: 1000,
                        success: true,
                        error: None,
                    },
                    ToolCall {
                        id: "tc-002".to_string(),
                        tool: "write_file".to_string(),
                        params: serde_json::json!({"path": "/tmp/test", "content": "hello"}),
                        result: serde_json::json!({"success": true}),
                        started_at: "2024-01-01T00:00:02Z".to_string(),
                        completed_at: "2024-01-01T00:00:03Z".to_string(),
                        duration_ms: 500,
                        success: true,
                        error: None,
                    },
                ],
            )
            .with_tool_calls(
                "parent-agent",
                vec![ToolCall {
                    id: "tc-parent-001".to_string(),
                    tool: "search".to_string(),
                    params: serde_json::json!({"query": "test"}),
                    result: serde_json::json!({"results": []}),
                    started_at: "2024-01-01T00:00:00Z".to_string(),
                    completed_at: "2024-01-01T00:00:05Z".to_string(),
                    duration_ms: 5000,
                    success: true,
                    error: None,
                }],
            )
            .with_messages(
                "agent-1",
                vec![
                    Message {
                        role: "user".to_string(),
                        content: "Hello, agent!".to_string(),
                    },
                    Message {
                        role: "assistant".to_string(),
                        content: "Hello! How can I help?".to_string(),
                    },
                ],
            )
    }

    fn create_test_fs(provider: Arc<dyn ContextProvider>, policy: AccessPolicy) -> ContextFs {
        ContextFs::new(provider, policy)
    }

    // ==================== Path Parsing Tests ====================

    #[test]
    fn test_parse_root() {
        let provider = Arc::new(MockContextProvider::new());
        let fs = create_test_fs(provider, AccessPolicy::default());

        let parsed = fs.parse_path("/").unwrap();
        assert!(matches!(parsed, ContextPath::Root));

        let parsed = fs.parse_path("/ctx").unwrap();
        assert!(matches!(parsed, ContextPath::Root));

        let parsed = fs.parse_path("").unwrap();
        assert!(matches!(parsed, ContextPath::Root));
    }

    #[test]
    fn test_parse_self() {
        let provider = Arc::new(MockContextProvider::new());
        let fs = create_test_fs(provider, AccessPolicy::default());

        let parsed = fs.parse_path("/self").unwrap();
        assert!(matches!(parsed, ContextPath::Self_));

        let parsed = fs.parse_path("/ctx/self").unwrap();
        assert!(matches!(parsed, ContextPath::Self_));
    }

    #[test]
    fn test_parse_self_tools() {
        let provider = Arc::new(MockContextProvider::new());
        let fs = create_test_fs(provider, AccessPolicy::default());

        let parsed = fs.parse_path("/self/tools").unwrap();
        assert!(matches!(parsed, ContextPath::SelfTools));

        let parsed = fs.parse_path("/ctx/self/tools").unwrap();
        assert!(matches!(parsed, ContextPath::SelfTools));
    }

    #[test]
    fn test_parse_self_tool_call() {
        let provider = Arc::new(MockContextProvider::new());
        let fs = create_test_fs(provider, AccessPolicy::default());

        let parsed = fs.parse_path("/self/tools/tc-001").unwrap();
        if let ContextPath::SelfToolCall { id } = parsed {
            assert_eq!(id, "tc-001");
        } else {
            panic!("expected SelfToolCall");
        }
    }

    #[test]
    fn test_parse_self_tool_call_file() {
        let provider = Arc::new(MockContextProvider::new());
        let fs = create_test_fs(provider, AccessPolicy::default());

        let parsed = fs.parse_path("/self/tools/tc-001/request.json").unwrap();
        if let ContextPath::SelfToolCallFile { id, file } = parsed {
            assert_eq!(id, "tc-001");
            assert_eq!(file, "request.json");
        } else {
            panic!("expected SelfToolCallFile");
        }

        let parsed = fs.parse_path("/self/tools/tc-001/result.json").unwrap();
        if let ContextPath::SelfToolCallFile { id, file } = parsed {
            assert_eq!(id, "tc-001");
            assert_eq!(file, "result.json");
        } else {
            panic!("expected SelfToolCallFile");
        }
    }

    #[test]
    fn test_parse_self_messages() {
        let provider = Arc::new(MockContextProvider::new());
        let fs = create_test_fs(provider, AccessPolicy::default());

        let parsed = fs.parse_path("/self/messages").unwrap();
        assert!(matches!(parsed, ContextPath::SelfMessages));
    }

    #[test]
    fn test_parse_self_message_index() {
        let provider = Arc::new(MockContextProvider::new());
        let fs = create_test_fs(provider, AccessPolicy::default());

        let parsed = fs.parse_path("/self/messages/0").unwrap();
        if let ContextPath::SelfMessage { index } = parsed {
            assert_eq!(index, 0);
        } else {
            panic!("expected SelfMessage");
        }

        let parsed = fs.parse_path("/self/messages/5").unwrap();
        if let ContextPath::SelfMessage { index } = parsed {
            assert_eq!(index, 5);
        } else {
            panic!("expected SelfMessage");
        }
    }

    #[test]
    fn test_parse_self_scratch() {
        let provider = Arc::new(MockContextProvider::new());
        let fs = create_test_fs(provider, AccessPolicy::default());

        let parsed = fs.parse_path("/self/scratch").unwrap();
        assert!(matches!(parsed, ContextPath::SelfScratch));
    }

    #[test]
    fn test_parse_self_scratch_file() {
        let provider = Arc::new(MockContextProvider::new());
        let fs = create_test_fs(provider, AccessPolicy::default());

        let parsed = fs.parse_path("/self/scratch/data.json").unwrap();
        if let ContextPath::SelfScratchFile { path } = parsed {
            assert_eq!(path, "data.json");
        } else {
            panic!("expected SelfScratchFile");
        }

        // Nested path
        let parsed = fs.parse_path("/self/scratch/nested/deep/file.txt").unwrap();
        if let ContextPath::SelfScratchFile { path } = parsed {
            assert_eq!(path, "nested/deep/file.txt");
        } else {
            panic!("expected SelfScratchFile");
        }
    }

    #[test]
    fn test_parse_parent_tools() {
        let provider = Arc::new(MockContextProvider::new());
        let fs = create_test_fs(provider, AccessPolicy::default());

        let parsed = fs.parse_path("/parent/tools").unwrap();
        assert!(matches!(parsed, ContextPath::ParentTools));
    }

    #[test]
    fn test_parse_children() {
        let provider = Arc::new(MockContextProvider::new());
        let fs = create_test_fs(provider, AccessPolicy::default());

        let parsed = fs.parse_path("/children").unwrap();
        assert!(matches!(parsed, ContextPath::Children));

        let parsed = fs.parse_path("/children/child-1").unwrap();
        if let ContextPath::Child { id } = parsed {
            assert_eq!(id, "child-1");
        } else {
            panic!("expected Child");
        }
    }

    #[test]
    fn test_parse_shared() {
        let provider = Arc::new(MockContextProvider::new());
        let fs = create_test_fs(provider, AccessPolicy::default());

        let parsed = fs.parse_path("/shared").unwrap();
        assert!(matches!(parsed, ContextPath::Shared));

        let parsed = fs.parse_path("/shared/investigation-123").unwrap();
        if let ContextPath::SharedInvestigation { id } = parsed {
            assert_eq!(id, "investigation-123");
        } else {
            panic!("expected SharedInvestigation");
        }
    }

    #[test]
    fn test_parse_invalid_path() {
        let provider = Arc::new(MockContextProvider::new());
        let fs = create_test_fs(provider, AccessPolicy::default());

        // Invalid message index
        let result = fs.parse_path("/self/messages/not-a-number");
        assert!(matches!(result, Err(FsError::InvalidPath(_))));
    }

    #[test]
    fn test_parse_with_ctx_prefix() {
        let provider = Arc::new(MockContextProvider::new());
        let fs = create_test_fs(provider, AccessPolicy::default());

        let parsed = fs.parse_path("/ctx/self/tools").unwrap();
        assert!(matches!(parsed, ContextPath::SelfTools));

        let parsed = fs.parse_path("ctx/self/tools").unwrap();
        assert!(matches!(parsed, ContextPath::SelfTools));
    }

    #[test]
    fn test_parse_without_ctx_prefix() {
        let provider = Arc::new(MockContextProvider::new());
        let fs = create_test_fs(provider, AccessPolicy::default());

        let parsed = fs.parse_path("/self/tools").unwrap();
        assert!(matches!(parsed, ContextPath::SelfTools));

        let parsed = fs.parse_path("self/tools").unwrap();
        assert!(matches!(parsed, ContextPath::SelfTools));
    }

    // ==================== Access Control Tests ====================

    #[test]
    fn test_access_self_always_allowed() {
        let provider = Arc::new(MockContextProvider::new());
        let policy = AccessPolicy {
            agent_id: "agent-1".to_string(),
            ..Default::default()
        };
        let fs = create_test_fs(provider, policy);

        // Self access should always be allowed
        assert!(fs.check_access(&ContextPath::Self_).is_ok());
        assert!(fs.check_access(&ContextPath::SelfTools).is_ok());
        assert!(
            fs.check_access(&ContextPath::SelfToolCall {
                id: "tc-001".to_string()
            })
            .is_ok()
        );
        assert!(fs.check_access(&ContextPath::SelfMessages).is_ok());
        assert!(fs.check_access(&ContextPath::SelfScratch).is_ok());
    }

    #[test]
    fn test_access_parent_denied_when_no_parent() {
        let provider = Arc::new(MockContextProvider::new());
        let policy = AccessPolicy {
            agent_id: "agent-1".to_string(),
            parent_agent_id: None,
            can_read_parent_tools: true,
            ..Default::default()
        };
        let fs = create_test_fs(provider, policy);

        let result = fs.check_access(&ContextPath::Parent);
        assert!(matches!(result, Err(FsError::PermissionDenied(_))));

        let result = fs.check_access(&ContextPath::ParentTools);
        assert!(matches!(result, Err(FsError::PermissionDenied(_))));
    }

    #[test]
    fn test_access_parent_denied_when_not_allowed() {
        let provider = Arc::new(MockContextProvider::new());
        let policy = AccessPolicy {
            agent_id: "agent-1".to_string(),
            parent_agent_id: Some("parent-agent".to_string()),
            can_read_parent_tools: false,
            ..Default::default()
        };
        let fs = create_test_fs(provider, policy);

        let result = fs.check_access(&ContextPath::ParentTools);
        assert!(matches!(result, Err(FsError::PermissionDenied(_))));
    }

    #[test]
    fn test_access_parent_allowed_when_configured() {
        let provider = Arc::new(MockContextProvider::new());
        let policy = AccessPolicy {
            agent_id: "agent-1".to_string(),
            parent_agent_id: Some("parent-agent".to_string()),
            can_read_parent_tools: true,
            ..Default::default()
        };
        let fs = create_test_fs(provider, policy);

        assert!(fs.check_access(&ContextPath::Parent).is_ok());
        assert!(fs.check_access(&ContextPath::ParentTools).is_ok());
        assert!(
            fs.check_access(&ContextPath::ParentToolCall {
                id: "tc-parent-001".to_string()
            })
            .is_ok()
        );
    }

    #[test]
    fn test_access_child_denied_when_not_in_list() {
        let provider = Arc::new(MockContextProvider::new());
        let policy = AccessPolicy {
            agent_id: "agent-1".to_string(),
            child_agent_ids: vec!["child-1".to_string(), "child-2".to_string()],
            ..Default::default()
        };
        let fs = create_test_fs(provider, policy);

        let result = fs.check_access(&ContextPath::Child {
            id: "child-3".to_string(),
        });
        assert!(matches!(result, Err(FsError::PermissionDenied(_))));
    }

    #[test]
    fn test_access_child_allowed_when_in_list() {
        let provider = Arc::new(MockContextProvider::new());
        let policy = AccessPolicy {
            agent_id: "agent-1".to_string(),
            child_agent_ids: vec!["child-1".to_string(), "child-2".to_string()],
            ..Default::default()
        };
        let fs = create_test_fs(provider, policy);

        assert!(
            fs.check_access(&ContextPath::Child {
                id: "child-1".to_string()
            })
            .is_ok()
        );
        assert!(
            fs.check_access(&ContextPath::Child {
                id: "child-2".to_string()
            })
            .is_ok()
        );
    }

    #[test]
    fn test_access_shared_investigation() {
        let provider = Arc::new(MockContextProvider::new());
        let policy = AccessPolicy {
            agent_id: "agent-1".to_string(),
            investigation_id: Some("inv-123".to_string()),
            ..Default::default()
        };
        let fs = create_test_fs(provider, policy);

        // Shared access should be allowed (no explicit check in current implementation)
        assert!(fs.check_access(&ContextPath::Shared).is_ok());
        assert!(
            fs.check_access(&ContextPath::SharedInvestigation {
                id: "inv-123".to_string()
            })
            .is_ok()
        );
    }

    // ==================== Read Operations Tests ====================

    #[tokio::test]
    async fn test_read_self_meta() {
        let provider = Arc::new(create_test_provider());
        let policy = AccessPolicy {
            agent_id: "agent-1".to_string(),
            parent_agent_id: Some("parent-agent".to_string()),
            child_agent_ids: vec!["child-1".to_string()],
            ..Default::default()
        };
        let fs = create_test_fs(provider, policy);

        let content = fs.read("/self/meta.json").await.unwrap();
        let meta: serde_json::Value = serde_json::from_slice(&content).unwrap();

        assert_eq!(meta["agent_id"], "agent-1");
        assert_eq!(meta["parent_agent_id"], "parent-agent");
        assert!(
            meta["child_agent_ids"]
                .as_array()
                .unwrap()
                .contains(&serde_json::json!("child-1"))
        );
    }

    #[tokio::test]
    async fn test_read_tool_call_request() {
        let provider = Arc::new(create_test_provider());
        let policy = AccessPolicy {
            agent_id: "agent-1".to_string(),
            ..Default::default()
        };
        let fs = create_test_fs(provider, policy);

        let content = fs.read("/self/tools/tc-001/request.json").await.unwrap();
        let request: serde_json::Value = serde_json::from_slice(&content).unwrap();

        assert_eq!(request["tool"], "read_file");
        assert_eq!(request["params"]["path"], "/etc/hosts");
    }

    #[tokio::test]
    async fn test_read_tool_call_result() {
        let provider = Arc::new(create_test_provider());
        let policy = AccessPolicy {
            agent_id: "agent-1".to_string(),
            ..Default::default()
        };
        let fs = create_test_fs(provider, policy);

        let content = fs.read("/self/tools/tc-001/result.json").await.unwrap();
        let result: serde_json::Value = serde_json::from_slice(&content).unwrap();

        assert_eq!(result["content"], "127.0.0.1 localhost");
    }

    #[tokio::test]
    async fn test_read_tool_call_meta() {
        let provider = Arc::new(create_test_provider());
        let policy = AccessPolicy {
            agent_id: "agent-1".to_string(),
            ..Default::default()
        };
        let fs = create_test_fs(provider, policy);

        let content = fs.read("/self/tools/tc-001/meta.json").await.unwrap();
        let meta: serde_json::Value = serde_json::from_slice(&content).unwrap();

        assert_eq!(meta["id"], "tc-001");
        assert_eq!(meta["tool"], "read_file");
        assert_eq!(meta["duration_ms"], 1000);
        assert_eq!(meta["success"], true);
    }

    #[tokio::test]
    async fn test_read_message() {
        let provider = Arc::new(create_test_provider());
        let policy = AccessPolicy {
            agent_id: "agent-1".to_string(),
            ..Default::default()
        };
        let fs = create_test_fs(provider, policy);

        let content = fs.read("/self/messages/0").await.unwrap();
        let text = String::from_utf8_lossy(&content);
        assert_eq!(text, "Hello, agent!");

        let content = fs.read("/self/messages/1").await.unwrap();
        let text = String::from_utf8_lossy(&content);
        assert_eq!(text, "Hello! How can I help?");
    }

    #[tokio::test]
    async fn test_read_scratch_file() {
        let provider = Arc::new(create_test_provider());
        let policy = AccessPolicy {
            agent_id: "agent-1".to_string(),
            ..Default::default()
        };

        // Write some data to scratch first
        provider
            .write_scratch("agent-1", "test.txt", b"scratch data")
            .await
            .unwrap();

        let fs = create_test_fs(provider, policy);

        let content = fs.read("/self/scratch/test.txt").await.unwrap();
        assert_eq!(content, b"scratch data");
    }

    #[tokio::test]
    async fn test_read_nonexistent_file() {
        let provider = Arc::new(create_test_provider());
        let policy = AccessPolicy {
            agent_id: "agent-1".to_string(),
            ..Default::default()
        };
        let fs = create_test_fs(provider, policy);

        let result = fs.read("/self/tools/nonexistent/request.json").await;
        assert!(matches!(result, Err(FsError::NotFound(_))));

        let result = fs.read("/self/messages/999").await;
        assert!(matches!(result, Err(FsError::NotFound(_))));
    }

    #[tokio::test]
    async fn test_read_directory_fails() {
        let provider = Arc::new(create_test_provider());
        let policy = AccessPolicy {
            agent_id: "agent-1".to_string(),
            ..Default::default()
        };
        let fs = create_test_fs(provider, policy);

        let result = fs.read("/self/tools").await;
        assert!(matches!(result, Err(FsError::NotAFile(_))));

        let result = fs.read("/self").await;
        assert!(matches!(result, Err(FsError::NotAFile(_))));
    }

    // ==================== Directory Listing Tests ====================

    #[tokio::test]
    async fn test_readdir_root() {
        let provider = Arc::new(create_test_provider());
        let fs = create_test_fs(provider, AccessPolicy::default());

        let entries = fs.read_dir("/").await.unwrap();
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();

        assert!(names.contains(&"self"));
        assert!(names.contains(&"parent"));
        assert!(names.contains(&"children"));
        assert!(names.contains(&"shared"));

        // All should be directories
        assert!(entries.iter().all(|e| e.is_dir));
    }

    #[tokio::test]
    async fn test_readdir_self() {
        let provider = Arc::new(create_test_provider());
        let fs = create_test_fs(provider, AccessPolicy::default());

        let entries = fs.read_dir("/self").await.unwrap();
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();

        assert!(names.contains(&"meta.json"));
        assert!(names.contains(&"tools"));
        assert!(names.contains(&"messages"));
        assert!(names.contains(&"scratch"));
        assert!(names.contains(&"artifacts"));

        // Check that meta.json is a file
        let meta = entries.iter().find(|e| e.name == "meta.json").unwrap();
        assert!(!meta.is_dir);

        // Check that tools is a directory
        let tools = entries.iter().find(|e| e.name == "tools").unwrap();
        assert!(tools.is_dir);
    }

    #[tokio::test]
    async fn test_readdir_self_tools() {
        let provider = Arc::new(create_test_provider());
        let policy = AccessPolicy {
            agent_id: "agent-1".to_string(),
            ..Default::default()
        };
        let fs = create_test_fs(provider, policy);

        let entries = fs.read_dir("/self/tools").await.unwrap();
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();

        assert!(names.contains(&"tc-001"));
        assert!(names.contains(&"tc-002"));
        assert!(names.contains(&"by-name")); // Virtual directory

        // Tool call directories should be directories
        let tc001 = entries.iter().find(|e| e.name == "tc-001").unwrap();
        assert!(tc001.is_dir);
    }

    #[tokio::test]
    async fn test_readdir_self_tool_call() {
        let provider = Arc::new(create_test_provider());
        let policy = AccessPolicy {
            agent_id: "agent-1".to_string(),
            ..Default::default()
        };
        let fs = create_test_fs(provider, policy);

        let entries = fs.read_dir("/self/tools/tc-001").await.unwrap();
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();

        assert!(names.contains(&"request.json"));
        assert!(names.contains(&"result.json"));
        assert!(names.contains(&"meta.json"));

        // All should be files
        assert!(entries.iter().all(|e| !e.is_dir));
    }

    #[tokio::test]
    async fn test_readdir_parent_tools() {
        let provider = Arc::new(create_test_provider());
        let policy = AccessPolicy {
            agent_id: "agent-1".to_string(),
            parent_agent_id: Some("parent-agent".to_string()),
            can_read_parent_tools: true,
            ..Default::default()
        };
        let fs = create_test_fs(provider, policy);

        let entries = fs.read_dir("/parent/tools").await.unwrap();
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();

        assert!(names.contains(&"tc-parent-001"));
    }

    #[tokio::test]
    async fn test_readdir_children() {
        let provider = Arc::new(create_test_provider());
        let policy = AccessPolicy {
            agent_id: "agent-1".to_string(),
            child_agent_ids: vec!["child-1".to_string(), "child-2".to_string()],
            ..Default::default()
        };
        let fs = create_test_fs(provider, policy);

        let entries = fs.read_dir("/children").await.unwrap();
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();

        assert!(names.contains(&"child-1"));
        assert!(names.contains(&"child-2"));
        assert!(entries.iter().all(|e| e.is_dir));
    }

    #[tokio::test]
    async fn test_readdir_file_fails() {
        let provider = Arc::new(create_test_provider());
        let policy = AccessPolicy {
            agent_id: "agent-1".to_string(),
            ..Default::default()
        };
        let fs = create_test_fs(provider, policy);

        let result = fs.read_dir("/self/meta.json").await;
        assert!(matches!(result, Err(FsError::NotADirectory(_))));
    }

    // ==================== Stat/Exists Tests ====================

    #[tokio::test]
    async fn test_stat_directory() {
        let provider = Arc::new(create_test_provider());
        let fs = create_test_fs(provider, AccessPolicy::default());

        let meta = fs.stat("/self").await.unwrap();
        assert!(meta.is_dir);
        assert!(!meta.is_file);

        let meta = fs.stat("/self/tools").await.unwrap();
        assert!(meta.is_dir);

        let meta = fs.stat("/self/tools/tc-001").await.unwrap();
        assert!(meta.is_dir);
    }

    #[tokio::test]
    async fn test_stat_file() {
        let provider = Arc::new(create_test_provider());
        let policy = AccessPolicy {
            agent_id: "agent-1".to_string(),
            ..Default::default()
        };
        let fs = create_test_fs(provider, policy);

        let meta = fs.stat("/self/meta.json").await.unwrap();
        assert!(meta.is_file);
        assert!(!meta.is_dir);

        let meta = fs.stat("/self/tools/tc-001/request.json").await.unwrap();
        assert!(meta.is_file);
    }

    #[tokio::test]
    async fn test_exists_true() {
        let provider = Arc::new(create_test_provider());
        let fs = create_test_fs(provider, AccessPolicy::default());

        assert!(fs.exists("/self").await.unwrap());
        assert!(fs.exists("/self/tools").await.unwrap());
        assert!(fs.exists("/ctx/self").await.unwrap());
    }

    #[tokio::test]
    async fn test_exists_false() {
        let provider = Arc::new(create_test_provider());
        let fs = create_test_fs(provider, AccessPolicy::default());

        // Note: The exists check goes through parse_path and stat,
        // which may not fail for unknown paths. Let's test paths
        // that would cause actual NotFound errors
        let result = fs.exists("/nonexistent/path").await;
        // This might return false or error depending on implementation
        match result {
            Ok(false) => {}                 // Expected
            Err(FsError::NotFound(_)) => {} // Also acceptable
            other => panic!("unexpected result: {:?}", other),
        }
    }

    // ==================== MockContextProvider Tests ====================

    #[tokio::test]
    async fn test_mock_get_tool_call() {
        let provider = create_test_provider();

        let tc = provider.get_tool_call("agent-1", "tc-001").await.unwrap();
        assert_eq!(tc.id, "tc-001");
        assert_eq!(tc.tool, "read_file");

        let result = provider.get_tool_call("agent-1", "nonexistent").await;
        assert!(matches!(result, Err(FsError::NotFound(_))));
    }

    #[tokio::test]
    async fn test_mock_list_tool_calls() {
        let provider = create_test_provider();

        let calls = provider.list_tool_calls("agent-1").await.unwrap();
        assert_eq!(calls.len(), 2);
        assert!(calls.iter().any(|tc| tc.id == "tc-001"));
        assert!(calls.iter().any(|tc| tc.id == "tc-002"));

        let calls = provider.list_tool_calls("nonexistent").await.unwrap();
        assert!(calls.is_empty());
    }

    #[tokio::test]
    async fn test_mock_get_message() {
        let provider = create_test_provider();

        let msg = provider.get_message("agent-1", 0).await.unwrap();
        assert_eq!(msg.role, "user");
        assert_eq!(msg.content, "Hello, agent!");

        let result = provider.get_message("agent-1", 999).await;
        assert!(matches!(result, Err(FsError::NotFound(_))));
    }

    #[tokio::test]
    async fn test_mock_message_count() {
        let provider = create_test_provider();

        let count = provider.message_count("agent-1").await.unwrap();
        assert_eq!(count, 2);

        let count = provider.message_count("nonexistent").await.unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn test_mock_scratch_read_write() {
        let provider = MockContextProvider::new();

        // Write
        provider
            .write_scratch("agent-1", "test.txt", b"hello world")
            .await
            .unwrap();

        // Read back
        let data = provider.read_scratch("agent-1", "test.txt").await.unwrap();
        assert_eq!(data, b"hello world");

        // Overwrite
        provider
            .write_scratch("agent-1", "test.txt", b"updated")
            .await
            .unwrap();
        let data = provider.read_scratch("agent-1", "test.txt").await.unwrap();
        assert_eq!(data, b"updated");

        // Read nonexistent
        let result = provider.read_scratch("agent-1", "nonexistent").await;
        assert!(matches!(result, Err(FsError::NotFound(_))));
    }

    #[tokio::test]
    async fn test_mock_publish_artifact() {
        let provider = MockContextProvider::new();

        // Should succeed
        let result = provider
            .publish_artifact("agent-1", "report.json", b"{}")
            .await;
        assert!(result.is_ok());
    }

    // ==================== Tool Call Caching Tests ====================

    #[tokio::test]
    async fn test_tool_call_caching() {
        let provider = Arc::new(create_test_provider());
        let policy = AccessPolicy {
            agent_id: "agent-1".to_string(),
            ..Default::default()
        };
        let fs = create_test_fs(provider, policy);

        // First read - fetches from provider
        let content1 = fs.read("/self/tools/tc-001/request.json").await.unwrap();

        // Second read - should use cache
        let content2 = fs.read("/self/tools/tc-001/result.json").await.unwrap();

        // Verify we got valid content both times
        let request: serde_json::Value = serde_json::from_slice(&content1).unwrap();
        let result: serde_json::Value = serde_json::from_slice(&content2).unwrap();

        assert_eq!(request["tool"], "read_file");
        assert_eq!(result["content"], "127.0.0.1 localhost");

        // Check cache has the entry
        let cache = fs.cache.read().await;
        assert!(cache.contains_key(&("agent-1".to_string(), "tc-001".to_string())));
    }

    // ==================== Parent Tool Call Access Tests ====================

    #[tokio::test]
    async fn test_read_parent_tool_call() {
        let provider = Arc::new(create_test_provider());
        let policy = AccessPolicy {
            agent_id: "agent-1".to_string(),
            parent_agent_id: Some("parent-agent".to_string()),
            can_read_parent_tools: true,
            ..Default::default()
        };
        let fs = create_test_fs(provider, policy);

        let content = fs
            .read("/parent/tools/tc-parent-001/result.json")
            .await
            .unwrap();
        let result: serde_json::Value = serde_json::from_slice(&content).unwrap();

        assert!(result["results"].is_array());
    }

    #[tokio::test]
    async fn test_read_parent_tool_call_denied() {
        let provider = Arc::new(create_test_provider());
        let policy = AccessPolicy {
            agent_id: "agent-1".to_string(),
            parent_agent_id: Some("parent-agent".to_string()),
            can_read_parent_tools: false,
            ..Default::default()
        };
        let fs = create_test_fs(provider, policy);

        let result = fs.read("/parent/tools/tc-parent-001/result.json").await;
        assert!(matches!(result, Err(FsError::PermissionDenied(_))));
    }
}
