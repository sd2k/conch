//! WebAssembly runtime for executing shell scripts.
//!
//! This module handles loading and running the conch-wasm component using wasmtime.
//!
//! # Embedded Component
//!
//! When built with the `embedded-component` feature, the WASM component is embedded
//! directly into the library binary, allowing you to create an executor without
//! providing a path to the component file:
//!
//! ```ignore
//! let executor = ShellExecutor::embedded()?;
//! let result = executor.execute("echo hello", &ResourceLimits::default())?;
//! ```
//!
//! To build with the embedded component:
//! ```bash
//! # First, build the WASM component
//! cargo build -p conch-wasm --target wasm32-unknown-unknown --release
//! wasm-tools component new target/wasm32-unknown-unknown/release/conch_wasm.wasm \
//!   -o target/conch_component.wasm
//!
//! # Then build the library with the feature enabled
//! cargo build -p conch --features embedded-component --release
//! ```

use std::path::Path;

use wasmtime::component::{Component, Linker, ResourceTable};
use wasmtime::{Config, Engine, Store};

use crate::limits::ResourceLimits;
use crate::runtime::{ExecutionResult, ExecutionStats, RuntimeError};

/// Embedded WASM component bytes (when built with `embedded-component` feature).
#[cfg(feature = "embedded-component")]
static EMBEDDED_COMPONENT: &[u8] = include_bytes!("../../../target/conch_component.wasm");

// Generate bindings from the WIT file
wasmtime::component::bindgen!({
    world: "shell",
    path: "wit/shell.wit",
});

/// State held by the WASM store during execution.
struct ShellState {
    /// Resource table for WASI (currently unused but required by wasmtime)
    #[allow(dead_code)]
    table: ResourceTable,
}

/// Executor for running shell scripts in WASM.
///
/// This loads a pre-compiled shell component and can execute scripts within it.
#[derive(Clone)]
pub struct ShellExecutor {
    engine: Engine,
    component: Component,
}

impl std::fmt::Debug for ShellExecutor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ShellExecutor").finish_non_exhaustive()
    }
}

impl ShellExecutor {
    /// Create a new executor by loading a component from a file.
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, RuntimeError> {
        let mut config = Config::new();
        config.wasm_component_model(true);

        let engine = Engine::new(&config).map_err(|e| RuntimeError::Wasm(e.to_string()))?;

        let component = Component::from_file(&engine, path.as_ref())
            .map_err(|e| RuntimeError::Wasm(e.to_string()))?;

        Ok(Self { engine, component })
    }

    /// Create a new executor by loading a component from bytes.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, RuntimeError> {
        let mut config = Config::new();
        config.wasm_component_model(true);

        let engine = Engine::new(&config).map_err(|e| RuntimeError::Wasm(e.to_string()))?;

        let component =
            Component::new(&engine, bytes).map_err(|e| RuntimeError::Wasm(e.to_string()))?;

        Ok(Self { engine, component })
    }

    /// Create a new executor using the embedded WASM component.
    ///
    /// This is only available when built with the `embedded-component` feature.
    #[cfg(feature = "embedded-component")]
    pub fn embedded() -> Result<Self, RuntimeError> {
        Self::from_bytes(EMBEDDED_COMPONENT)
    }

    /// Get the embedded component bytes (if available).
    ///
    /// This is only available when built with the `embedded-component` feature.
    #[cfg(feature = "embedded-component")]
    pub fn embedded_component_bytes() -> &'static [u8] {
        EMBEDDED_COMPONENT
    }

    /// Create a new executor with a shared engine (for efficiency when running multiple scripts).
    pub fn with_engine(engine: Engine, component: Component) -> Self {
        Self { engine, component }
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
        self.execute_with_stdin(script, &[], _limits)
    }

    /// Execute a shell script with stdin input.
    pub fn execute_with_stdin(
        &self,
        script: &str,
        stdin: &[u8],
        _limits: &ResourceLimits,
    ) -> Result<ExecutionResult, RuntimeError> {
        let start = std::time::Instant::now();

        // Create store with state
        let state = ShellState {
            table: ResourceTable::new(),
        };
        let mut store = Store::new(&self.engine, state);

        // Create linker - no imports needed for this simple component
        let linker = Linker::new(&self.engine);

        // Instantiate the component
        let instance = Shell::instantiate(&mut store, &self.component, &linker)
            .map_err(|e| RuntimeError::Wasm(e.to_string()))?;

        // Call the execute function
        let output = if stdin.is_empty() {
            instance
                .call_execute(&mut store, script)
                .map_err(|e| RuntimeError::Wasm(e.to_string()))?
        } else {
            instance
                .call_execute_with_stdin(&mut store, script, stdin)
                .map_err(|e| RuntimeError::Wasm(e.to_string()))?
        };

        Ok(ExecutionResult {
            exit_code: output.exit_code,
            stdout: output.stdout,
            stderr: output.stderr,
            truncated: false,
            stats: ExecutionStats {
                cpu_time_ms: 0,
                peak_memory_bytes: 0,
                wall_time_ms: start.elapsed().as_millis() as u64,
            },
        })
    }
}

/// Create a shared engine for multiple executors.
pub fn create_engine() -> Result<Engine, RuntimeError> {
    let mut config = Config::new();
    config.wasm_component_model(true);
    Engine::new(&config).map_err(|e| RuntimeError::Wasm(e.to_string()))
}

/// Load a component from bytes using a shared engine.
pub fn load_component(engine: &Engine, bytes: &[u8]) -> Result<Component, RuntimeError> {
    Component::new(engine, bytes).map_err(|e| RuntimeError::Wasm(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Path to the compiled component (relative to workspace root)
    const COMPONENT_PATH: &str = "target/conch_component_release.wasm";

    fn get_executor() -> Option<ShellExecutor> {
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

        let component_path = workspace_root.join(COMPONENT_PATH);

        if component_path.exists() {
            ShellExecutor::from_file(&component_path).ok()
        } else {
            None
        }
    }

    #[test]
    fn test_execute_echo() {
        let Some(executor) = get_executor() else {
            eprintln!(
                "Skipping test: component not built. Run: cargo build --package conch-wasm --target wasm32-unknown-unknown --release && wasm-tools component new target/wasm32-unknown-unknown/release/conch_wasm.wasm -o target/conch_component_release.wasm"
            );
            return;
        };

        let result = executor
            .execute("echo hello world", &ResourceLimits::default())
            .expect("execute failed");

        assert_eq!(result.exit_code, 0);
        assert_eq!(
            String::from_utf8_lossy(&result.stdout).trim(),
            "hello world"
        );
    }

    #[test]
    fn test_execute_pipeline() {
        let Some(executor) = get_executor() else {
            return;
        };

        let result = executor
            .execute(
                "echo 'hello\nworld' | grep world",
                &ResourceLimits::default(),
            )
            .expect("execute failed");

        assert_eq!(result.exit_code, 0);
        assert!(String::from_utf8_lossy(&result.stdout).contains("world"));
    }

    #[test]
    fn test_execute_with_stdin() {
        let Some(executor) = get_executor() else {
            return;
        };

        let result = executor
            .execute_with_stdin("cat", b"hello from stdin", &ResourceLimits::default())
            .expect("execute failed");

        assert_eq!(result.exit_code, 0);
        assert_eq!(String::from_utf8_lossy(&result.stdout), "hello from stdin");
    }

    #[test]
    fn test_execute_jq() {
        let Some(executor) = get_executor() else {
            return;
        };

        // Use simple field access without quotes (quotes may confuse the parser)
        let result = executor
            .execute_with_stdin(
                "jq .name",
                br#"{"name": "test", "value": 42}"#,
                &ResourceLimits::default(),
            )
            .expect("execute failed");

        assert_eq!(result.exit_code, 0);
        assert!(String::from_utf8_lossy(&result.stdout).contains("test"));
    }

    #[test]
    fn test_execute_variable() {
        let Some(executor) = get_executor() else {
            return;
        };

        let result = executor
            .execute("x=hello; echo $x", &ResourceLimits::default())
            .expect("execute failed");

        assert_eq!(result.exit_code, 0);
        assert_eq!(String::from_utf8_lossy(&result.stdout).trim(), "hello");
    }

    #[test]
    fn test_execute_exit_code() {
        let Some(executor) = get_executor() else {
            return;
        };

        let result = executor
            .execute("false", &ResourceLimits::default())
            .expect("execute failed");

        assert_ne!(result.exit_code, 0);
    }
}
