//! Integration tests for the agent sandbox system.
//!
//! These tests verify the complete agent lifecycle including:
//! - VFS structure and permissions
//! - Tool invocation via callback handlers
//! - Conversation history access
//! - Multi-tool execution cycles

#![cfg(feature = "embedded-shell")]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use conch::ResourceLimits;
use conch::agent::{
    AgentSandbox, ConversationMetadata, ConversationSummary, SimpleHistoryProvider, ToolDefinition,
};
use conch::{ToolRequest, ToolResult};

fn limits() -> ResourceLimits {
    ResourceLimits::default()
}

// =============================================================================
// VFS Structure Tests
// =============================================================================

mod vfs_structure {
    use super::*;

    #[tokio::test]
    async fn test_agent_directories_exist() {
        let mut sandbox = AgentSandbox::builder("test-agent")
            .build()
            .await
            .expect("build sandbox");

        // Verify key files exist by reading them (no ls available)
        // The shell only has: cat, grep, wc, jq, tail, head, tool
        let result = sandbox
            .execute("cat /agent/metadata.json", &limits())
            .await
            .expect("cat metadata");
        assert_eq!(result.exit_code, 0, "metadata.json should exist");

        let result = sandbox
            .execute("cat /agent/params.json", &limits())
            .await
            .expect("cat params");
        assert_eq!(result.exit_code, 0, "params.json should exist");
    }

    #[tokio::test]
    async fn test_agent_metadata_content() {
        let mut sandbox = AgentSandbox::builder("my-agent-id")
            .name("Test Agent")
            .parent("parent-123")
            .capability("read_files")
            .capability("write_files")
            .build()
            .await
            .expect("build sandbox");

        let result = sandbox
            .execute("cat /agent/metadata.json", &limits())
            .await
            .expect("cat metadata");

        assert_eq!(result.exit_code, 0);
        let metadata: serde_json::Value =
            serde_json::from_str(&String::from_utf8_lossy(&result.stdout)).expect("parse json");

        assert_eq!(metadata["id"], "my-agent-id");
        assert_eq!(metadata["name"], "Test Agent");
        assert_eq!(metadata["parent_id"], "parent-123");
        assert!(metadata["capabilities"].as_array().unwrap().len() == 2);
    }

    #[tokio::test]
    async fn test_agent_params_content() {
        let mut sandbox = AgentSandbox::builder("agent-123")
            .params(serde_json::json!({
                "task": "analyze code",
                "options": {
                    "verbose": true,
                    "max_depth": 3
                }
            }))
            .build()
            .await
            .expect("build sandbox");

        let result = sandbox
            .execute("cat /agent/params.json", &limits())
            .await
            .expect("cat params");

        assert_eq!(result.exit_code, 0);
        let params: serde_json::Value =
            serde_json::from_str(&String::from_utf8_lossy(&result.stdout)).expect("parse json");

        assert_eq!(params["task"], "analyze code");
        assert_eq!(params["options"]["verbose"], true);
        assert_eq!(params["options"]["max_depth"], 3);
    }

    #[tokio::test]
    async fn test_scratch_directory_writable() {
        let mut sandbox = AgentSandbox::builder("agent-123")
            .build()
            .await
            .expect("build sandbox");

        // Write to scratch
        let result = sandbox
            .execute("echo 'test content' > /agent/scratch/output.txt", &limits())
            .await
            .expect("write scratch");

        assert_eq!(result.exit_code, 0);

        // Read back
        let result = sandbox
            .execute("cat /agent/scratch/output.txt", &limits())
            .await
            .expect("read scratch");

        assert_eq!(result.exit_code, 0);
        assert!(String::from_utf8_lossy(&result.stdout).contains("test content"));
    }

    #[tokio::test]
    async fn test_state_directory_writable() {
        let mut sandbox = AgentSandbox::builder("agent-123")
            .build()
            .await
            .expect("build sandbox");

        // Write to state
        let result = sandbox
            .execute(
                r#"echo '{"progress": 50}' > /agent/state/status.json"#,
                &limits(),
            )
            .await
            .expect("write state");

        assert_eq!(result.exit_code, 0);

        // Read back and parse
        let result = sandbox
            .execute("cat /agent/state/status.json | jq .progress", &limits())
            .await
            .expect("read state");

        assert_eq!(result.exit_code, 0);
        assert!(String::from_utf8_lossy(&result.stdout).trim() == "50");
    }

    #[tokio::test]
    async fn test_tools_directory_structure() {
        let mut sandbox = AgentSandbox::builder("agent-123")
            .tool(ToolDefinition::no_params("tool_a", "First tool"))
            .tool(ToolDefinition::no_params("tool_b", "Second tool"))
            .build()
            .await
            .expect("build sandbox");

        // Check index
        let result = sandbox
            .execute("cat /tools/index.txt", &limits())
            .await
            .expect("cat index");

        assert_eq!(result.exit_code, 0);
        let stdout = String::from_utf8_lossy(&result.stdout);
        assert!(stdout.contains("tool_a"));
        assert!(stdout.contains("tool_b"));

        // Check tool definition
        let result = sandbox
            .execute("cat /tools/available/tool_a.json", &limits())
            .await
            .expect("cat tool_a");

        assert_eq!(result.exit_code, 0);
        let tool_def: serde_json::Value =
            serde_json::from_str(&String::from_utf8_lossy(&result.stdout)).expect("parse");
        assert_eq!(tool_def["name"], "tool_a");
        assert_eq!(tool_def["description"], "First tool");
    }
}

// =============================================================================
// Tool Registry Tests
// =============================================================================

mod tool_registry {
    use super::*;

    #[tokio::test]
    async fn test_tool_definitions_available() {
        let mut sandbox = AgentSandbox::builder("agent-123")
            .tool(ToolDefinition::new(
                "web_search",
                "Search the web",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "query": { "type": "string" }
                    },
                    "required": ["query"]
                }),
            ))
            .build()
            .await
            .expect("build sandbox");

        // Note: ToolDefinition has `parameters` field, not `schema`
        let result = sandbox
            .execute(
                "cat /tools/available/web_search.json | jq .parameters",
                &limits(),
            )
            .await
            .expect("cat tool");

        assert_eq!(result.exit_code, 0);
        let parameters: serde_json::Value =
            serde_json::from_str(&String::from_utf8_lossy(&result.stdout)).expect("parse");
        assert_eq!(parameters["type"], "object");
        assert!(
            parameters["required"]
                .as_array()
                .unwrap()
                .contains(&"query".into())
        );
    }

    #[tokio::test]
    async fn test_multiple_tools() {
        let mut sandbox = AgentSandbox::builder("agent-123")
            .tool(ToolDefinition::no_params("tool_1", "First"))
            .tool(ToolDefinition::no_params("tool_2", "Second"))
            .tool(ToolDefinition::no_params("tool_3", "Third"))
            .build()
            .await
            .expect("build sandbox");

        // Count tools in index
        let result = sandbox
            .execute("cat /tools/index.txt | wc -l", &limits())
            .await
            .expect("count tools");

        assert_eq!(result.exit_code, 0);
        let count: i32 = String::from_utf8_lossy(&result.stdout)
            .trim()
            .parse()
            .unwrap_or(0);
        assert_eq!(count, 3);
    }

    #[tokio::test]
    async fn test_grep_tools() {
        let mut sandbox = AgentSandbox::builder("agent-123")
            .tool(ToolDefinition::no_params("web_search", "Search the web"))
            .tool(ToolDefinition::no_params("file_search", "Search files"))
            .tool(ToolDefinition::no_params("calculator", "Do math"))
            .build()
            .await
            .expect("build sandbox");

        // Grep for 'search' tools
        let result = sandbox
            .execute("grep -i search /tools/index.txt", &limits())
            .await
            .expect("grep");

        assert_eq!(result.exit_code, 0);
        let stdout = String::from_utf8_lossy(&result.stdout);
        assert!(stdout.contains("web_search"));
        assert!(stdout.contains("file_search"));
        assert!(!stdout.contains("calculator"));
    }

    #[tokio::test]
    async fn test_no_tools() {
        let mut sandbox = AgentSandbox::builder("agent-123")
            .build()
            .await
            .expect("build sandbox");

        let result = sandbox
            .execute("cat /tools/index.txt", &limits())
            .await
            .expect("cat index");

        assert_eq!(result.exit_code, 0);
        // Index should be empty or minimal
        assert!(result.stdout.is_empty() || result.stdout.len() < 10);
    }
}

// =============================================================================
// Tool Execution Tests (Callback-based)
// =============================================================================

mod tool_execution {
    use super::*;

    #[tokio::test]
    async fn test_tool_handler_called() {
        let call_count = Arc::new(AtomicUsize::new(0));
        let call_count_clone = call_count.clone();

        let handler = move |_request: ToolRequest| {
            let count = call_count_clone.clone();
            async move {
                count.fetch_add(1, Ordering::SeqCst);
                ToolResult {
                    success: true,
                    output: "tool executed".to_string(),
                }
            }
        };

        let mut sandbox = AgentSandbox::builder("agent-123")
            .tool(ToolDefinition::no_params("test_tool", "A test tool"))
            .tool_handler(handler)
            .build()
            .await
            .expect("build sandbox");

        let result = sandbox
            .execute("tool test_tool", &limits())
            .await
            .expect("execute");

        assert_eq!(result.exit_code, 0);
        assert_eq!(call_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_tool_receives_params() {
        let received_params = Arc::new(std::sync::Mutex::new(String::new()));
        let params_clone = received_params.clone();

        let handler = move |request: ToolRequest| {
            let params = params_clone.clone();
            async move {
                *params.lock().unwrap() = request.params.clone();
                ToolResult {
                    success: true,
                    output: "ok".to_string(),
                }
            }
        };

        let mut sandbox = AgentSandbox::builder("agent-123")
            .tool(ToolDefinition::no_params("calculator", "Calculate"))
            .tool_handler(handler)
            .build()
            .await
            .expect("build sandbox");

        sandbox
            .execute("tool calculator --operation add --a 10 --b 20", &limits())
            .await
            .expect("execute");

        let params_str = received_params.lock().unwrap().clone();
        let params: serde_json::Value = serde_json::from_str(&params_str).expect("parse params");
        assert_eq!(params["operation"], "add");
        assert_eq!(params["a"], 10);
        assert_eq!(params["b"], 20);
    }

    #[tokio::test]
    async fn test_tool_output_returned_to_script() {
        let handler = |_request: ToolRequest| async move {
            ToolResult {
                success: true,
                output: serde_json::to_string(&serde_json::json!({
                    "result": 42,
                    "message": "success"
                }))
                .unwrap(),
            }
        };

        let mut sandbox = AgentSandbox::builder("agent-123")
            .tool(ToolDefinition::no_params("my_tool", "My tool"))
            .tool_handler(handler)
            .build()
            .await
            .expect("build sandbox");

        let result = sandbox
            .execute("tool my_tool", &limits())
            .await
            .expect("execute");

        assert_eq!(result.exit_code, 0);
        let stdout = String::from_utf8_lossy(&result.stdout);
        assert!(stdout.contains("42"));
        assert!(stdout.contains("success"));
    }

    #[tokio::test]
    async fn test_tool_error_handling() {
        let handler = |_request: ToolRequest| async move {
            ToolResult {
                success: false,
                output: "Connection timeout".to_string(),
            }
        };

        let mut sandbox = AgentSandbox::builder("agent-123")
            .tool(ToolDefinition::no_params("failing_tool", "Will fail"))
            .tool_handler(handler)
            .build()
            .await
            .expect("build sandbox");

        let result = sandbox
            .execute("tool failing_tool", &limits())
            .await
            .expect("execute");

        // Tool returns error (written to stderr) but script continues
        // Exit code should be 1 for a failed tool call
        assert_eq!(result.exit_code, 1);
        let stderr = String::from_utf8_lossy(&result.stderr);
        assert!(stderr.contains("Connection timeout"), "stderr: {}", stderr);
    }

    #[tokio::test]
    async fn test_multiple_tool_calls_in_script() {
        let call_count = Arc::new(AtomicUsize::new(0));
        let call_count_clone = call_count.clone();

        let handler = move |request: ToolRequest| {
            let count = call_count_clone.clone();
            async move {
                let n = count.fetch_add(1, Ordering::SeqCst) + 1;
                ToolResult {
                    success: true,
                    output: format!("Call {} for {}", n, request.tool),
                }
            }
        };

        let mut sandbox = AgentSandbox::builder("agent-123")
            .tool(ToolDefinition::no_params("tool_a", "First"))
            .tool(ToolDefinition::no_params("tool_b", "Second"))
            .tool_handler(handler)
            .build()
            .await
            .expect("build sandbox");

        let result = sandbox
            .execute(
                r#"
                echo "Starting"
                tool tool_a --step 1
                tool tool_b --step 2
                echo "Done"
            "#,
                &limits(),
            )
            .await
            .expect("execute");

        assert_eq!(result.exit_code, 0);
        assert_eq!(call_count.load(Ordering::SeqCst), 2);

        let stdout = String::from_utf8_lossy(&result.stdout);
        assert!(stdout.contains("Starting"));
        assert!(stdout.contains("Call 1"));
        assert!(stdout.contains("Call 2"));
        assert!(stdout.contains("Done"));
    }

    #[tokio::test]
    async fn test_tool_with_json_params() {
        let received_params = Arc::new(std::sync::Mutex::new(String::new()));
        let params_clone = received_params.clone();

        let handler = move |request: ToolRequest| {
            let params = params_clone.clone();
            async move {
                *params.lock().unwrap() = request.params.clone();
                ToolResult {
                    success: true,
                    output: "ok".to_string(),
                }
            }
        };

        let mut sandbox = AgentSandbox::builder("agent-123")
            .tool(ToolDefinition::no_params("complex", "Complex params"))
            .tool_handler(handler)
            .build()
            .await
            .expect("build sandbox");

        sandbox
            .execute(
                r#"tool complex --json '{"nested": {"key": "value"}, "array": [1, 2, 3]}'"#,
                &limits(),
            )
            .await
            .expect("execute");

        let params_str = received_params.lock().unwrap().clone();
        let params: serde_json::Value = serde_json::from_str(&params_str).expect("parse");
        assert_eq!(params["nested"]["key"], "value");
        assert_eq!(params["array"][0], 1);
    }

    #[tokio::test]
    async fn test_no_handler_returns_error() {
        // No tool_handler configured
        let mut sandbox = AgentSandbox::builder("agent-123")
            .tool(ToolDefinition::no_params("orphan_tool", "No handler"))
            .build()
            .await
            .expect("build sandbox");

        let result = sandbox
            .execute("tool orphan_tool", &limits())
            .await
            .expect("execute");

        // Should fail with exit code 1 and error message in stderr
        assert_eq!(result.exit_code, 1);
        let stderr = String::from_utf8_lossy(&result.stderr);
        assert!(
            stderr.contains("No tool handler") || stderr.contains("not configured"),
            "stderr: {}",
            stderr
        );
    }
}

// =============================================================================
// Conversation History Tests
// =============================================================================

mod conversation_history {
    use super::*;

    #[tokio::test]
    async fn test_current_transcript_accessible() {
        let transcript = r#"U> Can you help me understand this code?

A> Of course! I'll analyze the code for you.

T[file_read] {"path": "src/main.rs"}
R> {"content": "fn main() { println!(\"Hello\"); }"}

A> This is a simple Rust main function that prints "Hello"."#;

        let history = SimpleHistoryProvider::new().with_transcript(transcript);

        let mut sandbox = AgentSandbox::builder("agent-123")
            .history(Arc::new(history))
            .build()
            .await
            .expect("build sandbox");

        let result = sandbox
            .execute("cat /history/current/transcript.md", &limits())
            .await
            .expect("cat transcript");

        assert_eq!(result.exit_code, 0);
        let stdout = String::from_utf8_lossy(&result.stdout);
        assert!(stdout.contains("U>"));
        assert!(stdout.contains("understand this code"));
        assert!(stdout.contains("T[file_read]"));
        assert!(stdout.contains("R>"));
    }

    #[tokio::test]
    async fn test_history_metadata() {
        let history = SimpleHistoryProvider::new()
            .with_transcript("U> Test message")
            .with_metadata(ConversationMetadata {
                id: "current".to_string(),
                title: Some("Code Review Session".to_string()),
                started_at: "2024-01-15T10:30:00Z".to_string(),
                updated_at: Some("2024-01-15T11:45:00Z".to_string()),
                user_message_count: 5,
                assistant_message_count: 4,
                tool_call_count: 2,
            });

        let mut sandbox = AgentSandbox::builder("agent-123")
            .history(Arc::new(history))
            .build()
            .await
            .expect("build sandbox");

        let result = sandbox
            .execute("cat /history/current/metadata.json", &limits())
            .await
            .expect("cat metadata");

        assert_eq!(result.exit_code, 0);
        let metadata: serde_json::Value =
            serde_json::from_str(&String::from_utf8_lossy(&result.stdout)).expect("parse");

        assert_eq!(metadata["title"], "Code Review Session");
        assert_eq!(metadata["user_message_count"], 5);
        assert_eq!(metadata["assistant_message_count"], 4);
        assert_eq!(metadata["tool_call_count"], 2);
    }

    #[tokio::test]
    async fn test_history_index() {
        let history = SimpleHistoryProvider::new()
            .with_transcript("U> Current task")
            .with_metadata(ConversationMetadata {
                id: "current".to_string(),
                title: Some("Current Task".to_string()),
                started_at: "2024-01-15T00:00:00Z".to_string(),
                updated_at: None,
                user_message_count: 1,
                assistant_message_count: 0,
                tool_call_count: 0,
            })
            .with_conversation(
                ConversationSummary {
                    id: "conv-001".to_string(),
                    title: "Previous Task".to_string(),
                    started_at: "2024-01-14T00:00:00Z".to_string(),
                    message_count: 10,
                    is_current: false,
                },
                "U> Old conversation",
                None,
            )
            .with_conversation(
                ConversationSummary {
                    id: "conv-002".to_string(),
                    title: "Even Older".to_string(),
                    started_at: "2024-01-13T00:00:00Z".to_string(),
                    message_count: 5,
                    is_current: false,
                },
                "U> Very old",
                None,
            );

        let mut sandbox = AgentSandbox::builder("agent-123")
            .history(Arc::new(history))
            .build()
            .await
            .expect("build sandbox");

        let result = sandbox
            .execute("cat /history/index.txt", &limits())
            .await
            .expect("cat index");

        assert_eq!(result.exit_code, 0);
        let stdout = String::from_utf8_lossy(&result.stdout);
        assert!(stdout.contains("current (current)"));
        assert!(stdout.contains("conv-001"));
        assert!(stdout.contains("conv-002"));
        assert!(stdout.contains("10 messages"));
    }

    #[tokio::test]
    async fn test_grep_history_for_tool_calls() {
        let transcript = r#"U> Search for Rust tutorials

A> I'll search for that.

T[web_search] {"query": "Rust programming tutorials"}
R> {"results": [{"title": "Learn Rust", "url": "https://rust-lang.org"}]}

A> Found some great resources.

U> Now search for async patterns

A> Searching for async patterns.

T[web_search] {"query": "Rust async patterns"}
R> {"results": [{"title": "Async Rust", "url": "https://async.rs"}]}

A> Here are async resources."#;

        let history = SimpleHistoryProvider::new().with_transcript(transcript);

        let mut sandbox = AgentSandbox::builder("agent-123")
            .history(Arc::new(history))
            .build()
            .await
            .expect("build sandbox");

        // Count tool calls
        let result = sandbox
            .execute(
                "grep 'T\\[' /history/current/transcript.md | wc -l",
                &limits(),
            )
            .await
            .expect("grep");

        assert_eq!(result.exit_code, 0);
        let count: i32 = String::from_utf8_lossy(&result.stdout)
            .trim()
            .parse()
            .unwrap_or(0);
        assert_eq!(count, 2);
    }

    #[tokio::test]
    async fn test_no_history_provider() {
        let mut sandbox = AgentSandbox::builder("agent-123")
            .build()
            .await
            .expect("build sandbox");

        // Without a history provider, sandbox should still be functional
        let result = sandbox
            .execute(
                "echo test > /agent/scratch/test.txt && cat /agent/scratch/test.txt",
                &limits(),
            )
            .await
            .expect("scratch test");

        assert_eq!(result.exit_code, 0);
        assert!(String::from_utf8_lossy(&result.stdout).contains("test"));
    }
}

// =============================================================================
// Agent Lifecycle Tests
// =============================================================================

mod agent_lifecycle {
    use super::*;

    #[tokio::test]
    async fn test_research_task_lifecycle() {
        let search_results = serde_json::json!({
            "results": [
                {"title": "Async Rust Book", "url": "https://rust-lang.org/async"},
                {"title": "Tokio Tutorial", "url": "https://tokio.rs"}
            ]
        });

        let handler = move |request: ToolRequest| {
            let results = search_results.clone();
            async move {
                match request.tool.as_str() {
                    "web_search" => ToolResult {
                        success: true,
                        output: serde_json::to_string(&results).unwrap(),
                    },
                    _ => ToolResult {
                        success: false,
                        output: "Unknown tool".to_string(),
                    },
                }
            }
        };

        let mut sandbox = AgentSandbox::builder("researcher-001")
            .name("Research Agent")
            .params(serde_json::json!({
                "task": "Research Rust async patterns",
                "output_format": "markdown"
            }))
            .tool(ToolDefinition::no_params("web_search", "Search the web"))
            .tool_handler(handler)
            .build()
            .await
            .expect("build sandbox");

        // Read task params
        let result = sandbox
            .execute("cat /agent/params.json | jq -r .task", &limits())
            .await
            .expect("read params");
        assert!(String::from_utf8_lossy(&result.stdout).contains("Research"));

        // Execute search tool
        let result = sandbox
            .execute("tool web_search --query 'rust async'", &limits())
            .await
            .expect("search");
        assert_eq!(result.exit_code, 0);
        assert!(String::from_utf8_lossy(&result.stdout).contains("Tokio"));

        // Write findings to scratch
        let result = sandbox
            .execute(
                "echo '# Research Results' > /agent/scratch/findings.md && cat /agent/scratch/findings.md",
                &limits(),
            )
            .await
            .expect("write findings");
        assert_eq!(result.exit_code, 0);
    }

    #[tokio::test]
    async fn test_code_analysis_task() {
        let handler = |request: ToolRequest| async move {
            let params: serde_json::Value =
                serde_json::from_str(&request.params).unwrap_or_default();

            match request.tool.as_str() {
                "file_read" => {
                    let path = params["path"].as_str().unwrap_or("unknown");
                    ToolResult {
                        success: true,
                        output: serde_json::to_string(&serde_json::json!({
                            "path": path,
                            "content": "fn main() { println!(\"Hello\"); }",
                            "lines": 1
                        }))
                        .unwrap(),
                    }
                }
                "code_search" => ToolResult {
                    success: true,
                    output: serde_json::to_string(&serde_json::json!({
                        "matches": [
                            {"file": "src/lib.rs", "line": 42, "text": "for item in items {"}
                        ]
                    }))
                    .unwrap(),
                },
                _ => ToolResult {
                    success: false,
                    output: "Unknown tool".to_string(),
                },
            }
        };

        let mut sandbox = AgentSandbox::builder("analyzer-001")
            .tool(ToolDefinition::no_params("file_read", "Read files"))
            .tool(ToolDefinition::no_params("code_search", "Search code"))
            .tool_handler(handler)
            .build()
            .await
            .expect("build sandbox");

        // Read a file
        let result = sandbox
            .execute("tool file_read --path src/main.rs", &limits())
            .await
            .expect("file read");
        assert!(String::from_utf8_lossy(&result.stdout).contains("Hello"));

        // Search code
        let result = sandbox
            .execute("tool code_search --pattern 'for.*in'", &limits())
            .await
            .expect("code search");
        assert!(String::from_utf8_lossy(&result.stdout).contains("matches"));
    }
}

// =============================================================================
// Edge Cases
// =============================================================================

mod edge_cases {
    use super::*;

    #[tokio::test]
    async fn test_empty_params() {
        let mut sandbox = AgentSandbox::builder("agent-123")
            .build()
            .await
            .expect("build sandbox");

        let result = sandbox
            .execute("cat /agent/params.json", &limits())
            .await
            .expect("cat params");

        assert_eq!(result.exit_code, 0);
        let params: serde_json::Value =
            serde_json::from_str(&String::from_utf8_lossy(&result.stdout)).expect("parse");
        assert!(params.is_object());
    }

    #[tokio::test]
    async fn test_special_characters_in_params() {
        let mut sandbox = AgentSandbox::builder("agent-123")
            .params(serde_json::json!({
                "query": "path/to/file.rs",
                "pattern": "fn\\s+\\w+",
                "message": "Hello \"World\"!"
            }))
            .build()
            .await
            .expect("build sandbox");

        let result = sandbox
            .execute("cat /agent/params.json", &limits())
            .await
            .expect("cat params");

        assert_eq!(result.exit_code, 0);
        let params: serde_json::Value =
            serde_json::from_str(&String::from_utf8_lossy(&result.stdout)).expect("parse");
        assert_eq!(params["query"], "path/to/file.rs");
        assert_eq!(params["pattern"], "fn\\s+\\w+");
        assert_eq!(params["message"], "Hello \"World\"!");
    }

    #[tokio::test]
    async fn test_large_tool_output() {
        let large_data: Vec<serde_json::Value> = (0..1000)
            .map(|i| {
                serde_json::json!({
                    "id": i,
                    "name": format!("Item {}", i),
                    "data": "x".repeat(100)
                })
            })
            .collect();

        let handler = move |_request: ToolRequest| {
            let data = large_data.clone();
            async move {
                ToolResult {
                    success: true,
                    output: serde_json::to_string(&data).unwrap(),
                }
            }
        };

        let mut sandbox = AgentSandbox::builder("agent-123")
            .tool(ToolDefinition::no_params("bulk_data", "Get bulk data"))
            .tool_handler(handler)
            .build()
            .await
            .expect("build sandbox");

        let result = sandbox
            .execute("tool bulk_data", &limits())
            .await
            .expect("bulk data");

        assert_eq!(result.exit_code, 0);
        // Output should contain our data
        let stdout = String::from_utf8_lossy(&result.stdout);
        assert!(stdout.contains("Item 0"));
        assert!(stdout.contains("Item 999"));
    }

    #[tokio::test]
    async fn test_tool_with_no_params() {
        let received_params = Arc::new(std::sync::Mutex::new(String::new()));
        let params_clone = received_params.clone();

        let handler = move |request: ToolRequest| {
            let params = params_clone.clone();
            async move {
                *params.lock().unwrap() = request.params.clone();
                ToolResult {
                    success: true,
                    output: "ok".to_string(),
                }
            }
        };

        let mut sandbox = AgentSandbox::builder("agent-123")
            .tool(ToolDefinition::no_params("no_args", "No arguments"))
            .tool_handler(handler)
            .build()
            .await
            .expect("build sandbox");

        sandbox
            .execute("tool no_args", &limits())
            .await
            .expect("execute");

        let params_str = received_params.lock().unwrap().clone();
        // Should be empty object or similar
        let params: serde_json::Value = serde_json::from_str(&params_str).unwrap_or_default();
        assert!(params.is_object());
    }

    #[tokio::test]
    async fn test_unicode_in_tool_output() {
        let handler = |_request: ToolRequest| async move {
            ToolResult {
                success: true,
                output: "Hello 世界! 🎉 Привет мир".to_string(),
            }
        };

        let mut sandbox = AgentSandbox::builder("agent-123")
            .tool(ToolDefinition::no_params("unicode", "Unicode test"))
            .tool_handler(handler)
            .build()
            .await
            .expect("build sandbox");

        let result = sandbox
            .execute("tool unicode", &limits())
            .await
            .expect("execute");

        assert_eq!(result.exit_code, 0);
        let stdout = String::from_utf8_lossy(&result.stdout);
        assert!(stdout.contains("世界"));
        assert!(stdout.contains("🎉"));
        assert!(stdout.contains("Привет"));
    }
}
