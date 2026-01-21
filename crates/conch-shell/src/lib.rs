//! Conch Shell - Brush-based WASM Component
//!
//! This crate wraps brush-core to provide a full bash-compatible shell
//! that runs inside a WASM sandbox. It exports a simple execute interface
//! via the component model.

use std::sync::{Mutex, OnceLock};

use brush_core::{ExecutionParameters, Shell, SourceInfo};

mod builtins;

/// Global buffers for capturing output (WASM is single-threaded).
static STDOUT_BUF: OnceLock<Mutex<Vec<u8>>> = OnceLock::new();
static STDERR_BUF: OnceLock<Mutex<Vec<u8>>> = OnceLock::new();
static EXIT_CODE: OnceLock<Mutex<i32>> = OnceLock::new();

fn stdout_buf() -> &'static Mutex<Vec<u8>> {
    STDOUT_BUF.get_or_init(|| Mutex::new(Vec::new()))
}

fn stderr_buf() -> &'static Mutex<Vec<u8>> {
    STDERR_BUF.get_or_init(|| Mutex::new(Vec::new()))
}

fn exit_code() -> &'static Mutex<i32> {
    EXIT_CODE.get_or_init(|| Mutex::new(0))
}

/// Execute a shell script and return the exit code.
///
/// Output is captured in global buffers and can be retrieved with
/// `get_stdout`, `get_stderr`.
#[unsafe(no_mangle)]
pub extern "C" fn execute(script_ptr: *const u8, script_len: usize) -> i32 {
    // Clear buffers
    stdout_buf().lock().unwrap().clear();
    stderr_buf().lock().unwrap().clear();
    *exit_code().lock().unwrap() = 0;

    // Parse script string
    let script = if script_ptr.is_null() || script_len == 0 {
        return 0;
    } else {
        unsafe {
            let slice = std::slice::from_raw_parts(script_ptr, script_len);
            match std::str::from_utf8(slice) {
                Ok(s) => s.to_string(),
                Err(_) => {
                    stderr_buf()
                        .lock()
                        .unwrap()
                        .extend_from_slice(b"invalid UTF-8 in script\n");
                    *exit_code().lock().unwrap() = 1;
                    return 1;
                }
            }
        }
    };

    // Build tokio runtime (single-threaded for WASM)
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            stderr_buf()
                .lock()
                .unwrap()
                .extend_from_slice(format!("failed to create runtime: {}\n", e).as_bytes());
            *exit_code().lock().unwrap() = 1;
            return 1;
        }
    };

    // Execute the script
    let result = rt.block_on(async {
        // Get default builtins and add our custom ones
        let mut shell_builtins =
            brush_builtins::default_builtins(brush_builtins::BuiltinSet::BashMode);
        builtins::register_builtins(&mut shell_builtins);

        let shell_result = Shell::builder().builtins(shell_builtins).build().await;

        let mut shell = match shell_result {
            Ok(s) => s,
            Err(e) => {
                return Err(format!("failed to create shell: {}", e));
            }
        };

        let source_info = SourceInfo::default();
        let exec_params = ExecutionParameters::default();

        match shell.run_string(&script, &source_info, &exec_params).await {
            Ok(result) => Ok(result.exit_code),
            Err(e) => Err(format!("execution error: {}", e)),
        }
    });

    match result {
        Ok(code) => {
            let code_i32 = i32::from(u8::from(code));
            *exit_code().lock().unwrap() = code_i32;
            code_i32
        }
        Err(e) => {
            stderr_buf().lock().unwrap().extend_from_slice(e.as_bytes());
            stderr_buf().lock().unwrap().push(b'\n');
            *exit_code().lock().unwrap() = 1;
            1
        }
    }
}

/// Get the length of captured stdout.
#[unsafe(no_mangle)]
pub extern "C" fn get_stdout_len() -> usize {
    stdout_buf().lock().unwrap().len()
}

/// Copy captured stdout to the provided buffer.
#[unsafe(no_mangle)]
pub extern "C" fn get_stdout(buf_ptr: *mut u8, buf_len: usize) -> usize {
    let stdout = stdout_buf().lock().unwrap();
    let copy_len = std::cmp::min(stdout.len(), buf_len);
    if !buf_ptr.is_null() && copy_len > 0 {
        unsafe {
            std::ptr::copy_nonoverlapping(stdout.as_ptr(), buf_ptr, copy_len);
        }
    }
    copy_len
}

/// Get the length of captured stderr.
#[unsafe(no_mangle)]
pub extern "C" fn get_stderr_len() -> usize {
    stderr_buf().lock().unwrap().len()
}

/// Copy captured stderr to the provided buffer.
#[unsafe(no_mangle)]
pub extern "C" fn get_stderr(buf_ptr: *mut u8, buf_len: usize) -> usize {
    let stderr = stderr_buf().lock().unwrap();
    let copy_len = std::cmp::min(stderr.len(), buf_len);
    if !buf_ptr.is_null() && copy_len > 0 {
        unsafe {
            std::ptr::copy_nonoverlapping(stderr.as_ptr(), buf_ptr, copy_len);
        }
    }
    copy_len
}

/// Get the exit code from the last execution.
#[unsafe(no_mangle)]
pub extern "C" fn get_exit_code() -> i32 {
    *exit_code().lock().unwrap()
}

// ============================================================================
// Native API (for testing)
// ============================================================================

/// Result of executing a shell script.
#[derive(Debug, Clone)]
pub struct ExecuteResult {
    /// Shell exit code (0 = success).
    pub exit_code: i32,
    /// Captured stdout.
    pub stdout: Vec<u8>,
    /// Captured stderr.
    pub stderr: Vec<u8>,
}

/// Execute a shell script and return the result (native API for testing).
pub fn execute_script(script: &str) -> ExecuteResult {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("failed to create runtime");

    rt.block_on(async {
        let stdout = Vec::new();
        let mut stderr = Vec::new();

        // Get default builtins and add our custom ones
        let mut shell_builtins =
            brush_builtins::default_builtins(brush_builtins::BuiltinSet::BashMode);
        builtins::register_builtins(&mut shell_builtins);

        let shell_result = Shell::builder().builtins(shell_builtins).build().await;

        let mut shell = match shell_result {
            Ok(s) => s,
            Err(e) => {
                stderr.extend_from_slice(format!("failed to create shell: {}\n", e).as_bytes());
                return ExecuteResult {
                    exit_code: 1,
                    stdout,
                    stderr,
                };
            }
        };

        let source_info = SourceInfo::default();
        let exec_params = ExecutionParameters::default();

        match shell.run_string(script, &source_info, &exec_params).await {
            Ok(result) => ExecuteResult {
                exit_code: i32::from(u8::from(result.exit_code)),
                stdout,
                stderr,
            },
            Err(e) => {
                stderr.extend_from_slice(format!("execution error: {}\n", e).as_bytes());
                ExecuteResult {
                    exit_code: 1,
                    stdout,
                    stderr,
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_echo() {
        let result = execute_script("echo hello");
        assert_eq!(result.exit_code, 0);
    }

    #[test]
    fn test_true() {
        let result = execute_script("true");
        assert_eq!(result.exit_code, 0);
    }

    #[test]
    fn test_false() {
        let result = execute_script("false");
        assert_eq!(result.exit_code, 1);
    }

    #[test]
    fn test_variable() {
        let result = execute_script("x=hello; echo $x");
        assert_eq!(result.exit_code, 0);
    }

    #[test]
    fn test_arithmetic() {
        let result = execute_script("echo $((2 + 2))");
        assert_eq!(result.exit_code, 0);
    }

    #[test]
    fn test_conditional() {
        let result = execute_script("if true; then echo yes; fi");
        assert_eq!(result.exit_code, 0);
    }

    #[test]
    fn test_loop() {
        let result = execute_script("for i in 1 2 3; do echo $i; done");
        assert_eq!(result.exit_code, 0);
    }
}

#[cfg(test)]
mod pipe_tests {
    use super::*;

    #[test]
    fn test_simple_pipe() {
        eprintln!("Starting pipe test...");
        let result = execute_script("echo hello | cat");
        eprintln!("Result: exit_code={}, stdout={:?}, stderr={:?}", 
            result.exit_code,
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr));
        assert_eq!(result.exit_code, 0, "stderr: {}", String::from_utf8_lossy(&result.stderr));
    }
}
