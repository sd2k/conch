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

use std::sync::Arc;

use tokio::sync::oneshot;
use wasmtime::component::{Component, Linker, ResourceTable};
use wasmtime::{Config, Engine, Store};
use wasmtime_wasi::p2::pipe::{MemoryInputPipe, MemoryOutputPipe};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

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
pub(crate) struct ChildProcess {
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
struct ChildWasiState {
    wasi: WasiCtx,
    table: ResourceTable,
}

impl WasiView for ChildWasiState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
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
pub(crate) fn spawn_child(
    bytes: ComponentBytes<'_>,
    cmd: &str,
    args: &[String],
    env: &[(String, String)],
    cwd: &str,
) -> Result<ChildProcess, String> {
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
        stdin_buffer: Vec::new(),
        stdin_closed: false,
        result_rx: None,
        result: None,
        _thread_handle: None,
    })
}

impl ChildProcess {
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
            ));
            if let Err(ref e) = result {
                eprintln!("[child] error: {e}");
            }
            let _ = tx.send(result);
        });

        self._thread_handle = Some(handle);
    }
}

/// Run the child component inside a WASI p3 environment.
async fn run_child_component(
    engine: &Engine,
    component: &Component,
    stdin_data: &[u8],
    args: &[String],
    env: &[(String, String)],
    _cwd: &str,
) -> Result<ChildResult, String> {
    let stdout_pipe = MemoryOutputPipe::new(1024 * 1024); // 1MB
    let stderr_pipe = MemoryOutputPipe::new(1024 * 1024);

    let make_state = |stdin_data: &[u8]| -> ChildWasiState {
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

        // Mount a sandbox root with symlink-free copies of system files
        // (resolv.conf, TLS certs, gh config). Falls back to real /.
        // Multiple preopens cause handle issues with wasip3, so use one.
        let sandbox_root = std::path::Path::new("/tmp/gh-root");
        let root = if sandbox_root.exists() {
            sandbox_root
        } else {
            std::path::Path::new("/")
        };
        let _ = builder.preopened_dir(
            root,
            "/",
            wasmtime_wasi::DirPerms::all(),
            wasmtime_wasi::FilePerms::all(),
        );
        ChildWasiState {
            wasi: builder.build(),
            table: ResourceTable::new(),
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
async fn run_with_linker(
    engine: &Engine,
    state: ChildWasiState,
    component: &Component,
) -> Result<i32, String> {
    let mut store = Store::new(engine, state);
    let mut linker = Linker::<ChildWasiState>::new(engine);

    // Add both p2 and p3 WASI interfaces (same as wasmtime CLI with -S p3)
    wasmtime_wasi::p2::add_to_linker_async(&mut linker)
        .map_err(|e| format!("failed to add WASI p2 to linker: {e}"))?;
    wasmtime_wasi::p3::add_to_linker(&mut linker)
        .map_err(|e| format!("failed to add WASI p3 to linker: {e}"))?;

    // Instantiate the component once
    let instance = linker
        .instantiate_async(&mut store, component)
        .await
        .map_err(|e| format!("failed to instantiate component: {e}"))?;

    // Try p3 Command first (like wasmtime CLI does)
    if let Ok(command) = wasmtime_wasi::p3::bindings::Command::new(&mut store, &instance) {
        let result = store
            .run_concurrent(async |store| command.wasi_cli_run().call_run(store).await)
            .await
            .map_err(|e| format!("event loop failed: {e:?}"))?
            .map_err(|e| format!("component execution failed: {e:?}"))?;
        return Ok(match result {
            Ok(()) => 0,
            Err(()) => 1,
        });
    }

    // Fall back to p2 Command
    let command = wasmtime_wasi::p2::bindings::Command::new(&mut store, &instance)
        .map_err(|e| format!("failed to create command (tried p3 and p2): {e}"))?;

    match command.wasi_cli_run().call_run(&mut store).await {
        Ok(Ok(())) => Ok(0),
        Ok(Err(())) => Ok(1),
        Err(e) => Err(format!("component execution failed: {e:?}")),
    }
}
