//! Tool Execution Cycle Example
//!
#![allow(clippy::expect_used, clippy::unwrap_used)]
//!
//! Demonstrates the callback-based tool invocation pattern:
//! 1. Agent script invokes a tool via the `tool` builtin
//! 2. The configured tool_handler callback is invoked immediately
//! 3. The handler executes the tool logic and returns a result
//! 4. The script receives the result and continues execution
//!
//! This pattern enables agents to use external tools (APIs, databases, etc.)
//! while maintaining sandbox isolation. The tool execution happens synchronously
//! from the script's perspective, but the handler can be async.
//!
//! Run with: cargo run -p conch --example tool_execution --features embedded-shell

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use conch::ResourceLimits;
use conch::agent::{AgentSandbox, ToolDefinition};
use conch::{ToolRequest, ToolResult};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Tool Execution Cycle Example ===\n");

    // Track how many tool calls we've handled
    let call_count = Arc::new(AtomicUsize::new(0));
    let call_count_clone = call_count.clone();

    // Create a tool handler that implements the actual tool logic
    // Note: request.params is a JSON string that needs to be parsed
    let tool_handler = move |request: ToolRequest| {
        let count = call_count_clone.clone();
        async move {
            let call_num = count.fetch_add(1, Ordering::SeqCst) + 1;
            println!(
                "  [Handler] Call #{}: {} with params {}",
                call_num, request.tool, request.params
            );

            // Parse params JSON string
            let params: serde_json::Value =
                serde_json::from_str(&request.params).unwrap_or_default();

            match request.tool.as_str() {
                "calculator" => {
                    let op = params["operation"].as_str().unwrap_or("add");
                    let a = params["a"].as_f64().unwrap_or(0.0);
                    let b = params["b"].as_f64().unwrap_or(0.0);

                    let result = match op {
                        "add" => a + b,
                        "subtract" => a - b,
                        "multiply" => a * b,
                        "divide" if b != 0.0 => a / b,
                        "divide" => {
                            return ToolResult {
                                success: false,
                                output: "Division by zero".to_string(),
                            };
                        }
                        _ => {
                            return ToolResult {
                                success: false,
                                output: format!("Unknown operation: {}", op),
                            };
                        }
                    };

                    println!("  [Handler] Calculated: {} {} {} = {}", a, op, b, result);
                    ToolResult {
                        success: true,
                        output: serde_json::to_string(&serde_json::json!({ "result": result }))
                            .unwrap(),
                    }
                }
                "web_fetch" => {
                    let url = params["url"].as_str().unwrap_or("unknown");
                    println!("  [Handler] Simulating fetch from: {}", url);

                    // Simulate different responses based on URL
                    if url.contains("api.example.com/data") {
                        ToolResult {
                            success: false,
                            output: "Connection refused: unable to reach api.example.com"
                                .to_string(),
                        }
                    } else if url.contains("api.example.com/numbers") {
                        ToolResult {
                            success: true,
                            output: serde_json::to_string(&serde_json::json!({
                                "numbers": [10, 20, 30, 40, 50]
                            }))
                            .unwrap(),
                        }
                    } else {
                        ToolResult {
                            success: true,
                            output: serde_json::to_string(&serde_json::json!({
                                "status": 200,
                                "body": "OK"
                            }))
                            .unwrap(),
                        }
                    }
                }
                "database_query" => {
                    let query = params["query"].as_str().unwrap_or("");
                    println!("  [Handler] Executing query: {}", query);

                    ToolResult {
                        success: true,
                        output: serde_json::to_string(&serde_json::json!({
                            "rows": [
                                {"id": 1, "name": "Alice", "role": "admin"},
                                {"id": 2, "name": "Bob", "role": "admin"}
                            ],
                            "count": 2,
                            "query_time_ms": 12
                        }))
                        .unwrap(),
                    }
                }
                _ => ToolResult {
                    success: false,
                    output: format!("Unknown tool: {}", request.tool),
                },
            }
        }
    };

    // Create a sandbox with several tools
    let mut sandbox = AgentSandbox::builder("worker-001")
        .tool(ToolDefinition::new(
            "calculator",
            "Perform mathematical calculations",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "operation": {
                        "type": "string",
                        "enum": ["add", "subtract", "multiply", "divide"]
                    },
                    "a": { "type": "number" },
                    "b": { "type": "number" }
                },
                "required": ["operation", "a", "b"]
            }),
        ))
        .tool(ToolDefinition::new(
            "web_fetch",
            "Fetch content from a URL",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "url": { "type": "string", "format": "uri" },
                    "method": { "type": "string", "default": "GET" }
                },
                "required": ["url"]
            }),
        ))
        .tool(ToolDefinition::new(
            "database_query",
            "Execute a database query",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string" },
                    "params": { "type": "array" }
                },
                "required": ["query"]
            }),
        ))
        .tool_handler(tool_handler)
        .build()
        .await?;

    let limits = ResourceLimits::default();

    // =========================================================================
    // Example 1: Simple Calculator Tool
    // =========================================================================
    println!("--- Example 1: Calculator Tool ---\n");

    let result = sandbox
        .execute("tool calculator --operation multiply --a 7 --b 6", &limits)
        .await?;

    println!("Exit code: {}", result.exit_code);
    println!("Output: {}", String::from_utf8_lossy(&result.stdout));

    // =========================================================================
    // Example 2: Tool with JSON Parameters
    // =========================================================================
    println!("\n--- Example 2: Complex JSON Parameters ---\n");

    let result = sandbox
        .execute(
            r#"tool database_query --json '{"query": "SELECT * FROM users WHERE role = ?", "params": ["admin"]}'"#,
            &limits,
        )
        .await?;

    println!("Exit code: {}", result.exit_code);
    println!("Output: {}", String::from_utf8_lossy(&result.stdout));

    // =========================================================================
    // Example 3: Error Handling
    // =========================================================================
    println!("\n--- Example 3: Error Handling ---\n");

    let result = sandbox
        .execute("tool web_fetch --url https://api.example.com/data", &limits)
        .await?;

    // Tool errors are returned in the output, exit code indicates tool success
    println!("Exit code: {}", result.exit_code);
    println!("Output: {}", String::from_utf8_lossy(&result.stdout));

    // =========================================================================
    // Example 4: Multi-Step Script with Tools
    // =========================================================================
    println!("\n--- Example 4: Multi-Step Script ---\n");

    // With callback-based tools, we can use tools inline in a script
    let result = sandbox
        .execute(
            r#"
            echo "Step 1: Fetching data..."
            tool web_fetch --url https://api.example.com/numbers

            echo ""
            echo "Step 2: Calculating sum doubled..."
            tool calculator --operation multiply --a 150 --b 2

            echo ""
            echo "Done!"
        "#,
            &limits,
        )
        .await?;

    println!("Multi-step script output:");
    println!("{}", String::from_utf8_lossy(&result.stdout));

    // =========================================================================
    // Example 5: Normal Commands Still Work
    // =========================================================================
    println!("\n--- Example 5: Normal Commands ---\n");

    let result = sandbox
        .execute(
            "echo 'This is a normal command' && cat /agent/metadata.json",
            &limits,
        )
        .await?;

    println!("Exit code: {}", result.exit_code);
    println!("Output: {}", String::from_utf8_lossy(&result.stdout));

    // =========================================================================
    // Summary
    // =========================================================================
    println!("\n--- Summary ---");
    println!(
        "Total tool calls handled: {}",
        call_count.load(Ordering::SeqCst)
    );

    println!("\n=== Example Complete ===");

    Ok(())
}
