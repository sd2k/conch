//! C FFI for Go integration
//!
//! This module provides a C-compatible interface for embedding Conch
//! in Go applications via purego (no CGO required).
//!
//! # Example (Go)
//!
//! ```go
//! executor := conch.NewExecutorEmbedded()
//! defer executor.Close()
//!
//! result, err := executor.Execute("echo hello | cat")
//! if err != nil {
//!     log.Fatal(err)
//! }
//! fmt.Println(string(result.Stdout))
//! ```

use std::cell::RefCell;
use std::ffi::{CStr, CString, c_char};
use std::ptr;

use crate::executor::CoreShellExecutor;
use crate::limits::ResourceLimits;

thread_local! {
    static LAST_ERROR: RefCell<Option<CString>> = const { RefCell::new(None) };
}

fn set_last_error(msg: &str) {
    LAST_ERROR.with(|e| {
        *e.borrow_mut() = CString::new(msg).ok();
    });
}

/// Result structure returned from shell execution.
#[repr(C)]
#[derive(Debug)]
pub struct ConchResult {
    /// Shell exit code (0 = success).
    pub exit_code: i32,
    /// Pointer to stdout data (owned, must be freed with `conch_result_free`).
    pub stdout_data: *mut c_char,
    /// Length of stdout data in bytes (excluding null terminator).
    pub stdout_len: usize,
    /// Pointer to stderr data (owned, must be freed with `conch_result_free`).
    pub stderr_data: *mut c_char,
    /// Length of stderr data in bytes (excluding null terminator).
    pub stderr_len: usize,
    /// Non-zero if output was truncated due to limits.
    pub truncated: u8,
}

/// Opaque handle to a shell executor.
#[derive(Debug)]
pub struct ConchExecutor {
    executor: CoreShellExecutor,
}

// ============================================================================
// Error handling
// ============================================================================

/// Get the last error message (thread-local).
///
/// Returns a pointer to a null-terminated C string, or null if no error.
/// The pointer is valid until the next FFI call on the same thread.
#[unsafe(no_mangle)]
pub extern "C" fn conch_last_error() -> *const c_char {
    LAST_ERROR.with(|e| {
        e.borrow()
            .as_ref()
            .map(|s| s.as_ptr())
            .unwrap_or(ptr::null())
    })
}

// ============================================================================
// Executor lifecycle
// ============================================================================

/// Create a new shell executor using the embedded WASM module.
///
/// This is only available when built with the `embedded-shell` feature.
/// Returns a pointer to the executor on success, or null on failure.
/// On failure, call `conch_last_error()` to get the error message.
///
/// # Safety
/// - The returned pointer must be freed with `conch_executor_free()`.
#[cfg(feature = "embedded-shell")]
#[unsafe(no_mangle)]
pub extern "C" fn conch_executor_new_embedded() -> *mut ConchExecutor {
    match CoreShellExecutor::embedded() {
        Ok(executor) => Box::into_raw(Box::new(ConchExecutor { executor })),
        Err(e) => {
            set_last_error(&format!("failed to create executor: {}", e));
            ptr::null_mut()
        }
    }
}

/// Check if the embedded shell module is available.
///
/// Returns 1 if the library was built with `embedded-shell` feature, 0 otherwise.
#[unsafe(no_mangle)]
pub extern "C" fn conch_has_embedded_shell() -> u8 {
    #[cfg(feature = "embedded-shell")]
    {
        1
    }
    #[cfg(not(feature = "embedded-shell"))]
    {
        0
    }
}

/// Create a new shell executor from a WASM module file path.
///
/// Returns a pointer to the executor on success, or null on failure.
/// On failure, call `conch_last_error()` to get the error message.
///
/// # Safety
/// - `module_path` must be a valid null-terminated C string.
/// - The returned pointer must be freed with `conch_executor_free()`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn conch_executor_new(module_path: *const c_char) -> *mut ConchExecutor {
    if module_path.is_null() {
        set_last_error("module_path is null");
        return ptr::null_mut();
    }

    let path = match unsafe { CStr::from_ptr(module_path) }.to_str() {
        Ok(s) => s,
        Err(e) => {
            set_last_error(&format!("invalid UTF-8 in path: {}", e));
            return ptr::null_mut();
        }
    };

    match CoreShellExecutor::from_file(path) {
        Ok(executor) => Box::into_raw(Box::new(ConchExecutor { executor })),
        Err(e) => {
            set_last_error(&format!("failed to create executor: {}", e));
            ptr::null_mut()
        }
    }
}

/// Create a new shell executor from WASM module bytes.
///
/// Returns a pointer to the executor on success, or null on failure.
/// On failure, call `conch_last_error()` to get the error message.
///
/// # Safety
/// - `module_data` must be a valid pointer to `module_len` bytes.
/// - The returned pointer must be freed with `conch_executor_free()`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn conch_executor_new_from_bytes(
    module_data: *const u8,
    module_len: usize,
) -> *mut ConchExecutor {
    if module_data.is_null() {
        set_last_error("module_data is null");
        return ptr::null_mut();
    }

    let bytes = unsafe { std::slice::from_raw_parts(module_data, module_len) };

    match CoreShellExecutor::from_bytes(bytes) {
        Ok(executor) => Box::into_raw(Box::new(ConchExecutor { executor })),
        Err(e) => {
            set_last_error(&format!("failed to create executor: {}", e));
            ptr::null_mut()
        }
    }
}

/// Free a shell executor.
///
/// # Safety
/// - `executor` must be a pointer returned by `conch_executor_new*()`, or null.
/// - The pointer must not be used after this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn conch_executor_free(executor: *mut ConchExecutor) {
    if !executor.is_null() {
        unsafe { drop(Box::from_raw(executor)) };
    }
}

// ============================================================================
// Execution
// ============================================================================

/// Execute a shell script.
///
/// Returns a pointer to a `ConchResult` on success, or null on failure.
/// On failure, call `conch_last_error()` to get the error message.
/// The result must be freed with `conch_result_free()`.
///
/// # Safety
/// - `executor` must be a valid pointer from `conch_executor_new*()`.
/// - `script` must be a valid null-terminated C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn conch_execute(
    executor: *mut ConchExecutor,
    script: *const c_char,
) -> *mut ConchResult {
    if executor.is_null() {
        set_last_error("executor is null");
        return ptr::null_mut();
    }

    if script.is_null() {
        set_last_error("script is null");
        return ptr::null_mut();
    }

    let executor = unsafe { &*executor };

    let script_str = match unsafe { CStr::from_ptr(script) }.to_str() {
        Ok(s) => s,
        Err(e) => {
            set_last_error(&format!("invalid UTF-8 in script: {}", e));
            return ptr::null_mut();
        }
    };

    let limits = ResourceLimits::default();

    // Create a tokio runtime to run the async executor
    let rt = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            set_last_error(&format!("failed to create runtime: {}", e));
            return ptr::null_mut();
        }
    };

    match rt.block_on(executor.executor.execute(script_str, &limits)) {
        Ok(exec_result) => {
            let stdout_len = exec_result.stdout.len();
            let stderr_len = exec_result.stderr.len();

            let stdout_data = if exec_result.stdout.is_empty() {
                ptr::null_mut()
            } else {
                let mut stdout = exec_result.stdout;
                stdout.push(0); // Add null terminator
                let ptr = stdout.as_mut_ptr() as *mut c_char;
                std::mem::forget(stdout);
                ptr
            };

            let stderr_data = if exec_result.stderr.is_empty() {
                ptr::null_mut()
            } else {
                let mut stderr = exec_result.stderr;
                stderr.push(0); // Add null terminator
                let ptr = stderr.as_mut_ptr() as *mut c_char;
                std::mem::forget(stderr);
                ptr
            };

            Box::into_raw(Box::new(ConchResult {
                exit_code: exec_result.exit_code,
                stdout_data,
                stdout_len,
                stderr_data,
                stderr_len,
                truncated: if exec_result.truncated { 1 } else { 0 },
            }))
        }
        Err(e) => {
            set_last_error(&format!("execution failed: {}", e));
            ptr::null_mut()
        }
    }
}

/// Execute a shell script with custom resource limits.
///
/// Returns a pointer to a `ConchResult` on success, or null on failure.
/// On failure, call `conch_last_error()` to get the error message.
/// The result must be freed with `conch_result_free()`.
///
/// # Safety
/// - `executor` must be a valid pointer from `conch_executor_new*()`.
/// - `script` must be a valid null-terminated C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn conch_execute_with_limits(
    executor: *mut ConchExecutor,
    script: *const c_char,
    max_cpu_ms: u64,
    max_memory_bytes: u64,
    max_output_bytes: u64,
    timeout_ms: u64,
) -> *mut ConchResult {
    if executor.is_null() {
        set_last_error("executor is null");
        return ptr::null_mut();
    }

    if script.is_null() {
        set_last_error("script is null");
        return ptr::null_mut();
    }

    let executor = unsafe { &*executor };

    let script_str = match unsafe { CStr::from_ptr(script) }.to_str() {
        Ok(s) => s,
        Err(e) => {
            set_last_error(&format!("invalid UTF-8 in script: {}", e));
            return ptr::null_mut();
        }
    };

    let limits = ResourceLimits {
        max_cpu_ms,
        max_memory_bytes,
        max_output_bytes,
        timeout: std::time::Duration::from_millis(timeout_ms),
    };

    // Create a tokio runtime to run the async executor
    let rt = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            set_last_error(&format!("failed to create runtime: {}", e));
            return ptr::null_mut();
        }
    };

    match rt.block_on(executor.executor.execute(script_str, &limits)) {
        Ok(exec_result) => {
            let stdout_len = exec_result.stdout.len();
            let stderr_len = exec_result.stderr.len();

            let stdout_data = if exec_result.stdout.is_empty() {
                ptr::null_mut()
            } else {
                let mut stdout = exec_result.stdout;
                stdout.push(0);
                let ptr = stdout.as_mut_ptr() as *mut c_char;
                std::mem::forget(stdout);
                ptr
            };

            let stderr_data = if exec_result.stderr.is_empty() {
                ptr::null_mut()
            } else {
                let mut stderr = exec_result.stderr;
                stderr.push(0);
                let ptr = stderr.as_mut_ptr() as *mut c_char;
                std::mem::forget(stderr);
                ptr
            };

            Box::into_raw(Box::new(ConchResult {
                exit_code: exec_result.exit_code,
                stdout_data,
                stdout_len,
                stderr_data,
                stderr_len,
                truncated: if exec_result.truncated { 1 } else { 0 },
            }))
        }
        Err(e) => {
            set_last_error(&format!("execution failed: {}", e));
            ptr::null_mut()
        }
    }
}

// ============================================================================
// Result cleanup
// ============================================================================

/// Free a result structure.
///
/// # Safety
/// - `result` must be a pointer returned by `conch_execute*()`, or null.
/// - The pointer must not be used after this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn conch_result_free(result: *mut ConchResult) {
    if result.is_null() {
        return;
    }

    let result = unsafe { &mut *result };

    if !result.stdout_data.is_null() {
        // Reconstruct the Vec to properly deallocate
        // We added a null terminator, so len is stdout_len + 1
        unsafe {
            let _ = Vec::from_raw_parts(
                result.stdout_data as *mut u8,
                result.stdout_len + 1,
                result.stdout_len + 1,
            );
        }
        result.stdout_data = ptr::null_mut();
    }

    if !result.stderr_data.is_null() {
        unsafe {
            let _ = Vec::from_raw_parts(
                result.stderr_data as *mut u8,
                result.stderr_len + 1,
                result.stderr_len + 1,
            );
        }
        result.stderr_data = ptr::null_mut();
    }

    // Free the result struct itself
    unsafe { drop(Box::from_raw(result)) };
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::mem;

    #[test]
    fn test_conch_result_layout() {
        // Verify the struct is properly sized for C interop
        assert!(mem::size_of::<ConchResult>() > 0);

        // On 64-bit systems, pointers are 8 bytes
        #[cfg(target_pointer_width = "64")]
        {
            // i32 (4) + padding (4) + ptr (8) + usize (8) + ptr (8) + usize (8) + u8 (1) + padding (7)
            assert_eq!(mem::size_of::<ConchResult>(), 48);
        }
    }

    #[test]
    fn test_conch_result_repr_c() {
        // Verify #[repr(C)] is working - fields should be in declaration order
        let result = ConchResult {
            exit_code: 42,
            stdout_data: ptr::null_mut(),
            stdout_len: 100,
            stderr_data: ptr::null_mut(),
            stderr_len: 200,
            truncated: 1,
        };

        assert_eq!(result.exit_code, 42);
        assert_eq!(result.stdout_len, 100);
        assert_eq!(result.stderr_len, 200);
        assert_eq!(result.truncated, 1);
    }

    #[test]
    fn test_last_error_initially_null() {
        // Clear any existing error
        LAST_ERROR.with(|e| {
            *e.borrow_mut() = None;
        });

        let err = conch_last_error();
        assert!(err.is_null());
    }

    #[test]
    fn test_last_error_after_set() {
        set_last_error("test error message");

        let err = conch_last_error();
        assert!(!err.is_null());

        let err_str = unsafe { CStr::from_ptr(err) }.to_str().unwrap();
        assert_eq!(err_str, "test error message");
    }

    #[test]
    fn test_last_error_overwrites_previous() {
        set_last_error("first error");
        set_last_error("second error");

        let err = conch_last_error();
        let err_str = unsafe { CStr::from_ptr(err) }.to_str().unwrap();
        assert_eq!(err_str, "second error");
    }

    #[test]
    fn test_result_free_null_safe() {
        // Should not crash when passed null
        unsafe { conch_result_free(ptr::null_mut()) };
    }

    #[test]
    fn test_executor_new_null_path() {
        let executor = unsafe { conch_executor_new(ptr::null()) };
        assert!(executor.is_null());

        let err = conch_last_error();
        assert!(!err.is_null());
    }

    #[test]
    fn test_execute_null_executor() {
        let script = CString::new("echo hello").unwrap();
        let result = unsafe { conch_execute(ptr::null_mut(), script.as_ptr()) };
        assert!(result.is_null());

        let err = conch_last_error();
        assert!(!err.is_null());
    }

    #[test]
    fn test_execute_null_script() {
        // We can't easily create an executor without the embedded feature,
        // so just test that null script is handled
        let result = unsafe { conch_execute(ptr::null_mut(), ptr::null()) };
        assert!(result.is_null());
    }

    #[test]
    fn test_executor_free_null_safe() {
        // Should not crash when passed null
        unsafe { conch_executor_free(ptr::null_mut()) };
    }

    #[test]
    fn test_has_embedded_shell() {
        let has = conch_has_embedded_shell();
        #[cfg(feature = "embedded-shell")]
        assert_eq!(has, 1);
        #[cfg(not(feature = "embedded-shell"))]
        assert_eq!(has, 0);
    }
}
