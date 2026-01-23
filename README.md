# Conch

A sandboxed shell execution engine using WebAssembly.

Named after the conch shell from *Lord of the Flies* — "whoever holds the conch gets to speak."

## Overview

Conch provides secure shell execution in a WASM sandbox using [wasmtime](https://wasmtime.dev/). It wraps [brush](https://github.com/reubeno/brush), a bash-compatible shell written in Rust, and provides a hybrid virtual filesystem that combines:

- **VFS storage**: In-memory or custom storage for orchestrator-controlled paths
- **Real filesystem**: cap-std secured mounts for host directory access

## Features

- **Full bash compatibility** via brush-core
- **Secure sandboxing** — WASM isolation with capability-based filesystem access
- **Hybrid VFS** — combine virtual storage with real filesystem mounts
- **Resource limits** — CPU time, memory, output size
- **Multiple APIs** — high-level `Shell` API or low-level `ComponentShellExecutor`
- **MCP server** — integrate with AI assistants via Model Context Protocol
- **FFI bindings** — C ABI for Go integration (via purego, no CGO required)

## Quick Start

### Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
conch = { version = "0.1", features = ["embedded-shell"] }
```

### Basic Usage

```rust
use conch::{Shell, ResourceLimits};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create a shell with default settings (VFS at /scratch)
    let shell = Shell::builder().build()?;
    
    // Write data to VFS
    shell.vfs().write("/scratch/input.txt", b"hello world").await?;
    
    // Execute commands
    let result = shell.execute(
        "cat /scratch/input.txt | wc -w",
        &ResourceLimits::default(),
    ).await?;
    
    println!("Output: {}", String::from_utf8_lossy(&result.stdout));
    println!("Exit code: {}", result.exit_code);
    
    Ok(())
}
```

### With Real Filesystem Mounts

```rust
use conch::{Shell, Mount, ResourceLimits};

let shell = Shell::builder()
    // Mount host directory as read-only
    .mount("/project", "/home/user/code", Mount::readonly())
    // Mount output directory as read-write
    .mount("/output", "/tmp/agent-output", Mount::readwrite())
    .build()?;

let result = shell.execute(
    "grep -r 'TODO' /project/src > /output/todos.txt",
    &ResourceLimits::default(),
).await?;
```

## Project Structure

```
conch/
├── crates/
│   ├── conch/           # Host library (Shell API, executor, FFI)
│   ├── conch-shell/     # Brush-based shell (compiles to WASM)
│   ├── conch-mcp/       # MCP server for AI assistants
│   └── conch-cli/       # CLI test harness
├── examples/            # Usage examples
├── tests/go/            # Go FFI integration tests
└── mise.toml            # Task definitions
```

## APIs

### Shell API (Recommended)

The `Shell` struct provides a high-level interface with builder pattern:

```rust
use conch::{Shell, Mount, DirPerms, FilePerms};

let shell = Shell::builder()
    // Custom VFS paths
    .vfs_path("/data", DirPerms::all(), FilePerms::all())
    .vfs_path("/config", DirPerms::READ, FilePerms::READ)
    // Real filesystem mounts
    .mount("/src", "./src", Mount::readonly())
    .build()?;
```

### Conch API (Simple)

For simple execution without VFS:

```rust
use conch::{Conch, ResourceLimits};

let conch = Conch::embedded(4)?;  // max 4 concurrent executions
let result = conch.execute("echo hello | cat", ResourceLimits::default()).await?;
```

### ComponentShellExecutor (Low-level)

For maximum control over the WASM runtime:

```rust
use conch::ComponentShellExecutor;

let executor = ComponentShellExecutor::embedded()?;
let result = executor.execute("echo hello", &limits).await?;
```

## MCP Server

Conch includes an MCP server for AI assistant integration:

```bash
# Build and run
cargo build -p conch-mcp --release
./target/release/conch-mcp
```

Configure in Claude Desktop:
```json
{
  "mcpServers": {
    "conch": {
      "command": "/path/to/conch-mcp"
    }
  }
}
```

## Building from Source

### Prerequisites

- Rust 1.85+ (1.92 recommended)
- [mise](https://mise.jdx.dev/) for task running (optional but recommended)

### Build Commands

```bash
# Install dependencies and build
mise install
mise run build

# Build the WASM shell module
mise run wasm-build

# Build with embedded shell (for deployment)
mise run build-embedded

# Run tests
mise run test-all
```

### CLI Usage

```bash
./target/debug/conch-cli -c "echo hello | cat"
./target/debug/conch-cli script.sh
```

## Go Integration

The library exposes a C FFI callable from Go using purego:

```go
import "github.com/yourorg/conch/tests/go"

executor, _ := conch.NewExecutorEmbedded()
defer executor.Close()

result, _ := executor.Execute("echo hello | grep hello")
fmt.Println(string(result.Stdout))  // "hello\n"
```

## Available Builtins

The shell includes these utilities:
- **Core**: echo, printf, test, true, false, exit
- **Text**: cat, head, tail, wc, grep, sort, uniq
- **Data**: jq (JSON processing)
- **Standard brush builtins**: cd, pwd, export, etc.

## Resource Limits

```rust
use conch::ResourceLimits;
use std::time::Duration;

let limits = ResourceLimits {
    max_cpu_ms: 5000,                    // 5 seconds CPU time
    max_memory_bytes: 64 * 1024 * 1024,  // 64 MB memory
    max_output_bytes: 1024 * 1024,       // 1 MB output
    timeout: Duration::from_secs(30),    // 30 second wall clock
};
```

## License

Apache-2.0
