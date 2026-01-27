//! Tool Execution Cycle Example
//!
//! Demonstrates the complete tool invocation lifecycle:
//! 1. Agent script invokes a tool via the `tool` builtin
//! 2. Sandbox yields with ExecutionOutcome::ToolRequest
//! 3. Orchestrator executes the tool externally
//! 4. Orchestrator writes result back via write_tool_result()
//! 5. Agent script can read the result
//!
//! This pattern enables agents to use external tools (APIs, databases, etc.)
//! while maintaining sandbox isolation.
//!
//! Run with: cargo run -p conch --example tool_execution --features embedded-shell

use conch::ResourceLimits;
use conch::agent::{AgentSandbox, ExecutionOutcome, ToolDefinition, ToolResult};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Tool Execution Cycle Example ===\n");

    // Create a sandbox with several tools
    let sandbox = AgentSandbox::builder("worker-001")
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
        .build()
        .await?;

    let limits = ResourceLimits::default();

    // =========================================================================
    // Example 1: Simple Calculator Tool
    // =========================================================================
    println!("--- Example 1: Calculator Tool ---\n");

    let outcome = sandbox
        .execute_with_tools("tool calculator --operation multiply --a 7 --b 6", &limits)
        .await?;

    match outcome {
        ExecutionOutcome::ToolRequest(request) => {
            println!("Tool requested: {}", request.tool);
            println!("Call ID: {}", request.call_id);
            println!(
                "Parameters: {}",
                serde_json::to_string_pretty(&request.params)?
            );

            // Simulate executing the calculator
            let result = match request.params["operation"].as_str() {
                Some("multiply") => {
                    let a = request.params["a"].as_f64().unwrap_or(0.0);
                    let b = request.params["b"].as_f64().unwrap_or(0.0);
                    serde_json::json!({ "result": a * b })
                }
                _ => serde_json::json!({ "error": "Unknown operation" }),
            };

            println!("\nExecuting calculation...");
            println!("Result: {}", result);

            // Write result back to sandbox
            sandbox
                .write_tool_result(&request.call_id, ToolResult::success(result))
                .await?;

            // Agent can now access the result
            let read_result = sandbox
                .execute("cat /tools/last_result.json | jq .result", &limits)
                .await?;
            println!(
                "Agent reads result: {}",
                String::from_utf8_lossy(&read_result.stdout).trim()
            );
        }
        ExecutionOutcome::Completed(r) => {
            println!("Unexpected completion: exit code {}", r.exit_code);
        }
    }

    // =========================================================================
    // Example 2: Tool with JSON Parameters
    // =========================================================================
    println!("\n--- Example 2: Complex JSON Parameters ---\n");

    let outcome = sandbox
        .execute_with_tools(
            r#"tool database_query --json '{"query": "SELECT * FROM users WHERE role = ?", "params": ["admin"]}'"#,
            &limits,
        )
        .await?;

    match outcome {
        ExecutionOutcome::ToolRequest(request) => {
            println!("Database query requested:");
            println!("  Query: {}", request.params["query"]);
            println!("  Params: {}", request.params["params"]);

            // Simulate database query result
            let db_result = serde_json::json!({
                "rows": [
                    {"id": 1, "name": "Alice", "role": "admin"},
                    {"id": 2, "name": "Bob", "role": "admin"}
                ],
                "count": 2,
                "query_time_ms": 12
            });

            sandbox
                .write_tool_result(&request.call_id, ToolResult::success(db_result))
                .await?;

            // Agent processes the result
            let result = sandbox
                .execute("cat /tools/last_result.json | jq '.rows[].name'", &limits)
                .await?;
            println!("Users found:\n{}", String::from_utf8_lossy(&result.stdout));
        }
        _ => {}
    }

    // =========================================================================
    // Example 3: Error Handling
    // =========================================================================
    println!("\n--- Example 3: Error Handling ---\n");

    let outcome = sandbox
        .execute_with_tools("tool web_fetch --url https://api.example.com/data", &limits)
        .await?;

    match outcome {
        ExecutionOutcome::ToolRequest(request) => {
            println!("Fetch requested: {}", request.params["url"]);

            // Simulate a network error
            sandbox
                .write_tool_result(
                    &request.call_id,
                    ToolResult::error("Connection refused: unable to reach api.example.com"),
                )
                .await?;

            // Agent sees the error
            let result = sandbox
                .execute("cat /tools/last_result.json", &limits)
                .await?;
            println!(
                "Error response:\n{}",
                String::from_utf8_lossy(&result.stdout)
            );

            // Check tool history metadata shows failure
            let result = sandbox
                .execute(
                    &format!(
                        "cat /tools/history/{}/metadata.json | jq .success",
                        request.call_id
                    ),
                    &limits,
                )
                .await?;
            println!(
                "Success status: {}",
                String::from_utf8_lossy(&result.stdout).trim()
            );
        }
        _ => {}
    }

    // =========================================================================
    // Example 4: Multi-Step Workflow
    // =========================================================================
    println!("\n--- Example 4: Multi-Step Workflow ---\n");

    // Step 1: Fetch data
    println!("Step 1: Fetching data...");
    let outcome = sandbox
        .execute_with_tools(
            "tool web_fetch --url https://api.example.com/numbers",
            &limits,
        )
        .await?;

    if let ExecutionOutcome::ToolRequest(request) = outcome {
        sandbox
            .write_tool_result(
                &request.call_id,
                ToolResult::success(serde_json::json!({
                    "numbers": [10, 20, 30, 40, 50]
                })),
            )
            .await?;
        println!("  Data fetched: [10, 20, 30, 40, 50]");
    }

    // Step 2: Process with calculator
    println!("Step 2: Calculating sum...");

    // Store intermediate result
    sandbox
        .execute(
            "cat /tools/last_result.json | jq '.numbers | add' > /agent/scratch/sum.txt",
            &limits,
        )
        .await?;

    // Use calculator to multiply
    let outcome = sandbox
        .execute_with_tools(
            "tool calculator --operation multiply --a 150 --b 2",
            &limits,
        )
        .await?;

    if let ExecutionOutcome::ToolRequest(request) = outcome {
        sandbox
            .write_tool_result(
                &request.call_id,
                ToolResult::success(serde_json::json!({ "result": 300 })),
            )
            .await?;
        println!("  Sum doubled: 300");
    }

    // Step 3: Store final result
    println!("Step 3: Storing result...");
    let result = sandbox
        .execute(
            r#"
            cat /tools/last_result.json > /agent/state/final_result.json
            echo "Workflow complete. Final result:"
            cat /agent/state/final_result.json
        "#,
            &limits,
        )
        .await?;
    println!("{}", String::from_utf8_lossy(&result.stdout));

    // =========================================================================
    // Example 5: Checking Tool History
    // =========================================================================
    println!("\n--- Example 5: Tool History ---\n");

    // Read metadata for each known call (ls not available in embedded shell)
    println!("Tool execution summary:");
    for call_id in ["call-001", "call-002", "call-003", "call-004", "call-005"] {
        let path = format!("/tools/history/{}/metadata.json", call_id);
        if let Ok(result) = sandbox.execute(&format!("cat {}", path), &limits).await {
            if result.exit_code == 0 {
                let metadata: serde_json::Value =
                    serde_json::from_slice(&result.stdout).unwrap_or_default();
                let success = metadata["success"].as_bool().unwrap_or(false);
                println!("  {}: success={}", call_id, success);
            }
        }
    }

    // =========================================================================
    // Example 6: Normal Commands Complete Normally
    // =========================================================================
    println!("\n--- Example 6: Normal Commands ---\n");

    let outcome = sandbox
        .execute_with_tools(
            "echo 'This is a normal command' && cat /agent/metadata.json",
            &limits,
        )
        .await?;

    match outcome {
        ExecutionOutcome::Completed(result) => {
            println!(
                "Normal command completed with exit code {}",
                result.exit_code
            );
            println!("Output: {}", String::from_utf8_lossy(&result.stdout));
        }
        ExecutionOutcome::ToolRequest(_) => {
            println!("Unexpected tool request!");
        }
    }

    println!("\n=== Example Complete ===");

    Ok(())
}
