//! Integration tests for the agent sandbox system.
//!
//! These tests verify the complete agent lifecycle including:
//! - VFS structure and permissions
//! - Tool invocation and result handling
//! - Conversation history access
//! - Multi-tool execution cycles

#![cfg(feature = "embedded-shell")]

use std::sync::Arc;

use conch::ResourceLimits;
use conch::agent::{
    AgentSandbox, ConversationMetadata, ConversationSummary, ExecutionOutcome,
    SimpleHistoryProvider, ToolDefinition, ToolResult,
};

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
        let sandbox = AgentSandbox::builder("test-agent")
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

        let result = sandbox
            .execute("cat /tools/index.txt", &limits())
            .await
            .expect("cat tools index");
        assert_eq!(result.exit_code, 0, "tools/index.txt should exist");

        // Write and read back to verify scratch is writable
        let result = sandbox
            .execute(
                "echo test > /agent/scratch/test.txt && cat /agent/scratch/test.txt",
                &limits(),
            )
            .await
            .expect("scratch write/read");
        assert_eq!(result.exit_code, 0, "scratch should be writable");
    }

    #[tokio::test]
    async fn test_agent_metadata_structure() {
        let sandbox = AgentSandbox::builder("agent-abc")
            .name("Test Agent")
            .parent("parent-xyz")
            .capability("read")
            .capability("write")
            .capability("execute")
            .build()
            .await
            .expect("build sandbox");

        let result = sandbox
            .execute("cat /agent/metadata.json", &limits())
            .await
            .expect("cat metadata");

        assert_eq!(result.exit_code, 0);
        let stdout = String::from_utf8_lossy(&result.stdout);

        // Parse and verify JSON structure
        let metadata: serde_json::Value =
            serde_json::from_str(&stdout).expect("parse metadata JSON");

        assert_eq!(metadata["id"], "agent-abc");
        assert_eq!(metadata["name"], "Test Agent");
        assert_eq!(metadata["parent_id"], "parent-xyz");
        assert!(metadata["capabilities"].as_array().unwrap().len() == 3);
        assert!(metadata["spawned_at"].as_str().is_some());
    }

    #[tokio::test]
    async fn test_agent_params_accessible() {
        let params = serde_json::json!({
            "task": "analyze code",
            "target_files": ["src/main.rs", "src/lib.rs"],
            "options": {
                "verbose": true,
                "max_depth": 5
            }
        });

        let sandbox = AgentSandbox::builder("agent-123")
            .params(params.clone())
            .build()
            .await
            .expect("build sandbox");

        // Read params and verify structure
        let result = sandbox
            .execute("cat /agent/params.json", &limits())
            .await
            .expect("cat params");

        assert_eq!(result.exit_code, 0);
        let read_params: serde_json::Value =
            serde_json::from_str(&String::from_utf8_lossy(&result.stdout)).expect("parse params");

        assert_eq!(read_params, params);
    }

    #[tokio::test]
    async fn test_scratch_directory_writable() {
        let sandbox = AgentSandbox::builder("agent-123")
            .build()
            .await
            .expect("build sandbox");

        // Write multiple files to scratch and read them back
        let script = r#"
            echo "file1 content" > /agent/scratch/file1.txt
            echo "file2 content" > /agent/scratch/file2.txt
            cat /agent/scratch/file1.txt
            cat /agent/scratch/file2.txt
        "#;

        let result = sandbox.execute(script, &limits()).await.expect("execute");

        assert_eq!(
            result.exit_code,
            0,
            "stderr: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        let stdout = String::from_utf8_lossy(&result.stdout);
        assert!(stdout.contains("file1 content"));
        assert!(stdout.contains("file2 content"));
    }

    #[tokio::test]
    async fn test_state_directory_persists() {
        let sandbox = AgentSandbox::builder("agent-123")
            .build()
            .await
            .expect("build sandbox");

        // Write state
        sandbox
            .execute(
                r#"echo '{"counter": 42}' > /agent/state/counter.json"#,
                &limits(),
            )
            .await
            .expect("write state");

        // Read state back in a separate execution
        let result = sandbox
            .execute("cat /agent/state/counter.json", &limits())
            .await
            .expect("read state");

        assert_eq!(result.exit_code, 0);
        let state: serde_json::Value =
            serde_json::from_str(&String::from_utf8_lossy(&result.stdout)).expect("parse state");
        assert_eq!(state["counter"], 42);
    }
}

// =============================================================================
// Tool Registry Tests
// =============================================================================

mod tool_registry {
    use super::*;

    #[tokio::test]
    async fn test_tools_index_content() {
        let sandbox = AgentSandbox::builder("agent-123")
            .tool(ToolDefinition::no_params(
                "web_search",
                "Search the web for information",
            ))
            .tool(ToolDefinition::no_params(
                "code_edit",
                "Edit source code files",
            ))
            .tool(ToolDefinition::no_params(
                "file_read",
                "Read file contents from disk",
            ))
            .build()
            .await
            .expect("build sandbox");

        let result = sandbox
            .execute("cat /tools/index.txt", &limits())
            .await
            .expect("cat index");

        assert_eq!(result.exit_code, 0);
        let stdout = String::from_utf8_lossy(&result.stdout);

        // Verify all tools listed with descriptions
        assert!(stdout.contains("web_search"));
        assert!(stdout.contains("Search the web"));
        assert!(stdout.contains("code_edit"));
        assert!(stdout.contains("Edit source code"));
        assert!(stdout.contains("file_read"));
        assert!(stdout.contains("Read file contents"));
    }

    #[tokio::test]
    async fn test_tool_definitions_accessible() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "The search query"
                },
                "limit": {
                    "type": "integer",
                    "default": 10
                }
            },
            "required": ["query"]
        });

        let sandbox = AgentSandbox::builder("agent-123")
            .tool(ToolDefinition::new(
                "web_search",
                "Search the web",
                schema.clone(),
            ))
            .build()
            .await
            .expect("build sandbox");

        let result = sandbox
            .execute("cat /tools/available/web_search.json", &limits())
            .await
            .expect("cat tool def");

        assert_eq!(result.exit_code, 0);
        let tool_def: serde_json::Value =
            serde_json::from_str(&String::from_utf8_lossy(&result.stdout)).expect("parse tool");

        assert_eq!(tool_def["name"], "web_search");
        assert_eq!(tool_def["description"], "Search the web");
        assert!(tool_def["parameters"]["properties"]["query"].is_object());
    }

    #[tokio::test]
    async fn test_grep_tools_by_capability() {
        let sandbox = AgentSandbox::builder("agent-123")
            .tool(ToolDefinition::no_params(
                "web_search",
                "Search the web for information",
            ))
            .tool(ToolDefinition::no_params(
                "code_search",
                "Search code in repository",
            ))
            .tool(ToolDefinition::no_params("file_read", "Read file contents"))
            .tool(ToolDefinition::no_params("file_write", "Write to files"))
            .build()
            .await
            .expect("build sandbox");

        // Grep for search-related tools
        let result = sandbox
            .execute("grep -i search /tools/index.txt", &limits())
            .await
            .expect("grep");

        assert_eq!(result.exit_code, 0);
        let stdout = String::from_utf8_lossy(&result.stdout);
        assert!(stdout.contains("web_search"));
        assert!(stdout.contains("code_search"));
        assert!(!stdout.contains("file_read"));
        assert!(!stdout.contains("file_write"));
    }

    #[tokio::test]
    async fn test_empty_tools_list() {
        let sandbox = AgentSandbox::builder("agent-123")
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
// Tool Execution Cycle Tests
// =============================================================================

mod tool_execution {
    use super::*;

    #[tokio::test]
    async fn test_single_tool_invocation() {
        let sandbox = AgentSandbox::builder("agent-123")
            .tool(ToolDefinition::no_params(
                "calculator",
                "Perform calculations",
            ))
            .build()
            .await
            .expect("build sandbox");

        let outcome = sandbox
            .execute_with_tools("tool calculator --operation add --a 10 --b 20", &limits())
            .await
            .expect("execute");

        match outcome {
            ExecutionOutcome::ToolRequest(request) => {
                assert_eq!(request.tool, "calculator");
                assert_eq!(request.params["operation"], "add");
                assert_eq!(request.params["a"], 10);
                assert_eq!(request.params["b"], 20);
                assert!(request.call_id.starts_with("call-"));
            }
            ExecutionOutcome::Completed(r) => {
                panic!("Expected ToolRequest, got Completed(exit={})", r.exit_code);
            }
        }
    }

    #[tokio::test]
    async fn test_tool_result_written_to_history() {
        let sandbox = AgentSandbox::builder("agent-123")
            .tool(ToolDefinition::no_params("my_tool", "Test tool"))
            .build()
            .await
            .expect("build sandbox");

        // Invoke tool
        let outcome = sandbox
            .execute_with_tools("tool my_tool --input data", &limits())
            .await
            .expect("execute");

        let request = match outcome {
            ExecutionOutcome::ToolRequest(r) => r,
            _ => panic!("Expected ToolRequest"),
        };

        // Write result
        let result_value = serde_json::json!({
            "output": "processed data",
            "status": "success"
        });
        sandbox
            .write_tool_result(&request.call_id, ToolResult::success(result_value.clone()))
            .await
            .expect("write result");

        // Verify result in history
        let history_path = format!("/tools/history/{}/response.json", request.call_id);
        let result = sandbox
            .execute(&format!("cat {history_path}"), &limits())
            .await
            .expect("cat history");

        assert_eq!(result.exit_code, 0);
        let read_result: serde_json::Value =
            serde_json::from_str(&String::from_utf8_lossy(&result.stdout)).expect("parse result");
        assert_eq!(read_result, result_value);

        // Verify last_result.json
        let result = sandbox
            .execute("cat /tools/last_result.json", &limits())
            .await
            .expect("cat last_result");

        assert_eq!(result.exit_code, 0);
        let last_result: serde_json::Value =
            serde_json::from_str(&String::from_utf8_lossy(&result.stdout)).expect("parse");
        assert_eq!(last_result, result_value);
    }

    #[tokio::test]
    async fn test_tool_error_result() {
        let sandbox = AgentSandbox::builder("agent-123")
            .tool(ToolDefinition::no_params("failing_tool", "Tool that fails"))
            .build()
            .await
            .expect("build sandbox");

        let outcome = sandbox
            .execute_with_tools("tool failing_tool", &limits())
            .await
            .expect("execute");

        let request = match outcome {
            ExecutionOutcome::ToolRequest(r) => r,
            _ => panic!("Expected ToolRequest"),
        };

        // Write error result
        sandbox
            .write_tool_result(
                &request.call_id,
                ToolResult::error("Connection timeout after 30s"),
            )
            .await
            .expect("write error");

        // Verify error file exists
        let error_path = format!("/tools/history/{}/error.json", request.call_id);
        let result = sandbox
            .execute(&format!("cat {error_path}"), &limits())
            .await
            .expect("cat error");

        assert_eq!(result.exit_code, 0);
        let error: serde_json::Value =
            serde_json::from_str(&String::from_utf8_lossy(&result.stdout)).expect("parse");
        assert!(
            error["error"]
                .as_str()
                .unwrap()
                .contains("Connection timeout")
        );

        // Verify metadata shows failure
        let metadata_path = format!("/tools/history/{}/metadata.json", request.call_id);
        let result = sandbox
            .execute(&format!("cat {metadata_path}"), &limits())
            .await
            .expect("cat metadata");

        let metadata: serde_json::Value =
            serde_json::from_str(&String::from_utf8_lossy(&result.stdout)).expect("parse");
        assert_eq!(metadata["success"], false);
    }

    #[tokio::test]
    async fn test_multiple_sequential_tool_calls() {
        let sandbox = AgentSandbox::builder("agent-123")
            .tool(ToolDefinition::no_params("tool_a", "First tool"))
            .tool(ToolDefinition::no_params("tool_b", "Second tool"))
            .build()
            .await
            .expect("build sandbox");

        // First tool call
        let outcome1 = sandbox
            .execute_with_tools("tool tool_a --step 1", &limits())
            .await
            .expect("execute 1");

        let request1 = match outcome1 {
            ExecutionOutcome::ToolRequest(r) => r,
            _ => panic!("Expected ToolRequest 1"),
        };
        assert_eq!(request1.call_id, "call-001");

        sandbox
            .write_tool_result(
                &request1.call_id,
                ToolResult::success(serde_json::json!({"step": 1, "result": "ok"})),
            )
            .await
            .expect("write 1");

        // Second tool call
        let outcome2 = sandbox
            .execute_with_tools("tool tool_b --step 2", &limits())
            .await
            .expect("execute 2");

        let request2 = match outcome2 {
            ExecutionOutcome::ToolRequest(r) => r,
            _ => panic!("Expected ToolRequest 2"),
        };
        assert_eq!(request2.call_id, "call-002");

        sandbox
            .write_tool_result(
                &request2.call_id,
                ToolResult::success(serde_json::json!({"step": 2, "result": "done"})),
            )
            .await
            .expect("write 2");

        // Verify both in history by reading their metadata
        let result = sandbox
            .execute("cat /tools/history/call-001/metadata.json", &limits())
            .await
            .expect("read call-001 metadata");
        assert_eq!(result.exit_code, 0, "call-001 should exist in history");

        let result = sandbox
            .execute("cat /tools/history/call-002/metadata.json", &limits())
            .await
            .expect("read call-002 metadata");
        assert_eq!(result.exit_code, 0, "call-002 should exist in history");
    }

    #[tokio::test]
    async fn test_tool_with_json_params() {
        let sandbox = AgentSandbox::builder("agent-123")
            .tool(ToolDefinition::no_params("complex_tool", "Complex params"))
            .build()
            .await
            .expect("build sandbox");

        let outcome = sandbox
            .execute_with_tools(
                r#"tool complex_tool --json '{"nested": {"key": "value"}, "array": [1, 2, 3]}'"#,
                &limits(),
            )
            .await
            .expect("execute");

        match outcome {
            ExecutionOutcome::ToolRequest(request) => {
                assert_eq!(request.params["nested"]["key"], "value");
                assert_eq!(request.params["array"][0], 1);
                assert_eq!(request.params["array"][2], 3);
            }
            _ => panic!("Expected ToolRequest"),
        }
    }

    #[tokio::test]
    async fn test_normal_command_completes() {
        let sandbox = AgentSandbox::builder("agent-123")
            .tool(ToolDefinition::no_params("some_tool", "A tool"))
            .build()
            .await
            .expect("build sandbox");

        // Regular command should complete normally
        let outcome = sandbox
            .execute_with_tools("echo hello && cat /agent/metadata.json", &limits())
            .await
            .expect("execute");

        match outcome {
            ExecutionOutcome::Completed(result) => {
                assert_eq!(result.exit_code, 0);
                let stdout = String::from_utf8_lossy(&result.stdout);
                assert!(stdout.contains("hello"));
                assert!(stdout.contains("agent-123")); // metadata contains agent id
            }
            ExecutionOutcome::ToolRequest(_) => {
                panic!("Expected Completed, got ToolRequest");
            }
        }
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

        let sandbox = AgentSandbox::builder("agent-123")
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

        let sandbox = AgentSandbox::builder("agent-123")
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

        let sandbox = AgentSandbox::builder("agent-123")
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

        let sandbox = AgentSandbox::builder("agent-123")
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

        // Find specific tool
        let result = sandbox
            .execute(
                "grep 'T\\[web_search\\]' /history/current/transcript.md",
                &limits(),
            )
            .await
            .expect("grep");

        assert_eq!(result.exit_code, 0);
        assert!(String::from_utf8_lossy(&result.stdout).contains("web_search"));
    }

    #[tokio::test]
    async fn test_history_read_only() {
        let history =
            SimpleHistoryProvider::new().with_transcript("U> Important conversation data");

        let sandbox = AgentSandbox::builder("agent-123")
            .history(Arc::new(history))
            .build()
            .await
            .expect("build sandbox");

        // Attempt to write should fail (history is read-only for agents)
        let result = sandbox
            .execute(
                "echo 'tampered' > /history/current/transcript.md",
                &limits(),
            )
            .await
            .expect("execute");

        // Should fail - history is read-only
        assert_ne!(result.exit_code, 0);
    }

    #[tokio::test]
    async fn test_no_history_provider() {
        let sandbox = AgentSandbox::builder("agent-123")
            .build()
            .await
            .expect("build sandbox");

        // Without a history provider, /history/current/ directory exists but may be empty
        // Verify we can at least write to scratch (sandbox is functional)
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
// Full Agent Lifecycle Tests
// =============================================================================

mod agent_lifecycle {
    use super::*;

    /// Simulates a complete agent task: research and summarize.
    #[tokio::test]
    async fn test_research_task_lifecycle() {
        let sandbox = AgentSandbox::builder("researcher-001")
            .name("Research Agent")
            .params(serde_json::json!({
                "task": "Research Rust async patterns",
                "output_format": "markdown"
            }))
            .tool(ToolDefinition::no_params(
                "web_search",
                "Search the web for information",
            ))
            .tool(ToolDefinition::no_params("file_write", "Write to a file"))
            .build()
            .await
            .expect("build sandbox");

        // Step 1: Agent reads its task
        let result = sandbox
            .execute("cat /agent/params.json | jq -r .task", &limits())
            .await
            .expect("read task");
        assert_eq!(result.exit_code, 0);
        assert!(String::from_utf8_lossy(&result.stdout).contains("Research Rust"));

        // Step 2: Agent invokes web_search
        let outcome = sandbox
            .execute_with_tools(
                "tool web_search --query 'Rust async await patterns'",
                &limits(),
            )
            .await
            .expect("invoke search");

        let search_request = match outcome {
            ExecutionOutcome::ToolRequest(r) => r,
            _ => panic!("Expected ToolRequest"),
        };
        assert_eq!(search_request.tool, "web_search");

        // Orchestrator executes search and returns results
        let search_results = serde_json::json!({
            "results": [
                {"title": "Async/Await in Rust", "url": "https://rust-lang.org/async"},
                {"title": "Tokio Tutorial", "url": "https://tokio.rs/tutorial"}
            ]
        });
        sandbox
            .write_tool_result(&search_request.call_id, ToolResult::success(search_results))
            .await
            .expect("write search result");

        // Step 3: Agent reads results and stores them
        let result = sandbox
            .execute("cat /tools/last_result.json", &limits())
            .await
            .expect("read results");

        assert_eq!(
            result.exit_code,
            0,
            "stderr: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        let stdout = String::from_utf8_lossy(&result.stdout);
        assert!(
            stdout.contains("Async/Await in Rust"),
            "Should contain search result title: {}",
            stdout
        );

        // Step 4: Agent writes summary to scratch
        let result = sandbox
            .execute(
                r#"echo "Research complete" > /agent/scratch/summary.txt && cat /agent/scratch/summary.txt"#,
                &limits(),
            )
            .await
            .expect("write summary");
        assert_eq!(result.exit_code, 0);
        assert!(String::from_utf8_lossy(&result.stdout).contains("Research complete"));
    }

    /// Tests error recovery in a multi-step workflow.
    #[tokio::test]
    async fn test_error_recovery_workflow() {
        let sandbox = AgentSandbox::builder("resilient-agent")
            .tool(ToolDefinition::no_params("api_call", "Make API call"))
            .build()
            .await
            .expect("build sandbox");

        // First attempt fails
        let outcome1 = sandbox
            .execute_with_tools("tool api_call --endpoint /users", &limits())
            .await
            .expect("first call");

        let request1 = match outcome1 {
            ExecutionOutcome::ToolRequest(r) => r,
            _ => panic!("Expected ToolRequest"),
        };

        sandbox
            .write_tool_result(
                &request1.call_id,
                ToolResult::error("503 Service Unavailable"),
            )
            .await
            .expect("write error");

        // Agent can check the error
        let result = sandbox
            .execute("cat /tools/last_result.json", &limits())
            .await
            .expect("read error");

        let error: serde_json::Value =
            serde_json::from_str(&String::from_utf8_lossy(&result.stdout)).expect("parse");
        assert!(
            error["error"]
                .as_str()
                .unwrap()
                .contains("Service Unavailable")
        );

        // Retry succeeds
        let outcome2 = sandbox
            .execute_with_tools("tool api_call --endpoint /users", &limits())
            .await
            .expect("retry");

        let request2 = match outcome2 {
            ExecutionOutcome::ToolRequest(r) => r,
            _ => panic!("Expected ToolRequest"),
        };

        sandbox
            .write_tool_result(
                &request2.call_id,
                ToolResult::success(serde_json::json!({"users": ["alice", "bob"]})),
            )
            .await
            .expect("write success");

        // Verify success
        let result = sandbox
            .execute("cat /tools/last_result.json | jq -r '.users[0]'", &limits())
            .await
            .expect("read result");

        assert_eq!(result.exit_code, 0);
        assert!(String::from_utf8_lossy(&result.stdout).contains("alice"));

        // Both calls should be in history - verify by reading their metadata
        let result = sandbox
            .execute("cat /tools/history/call-001/metadata.json", &limits())
            .await
            .expect("read call-001");
        assert_eq!(result.exit_code, 0, "call-001 should exist");

        let result = sandbox
            .execute("cat /tools/history/call-002/metadata.json", &limits())
            .await
            .expect("read call-002");
        assert_eq!(result.exit_code, 0, "call-002 should exist");
    }

    /// Tests a code analysis workflow with multiple tools.
    #[tokio::test]
    async fn test_code_analysis_workflow() {
        let history = SimpleHistoryProvider::new()
            .with_transcript("U> Please analyze the main.rs file and suggest improvements.");

        let sandbox = AgentSandbox::builder("code-analyst")
            .params(serde_json::json!({
                "target": "src/main.rs",
                "analysis_type": "performance"
            }))
            .tool(ToolDefinition::no_params("file_read", "Read file contents"))
            .tool(ToolDefinition::no_params(
                "code_search",
                "Search for patterns in code",
            ))
            .history(Arc::new(history))
            .build()
            .await
            .expect("build sandbox");

        // Agent can see the user's request in history
        let result = sandbox
            .execute("grep 'analyze' /history/current/transcript.md", &limits())
            .await
            .expect("grep history");

        assert_eq!(result.exit_code, 0);
        assert!(String::from_utf8_lossy(&result.stdout).contains("main.rs"));

        // Agent reads the target file
        let outcome = sandbox
            .execute_with_tools("tool file_read --path src/main.rs", &limits())
            .await
            .expect("read file");

        let read_request = match outcome {
            ExecutionOutcome::ToolRequest(r) => r,
            _ => panic!("Expected ToolRequest"),
        };

        sandbox
            .write_tool_result(
                &read_request.call_id,
                ToolResult::success(serde_json::json!({
                    "content": "fn main() {\n    let data = vec![1, 2, 3];\n    for i in data {\n        println!(\"{}\", i);\n    }\n}"
                })),
            )
            .await
            .expect("write file content");

        // Agent searches for patterns
        let outcome = sandbox
            .execute_with_tools("tool code_search --pattern 'for.*in'", &limits())
            .await
            .expect("search");

        let search_request = match outcome {
            ExecutionOutcome::ToolRequest(r) => r,
            _ => panic!("Expected ToolRequest"),
        };

        sandbox
            .write_tool_result(
                &search_request.call_id,
                ToolResult::success(serde_json::json!({
                    "matches": [
                        {"line": 3, "text": "    for i in data {"}
                    ]
                })),
            )
            .await
            .expect("write search result");

        // Agent stores analysis in state
        let result = sandbox
            .execute(
                r#"
                echo '{"findings": ["Loop could use iterator methods"], "severity": "low"}' > /agent/state/analysis.json
                cat /agent/state/analysis.json
            "#,
                &limits(),
            )
            .await
            .expect("write analysis");

        assert_eq!(result.exit_code, 0);
        let analysis: serde_json::Value =
            serde_json::from_str(&String::from_utf8_lossy(&result.stdout)).expect("parse");
        assert!(
            analysis["findings"][0]
                .as_str()
                .unwrap()
                .contains("iterator")
        );
    }
}

// =============================================================================
// Edge Cases and Error Handling
// =============================================================================

mod edge_cases {
    use super::*;

    #[tokio::test]
    async fn test_large_tool_result() {
        let sandbox = AgentSandbox::builder("agent-123")
            .tool(ToolDefinition::no_params(
                "bulk_data",
                "Returns lots of data",
            ))
            .build()
            .await
            .expect("build sandbox");

        let outcome = sandbox
            .execute_with_tools("tool bulk_data", &limits())
            .await
            .expect("execute");

        let request = match outcome {
            ExecutionOutcome::ToolRequest(r) => r,
            _ => panic!("Expected ToolRequest"),
        };

        // Create a large result (100KB of data)
        let large_array: Vec<i32> = (0..10000).collect();
        let large_result = serde_json::json!({
            "data": large_array,
            "metadata": {"count": 10000}
        });

        sandbox
            .write_tool_result(&request.call_id, ToolResult::success(large_result))
            .await
            .expect("write large result");

        // Verify we can read it back
        let result = sandbox
            .execute(
                "cat /tools/last_result.json | jq '.metadata.count'",
                &limits(),
            )
            .await
            .expect("read");

        assert_eq!(result.exit_code, 0);
        assert!(String::from_utf8_lossy(&result.stdout).contains("10000"));
    }

    #[tokio::test]
    async fn test_special_characters_in_params() {
        let sandbox = AgentSandbox::builder("agent-123")
            .tool(ToolDefinition::no_params("echo_tool", "Echoes input"))
            .build()
            .await
            .expect("build sandbox");

        let outcome = sandbox
            .execute_with_tools(
                r#"tool echo_tool --json '{"message": "Hello \"World\"!\nNew line\ttab"}'"#,
                &limits(),
            )
            .await
            .expect("execute");

        match outcome {
            ExecutionOutcome::ToolRequest(request) => {
                let msg = request.params["message"].as_str().unwrap();
                assert!(msg.contains("Hello"));
                assert!(msg.contains("World"));
            }
            _ => panic!("Expected ToolRequest"),
        }
    }

    #[tokio::test]
    async fn test_unicode_in_transcript() {
        let transcript = r#"U> Can you help with this: 你好世界 🌍

A> Of course! That means "Hello World" in Chinese.

T[translate] {"text": "你好世界", "to": "en"}
R> {"translation": "Hello World", "confidence": 0.99}"#;

        let history = SimpleHistoryProvider::new().with_transcript(transcript);

        let sandbox = AgentSandbox::builder("agent-123")
            .history(Arc::new(history))
            .build()
            .await
            .expect("build sandbox");

        let result = sandbox
            .execute("cat /history/current/transcript.md", &limits())
            .await
            .expect("cat");

        assert_eq!(result.exit_code, 0);
        let stdout = String::from_utf8_lossy(&result.stdout);
        assert!(stdout.contains("你好世界"));
        assert!(stdout.contains("🌍"));
    }

    #[tokio::test]
    async fn test_concurrent_sandbox_isolation() {
        // Create two sandboxes with the same structure
        let sandbox1 = AgentSandbox::builder("agent-1")
            .params(serde_json::json!({"id": 1}))
            .build()
            .await
            .expect("build sandbox1");

        let sandbox2 = AgentSandbox::builder("agent-2")
            .params(serde_json::json!({"id": 2}))
            .build()
            .await
            .expect("build sandbox2");

        // Write to sandbox1
        sandbox1
            .execute("echo 'sandbox1 data' > /agent/scratch/test.txt", &limits())
            .await
            .expect("write 1");

        // Write to sandbox2
        sandbox2
            .execute("echo 'sandbox2 data' > /agent/scratch/test.txt", &limits())
            .await
            .expect("write 2");

        // Verify isolation
        let result1 = sandbox1
            .execute("cat /agent/scratch/test.txt", &limits())
            .await
            .expect("read 1");
        assert!(String::from_utf8_lossy(&result1.stdout).contains("sandbox1"));

        let result2 = sandbox2
            .execute("cat /agent/scratch/test.txt", &limits())
            .await
            .expect("read 2");
        assert!(String::from_utf8_lossy(&result2.stdout).contains("sandbox2"));

        // Verify metadata isolation
        let result1 = sandbox1
            .execute("cat /agent/params.json | jq .id", &limits())
            .await
            .expect("params 1");
        assert!(String::from_utf8_lossy(&result1.stdout).contains("1"));

        let result2 = sandbox2
            .execute("cat /agent/params.json | jq .id", &limits())
            .await
            .expect("params 2");
        assert!(String::from_utf8_lossy(&result2.stdout).contains("2"));
    }

    #[tokio::test]
    async fn test_empty_tool_params() {
        let sandbox = AgentSandbox::builder("agent-123")
            .tool(ToolDefinition::no_params(
                "no_args",
                "Tool with no arguments",
            ))
            .build()
            .await
            .expect("build sandbox");

        let outcome = sandbox
            .execute_with_tools("tool no_args", &limits())
            .await
            .expect("execute");

        match outcome {
            ExecutionOutcome::ToolRequest(request) => {
                assert_eq!(request.tool, "no_args");
                // Params should be empty object
                assert!(request.params.is_object());
            }
            _ => panic!("Expected ToolRequest"),
        }
    }
}
