//! WebAssembly runtime for executing shell scripts using core modules (wasip1).
//!
//! This module handles loading and running the conch-shell WASM module using wasmtime.
//! It uses `InstancePre` to pre-link the module once, then efficiently instantiate
//! per execution call (only creating a new Store, not re-linking).

use std::path::Path;
use std::sync::Arc;

use wasmtime::{Config, Engine, InstancePre, Linker, Module, Store, TypedFunc};
use wasmtime_wasi::WasiCtxBuilder;
use wasmtime_wasi::p1::WasiP1Ctx;
use wasmtime_wasi::p2::pipe::MemoryOutputPipe;

use crate::limits::ResourceLimits;
use crate::runtime::{ExecutionResult, ExecutionStats, RuntimeError};

/// Embedded WASM module bytes (when built with `embedded-shell` feature).
#[cfg(feature = "embedded-shell")]
static EMBEDDED_SHELL: &[u8] =
    include_bytes!("../../../target/wasm32-wasip1/release/conch_shell.wasm");

/// State held by the WASM store during execution.
pub struct ShellState {
    wasi: WasiP1Ctx,
    stdout_pipe: MemoryOutputPipe,
    stderr_pipe: MemoryOutputPipe,
    limiter: StoreLimiter,
}

impl ShellState {
    /// Create a new shell state with fresh pipes for capturing output.
    fn new(stdout_capacity: usize, stderr_capacity: usize, max_memory_bytes: u64) -> Self {
        let stdout_pipe = MemoryOutputPipe::new(stdout_capacity);
        let stderr_pipe = MemoryOutputPipe::new(stderr_capacity);

        let wasi = WasiCtxBuilder::new()
            .stdout(stdout_pipe.clone())
            .stderr(stderr_pipe.clone())
            .build_p1();

        Self {
            wasi,
            stdout_pipe,
            stderr_pipe,
            limiter: StoreLimiter::new(max_memory_bytes),
        }
    }

    /// Get the captured stdout contents.
    fn stdout(&self) -> Vec<u8> {
        self.stdout_pipe.contents().to_vec()
    }

    /// Get the captured stderr contents.
    fn stderr(&self) -> Vec<u8> {
        self.stderr_pipe.contents().to_vec()
    }
}

/// Executor for running shell scripts in WASM (core module version).
///
/// This pre-links the WASI module once at construction time, then efficiently
/// instantiates per execution call. The `Engine` and `InstancePre` are shared
/// across all executions, while each execution gets its own `Store` with fresh
/// WASI context and output pipes.
///
/// # Example
///
/// ```ignore
/// let executor = CoreShellExecutor::embedded()?;
///
/// // Each execute call creates a fresh Store but reuses the pre-linked instance
/// let result1 = executor.execute("echo hello", &ResourceLimits::default()).await?;
/// let result2 = executor.execute("echo world", &ResourceLimits::default()).await?;
/// ```
#[derive(Clone)]
pub struct CoreShellExecutor {
    engine: Arc<Engine>,
    instance_pre: Arc<InstancePre<ShellState>>,
}

impl std::fmt::Debug for CoreShellExecutor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CoreShellExecutor").finish_non_exhaustive()
    }
}

impl CoreShellExecutor {
    /// Create a new executor by loading a module from a file.
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, RuntimeError> {
        let engine = Self::create_engine()?;
        let module = Module::from_file(&engine, path.as_ref())
            .map_err(|e| RuntimeError::Wasm(e.to_string()))?;
        Self::from_module(engine, module)
    }

    /// Create a new executor by loading a module from bytes.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, RuntimeError> {
        let engine = Self::create_engine()?;
        let module = Module::new(&engine, bytes).map_err(|e| RuntimeError::Wasm(e.to_string()))?;
        Self::from_module(engine, module)
    }

    /// Create a new executor using the embedded WASM module.
    ///
    /// This is only available when built with the `embedded-shell` feature.
    #[cfg(feature = "embedded-shell")]
    pub fn embedded() -> Result<Self, RuntimeError> {
        Self::from_bytes(EMBEDDED_SHELL)
    }

    /// Get the embedded module bytes (if available).
    #[cfg(feature = "embedded-shell")]
    pub fn embedded_module_bytes() -> &'static [u8] {
        EMBEDDED_SHELL
    }

    /// Create an engine with the appropriate configuration.
    fn create_engine() -> Result<Engine, RuntimeError> {
        let mut config = Config::new();
        config.async_support(true);
        // Enable epoch-based interruption for CPU limiting
        config.epoch_interruption(true);
        Engine::new(&config).map_err(|e| RuntimeError::Wasm(e.to_string()))
    }

    /// Create an executor from an existing engine and module.
    fn from_module(engine: Engine, module: Module) -> Result<Self, RuntimeError> {
        // Create linker and add WASI imports once
        let mut linker = Linker::new(&engine);
        wasmtime_wasi::p1::add_to_linker_async(&mut linker, |s: &mut ShellState| &mut s.wasi)
            .map_err(|e| RuntimeError::Wasm(e.to_string()))?;

        // Pre-instantiate: does all the linking work upfront
        let instance_pre = linker
            .instantiate_pre(&module)
            .map_err(|e| RuntimeError::Wasm(e.to_string()))?;

        Ok(Self {
            engine: Arc::new(engine),
            instance_pre: Arc::new(instance_pre),
        })
    }

    /// Get a reference to the underlying engine.
    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    /// Execute a shell script asynchronously.
    ///
    /// Each call creates a fresh `Store` with new WASI context and output pipes,
    /// but reuses the pre-linked instance for fast instantiation.
    pub async fn execute(
        &self,
        script: &str,
        limits: &ResourceLimits,
    ) -> Result<ExecutionResult, RuntimeError> {
        let start = std::time::Instant::now();

        // Calculate output capacity from limits
        let output_capacity = limits.max_output_bytes as usize;

        // Create fresh state for this execution
        let state = ShellState::new(output_capacity, output_capacity, limits.max_memory_bytes);
        let mut store = Store::new(&self.engine, state);

        // Configure resource limits - closure returns reference to limiter in state
        store.limiter(|state| &mut state.limiter);

        // Configure epoch-based CPU interruption
        // Use timeout if set, otherwise fall back to max_cpu_ms
        let deadline_ms = limits.timeout.as_millis() as u64;
        store.set_epoch_deadline(deadline_ms.max(1));

        // Instantiate from pre-linked instance (fast - no re-linking needed)
        let instance = self
            .instance_pre
            .instantiate_async(&mut store)
            .await
            .map_err(|e| RuntimeError::Wasm(e.to_string()))?;

        // Get memory and exported execute function
        let memory = instance
            .get_memory(&mut store, "memory")
            .ok_or_else(|| RuntimeError::Wasm("no memory export".to_string()))?;

        let execute_fn: TypedFunc<(i32, i32), i32> = instance
            .get_typed_func(&mut store, "execute")
            .map_err(|e| RuntimeError::Wasm(format!("no execute function: {}", e)))?;

        // Allocate memory for the script string
        // For now, we'll write directly to a known location in memory
        // A proper implementation would use the module's allocator
        let script_bytes = script.as_bytes();
        let script_ptr = 1024; // Arbitrary offset, assuming memory is big enough

        memory
            .write(&mut store, script_ptr as usize, script_bytes)
            .map_err(|e| RuntimeError::Wasm(format!("failed to write script: {}", e)))?;

        // Call execute asynchronously
        let exit_code = execute_fn
            .call_async(&mut store, (script_ptr, script_bytes.len() as i32))
            .await
            .map_err(|e| RuntimeError::Wasm(format!("execute failed: {}", e)))?;

        // Get captured stdout/stderr from the pipes
        let stdout = store.data().stdout();
        let stderr = store.data().stderr();

        // Check if output was truncated
        let truncated = stdout.len() >= output_capacity || stderr.len() >= output_capacity;

        Ok(ExecutionResult {
            exit_code,
            stdout,
            stderr,
            truncated,
            stats: ExecutionStats {
                cpu_time_ms: 0,       // TODO: track actual CPU time via epochs
                peak_memory_bytes: 0, // TODO: track via limiter
                wall_time_ms: start.elapsed().as_millis() as u64,
            },
        })
    }
}

/// Simple memory limiter for WASM execution.
struct StoreLimiter {
    max_memory: u64,
}

impl StoreLimiter {
    fn new(max_memory: u64) -> Self {
        Self { max_memory }
    }
}

impl wasmtime::ResourceLimiter for StoreLimiter {
    fn memory_growing(
        &mut self,
        current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> anyhow::Result<bool> {
        Ok(desired as u64 <= self.max_memory || current == desired)
    }

    fn table_growing(
        &mut self,
        _current: usize,
        _desired: usize,
        _maximum: Option<usize>,
    ) -> anyhow::Result<bool> {
        Ok(true)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn get_executor() -> Option<CoreShellExecutor> {
        let workspace_root = std::env::var("CARGO_MANIFEST_DIR")
            .map(|p| {
                std::path::PathBuf::from(p)
                    .parent()
                    .unwrap()
                    .parent()
                    .unwrap()
                    .to_path_buf()
            })
            .unwrap_or_else(|_| std::path::PathBuf::from("."));

        let module_path = workspace_root.join("target/wasm32-wasip1/release/conch_shell.wasm");

        if module_path.exists() {
            CoreShellExecutor::from_file(&module_path).ok()
        } else {
            eprintln!("Module not found at {:?}", module_path);
            None
        }
    }

    #[tokio::test]
    async fn test_core_execute_echo() {
        let Some(executor) = get_executor() else {
            eprintln!(
                "Skipping test: module not built. Run: cargo build -p conch-shell --target wasm32-wasip1 --release"
            );
            return;
        };

        let result = executor
            .execute("echo hello", &ResourceLimits::default())
            .await
            .expect("execute failed");

        assert_eq!(result.exit_code, 0);
    }

    #[tokio::test]
    async fn test_core_execute_variable() {
        let Some(executor) = get_executor() else {
            return;
        };

        let result = executor
            .execute("x=42; echo $x", &ResourceLimits::default())
            .await
            .expect("execute failed");

        assert_eq!(result.exit_code, 0);
    }

    #[tokio::test]
    async fn test_core_execute_false() {
        let Some(executor) = get_executor() else {
            return;
        };

        let result = executor
            .execute("false", &ResourceLimits::default())
            .await
            .expect("execute failed");

        assert_eq!(result.exit_code, 1);
    }

    #[tokio::test]
    async fn test_multiple_executions_reuse_instance_pre() {
        let Some(executor) = get_executor() else {
            return;
        };

        // Multiple executions should all work and be independent
        let result1 = executor
            .execute("x=hello; echo $x", &ResourceLimits::default())
            .await
            .expect("execute 1 failed");

        let result2 = executor
            .execute("y=world; echo $y", &ResourceLimits::default())
            .await
            .expect("execute 2 failed");

        let result3 = executor
            .execute("echo done", &ResourceLimits::default())
            .await
            .expect("execute 3 failed");

        assert_eq!(result1.exit_code, 0);
        assert_eq!(result2.exit_code, 0);
        assert_eq!(result3.exit_code, 0);
    }

    #[tokio::test]
    async fn test_executor_is_clone() {
        let Some(executor) = get_executor() else {
            return;
        };

        // Cloning should be cheap (Arc)
        let executor2 = executor.clone();

        let result1 = executor
            .execute("echo from original", &ResourceLimits::default())
            .await
            .expect("execute failed");

        let result2 = executor2
            .execute("echo from clone", &ResourceLimits::default())
            .await
            .expect("execute failed");

        assert_eq!(result1.exit_code, 0);
        assert_eq!(result2.exit_code, 0);
    }
}
