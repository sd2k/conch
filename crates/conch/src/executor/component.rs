//! WebAssembly runtime for executing shell scripts using the component model (wasip2).
//!
//! This module handles loading and running the conch-shell WASM component using wasmtime
//! with the component model. It uses the `InstancePre` pattern for efficient per-execution
//! instantiation.

use std::path::Path;
use std::sync::Arc;

use wasmtime::component::{Component, Linker, ResourceTable};
use wasmtime::{Config, Engine, Store};
use wasmtime_wasi::p2::pipe::MemoryOutputPipe;
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

use crate::limits::ResourceLimits;
use crate::runtime::{ExecutionResult, RuntimeError};

// Generate host bindings for the shell component.
// This creates the `Shell` struct for calling into the component.
wasmtime::component::bindgen!({
    path: "wit/shell.wit",
    world: "shell",
    // Enable async for exported functions (we call them from the host)
    exports: { default: async },
});

/// Embedded WASM component bytes (when built with `embedded-shell` feature).
#[cfg(feature = "embedded-shell")]
static EMBEDDED_COMPONENT: &[u8] =
    include_bytes!("../../../../target/wasm32-wasip2/release/conch_shell.wasm");

/// State held by the WASM store during execution.
pub struct ComponentState {
    wasi: WasiCtx,
    table: ResourceTable,
    stdout_pipe: MemoryOutputPipe,
    stderr_pipe: MemoryOutputPipe,
    limiter: StoreLimiter,
}

impl ComponentState {
    /// Create a new component state with fresh pipes for capturing output.
    fn new(output_capacity: usize, max_memory_bytes: u64) -> Self {
        let stdout_pipe = MemoryOutputPipe::new(output_capacity);
        let stderr_pipe = MemoryOutputPipe::new(output_capacity);

        let wasi = WasiCtxBuilder::new()
            .stdout(stdout_pipe.clone())
            .stderr(stderr_pipe.clone())
            .build();

        Self {
            wasi,
            table: ResourceTable::new(),
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

impl WasiView for ComponentState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

/// Executor for running shell scripts in WASM using the component model.
///
/// This pre-links the WASI component once at construction time, then efficiently
/// instantiate per execution call. The `Engine` and `InstancePre` are shared
/// across all executions, while each execution gets its own `Store` with fresh
/// WASI context and output pipes.
#[derive(Clone)]
pub struct ComponentShellExecutor {
    engine: Arc<Engine>,
    instance_pre: Arc<wasmtime::component::InstancePre<ComponentState>>,
}

impl std::fmt::Debug for ComponentShellExecutor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ComponentShellExecutor")
            .finish_non_exhaustive()
    }
}

impl ComponentShellExecutor {
    /// Create a new executor by loading a component from a file.
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, RuntimeError> {
        let engine = Self::create_engine()?;
        let component = Component::from_file(&engine, path.as_ref())
            .map_err(|e| RuntimeError::Wasm(e.to_string()))?;
        Self::from_component(engine, component)
    }

    /// Create a new executor by loading a component from bytes.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, RuntimeError> {
        let engine = Self::create_engine()?;
        let component =
            Component::new(&engine, bytes).map_err(|e| RuntimeError::Wasm(e.to_string()))?;
        Self::from_component(engine, component)
    }

    /// Create a new executor using the embedded WASM component.
    ///
    /// This is only available when built with the `embedded-shell` feature.
    #[cfg(feature = "embedded-shell")]
    pub fn embedded() -> Result<Self, RuntimeError> {
        Self::from_bytes(EMBEDDED_COMPONENT)
    }

    /// Get the embedded component bytes (if available).
    #[cfg(feature = "embedded-shell")]
    pub fn embedded_component_bytes() -> &'static [u8] {
        EMBEDDED_COMPONENT
    }

    /// Create an engine with the appropriate configuration.
    fn create_engine() -> Result<Engine, RuntimeError> {
        let mut config = Config::new();
        config.async_support(true);
        config.epoch_interruption(true);
        Engine::new(&config).map_err(|e| RuntimeError::Wasm(e.to_string()))
    }

    /// Create an executor from an existing engine and component.
    fn from_component(engine: Engine, component: Component) -> Result<Self, RuntimeError> {
        let mut linker = Linker::new(&engine);

        // Add WASI imports
        wasmtime_wasi::p2::add_to_linker_async(&mut linker)
            .map_err(|e| RuntimeError::Wasm(e.to_string()))?;

        // Pre-instantiate the component for efficient per-call instantiation
        let instance_pre = linker
            .instantiate_pre(&component)
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
    /// Each execution gets a fresh store with isolated WASI context.
    /// Output is captured via memory pipes and returned in the result.
    pub async fn execute(
        &self,
        script: &str,
        limits: &ResourceLimits,
    ) -> Result<ExecutionResult, RuntimeError> {
        // Create fresh state for this execution
        let state = ComponentState::new(limits.max_output_bytes as usize, limits.max_memory_bytes);

        let mut store = Store::new(&self.engine, state);

        // Set up resource limiter
        store.limiter(|state| &mut state.limiter);

        // Set up epoch-based timeout
        store.set_epoch_deadline(limits.max_cpu_ms);

        // Increment epoch in background for timeout
        let engine = self.engine.clone();
        let timeout_ms = limits.max_cpu_ms;
        let epoch_handle = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(timeout_ms)).await;
            engine.increment_epoch();
        });

        // Instantiate the component
        let instance = self
            .instance_pre
            .instantiate_async(&mut store)
            .await
            .map_err(|e| RuntimeError::Wasm(format!("instantiation failed: {}", e)))?;

        // Get the shell interface and call execute
        let shell = Shell::new(&mut store, &instance)
            .map_err(|e| RuntimeError::Wasm(format!("failed to get shell interface: {}", e)))?;

        let result = shell.call_execute(&mut store, script).await.map_err(|e| {
            // Check if this was a timeout
            if e.to_string().contains("epoch") {
                RuntimeError::Timeout
            } else {
                RuntimeError::Wasm(format!("execute failed: {}", e))
            }
        })?;

        // Cancel the timeout task
        epoch_handle.abort();

        // Get captured output from the store
        let state = store.data();
        let captured_stdout = state.stdout();
        let captured_stderr = state.stderr();

        // Combine captured output with result output
        // The WIT interface returns stdout/stderr from the shell,
        // but WASI stdout/stderr are captured separately
        let mut stdout = result.stdout;
        let mut stderr = result.stderr;

        // If the WIT result is empty but WASI captured output, use that
        if stdout.is_empty() && !captured_stdout.is_empty() {
            stdout = captured_stdout;
        }
        if stderr.is_empty() && !captured_stderr.is_empty() {
            stderr = captured_stderr;
        }

        Ok(ExecutionResult {
            exit_code: result.exit_code,
            stdout,
            stderr,
            truncated: false,
            stats: crate::runtime::ExecutionStats::default(),
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
#[cfg(feature = "embedded-shell")]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_component_execute_echo() {
        let executor = ComponentShellExecutor::embedded().expect("Failed to create executor");
        let limits = ResourceLimits::default();

        let result = executor
            .execute("echo hello", &limits)
            .await
            .expect("execute failed");

        assert_eq!(
            result.exit_code,
            0,
            "stderr: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        let stdout = String::from_utf8_lossy(&result.stdout);
        assert!(stdout.contains("hello"), "stdout: {:?}", stdout);
    }

    #[tokio::test]
    async fn test_component_execute_variable() {
        let executor = ComponentShellExecutor::embedded().expect("Failed to create executor");
        let limits = ResourceLimits::default();

        let result = executor
            .execute("x=world; echo hello $x", &limits)
            .await
            .expect("execute failed");

        assert_eq!(
            result.exit_code,
            0,
            "stderr: {}",
            String::from_utf8_lossy(&result.stderr)
        );
    }

    #[tokio::test]
    async fn test_component_execute_false() {
        let executor = ComponentShellExecutor::embedded().expect("Failed to create executor");
        let limits = ResourceLimits::default();

        let result = executor
            .execute("false", &limits)
            .await
            .expect("execute failed");

        assert_eq!(result.exit_code, 1);
    }

    #[tokio::test]
    async fn test_multiple_executions_reuse_instance_pre() {
        let executor = ComponentShellExecutor::embedded().expect("Failed to create executor");
        let limits = ResourceLimits::default();

        // Execute multiple scripts - each should get fresh state
        for i in 0..3 {
            let result = executor
                .execute(&format!("echo run {}", i), &limits)
                .await
                .expect("execute failed");
            assert_eq!(result.exit_code, 0);
        }
    }

    #[tokio::test]
    async fn test_executor_is_clone() {
        let executor = ComponentShellExecutor::embedded().expect("Failed to create executor");
        let executor2 = executor.clone();
        let limits = ResourceLimits::default();

        // Both executors should work independently
        let result1 = executor
            .execute("echo one", &limits)
            .await
            .expect("execute failed");
        let result2 = executor2
            .execute("echo two", &limits)
            .await
            .expect("execute failed");

        assert_eq!(result1.exit_code, 0);
        assert_eq!(result2.exit_code, 0);
    }
}
