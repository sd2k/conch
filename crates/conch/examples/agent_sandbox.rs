//! Agent Sandbox Example
//!
//! Demonstrates the AgentSandbox API for creating isolated agent execution
//! environments with tool access and conversation history.
//!
//! Run with: cargo run -p conch --example agent_sandbox --features embedded-shell

use std::sync::Arc;

use conch::ResourceLimits;
use conch::agent::{AgentSandbox, ConversationMetadata, SimpleHistoryProvider, ToolDefinition};
use conch::{ToolRequest, ToolResult};

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

    // Create a tool handler that will be called when the agent invokes tools
    // Note: request.params is a JSON string that needs to be parsed
    let tool_handler = |request: ToolRequest| async move {
        println!("  [Tool Handler] Received request for: {}", request.tool);
        println!("  [Tool Handler] Params: {}", request.params);

        // Parse params JSON string
        let params: serde_json::Value = serde_json::from_str(&request.params).unwrap_or_default();

        match request.tool.as_str() {
            "file_read" => {
                let path = params["path"].as_str().unwrap_or("unknown");
                println!("  [Tool Handler] Simulating file read for: {}", path);
                ToolResult {
                    success: true,
                    output: serde_json::to_string(&serde_json::json!({
                        "content": "[package]\nname = \"my-project\"\nversion = \"0.1.0\"\n\n[dependencies]\nserde = \"1.0\"\ntokio = { version = \"1.0\", features = [\"full\"] }",
                        "path": path,
                        "size": 142
                    })).unwrap(),
                }
            }
            "dependency_check" => ToolResult {
                success: true,
                output: serde_json::to_string(&serde_json::json!({
                    "outdated": [],
                    "vulnerable": [],
                    "total": 2
                }))
                .unwrap(),
            },
            "list_files" => ToolResult {
                success: true,
                output: serde_json::to_string(&serde_json::json!({
                    "files": ["Cargo.toml", "src/main.rs", "README.md"]
                }))
                .unwrap(),
            },
            _ => ToolResult {
                success: false,
                output: format!("Unknown tool: {}", request.tool),
            },
        }
    };

    // Build an agent sandbox with tools and configuration
    let mut sandbox = AgentSandbox::builder("analyst-001")
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
        // Set up tool handler callback
        .tool_handler(tool_handler)
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
    // With the callback-based approach, tool execution happens inline during script execution
    println!("\n--- Invoking file_read Tool ---");
    let result = sandbox
        .execute("tool file_read --path Cargo.toml", &limits)
        .await?;

    println!("Tool result (exit code {}):", result.exit_code);
    println!("{}", String::from_utf8_lossy(&result.stdout));

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
