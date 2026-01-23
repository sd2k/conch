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
    /// Execute a shell script and return the result.
    fn execute(script: String) -> ExecuteResult {
        // Build tokio runtime (single-threaded for WASM)
        let rt = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(e) => {
                return ExecuteResult {
                    exit_code: 1,
                    stdout: Vec::new(),
                    stderr: format!("failed to create runtime: {}\n", e).into_bytes(),
                };
            }
        };

        // Execute the script
        rt.block_on(execute_script_async(&script))
    }
}

/// Async implementation of script execution.
async fn execute_script_async(script: &str) -> ExecuteResult {
    // Get default builtins and add our custom ones
    let mut shell_builtins = brush_builtins::default_builtins(brush_builtins::BuiltinSet::BashMode);
    builtins::register_builtins(&mut shell_builtins);

    let shell_result = Shell::builder().builtins(shell_builtins).build().await;

    let mut shell = match shell_result {
        Ok(s) => s,
        Err(e) => {
            return ExecuteResult {
                exit_code: 1,
                stdout: Vec::new(),
                stderr: format!("failed to create shell: {}\n", e).into_bytes(),
            };
        }
    };

    let source_info = SourceInfo::default();
    let exec_params = ExecutionParameters::default();

    match shell.run_string(script, &source_info, &exec_params).await {
        Ok(result) => {
            let exit_code = i32::from(u8::from(result.exit_code));
            ExecuteResult {
                exit_code,
                stdout: Vec::new(), // TODO: capture stdout via WASI
                stderr: Vec::new(), // TODO: capture stderr via WASI
            }
        }
        Err(e) => ExecuteResult {
            exit_code: 1,
            stdout: Vec::new(),
            stderr: format!("execution error: {}\n", e).into_bytes(),
        },
    }
}

// Export the component.
export!(ShellComponent);

// ============================================================================
// Tests (run natively, not in WASM)
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn execute_test(script: &str) -> ExecuteResult {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("failed to create runtime");
        rt.block_on(execute_script_async(script))
    }

    #[test]
    fn test_echo() {
        let result = execute_test("echo hello");
        assert_eq!(result.exit_code, 0);
    }

    #[test]
    fn test_true() {
        let result = execute_test("true");
        assert_eq!(result.exit_code, 0);
    }

    #[test]
    fn test_false() {
        let result = execute_test("false");
        assert_eq!(result.exit_code, 1);
    }

    #[test]
    fn test_variable() {
        let result = execute_test("x=hello; echo $x");
        assert_eq!(result.exit_code, 0);
    }

    #[test]
    fn test_arithmetic() {
        let result = execute_test("echo $((2 + 2))");
        assert_eq!(result.exit_code, 0);
    }

    #[test]
    fn test_conditional() {
        let result = execute_test("if true; then echo yes; fi");
        assert_eq!(result.exit_code, 0);
    }

    #[test]
    fn test_loop() {
        let result = execute_test("for i in 1 2 3; do echo $i; done");
        assert_eq!(result.exit_code, 0);
    }
}

#[cfg(test)]
mod pipe_tests {
    use super::*;

    fn execute_test(script: &str) -> ExecuteResult {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("failed to create runtime");
        rt.block_on(execute_script_async(script))
    }

    #[test]
    fn test_simple_pipe() {
        let result = execute_test("echo hello | cat");
        assert_eq!(
            result.exit_code,
            0,
            "stderr: {}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
}
