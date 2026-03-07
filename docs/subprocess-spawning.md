# Subprocess Component Spawning — Implementation Notes

## Overview

Conch can execute external CLI tools compiled to WASI as subprocess components.
The shell (brush) intercepts unknown commands and calls back to the host via a
WIT `process` interface. The host instantiates the command as a separate WASI
component, bridges stdio, and returns the exit code.

**Working demo:**
```bash
cargo run -p conch-cli -- --commands-dir /tmp/conch-cmds -c "gh pr list -R cli/cli --limit 3 | upper"
```

This runs the Go `gh` CLI (wasip3) piped through a Rust `upper` command (wasip2),
both as WASI components inside conch's sandboxed shell.

## Architecture

### Component flow

```
Shell (brush, wasip2 guest)
  → encounters unknown command "gh"
  → calls WIT process::child::spawn("gh", args, env, cwd)
  → host looks up "gh" in ComponentRegistry
  → host spawns child on separate OS thread
  → host bridges stdin/stdout/stderr via batch approach
  → child runs to completion, exit code propagated
```

### Key files

- `crates/conch/src/executor/child.rs` — child process execution (p3+p2 engine)
- `crates/conch/src/executor/component.rs` — WIT HostChild impl, HybridComponentState
- `crates/conch/src/executor/registry.rs` — ComponentRegistry (wasm/cwasm bytes)
- `crates/conch-shell/src/lib.rs` — guest-side spawn handler (WitChildWrapper)
- `vendor/brush/brush-core/src/sys/stubs/process.rs` — thread_local spawn handler
- `vendor/brush/brush-core/src/commands.rs` — WASI dispatch in SimpleCommand::execute
- `crates/conch-cli/src/main.rs` — CLI with --commands-dir flag
- `crates/conch/wit/shell.wit` — process interface definition

### Two engines

The parent shell runs on a **p2 engine** (wasmtime with `epoch_interruption`).
Child commands run on a separate **p3 engine** (with `component_model_async`,
`component_model_async_builtins`, `component_model_async_stackful`).

The child engine is created lazily via `OnceLock` and shared across all child
invocations. Components from the registry are compiled for this engine on first use.

This means components in the registry store **raw bytes** (wasm or cwasm), not
pre-compiled `Component` objects, because `Component` is engine-specific.

## Critical gotchas

### 1. `allow_blocking_current_thread(true)` is REQUIRED for p3 filesystem

Without this, preopened directory operations fail with `unknown handle index 0`.
The p3 async filesystem implementation needs to complete certain operations
synchronously on the current thread. The wasmtime CLI sets this too.

```rust
builder.allow_blocking_current_thread(true);
```

### 2. Multiple preopened directories cause `bad-descriptor` errors with wasip3

When mounting both `/etc` and `/` (or `$HOME` and `/`), wasip3 Go components
crash with `bad-descriptor (error 2)` on filesystem reads. Single preopened
directory works fine.

**Workaround:** Mount a single directory containing everything the command needs.
For `gh`, we create `/tmp/gh-root/` with symlink-free copies of:
- `/etc/resolv.conf` (with real DNS server, not systemd stub)
- `/etc/hosts`
- `/etc/ssl/certs/ca-certificates.crt` (resolved from symlink)
- `~/.config/gh/config.yml` and `hosts.yml`

The wasmtime CLI handles multiple preopens fine, so this is likely a bug in how
we set up the store or an interaction with `run_concurrent`. Needs investigation.

### 3. WASI doesn't follow symlinks

`/etc/ssl/certs/ca-certificates.crt` is typically a symlink to
`/etc/ca-certificates/extracted/tls-ca-bundle.pem`. WASI's filesystem
implementation doesn't resolve symlinks, so Go's `crypto/x509.loadSystemRoots`
fails to read the cert file.

**Fix:** Use `std::fs::canonicalize()` to resolve symlinks before setting
`SSL_CERT_FILE`, or copy the resolved file into the sandbox root.

### 4. DNS stub resolver doesn't work in WASI

systemd-resolved's stub resolver at `127.0.0.53` (in `/etc/resolv.conf`) doesn't
work from WASI because the component can't connect to localhost services.

**Fix:** Create a `fake-etc/resolv.conf` pointing to the real upstream DNS server:
```
nameserver 192.168.1.254  # or whatever your real DNS is
```
Find it with: `resolvectl status | grep "DNS Servers"`

### 5. Go programs need larger stack and async stackful

```rust
config.max_wasm_stack(8 * 1024 * 1024);
config.async_stack_size(8 * 1024 * 1024);  // must be >= max_wasm_stack
config.wasm_component_model_async_stackful(true);
```

Without `async_stackful`, Go's goroutine-based concurrency model can't function.
The `async_stack_size` must be >= `max_wasm_stack` or engine creation fails.

### 6. Epoch interruption causes child components to trap

The parent engine uses epoch interruption for timeouts. If the child shares
the parent engine and the parent's timeout fires, the child gets interrupted.

**Fix:** Children use a separate engine without epoch interruption. Originally
we tried `store.set_epoch_deadline(u64::MAX)` on the same engine, but this can
overflow when added to the current epoch counter, and doesn't work reliably.

### 7. Brush WASM pipes use Arc — read_to_end hangs

On WASM, brush's pipe implementation uses `Arc<Mutex<Vec<u8>>>` for cloning.
When `ExecutionParameters` are cloned for pipeline stages, the pipe's write end
gets an extra `Arc` reference. Even after the producing stage finishes and drops
its params, the cloned reference keeps the pipe "open", so `read_to_end` on the
read end blocks forever waiting for EOF.

**Workaround:** Instead of `read_to_end`, use chunked reads that break after
reading less than a full buffer (indicating available data is exhausted):
```rust
loop {
    match stdin.read(&mut buf) {
        Ok(0) => break,
        Ok(n) => { stdin_data.extend_from_slice(&buf[..n]); }
        Err(_) => break,
    }
    if stdin_data.len() >= buf.len() { continue; }
    break;
}
```

### 8. wasip2 vs wasip3 component detection

The child executor tries p3 first, then falls back to p2. Both linkers are added,
then `Command::new` (not `instantiate_async`) is used to check which version
the component exports:

```rust
// Add both p2 and p3 WASI to the linker
wasmtime_wasi::p2::add_to_linker_async(&mut linker)?;
wasmtime_wasi::p3::add_to_linker(&mut linker)?;

// Instantiate once
let instance = linker.instantiate_async(&mut store, component).await?;

// Try p3 first
if let Ok(command) = wasmtime_wasi::p3::bindings::Command::new(&mut store, &instance) {
    // Use store.run_concurrent for p3
    store.run_concurrent(async |store| command.wasi_cli_run().call_run(store).await).await
} else {
    // Fall back to p2
    let command = wasmtime_wasi::p2::bindings::Command::new(&mut store, &instance)?;
    command.wasi_cli_run().call_run(&mut store).await
}
```

This matches the approach used by the wasmtime CLI (`src/commands/run.rs:660`).

### 9. ChildProcess::read_stdout must drain on first call

`read_stdout` returns the cached output from after `wait()` completes. If it
returns a clone each time, the guest's read loop never terminates (it checks for
0 bytes). Use `std::mem::take` to drain on first call:

```rust
pub fn read_stdout(&mut self) -> Vec<u8> {
    self.result.as_mut().map(|r| std::mem::take(&mut r.stdout)).unwrap_or_default()
}
```

## Performance: wasm vs cwasm

Pre-compiling to cwasm eliminates JIT compilation time:

| Configuration | `echo done` | `gh pr list \| upper` |
|---|---|---|
| wasm shell + wasm gh | 2.4s | 8.8s |
| wasm shell + cwasm gh | 2.4s | 3.2s |
| cwasm shell + cwasm gh | 0.2s | 1.3s |

### Compiling to cwasm

Shell (p2 engine config — epoch interruption only):
```bash
scratch/wasmtime/target/release/wasmtime compile \
    -W epoch-interruption \
    target/wasm32-wasip2/release/conch_shell.wasm \
    -o target/wasm32-wasip2/release/conch_shell.cwasm
```

Commands (p3 engine config — async + builtins + stackful):
```bash
scratch/wasmtime/target/release/wasmtime compile \
    -W component-model-async -W component-model-async-builtins \
    scratch/gh-component/component.wasm \
    -o scratch/gh-component/gh.cwasm
```

The cwasm must be compiled with the **exact same engine configuration** as the
runtime engine, or `Component::deserialize` will fail.

### Shell cwasm loading

The executor tries to load `conch_shell.cwasm` from the build directory at
runtime. If found, it uses `Component::deserialize` (instant). Otherwise falls
back to `Component::new` on the embedded wasm bytes (2+ seconds).

## Dependency situation

### wasmtime 44 (from source)

We use wasmtime 44.0.0 built from source at `scratch/wasmtime/` because:
- wasip3 support requires `component-model-async` feature
- The Go wasip3 fork targets a specific RC that wasmtime 41 can't parse
- wasmtime 42 (system) can parse it but doesn't fully implement the RC interfaces

The workspace `Cargo.toml` uses path dependencies to `scratch/wasmtime/crates/*`
and `[patch.crates-io]` to override transitive deps.

### eryx-vfs

Local path dependency to `../eryx/crates/eryx-vfs` with wasmtime bumped to 44.
Changes needed for wasmtime 44 compatibility:
- `anyhow::Result` → `wasmtime::Result` in host trait impls
- `Resource<anyhow::Error>` → `Resource<wasmtime::Error>`
- `anyhow::anyhow!()` → `wasmtime::Error::msg()` in StreamError
- `VfsStorage` now requires `Clone` bound — use `ArcStorage` wrapper

### brush fork

Local vendor copy at `vendor/brush/` with two changes:
- `brush-core/src/sys/stubs/process.rs` — thread_local spawn handler
- `brush-core/src/commands.rs` — `#[cfg(target_os = "wasi")] try_execute_via_spawn_handler()`

## Building the gh CLI

See `scratch/build-gh-wasm.sh` for the full pipeline. Requires:
- Go wasip3 fork at `scratch/go-wasip3/`
- `gh` source at `scratch/gh-cli/`
- `wasm-tools` at `scratch/wasm-tools/`
- Vendor patching via `scratch/patch-gh-vendor.sh`

The gh sandbox root at `/tmp/gh-root/` needs:
```
/tmp/gh-root/
├── etc/
│   ├── resolv.conf          # Real DNS (not systemd stub)
│   ├── hosts
│   └── ssl/certs/
│       └── ca-certificates.crt  # Resolved from symlink
└── home/ben/.config/gh/
    ├── config.yml
    └── hosts.yml             # Auth tokens
```

## Known issues / future work

- **Multiple preopens + wasip3**: crashes with `bad-descriptor`. Single preopen works.
- **Batch stdio**: stdin is fully read before child starts. Streaming would be better.
- **No per-command mount policy**: all commands get the same filesystem access.
- **Shell cwasm not auto-built**: need to manually run `wasmtime compile` after
  building the shell wasm. Could add to mise tasks.
- **`Config::async_support` deprecated**: wasmtime 44 has async always-on.
  The parent engine still calls it (harmless warning).
- **wasmtime 44 not on crates.io**: using path deps from `scratch/`. Need to
  track upstream releases and switch when 44+ is published.

## Setting up from scratch

If starting fresh (new machine, or the `vendor/` and `scratch/` dirs are gone):

### 1. Brush fork

```bash
git clone --branch more-wasi-compat --single-branch \
    https://github.com/sd2k/brush.git vendor/brush
git -C vendor/brush apply ../../patches/brush-subprocess.patch
```

### 2. Wasmtime 44 (from source)

```bash
git clone --depth 1 https://github.com/bytecodealliance/wasmtime.git scratch/wasmtime
cd scratch/wasmtime
cargo build --release --all-features
cd ../..
```

This takes a while (~5-10 min). The binary at `scratch/wasmtime/target/release/wasmtime`
is used for `wasmtime compile` (cwasm pre-compilation). The crate source at
`scratch/wasmtime/crates/wasmtime` etc. is used as a path dependency.

### 3. Eryx (wasmtime 44 branch)

The conch workspace expects eryx-vfs at `../eryx/crates/eryx-vfs`. Ensure the
eryx repo is checked out on the `feat/wasmtime-44` branch:

```bash
cd ../eryx
git checkout feat/wasmtime-44
cd ../conch
```

### 4. Build the shell + test command

```bash
cargo build -p conch-shell --target wasm32-wasip2 --release
cargo build -p conch-test-cmd --target wasm32-wasip2 --release
```

### 5. Pre-compile to cwasm (optional, for fast startup)

```bash
mise run compile-shell-cwasm
mise run compile-cwasm -- target/wasm32-wasip2/release/conch-test-cmd.wasm
```

### 6. Test it

```bash
mise run setup-commands-dir
cargo run -p conch-cli -- --commands-dir /tmp/conch-cmds -c "echo hello | upper"
```

### 7. gh CLI (optional)

Requires the Go wasip3 fork. See `patches/build-gh-wasm.sh` for the full
pipeline. Once built:

```bash
mise run setup-gh-sandbox
cp scratch/gh-component/gh.cwasm /tmp/conch-cmds/
cargo run -p conch-cli -- --commands-dir /tmp/conch-cmds \
    -c "gh pr list -R cli/cli --limit 3 | upper"
```
