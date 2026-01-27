//! External Tool Execution Example
//!
//! Demonstrates how an orchestrator executes tools outside the sandbox,
//! simulating a real agent loop where tool calls are handled by external
//! systems (APIs, databases, file systems, etc.).
//!
//! This example shows the complete flow:
//! 1. Agent script requests a tool via `tool` builtin
//! 2. Sandbox yields with ExecutionOutcome::ToolRequest
//! 3. Orchestrator dispatches to actual external implementation
//! 4. Result is written back to sandbox
//! 5. Agent can read and process the result
//!
//! Run with: cargo run -p conch --example external_tools --features embedded-shell

use std::collections::HashMap;

use conch::ResourceLimits;
use conch::agent::{AgentSandbox, ExecutionOutcome, ToolDefinition, ToolResult};

/// Simulated external tool implementations.
/// In a real system, these would call actual APIs, databases, etc.
struct ExternalToolExecutor {
    /// Simulated file system
    files: HashMap<String, String>,
    /// Simulated key-value store
    kv_store: HashMap<String, String>,
}

impl ExternalToolExecutor {
    fn new() -> Self {
        let mut files = HashMap::new();
        files.insert(
            "README.md".to_string(),
            "# My Project\n\nA sample project for testing.".to_string(),
        );
        files.insert(
            "src/main.rs".to_string(),
            r#"fn main() {
    println!("Hello, world!");
}
"#
            .to_string(),
        );
        files.insert(
            "Cargo.toml".to_string(),
            r#"[package]
name = "my-project"
version = "0.1.0"

[dependencies]
serde = "1.0"
tokio = "1.0"
"#
            .to_string(),
        );

        Self {
            files,
            kv_store: HashMap::new(),
        }
    }

    /// Execute a tool and return the result.
    /// This is where you'd integrate with real external systems.
    fn execute(&mut self, tool: &str, params: &serde_json::Value) -> ToolResult {
        match tool {
            "file_read" => self.file_read(params),
            "file_list" => self.file_list(params),
            "file_write" => self.file_write(params),
            "kv_get" => self.kv_get(params),
            "kv_set" => self.kv_set(params),
            "http_get" => self.http_get(params),
            _ => ToolResult::error(format!("Unknown tool: {}", tool)),
        }
    }

    fn file_read(&self, params: &serde_json::Value) -> ToolResult {
        let path = match params.get("path").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return ToolResult::error("Missing required parameter: path"),
        };

        match self.files.get(path) {
            Some(content) => ToolResult::success(serde_json::json!({
                "path": path,
                "content": content,
                "size": content.len()
            })),
            None => ToolResult::error(format!("File not found: {}", path)),
        }
    }

    fn file_list(&self, params: &serde_json::Value) -> ToolResult {
        let prefix = params.get("prefix").and_then(|v| v.as_str()).unwrap_or("");

        let files: Vec<&str> = self
            .files
            .keys()
            .filter(|k| k.starts_with(prefix))
            .map(|s| s.as_str())
            .collect();

        ToolResult::success(serde_json::json!({
            "files": files,
            "count": files.len()
        }))
    }

    fn file_write(&mut self, params: &serde_json::Value) -> ToolResult {
        let path = match params.get("path").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return ToolResult::error("Missing required parameter: path"),
        };
        let content = match params.get("content").and_then(|v| v.as_str()) {
            Some(c) => c,
            None => return ToolResult::error("Missing required parameter: content"),
        };

        self.files.insert(path.to_string(), content.to_string());

        ToolResult::success(serde_json::json!({
            "path": path,
            "written": content.len(),
            "success": true
        }))
    }

    fn kv_get(&self, params: &serde_json::Value) -> ToolResult {
        let key = match params.get("key").and_then(|v| v.as_str()) {
            Some(k) => k,
            None => return ToolResult::error("Missing required parameter: key"),
        };

        match self.kv_store.get(key) {
            Some(value) => ToolResult::success(serde_json::json!({
                "key": key,
                "value": value,
                "found": true
            })),
            None => ToolResult::success(serde_json::json!({
                "key": key,
                "value": null,
                "found": false
            })),
        }
    }

    fn kv_set(&mut self, params: &serde_json::Value) -> ToolResult {
        let key = match params.get("key").and_then(|v| v.as_str()) {
            Some(k) => k,
            None => return ToolResult::error("Missing required parameter: key"),
        };
        let value = match params.get("value").and_then(|v| v.as_str()) {
            Some(v) => v,
            None => return ToolResult::error("Missing required parameter: value"),
        };

        self.kv_store.insert(key.to_string(), value.to_string());

        ToolResult::success(serde_json::json!({
            "key": key,
            "stored": true
        }))
    }

    fn http_get(&self, params: &serde_json::Value) -> ToolResult {
        let url = match params.get("url").and_then(|v| v.as_str()) {
            Some(u) => u,
            None => return ToolResult::error("Missing required parameter: url"),
        };

        // Simulate HTTP responses based on URL
        if url.contains("api.github.com") {
            ToolResult::success(serde_json::json!({
                "status": 200,
                "body": {
                    "name": "example-repo",
                    "stars": 42,
                    "language": "Rust"
                }
            }))
        } else if url.contains("httpbin.org") {
            ToolResult::success(serde_json::json!({
                "status": 200,
                "body": {
                    "origin": "127.0.0.1",
                    "url": url
                }
            }))
        } else {
            ToolResult::error(format!("Connection refused: {}", url))
        }
    }
}

/// Run a script in the sandbox, handling any tool requests via external executor.
async fn run_with_external_tools(
    sandbox: &AgentSandbox,
    executor: &mut ExternalToolExecutor,
    script: &str,
    limits: &ResourceLimits,
) -> Result<conch::ExecutionResult, conch::RuntimeError> {
    let outcome = sandbox.execute_with_tools(script, limits).await?;

    match outcome {
        ExecutionOutcome::Completed(result) => Ok(result),
        ExecutionOutcome::ToolRequest(request) => {
            println!("  [Orchestrator] Tool request: {}", request.tool);
            println!("  [Orchestrator] Params: {}", request.params);

            // Execute tool externally
            let result = executor.execute(&request.tool, &request.params);

            let success = result.is_success();
            println!("  [Orchestrator] Result success: {}", success);

            // Write result back to sandbox
            sandbox.write_tool_result(&request.call_id, result).await?;

            // Return a synthetic result indicating tool was executed
            Ok(conch::ExecutionResult {
                exit_code: 0,
                stdout: format!(
                    "Tool {} executed (call_id: {})",
                    request.tool, request.call_id
                )
                .into_bytes(),
                stderr: Vec::new(),
                truncated: false,
                stats: Default::default(),
            })
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== External Tool Execution Example ===\n");

    // Create external tool executor (simulates real external systems)
    let mut executor = ExternalToolExecutor::new();

    // Create sandbox with tool definitions
    let sandbox = AgentSandbox::builder("file-agent")
        .name("File Operations Agent")
        .tool(ToolDefinition::new(
            "file_read",
            "Read contents of a file from the external filesystem",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "File path to read" }
                },
                "required": ["path"]
            }),
        ))
        .tool(ToolDefinition::new(
            "file_list",
            "List files in the external filesystem",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "prefix": { "type": "string", "description": "Optional path prefix filter" }
                }
            }),
        ))
        .tool(ToolDefinition::new(
            "file_write",
            "Write content to a file in the external filesystem",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "content": { "type": "string" }
                },
                "required": ["path", "content"]
            }),
        ))
        .tool(ToolDefinition::new(
            "kv_get",
            "Get a value from the key-value store",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "key": { "type": "string" }
                },
                "required": ["key"]
            }),
        ))
        .tool(ToolDefinition::new(
            "kv_set",
            "Set a value in the key-value store",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "key": { "type": "string" },
                    "value": { "type": "string" }
                },
                "required": ["key", "value"]
            }),
        ))
        .tool(ToolDefinition::new(
            "http_get",
            "Make an HTTP GET request",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "url": { "type": "string", "format": "uri" }
                },
                "required": ["url"]
            }),
        ))
        .build()
        .await?;

    let limits = ResourceLimits::default();

    // =========================================================================
    // Scenario 1: Read a file from the external filesystem
    // =========================================================================
    println!("--- Scenario 1: Reading External File ---\n");

    run_with_external_tools(
        &sandbox,
        &mut executor,
        "tool file_read --path README.md",
        &limits,
    )
    .await?;

    // Agent reads the result
    let result = sandbox
        .execute("cat /tools/last_result.json", &limits)
        .await?;
    println!("Agent sees file content:");
    println!("{}\n", String::from_utf8_lossy(&result.stdout));

    // =========================================================================
    // Scenario 2: List files and read specific ones
    // =========================================================================
    println!("--- Scenario 2: List and Read Files ---\n");

    // List all files
    run_with_external_tools(&sandbox, &mut executor, "tool file_list", &limits).await?;

    let result = sandbox
        .execute("cat /tools/last_result.json", &limits)
        .await?;
    println!("Available files:");
    println!("{}\n", String::from_utf8_lossy(&result.stdout));

    // Read source file
    run_with_external_tools(
        &sandbox,
        &mut executor,
        "tool file_read --path src/main.rs",
        &limits,
    )
    .await?;

    let result = sandbox
        .execute("cat /tools/last_result.json | jq -r .content", &limits)
        .await?;
    println!("Source code:");
    println!("{}", String::from_utf8_lossy(&result.stdout));

    // =========================================================================
    // Scenario 3: Write to external filesystem
    // =========================================================================
    println!("--- Scenario 3: Writing to External Filesystem ---\n");

    run_with_external_tools(
        &sandbox,
        &mut executor,
        r#"tool file_write --json '{"path": "output.txt", "content": "Generated by agent"}'"#,
        &limits,
    )
    .await?;

    let result = sandbox
        .execute("cat /tools/last_result.json", &limits)
        .await?;
    println!("Write result:");
    println!("{}\n", String::from_utf8_lossy(&result.stdout));

    // Verify by reading it back
    run_with_external_tools(
        &sandbox,
        &mut executor,
        "tool file_read --path output.txt",
        &limits,
    )
    .await?;

    let result = sandbox
        .execute("cat /tools/last_result.json | jq -r .content", &limits)
        .await?;
    println!("Read back: {}", String::from_utf8_lossy(&result.stdout));

    // =========================================================================
    // Scenario 4: Key-Value Store Operations
    // =========================================================================
    println!("--- Scenario 4: Key-Value Store ---\n");

    // Store a value
    run_with_external_tools(
        &sandbox,
        &mut executor,
        r#"tool kv_set --key "session_id" --value "abc123""#,
        &limits,
    )
    .await?;
    println!("Stored session_id");

    // Retrieve it
    run_with_external_tools(
        &sandbox,
        &mut executor,
        r#"tool kv_get --key "session_id""#,
        &limits,
    )
    .await?;

    let result = sandbox
        .execute("cat /tools/last_result.json", &limits)
        .await?;
    println!("Retrieved:");
    println!("{}\n", String::from_utf8_lossy(&result.stdout));

    // Try non-existent key
    run_with_external_tools(
        &sandbox,
        &mut executor,
        r#"tool kv_get --key "nonexistent""#,
        &limits,
    )
    .await?;

    let result = sandbox
        .execute("cat /tools/last_result.json | jq .found", &limits)
        .await?;
    println!(
        "Nonexistent key found: {}",
        String::from_utf8_lossy(&result.stdout)
    );

    // =========================================================================
    // Scenario 5: HTTP Request (simulated)
    // =========================================================================
    println!("--- Scenario 5: HTTP Request ---\n");

    run_with_external_tools(
        &sandbox,
        &mut executor,
        r#"tool http_get --url "https://api.github.com/repos/example/repo""#,
        &limits,
    )
    .await?;

    let result = sandbox
        .execute("cat /tools/last_result.json", &limits)
        .await?;
    println!("GitHub API response:");
    println!("{}\n", String::from_utf8_lossy(&result.stdout));

    // Failed request
    run_with_external_tools(
        &sandbox,
        &mut executor,
        r#"tool http_get --url "https://invalid.example.com/api""#,
        &limits,
    )
    .await?;

    let result = sandbox
        .execute("cat /tools/last_result.json", &limits)
        .await?;
    println!("Failed request:");
    println!("{}\n", String::from_utf8_lossy(&result.stdout));

    // =========================================================================
    // Scenario 6: Agent stores external data in sandbox scratch
    // =========================================================================
    println!("--- Scenario 6: Combining External and Sandbox Storage ---\n");

    // Read external file
    run_with_external_tools(
        &sandbox,
        &mut executor,
        "tool file_read --path Cargo.toml",
        &limits,
    )
    .await?;

    // Agent processes and stores in sandbox scratch
    let result = sandbox
        .execute(
            r#"
            cat /tools/last_result.json | jq -r .content > /agent/scratch/cargo_backup.toml
            echo "Backed up Cargo.toml to sandbox scratch"
            wc -l /agent/scratch/cargo_backup.toml
        "#,
            &limits,
        )
        .await?;
    println!("{}", String::from_utf8_lossy(&result.stdout));

    // Verify backup
    let result = sandbox
        .execute("cat /agent/scratch/cargo_backup.toml", &limits)
        .await?;
    println!("Backup contents:");
    println!("{}", String::from_utf8_lossy(&result.stdout));

    println!("=== Example Complete ===");

    Ok(())
}
