//! Conch MCP Server
//!
//! This binary runs the Conch shell as an MCP server over stdio.
//! It exposes a `run_command` tool that allows AI assistants to execute
//! shell commands in a sandboxed WASM environment.
//!
//! # Mount Configuration
//!
//! You can mount host directories into the sandbox using `--mount`:
//!
//! ```bash
//! conch-mcp --mount /data:/home/user/data:ro --mount /workspace:/home/user/project:rw
//! ```
//!
//! Mount format: `<guest_path>:<host_path>[:ro|rw]`
//! - `guest_path`: Path visible inside the shell
//! - `host_path`: Real filesystem path to mount
//! - `ro` or `rw`: Read-only (default) or read-write access

use std::path::PathBuf;

use clap::Parser;
use conch_mcp::{ConchServer, MountConfig};
use rmcp::ServiceExt;
use tracing_subscriber::{EnvFilter, fmt, layer::SubscriberExt, util::SubscriberInitExt};

/// Conch MCP Server - Sandboxed shell execution for AI assistants
#[derive(Parser, Debug)]
#[command(name = "conch-mcp")]
#[command(about = "MCP server providing sandboxed shell execution")]
struct Args {
    /// Mount a host directory into the sandbox.
    /// Format: <guest_path>:<host_path>[:ro|rw]
    /// Examples:
    ///   --mount /data:/home/user/data:ro
    ///   --mount /workspace:/tmp/work:rw
    #[arg(long = "mount", value_name = "MOUNT")]
    mounts: Vec<String>,

    /// Maximum number of concurrent shell executions
    #[arg(long, default_value = "4")]
    max_concurrent: usize,
}

/// Parse a mount specification string into a MountConfig.
///
/// Format: `<guest_path>:<host_path>[:ro|rw]`
fn parse_mount(spec: &str) -> anyhow::Result<MountConfig> {
    let parts: Vec<&str> = spec.split(':').collect();

    match parts.len() {
        2 => {
            // guest:host (default to read-only)
            Ok(MountConfig {
                guest_path: parts[0].to_string(),
                host_path: PathBuf::from(parts[1]),
                readonly: true,
            })
        }
        3 => {
            // guest:host:mode
            let readonly = match parts[2] {
                "ro" => true,
                "rw" => false,
                other => anyhow::bail!(
                    "Invalid mount mode '{}'. Use 'ro' (read-only) or 'rw' (read-write)",
                    other
                ),
            };
            Ok(MountConfig {
                guest_path: parts[0].to_string(),
                host_path: PathBuf::from(parts[1]),
                readonly,
            })
        }
        _ => anyhow::bail!(
            "Invalid mount format '{}'. Expected <guest_path>:<host_path>[:ro|rw]",
            spec
        ),
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize tracing - output to stderr so it doesn't interfere with MCP stdio
    tracing_subscriber::registry()
        .with(fmt::layer().with_writer(std::io::stderr))
        .with(EnvFilter::from_default_env().add_directive(tracing::Level::INFO.into()))
        .init();

    let args = Args::parse();

    // Parse mount specifications
    let mounts: Vec<MountConfig> = args
        .mounts
        .iter()
        .map(|s| parse_mount(s))
        .collect::<anyhow::Result<Vec<_>>>()?;

    // Validate that mount paths exist
    for mount in &mounts {
        if !mount.host_path.exists() {
            anyhow::bail!(
                "Mount host path does not exist: {}",
                mount.host_path.display()
            );
        }
        if !mount.host_path.is_dir() {
            anyhow::bail!(
                "Mount host path is not a directory: {}",
                mount.host_path.display()
            );
        }
    }

    tracing::info!("Starting Conch MCP server");

    // Log mount configuration
    if mounts.is_empty() {
        tracing::info!("No filesystem mounts configured");
    } else {
        for mount in &mounts {
            tracing::info!(
                "Mount: {} -> {} ({})",
                mount.guest_path,
                mount.host_path.display(),
                if mount.readonly {
                    "read-only"
                } else {
                    "read-write"
                }
            );
        }
    }

    // Create the Conch server
    let server = ConchServer::new(args.max_concurrent, mounts);

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

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_mount_two_parts() {
        let mount = parse_mount("/data:/home/user/data").unwrap();
        assert_eq!(mount.guest_path, "/data");
        assert_eq!(mount.host_path, PathBuf::from("/home/user/data"));
        assert!(mount.readonly); // default
    }

    #[test]
    fn test_parse_mount_readonly() {
        let mount = parse_mount("/data:/home/user/data:ro").unwrap();
        assert_eq!(mount.guest_path, "/data");
        assert_eq!(mount.host_path, PathBuf::from("/home/user/data"));
        assert!(mount.readonly);
    }

    #[test]
    fn test_parse_mount_readwrite() {
        let mount = parse_mount("/workspace:/tmp/work:rw").unwrap();
        assert_eq!(mount.guest_path, "/workspace");
        assert_eq!(mount.host_path, PathBuf::from("/tmp/work"));
        assert!(!mount.readonly);
    }

    #[test]
    fn test_parse_mount_invalid_mode() {
        let result = parse_mount("/data:/home:invalid");
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Invalid mount mode")
        );
    }

    #[test]
    fn test_parse_mount_invalid_format() {
        let result = parse_mount("/data");
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Invalid mount format")
        );
    }
}
