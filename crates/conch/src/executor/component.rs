//! WebAssembly runtime for executing shell scripts using the component model (wasip2).
//!
//! This module handles loading and running the conch-shell WASM component using wasmtime
//! with the component model. It uses the `InstancePre` pattern for efficient per-execution
//! instantiation.
//!
//! ## VFS Integration
//!
//! When executing with a context provider, the executor exposes agent context via WASI
//! filesystem shadowing:
//!
//! - `/ctx/self/tools/<id>/` - Tool call history
//! - `/ctx/self/messages/` - Conversation history
//! - `/ctx/self/scratch/` - Read-write workspace
//! - `/tmp/` - Temporary scratch space (ephemeral)

use std::path::Path;
use std::sync::Arc;

use eryx_vfs::{VfsCtx, VfsState, VfsView, add_vfs_to_linker};
use wasmtime::component::{Component, Linker, ResourceTable};
use wasmtime::{Config, Engine, Store};
use wasmtime_wasi::p2::pipe::MemoryOutputPipe;
use wasmtime_wasi::{DirPerms, FilePerms, WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

use crate::limits::ResourceLimits;
use crate::runtime::{ExecutionResult, RuntimeError};
use crate::vfs::{AccessPolicy, ContextProvider, ContextStorage};

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
    /// Optional VFS context for exposing agent context as filesystem.
    vfs_ctx: Option<VfsCtx<ContextStorage>>,
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
            vfs_ctx: None,
        }
    }

    /// Create a new component state with VFS context for agent context access.
    fn with_vfs(
        output_capacity: usize,
        max_memory_bytes: u64,
        provider: Arc<dyn ContextProvider>,
        policy: AccessPolicy,
    ) -> Self {
        let stdout_pipe = MemoryOutputPipe::new(output_capacity);
        let stderr_pipe = MemoryOutputPipe::new(output_capacity);

        let wasi = WasiCtxBuilder::new()
            .stdout(stdout_pipe.clone())
            .stderr(stderr_pipe.clone())
            .build();

        // Create VFS context with preopened directories
        let storage = Arc::new(ContextStorage::new(provider, policy));
        let mut vfs_ctx = VfsCtx::new_empty(storage);
        // Preopen root "/" so all absolute paths work through VFS
        // The ContextStorage routes /ctx/* to context and /tmp/* to scratch
        vfs_ctx.preopen("/", DirPerms::READ, FilePerms::READ);

        Self {
            wasi,
            table: ResourceTable::new(),
            stdout_pipe,
            stderr_pipe,
            limiter: StoreLimiter::new(max_memory_bytes),
            vfs_ctx: Some(vfs_ctx),
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

impl VfsView for ComponentState {
    type Storage = ContextStorage;

    fn vfs(&mut self) -> VfsState<'_, Self::Storage> {
        VfsState {
            ctx: self.vfs_ctx.as_mut().expect("VFS context not initialized"),
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
///
/// ## VFS Support
///
/// The executor supports two modes:
/// - **Basic mode**: Uses standard WASI filesystem (default)
/// - **VFS mode**: Exposes agent context via WASI filesystem shadowing
///
/// Use `execute_with_context()` to run scripts with VFS access to agent context.
#[derive(Clone)]
pub struct ComponentShellExecutor {
    engine: Arc<Engine>,
    /// Pre-instantiated component for basic execution (WASI only).
    instance_pre: Arc<wasmtime::component::InstancePre<ComponentState>>,
    /// Pre-instantiated component for VFS execution (WASI + VFS shadowing).
    instance_pre_vfs: Arc<wasmtime::component::InstancePre<ComponentState>>,
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

        // Create VFS linker: WASI first, then VFS to shadow filesystem
        let mut linker_vfs = Linker::new(&engine);
        wasmtime_wasi::p2::add_to_linker_async(&mut linker_vfs)
            .map_err(|e| RuntimeError::Wasm(e.to_string()))?;
        // Enable shadowing to allow VFS to override WASI filesystem bindings
        linker_vfs.allow_shadowing(true);
        add_vfs_to_linker(&mut linker_vfs)
            .map_err(|e| RuntimeError::Wasm(format!("failed to add VFS to linker: {}", e)))?;
        linker_vfs.allow_shadowing(false);

        // Pre-instantiate for VFS execution
        let instance_pre_vfs = linker_vfs
            .instantiate_pre(&component)
            .map_err(|e| RuntimeError::Wasm(e.to_string()))?;

        Ok(Self {
            engine: Arc::new(engine),
            instance_pre: Arc::new(instance_pre),
            instance_pre_vfs: Arc::new(instance_pre_vfs),
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

    /// Execute a shell script with VFS access to agent context.
    ///
    /// This method exposes agent context as a virtual filesystem:
    /// - `/ctx/self/tools/<id>/` - Tool call history
    /// - `/ctx/self/messages/` - Conversation history
    /// - `/ctx/self/scratch/` - Read-write workspace
    /// - `/tmp/` - Temporary scratch space
    ///
    /// # Example
    ///
    /// ```ignore
    /// let provider = Arc::new(MyContextProvider::new());
    /// let result = executor.execute_with_context(
    ///     "cat /ctx/self/tools/latest/output.txt",
    ///     &limits,
    ///     provider,
    ///     AccessPolicy::default(),
    /// ).await?;
    /// ```
    pub async fn execute_with_context(
        &self,
        script: &str,
        limits: &ResourceLimits,
        provider: Arc<dyn ContextProvider>,
        policy: AccessPolicy,
    ) -> Result<ExecutionResult, RuntimeError> {
        // Create state with VFS context
        let state = ComponentState::with_vfs(
            limits.max_output_bytes as usize,
            limits.max_memory_bytes,
            provider,
            policy,
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

        // Instantiate using VFS-enabled pre-instance
        let instance = self
            .instance_pre_vfs
            .instantiate_async(&mut store)
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

    #[tokio::test]
    async fn test_execute_with_context_basic() {
        use crate::vfs::MockContextProvider;

        let executor = ComponentShellExecutor::embedded().expect("Failed to create executor");
        let limits = ResourceLimits::default();
        let provider = Arc::new(MockContextProvider::new());
        let policy = AccessPolicy::default();

        // Simple echo test to verify VFS executor works
        let result = executor
            .execute_with_context("echo 'vfs test'", &limits, provider, policy)
            .await
            .expect("execute failed");

        let stdout = String::from_utf8_lossy(&result.stdout);
        let stderr = String::from_utf8_lossy(&result.stderr);
        assert_eq!(
            result.exit_code, 0,
            "expected exit 0, got {}. stdout: {}, stderr: {}",
            result.exit_code, stdout, stderr
        );
        assert!(
            stdout.contains("vfs test"),
            "stdout should contain output: {}",
            stdout
        );
    }

    #[tokio::test]
    async fn test_execute_with_context_cat_file() {
        use crate::vfs::{MockContextProvider, ToolCall};

        let executor = ComponentShellExecutor::embedded().expect("Failed to create executor");
        let limits = ResourceLimits::default();

        // Create a mock provider with a test tool call
        let tool_call = ToolCall {
            id: "tool-0".to_string(),
            tool: "test_tool".to_string(),
            params: serde_json::json!({"arg": "value"}),
            result: serde_json::json!({"output": "test result"}),
            started_at: "2025-01-01T00:00:00Z".to_string(),
            completed_at: "2025-01-01T00:00:01Z".to_string(),
            duration_ms: 1000,
            success: true,
            error: None,
        };
        let provider =
            Arc::new(MockContextProvider::new().with_tool_calls("self", vec![tool_call]));
        let policy = AccessPolicy::default();

        // Try to cat the tool call request file from the VFS
        let result = executor
            .execute_with_context(
                "cat /ctx/self/tools/tool-0/request.json",
                &limits,
                provider,
                policy,
            )
            .await
            .expect("execute failed");

        let stdout = String::from_utf8_lossy(&result.stdout);
        let stderr = String::from_utf8_lossy(&result.stderr);

        assert_eq!(
            result.exit_code, 0,
            "cat should succeed. stdout: {}, stderr: {}",
            stdout, stderr
        );
        assert!(
            stdout.contains("arg"),
            "stdout should contain request JSON: {}",
            stdout
        );
    }
}
