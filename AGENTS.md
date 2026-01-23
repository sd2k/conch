# Conch — Agent Notes

Context for AI agents working on this codebase.

## Important: Read the Notes

Before making significant changes, check the `notes/` directory:

- `vfs-architecture.md` — How hybrid VFS works via WASI shadowing (eryx pattern)
- `wasip1-vs-wasip2.md` — Why we use wasip2 component model

These documents explain *why* things are the way they are.

## Important: Use mise for all commands

**Always prefer `mise run <task>` over manual `cargo` commands.** The mise tasks handle proper build ordering, feature flags, and dependencies.

```bash
# Good - use mise tasks
mise run build
mise run test
mise run lint

# Avoid - manual cargo commands may miss dependencies
cargo build
```

See "Commands (via mise)" below for the full list.

## What This Project Does

Conch is a **sandboxed shell execution engine**. It compiles a bash-compatible shell (brush) to WebAssembly and runs it via wasmtime with:

- **Hybrid VFS**: Combine in-memory storage with real filesystem mounts
- **Strict resource limits**: CPU time, memory, output size
- **Capability-based security**: Only explicitly mounted paths are accessible

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│  Host Application (Rust/Go)                                 │
│                                                             │
│  ┌────────────────────────────────────────────────────────┐ │
│  │  Shell API (high-level)                                │ │
│  │  - ShellBuilder: configure VFS paths and real mounts   │ │
│  │  - Shell::execute(): run commands with hybrid VFS      │ │
│  │  - VfsStorage: in-memory or custom storage backend     │ │
│  └────────────────────────────────────────────────────────┘ │
│                           │                                 │
│  ┌────────────────────────────────────────────────────────┐ │
│  │  Conch API (simple)                                    │ │
│  │  - Conch::execute(): run commands without VFS          │ │
│  │  - Concurrency limiting via semaphore                  │ │
│  └────────────────────────────────────────────────────────┘ │
│                           │                                 │
│  ┌────────────────────────────────────────────────────────┐ │
│  │  ComponentShellExecutor (low-level)                    │ │
│  │  - execute(): basic WASI execution                     │ │
│  │  - execute_with_hybrid_vfs(): VFS + real mounts        │ │
│  │  - InstancePre for fast per-call instantiation         │ │
│  └────────────────────────────────────────────────────────┘ │
│                           │                                 │
│  ┌────────────────────────────────────────────────────────┐ │
│  │  wasmtime runtime                                      │ │
│  │  - WASI Preview 2 (wasip2) component model             │ │
│  │  - eryx-vfs: hybrid VFS via WASI shadowing             │ │
│  │  - Epoch-based CPU interruption                        │ │
│  │  - Memory limits via ResourceLimiter                   │ │
│  └────────────────────────────────────────────────────────┘ │
│                           │                                 │
│  ┌────────────────────────────────────────────────────────┐ │
│  │  conch-shell (WASM component)                          │ │
│  │  - brush-core: full bash interpreter                   │ │
│  │  - Custom builtins: cat, grep, head, tail, jq, wc      │ │
│  └────────────────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────────────────┘
```

## Key Design Decisions

1. **Hybrid VFS via eryx-vfs**: The shell sees a unified filesystem that combines:
   - VFS paths (e.g., `/scratch`) backed by `VfsStorage` trait
   - Real paths (e.g., `/project`) backed by cap-std `Dir` handles
   - WASI filesystem interfaces are shadowed to route to the appropriate backend

2. **wasip2 component model**: We use WASI Preview 2 for better composability and the component model's cleaner interface boundaries.

3. **Three API levels**:
   - `Shell`: High-level builder pattern, recommended for most uses
   - `Conch`: Simple API with concurrency limiting, no VFS
   - `ComponentShellExecutor`: Low-level control over WASM execution

4. **purego over CGO**: Go bindings use purego for CGO-free cross-compilation.

5. **InstancePre for performance**: The executor pre-links the WASM component once, then creates a fresh Store per execution.

## Repository Layout

```
crates/
├── conch/                    # Main host library
│   ├── src/lib.rs           # Public API exports
│   ├── src/shell.rs         # Shell and ShellBuilder (high-level API)
│   ├── src/runtime.rs       # Conch struct, ExecutionResult
│   ├── src/executor/        # WASM executors
│   │   ├── mod.rs           # Module exports
│   │   └── component.rs     # ComponentShellExecutor (wasip2)
│   ├── src/limits.rs        # ResourceLimits struct
│   └── src/ffi.rs           # C FFI exports for Go
│
├── conch-mcp/                # MCP server for AI assistant integration
│   ├── src/lib.rs           # ConchServer with execute tool
│   └── src/main.rs          # MCP server binary (stdio transport)
│
├── conch-shell/              # WASM shell component
│   ├── src/lib.rs           # WIT component entry point
│   └── src/builtins/        # Custom builtins (cat, grep, jq, etc.)
│
└── conch-cli/                # CLI test tool
    └── src/main.rs          # Simple CLI wrapper

examples/                     # Usage examples
├── basic.rs                 # Simple Shell API usage
├── vfs_mounts.rs            # VFS and real filesystem mounts
├── low_level.rs             # ComponentShellExecutor usage
└── custom_storage.rs        # Custom VfsStorage implementation

tests/go/
├── conch.go                  # Go bindings (purego)
└── conch_test.go             # Go tests
```

## Commands (via mise)

```bash
# Development
mise run check           # Check all crates compile
mise run build           # Build all crates
mise run test            # Run tests (cargo-nextest)
mise run lint            # Run clippy lints
mise run fmt             # Format code

# WASM and embedded builds
mise run wasm-build      # Build WASM component (wasm32-wasip2)
mise run build-embedded  # Build library with embedded shell
mise run build-release   # Full release build

# MCP server
mise run build-mcp       # Build the MCP server binary
mise run run-mcp         # Run the MCP server

# Testing
mise run test-go         # Run Go FFI tests
mise run test-all        # Rust + Go tests

# CI
mise run ci              # fmt-check, lint, test
```

## Key Files to Understand

| File | Purpose |
|------|---------|
| `crates/conch/src/shell.rs` | Shell and ShellBuilder — high-level API |
| `crates/conch/src/runtime.rs` | Conch struct — simple execution with concurrency |
| `crates/conch/src/executor/component.rs` | ComponentShellExecutor — low-level WASM control |
| `crates/conch/src/ffi.rs` | C FFI layer for Go integration |
| `crates/conch-shell/src/lib.rs` | WASM component entry point |
| `crates/conch-shell/src/builtins/*.rs` | Custom shell builtins |

## Build Artifacts

- `target/wasm32-wasip2/release/conch_shell.wasm` — Shell WASM component
- `target/release/libconch.so` (Linux) / `.dylib` (macOS) — FFI library
- `target/release/conch-mcp` — MCP server binary

## Cargo Features

- `embedded-shell`: Embeds the pre-built WASM component in the library

## Adding a Builtin

1. Create `crates/conch-shell/src/builtins/mybuiltin.rs`
2. Implement `brush_core::builtins::SimpleCommand`
3. Register in `builtins::register_builtins()` in `mod.rs`
4. **Important**: Add `context.stdout().flush()?` before returning
5. Rebuild WASM: `mise run wasm-build`

## VFS Integration

The hybrid VFS uses WASI shadowing via eryx-vfs:

1. Shell compiles to `wasm32-wasip2` component
2. Host adds WASI bindings, then shadows with eryx-vfs bindings
3. `HybridVfsCtx` routes paths to either:
   - `VfsStorage` (in-memory or custom) for VFS paths
   - `cap_std::Dir` for real filesystem mounts
4. Shell uses standard filesystem operations transparently

```rust
// Example: Configure hybrid VFS
let shell = Shell::builder()
    .vfs_path("/scratch", DirPerms::all(), FilePerms::all())  // VFS
    .mount("/project", "./code", Mount::readonly())           // Real FS
    .build()?;
```

## Common Tasks

### Testing changes to builtins

```bash
mise run wasm-build      # Rebuild WASM component
mise run test            # Run tests
```

### Testing Go integration

```bash
mise run build-embedded  # Build library with embedded WASM
mise run test-go         # Run Go tests
```

### Debugging execution

```bash
mise run build-embedded
./target/release/conch-cli -c "echo hello | cat"
```

### Running MCP server

```bash
mise run build-mcp
./target/release/conch-mcp
```

## Dependencies

Key crates:
- `wasmtime` (v41): WASM runtime with component model support
- `wasmtime-wasi`: WASI Preview 2 support
- `eryx-vfs`: Hybrid VFS via WASI shadowing
- `brush-core`, `brush-builtins`: Bash shell implementation
- `tokio`: Async runtime

Go:
- `github.com/ebitengine/purego`: FFI without CGO
