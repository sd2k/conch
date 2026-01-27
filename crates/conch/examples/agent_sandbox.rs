//! Agent Sandbox Example
//!
//! Demonstrates the AgentSandbox API for creating isolated agent execution
//! environments with tool access and conversation history.
//!
//! Run with: cargo run -p conch --example agent_sandbox --features embedded-shell

use std::sync::Arc;

use conch::ResourceLimits;
use conch::agent::{
    AgentSandbox, ConversationMetadata, ExecutionOutcome, SimpleHistoryProvider, ToolDefinition,
    ToolResult,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Agent Sandbox Example ===\n");

    // Create a history provider with conversation context
    let history = SimpleHistoryProvider::new()
        .with_transcript(
            r#"U> Can you help me analyze this project's dependencies?

A> I'll check the Cargo.toml file for you."#,
        )
        .with_metadata(ConversationMetadata {
            id: "current".to_string(),
            title: Some("Dependency Analysis".to_string()),
            started_at: "2024-01-15T10:00:00Z".to_string(),
            updated_at: None,
            user_message_count: 1,
            assistant_message_count: 1,
            tool_call_count: 0,
        });

    // Build an agent sandbox with tools and configuration
    let sandbox = AgentSandbox::builder("analyst-001")
        .name("Code Analyst")
        .params(serde_json::json!({
            "task": "Analyze project dependencies",
            "target_file": "Cargo.toml",
            "output_format": "json"
        }))
        .capability("read_files")
        .capability("analyze_code")
        // Register available tools
        .tool(ToolDefinition::new(
            "file_read",
            "Read the contents of a file",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Path to the file to read"
                    }
                },
                "required": ["path"]
            }),
        ))
        .tool(ToolDefinition::new(
            "dependency_check",
            "Check for outdated or vulnerable dependencies",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "manifest_path": {
                        "type": "string",
                        "description": "Path to Cargo.toml or package.json"
                    }
                },
                "required": ["manifest_path"]
            }),
        ))
        .tool(ToolDefinition::no_params(
            "list_files",
            "List files in the project directory",
        ))
        // Attach conversation history
        .history(Arc::new(history))
        .build()
        .await?;

    let limits = ResourceLimits::default();

    // === Explore the VFS Structure ===
    // Note: The embedded shell has limited commands (cat, grep, jq, wc, head, tail, tool)
    // We demonstrate the VFS by reading key files
    println!("--- VFS Structure (via file reads) ---");
    println!("Available paths: /agent/, /tools/, /history/");

    // === Read Agent Metadata ===
    println!("\n--- Agent Metadata ---");
    let result = sandbox.execute("cat /agent/metadata.json", &limits).await?;
    println!("{}", String::from_utf8_lossy(&result.stdout));

    // === Read Task Parameters ===
    println!("\n--- Task Parameters ---");
    let result = sandbox.execute("cat /agent/params.json", &limits).await?;
    println!("{}", String::from_utf8_lossy(&result.stdout));

    // === List Available Tools ===
    println!("\n--- Available Tools ---");
    let result = sandbox.execute("cat /tools/index.txt", &limits).await?;
    println!("{}", String::from_utf8_lossy(&result.stdout));

    // === Read Tool Definition ===
    println!("\n--- file_read Tool Definition ---");
    let result = sandbox
        .execute("cat /tools/available/file_read.json", &limits)
        .await?;
    println!("{}", String::from_utf8_lossy(&result.stdout));

    // === Access Conversation History ===
    println!("\n--- Conversation History ---");
    let result = sandbox
        .execute("cat /history/current/transcript.md", &limits)
        .await?;
    println!("{}", String::from_utf8_lossy(&result.stdout));

    // === Invoke a Tool ===
    println!("\n--- Invoking file_read Tool ---");
    let outcome = sandbox
        .execute_with_tools("tool file_read --path Cargo.toml", &limits)
        .await?;

    match outcome {
        ExecutionOutcome::ToolRequest(request) => {
            println!("Tool request received:");
            println!("  Call ID: {}", request.call_id);
            println!("  Tool: {}", request.tool);
            println!("  Params: {}", request.params);

            // Simulate orchestrator executing the tool
            println!("\n--- Simulating Tool Execution ---");
            let tool_result = serde_json::json!({
                "content": "[package]\nname = \"my-project\"\nversion = \"0.1.0\"\n\n[dependencies]\nserde = \"1.0\"\ntokio = { version = \"1.0\", features = [\"full\"] }",
                "path": "Cargo.toml",
                "size": 142
            });

            // Write result back to sandbox
            sandbox
                .write_tool_result(&request.call_id, ToolResult::success(tool_result))
                .await?;
            println!("Tool result written to VFS");

            // === Agent Reads Result ===
            println!("\n--- Agent Reading Tool Result ---");
            let result = sandbox
                .execute("cat /tools/last_result.json", &limits)
                .await?;
            println!("{}", String::from_utf8_lossy(&result.stdout));

            // === Check Tool History ===
            println!("\n--- Tool History ---");
            let result = sandbox
                .execute(
                    &format!("cat /tools/history/{}/metadata.json", request.call_id),
                    &limits,
                )
                .await?;
            println!("{}", String::from_utf8_lossy(&result.stdout));
        }
        ExecutionOutcome::Completed(result) => {
            println!(
                "Unexpected completion (exit {}): {}",
                result.exit_code,
                String::from_utf8_lossy(&result.stdout)
            );
        }
    }

    // === Write to Scratch Space ===
    println!("\n--- Writing Analysis to Scratch ---");
    let result = sandbox
        .execute(
            r#"
            echo '{"status": "complete", "dependencies": 2}' > /agent/scratch/analysis.json
            cat /agent/scratch/analysis.json
        "#,
            &limits,
        )
        .await?;
    println!("{}", String::from_utf8_lossy(&result.stdout));

    // === Store Persistent State ===
    println!("\n--- Storing State ---");
    let result = sandbox
        .execute(
            r#"
            echo '{"last_analysis": "2024-01-15", "total_runs": 1}' > /agent/state/progress.json
            cat /agent/state/progress.json
        "#,
            &limits,
        )
        .await?;
    println!("{}", String::from_utf8_lossy(&result.stdout));

    println!("\n=== Example Complete ===");

    Ok(())
}
