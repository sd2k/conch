//! WebAssembly runtime for executing shell scripts using the component model (wasip2).
//!
//! This module handles loading and running the conch-shell WASM component using wasmtime
//! with the component model.
//!
//! ## Shell Resource
//!
//! The executor now supports creating persistent shell instances via the WIT `shell` resource.
//! Shell state (variables, functions, aliases) persists across multiple `execute` calls on
//! the same instance.
//!
//! ## VFS Integration
//!
//! The executor supports hybrid VFS mode that combines virtual storage with real filesystem mounts.

use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use eryx_vfs::{HybridVfsCtx, VfsStorage};
#[cfg(feature = "embedded-shell")]
use eryx_vfs::{HybridVfsState, HybridVfsView, add_hybrid_vfs_to_linker};
use wasmtime::component::{Component, HasSelf, Linker, ResourceTable};
use wasmtime::{Config, Engine, Store};
use wasmtime_wasi::p2::pipe::MemoryOutputPipe;
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

use crate::limits::ResourceLimits;
use crate::runtime::{ExecutionResult, RuntimeError};

// Generate host bindings for the shell component.
wasmtime::component::bindgen!({
    path: "wit/shell.wit",
    world: "shell-sandbox",
    // Make exports async so we can call them from async Rust code
    // when the engine has async_support(true) enabled.
    exports: { default: async },
});

// Re-export the generated types for public use
pub use conch::shell::tools::{ToolRequest, ToolResult};

/// Handler for tool invocations from shell scripts.
///
/// Implement this trait to handle `tool <name> --params` commands from shell scripts.
/// The handler is called asynchronously when the shell executes a tool command.
///
/// # Example
///
/// ```rust,ignore
/// use conch::{ToolHandler, ToolRequest, ToolResult};
///
/// struct MyToolHandler;
///
/// #[async_trait::async_trait]
/// impl ToolHandler for MyToolHandler {
///     async fn invoke(&self, request: ToolRequest) -> ToolResult {
///         match request.tool.as_str() {
///             "echo" => ToolResult {
///                 success: true,
///                 output: request.params.clone(),
///             },
///             _ => ToolResult {
///                 success: false,
///                 output: format!("Unknown tool: {}", request.tool),
///             },
///         }
///     }
/// }
/// ```
#[async_trait]
pub trait ToolHandler: Send + Sync {
    /// Invoke a tool with the given request.
    ///
    /// Called when a shell script runs `tool <name> [--param value]...`.
    async fn invoke(&self, request: ToolRequest) -> ToolResult;
}

/// Blanket implementation for async closures.
///
/// This allows using closures as tool handlers:
/// ```rust,ignore
/// shell.tool_handler(|req| async move {
///     ToolResult { success: true, output: format!("got {}", req.tool) }
/// });
/// ```
#[async_trait]
impl<F, Fut> ToolHandler for F
where
    F: Fn(ToolRequest) -> Fut + Send + Sync,
    Fut: std::future::Future<Output = ToolResult> + Send,
{
    async fn invoke(&self, request: ToolRequest) -> ToolResult {
        self(request).await
    }
}

/// Embedded WASM component bytes (when built with `embedded-shell` feature).
#[cfg(feature = "embedded-shell")]
static EMBEDDED_COMPONENT: &[u8] =
    include_bytes!("../../../../target/wasm32-wasip2/release/conch_shell.wasm");

/// Default tool handler that returns an error.
fn default_tool_handler(request: &ToolRequest) -> ToolResult {
    ToolResult {
        success: false,
        output: format!(
            "No tool handler configured. Cannot execute tool: {}",
            request.tool
        ),
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
    tool_handler: Option<Arc<dyn ToolHandler>>,
}

#[cfg(feature = "embedded-shell")]
impl<S: VfsStorage + 'static> HybridComponentState<S> {
    /// Create a new hybrid component state with an optional tool handler.
    pub fn new(
        output_capacity: usize,
        max_memory_bytes: u64,
        hybrid_vfs_ctx: HybridVfsCtx<S>,
        tool_handler: Option<Arc<dyn ToolHandler>>,
    ) -> Self {
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
            tool_handler,
        }
    }

    /// Get the captured stdout contents.
    pub fn stdout(&self) -> Vec<u8> {
        self.stdout_pipe.contents().to_vec()
    }

    /// Get the captured stderr contents.
    pub fn stderr(&self) -> Vec<u8> {
        self.stderr_pipe.contents().to_vec()
    }
}

#[cfg(feature = "embedded-shell")]
impl<S: VfsStorage + 'static> conch::shell::tools::Host for HybridComponentState<S> {
    fn invoke_tool(&mut self, request: ToolRequest) -> ToolResult {
        // For synchronous trait, we need to block on the async handler.
        // We use futures::executor::block_on instead of tokio's block_on
        // to avoid "cannot start a runtime from within a runtime" panic
        // since this is called from within WASM execution which is already
        // running in the tokio runtime.
        match &self.tool_handler {
            Some(handler) => futures::executor::block_on(handler.invoke(request)),
            None => default_tool_handler(&request),
        }
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
/// This handles loading the WASM component and setting up the execution environment.
/// For persistent shell state, use [`ShellInstance`] which maintains state across
/// multiple execute calls.
#[derive(Clone)]
pub struct ComponentShellExecutor {
    engine: Arc<Engine>,
    component_bytes: Arc<Vec<u8>>,
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
        let bytes = std::fs::read(path.as_ref())
            .map_err(|e| RuntimeError::Wasm(format!("failed to read component file: {}", e)))?;
        Self::from_bytes(&bytes)
    }

    /// Create a new executor by loading a component from bytes.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, RuntimeError> {
        let engine = Self::create_engine()?;
        // Validate the component can be loaded
        Component::new(&engine, bytes).map_err(|e| RuntimeError::Wasm(e.to_string()))?;
        Ok(Self {
            engine: Arc::new(engine),
            component_bytes: Arc::new(bytes.to_vec()),
        })
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

    /// Get a reference to the underlying engine.
    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    /// Create a persistent shell instance with hybrid VFS.
    ///
    /// This creates a new WASM instance with a shell resource that maintains
    /// state across multiple `execute` calls. The instance has its own isolated
    /// filesystem and shell state.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let storage = Arc::new(InMemoryStorage::new());
    /// let mut hybrid_ctx = HybridVfsCtx::new(storage);
    /// hybrid_ctx.add_vfs_preopen("/scratch", DirPerms::all(), FilePerms::all());
    ///
    /// let mut instance = executor.create_instance(
    ///     &limits,
    ///     hybrid_ctx,
    ///     None, // No tool handler
    /// ).await?;
    ///
    /// // State persists across calls
    /// instance.execute("x=42").await?;
    /// instance.execute("echo $x").await?;  // Outputs "42"
    /// ```
    #[cfg(feature = "embedded-shell")]
    pub async fn create_instance<S: VfsStorage + 'static>(
        &self,
        limits: &ResourceLimits,
        hybrid_ctx: HybridVfsCtx<S>,
        tool_handler: Option<Arc<dyn ToolHandler>>,
    ) -> Result<ShellInstance<S>, RuntimeError> {
        ShellInstance::new(
            self.engine.clone(),
            self.component_bytes.clone(),
            limits,
            hybrid_ctx,
            tool_handler,
        )
        .await
    }
}

/// A persistent shell instance with isolated filesystem and state.
///
/// This holds a WASM Store and shell resource handle, allowing shell state
/// (variables, functions, aliases) to persist across multiple `execute` calls.
///
/// Each `ShellInstance` has its own:
/// - WASM linear memory (isolated from other instances)
/// - Filesystem (via HybridVfsCtx)
/// - Shell state (variables, functions, aliases)
#[cfg(feature = "embedded-shell")]
pub struct ShellInstance<S: VfsStorage + 'static> {
    store: Store<HybridComponentState<S>>,
    shell_resource: wasmtime::component::ResourceAny,
    bindings: ShellSandbox,
    engine: Arc<Engine>,
}

#[cfg(feature = "embedded-shell")]
impl<S: VfsStorage + 'static> std::fmt::Debug for ShellInstance<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ShellInstance").finish_non_exhaustive()
    }
}

#[cfg(feature = "embedded-shell")]
impl<S: VfsStorage + 'static> ShellInstance<S> {
    /// Create a new shell instance.
    async fn new(
        engine: Arc<Engine>,
        component_bytes: Arc<Vec<u8>>,
        limits: &ResourceLimits,
        hybrid_ctx: HybridVfsCtx<S>,
        tool_handler: Option<Arc<dyn ToolHandler>>,
    ) -> Result<Self, RuntimeError> {
        // Create state with hybrid VFS context
        let state = HybridComponentState::new(
            limits.max_output_bytes as usize,
            limits.max_memory_bytes,
            hybrid_ctx,
            tool_handler,
        );

        let mut store = Store::new(&engine, state);

        // Set up resource limiter
        store.limiter(|state| &mut state.limiter);

        // Create linker with WASI and hybrid VFS
        let mut linker = Linker::<HybridComponentState<S>>::new(&engine);
        wasmtime_wasi::p2::add_to_linker_async(&mut linker)
            .map_err(|e| RuntimeError::Wasm(e.to_string()))?;
        linker.allow_shadowing(true);
        add_hybrid_vfs_to_linker(&mut linker).map_err(|e| {
            RuntimeError::Wasm(format!("failed to add hybrid VFS to linker: {}", e))
        })?;
        // Add shell imports (invoke-tool) using HasSelf wrapper for type projection
        ShellSandbox::add_to_linker::<_, HasSelf<HybridComponentState<S>>>(&mut linker, |state| {
            state
        })
        .map_err(|e| RuntimeError::Wasm(e.to_string()))?;
        linker.allow_shadowing(false);

        // Load the component
        let component = Component::new(&engine, &*component_bytes)
            .map_err(|e| RuntimeError::Wasm(e.to_string()))?;

        // Instantiate the component
        let bindings = ShellSandbox::instantiate_async(&mut store, &component, &linker)
            .await
            .map_err(|e| RuntimeError::Wasm(format!("instantiation failed: {}", e)))?;

        // Set a generous epoch deadline for constructor (shell initialization can take time)
        // The constructor creates a tokio runtime and brush shell, which is expensive.
        store.set_epoch_deadline(u64::MAX);

        // Create the shell resource via its constructor
        let shell_interface = bindings.conch_shell_shell();
        let shell_resource = shell_interface
            .instance()
            .call_constructor(&mut store)
            .await
            .map_err(|e| {
                // Try to get more details from the error
                let mut details = format!("failed to create shell instance: {}", e);
                if let Some(source) = e.source() {
                    details.push_str(&format!("\n  caused by: {}", source));
                }
                // Check for trap info
                if let Some(trap) = e.downcast_ref::<wasmtime::Trap>() {
                    details.push_str(&format!("\n  trap: {:?}", trap));
                }
                RuntimeError::Wasm(details)
            })?;

        Ok(Self {
            store,
            shell_resource,
            bindings,
            engine,
        })
    }

    /// Execute a shell script.
    ///
    /// Variables, functions, and aliases defined in previous calls persist.
    /// stdout/stderr are captured and returned in the result.
    pub async fn execute(
        &mut self,
        script: &str,
        limits: &ResourceLimits,
    ) -> Result<ExecutionResult, RuntimeError> {
        // Clear previous output
        let _ = self.store.data().stdout();
        let _ = self.store.data().stderr();

        // Set up epoch-based timeout
        self.store.set_epoch_deadline(limits.max_cpu_ms);

        // Increment epoch in background for timeout
        let engine = self.engine.clone();
        let timeout_ms = limits.max_cpu_ms;
        let epoch_handle = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(timeout_ms)).await;
            engine.increment_epoch();
        });

        // Call execute on the shell resource
        let shell_interface = self.bindings.conch_shell_shell();
        let result = shell_interface
            .instance()
            .call_execute(&mut self.store, self.shell_resource, script)
            .await
            .map_err(|e: wasmtime::Error| {
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

        // Get captured output
        let stdout = self.store.data().stdout();
        let stderr = self.store.data().stderr();

        Ok(ExecutionResult {
            exit_code,
            stdout,
            stderr,
            truncated: false,
            stats: crate::runtime::ExecutionStats::default(),
        })
    }

    /// Get a shell variable's value.
    pub async fn get_var(&mut self, name: &str) -> Result<Option<String>, RuntimeError> {
        let shell_interface = self.bindings.conch_shell_shell();
        shell_interface
            .instance()
            .call_get_var(&mut self.store, self.shell_resource, name)
            .await
            .map_err(|e| RuntimeError::Wasm(format!("get_var failed: {}", e)))
    }

    /// Set a shell variable.
    pub async fn set_var(&mut self, name: &str, value: &str) -> Result<(), RuntimeError> {
        let shell_interface = self.bindings.conch_shell_shell();
        shell_interface
            .instance()
            .call_set_var(&mut self.store, self.shell_resource, name, value)
            .await
            .map_err(|e| RuntimeError::Wasm(format!("set_var failed: {}", e)))
    }

    /// Get the exit code from the last executed command.
    pub async fn last_exit_code(&mut self) -> Result<i32, RuntimeError> {
        let shell_interface = self.bindings.conch_shell_shell();
        shell_interface
            .instance()
            .call_last_exit_code(&mut self.store, self.shell_resource)
            .await
            .map_err(|e| RuntimeError::Wasm(format!("last_exit_code failed: {}", e)))
    }
}

/// Simple memory limiter for WASM execution.
pub struct StoreLimiter {
    max_memory: u64,
}

impl StoreLimiter {
    /// Create a new store limiter with the given memory limit.
    pub fn new(max_memory: u64) -> Self {
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
    use eryx_vfs::{DirPerms, FilePerms, InMemoryStorage};

    async fn create_test_instance() -> ShellInstance<InMemoryStorage> {
        let executor = ComponentShellExecutor::embedded().expect("Failed to create executor");
        let limits = ResourceLimits::default();

        let storage = Arc::new(InMemoryStorage::new());
        let mut hybrid_ctx = HybridVfsCtx::new(storage);
        hybrid_ctx.add_vfs_preopen("/scratch", DirPerms::all(), FilePerms::all());

        executor
            .create_instance(&limits, hybrid_ctx, None)
            .await
            .expect("Failed to create instance")
    }

    #[tokio::test]
    async fn test_shell_instance_echo() {
        let mut instance = create_test_instance().await;
        let limits = ResourceLimits::default();

        let result = instance
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
    async fn test_shell_instance_variable_persistence() {
        let mut instance = create_test_instance().await;
        let limits = ResourceLimits::default();

        // Set a variable
        let result = instance
            .execute("x=42", &limits)
            .await
            .expect("execute failed");
        assert_eq!(result.exit_code, 0);

        // Variable should persist to next call
        let result = instance
            .execute("echo $x", &limits)
            .await
            .expect("execute failed");
        assert_eq!(result.exit_code, 0);
        let stdout = String::from_utf8_lossy(&result.stdout);
        assert!(
            stdout.contains("42"),
            "stdout should contain 42: {:?}",
            stdout
        );
    }

    #[tokio::test]
    async fn test_shell_instance_function_persistence() {
        let mut instance = create_test_instance().await;
        let limits = ResourceLimits::default();

        // Define a function
        let result = instance
            .execute("greet() { echo \"Hello, $1!\"; }", &limits)
            .await
            .expect("execute failed");
        assert_eq!(result.exit_code, 0);

        // Function should persist to next call
        let result = instance
            .execute("greet World", &limits)
            .await
            .expect("execute failed");
        assert_eq!(result.exit_code, 0);
        let stdout = String::from_utf8_lossy(&result.stdout);
        assert!(
            stdout.contains("Hello, World!"),
            "stdout should contain greeting: {:?}",
            stdout
        );
    }

    #[tokio::test]
    async fn test_shell_instance_get_set_var() {
        let mut instance = create_test_instance().await;

        // Set via API
        instance
            .set_var("myvar", "myvalue")
            .await
            .expect("set_var failed");

        // Get via API
        let value = instance.get_var("myvar").await.expect("get_var failed");
        assert_eq!(value, Some("myvalue".to_string()));

        // Should also be visible in shell
        let limits = ResourceLimits::default();
        let result = instance
            .execute("echo $myvar", &limits)
            .await
            .expect("execute failed");
        let stdout = String::from_utf8_lossy(&result.stdout);
        assert!(stdout.contains("myvalue"), "stdout: {:?}", stdout);
    }

    #[tokio::test]
    async fn test_shell_instance_isolation() {
        // Create two separate instances
        let mut instance1 = create_test_instance().await;
        let mut instance2 = create_test_instance().await;
        let limits = ResourceLimits::default();

        // Set variable in instance1
        instance1
            .execute("x=from_instance1", &limits)
            .await
            .expect("execute failed");

        // Set different variable in instance2
        instance2
            .execute("x=from_instance2", &limits)
            .await
            .expect("execute failed");

        // Each instance should see its own value
        let result1 = instance1
            .execute("echo $x", &limits)
            .await
            .expect("execute failed");
        let stdout1 = String::from_utf8_lossy(&result1.stdout);
        assert!(
            stdout1.contains("from_instance1"),
            "instance1 stdout: {:?}",
            stdout1
        );

        let result2 = instance2
            .execute("echo $x", &limits)
            .await
            .expect("execute failed");
        let stdout2 = String::from_utf8_lossy(&result2.stdout);
        assert!(
            stdout2.contains("from_instance2"),
            "instance2 stdout: {:?}",
            stdout2
        );
    }

    #[tokio::test]
    async fn test_shell_instance_last_exit_code() {
        let mut instance = create_test_instance().await;
        let limits = ResourceLimits::default();

        // Run a successful command
        instance
            .execute("true", &limits)
            .await
            .expect("execute failed");
        let code = instance
            .last_exit_code()
            .await
            .expect("last_exit_code failed");
        assert_eq!(code, 0);

        // Run a failing command
        instance
            .execute("false", &limits)
            .await
            .expect("execute failed");
        let code = instance
            .last_exit_code()
            .await
            .expect("last_exit_code failed");
        assert_eq!(code, 1);
    }
}
