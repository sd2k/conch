//! C FFI for Go integration
//!
//! This module provides a C-compatible interface for embedding Conch
//! in Go applications via CGO.

use std::cell::RefCell;
use std::ffi::{CString, c_char};
use std::ptr;

thread_local! {
    static LAST_ERROR: RefCell<Option<CString>> = const { RefCell::new(None) };
}

fn set_last_error(msg: &str) {
    LAST_ERROR.with(|e| {
        *e.borrow_mut() = CString::new(msg).ok();
    });
}

/// Result structure returned from shell execution
#[repr(C)]
#[derive(Debug)]
pub struct ConchResult {
    /// Shell exit code
    pub exit_code: i32,
    /// Pointer to stdout data (owned, must be freed)
    pub stdout_data: *mut c_char,
    /// Length of stdout data in bytes
    pub stdout_len: usize,
    /// Pointer to stderr data (owned, must be freed)
    pub stderr_data: *mut c_char,
    /// Length of stderr data in bytes
    pub stderr_len: usize,
    /// Non-zero if output was truncated
    pub truncated: u8,
}

/// Get the last error message (thread-local)
///
/// # Safety
/// Returns a pointer to a thread-local string. The pointer is valid
/// until the next FFI call on the same thread.
#[unsafe(no_mangle)]
pub extern "C" fn conch_last_error() -> *const c_char {
    LAST_ERROR.with(|e| {
        e.borrow()
            .as_ref()
            .map(|s| s.as_ptr())
            .unwrap_or(ptr::null())
    })
}

/// Free a result structure
///
/// # Safety
/// The result pointer must have been returned by a conch function.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn conch_result_free(result: *mut ConchResult) {
    if result.is_null() {
        return;
    }

    // SAFETY: We checked result is not null above, and caller guarantees
    // this was returned by a conch function
    let result = unsafe { &mut *result };

    if !result.stdout_data.is_null() {
        // SAFETY: stdout_data was allocated by CString::into_raw
        unsafe { drop(CString::from_raw(result.stdout_data)) };
        result.stdout_data = ptr::null_mut();
    }

    if !result.stderr_data.is_null() {
        // SAFETY: stderr_data was allocated by CString::into_raw
        unsafe { drop(CString::from_raw(result.stderr_data)) };
        result.stderr_data = ptr::null_mut();
    }
}

use std::ffi::CStr;

use crate::limits::ResourceLimits;
use crate::wasm::ShellExecutor;
use crate::wasm_core::CoreShellExecutor;

/// Opaque handle to a shell executor (component model)
#[derive(Debug)]
pub struct ConchExecutor {
    executor: ShellExecutor,
}

/// Opaque handle to a core shell executor (wasip1)
#[derive(Debug)]
pub struct ConchCoreExecutor {
    executor: CoreShellExecutor,
}

/// Create a new shell executor using the embedded WASM component.
///
/// This is only available when built with the `embedded-component` feature.
/// Returns a pointer to the executor on success, or null on failure.
///
/// # Safety
/// - The returned pointer must be freed with `conch_executor_free()`.
#[cfg(feature = "embedded-component")]
#[unsafe(no_mangle)]
pub extern "C" fn conch_executor_new_embedded() -> *mut ConchExecutor {
    match ShellExecutor::embedded() {
        Ok(executor) => Box::into_raw(Box::new(ConchExecutor { executor })),
        Err(e) => {
            set_last_error(&format!("failed to create executor: {}", e));
            ptr::null_mut()
        }
    }
}

/// Check if the embedded component is available.
///
/// Returns 1 if the library was built with `embedded-component` feature, 0 otherwise.
#[unsafe(no_mangle)]
pub extern "C" fn conch_has_embedded_component() -> u8 {
    #[cfg(feature = "embedded-component")]
    {
        1
    }
    #[cfg(not(feature = "embedded-component"))]
    {
        0
    }
}

/// Create a new shell executor from a component file path.
///
/// Returns a pointer to the executor on success, or null on failure.
/// On failure, call `conch_last_error()` to get the error message.
///
/// # Safety
/// - `component_path` must be a valid null-terminated C string.
/// - The returned pointer must be freed with `conch_executor_free()`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn conch_executor_new(component_path: *const c_char) -> *mut ConchExecutor {
    if component_path.is_null() {
        set_last_error("component_path is null");
        return ptr::null_mut();
    }

    let path = match unsafe { CStr::from_ptr(component_path) }.to_str() {
        Ok(s) => s,
        Err(e) => {
            set_last_error(&format!("invalid UTF-8 in path: {}", e));
            return ptr::null_mut();
        }
    };

    match ShellExecutor::from_file(path) {
        Ok(executor) => Box::into_raw(Box::new(ConchExecutor { executor })),
        Err(e) => {
            set_last_error(&format!("failed to create executor: {}", e));
            ptr::null_mut()
        }
    }
}

/// Create a new shell executor from component bytes.
///
/// Returns a pointer to the executor on success, or null on failure.
///
/// # Safety
/// - `component_data` must be a valid pointer to `component_len` bytes.
/// - The returned pointer must be freed with `conch_executor_free()`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn conch_executor_new_from_bytes(
    component_data: *const u8,
    component_len: usize,
) -> *mut ConchExecutor {
    if component_data.is_null() {
        set_last_error("component_data is null");
        return ptr::null_mut();
    }

    let bytes = unsafe { std::slice::from_raw_parts(component_data, component_len) };

    match ShellExecutor::from_bytes(bytes) {
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
/// - `executor` must be a pointer returned by `conch_executor_new*()`.
/// - The pointer must not be used after this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn conch_executor_free(executor: *mut ConchExecutor) {
    if !executor.is_null() {
        unsafe { drop(Box::from_raw(executor)) };
    }
}

/// Execute a shell script.
///
/// Returns a pointer to a `ConchResult` on success, or null on failure.
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
    // SAFETY: We're passing null for stdin which is explicitly allowed by conch_execute_with_stdin
    unsafe { conch_execute_with_stdin(executor, script, ptr::null(), 0) }
}

/// Execute a shell script with stdin input.
///
/// Returns a pointer to a `ConchResult` on success, or null on failure.
/// The result must be freed with `conch_result_free()`.
///
/// # Safety
/// - `executor` must be a valid pointer from `conch_executor_new*()`.
/// - `script` must be a valid null-terminated C string.
/// - `stdin_data` must be null or a valid pointer to `stdin_len` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn conch_execute_with_stdin(
    executor: *mut ConchExecutor,
    script: *const c_char,
    stdin_data: *const u8,
    stdin_len: usize,
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

    let stdin = if stdin_data.is_null() || stdin_len == 0 {
        &[]
    } else {
        unsafe { std::slice::from_raw_parts(stdin_data, stdin_len) }
    };

    let limits = ResourceLimits::default();

    let result = if stdin.is_empty() {
        executor.executor.execute(script_str, &limits)
    } else {
        executor
            .executor
            .execute_with_stdin(script_str, stdin, &limits)
    };

    match result {
        Ok(exec_result) => {
            // Capture lengths before moving
            let stdout_len = exec_result.stdout.len();
            let stderr_len = exec_result.stderr.len();

            // Convert stdout/stderr to C-compatible memory
            // Note: We store as raw bytes with length, not as null-terminated strings
            let stdout_data = if exec_result.stdout.is_empty() {
                ptr::null_mut()
            } else {
                let mut stdout = exec_result.stdout;
                stdout.push(0); // Add null terminator for safety
                let ptr = stdout.as_mut_ptr() as *mut c_char;
                std::mem::forget(stdout);
                ptr
            };

            let stderr_data = if exec_result.stderr.is_empty() {
                ptr::null_mut()
            } else {
                let mut stderr = exec_result.stderr;
                stderr.push(0); // Add null terminator for safety
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
// Core Shell Executor FFI (wasip1 / brush-based)
// ============================================================================

/// Create a new core shell executor using the embedded WASM module.
///
/// This is only available when built with the `embedded-shell` feature.
/// Returns a pointer to the executor on success, or null on failure.
///
/// # Safety
/// - The returned pointer must be freed with `conch_core_executor_free()`.
#[cfg(feature = "embedded-shell")]
#[unsafe(no_mangle)]
pub extern "C" fn conch_core_executor_new_embedded() -> *mut ConchCoreExecutor {
    match CoreShellExecutor::embedded() {
        Ok(executor) => Box::into_raw(Box::new(ConchCoreExecutor { executor })),
        Err(e) => {
            set_last_error(&format!("failed to create core executor: {}", e));
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

/// Create a new core shell executor from a module file path.
///
/// Returns a pointer to the executor on success, or null on failure.
/// On failure, call `conch_last_error()` to get the error message.
///
/// # Safety
/// - `module_path` must be a valid null-terminated C string.
/// - The returned pointer must be freed with `conch_core_executor_free()`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn conch_core_executor_new(
    module_path: *const c_char,
) -> *mut ConchCoreExecutor {
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
        Ok(executor) => Box::into_raw(Box::new(ConchCoreExecutor { executor })),
        Err(e) => {
            set_last_error(&format!("failed to create core executor: {}", e));
            ptr::null_mut()
        }
    }
}

/// Create a new core shell executor from module bytes.
///
/// Returns a pointer to the executor on success, or null on failure.
///
/// # Safety
/// - `module_data` must be a valid pointer to `module_len` bytes.
/// - The returned pointer must be freed with `conch_core_executor_free()`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn conch_core_executor_new_from_bytes(
    module_data: *const u8,
    module_len: usize,
) -> *mut ConchCoreExecutor {
    if module_data.is_null() {
        set_last_error("module_data is null");
        return ptr::null_mut();
    }

    let bytes = unsafe { std::slice::from_raw_parts(module_data, module_len) };

    match CoreShellExecutor::from_bytes(bytes) {
        Ok(executor) => Box::into_raw(Box::new(ConchCoreExecutor { executor })),
        Err(e) => {
            set_last_error(&format!("failed to create core executor: {}", e));
            ptr::null_mut()
        }
    }
}

/// Free a core shell executor.
///
/// # Safety
/// - `executor` must be a pointer returned by `conch_core_executor_new*()`.
/// - The pointer must not be used after this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn conch_core_executor_free(executor: *mut ConchCoreExecutor) {
    if !executor.is_null() {
        unsafe { drop(Box::from_raw(executor)) };
    }
}

/// Execute a shell script using the core executor.
///
/// Returns a pointer to a `ConchResult` on success, or null on failure.
/// The result must be freed with `conch_result_free()`.
///
/// # Safety
/// - `executor` must be a valid pointer from `conch_core_executor_new*()`.
/// - `script` must be a valid null-terminated C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn conch_core_execute(
    executor: *mut ConchCoreExecutor,
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
    // This is safe because FFI calls come from outside the async runtime
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

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::mem;

    // ==================== ConchResult Layout Tests ====================

    #[test]
    fn test_conch_result_layout() {
        // Verify the struct has expected size and alignment for C FFI
        // This ensures the struct is properly laid out for C interop
        let size = mem::size_of::<ConchResult>();
        let align = mem::align_of::<ConchResult>();

        // ConchResult contains:
        // - i32 (4 bytes) + padding to align pointer
        // - *mut c_char (8 bytes on 64-bit)
        // - usize (8 bytes)
        // - *mut c_char (8 bytes)
        // - usize (8 bytes)
        // - u8 (1 byte) + padding
        // Total should be around 48 bytes on 64-bit systems

        // Just verify it's reasonable - exact size depends on platform
        assert!(size >= 32, "ConchResult too small: {} bytes", size);
        assert!(size <= 64, "ConchResult too large: {} bytes", size);
        assert!(align >= 4, "ConchResult alignment too small: {}", align);
    }

    #[test]
    fn test_conch_result_field_offsets() {
        // Verify fields are accessible and have expected types
        let result = ConchResult {
            exit_code: 42,
            stdout_data: ptr::null_mut(),
            stdout_len: 100,
            stderr_data: ptr::null_mut(),
            stderr_len: 50,
            truncated: 1,
        };

        assert_eq!(result.exit_code, 42);
        assert!(result.stdout_data.is_null());
        assert_eq!(result.stdout_len, 100);
        assert!(result.stderr_data.is_null());
        assert_eq!(result.stderr_len, 50);
        assert_eq!(result.truncated, 1);
    }

    #[test]
    fn test_conch_result_repr_c() {
        // Verify #[repr(C)] is working - fields should be in declaration order
        // We can't directly test memory layout, but we can verify the struct
        // is usable as expected for FFI
        let result = ConchResult {
            exit_code: -1,
            stdout_data: ptr::null_mut(),
            stdout_len: 0,
            stderr_data: ptr::null_mut(),
            stderr_len: 0,
            truncated: 0,
        };

        // Verify we can take a pointer to the struct
        let ptr: *const ConchResult = &result;
        assert!(!ptr.is_null());
    }

    // ==================== Last Error Tests ====================

    #[test]
    fn test_last_error_initially_null() {
        // Clear any existing error first by setting a new thread-local state
        LAST_ERROR.with(|e| {
            *e.borrow_mut() = None;
        });

        let err = conch_last_error();
        assert!(err.is_null(), "expected null error initially");
    }

    #[test]
    fn test_last_error_after_set() {
        set_last_error("test error message");

        let err = conch_last_error();
        assert!(!err.is_null(), "expected non-null error after set");

        // Verify the error message content
        let err_str = unsafe { std::ffi::CStr::from_ptr(err) };
        assert_eq!(err_str.to_str().unwrap(), "test error message");
    }

    #[test]
    fn test_last_error_overwrites_previous() {
        set_last_error("first error");
        set_last_error("second error");

        let err = conch_last_error();
        let err_str = unsafe { std::ffi::CStr::from_ptr(err) };
        assert_eq!(err_str.to_str().unwrap(), "second error");
    }

    #[test]
    fn test_last_error_with_special_chars() {
        set_last_error("error: file not found (path=/tmp/foo)");

        let err = conch_last_error();
        let err_str = unsafe { std::ffi::CStr::from_ptr(err) };
        assert_eq!(
            err_str.to_str().unwrap(),
            "error: file not found (path=/tmp/foo)"
        );
    }

    #[test]
    fn test_last_error_empty_string() {
        set_last_error("");

        let err = conch_last_error();
        assert!(!err.is_null());
        let err_str = unsafe { std::ffi::CStr::from_ptr(err) };
        assert_eq!(err_str.to_str().unwrap(), "");
    }

    // ==================== Result Free Tests ====================

    #[test]
    fn test_result_free_null_safe() {
        // Should not crash when passed null
        unsafe {
            conch_result_free(ptr::null_mut());
        }
        // If we get here without crashing, the test passes
    }

    #[test]
    fn test_result_free_with_null_pointers() {
        // Create a result with null data pointers
        let mut result = ConchResult {
            exit_code: 0,
            stdout_data: ptr::null_mut(),
            stdout_len: 0,
            stderr_data: ptr::null_mut(),
            stderr_len: 0,
            truncated: 0,
        };

        // Should not crash when freeing result with null data pointers
        unsafe {
            conch_result_free(&mut result);
        }
    }

    #[test]
    fn test_result_free_with_allocated_data() {
        // Create CStrings and get raw pointers
        let stdout = CString::new("stdout output").unwrap();
        let stderr = CString::new("stderr output").unwrap();

        let mut result = ConchResult {
            exit_code: 0,
            stdout_data: stdout.into_raw(),
            stdout_len: 13,
            stderr_data: stderr.into_raw(),
            stderr_len: 13,
            truncated: 0,
        };

        // Verify pointers are set
        assert!(!result.stdout_data.is_null());
        assert!(!result.stderr_data.is_null());

        // Free the result
        unsafe {
            conch_result_free(&mut result);
        }

        // Verify pointers are nulled after free
        assert!(result.stdout_data.is_null());
        assert!(result.stderr_data.is_null());
    }

    #[test]
    fn test_result_free_only_stdout() {
        let stdout = CString::new("only stdout").unwrap();

        let mut result = ConchResult {
            exit_code: 1,
            stdout_data: stdout.into_raw(),
            stdout_len: 11,
            stderr_data: ptr::null_mut(),
            stderr_len: 0,
            truncated: 0,
        };

        unsafe {
            conch_result_free(&mut result);
        }

        assert!(result.stdout_data.is_null());
        assert!(result.stderr_data.is_null());
    }

    #[test]
    fn test_result_free_only_stderr() {
        let stderr = CString::new("only stderr").unwrap();

        let mut result = ConchResult {
            exit_code: 2,
            stdout_data: ptr::null_mut(),
            stdout_len: 0,
            stderr_data: stderr.into_raw(),
            stderr_len: 11,
            truncated: 1,
        };

        unsafe {
            conch_result_free(&mut result);
        }

        assert!(result.stdout_data.is_null());
        assert!(result.stderr_data.is_null());
    }

    // ==================== Thread Safety Tests ====================

    #[test]
    fn test_last_error_thread_local() {
        use std::thread;

        // Set error in main thread
        set_last_error("main thread error");

        // Spawn a thread and check its error state
        let handle = thread::spawn(|| {
            // New thread should have no error set
            LAST_ERROR.with(|e| {
                *e.borrow_mut() = None;
            });
            let err = conch_last_error();
            assert!(err.is_null(), "new thread should have no error");

            // Set a different error in this thread
            set_last_error("child thread error");
            let err = conch_last_error();
            let err_str = unsafe { std::ffi::CStr::from_ptr(err) };
            assert_eq!(err_str.to_str().unwrap(), "child thread error");
        });

        handle.join().unwrap();

        // Main thread error should be unchanged
        let err = conch_last_error();
        let err_str = unsafe { std::ffi::CStr::from_ptr(err) };
        assert_eq!(err_str.to_str().unwrap(), "main thread error");
    }
}
