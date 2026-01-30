//! Conch Shell - Brush-based WASM Component
//!
//! This crate wraps brush-core to provide a full bash-compatible shell
//! that runs inside a WASM sandbox. It exports a shell resource that
//! maintains state across executions via the WebAssembly Component Model (WIT).

#![allow(clippy::expect_used)] // WASM is single-threaded, mutex poisoning is fatal
#![allow(missing_docs)] // WIT-generated code doesn't have docs

use std::cell::RefCell;

use brush_core::env::{EnvironmentLookup, EnvironmentScope};
use brush_core::variables::ShellValueLiteral;
use brush_core::{ExecutionParameters, Shell, SourceInfo};

mod builtins;

// Generate WIT bindings for the shell-sandbox world.
wit_bindgen::generate!({
    path: "wit/shell.wit",
    world: "shell-sandbox",
});

// Re-export types for use in builtins - tools interface has the invoke_tool import
pub use conch::shell::tools::{ToolRequest, ToolResult, invoke_tool};

// Global tokio runtime for the WASM component.
//
// Why a global runtime?
// - brush-core is async and requires tokio
// - WIT interfaces are synchronous, so we need block_on()
// - Creating a runtime per-instance causes "nested runtime" panics
//   when multiple shells exist or when tool handlers are called
//
// This is safe because:
// - WASM is single-threaded (no thread safety concerns)
// - The runtime is isolated within WASM memory (not shared with host)
// - OnceCell ensures it's created once and reused for all calls
//
// Note for embedders: If you're embedding this WASM in a Rust program with
// its own tokio runtime, that's fine - the WASM's runtime is completely
// isolated in WASM linear memory and doesn't interact with the host runtime.
thread_local! {
    static RUNTIME: std::cell::OnceCell<tokio::runtime::Runtime> = const { std::cell::OnceCell::new() };
}

/// Get or create the global tokio runtime and run an async block on it.
fn block_on<F, R>(f: F) -> R
where
    F: std::future::Future<Output = R>,
{
    RUNTIME.with(|rt| {
        let runtime = rt.get_or_init(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("failed to create tokio runtime")
        });
        runtime.block_on(f)
    })
}

/// A persistent shell instance that maintains state across executions.
///
/// This struct holds the brush shell, allowing variables, functions, and
/// aliases to persist between `execute` calls. All instances share a
/// global tokio runtime to avoid nested runtime issues.
pub struct ShellInstance {
    /// The brush shell instance. RefCell for interior mutability since
    /// wit-bindgen gives us &self, not &mut self.
    shell: RefCell<Shell>,
    /// Last exit code from executed command.
    last_exit_code: RefCell<i32>,
}

impl std::fmt::Debug for ShellInstance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ShellInstance")
            .field("last_exit_code", &self.last_exit_code)
            .finish_non_exhaustive()
    }
}

impl exports::conch::shell::shell::GuestInstance for ShellInstance {
    /// Create a new shell instance with default configuration.
    fn new() -> Self {
        // Get default builtins and add our custom ones
        let mut shell_builtins =
            brush_builtins::default_builtins(brush_builtins::BuiltinSet::BashMode);
        builtins::register_builtins(&mut shell_builtins);

        // Create the shell using the global runtime
        let shell = block_on(async {
            Shell::builder()
                .builtins(shell_builtins)
                .build()
                .await
                .expect("failed to create shell")
        });

        Self {
            shell: RefCell::new(shell),
            last_exit_code: RefCell::new(0),
        }
    }

    /// Execute a shell script.
    ///
    /// Returns Ok(exit_code) on success, or Err(message) if the shell
    /// itself failed. Script errors (like "command not found") return
    /// Ok with a non-zero exit code.
    ///
    /// State (variables, functions, aliases) persists between calls.
    fn execute(&self, script: String) -> Result<i32, String> {
        let mut shell = self.shell.borrow_mut();

        let result = block_on(async {
            let source_info = SourceInfo::default();
            let exec_params = ExecutionParameters::default();

            shell
                .run_string(&script, &source_info, &exec_params)
                .await
                .map_err(|e| format!("execution error: {}", e))
        })?;

        let exit_code = i32::from(u8::from(result.exit_code));
        *self.last_exit_code.borrow_mut() = exit_code;

        Ok(exit_code)
    }

    /// Get a shell variable's value.
    fn get_var(&self, name: String) -> Option<String> {
        let shell = self.shell.borrow();
        shell
            .env()
            .get(&name)
            .map(|(_, var)| var.value().to_cow_str(&*shell).into_owned())
    }

    /// Set a shell variable.
    fn set_var(&self, name: String, value: String) {
        let mut shell = self.shell.borrow_mut();
        // Use update_or_add to set the variable in the global scope
        shell
            .env_mut()
            .update_or_add(
                &name,
                ShellValueLiteral::Scalar(value),
                |_| Ok(()),
                EnvironmentLookup::Anywhere,
                EnvironmentScope::Global,
            )
            .expect("failed to set variable");
    }

    /// Get the exit code from the last executed command.
    fn last_exit_code(&self) -> i32 {
        *self.last_exit_code.borrow()
    }
}

/// Component struct that implements the Guest trait.
struct Component;

impl exports::conch::shell::shell::Guest for Component {
    type Instance = ShellInstance;
}

// Export the component using the generated export macro.
export!(Component);

// ============================================================================
// Tests (run natively, not in WASM)
// ============================================================================

#[cfg(test)]
mod tests {
    use brush_core::{ExecutionParameters, Shell, SourceInfo, openfiles};
    use std::collections::HashMap;

    /// Execute a script with stdout/stderr redirected to /dev/null.
    /// This prevents test output from being noisy.
    pub(crate) async fn execute_quiet(script: &str) -> Result<i32, String> {
        let mut shell_builtins =
            brush_builtins::default_builtins(brush_builtins::BuiltinSet::BashMode);
        crate::builtins::register_builtins(&mut shell_builtins);

        // Redirect stdout (fd 1) and stderr (fd 2) to /dev/null
        let null_out = openfiles::null().map_err(|e| e.to_string())?;
        let null_err = openfiles::null().map_err(|e| e.to_string())?;

        let mut shell = Shell::builder()
            .builtins(shell_builtins)
            .fds(HashMap::from([(1.into(), null_out), (2.into(), null_err)]))
            .build()
            .await
            .map_err(|e| format!("failed to create shell: {}", e))?;

        let source_info = SourceInfo::default();
        let exec_params = ExecutionParameters::default();

        let result = shell
            .run_string(script, &source_info, &exec_params)
            .await
            .map_err(|e| format!("execution error: {}", e))?;

        Ok(i32::from(u8::from(result.exit_code)))
    }

    /// Execute multiple scripts on the same shell instance (for persistence tests).
    pub(crate) async fn execute_sequence_quiet(scripts: &[&str]) -> Result<Vec<i32>, String> {
        let mut shell_builtins =
            brush_builtins::default_builtins(brush_builtins::BuiltinSet::BashMode);
        crate::builtins::register_builtins(&mut shell_builtins);

        let null_out = openfiles::null().map_err(|e| e.to_string())?;
        let null_err = openfiles::null().map_err(|e| e.to_string())?;

        let mut shell = Shell::builder()
            .builtins(shell_builtins)
            .fds(HashMap::from([(1.into(), null_out), (2.into(), null_err)]))
            .build()
            .await
            .map_err(|e| format!("failed to create shell: {}", e))?;

        let source_info = SourceInfo::default();
        let exec_params = ExecutionParameters::default();

        let mut results = Vec::new();
        for script in scripts {
            let result = shell
                .run_string(*script, &source_info, &exec_params)
                .await
                .map_err(|e| format!("execution error: {}", e))?;
            results.push(i32::from(u8::from(result.exit_code)));
        }

        Ok(results)
    }

    fn execute_test(script: &str) -> Result<i32, String> {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("failed to create runtime");
        rt.block_on(execute_quiet(script))
    }

    fn execute_sequence_test(scripts: &[&str]) -> Result<Vec<i32>, String> {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("failed to create runtime");
        rt.block_on(execute_sequence_quiet(scripts))
    }

    #[test]
    fn test_echo() {
        let result = execute_test("echo hello");
        assert_eq!(result, Ok(0));
    }

    #[test]
    fn test_true() {
        let result = execute_test("true");
        assert_eq!(result, Ok(0));
    }

    #[test]
    fn test_false() {
        let result = execute_test("false");
        assert_eq!(result, Ok(1));
    }

    #[test]
    fn test_variable() {
        let result = execute_test("x=hello; echo $x");
        assert_eq!(result, Ok(0));
    }

    #[test]
    fn test_arithmetic() {
        let result = execute_test("echo $((2 + 2))");
        assert_eq!(result, Ok(0));
    }

    #[test]
    fn test_conditional() {
        let result = execute_test("if true; then echo yes; fi");
        assert_eq!(result, Ok(0));
    }

    #[test]
    fn test_loop() {
        let result = execute_test("for i in 1 2 3; do echo $i; done");
        assert_eq!(result, Ok(0));
    }

    #[test]
    fn test_variable_persistence() {
        // Test that variables persist across multiple execute calls
        let results = execute_sequence_test(&[
            "x=42",
            "echo $x", // Should see x=42
            "y=$((x + 8))",
            "echo $y", // Should see y=50
        ]);
        assert_eq!(results, Ok(vec![0, 0, 0, 0]));
    }

    #[test]
    fn test_function_persistence() {
        // Test that functions persist across multiple execute calls
        let results = execute_sequence_test(&[
            "greet() { echo \"Hello, $1!\"; }",
            "greet World", // Should work
        ]);
        assert_eq!(results, Ok(vec![0, 0]));
    }
}

#[cfg(test)]
mod pipe_tests {
    use super::tests::execute_quiet;

    fn execute_test(script: &str) -> Result<i32, String> {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("failed to create runtime");
        rt.block_on(execute_quiet(script))
    }

    #[test]
    fn test_simple_pipe() {
        let result = execute_test("echo hello | cat");
        assert_eq!(result, Ok(0), "error: {:?}", result);
    }
}
