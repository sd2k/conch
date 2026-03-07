# Subprocess Execution via WASI Components

This document explores implementing subprocess support in conch by running
external commands as WASI components, using the component model to call back
to the host for orchestration.

## Motivation

Conch's shell (brush) can currently only execute builtins. There is no
subprocess spawning in WASI — no `posix_spawn`, no `fork`/`exec`. This limits
conch to the commands compiled directly into the shell binary.

The goal: when a conch shell encounters `gh pr list`, the host recognises `gh`,
instantiates a pre-compiled WASI component for it, wires up stdio/env/filesystem,
runs it, and returns the exit code. Pipelines like `gh issue list | grep bug | wc -l`
should work, with the host orchestrating multiple concurrent component instances.

## Standards Landscape

### What exists today

- **No `wasi-process` proposal** at any phase. Tracking issues:
  [WASI#414](https://github.com/WebAssembly/WASI/issues/414) (posix_spawn, 2021),
  [WASI#763](https://github.com/WebAssembly/WASI/issues/763) (spawn component, 2023).
- **No guest-initiated instantiation.** A running component cannot create another
  component. All instantiation must be done by the host. This is by design — the
  security model requires the host to control what code runs.
- **Component Model runtime instantiation** is planned for CM 1.0, after WASI 0.3
  async lands. Luke Wagner confirmed this in Dec 2024
  ([component-model#423](https://github.com/WebAssembly/component-model/issues/423)).
  Estimated 2026–2027.

### WASI 0.3 async

WASI 0.3 adds native `stream<T>` and `future<T>` types to the component model,
making async I/O first-class. This is ideal for subprocess stdio bridging — the
host and guest can share async streams without manual byte-shuttling.

Wasmtime has experimental support (`-S p3`, `-W component-model-async`). The
conch project already uses wasmtime as its runtime.

### WASIX (alternative, not recommended)

Wasmer's WASIX extensions provide `proc_fork`/`proc_exec`/`proc_spawn` today,
but lock you to Wasmer, are based on Preview 1 (not the component model), and
are likely a dead end as WASI 0.3 matures.

## Architecture: Host-Mediated Subprocess Spawning

Since guests can't instantiate components, the shell guest calls the host via a
WIT import when it encounters an unknown command. The host handles instantiation,
stdio wiring, and lifecycle.

### Extension point in brush

Brush (the shell engine) already orchestrates pipelines in `interp.rs`. For each
pipeline stage it:

1. Creates in-memory pipes between commands (n-1 pipes for n commands)
2. Wires each stage's stdin/stdout to the right pipe ends
3. Applies redirections (`>`, `>>`, `<`, `2>&1`, here-docs, process substitution)
4. Dispatches to builtin/function/external

For external commands, brush calls `std::process::Command::spawn()` which fails
on WASI. The interception point is in `commands.rs` where brush falls through
from builtins → functions → PATH lookup → external spawn.

### Proposed WIT interface

```wit
interface process {
    resource child {
        spawn: static func(
            cmd: string,
            args: list<string>,
            env: list<tuple<string, string>>,
            cwd: string,
        ) -> result<child, string>;

        write-stdin: func(data: list<u8>) -> result<u64, string>;
        close-stdin: func();
        read-stdout: func(max-bytes: u32) -> result<list<u8>, string>;
        read-stderr: func(max-bytes: u32) -> result<list<u8>, string>;
        wait: func() -> result<s32, string>;
    }
}
```

With WASI 0.3 async streams, this simplifies to:

```wit
interface process {
    resource child {
        spawn: static func(
            cmd: string,
            args: list<string>,
            env: list<tuple<string, string>>,
            cwd: string,
        ) -> result<child, string>;

        stdin: func() -> stream<u8>;
        stdout: func() -> stream<u8>;
        stderr: func() -> stream<u8>;
        wait: async func() -> result<s32, string>;
    }
}
```

### Host-side implementation

On the host (the `conch` crate), `spawn` would:

1. Look up the command name in a registry of pre-compiled WASI components
2. Instantiate a new wasmtime `Component` (~5μs per instance)
3. Configure preopens, environment variables, working directory
4. Wire stdin/stdout/stderr streams between parent and child
5. Run the child's `wasi:cli/run` export on a separate tokio task
6. Return a handle for the guest to interact with

### Pipeline orchestration

Pipelines work because brush already sets up pipes and redirections before
dispatching each command. For a pipeline mixing builtins and external commands:

```
cat foo.txt | gh api /repos --jq '.name' | sort
```

- `cat` runs as a builtin in the guest, writing to pipe 1
- The guest reads pipe 1 and writes to the host via `write-stdin` on the `gh` child
- The host streams `gh`'s stdout back; the guest writes it to pipe 2
- `sort` runs as a builtin in the guest, reading from pipe 2

With WASI 0.3 async streams, the manual byte-shuttling is replaced by the
runtime managing data flow between stream handles.

## Fundamental Limitations

1. **No guest-initiated spawning.** Always requires host mediation. This is a
   security feature.
2. **No `fork()`.** WASI's capability model is incompatible with fork. Only
   spawn-style execution works. Fine for a shell.
3. **No shared memory between instances.** All communication goes through
   streams. Pipes work; shared memory IPC doesn't.
4. **No signals/job control.** No `SIGTERM`, `SIGINT`, `SIGPIPE`. Could be
   modelled in the WIT interface but tools won't expect it.
5. **Compiling tools to WASI is the real bottleneck.** Each tool's dependency
   tree needs wasip3 support. See below.

## Proof of Concept: Compiling `gh` CLI to WASI

We successfully compiled and ran the GitHub CLI (`gh` v2.87.3) as a WASI
component. This section documents exactly what was required.

### Prerequisites (all built from source)

| Tool | Version | Why from source |
|------|---------|-----------------|
| Go toolchain | [jellevandenhooff/go `wasip3-prototype`](https://github.com/jellevandenhooff/go/commits/wasip3-prototype/) | Experimental wasip3 Go port with async scheduler integration |
| wasm-tools | built from [main](https://github.com/bytecodealliance/wasm-tools) | Released 1.245.1 doesn't understand the `[async]wait-until` export from the Feb 2026 WASI RC |
| wasmtime | built from [main](https://github.com/bytecodealliance/wasmtime) (v43-dev) | v42.0.1 implements `0.3.0-rc-2026-01-06` but the Go fork targets `0.3.0-rc-2026-02-09` |

### Step 1: Build the wasip3 Go toolchain

```bash
git clone --branch wasip3-prototype https://github.com/jellevandenhooff/go.git
cd go/src && ./make.bash
```

This fork integrates Go's goroutine scheduler with the WASI 0.3 async model.
Passes `go tool dist test` including all `net` and `net/http` tests.

### Step 2: Vendor and patch dependencies

```bash
cd gh-cli
GOROOT=/path/to/go-wasip3 go mod vendor
```

Nine packages needed wasip3 stubs or build constraint fixes:

**Packages needing new `_wasip3.go` stub files:**

| Package | Stub behaviour |
|---------|---------------|
| `mattn/go-isatty` | `IsTerminal()` → false, `IsCygwinTerminal()` → false |
| `atotto/clipboard` | `readAll`/`writeAll` → error "not supported" |
| `muesli/termenv` | Hardcoded ANSI256 colour profile, no-op VT processing |
| `charmbracelet/bubbletea` | Stub `initInput`, `openInputTTY`, `suspendProcess`, `listenForResize` |
| `gdamore/tcell/v2` | `tScreen.initialize()` → error "not supported" |
| `google/certificate-transparency-go/x509` | Empty cert pool (same as wasip1 stub) |

**Packages needing build constraint patches _and_ new stubs:**

| Package | Problem | Fix |
|---------|---------|-----|
| `AlecAivazis/survey/v2/terminal` | `runereader_posix.go` has `//go:build !windows` which matches wasip3 | Add `&& !wasip3`, provide `runeReaderState` + `Buffer()` stub |
| `sirupsen/logrus` | `terminal_check_notappengine.go` has `!appengine,!js,!windows,!nacl,!plan9` | Add `,!wasip3`, provide `isTerminal()` + `checkIfTerminal()` |
| `in-toto/in-toto-golang` | `util_unix.go` has `linux \|\| darwin \|\| !windows` | Add `&& !wasip3`, provide `isWritable()` stub |

All stubs are trivial — they return "not a terminal" / "not supported" / no-op.
For a non-interactive agent context (conch's primary use case), this is correct
behaviour.

### Step 3: Compile

```bash
GOROOT=/path/to/go-wasip3 GOOS=wasip3 GOARCH=wasm32 \
    go build -mod=vendor -o gh.wasm ./cmd/gh
```

Produces a 71MB core wasm module.

### Step 4: Wrap as component

The raw `.wasm` is a core module using the async callback ABI. It must be
wrapped into a component using WIT definitions bundled with the Go fork:

```bash
wasm-tools component embed --all-features \
    $GOROOT/src/internal/wasi/wit \
    --world command \
    gh.wasm -o embedded.wasm

wasm-tools component new embedded.wasm -o component.wasm
```

The `--all-features` flag is required because the WIT files use
`@unstable(feature = clocks-timezone)` annotations.

### Step 5: Run

```bash
wasmtime run \
    -S p3 \
    -W component-model-async \
    -W component-model-async-builtins \
    -W max-wasm-stack=8388608 \
    -S inherit-env \
    -S inherit-network \
    -S tcp -S udp -S allow-ip-name-lookup \
    --dir /path/to/fake-etc::/etc \
    --dir / \
    --env SSL_CERT_FILE=/etc/ssl/certs/ca-certificates.crt \
    -- component.wasm api /user --jq '.login'
```

### Runtime workarounds

Two issues required workarounds at the host level:

**DNS resolution failure.** On systemd-based systems, `/etc/resolv.conf` is a
symlink to `/run/systemd/resolve/stub-resolv.conf`. WASI's `readlink` returns
"Operation not permitted", so Go's resolver can't follow it and falls back to
`127.0.0.1:53` (nothing listening) instead of `127.0.0.53:53`
(systemd-resolved).

_Fix:_ Mount a directory containing a real (non-symlink) copy of `resolv.conf`
as `/etc` in the guest via `--dir /path/to/fake-etc::/etc`.

**TLS certificate verification failure.** Go's `crypto/x509` includes both
`root_unix.go` (matches `wasip3`) and `root_wasm_common.go` (matches `wasm32`).
The latter defines empty `certFiles` and `certDirectories` slices, so no system
CA roots are loaded. The `root_linux.go` file that normally provides cert paths
doesn't match `GOOS=wasip3`.

_Fix:_ Set `SSL_CERT_FILE=/etc/ssl/certs/ca-certificates.crt` (or equivalent)
in the guest environment.

**Both fixes are trivial to implement on the host side** when spawning
subprocess components. The conch host would provide resolved-symlink preopens
and set `SSL_CERT_FILE` automatically.

## Implications for conch

### What works today

- Building the subprocess spawning WIT interface and host implementation
- Compiling Rust CLI tools to wasip2 components (`cargo build --target
  wasm32-wasip2` — stable target, no adapter or wrapping needed)
- Compiling Go CLI tools to wasip3 components (via the experimental Go fork,
  with per-dependency vendor patching)
- Running components with full networking (DNS, TCP, TLS) given the two
  host-side workarounds above

The host can instantiate both wasip2 and wasip3 components — wasmtime supports
both. The difference is in stdio handling: wasip2 components use synchronous
blocking `wasi:io/streams` (host runs them on a blocking task), while wasip3
components use native async `stream<u8>` (more efficient for high-throughput
piping). Both produce the same result from the shell's perspective.

### What to build next

1. **WIT `process` interface** — add to conch's existing WIT world
2. **Brush fork integration** — intercept external command dispatch on WASI
   (`#[cfg(target_os = "wasi")]`) to call the host instead of
   `std::process::Command`
3. **Host-side component registry** — map command names to pre-compiled
   `.wasm` components, handle instantiation and lifecycle
4. **Pipeline stdio bridging** — proxy bytes between brush's in-memory pipes
   and host-spawned component streams (or use WASI 0.3 async streams directly)
5. **Automate vendor patching** — script the wasip3 stub generation for Go
   dependencies to make it reproducible

### Go ecosystem maturity

The wasip3 Go fork is experimental. Key milestones to watch:

- [golang/go#77141](https://github.com/golang/go/issues/77141) — wasip3 proposal.
  The Go team is cautious ("we should only implement wasip3 as and when it becomes
  clear the larger Wasm community is settling on it as a standard").
- [golang/go#65333](https://github.com/golang/go/issues/65333) — wasip2 proposal
  (still open, may be skipped in favour of wasip3).
- The vendor patching burden is per-tool and grows with dependency count. Simpler
  Go tools (fewer deps, no TUI) compile with fewer patches. `gh` was a
  worst-case stress test.
- Rust tools compile to WASI components with no patching needed.

### wasip1 adapter as alternative

For Go tools that don't need concurrency, `GOOS=wasip1 GOARCH=wasm` + the
wasip1→wasip2 adapter (`wasm-tools component new --adapt`) works today. But
the adapter **blocks all goroutines** on any `poll_oneoff` call, making it
impractical for tools with concurrent I/O like `gh`.
