//! Conversation history provider for agent context.
//!
//! This module provides the [`HistoryProvider`] trait for exposing conversation
//! history to agents via the VFS at `/history/`.
//!
//! # Transcript Format
//!
//! The transcript uses a compact markdown-compatible format:
//!
//! ```text
//! U> User message here
//!
//! A> Assistant response here
//!
//! T[web_search] {"query": "rust async"}
//! R> {"results": [...]}
//!
//! A> Based on the search results...
//! ```
//!
//! - `U>` - User message
//! - `A>` - Assistant message
//! - `T[tool_name]` - Tool invocation with JSON params
//! - `R>` - Tool result (JSON)
//!
//! # VFS Structure
//!
//! ```text
//! /history/
//! ├── index.txt           # List of available conversations
//! ├── current/
//! │   ├── transcript.md   # Current conversation transcript
//! │   └── metadata.json   # Conversation metadata
//! └── <conv_id>/          # Historical conversations (optional)
//!     ├── transcript.md
//!     └── metadata.json
//! ```

use serde::{Deserialize, Serialize};

/// Summary of a conversation for the history index.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationSummary {
    /// Unique conversation ID.
    pub id: String,
    /// Short title or first line of the conversation.
    pub title: String,
    /// When the conversation started (RFC3339).
    pub started_at: String,
    /// Number of messages in the conversation.
    pub message_count: usize,
    /// Whether this is the current/active conversation.
    #[serde(default)]
    pub is_current: bool,
}

/// Metadata about a conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationMetadata {
    /// Unique conversation ID.
    pub id: String,
    /// Short title or summary.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// When the conversation started (RFC3339).
    pub started_at: String,
    /// When the conversation was last updated (RFC3339).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    /// Number of user messages.
    pub user_message_count: usize,
    /// Number of assistant messages.
    pub assistant_message_count: usize,
    /// Number of tool calls.
    pub tool_call_count: usize,
}

/// Provider for conversation history data.
///
/// Implementations of this trait supply conversation history that agents
/// can access via the `/history/` directory in the VFS.
///
/// # Example
///
/// ```rust,ignore
/// use conch::agent::{HistoryProvider, ConversationSummary};
///
/// struct MyHistoryProvider {
///     transcript: String,
/// }
///
/// impl HistoryProvider for MyHistoryProvider {
///     fn current_transcript(&self) -> Option<String> {
///         Some(self.transcript.clone())
///     }
///
///     fn current_metadata(&self) -> Option<ConversationMetadata> {
///         None
///     }
///
///     fn list_conversations(&self) -> Vec<ConversationSummary> {
///         vec![]
///     }
///
///     fn get_transcript(&self, _id: &str) -> Option<String> {
///         None
///     }
///
///     fn get_metadata(&self, _id: &str) -> Option<ConversationMetadata> {
///         None
///     }
/// }
/// ```
pub trait HistoryProvider: Send + Sync {
    /// Get the transcript for the current conversation.
    ///
    /// Returns `None` if there is no current conversation.
    fn current_transcript(&self) -> Option<String>;

    /// Get metadata for the current conversation.
    fn current_metadata(&self) -> Option<ConversationMetadata>;

    /// List available conversations (current and historical).
    fn list_conversations(&self) -> Vec<ConversationSummary>;

    /// Get the transcript for a specific conversation by ID.
    fn get_transcript(&self, id: &str) -> Option<String>;

    /// Get metadata for a specific conversation by ID.
    fn get_metadata(&self, id: &str) -> Option<ConversationMetadata>;
}

/// A simple in-memory history provider for testing.
///
/// # Example
///
/// ```rust
/// use conch::agent::SimpleHistoryProvider;
///
/// let history = SimpleHistoryProvider::new()
///     .with_transcript("U> Hello\n\nA> Hi there!");
///
/// assert!(history.current_transcript().unwrap().contains("Hello"));
/// ```
#[derive(Debug, Clone, Default)]
pub struct SimpleHistoryProvider {
    transcript: Option<String>,
    metadata: Option<ConversationMetadata>,
    conversations: Vec<(ConversationSummary, String, Option<ConversationMetadata>)>,
}

impl SimpleHistoryProvider {
    /// Create a new empty history provider.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the current conversation transcript.
    pub fn with_transcript(mut self, transcript: impl Into<String>) -> Self {
        self.transcript = Some(transcript.into());
        self
    }

    /// Set the current conversation metadata.
    pub fn with_metadata(mut self, metadata: ConversationMetadata) -> Self {
        self.metadata = Some(metadata);
        self
    }

    /// Add a historical conversation.
    pub fn with_conversation(
        mut self,
        summary: ConversationSummary,
        transcript: impl Into<String>,
        metadata: Option<ConversationMetadata>,
    ) -> Self {
        self.conversations
            .push((summary, transcript.into(), metadata));
        self
    }
}

impl HistoryProvider for SimpleHistoryProvider {
    fn current_transcript(&self) -> Option<String> {
        self.transcript.clone()
    }

    fn current_metadata(&self) -> Option<ConversationMetadata> {
        self.metadata.clone()
    }

    fn list_conversations(&self) -> Vec<ConversationSummary> {
        let mut summaries: Vec<ConversationSummary> = self
            .conversations
            .iter()
            .map(|(s, _, _)| s.clone())
            .collect();

        // Add current conversation if it exists
        if self.transcript.is_some() {
            let current = ConversationSummary {
                id: "current".to_string(),
                title: self
                    .metadata
                    .as_ref()
                    .and_then(|m| m.title.clone())
                    .unwrap_or_else(|| "Current conversation".to_string()),
                started_at: self
                    .metadata
                    .as_ref()
                    .map(|m| m.started_at.clone())
                    .unwrap_or_default(),
                message_count: self
                    .metadata
                    .as_ref()
                    .map(|m| m.user_message_count + m.assistant_message_count)
                    .unwrap_or(0),
                is_current: true,
            };
            summaries.insert(0, current);
        }

        summaries
    }

    fn get_transcript(&self, id: &str) -> Option<String> {
        if id == "current" {
            return self.transcript.clone();
        }
        self.conversations
            .iter()
            .find(|(s, _, _)| s.id == id)
            .map(|(_, t, _)| t.clone())
    }

    fn get_metadata(&self, id: &str) -> Option<ConversationMetadata> {
        if id == "current" {
            return self.metadata.clone();
        }
        self.conversations
            .iter()
            .find(|(s, _, _)| s.id == id)
            .and_then(|(_, _, m)| m.clone())
    }
}

/// Generate the index.txt content for a list of conversations.
pub fn generate_history_index(conversations: &[ConversationSummary]) -> String {
    if conversations.is_empty() {
        return String::from("# No conversation history available\n");
    }

    let mut output = String::from("# Conversation History\n\n");

    for conv in conversations {
        let current_marker = if conv.is_current { " (current)" } else { "" };
        output.push_str(&format!(
            "{}{} - {} ({} messages)\n",
            conv.id, current_marker, conv.title, conv.message_count
        ));
    }

    output
}

/// Parse a transcript to extract basic statistics.
pub fn parse_transcript_stats(transcript: &str) -> (usize, usize, usize) {
    let mut user_count = 0;
    let mut assistant_count = 0;
    let mut tool_count = 0;

    for line in transcript.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("U>") {
            user_count += 1;
        } else if trimmed.starts_with("A>") {
            assistant_count += 1;
        } else if trimmed.starts_with("T[") {
            tool_count += 1;
        }
    }

    (user_count, assistant_count, tool_count)
}

/// Format a user message for the transcript.
pub fn format_user_message(message: &str) -> String {
    let mut output = String::from("U> ");
    for (i, line) in message.lines().enumerate() {
        if i > 0 {
            output.push_str("\n   ");
        }
        output.push_str(line);
    }
    output.push_str("\n\n");
    output
}

/// Format an assistant message for the transcript.
pub fn format_assistant_message(message: &str) -> String {
    let mut output = String::from("A> ");
    for (i, line) in message.lines().enumerate() {
        if i > 0 {
            output.push_str("\n   ");
        }
        output.push_str(line);
    }
    output.push_str("\n\n");
    output
}

/// Format a tool call for the transcript.
pub fn format_tool_call(tool_name: &str, params: &serde_json::Value) -> String {
    let params_str = serde_json::to_string(params).unwrap_or_else(|_| "{}".to_string());
    format!("T[{tool_name}] {params_str}\n")
}

/// Format a tool result for the transcript.
pub fn format_tool_result(result: &serde_json::Value) -> String {
    let result_str = serde_json::to_string_pretty(result).unwrap_or_else(|_| "null".to_string());
    // Indent multi-line results
    let indented: String = result_str
        .lines()
        .enumerate()
        .map(|(i, line)| {
            if i == 0 {
                format!("R> {line}")
            } else {
                format!("   {line}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!("{indented}\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_history_provider_empty() {
        let provider = SimpleHistoryProvider::new();
        assert!(provider.current_transcript().is_none());
        assert!(provider.list_conversations().is_empty());
    }

    #[test]
    fn test_simple_history_provider_with_transcript() {
        let provider = SimpleHistoryProvider::new().with_transcript("U> Hello\n\nA> Hi!");

        let transcript = provider.current_transcript().unwrap();
        assert!(transcript.contains("Hello"));
        assert!(transcript.contains("Hi!"));
    }

    #[test]
    fn test_simple_history_provider_list_conversations() {
        let provider = SimpleHistoryProvider::new()
            .with_transcript("U> Test")
            .with_metadata(ConversationMetadata {
                id: "current".to_string(),
                title: Some("Test conv".to_string()),
                started_at: "2024-01-01T00:00:00Z".to_string(),
                updated_at: None,
                user_message_count: 1,
                assistant_message_count: 0,
                tool_call_count: 0,
            });

        let convs = provider.list_conversations();
        assert_eq!(convs.len(), 1);
        assert!(convs[0].is_current);
        assert_eq!(convs[0].title, "Test conv");
    }

    #[test]
    fn test_generate_history_index_empty() {
        let index = generate_history_index(&[]);
        assert!(index.contains("No conversation history"));
    }

    #[test]
    fn test_generate_history_index() {
        let convs = vec![
            ConversationSummary {
                id: "current".to_string(),
                title: "Current task".to_string(),
                started_at: "2024-01-01T00:00:00Z".to_string(),
                message_count: 5,
                is_current: true,
            },
            ConversationSummary {
                id: "conv-001".to_string(),
                title: "Previous task".to_string(),
                started_at: "2023-12-31T00:00:00Z".to_string(),
                message_count: 10,
                is_current: false,
            },
        ];

        let index = generate_history_index(&convs);
        assert!(index.contains("current (current)"));
        assert!(index.contains("conv-001"));
        assert!(index.contains("5 messages"));
        assert!(index.contains("10 messages"));
    }

    #[test]
    fn test_parse_transcript_stats() {
        let transcript = r#"
U> Hello

A> Hi there!

T[web_search] {"query": "test"}
R> {"results": []}

A> Here are the results

U> Thanks

A> You're welcome
"#;
        let (user, assistant, tool) = parse_transcript_stats(transcript);
        assert_eq!(user, 2);
        assert_eq!(assistant, 3);
        assert_eq!(tool, 1);
    }

    #[test]
    fn test_format_user_message() {
        let msg = format_user_message("Hello world");
        assert_eq!(msg, "U> Hello world\n\n");
    }

    #[test]
    fn test_format_user_message_multiline() {
        let msg = format_user_message("Line 1\nLine 2");
        assert_eq!(msg, "U> Line 1\n   Line 2\n\n");
    }

    #[test]
    fn test_format_assistant_message() {
        let msg = format_assistant_message("Hello!");
        assert_eq!(msg, "A> Hello!\n\n");
    }

    #[test]
    fn test_format_tool_call() {
        let params = serde_json::json!({"query": "rust"});
        let formatted = format_tool_call("web_search", &params);
        assert!(formatted.starts_with("T[web_search]"));
        assert!(formatted.contains("query"));
    }

    #[test]
    fn test_format_tool_result() {
        let result = serde_json::json!({"status": "ok"});
        let formatted = format_tool_result(&result);
        assert!(formatted.starts_with("R>"));
        assert!(formatted.contains("status"));
    }

    #[test]
    fn test_get_historical_transcript() {
        let provider = SimpleHistoryProvider::new()
            .with_transcript("U> Current")
            .with_conversation(
                ConversationSummary {
                    id: "old-001".to_string(),
                    title: "Old conversation".to_string(),
                    started_at: "2024-01-01T00:00:00Z".to_string(),
                    message_count: 3,
                    is_current: false,
                },
                "U> Old message\n\nA> Old response",
                None,
            );

        // Get current
        let current = provider.get_transcript("current").unwrap();
        assert!(current.contains("Current"));

        // Get historical
        let old = provider.get_transcript("old-001").unwrap();
        assert!(old.contains("Old message"));

        // Non-existent
        assert!(provider.get_transcript("nonexistent").is_none());
    }
}
