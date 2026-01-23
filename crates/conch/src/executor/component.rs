//! WebAssembly runtime for executing shell scripts using the component model (wasip2).
//!
//! This module handles loading and running the conch-shell WASM component using wasmtime
//! with the component model. It uses the `InstancePre` pattern for efficient per-execution
//! instantiation.
//!
//! ## VFS Integration
//!
//! The executor supports two modes:
//!
//! - **Basic mode**: Standard WASI filesystem (no VFS)
//! - **Hybrid VFS mode**: Combines virtual storage with real filesystem mounts

use std::path::Path;
use std::sync::Arc;

use eryx_vfs::{HybridVfsCtx, VfsStorage};
#[cfg(feature = "embedded-shell")]
use eryx_vfs::{HybridVfsState, HybridVfsView, add_hybrid_vfs_to_linker};
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

/// State held by the WASM store during hybrid VFS execution.
///
/// This state supports the Shell API with hybrid VFS that combines
/// virtual storage with real filesystem mounts.
#[cfg(feature = "embedded-shell")]
pub struct HybridComponentState<S: VfsStorage + 'static> {
    wasi: WasiCtx,
    table: ResourceTable,
    stdout_pipe: MemoryOutputPipe,
    stderr_pipe: MemoryOutputPipe,
    limiter: StoreLimiter,
    hybrid_vfs_ctx: HybridVfsCtx<S>,
}

#[cfg(feature = "embedded-shell")]
impl<S: VfsStorage + 'static> HybridComponentState<S> {
    /// Create a new hybrid component state.
    fn new(output_capacity: usize, max_memory_bytes: u64, hybrid_vfs_ctx: HybridVfsCtx<S>) -> Self {
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
            hybrid_vfs_ctx,
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

#[cfg(feature = "embedded-shell")]
impl<S: VfsStorage + 'static> WasiView for HybridComponentState<S> {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

#[cfg(feature = "embedded-shell")]
impl<S: VfsStorage + 'static> HybridVfsView for HybridComponentState<S> {
    type Storage = S;

    fn hybrid_vfs(&mut self) -> HybridVfsState<'_, Self::Storage> {
        HybridVfsState::new(&mut self.hybrid_vfs_ctx, &mut self.table)
    }
}

/// Executor for running shell scripts in WASM using the component model.
///
/// This pre-links the WASI component once at construction time, then efficiently
/// instantiates per execution call. The `Engine` and `InstancePre` are shared
/// across all executions, while each execution gets its own `Store` with fresh
/// WASI context and output pipes.
///
/// ## VFS Support
///
/// The executor supports two modes:
/// - **Basic mode**: Uses standard WASI filesystem (default)
/// - **Hybrid VFS mode**: Combines virtual storage with real filesystem mounts
///
/// For hybrid VFS, use the [`Shell`](crate::Shell) API which provides a higher-level
/// interface with builder pattern for configuring mounts.
#[derive(Clone)]
pub struct ComponentShellExecutor {
    engine: Arc<Engine>,
    /// Pre-instantiated component for basic execution (WASI only).
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
        // Create basic linker with WASI only
        let mut linker = Linker::new(&engine);
        wasmtime_wasi::p2::add_to_linker_async(&mut linker)
            .map_err(|e| RuntimeError::Wasm(e.to_string()))?;

        // Pre-instantiate for basic execution
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

        // Handle the result - the WIT interface returns Result<exit_code, error_message>
        let exit_code = match result {
            Ok(code) => code,
            Err(error_msg) => {
                // Shell initialization/execution error - return as stderr
                return Ok(ExecutionResult {
                    exit_code: 1,
                    stdout: Vec::new(),
                    stderr: error_msg.into_bytes(),
                    truncated: false,
                    stats: crate::runtime::ExecutionStats::default(),
                });
            }
        };

        // Get captured output from the WASI pipes.
        // The shell writes to WASI stdout/stderr which we intercept via MemoryOutputPipe.
        let state = store.data();
        let stdout = state.stdout();
        let stderr = state.stderr();

        Ok(ExecutionResult {
            exit_code,
            stdout,
            stderr,
            truncated: false,
            stats: crate::runtime::ExecutionStats::default(),
        })
    }

    /// Execute a shell script with hybrid VFS (virtual storage + real filesystem).
    ///
    /// This method supports the Shell API that combines:
    /// - VFS storage paths (backed by `VfsStorage` trait)
    /// - Real filesystem mounts (backed by cap-std)
    ///
    /// The `HybridVfsCtx` should be pre-configured with preopened directories
    /// via `add_vfs_preopen()` and `add_real_preopen()`.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let storage = Arc::new(InMemoryStorage::new());
    /// let mut hybrid_ctx = HybridVfsCtx::new(storage);
    /// hybrid_ctx.add_vfs_preopen("/scratch", DirPerms::all(), FilePerms::all());
    /// hybrid_ctx.add_real_preopen_path("/project", "./code", DirPerms::READ, FilePerms::READ)?;
    ///
    /// let result = executor.execute_with_hybrid_vfs(
    ///     "cat /project/README.md",
    ///     &limits,
    ///     hybrid_ctx,
    /// ).await?;
    /// ```
    #[cfg(feature = "embedded-shell")]
    pub async fn execute_with_hybrid_vfs<S: VfsStorage + 'static>(
        &self,
        script: &str,
        limits: &ResourceLimits,
        hybrid_ctx: HybridVfsCtx<S>,
    ) -> Result<ExecutionResult, RuntimeError> {
        // Create state with hybrid VFS context
        let state = HybridComponentState::new(
            limits.max_output_bytes as usize,
            limits.max_memory_bytes,
            hybrid_ctx,
        );

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

        // We need a linker for hybrid VFS - create it on demand
        // Note: This is less efficient than pre-instantiation, but hybrid VFS
        // requires the storage type at link time. A future optimization could
        // cache linkers per storage type.
        let mut linker = Linker::<HybridComponentState<S>>::new(&self.engine);
        wasmtime_wasi::p2::add_to_linker_async(&mut linker)
            .map_err(|e| RuntimeError::Wasm(e.to_string()))?;
        linker.allow_shadowing(true);
        add_hybrid_vfs_to_linker(&mut linker).map_err(|e| {
            RuntimeError::Wasm(format!("failed to add hybrid VFS to linker: {}", e))
        })?;
        linker.allow_shadowing(false);

        // Load the component
        let component = Component::new(&self.engine, EMBEDDED_COMPONENT)
            .map_err(|e| RuntimeError::Wasm(e.to_string()))?;

        // Instantiate the component
        let instance = linker
            .instantiate_async(&mut store, &component)
            .await
            .map_err(|e| RuntimeError::Wasm(format!("instantiation failed: {}", e)))?;

        // Get the shell interface and call execute
        let shell = Shell::new(&mut store, &instance)
            .map_err(|e| RuntimeError::Wasm(format!("failed to get shell interface: {}", e)))?;

        let result = shell.call_execute(&mut store, script).await.map_err(|e| {
            if e.to_string().contains("epoch") {
                RuntimeError::Timeout
            } else {
                RuntimeError::Wasm(format!("execute failed: {}", e))
            }
        })?;

        // Cancel the timeout task
        epoch_handle.abort();

        // Handle the result
        let exit_code = match result {
            Ok(code) => code,
            Err(error_msg) => {
                return Ok(ExecutionResult {
                    exit_code: 1,
                    stdout: Vec::new(),
                    stderr: error_msg.into_bytes(),
                    truncated: false,
                    stats: crate::runtime::ExecutionStats::default(),
                });
            }
        };

        // Get captured output
        let state = store.data();
        let stdout = state.stdout();
        let stderr = state.stderr();

        Ok(ExecutionResult {
            exit_code,
            stdout,
            stderr,
            truncated: false,
            stats: crate::runtime::ExecutionStats::default(),
        })
    }

    /// Execute a shell script with hybrid VFS (stub for when embedded-shell is disabled).
    #[cfg(not(feature = "embedded-shell"))]
    pub async fn execute_with_hybrid_vfs<S: VfsStorage + 'static>(
        &self,
        _script: &str,
        _limits: &ResourceLimits,
        _hybrid_ctx: HybridVfsCtx<S>,
    ) -> Result<ExecutionResult, RuntimeError> {
        Err(RuntimeError::Wasm(
            "execute_with_hybrid_vfs requires embedded-shell feature".to_string(),
        ))
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
