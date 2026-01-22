# Conch — Agent Notes

Context for AI agents working on this codebase.

## Important: Use mise for all commands

**Always prefer `mise run <task>` over manual `cargo` commands.** The mise tasks handle proper build ordering, feature flags, and dependencies. Manual cargo commands may miss required build steps or use incorrect flags.

```bash
# ✅ Good - use mise tasks
mise run build
mise run test
mise run lint
mise run check

# ❌ Avoid - manual cargo commands may miss dependencies
cargo build
cargo test
cargo clippy
```

See the "Commands (via mise)" section below for the full list of available tasks.

## What This Project Does

Conch is a **sandboxed shell execution engine**. It compiles a bash-compatible shell (brush) to WebAssembly and runs it via wasmtime with strict resource limits. The intended use case is letting AI agents query their execution context using shell commands.

## Architecture

```
┌─────────────────────────────────────────────────────────┐
│  Host (Go/Rust application)                             │
│                                                         │
│  ┌───────────────────────────────────────────────────┐  │
│  │  conch library (Rust)                             │  │
│  │  - CoreShellExecutor: loads & runs WASM module    │  │
│  │  - FFI layer: C ABI for Go integration            │  │
│  │  - VFS: virtual filesystem (planned)              │  │
│  │  - ResourceLimits: CPU, memory, output caps       │  │
│  └───────────────────────────────────────────────────┘  │
│                          │                              │
│                          ▼                              │
│  ┌───────────────────────────────────────────────────┐  │
│  │  wasmtime runtime                                 │  │
│  │  - WASI Preview 1 (wasip1)                        │  │
│  │  - Epoch-based CPU interruption                   │  │
│  │  - Memory limits                                  │  │
│  └───────────────────────────────────────────────────┘  │
│                          │                              │
│                          ▼                              │
│  ┌───────────────────────────────────────────────────┐  │
│  │  conch-shell (WASM module)                        │  │
│  │  - brush-core: full bash interpreter              │  │
│  │  - brush-builtins: cat, grep, head, tail, etc.    │  │
│  │  - Custom builtins (planned): ctx-search, jq      │  │
│  └───────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────┘
```

## Key Design Decisions

1. **brush over custom parser**: The plans mention building a custom tree-sitter parser, but the actual implementation uses [brush](https://github.com/reubeno/brush) — a mature bash-compatible shell. This gives full bash compatibility with less effort.

2. **wasip1 over wasip2**: Plans mention WASI Preview 2, but implementation uses `wasm32-wasip1` for broader compatibility with brush and wasmtime.

3. **purego over CGO**: Go bindings use [purego](https://github.com/ebitengine/purego) to call the Rust library without CGO, simplifying cross-compilation.

4. **Two executor types**:
   - `ShellExecutor`: Component model executor (planned, not fully implemented)
   - `CoreShellExecutor`: wasip1 executor with brush shell (working)

## Repository Layout

```
crates/
├── conch/                    # Main host library
│   ├── src/lib.rs           # Public API exports
│   ├── src/runtime.rs       # Conch struct, ExecutionContext, ExecutionResult
│   ├── src/wasm_core.rs     # CoreShellExecutor (wasip1 + brush)
│   ├── src/wasm.rs          # ShellExecutor (component model, planned)
│   ├── src/vfs.rs           # Virtual filesystem traits
│   ├── src/limits.rs        # ResourceLimits struct
│   ├── src/ffi.rs           # C FFI exports for Go
│   └── src/tests.rs         # Unit tests
│
├── conch-mcp/                # MCP server for AI assistant integration
│   ├── src/lib.rs           # ConchServer implementation with tool router
│   └── src/main.rs          # MCP server binary (stdio transport)
│
├── conch-shell/              # WASM shell module
│   ├── src/lib.rs           # FFI exports: execute(), get_stdout(), etc.
│   └── src/builtins/        # Custom builtins (placeholder)
│
└── conch-cli/                # CLI test tool
    └── src/main.rs          # Simple CLI wrapper

tests/go/
├── conch.go                  # Go bindings (purego)
├── conch_test.go             # Go tests
└── go.mod                    # Go module

plans/
├── conch.md                  # Original design doc (VFS, agent context)
├── conch-implementation.md   # Implementation plan (tree-sitter, wasip2)
└── testing.md                # Test strategy
```

## Commands (via mise)

**Always use these mise tasks instead of manual cargo commands.**

```bash
# Basic development
mise run check           # Check all crates compile
mise run build           # Build all crates
mise run test            # Run tests (uses cargo-nextest)
mise run lint            # Run clippy lints
mise run fmt             # Format code

# WASM and embedded builds
mise run wasm-build      # Build WASM shell module (wasm32-wasip1)
mise run build-embedded  # Build library with embedded shell
mise run build-release   # Full release build with embedded shell

# MCP server
mise run build-mcp       # Build the MCP server binary
mise run run-mcp         # Run the MCP server (stdio transport)

# Testing
mise run test-go         # Run Go FFI tests
mise run test-all        # Rust + Go tests

# CI and validation
mise run ci              # fmt-check, lint, test
mise run msrv            # Check MSRV (1.85)
```

Why mise over cargo?
- Tasks handle proper dependency ordering (e.g., WASM must be built before embedded builds)
- Tasks use the correct feature flags automatically
- Tasks ensure consistent behavior across development and CI

## Key Files to Understand

| File | Purpose |
|------|---------|
| `crates/conch/src/wasm_core.rs` | Main executor — loads WASM, runs scripts |
| `crates/conch/src/ffi.rs` | C FFI layer for Go integration |
| `crates/conch-mcp/src/lib.rs` | MCP server with `execute` tool |
| `crates/conch-shell/src/lib.rs` | WASM module entry point |
| `tests/go/conch.go` | Go bindings using purego |

## Build Artifacts

- `target/wasm32-wasip1/release/conch_shell.wasm` — The shell WASM module
- `target/release/libconch.so` (Linux) / `libconch.dylib` (macOS) — FFI library
- `target/release/conch-mcp` — MCP server binary

## Cargo Features

- `embedded-shell`: Embeds the pre-built WASM module in the library
- `embedded-component`: Embeds a component model WASM (not fully implemented)

## Testing Notes

1. **Rust tests**: Standard `cargo test` or `cargo nextest run`
2. **Go tests**: Require `mise run build-embedded` first to produce libconch.so
3. **WASM module**: Built separately with `mise run wasm-build`

## Current State vs Plans

The `plans/` directory describes an ambitious design with:
- Virtual filesystem exposing agent tool call history (`/ctx/self/tools/...`)
- Access control between agents
- Semantic search helpers (`ctx-search`)
- Custom builtins via tree-sitter parser

The current implementation is simpler:
- Working brush-based shell execution
- Basic resource limits
- Go FFI bindings
- No VFS integration yet (just standard WASI filesystem)

## Common Tasks

### Adding a builtin

1. Add to `crates/conch-shell/src/builtins/`
2. Register in `builtins::register_builtins()` 
3. Rebuild WASM: `mise run wasm-build`

### Testing Go integration

```bash
mise run build-embedded  # Must build library first
mise run test-go
```

### Debugging WASM execution

The CLI tool is useful for testing (after building with mise):
```bash
mise run build-embedded
./target/release/conch-cli -c "echo hello | cat"
```

### Running the MCP server

Build and run the MCP server for AI assistant integration:
```bash
mise run build-embedded  # Build with embedded WASM module
cargo run -p conch-mcp   # Run MCP server over stdio
```

Configure in Claude Desktop (`~/Library/Application Support/Claude/claude_desktop_config.json`):
```json
{
  "mcpServers": {
    "conch": {
      "command": "/path/to/target/release/conch-mcp"
    }
  }
}
```

## Dependencies

Key crates:
- `wasmtime` (v41): WASM runtime
- `brush-core`, `brush-builtins`: Bash shell implementation
- `tokio`: Async runtime (single-threaded in WASM)

Go:
- `github.com/ebitengine/purego`: FFI without CGO
