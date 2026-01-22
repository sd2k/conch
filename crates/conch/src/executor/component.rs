//! WebAssembly runtime for executing shell scripts using the component model (wasip2).
//!
//! This module will handle loading and running the conch-shell WASM component using wasmtime
//! with the component model. It uses the same `InstancePre` pattern as [`CoreShellExecutor`]
//! for efficient per-execution instantiation.
//!
//! # Current Status
//!
//! The component executor is a work in progress. The shell currently uses C-style FFI exports
//! (`execute`, `get_stdout`, etc.) which work with the core module executor. A future version
//! will add proper WIT interfaces to the shell component.
//!
//! # Future Architecture
//!
//! The component executor will:
//! 1. Load the wasip2 component
//! 2. Use WASI filesystem shadowing to inject the VFS for `/ctx/*` paths
//! 3. Call the shell via a typed WIT interface
//!
//! For now, use [`CoreShellExecutor`] which works with the current shell implementation.

use std::path::Path;
use std::sync::Arc;

use wasmtime::component::{Component, Linker, ResourceTable};
use wasmtime::{Config, Engine};
use wasmtime_wasi::p2::pipe::MemoryOutputPipe;
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

use crate::limits::ResourceLimits;
use crate::runtime::{ExecutionResult, RuntimeError};

/// Embedded WASM component bytes (when built with `embedded-shell` feature).
#[cfg(feature = "embedded-shell")]
static EMBEDDED_COMPONENT: &[u8] =
    include_bytes!("../../../../target/wasm32-wasip2/release/conch_shell.wasm");

/// State held by the WASM store during execution.
#[allow(dead_code)]
pub struct ComponentState {
    wasi: WasiCtx,
    table: ResourceTable,
    stdout_pipe: MemoryOutputPipe,
    stderr_pipe: MemoryOutputPipe,
    limiter: StoreLimiter,
}

#[allow(dead_code)]
impl ComponentState {
    /// Create a new component state with fresh pipes for capturing output.
    fn new(stdout_capacity: usize, stderr_capacity: usize, max_memory_bytes: u64) -> Self {
        let stdout_pipe = MemoryOutputPipe::new(stdout_capacity);
        let stderr_pipe = MemoryOutputPipe::new(stderr_capacity);

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

/// Executor for running shell scripts in WASM (component model version).
///
/// **Note:** This executor is not yet fully functional. Use [`CoreShellExecutor`] instead.
///
/// This will pre-link the WASI component once at construction time, then efficiently
/// instantiate per execution call. The `Engine` and `InstancePre` are shared
/// across all executions, while each execution gets its own `Store` with fresh
/// WASI context and output pipes.
#[derive(Clone)]
pub struct ComponentShellExecutor {
    engine: Arc<Engine>,
    #[allow(dead_code)]
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
        wasmtime_wasi::p2::add_to_linker_async(&mut linker)
            .map_err(|e| RuntimeError::Wasm(e.to_string()))?;

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
    /// **Note:** This method is not yet implemented. Use [`CoreShellExecutor`] instead.
    pub async fn execute(
        &self,
        _script: &str,
        limits: &ResourceLimits,
    ) -> Result<ExecutionResult, RuntimeError> {
        // TODO: Implement proper component model execution once the shell
        // has WIT-based exports instead of C FFI exports.
        //
        // For now, return an error directing users to use CoreShellExecutor.
        let _ = limits;
        Err(RuntimeError::Wasm(
            "ComponentShellExecutor not yet implemented - use CoreShellExecutor".to_string(),
        ))
    }
}

/// Simple memory limiter for WASM execution.
#[allow(dead_code)]
struct StoreLimiter {
    max_memory: u64,
}

#[allow(dead_code)]
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
