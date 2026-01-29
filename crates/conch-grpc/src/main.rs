//! Conch gRPC Server
//!
//! Runs the Conch sandbox as a gRPC server that accepts bidirectional
//! streaming connections for shell execution with VFS callbacks.

use std::net::SocketAddr;

use clap::Parser;
use tracing_subscriber::{EnvFilter, fmt, layer::SubscriberExt, util::SubscriberInitExt};

use conch_grpc::SandboxServer;

/// Conch gRPC Server - Sandboxed shell execution with VFS callbacks
#[derive(Parser, Debug)]
#[command(name = "conch-grpc")]
#[command(about = "gRPC server providing sandboxed shell execution")]
struct Args {
    /// Address to listen on
    #[arg(long, default_value = "[::1]:50051")]
    addr: SocketAddr,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing
    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(EnvFilter::from_default_env().add_directive(tracing::Level::INFO.into()))
        .init();

    let args = Args::parse();

    let server = SandboxServer::new(args.addr);
    server.run().await?;

    Ok(())
}
