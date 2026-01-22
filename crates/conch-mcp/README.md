# conch-mcp

An MCP (Model Context Protocol) server that exposes Conch's sandboxed shell execution as a tool.

## Overview

This crate provides an MCP server that allows AI assistants to execute shell commands in a secure WASM-based sandbox. The shell is a bash-compatible environment (brush) running inside a WebAssembly sandbox with strict resource limits.

## Features

- **Sandboxed Execution**: Commands run in a WASM sandbox with no network or filesystem access beyond what's explicitly provided
- **Resource Limits**: Configurable CPU time, memory, and output size limits
- **Bash Compatibility**: Supports bash-compatible syntax including pipes, redirects, and common utilities
- **MCP Protocol**: Communicates over stdio using the Model Context Protocol

## Build Requirements

Before building `conch-mcp`, you need to build the WASM shell module:

```bash
# Build the WASM shell module first
mise run wasm-build

# Then build the MCP server with embedded shell
mise run build-embedded

# Or build just conch-mcp
cargo build -p conch-mcp --release
```

The build requires the `embedded-shell` feature which embeds the pre-built WASM module into the binary.

## Usage

### Running the Server

```bash
# Run the MCP server (communicates over stdio)
./target/release/conch-mcp
```

### Configuring with Claude Desktop

Add to your Claude Desktop configuration (`~/Library/Application Support/Claude/claude_desktop_config.json` on macOS):

```json
{
  "mcpServers": {
    "conch": {
      "command": "/path/to/conch-mcp"
    }
  }
}
```

### Configuring with Zed

Add to your Zed settings:

```json
{
  "context_servers": {
    "conch": {
      "command": {
        "path": "/path/to/conch-mcp"
      }
    }
  }
}
```

## Available Tools

### `execute`

Execute a shell command in the Conch sandbox.

**Parameters:**

| Name | Type | Required | Description |
|------|------|----------|-------------|
| `command` | string | Yes | The shell command or script to execute |
| `max_cpu_ms` | integer | No | Maximum CPU time in milliseconds (default: 5000) |
| `max_memory_bytes` | integer | No | Maximum memory in bytes (default: 64MB) |
| `timeout_ms` | integer | No | Wall-clock timeout in milliseconds (default: 30000) |

**Example:**

```json
{
  "command": "echo 'hello world' | tr 'a-z' 'A-Z'",
  "timeout_ms": 10000
}
```

**Response:**

The tool returns the command output as text. If the command writes to stderr, it's included with a `--- stderr ---` separator. The exit code is shown for non-zero exits.

## Available Shell Commands

The sandbox includes common shell utilities:

- **I/O**: `echo`, `printf`, `cat`, `read`
- **Text Processing**: `grep`, `head`, `tail`, `wc`, `sort`, `uniq`
- **Control Flow**: `true`, `false`, `test`, `[`
- **Variables**: `export`, `set`, `unset`, `declare`
- **Shell Features**: `alias`, `eval`, `exec`, `type`, `command`

Pipes (`|`), redirects (`>`, `<`, `>>`), and command substitution (`$(...)`) are fully supported.

## Environment Variables

- `RUST_LOG`: Set log level (default: `info`). Logs go to stderr.

## Security

The shell runs in a WebAssembly sandbox with:

- No network access
- No filesystem access (beyond explicitly provided virtual files)
- CPU time limits (prevents infinite loops)
- Memory limits (prevents memory exhaustion)
- Output size limits (prevents output flooding)

## License

Apache-2.0