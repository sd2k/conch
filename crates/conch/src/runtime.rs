//! WASM runtime for shell execution

use std::fmt;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::Semaphore;

use crate::limits::ResourceLimits;
use crate::vfs::{AccessPolicy, ContextFs, ContextProvider};
use crate::wasm_core::CoreShellExecutor;

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
}

/// Execution context for the shell
#[derive(Clone)]
pub struct ExecutionContext {
    /// ID of the current agent
    pub agent_id: String,
    /// ID of the parent agent, if any
    pub parent_agent_id: Option<String>,
    /// IDs of child agents
    pub child_agent_ids: Vec<String>,
    /// ID of the shared investigation, if any
    pub investigation_id: Option<String>,
    /// Provider for context data (tool calls, messages, etc.)
    pub provider: Arc<dyn ContextProvider>,
}

impl fmt::Debug for ExecutionContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExecutionContext")
            .field("agent_id", &self.agent_id)
            .field("parent_agent_id", &self.parent_agent_id)
            .field("child_agent_ids", &self.child_agent_ids)
            .field("investigation_id", &self.investigation_id)
            .field("provider", &"<dyn ContextProvider>")
            .finish()
    }
}

impl ExecutionContext {
    /// Create a new execution context for an agent
    pub fn new(agent_id: impl Into<String>, provider: Arc<dyn ContextProvider>) -> Self {
        Self {
            agent_id: agent_id.into(),
            parent_agent_id: None,
            child_agent_ids: Vec::new(),
            investigation_id: None,
            provider,
        }
    }

    /// Set the parent agent ID
    pub fn with_parent(mut self, parent_id: impl Into<String>) -> Self {
        self.parent_agent_id = Some(parent_id.into());
        self
    }

    /// Set the child agent IDs
    pub fn with_children(mut self, children: Vec<String>) -> Self {
        self.child_agent_ids = children;
        self
    }

    /// Set the investigation ID for shared context
    pub fn with_investigation(mut self, investigation_id: impl Into<String>) -> Self {
        self.investigation_id = Some(investigation_id.into());
        self
    }

    /// Build an access policy from this context
    pub fn build_policy(&self) -> AccessPolicy {
        AccessPolicy {
            agent_id: self.agent_id.clone(),
            parent_agent_id: self.parent_agent_id.clone(),
            child_agent_ids: self.child_agent_ids.clone(),
            investigation_id: self.investigation_id.clone(),
            can_read_parent_tools: true,
            can_read_parent_messages: false,
        }
    }

    /// Build a virtual filesystem from this context
    pub fn build_fs(&self) -> ContextFs {
        ContextFs::new(self.provider.clone(), self.build_policy())
    }
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
/// Wraps the CoreShellExecutor to run shell scripts in a WASM sandbox.
/// Provides concurrency limiting and execution context management.
pub struct Conch {
    max_concurrent: usize,
    semaphore: Arc<Semaphore>,
    executor: CoreShellExecutor,
}

impl fmt::Debug for Conch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Conch")
            .field("max_concurrent", &self.max_concurrent)
            .finish_non_exhaustive()
    }
}

impl Conch {
    /// Create a new shell execution engine from a WASM module file.
    pub fn from_file(
        path: impl AsRef<std::path::Path>,
        max_concurrent: usize,
    ) -> Result<Self, RuntimeError> {
        let executor = CoreShellExecutor::from_file(path)?;
        Ok(Self {
            max_concurrent,
            semaphore: Arc::new(Semaphore::new(max_concurrent)),
            executor,
        })
    }

    /// Create a new shell execution engine from WASM module bytes.
    pub fn from_bytes(bytes: &[u8], max_concurrent: usize) -> Result<Self, RuntimeError> {
        let executor = CoreShellExecutor::from_bytes(bytes)?;
        Ok(Self {
            max_concurrent,
            semaphore: Arc::new(Semaphore::new(max_concurrent)),
            executor,
        })
    }

    /// Create a new shell execution engine using the embedded WASM module.
    ///
    /// This is only available when built with the `embedded-shell` feature.
    #[cfg(feature = "embedded-shell")]
    pub fn embedded(max_concurrent: usize) -> Result<Self, RuntimeError> {
        let executor = CoreShellExecutor::embedded()?;
        Ok(Self {
            max_concurrent,
            semaphore: Arc::new(Semaphore::new(max_concurrent)),
            executor,
        })
    }

    /// Execute a shell script with the given execution context.
    ///
    /// The execution context provides agent identity and access to the virtual
    /// filesystem (VFS). Note: VFS integration is not yet complete - currently
    /// only standard WASI filesystem is available.
    pub async fn execute(
        &self,
        script: &str,
        context: ExecutionContext,
        limits: ResourceLimits,
    ) -> Result<ExecutionResult, RuntimeError> {
        let _permit = self
            .semaphore
            .acquire()
            .await
            .map_err(|_| RuntimeError::Semaphore)?;

        // Build the virtual filesystem (for future use)
        // TODO: Wire up ContextFs to the WASM executor's WASI layer
        let _vfs = context.build_fs();

        // Execute via the core shell executor
        // This runs synchronously but we're in an async context for the semaphore
        self.executor.execute(script, &limits)
    }

    /// Execute a shell script without an execution context.
    ///
    /// This is a simpler API when you don't need agent context or VFS.
    pub async fn execute_simple(
        &self,
        script: &str,
        limits: ResourceLimits,
    ) -> Result<ExecutionResult, RuntimeError> {
        let _permit = self
            .semaphore
            .acquire()
            .await
            .map_err(|_| RuntimeError::Semaphore)?;

        self.executor.execute(script, &limits)
    }

    /// Get the maximum concurrent executions
    pub fn max_concurrent(&self) -> usize {
        self.max_concurrent
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vfs::MockContextProvider;

    fn get_test_shell() -> Option<Conch> {
        // Try embedded first
        #[cfg(feature = "embedded-shell")]
        {
            return Conch::embedded(1).ok();
        }

        // Fall back to file-based
        #[cfg(not(feature = "embedded-shell"))]
        {
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
                Conch::from_file(&module_path, 1).ok()
            } else {
                eprintln!("Module not found at {:?}", module_path);
                None
            }
        }
    }

    #[tokio::test]
    async fn test_basic_execution() {
        let Some(shell) = get_test_shell() else {
            eprintln!(
                "Skipping test: module not built. Run: cargo build -p conch-shell --target wasm32-wasip1 --release"
            );
            return;
        };

        let provider = Arc::new(MockContextProvider::new());
        let context = ExecutionContext::new("test-agent", provider);

        let result = shell
            .execute("echo hello", context, ResourceLimits::default())
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
            .execute_simple("echo hello world", ResourceLimits::default())
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
            .execute_simple("echo hello | cat", ResourceLimits::default())
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
            .execute_simple("false", ResourceLimits::default())
            .await
            .unwrap();

        assert_eq!(result.exit_code, 1);
    }
}
