# Conch

A virtual filesystem shell sandbox that runs bash commands in a secure WASM environment.

Named after the conch shell from *Lord of the Flies* — "whoever holds the conch gets to speak."

## Overview

Conch provides a sandboxed shell execution environment using WebAssembly. It wraps [brush](https://github.com/reubeno/brush), a bash-compatible shell written in Rust, compiled to WASM and executed via [wasmtime](https://wasmtime.dev/).

The primary use case is allowing AI agents to query their own execution context (tool call history, messages, etc.) using familiar shell commands like `grep`, `jq`, `cat`, and pipes — all within strict resource limits.

## Features

- **Full bash compatibility** via brush-core
- **Secure sandboxing** — no host filesystem or network access
- **Resource limits** — CPU time, memory, output size
- **FFI bindings** — C ABI for Go integration (via purego, no CGO required)
- **Embeddable** — WASM module can be embedded in the library

## Quick Start

### Prerequisites

- Rust 1.85+ (1.92 recommended)
- [mise](https://mise.jdx.dev/) for task running (optional but recommended)

### Build

```bash
# Install dependencies and build
mise install
mise run build

# Build the WASM shell module
mise run wasm-build

# Build with embedded shell (for deployment)
mise run build-embedded
```

### Run Tests

```bash
# Rust tests
mise run test

# Go FFI tests (requires build-embedded first)
mise run test-go

# All tests
mise run test-all
```

### CLI Usage

```bash
# Build the CLI
cargo build -p conch-cli

# Execute a command
./target/debug/conch -c "echo hello | cat"

# Execute a script file
./target/debug/conch script.sh
```

## Project Structure

```
conch/
├── crates/
│   ├── conch/           # Host library (runtime, VFS, FFI)
│   ├── conch-shell/     # Brush-based shell (compiles to WASM)
│   └── conch-cli/       # CLI test harness
├── tests/go/            # Go FFI integration tests
├── plans/               # Design documents
└── mise.toml            # Task definitions
```

## Go Integration

The library exposes a C FFI that can be called from Go using purego (no CGO needed):

```go
import "github.com/yourorg/conch/tests/go"

executor, _ := conch.NewCoreExecutorEmbedded()
defer executor.Close()

result, _ := executor.Execute("echo hello | grep hello")
fmt.Println(string(result.Stdout))  // "hello\n"
```

## License

Apache-2.0
