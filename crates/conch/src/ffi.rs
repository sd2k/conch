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
use std::sync::Arc;

use eryx_vfs::{DirPerms, FilePerms, HybridVfsCtx, InMemoryStorage};

use crate::executor::ComponentShellExecutor;
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
    executor: ComponentShellExecutor,
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
    match ComponentShellExecutor::embedded() {
        Ok(executor) => Box::into_raw(Box::new(ConchExecutor { executor })),
        Err(e) => {
            set_last_error(&format!("failed to create executor: {}", e));
            ptr::null_mut()
        }
    }
}

/// Stub for when embedded-shell feature is disabled.
#[cfg(not(feature = "embedded-shell"))]
#[unsafe(no_mangle)]
pub extern "C" fn conch_executor_new_embedded() -> *mut ConchExecutor {
    set_last_error("embedded-shell feature not enabled");
    ptr::null_mut()
}

/// Create a new shell executor from a WASM file path.
///
/// Returns a pointer to the executor on success, or null on failure.
/// On failure, call `conch_last_error()` to get the error message.
///
/// # Safety
/// - `path` must be a valid null-terminated C string.
/// - The returned pointer must be freed with `conch_executor_free()`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn conch_executor_new_from_file(path: *const c_char) -> *mut ConchExecutor {
    if path.is_null() {
        set_last_error("path is null");
        return ptr::null_mut();
    }

    let path_str = match unsafe { CStr::from_ptr(path) }.to_str() {
        Ok(s) => s,
        Err(e) => {
            set_last_error(&format!("invalid UTF-8 in path: {}", e));
            return ptr::null_mut();
        }
    };

    match ComponentShellExecutor::from_file(path_str) {
        Ok(executor) => Box::into_raw(Box::new(ConchExecutor { executor })),
        Err(e) => {
            set_last_error(&format!("failed to load component: {}", e));
            ptr::null_mut()
        }
    }
}

/// Create a new shell executor from WASM bytes.
///
/// Returns a pointer to the executor on success, or null on failure.
/// On failure, call `conch_last_error()` to get the error message.
///
/// # Safety
/// - `bytes` must be a valid pointer to `len` bytes.
/// - The returned pointer must be freed with `conch_executor_free()`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn conch_executor_new_from_bytes(
    bytes: *const u8,
    len: usize,
) -> *mut ConchExecutor {
    if bytes.is_null() {
        set_last_error("bytes is null");
        return ptr::null_mut();
    }

    let slice = unsafe { std::slice::from_raw_parts(bytes, len) };

    match ComponentShellExecutor::from_bytes(slice) {
        Ok(executor) => Box::into_raw(Box::new(ConchExecutor { executor })),
        Err(e) => {
            set_last_error(&format!("failed to load component: {}", e));
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
// Execution helpers
// ============================================================================

/// Helper to execute a script and convert the result to ConchResult.
#[cfg(feature = "embedded-shell")]
async fn execute_script_internal(
    executor: &ComponentShellExecutor,
    script: &str,
    limits: &ResourceLimits,
) -> Result<crate::runtime::ExecutionResult, crate::runtime::RuntimeError> {
    // Create a minimal VFS context with a /tmp directory
    let storage = Arc::new(InMemoryStorage::new());
    let mut hybrid_ctx = HybridVfsCtx::new(storage);
    hybrid_ctx.add_vfs_preopen("/tmp", DirPerms::all(), FilePerms::all());

    // Create a temporary shell instance
    let mut instance = executor.create_instance(limits, hybrid_ctx, None).await?;

    // Execute the script
    instance.execute(script, limits).await
}

/// Convert an ExecutionResult to a ConchResult pointer.
fn result_to_conch_result(exec_result: crate::runtime::ExecutionResult) -> *mut ConchResult {
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

// ============================================================================
// Execution
// ============================================================================

/// Execute a shell script.
///
/// Each call creates a fresh shell instance, so state (variables, functions)
/// does not persist between calls.
///
/// Returns a pointer to a `ConchResult` on success, or null on failure.
/// On failure, call `conch_last_error()` to get the error message.
/// The result must be freed with `conch_result_free()`.
///
/// # Safety
/// - `executor` must be a valid pointer from `conch_executor_new*()`.
/// - `script` must be a valid null-terminated C string.
#[cfg(feature = "embedded-shell")]
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

    match rt.block_on(execute_script_internal(
        &executor.executor,
        script_str,
        &limits,
    )) {
        Ok(exec_result) => result_to_conch_result(exec_result),
        Err(e) => {
            set_last_error(&format!("execution failed: {}", e));
            ptr::null_mut()
        }
    }
}

/// Stub for when embedded-shell feature is disabled.
#[cfg(not(feature = "embedded-shell"))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn conch_execute(
    _executor: *mut ConchExecutor,
    _script: *const c_char,
) -> *mut ConchResult {
    set_last_error("embedded-shell feature not enabled");
    ptr::null_mut()
}

/// Execute a shell script with custom resource limits.
///
/// Each call creates a fresh shell instance, so state (variables, functions)
/// does not persist between calls.
///
/// Returns a pointer to a `ConchResult` on success, or null on failure.
/// On failure, call `conch_last_error()` to get the error message.
/// The result must be freed with `conch_result_free()`.
///
/// # Safety
/// - `executor` must be a valid pointer from `conch_executor_new*()`.
/// - `script` must be a valid null-terminated C string.
#[cfg(feature = "embedded-shell")]
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

    match rt.block_on(execute_script_internal(
        &executor.executor,
        script_str,
        &limits,
    )) {
        Ok(exec_result) => result_to_conch_result(exec_result),
        Err(e) => {
            set_last_error(&format!("execution failed: {}", e));
            ptr::null_mut()
        }
    }
}

/// Stub for when embedded-shell feature is disabled.
#[cfg(not(feature = "embedded-shell"))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn conch_execute_with_limits(
    _executor: *mut ConchExecutor,
    _script: *const c_char,
    _max_cpu_ms: u64,
    _max_memory_bytes: u64,
    _max_output_bytes: u64,
    _timeout_ms: u64,
) -> *mut ConchResult {
    set_last_error("embedded-shell feature not enabled");
    ptr::null_mut()
}

// ============================================================================
// Result handling
// ============================================================================

/// Free a `ConchResult` returned by `conch_execute*()`.
///
/// # Safety
/// - `result` must be a pointer returned by `conch_execute*()`, or null.
/// - The pointer must not be used after this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn conch_result_free(result: *mut ConchResult) {
    if result.is_null() {
        return;
    }

    let result = unsafe { Box::from_raw(result) };

    // Free the stdout buffer if allocated
    if !result.stdout_data.is_null() {
        unsafe {
            let _ = Vec::from_raw_parts(
                result.stdout_data as *mut u8,
                result.stdout_len + 1, // +1 for null terminator
                result.stdout_len + 1,
            );
        }
    }

    // Free the stderr buffer if allocated
    if !result.stderr_data.is_null() {
        unsafe {
            let _ = Vec::from_raw_parts(
                result.stderr_data as *mut u8,
                result.stderr_len + 1, // +1 for null terminator
                result.stderr_len + 1,
            );
        }
    }

    // Box will be dropped here, freeing the ConchResult struct
}

// ============================================================================
// Component bytes (for Go to load the embedded component)
// ============================================================================

/// Get the embedded WASM component bytes.
///
/// Returns a pointer to the component bytes and sets `len` to the length.
/// Returns null if the embedded-shell feature is not enabled.
///
/// # Safety
/// - `len` must be a valid pointer to a usize.
/// - The returned pointer is valid for the lifetime of the program.
/// - Do not free the returned pointer.
#[cfg(feature = "embedded-shell")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn conch_embedded_component_bytes(len: *mut usize) -> *const u8 {
    let bytes = ComponentShellExecutor::embedded_component_bytes();
    if !len.is_null() {
        unsafe { *len = bytes.len() };
    }
    bytes.as_ptr()
}

/// Stub for when embedded-shell feature is disabled.
#[cfg(not(feature = "embedded-shell"))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn conch_embedded_component_bytes(len: *mut usize) -> *const u8 {
    if !len.is_null() {
        unsafe { *len = 0 };
    }
    ptr::null()
}
