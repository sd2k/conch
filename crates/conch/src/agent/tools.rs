//! Tool definitions and registry for agent sandboxes.
//!
//! Tools are external capabilities that agents can invoke via the `tool` builtin.
//! This module provides:
//!
//! - [`ToolDefinition`] - A tool's schema (name, description, JSON Schema parameters)
//! - [`ToolRegistry`] - Trait for listing and retrieving tool definitions
//! - [`VecToolRegistry`] - Simple in-memory implementation

use serde::{Deserialize, Serialize};

/// Summary of a tool for index listings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSummary {
    /// Tool name (unique identifier).
    pub name: String,
    /// One-line description for index.txt.
    pub description: String,
}

/// Full definition of a tool including its parameter schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    /// Tool name (unique identifier).
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// JSON Schema for the tool's parameters.
    #[serde(default)]
    pub parameters: serde_json::Value,
}

impl ToolDefinition {
    /// Create a new tool definition.
    ///
    /// # Example
    ///
    /// ```rust
    /// use conch::agent::ToolDefinition;
    /// use serde_json::json;
    ///
    /// let tool = ToolDefinition::new(
    ///     "web_search",
    ///     "Search the web for information",
    ///     json!({
    ///         "type": "object",
    ///         "properties": {
    ///             "query": {
    ///                 "type": "string",
    ///                 "description": "Search query"
    ///             }
    ///         },
    ///         "required": ["query"]
    ///     }),
    /// );
    /// ```
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        parameters: serde_json::Value,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            parameters,
        }
    }

    /// Create a tool definition with no parameters.
    pub fn no_params(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        }
    }

    /// Get a summary of this tool.
    pub fn summary(&self) -> ToolSummary {
        ToolSummary {
            name: self.name.clone(),
            description: self.description.clone(),
        }
    }
}

/// Trait for providing tool definitions to an agent sandbox.
///
/// Implementations must be thread-safe for use across async tasks.
pub trait ToolRegistry: Send + Sync {
    /// List all available tools (name and description only).
    fn list_tools(&self) -> Vec<ToolSummary>;

    /// Get the full definition of a specific tool.
    fn get_tool(&self, name: &str) -> Option<ToolDefinition>;

    /// Check if a tool exists.
    fn has_tool(&self, name: &str) -> bool {
        self.get_tool(name).is_some()
    }
}

/// Simple in-memory tool registry backed by a Vec.
#[derive(Debug, Default, Clone)]
pub struct VecToolRegistry {
    tools: Vec<ToolDefinition>,
}

impl VecToolRegistry {
    /// Create a new empty tool registry.
    pub fn new() -> Self {
        Self { tools: Vec::new() }
    }

    /// Create a tool registry from a collection of tool definitions.
    pub fn with_tools(tools: impl IntoIterator<Item = ToolDefinition>) -> Self {
        Self {
            tools: tools.into_iter().collect(),
        }
    }

    /// Add a tool to the registry.
    pub fn add(&mut self, tool: ToolDefinition) {
        self.tools.push(tool);
    }

    /// Add multiple tools to the registry.
    pub fn extend(&mut self, tools: impl IntoIterator<Item = ToolDefinition>) {
        self.tools.extend(tools);
    }
}

impl ToolRegistry for VecToolRegistry {
    fn list_tools(&self) -> Vec<ToolSummary> {
        self.tools.iter().map(ToolDefinition::summary).collect()
    }

    fn get_tool(&self, name: &str) -> Option<ToolDefinition> {
        self.tools.iter().find(|t| t.name == name).cloned()
    }
}

impl FromIterator<ToolDefinition> for VecToolRegistry {
    fn from_iter<I: IntoIterator<Item = ToolDefinition>>(iter: I) -> Self {
        Self::with_tools(iter)
    }
}

/// Generate the contents of `/tools/index.txt`.
///
/// Format: one tool per line, name and description separated by whitespace.
/// The name is left-padded to align descriptions.
///
/// ```text
/// web_search          Search the web for information
/// code_edit           Edit a file with natural language
/// spawn_agent         Create a sub-agent to handle a task
/// ```
pub fn generate_index_txt(tools: &[ToolSummary]) -> String {
    if tools.is_empty() {
        return String::new();
    }

    // Find max name length for alignment
    let max_name_len = tools.iter().map(|t| t.name.len()).max().unwrap_or(0);
    let padding = max_name_len + 4; // At least 4 spaces between name and description

    let mut output = String::new();
    for tool in tools {
        let spaces = " ".repeat(padding - tool.name.len());
        output.push_str(&tool.name);
        output.push_str(&spaces);
        output.push_str(&tool.description);
        output.push('\n');
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample_tools() -> Vec<ToolDefinition> {
        vec![
            ToolDefinition::new(
                "web_search",
                "Search the web for information",
                json!({
                    "type": "object",
                    "properties": {
                        "query": { "type": "string" }
                    },
                    "required": ["query"]
                }),
            ),
            ToolDefinition::new(
                "code_edit",
                "Edit a file with natural language instructions",
                json!({
                    "type": "object",
                    "properties": {
                        "file": { "type": "string" },
                        "instruction": { "type": "string" }
                    },
                    "required": ["file", "instruction"]
                }),
            ),
            ToolDefinition::no_params("list_files", "List files in a directory"),
        ]
    }

    #[test]
    fn test_tool_definition_new() {
        let tool = ToolDefinition::new("test_tool", "A test tool", json!({"type": "object"}));

        assert_eq!(tool.name, "test_tool");
        assert_eq!(tool.description, "A test tool");
        assert_eq!(tool.parameters, json!({"type": "object"}));
    }

    #[test]
    fn test_tool_definition_no_params() {
        let tool = ToolDefinition::no_params("simple", "A simple tool");

        assert_eq!(tool.name, "simple");
        assert!(tool.parameters["properties"].is_object());
    }

    #[test]
    fn test_tool_summary() {
        let tool = ToolDefinition::new("test", "Description", json!({}));
        let summary = tool.summary();

        assert_eq!(summary.name, "test");
        assert_eq!(summary.description, "Description");
    }

    #[test]
    fn test_vec_tool_registry_new() {
        let registry = VecToolRegistry::new();
        assert!(registry.list_tools().is_empty());
    }

    #[test]
    fn test_vec_tool_registry_with_tools() {
        let registry = VecToolRegistry::with_tools(sample_tools());
        assert_eq!(registry.list_tools().len(), 3);
    }

    #[test]
    fn test_vec_tool_registry_add() {
        let mut registry = VecToolRegistry::new();
        registry.add(ToolDefinition::no_params("tool1", "Tool 1"));
        registry.add(ToolDefinition::no_params("tool2", "Tool 2"));

        assert_eq!(registry.list_tools().len(), 2);
    }

    #[test]
    fn test_vec_tool_registry_get_tool() {
        let registry = VecToolRegistry::with_tools(sample_tools());

        let tool = registry.get_tool("web_search");
        assert!(tool.is_some());
        assert_eq!(tool.as_ref().map(|t| t.name.as_str()), Some("web_search"));

        let missing = registry.get_tool("nonexistent");
        assert!(missing.is_none());
    }

    #[test]
    fn test_vec_tool_registry_has_tool() {
        let registry = VecToolRegistry::with_tools(sample_tools());

        assert!(registry.has_tool("web_search"));
        assert!(registry.has_tool("code_edit"));
        assert!(!registry.has_tool("nonexistent"));
    }

    #[test]
    fn test_generate_index_txt_empty() {
        let index = generate_index_txt(&[]);
        assert!(index.is_empty());
    }

    #[test]
    fn test_generate_index_txt() {
        let tools = sample_tools();
        let summaries: Vec<_> = tools.iter().map(ToolDefinition::summary).collect();
        let index = generate_index_txt(&summaries);

        // Should contain all tool names
        assert!(index.contains("web_search"));
        assert!(index.contains("code_edit"));
        assert!(index.contains("list_files"));

        // Should contain descriptions
        assert!(index.contains("Search the web"));
        assert!(index.contains("Edit a file"));

        // Should have one line per tool
        assert_eq!(index.lines().count(), 3);
    }

    #[test]
    fn test_generate_index_txt_alignment() {
        let tools = vec![
            ToolSummary {
                name: "a".into(),
                description: "Short".into(),
            },
            ToolSummary {
                name: "longer_name".into(),
                description: "Longer".into(),
            },
        ];

        let index = generate_index_txt(&tools);
        let lines: Vec<_> = index.lines().collect();

        // The 'a' line should have more padding than 'longer_name'
        let a_desc_start = lines[0].find("Short").expect("should find Short");
        let longer_desc_start = lines[1].find("Longer").expect("should find Longer");

        // Both descriptions should start at the same column
        assert_eq!(a_desc_start, longer_desc_start);
    }

    #[test]
    fn test_tool_definition_serialization() {
        let tool = ToolDefinition::new("test", "Test tool", json!({"type": "object"}));

        let json = serde_json::to_string(&tool).expect("serialize");
        let parsed: ToolDefinition = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(parsed.name, tool.name);
        assert_eq!(parsed.description, tool.description);
        assert_eq!(parsed.parameters, tool.parameters);
    }

    #[test]
    fn test_from_iterator_trait() {
        let tools = sample_tools();
        let registry: VecToolRegistry = tools.into_iter().collect();

        assert_eq!(registry.list_tools().len(), 3);
    }
}
