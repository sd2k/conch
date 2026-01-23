//! Conch Shell - Brush-based WASM Component
//!
//! This crate wraps brush-core to provide a full bash-compatible shell
//! that runs inside a WASM sandbox. It exports a simple execute interface
//! via the WebAssembly Component Model (WIT).

#![allow(clippy::expect_used)] // WASM is single-threaded, mutex poisoning is fatal
#![allow(missing_docs)] // WIT-generated code doesn't have docs

use brush_core::{ExecutionParameters, Shell, SourceInfo};

mod builtins;

// Generate WIT bindings for the shell world.
// This creates the `Guest` trait we need to implement.
wit_bindgen::generate!({
    path: "wit/shell.wit",
    world: "shell",
});

/// Our implementation of the shell component.
struct ShellComponent;

impl Guest for ShellComponent {
    /// Execute a shell script and return the exit code.
    ///
    /// Returns Ok(exit_code) on success, or Err(message) if the shell
    /// itself failed to initialize. Note that script errors (like
    /// "command not found") still return Ok with a non-zero exit code.
    ///
    /// stdout/stderr are written to WASI pipes, not returned here.
    fn execute(script: String) -> Result<i32, String> {
        // Build tokio runtime (single-threaded for WASM)
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| format!("failed to create runtime: {}", e))?;

        // Execute the script
        rt.block_on(execute_script_async(&script))
    }
}

/// Async implementation of script execution.
async fn execute_script_async(script: &str) -> Result<i32, String> {
    // Get default builtins and add our custom ones
    let mut shell_builtins = brush_builtins::default_builtins(brush_builtins::BuiltinSet::BashMode);
    builtins::register_builtins(&mut shell_builtins);

    let mut shell = Shell::builder()
        .builtins(shell_builtins)
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

// Export the component.
export!(ShellComponent);

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

    fn execute_test(script: &str) -> Result<i32, String> {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("failed to create runtime");
        rt.block_on(execute_quiet(script))
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
