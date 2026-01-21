//! WebAssembly runtime for executing shell scripts using core modules (wasip1).
//!
//! This module handles loading and running the conch-shell WASM module using wasmtime.
//! Unlike the component model version, this uses core WASM modules with WASI.

use std::path::Path;

use wasmtime::{Config, Engine, Linker, Module, Store, TypedFunc};
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
struct ShellState {
    wasi: WasiP1Ctx,
    stdout_pipe: MemoryOutputPipe,
    stderr_pipe: MemoryOutputPipe,
}

/// Executor for running shell scripts in WASM (core module version).
///
/// This loads a pre-compiled WASI module and executes scripts within it.
#[derive(Clone)]
pub struct CoreShellExecutor {
    engine: Engine,
    module: Module,
}

impl std::fmt::Debug for CoreShellExecutor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CoreShellExecutor").finish_non_exhaustive()
    }
}

impl CoreShellExecutor {
    /// Create a new executor by loading a module from a file.
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, RuntimeError> {
        let config = Config::new();
        let engine = Engine::new(&config).map_err(|e| RuntimeError::Wasm(e.to_string()))?;

        let module = Module::from_file(&engine, path.as_ref())
            .map_err(|e| RuntimeError::Wasm(e.to_string()))?;

        Ok(Self { engine, module })
    }

    /// Create a new executor by loading a module from bytes.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, RuntimeError> {
        let config = Config::new();
        let engine = Engine::new(&config).map_err(|e| RuntimeError::Wasm(e.to_string()))?;

        let module = Module::new(&engine, bytes).map_err(|e| RuntimeError::Wasm(e.to_string()))?;

        Ok(Self { engine, module })
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

    /// Get a reference to the underlying engine.
    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    /// Execute a shell script.
    pub fn execute(
        &self,
        script: &str,
        _limits: &ResourceLimits,
    ) -> Result<ExecutionResult, RuntimeError> {
        let start = std::time::Instant::now();

        // Create pipes for capturing stdout/stderr
        let stdout_pipe = MemoryOutputPipe::new(1024 * 1024); // 1MB capacity
        let stderr_pipe = MemoryOutputPipe::new(1024 * 1024);

        // Create WASI context for preview1 with captured stdio
        let wasi = WasiCtxBuilder::new()
            .stdout(stdout_pipe.clone())
            .stderr(stderr_pipe.clone())
            .build_p1();

        let state = ShellState {
            wasi,
            stdout_pipe,
            stderr_pipe,
        };
        let mut store = Store::new(&self.engine, state);

        // Create linker with WASI preview1
        let mut linker = Linker::new(&self.engine);
        wasmtime_wasi::p1::add_to_linker_sync(&mut linker, |s: &mut ShellState| &mut s.wasi)
            .map_err(|e| RuntimeError::Wasm(e.to_string()))?;

        // Instantiate the module
        let instance = linker
            .instantiate(&mut store, &self.module)
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

        // Call execute - brush will write to WASI stdout/stderr which we capture via pipes
        let exit_code = execute_fn
            .call(&mut store, (script_ptr, script_bytes.len() as i32))
            .map_err(|e| RuntimeError::Wasm(format!("execute failed: {}", e)))?;

        // Get captured stdout/stderr from the pipes
        let stdout = store.data().stdout_pipe.contents().to_vec();
        let stderr = store.data().stderr_pipe.contents().to_vec();

        Ok(ExecutionResult {
            exit_code,
            stdout,
            stderr,
            truncated: false,
            stats: ExecutionStats {
                cpu_time_ms: 0,
                peak_memory_bytes: 0,
                wall_time_ms: start.elapsed().as_millis() as u64,
            },
        })
    }
}

#[cfg(test)]
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

    #[test]
    fn test_core_execute_echo() {
        let Some(executor) = get_executor() else {
            eprintln!(
                "Skipping test: module not built. Run: cargo build -p conch-shell --target wasm32-wasip1 --release"
            );
            return;
        };

        let result = executor
            .execute("echo hello", &ResourceLimits::default())
            .expect("execute failed");

        assert_eq!(result.exit_code, 0);
    }

    #[test]
    fn test_core_execute_variable() {
        let Some(executor) = get_executor() else {
            return;
        };

        let result = executor
            .execute("x=42; echo $x", &ResourceLimits::default())
            .expect("execute failed");

        assert_eq!(result.exit_code, 0);
    }

    #[test]
    fn test_core_execute_false() {
        let Some(executor) = get_executor() else {
            return;
        };

        let result = executor
            .execute("false", &ResourceLimits::default())
            .expect("execute failed");

        assert_eq!(result.exit_code, 1);
    }
}
