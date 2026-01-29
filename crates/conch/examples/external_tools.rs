//! External Tool Execution Example
//!
//! Demonstrates how an orchestrator executes tools outside the sandbox,
//! simulating a real agent loop where tool calls are handled by external
//! systems (APIs, databases, file systems, etc.).
//!
//! This example shows the callback-based flow:
//! 1. Agent script requests a tool via `tool` builtin
//! 2. Tool handler callback is invoked with the request
//! 3. Handler dispatches to actual external implementation
//! 4. Result is returned to the script immediately
//!
//! Run with: cargo run -p conch --example external_tools --features embedded-shell

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use conch::ResourceLimits;
use conch::agent::{AgentSandbox, ToolDefinition};
use conch::{ToolRequest, ToolResult};

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
            _ => ToolResult {
                success: false,
                output: format!("Unknown tool: {}", tool),
            },
        }
    }

    fn file_read(&self, params: &serde_json::Value) -> ToolResult {
        let path = match params.get("path").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => {
                return ToolResult {
                    success: false,
                    output: "Missing required parameter: path".to_string(),
                };
            }
        };

        match self.files.get(path) {
            Some(content) => ToolResult {
                success: true,
                output: serde_json::to_string(&serde_json::json!({
                    "path": path,
                    "content": content,
                    "size": content.len()
                }))
                .unwrap(),
            },
            None => ToolResult {
                success: false,
                output: format!("File not found: {}", path),
            },
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

        ToolResult {
            success: true,
            output: serde_json::to_string(&serde_json::json!({
                "files": files,
                "count": files.len()
            }))
            .unwrap(),
        }
    }

    fn file_write(&mut self, params: &serde_json::Value) -> ToolResult {
        let path = match params.get("path").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => {
                return ToolResult {
                    success: false,
                    output: "Missing required parameter: path".to_string(),
                };
            }
        };
        let content = match params.get("content").and_then(|v| v.as_str()) {
            Some(c) => c,
            None => {
                return ToolResult {
                    success: false,
                    output: "Missing required parameter: content".to_string(),
                };
            }
        };

        self.files.insert(path.to_string(), content.to_string());

        ToolResult {
            success: true,
            output: serde_json::to_string(&serde_json::json!({
                "path": path,
                "written": content.len(),
                "success": true
            }))
            .unwrap(),
        }
    }

    fn kv_get(&self, params: &serde_json::Value) -> ToolResult {
        let key = match params.get("key").and_then(|v| v.as_str()) {
            Some(k) => k,
            None => {
                return ToolResult {
                    success: false,
                    output: "Missing required parameter: key".to_string(),
                };
            }
        };

        match self.kv_store.get(key) {
            Some(value) => ToolResult {
                success: true,
                output: serde_json::to_string(&serde_json::json!({
                    "key": key,
                    "value": value,
                    "found": true
                }))
                .unwrap(),
            },
            None => ToolResult {
                success: true,
                output: serde_json::to_string(&serde_json::json!({
                    "key": key,
                    "value": null,
                    "found": false
                }))
                .unwrap(),
            },
        }
    }

    fn kv_set(&mut self, params: &serde_json::Value) -> ToolResult {
        let key = match params.get("key").and_then(|v| v.as_str()) {
            Some(k) => k,
            None => {
                return ToolResult {
                    success: false,
                    output: "Missing required parameter: key".to_string(),
                };
            }
        };
        let value = match params.get("value").and_then(|v| v.as_str()) {
            Some(v) => v,
            None => {
                return ToolResult {
                    success: false,
                    output: "Missing required parameter: value".to_string(),
                };
            }
        };

        self.kv_store.insert(key.to_string(), value.to_string());

        ToolResult {
            success: true,
            output: serde_json::to_string(&serde_json::json!({
                "key": key,
                "stored": true
            }))
            .unwrap(),
        }
    }

    fn http_get(&self, params: &serde_json::Value) -> ToolResult {
        let url = match params.get("url").and_then(|v| v.as_str()) {
            Some(u) => u,
            None => {
                return ToolResult {
                    success: false,
                    output: "Missing required parameter: url".to_string(),
                };
            }
        };

        // Simulate HTTP responses based on URL
        if url.contains("api.github.com") {
            ToolResult {
                success: true,
                output: serde_json::to_string(&serde_json::json!({
                    "status": 200,
                    "body": {
                        "name": "example-repo",
                        "stars": 42,
                        "language": "Rust"
                    }
                }))
                .unwrap(),
            }
        } else if url.contains("httpbin.org") {
            ToolResult {
                success: true,
                output: serde_json::to_string(&serde_json::json!({
                    "status": 200,
                    "body": {
                        "origin": "127.0.0.1",
                        "url": url
                    }
                }))
                .unwrap(),
            }
        } else {
            ToolResult {
                success: false,
                output: format!("Connection refused: {}", url),
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== External Tool Execution Example ===\n");

    // Create external tool executor (simulates real external systems)
    // Wrap in Arc<Mutex> so it can be shared with the async tool handler
    let executor = Arc::new(Mutex::new(ExternalToolExecutor::new()));
    let executor_clone = executor.clone();

    // Create the tool handler that dispatches to the external executor
    // Note: request.params is a JSON string that needs to be parsed
    let tool_handler = move |request: ToolRequest| {
        let exec = executor_clone.clone();
        async move {
            println!("  [Orchestrator] Tool request: {}", request.tool);
            println!("  [Orchestrator] Params: {}", request.params);

            // Parse params JSON string
            let params: serde_json::Value =
                serde_json::from_str(&request.params).unwrap_or_default();

            let result = exec.lock().unwrap().execute(&request.tool, &params);

            println!("  [Orchestrator] Result success: {}", result.success);
            result
        }
    };

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
        .tool_handler(tool_handler)
        .build()
        .await?;

    let limits = ResourceLimits::default();

    // =========================================================================
    // Scenario 1: Read a file from the external filesystem
    // =========================================================================
    println!("--- Scenario 1: Reading External File ---\n");

    let result = sandbox
        .execute("tool file_read --path README.md", &limits)
        .await?;

    println!("Agent sees file content:");
    println!("{}\n", String::from_utf8_lossy(&result.stdout));

    // =========================================================================
    // Scenario 2: List files and read specific ones
    // =========================================================================
    println!("--- Scenario 2: List and Read Files ---\n");

    let result = sandbox.execute("tool file_list", &limits).await?;

    println!("Available files:");
    println!("{}\n", String::from_utf8_lossy(&result.stdout));

    // Read source file
    let result = sandbox
        .execute("tool file_read --path src/main.rs", &limits)
        .await?;

    println!("Source code:");
    println!("{}", String::from_utf8_lossy(&result.stdout));

    // =========================================================================
    // Scenario 3: Write to external filesystem
    // =========================================================================
    println!("\n--- Scenario 3: Writing to External Filesystem ---\n");

    let result = sandbox
        .execute(
            r#"tool file_write --json '{"path": "output.txt", "content": "Generated by agent"}'"#,
            &limits,
        )
        .await?;

    println!("Write result:");
    println!("{}\n", String::from_utf8_lossy(&result.stdout));

    // Verify by reading it back
    let result = sandbox
        .execute("tool file_read --path output.txt", &limits)
        .await?;

    println!("Read back: {}", String::from_utf8_lossy(&result.stdout));

    // =========================================================================
    // Scenario 4: Key-Value Store Operations
    // =========================================================================
    println!("\n--- Scenario 4: Key-Value Store ---\n");

    // Store a value
    let result = sandbox
        .execute(
            r#"tool kv_set --key "session_id" --value "abc123""#,
            &limits,
        )
        .await?;
    println!(
        "Stored session_id: {}",
        String::from_utf8_lossy(&result.stdout)
    );

    // Retrieve it
    let result = sandbox
        .execute(r#"tool kv_get --key "session_id""#, &limits)
        .await?;

    println!("Retrieved: {}\n", String::from_utf8_lossy(&result.stdout));

    // Try non-existent key
    let result = sandbox
        .execute(r#"tool kv_get --key "nonexistent""#, &limits)
        .await?;

    println!(
        "Nonexistent key: {}",
        String::from_utf8_lossy(&result.stdout)
    );

    // =========================================================================
    // Scenario 5: HTTP Request (simulated)
    // =========================================================================
    println!("\n--- Scenario 5: HTTP Request ---\n");

    let result = sandbox
        .execute(
            r#"tool http_get --url "https://api.github.com/repos/example/repo""#,
            &limits,
        )
        .await?;

    println!("GitHub API response:");
    println!("{}\n", String::from_utf8_lossy(&result.stdout));

    // Failed request
    let result = sandbox
        .execute(
            r#"tool http_get --url "https://invalid.example.com/api""#,
            &limits,
        )
        .await?;

    println!("Failed request:");
    println!("{}\n", String::from_utf8_lossy(&result.stdout));

    // =========================================================================
    // Scenario 6: Multi-tool script
    // =========================================================================
    println!("--- Scenario 6: Multi-Tool Script ---\n");

    // Read external file, write to sandbox scratch
    let result = sandbox
        .execute(
            r#"
            echo "Reading Cargo.toml from external filesystem..."
            tool file_read --path Cargo.toml

            echo ""
            echo "Done!"
        "#,
            &limits,
        )
        .await?;

    println!("{}", String::from_utf8_lossy(&result.stdout));

    println!("\n=== Example Complete ===");

    Ok(())
}
