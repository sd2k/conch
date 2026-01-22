//! Conch MCP Server
//!
//! This binary runs the Conch shell as an MCP server over stdio.
//! It exposes a `run_command` tool that allows AI assistants to execute
//! shell commands in a sandboxed WASM environment.

use conch_mcp::ConchServer;
use rmcp::ServiceExt;
use tracing_subscriber::{EnvFilter, fmt, layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize tracing - output to stderr so it doesn't interfere with MCP stdio
    tracing_subscriber::registry()
        .with(fmt::layer().with_writer(std::io::stderr))
        .with(EnvFilter::from_default_env().add_directive(tracing::Level::INFO.into()))
        .init();

    tracing::info!("Starting Conch MCP server");

    // Create the Conch server with max 4 concurrent executions
    let server = ConchServer::new(4)?;

    // Serve over stdio
    let service = server
        .serve(rmcp::transport::stdio())
        .await
        .inspect_err(|e| {
            tracing::error!("Failed to start MCP service: {}", e);
        })?;

    tracing::info!("Conch MCP server running");

    // Wait for the service to complete
    service.waiting().await?;

    tracing::info!("Conch MCP server shutting down");

    Ok(())
}
