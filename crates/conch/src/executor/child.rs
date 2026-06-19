//! Child process management for subprocess component spawning.
//!
//! When the shell guest calls `child::spawn(cmd, ...)`, the host looks up the
//! command in its [`ComponentRegistry`], and prepares a child process handle.
//! The actual component execution happens on a separate OS thread when the
//! guest calls `wait()`, using a batch approach: all stdin is collected first,
//! then the component runs to completion, then stdout/stderr are available.
//!
//! Child components are executed using **wasip3** (WASI Preview 3), which
//! provides native async streams. This allows running Go programs compiled
//! with the experimental wasip3 Go fork as well as standard wasip2 Rust CLIs.

use std::path::PathBuf;
use std::sync::Arc;

use eryx_vfs::{
    DirPerms, FilePerms, HybridVfsCtx, HybridVfsState, HybridVfsView, RealDir, VfsStorage,
    add_hybrid_vfs_to_linker,
};
use tokio::sync::oneshot;
use wasmtime::component::{Component, Linker, ResourceTable};
use wasmtime::{Config, Engine, Store};
use wasmtime_wasi::p2::pipe::{MemoryInputPipe, MemoryOutputPipe};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

/// Filesystem template handed to a spawned child so it shares the shell's
/// virtual filesystem (eryx-vfs storage + the same mounts), rather than a
/// separate real-fs sandbox. The child rebuilds a [`HybridVfsCtx`] from this on
/// its own thread; the storage is `Clone` (Arc-backed) so it shares data with
/// the shell. Real mounts are re-opened per child from their host paths.
pub struct ChildVfs<S> {
    /// Shared VFS storage (same backing data as the shell).
    pub storage: S,
    /// Virtual preopens: `(guest_path, dir_perms, file_perms)`.
    pub vfs_mounts: Vec<(String, DirPerms, FilePerms)>,
    /// Real-fs preopens: `(guest_path, host_path, dir_perms, file_perms)`.
    pub real_mounts: Vec<(String, PathBuf, DirPerms, FilePerms)>,
}

impl<S: Clone> Clone for ChildVfs<S> {
    fn clone(&self) -> Self {
        Self {
            storage: self.storage.clone(),
            vfs_mounts: self.vfs_mounts.clone(),
            real_mounts: self.real_mounts.clone(),
        }
    }
}

impl<S> std::fmt::Debug for ChildVfs<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChildVfs")
            .field("vfs_mounts", &self.vfs_mounts.len())
            .field("real_mounts", &self.real_mounts.len())
            .finish_non_exhaustive()
    }
}

impl<S: VfsStorage + Clone + 'static> ChildVfs<S> {
    /// Build a fresh [`HybridVfsCtx`] sharing this storage, with the same mounts.
    fn build_ctx(&self) -> HybridVfsCtx<S> {
        let mut ctx = HybridVfsCtx::new(self.storage.clone());
        for (path, dperms, fperms) in &self.vfs_mounts {
            ctx.add_vfs_preopen(path, *dperms, *fperms);
        }
        for (guest, host, dperms, fperms) in &self.real_mounts {
            match RealDir::open_ambient(host, *dperms, *fperms) {
                Ok(dir) => ctx.add_real_preopen(guest, dir),
                Err(e) => eprintln!(
                    "[conch] child: failed to open real mount {} -> {}: {e}",
                    host.display(),
                    guest
                ),
            }
        }
        ctx
    }
}

/// Result of running a child component.
struct ChildResult {
    exit_code: i32,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

/// State for a spawned child process.
///
/// Uses a batch approach: stdin data is accumulated, then the component
/// runs to completion on `wait()`, producing stdout/stderr all at once.
pub(crate) struct ChildProcess<S: VfsStorage + Clone + 'static> {
    /// Engine for the child — separate from parent, with p3 async support.
    engine: Arc<Engine>,
    /// The component to run.
    component: Component,
    /// Command-line arguments.
    args: Vec<String>,
    /// Environment variables.
    env: Vec<(String, String)>,
    /// Working directory.
    cwd: String,
    /// Filesystem the child sees: the shell's VFS (shared storage + mounts)
    /// plus any real mounts (e.g. the `--sandbox-root` for certs).
    vfs: ChildVfs<S>,
    /// Accumulated stdin data.
    pub stdin_buffer: Vec<u8>,
    /// Whether stdin has been closed (no more writes allowed).
    pub stdin_closed: bool,
    /// Result receiver from the child thread (set when execution starts).
    result_rx: Option<oneshot::Receiver<Result<ChildResult, String>>>,
    /// Cached result after wait() completes.
    result: Option<ChildResult>,
    /// Handle to the child's OS thread.
    _thread_handle: Option<std::thread::JoinHandle<()>>,
}

/// WASI state for the child component's Store.
///
/// Holds both a [`WasiCtx`] (stdio/env/network, plus a real-fs preopen used by
/// p3 components) and a [`HybridVfsCtx`] that shadows the p2 `wasi:filesystem`
/// so p2 components see the shell's virtual filesystem.
struct ChildWasiState<S: VfsStorage + Clone + 'static> {
    wasi: WasiCtx,
    table: ResourceTable,
    hybrid: HybridVfsCtx<S>,
}

impl<S: VfsStorage + Clone + 'static> WasiView for ChildWasiState<S> {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

impl<S: VfsStorage + Clone + 'static> HybridVfsView for ChildWasiState<S> {
    type Storage = S;
    fn hybrid_vfs(&mut self) -> HybridVfsState<'_, Self::Storage> {
        HybridVfsState::new(&mut self.hybrid, &mut self.table)
    }
}

/// Create a child engine with wasip3 (component-model-async) support.
///
/// This is separate from the parent engine because:
/// - It enables `wasm_component_model_async` for wasip3 components
/// - No epoch interruption (child manages its own lifecycle)
fn create_child_engine() -> Result<Engine, String> {
    let mut config = Config::new();
    config.wasm_component_model_async(true);
    config.wasm_component_model_async_builtins(true);
    config.wasm_component_model_async_stackful(true);
    // Go programs need a larger stack
    config.max_wasm_stack(8 * 1024 * 1024);
    config.async_stack_size(8 * 1024 * 1024);
    // Persist cranelift output so child components aren't recompiled every
    // nextest process. Transparent; only affects speed.
    super::enable_compilation_cache(&mut config);
    Engine::new(&config).map_err(|e| format!("failed to create child engine: {e}"))
}

/// Shared child engine, created lazily.
///
/// All child components share one engine to avoid repeated compilation costs.
/// Components compiled for one engine can't be used with another, so we
/// re-compile components against this engine on first use.
fn child_engine() -> Result<&'static Engine, String> {
    use std::sync::OnceLock;
    static ENGINE: OnceLock<Engine> = OnceLock::new();
    // OnceLock::get_or_init doesn't support fallible initialization,
    // but engine creation only fails with invalid config which won't happen.
    if let Some(engine) = ENGINE.get() {
        return Ok(engine);
    }
    let engine = create_child_engine()?;
    // Race is fine — both engines would be equivalent.
    Ok(ENGINE.get_or_init(|| engine))
}

pub(crate) enum ComponentBytes<'a> {
    Wasm(&'a [u8]),
    Cwasm(&'a [u8]),
}

/// Create a child process handle (does not start execution yet).
///
/// The component is compiled from raw WASM bytes for the child's
/// p3-capable engine.
pub(crate) fn spawn_child<S: VfsStorage + Clone + 'static>(
    bytes: ComponentBytes<'_>,
    cmd: &str,
    args: &[String],
    env: &[(String, String)],
    cwd: &str,
    vfs: ChildVfs<S>,
) -> Result<ChildProcess<S>, String> {
    let engine = child_engine()?;

    let child_component = match bytes {
        ComponentBytes::Wasm(wasm_bytes) => Component::new(engine, wasm_bytes)
            .map_err(|e| format!("failed to compile component for p3 engine: {e}"))?,
        ComponentBytes::Cwasm(cwasm_bytes) => {
            unsafe { Component::deserialize(engine, cwasm_bytes) }
                .map_err(|e| format!("failed to deserialize component for p3 engine: {e}"))?
        }
    };

    // Prepend command name as argv[0]
    let mut full_args = vec![cmd.to_string()];
    full_args.extend_from_slice(args);

    Ok(ChildProcess {
        engine: Arc::new(engine.clone()),
        component: child_component,
        args: full_args,
        env: env.to_vec(),
        cwd: cwd.to_string(),
        vfs,
        stdin_buffer: Vec::new(),
        stdin_closed: false,
        result_rx: None,
        result: None,
        _thread_handle: None,
    })
}

impl<S: VfsStorage + Clone + Send + Sync + 'static> ChildProcess<S> {
    /// Write data to the stdin buffer.
    pub fn write_stdin(&mut self, data: Vec<u8>) -> Result<u64, String> {
        if self.stdin_closed {
            return Err("stdin is closed".to_string());
        }
        let len = data.len() as u64;
        self.stdin_buffer.extend(data);
        Ok(len)
    }

    /// Close stdin and start execution on a background thread.
    pub fn close_stdin(&mut self) {
        if self.stdin_closed {
            return;
        }
        self.stdin_closed = true;
        self.start_execution();
    }

    /// Wait for the child to complete. Closes stdin if not already closed.
    pub fn wait(&mut self) -> Result<i32, String> {
        // If we already have a result, return it
        if let Some(ref result) = self.result {
            return Ok(result.exit_code);
        }

        // Close stdin if not already closed (triggers execution)
        if !self.stdin_closed {
            self.close_stdin();
        }

        // Take the receiver and block on it
        let rx = self
            .result_rx
            .take()
            .ok_or_else(|| "no result receiver".to_string())?;

        let child_result = futures::executor::block_on(rx)
            .map_err(|e| format!("child thread panicked: {e}"))?
            .map_err(|e| format!("child execution failed: {e}"))?;

        let exit_code = child_result.exit_code;
        self.result = Some(child_result);
        Ok(exit_code)
    }

    /// Read stdout (available after wait() completes).
    /// Returns data once, then empty on subsequent calls.
    pub fn read_stdout(&mut self) -> Vec<u8> {
        self.result
            .as_mut()
            .map(|r| std::mem::take(&mut r.stdout))
            .unwrap_or_default()
    }

    /// Read stderr (available after wait() completes).
    /// Returns data once, then empty on subsequent calls.
    pub fn read_stderr(&mut self) -> Vec<u8> {
        self.result
            .as_mut()
            .map(|r| std::mem::take(&mut r.stderr))
            .unwrap_or_default()
    }

    /// Start executing the component on a background thread.
    fn start_execution(&mut self) {
        let (tx, rx) = oneshot::channel();
        self.result_rx = Some(rx);

        let engine = self.engine.clone();
        let component = self.component.clone();
        let stdin_data = std::mem::take(&mut self.stdin_buffer);
        let args = self.args.clone();
        let env = self.env.clone();
        let cwd = self.cwd.clone();
        let vfs = self.vfs.clone();

        let handle = std::thread::spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    let _ = tx.send(Err(format!("failed to create runtime: {e}")));
                    return;
                }
            };

            let result = rt.block_on(run_child_component(
                &engine,
                &component,
                &stdin_data,
                &args,
                &env,
                &cwd,
                &vfs,
            ));
            if let Err(ref e) = result {
                eprintln!("[child] error: {e}");
            }
            let _ = tx.send(result);
        });

        self._thread_handle = Some(handle);
    }
}

/// Run the child component, sharing the shell's filesystem.
///
/// The child's p2 `wasi:filesystem` is backed by the shell's eryx-vfs (shared
/// storage + the same mounts), so it sees the virtual filesystem (`/agent`,
/// `/scratch`, …). The real mounts are also preopened into the [`WasiCtx`] so
/// p3 components (which use a different `wasi:filesystem` snapshot, not yet
/// VFS-backed) still get real-fs access (e.g. `gh` and its certs).
async fn run_child_component<S: VfsStorage + Clone + 'static>(
    engine: &Engine,
    component: &Component,
    stdin_data: &[u8],
    args: &[String],
    _env: &[(String, String)],
    _cwd: &str,
    vfs: &ChildVfs<S>,
) -> Result<ChildResult, String> {
    let stdout_pipe = MemoryOutputPipe::new(1024 * 1024); // 1MB
    let stderr_pipe = MemoryOutputPipe::new(1024 * 1024);

    let make_state = |stdin_data: &[u8]| -> ChildWasiState<S> {
        let stdin_pipe = MemoryInputPipe::new(stdin_data.to_vec());
        let mut builder = WasiCtxBuilder::new();
        builder
            .allow_blocking_current_thread(true)
            .stdin(stdin_pipe)
            .stdout(stdout_pipe.clone())
            .stderr(stderr_pipe.clone());
        for arg in args {
            builder.arg(arg);
        }
        builder.inherit_env();
        // Network access for commands that need it (e.g. gh, curl)
        builder.inherit_network();
        builder.allow_ip_name_lookup(true);
        builder.env("SSL_CERT_FILE", "/etc/ssl/certs/ca-certificates.crt");

        // Real-fs preopens for the p3 path (p2 uses the hybrid VFS below, which
        // shadows wasi:filesystem). Existing dirs only.
        let mut real_preopened = 0usize;
        for (guest, host, _d, _f) in &vfs.real_mounts {
            if host.exists() {
                let _ = builder.preopened_dir(
                    host,
                    guest,
                    wasmtime_wasi::DirPerms::all(),
                    wasmtime_wasi::FilePerms::all(),
                );
                real_preopened += 1;
            }
        }
        // Fallback for p3 components (e.g. gh) when no real mount is configured:
        // mount the host's real `/`, matching the pre-VFS behaviour so they keep
        // filesystem access (config, certs). p2 components ignore this — they use
        // the shadowed hybrid VFS above.
        if real_preopened == 0 {
            let _ = builder.preopened_dir(
                "/",
                "/",
                wasmtime_wasi::DirPerms::all(),
                wasmtime_wasi::FilePerms::all(),
            );
        }

        ChildWasiState {
            wasi: builder.build(),
            table: ResourceTable::new(),
            hybrid: vfs.build_ctx(),
        }
    };

    let exit_code = run_with_linker(engine, make_state(stdin_data), component).await?;

    let stdout = stdout_pipe.contents().to_vec();
    let stderr = stderr_pipe.contents().to_vec();

    Ok(ChildResult {
        exit_code,
        stdout,
        stderr,
    })
}

/// Instantiate and run a component, trying p3 then p2.
///
/// Mirrors the approach used by the wasmtime CLI: add both p2 and p3
/// to the linker, instantiate once, then try Command::new for p3 first.
async fn run_with_linker<S: VfsStorage + Clone + 'static>(
    engine: &Engine,
    state: ChildWasiState<S>,
    component: &Component,
) -> Result<i32, String> {
    let mut store = Store::new(engine, state);
    let mut linker = Linker::<ChildWasiState<S>>::new(engine);

    // Add both p2 and p3 WASI interfaces (same as wasmtime CLI with -S p3)
    wasmtime_wasi::p2::add_to_linker_async(&mut linker)
        .map_err(|e| format!("failed to add WASI p2 to linker: {e}"))?;
    wasmtime_wasi::p3::add_to_linker(&mut linker)
        .map_err(|e| format!("failed to add WASI p3 to linker: {e}"))?;
    // Shadow the p2 `wasi:filesystem` with the shell's VFS so p2 components see
    // the virtual filesystem (same pattern the shell uses for itself).
    linker.allow_shadowing(true);
    add_hybrid_vfs_to_linker(&mut linker)
        .map_err(|e| format!("failed to add hybrid VFS to child linker: {e}"))?;
    linker.allow_shadowing(false);

    // Instantiate the component once
    let instance = linker
        .instantiate_async(&mut store, component)
        .await
        .map_err(|e| format!("failed to instantiate component: {e}"))?;

    // Try p3 Command first (like wasmtime CLI does)
    if let Ok(command) = wasmtime_wasi::p3::bindings::Command::new(&mut store, &instance) {
        let run_result = store
            .run_concurrent(async |store| command.wasi_cli_run().call_run(store).await)
            .await
            .map_err(|e| format!("event loop failed: {e:?}"))?;
        return match run_result {
            Ok(Ok(())) => Ok(0),
            Ok(Err(())) => Ok(1),
            Err(e) => exit_code_or_failure(e),
        };
    }

    // Fall back to p2 Command
    let command = wasmtime_wasi::p2::bindings::Command::new(&mut store, &instance)
        .map_err(|e| format!("failed to create command (tried p3 and p2): {e}"))?;

    match command.wasi_cli_run().call_run(&mut store).await {
        Ok(Ok(())) => Ok(0),
        Ok(Err(())) => Ok(1),
        Err(e) => exit_code_or_failure(e),
    }
}

/// Map an error from `call_run` to an exit code. A guest that calls
/// `proc_exit`/`wasi:cli/exit` (e.g. uutils' explicit `std::process::exit`)
/// surfaces as a `wasmtime_wasi::I32Exit` rather than a clean return — that's a
/// normal exit, so use its status (0 = success). Anything else is a real failure.
fn exit_code_or_failure(e: wasmtime::Error) -> Result<i32, String> {
    match e.downcast_ref::<wasmtime_wasi::I32Exit>() {
        Some(exit) => Ok(exit.0),
        None => Err(format!("component execution failed: {e:?}")),
    }
}
