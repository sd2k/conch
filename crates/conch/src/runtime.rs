//! WASM runtime for shell execution

use std::fmt;
use std::sync::Arc;

use eryx_vfs::{DirPerms, FilePerms, HybridVfsCtx, InMemoryStorage};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::Semaphore;

use crate::executor::ComponentShellExecutor;
use crate::limits::ResourceLimits;

/// Errors that can occur during shell execution
#[derive(Debug, Error)]
pub enum RuntimeError {
    /// Error from the WASM runtime
    #[error("WASM error: {0}")]
    Wasm(String),
    /// Execution timeout exceeded
    #[error("timeout exceeded")]
    Timeout,
    /// Memory limit exceeded
    #[error("memory limit exceeded")]
    MemoryLimit,
    /// Concurrency semaphore error
    #[error("semaphore error")]
    Semaphore,
    /// IO error
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    /// VFS error
    #[error("VFS error: {0}")]
    Vfs(String),
}

/// Statistics about shell execution
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExecutionStats {
    /// CPU time consumed in milliseconds
    pub cpu_time_ms: u64,
    /// Peak memory usage in bytes
    pub peak_memory_bytes: u64,
    /// Wall clock time in milliseconds
    pub wall_time_ms: u64,
}

/// Result of shell execution
#[derive(Debug, Clone)]
pub struct ExecutionResult {
    /// Shell exit code
    pub exit_code: i32,
    /// Standard output
    pub stdout: Vec<u8>,
    /// Standard error
    pub stderr: Vec<u8>,
    /// Whether output was truncated due to limits
    pub truncated: bool,
    /// Execution statistics
    pub stats: ExecutionStats,
}

/// Shell execution engine
///
/// Wraps the ComponentShellExecutor to run shell scripts in a WASM sandbox.
/// Provides concurrency limiting for executing multiple scripts.
///
/// Each execution creates a fresh shell instance, so state does not persist
/// between calls. For stateful execution with persistent variables and VFS,
/// use the [`Shell`](crate::Shell) API instead.
pub struct Conch {
    max_concurrent: usize,
    semaphore: Arc<Semaphore>,
    executor: ComponentShellExecutor,
}

impl fmt::Debug for Conch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Conch")
            .field("max_concurrent", &self.max_concurrent)
            .finish_non_exhaustive()
    }
}

impl Conch {
    /// Create a new shell execution engine from a WASM component file.
    pub fn from_file(
        path: impl AsRef<std::path::Path>,
        max_concurrent: usize,
    ) -> Result<Self, RuntimeError> {
        let executor = ComponentShellExecutor::from_file(path)?;
        Ok(Self {
            max_concurrent,
            semaphore: Arc::new(Semaphore::new(max_concurrent)),
            executor,
        })
    }

    /// Create a new shell execution engine from WASM component bytes.
    pub fn from_bytes(bytes: &[u8], max_concurrent: usize) -> Result<Self, RuntimeError> {
        let executor = ComponentShellExecutor::from_bytes(bytes)?;
        Ok(Self {
            max_concurrent,
            semaphore: Arc::new(Semaphore::new(max_concurrent)),
            executor,
        })
    }

    /// Create a new shell execution engine using the embedded WASM component.
    ///
    /// This is only available when built with the `embedded-shell` feature.
    #[cfg(feature = "embedded-shell")]
    pub fn embedded(max_concurrent: usize) -> Result<Self, RuntimeError> {
        let executor = ComponentShellExecutor::embedded()?;
        Ok(Self {
            max_concurrent,
            semaphore: Arc::new(Semaphore::new(max_concurrent)),
            executor,
        })
    }

    /// Execute a shell script.
    ///
    /// This creates a temporary shell instance for each execution, so state
    /// (variables, functions, aliases) does not persist between calls.
    ///
    /// For stateful execution, use [`Shell`](crate::Shell) instead.
    #[cfg(feature = "embedded-shell")]
    pub async fn execute(
        &self,
        script: &str,
        limits: ResourceLimits,
    ) -> Result<ExecutionResult, RuntimeError> {
        let _permit = self
            .semaphore
            .acquire()
            .await
            .map_err(|_| RuntimeError::Semaphore)?;

        // Create a minimal VFS context with a /tmp directory
        let storage = Arc::new(InMemoryStorage::new());
        let mut hybrid_ctx = HybridVfsCtx::new(storage);
        hybrid_ctx.add_vfs_preopen("/tmp", DirPerms::all(), FilePerms::all());

        // Create a temporary shell instance
        let mut instance = self
            .executor
            .create_instance(&limits, hybrid_ctx, None)
            .await?;

        // Execute the script
        instance.execute(script, &limits).await
    }

    /// Execute a shell script (stub for when embedded-shell is disabled).
    #[cfg(not(feature = "embedded-shell"))]
    pub async fn execute(
        &self,
        _script: &str,
        _limits: ResourceLimits,
    ) -> Result<ExecutionResult, RuntimeError> {
        Err(RuntimeError::Wasm(
            "execute requires embedded-shell feature".to_string(),
        ))
    }

    /// Get the maximum concurrent executions
    pub fn max_concurrent(&self) -> usize {
        self.max_concurrent
    }
}

#[cfg(test)]
#[cfg(feature = "embedded-shell")]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn get_test_shell() -> Option<Conch> {
        Conch::embedded(1).ok()
    }

    #[tokio::test]
    async fn test_basic_execution() {
        let Some(shell) = get_test_shell() else {
            eprintln!(
                "Skipping test: module not built. Run: cargo build -p conch-shell --target wasm32-wasip2 --release"
            );
            return;
        };

        let result = shell
            .execute("echo hello", ResourceLimits::default())
            .await
            .unwrap();

        assert_eq!(result.exit_code, 0);
    }

    #[tokio::test]
    async fn test_simple_execution() {
        let Some(shell) = get_test_shell() else {
            return;
        };

        let result = shell
            .execute("echo hello world", ResourceLimits::default())
            .await
            .unwrap();

        assert_eq!(result.exit_code, 0);
    }

    #[tokio::test]
    async fn test_pipeline_execution() {
        let Some(shell) = get_test_shell() else {
            return;
        };

        let result = shell
            .execute("echo hello | cat", ResourceLimits::default())
            .await
            .unwrap();

        assert_eq!(result.exit_code, 0);
    }

    #[tokio::test]
    async fn test_failing_command() {
        let Some(shell) = get_test_shell() else {
            return;
        };

        let result = shell
            .execute("false", ResourceLimits::default())
            .await
            .unwrap();

        assert_eq!(result.exit_code, 1);
    }

    #[tokio::test]
    async fn test_stateless_execution() {
        // Verify that state does NOT persist between calls (this is the Conch behavior)
        let Some(shell) = get_test_shell() else {
            return;
        };

        // Set a variable
        let result = shell
            .execute("x=42", ResourceLimits::default())
            .await
            .unwrap();
        assert_eq!(result.exit_code, 0);

        // Variable should NOT be visible in next call (fresh instance)
        let result = shell
            .execute("echo ${x:-unset}", ResourceLimits::default())
            .await
            .unwrap();
        assert_eq!(result.exit_code, 0);
        let stdout = String::from_utf8_lossy(&result.stdout);
        assert!(
            stdout.contains("unset"),
            "Variable should not persist: {:?}",
            stdout
        );
    }
}
