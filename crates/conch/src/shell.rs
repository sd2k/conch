//! Shell API for executing commands in a sandboxed environment.
//!
//! The [`Shell`] provides a high-level interface for executing shell commands
//! in a WASM sandbox with a hybrid virtual filesystem that combines:
//!
//! - **VFS storage**: In-memory or custom storage for orchestrator-controlled paths
//! - **Real filesystem**: cap-std secured mounts for host directory access
//!
//! ## State Persistence
//!
//! Unlike stateless execution, the Shell maintains state across multiple `execute` calls.
//! Variables, functions, and aliases defined in one execution persist to subsequent ones.
//!
//! # Example
//!
//! ```rust,ignore
//! use conch::{Shell, Mount, ResourceLimits};
//!
//! // Create a shell with a real filesystem mount
//! let mut shell = Shell::builder()
//!     .mount("/project", "/home/user/code", Mount::readonly())
//!     .build()
//!     .await?;
//!
//! // Write data to VFS scratch area
//! shell.vfs().write("/scratch/input.txt", b"hello").await?;
//!
//! // Execute commands - state persists between calls
//! shell.execute("x=42", &ResourceLimits::default()).await?;
//! let result = shell.execute("echo $x", &ResourceLimits::default()).await?;
//! // result.stdout contains "42"
//! ```

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use eryx_vfs::{
    DirEntry, DirPerms, FilePerms, HybridVfsCtx, InMemoryStorage, Metadata, RealDir, VfsResult,
    VfsStorage,
};

use crate::executor::ComponentShellExecutor;

#[cfg(feature = "embedded-shell")]
use crate::executor::ToolHandler;
use crate::limits::ResourceLimits;
use crate::runtime::{ExecutionResult, RuntimeError};

#[cfg(feature = "embedded-shell")]
use crate::executor::ShellInstance;
#[cfg(feature = "embedded-shell")]
use crate::snapshot::Snapshot;

/// A sized wrapper around `Arc<dyn VfsStorage>` that implements `VfsStorage`.
///
/// This allows us to use dynamic dispatch for VFS storage while still being
/// compatible with APIs that require `Sized` types (like `HybridVfsCtx<S>`).
#[derive(Clone)]
pub struct DynVfsStorage(Arc<dyn VfsStorage>);

impl std::fmt::Debug for DynVfsStorage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DynVfsStorage").finish_non_exhaustive()
    }
}

impl DynVfsStorage {
    /// Create a new `DynVfsStorage` from any `VfsStorage` implementation.
    pub fn new(storage: impl VfsStorage + 'static) -> Self {
        Self(Arc::new(storage))
    }

    /// Create a new `DynVfsStorage` from an existing `Arc<dyn VfsStorage>`.
    pub fn from_arc(storage: Arc<dyn VfsStorage>) -> Self {
        Self(storage)
    }

    /// Get a reference to the underlying storage.
    pub fn inner(&self) -> &dyn VfsStorage {
        &*self.0
    }

    /// Get the inner Arc.
    pub fn arc(&self) -> Arc<dyn VfsStorage> {
        Arc::clone(&self.0)
    }
}

#[async_trait]
impl VfsStorage for DynVfsStorage {
    async fn read(&self, path: &str) -> VfsResult<Vec<u8>> {
        self.0.read(path).await
    }

    async fn read_at(&self, path: &str, offset: u64, len: u64) -> VfsResult<Vec<u8>> {
        self.0.read_at(path, offset, len).await
    }

    async fn write(&self, path: &str, data: &[u8]) -> VfsResult<()> {
        self.0.write(path, data).await
    }

    async fn write_at(&self, path: &str, offset: u64, data: &[u8]) -> VfsResult<()> {
        self.0.write_at(path, offset, data).await
    }

    async fn set_size(&self, path: &str, size: u64) -> VfsResult<()> {
        self.0.set_size(path, size).await
    }

    async fn delete(&self, path: &str) -> VfsResult<()> {
        self.0.delete(path).await
    }

    async fn exists(&self, path: &str) -> VfsResult<bool> {
        self.0.exists(path).await
    }

    async fn list(&self, path: &str) -> VfsResult<Vec<DirEntry>> {
        self.0.list(path).await
    }

    async fn stat(&self, path: &str) -> VfsResult<Metadata> {
        self.0.stat(path).await
    }

    async fn mkdir(&self, path: &str) -> VfsResult<()> {
        self.0.mkdir(path).await
    }

    async fn rmdir(&self, path: &str) -> VfsResult<()> {
        self.0.rmdir(path).await
    }

    async fn rename(&self, from: &str, to: &str) -> VfsResult<()> {
        self.0.rename(from, to).await
    }

    fn mkdir_sync(&self, path: &str) -> VfsResult<()> {
        self.0.mkdir_sync(path)
    }
}

/// Mount permissions for real filesystem paths.
#[derive(Debug, Clone)]
pub struct Mount {
    dir_perms: DirPerms,
    file_perms: FilePerms,
}

impl Mount {
    /// Read-only access to directory and files.
    pub fn readonly() -> Self {
        Self {
            dir_perms: DirPerms::READ,
            file_perms: FilePerms::READ,
        }
    }

    /// Full read-write access.
    pub fn readwrite() -> Self {
        Self {
            dir_perms: DirPerms::all(),
            file_perms: FilePerms::all(),
        }
    }

    /// Custom permissions.
    pub fn with_perms(dir_perms: DirPerms, file_perms: FilePerms) -> Self {
        Self {
            dir_perms,
            file_perms,
        }
    }
}

/// Configuration for a VFS path (virtual storage).
#[derive(Debug, Clone)]
struct VfsMount {
    guest_path: String,
    dir_perms: DirPerms,
    file_perms: FilePerms,
}

/// Configuration for a real filesystem mount.
#[derive(Debug, Clone)]
struct RealMount {
    guest_path: String,
    host_path: PathBuf,
    dir_perms: DirPerms,
    file_perms: FilePerms,
}

/// Builder for constructing a [`Shell`] with custom configuration.
///
/// # Example
///
/// ```rust,ignore
/// let mut shell = Shell::builder()
///     .vfs(my_custom_storage)
///     .mount("/project", "/home/user/code", Mount::readonly())
///     .mount("/output", "/tmp/agent-output", Mount::readwrite())
///     .vfs_path("/data", DirPerms::READ, FilePerms::READ)
///     .tool_handler(|req| async move {
///         ToolResult { success: true, output: format!("Called: {}", req.tool) }
///     })
///     .build()
///     .await?;
/// ```
pub struct ShellBuilder {
    vfs: Option<DynVfsStorage>,
    vfs_mounts: Vec<VfsMount>,
    real_mounts: Vec<RealMount>,
    executor: Option<ComponentShellExecutor>,
    #[cfg(feature = "embedded-shell")]
    tool_handler: Option<Arc<dyn ToolHandler>>,
    #[cfg(feature = "embedded-shell")]
    component_registry: Option<Arc<crate::executor::ComponentRegistry>>,
    limits: Option<ResourceLimits>,
}

impl std::fmt::Debug for ShellBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut s = f.debug_struct("ShellBuilder");
        s.field("has_vfs", &self.vfs.is_some())
            .field("vfs_mounts", &self.vfs_mounts)
            .field("real_mounts", &self.real_mounts)
            .field("has_executor", &self.executor.is_some());
        #[cfg(feature = "embedded-shell")]
        s.field("has_tool_handler", &self.tool_handler.is_some());
        #[cfg(feature = "embedded-shell")]
        s.field("has_component_registry", &self.component_registry.is_some());
        s.finish()
    }
}

impl Default for ShellBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl ShellBuilder {
    /// Create a new shell builder with default settings.
    ///
    /// Uses [`InMemoryStorage`] for VFS by default.
    pub fn new() -> Self {
        Self {
            vfs: None,
            vfs_mounts: Vec::new(),
            real_mounts: Vec::new(),
            executor: None,
            #[cfg(feature = "embedded-shell")]
            tool_handler: None,
            #[cfg(feature = "embedded-shell")]
            component_registry: None,
            limits: None,
        }
    }

    /// Set a custom VFS storage backend.
    pub fn vfs(mut self, storage: impl VfsStorage + 'static) -> Self {
        self.vfs = Some(DynVfsStorage::new(storage));
        self
    }

    /// Set a custom VFS storage backend from an Arc.
    pub fn vfs_arc(mut self, storage: Arc<dyn VfsStorage>) -> Self {
        self.vfs = Some(DynVfsStorage::from_arc(storage));
        self
    }

    /// Add a VFS path (backed by virtual storage).
    ///
    /// Paths under `guest_path` will be handled by the VFS storage.
    pub fn vfs_path(
        mut self,
        guest_path: impl Into<String>,
        dir_perms: DirPerms,
        file_perms: FilePerms,
    ) -> Self {
        self.vfs_mounts.push(VfsMount {
            guest_path: guest_path.into(),
            dir_perms,
            file_perms,
        });
        self
    }

    /// Add a real filesystem mount (backed by cap-std).
    ///
    /// The `host_path` directory will be accessible at `guest_path` inside the shell.
    pub fn mount(
        mut self,
        guest_path: impl Into<String>,
        host_path: impl AsRef<Path>,
        mount: Mount,
    ) -> Self {
        self.real_mounts.push(RealMount {
            guest_path: guest_path.into(),
            host_path: host_path.as_ref().to_path_buf(),
            dir_perms: mount.dir_perms,
            file_perms: mount.file_perms,
        });
        self
    }

    /// Set a custom WASM executor.
    ///
    /// If not set, the embedded executor will be used (requires `embedded-shell` feature).
    pub fn executor(mut self, executor: ComponentShellExecutor) -> Self {
        self.executor = Some(executor);
        self
    }

    /// Set default resource limits for execution.
    ///
    /// These can be overridden per-execute call.
    pub fn limits(mut self, limits: ResourceLimits) -> Self {
        self.limits = Some(limits);
        self
    }

    /// Set a tool handler for processing tool invocations from shell scripts.
    ///
    /// When a script runs `tool <name> --param value`, the handler will be called
    /// to execute the tool asynchronously.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use conch::{Shell, ToolRequest, ToolResult};
    ///
    /// let mut shell = Shell::builder()
    ///     .tool_handler(|req: ToolRequest| async move {
    ///         match req.tool.as_str() {
    ///             "echo" => ToolResult {
    ///                 success: true,
    ///                 output: req.params.clone(),
    ///             },
    ///             _ => ToolResult {
    ///                 success: false,
    ///                 output: format!("Unknown tool: {}", req.tool),
    ///             },
    ///         }
    ///     })
    ///     .build()
    ///     .await?;
    /// ```
    #[cfg(feature = "embedded-shell")]
    pub fn tool_handler(mut self, handler: impl ToolHandler + 'static) -> Self {
        self.tool_handler = Some(Arc::new(handler));
        self
    }

    /// Set a component registry for subprocess spawning.
    ///
    /// When a shell script runs an unknown command, the registry is consulted
    /// to find a WASI component to instantiate for that command.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use conch::{Shell, ComponentRegistry};
    /// use wasmtime::component::Component;
    ///
    /// let mut registry = ComponentRegistry::new();
    /// registry.register("upper", component);
    ///
    /// let mut shell = Shell::builder()
    ///     .component_registry(registry)
    ///     .build()
    ///     .await?;
    ///
    /// // "upper" is now available as a command
    /// shell.execute("echo hello | upper", &limits).await?;
    /// ```
    #[cfg(feature = "embedded-shell")]
    pub fn component_registry(mut self, registry: crate::executor::ComponentRegistry) -> Self {
        self.component_registry = Some(Arc::new(registry));
        self
    }

    /// Build the shell with the configured settings.
    ///
    /// This is an async operation because it creates and initializes the WASM
    /// shell instance.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - A real filesystem mount path doesn't exist or can't be opened
    /// - The executor fails to initialize
    /// - The WASM shell instance fails to create
    #[cfg(feature = "embedded-shell")]
    pub async fn build(self) -> Result<Shell, RuntimeError> {
        // Use provided VFS or create default InMemoryStorage
        let vfs = self
            .vfs
            .unwrap_or_else(|| DynVfsStorage::new(InMemoryStorage::new()));

        // Collect VFS mounts, adding default /scratch if none specified
        let vfs_mounts = if self.vfs_mounts.is_empty() {
            vec![VfsMount {
                guest_path: "/scratch".to_string(),
                dir_perms: DirPerms::all(),
                file_perms: FilePerms::all(),
            }]
        } else {
            self.vfs_mounts
        };

        // Create VFS directories in storage so they exist for direct access
        for mount in &vfs_mounts {
            if let Err(e) = vfs.mkdir_sync(&mount.guest_path) {
                tracing::warn!("Failed to create VFS directory {}: {}", mount.guest_path, e);
            }
        }

        // Get or create executor
        let executor = match self.executor {
            Some(e) => e,
            None => ComponentShellExecutor::embedded()?,
        };

        // Default limits
        let limits = self.limits.unwrap_or_default();

        // Retain the config so the shell can fork later.
        let config = ShellConfig {
            executor,
            vfs_mounts,
            real_mounts: self.real_mounts,
            tool_handler: self.tool_handler,
            component_registry: self.component_registry,
            limits: limits.clone(),
        };

        let instance = build_shell_instance(&config, &vfs).await?;

        Ok(Shell {
            instance,
            vfs,
            default_limits: limits,
            config,
        })
    }

    /// Build the shell (stub for when embedded-shell is disabled).
    #[cfg(not(feature = "embedded-shell"))]
    pub async fn build(self) -> Result<Shell, RuntimeError> {
        Err(RuntimeError::Wasm(
            "Shell::build requires embedded-shell feature".to_string(),
        ))
    }
}

/// A sandboxed shell for executing commands with persistent state.
///
/// The shell provides:
/// - A hybrid filesystem combining virtual storage with real filesystem mounts
/// - Persistent shell state (variables, functions, aliases) across executions
/// - Isolated execution environment per Shell instance
///
/// # Example
///
/// ```rust,ignore
/// let mut shell = Shell::builder()
///     .mount("/project", "/home/user/code", Mount::readonly())
///     .build()
///     .await?;
///
/// // Write to VFS
/// shell.vfs().write("/scratch/data.txt", b"hello").await?;
///
/// // State persists between execute calls
/// shell.execute("x=42", &limits).await?;
/// let result = shell.execute("echo $x", &limits).await?;
/// assert!(String::from_utf8_lossy(&result.stdout).contains("42"));
/// ```
#[cfg(feature = "embedded-shell")]
pub struct Shell {
    instance: ShellInstance<DynVfsStorage>,
    vfs: DynVfsStorage,
    default_limits: ResourceLimits,
    /// Build configuration retained so [`Shell::fork`] can spawn faithful copies.
    config: ShellConfig,
}

/// Build configuration retained on a [`Shell`] so it can be reconstructed for
/// forking (a fork needs the same executor, mounts, tool handler, and limits).
#[cfg(feature = "embedded-shell")]
#[derive(Clone)]
struct ShellConfig {
    executor: ComponentShellExecutor,
    vfs_mounts: Vec<VfsMount>,
    real_mounts: Vec<RealMount>,
    tool_handler: Option<Arc<dyn ToolHandler>>,
    component_registry: Option<Arc<crate::executor::ComponentRegistry>>,
    limits: ResourceLimits,
}

/// Build a fresh shell instance for `vfs` using a retained [`ShellConfig`].
/// Shared by [`ShellBuilder::build`] and [`Shell::fork`].
#[cfg(feature = "embedded-shell")]
async fn build_shell_instance(
    config: &ShellConfig,
    vfs: &DynVfsStorage,
) -> Result<ShellInstance<DynVfsStorage>, RuntimeError> {
    let mut hybrid_ctx = HybridVfsCtx::new(vfs.clone());
    for mount in &config.vfs_mounts {
        hybrid_ctx.add_vfs_preopen(&mount.guest_path, mount.dir_perms, mount.file_perms);
    }
    for mount in &config.real_mounts {
        let real_dir = RealDir::open_ambient(&mount.host_path, mount.dir_perms, mount.file_perms)
            .map_err(|e| {
            RuntimeError::Wasm(format!(
                "Failed to open mount {}: {}",
                mount.host_path.display(),
                e
            ))
        })?;
        hybrid_ctx.add_real_preopen(&mount.guest_path, real_dir);
    }
    match &config.component_registry {
        Some(registry) => {
            config
                .executor
                .create_instance_with_registry(
                    &config.limits,
                    hybrid_ctx,
                    config.tool_handler.clone(),
                    registry.clone(),
                )
                .await
        }
        None => {
            config
                .executor
                .create_instance(&config.limits, hybrid_ctx, config.tool_handler.clone())
                .await
        }
    }
}

#[cfg(feature = "embedded-shell")]
impl std::fmt::Debug for Shell {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Shell").finish_non_exhaustive()
    }
}

#[cfg(feature = "embedded-shell")]
impl Shell {
    /// Create a new shell builder with default settings.
    pub fn builder() -> ShellBuilder {
        ShellBuilder::new()
    }

    /// Get access to the VFS storage.
    ///
    /// This allows the orchestrator to read/write data that will be visible
    /// to commands executed in the shell.
    pub fn vfs(&self) -> &dyn VfsStorage {
        self.vfs.inner()
    }

    /// Get access to the VFS storage as an Arc.
    ///
    /// Useful when you need to share the VFS with other components.
    pub fn vfs_arc(&self) -> Arc<dyn VfsStorage> {
        self.vfs.arc()
    }

    /// Execute a shell script.
    ///
    /// The script has access to:
    /// - VFS paths configured via `vfs_path()` (backed by VfsStorage)
    /// - Real filesystem paths configured via `mount()` (backed by cap-std)
    ///
    /// **State persists** between execute calls. Variables, functions, and
    /// aliases defined in one call are available in subsequent calls.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// // Define a variable
    /// shell.execute("greeting=hello", &limits).await?;
    ///
    /// // Use it in the next call
    /// let result = shell.execute("echo $greeting world", &limits).await?;
    /// assert!(String::from_utf8_lossy(&result.stdout).contains("hello world"));
    /// ```
    pub async fn execute(
        &mut self,
        script: &str,
        limits: &ResourceLimits,
    ) -> Result<ExecutionResult, RuntimeError> {
        self.instance.execute(script, limits).await
    }

    /// Execute a shell script using the default limits.
    pub async fn execute_default(&mut self, script: &str) -> Result<ExecutionResult, RuntimeError> {
        self.instance.execute(script, &self.default_limits).await
    }

    /// Get a shell variable's value.
    ///
    /// Returns `None` if the variable is not set.
    pub async fn get_var(&mut self, name: &str) -> Result<Option<String>, RuntimeError> {
        self.instance.get_var(name).await
    }

    /// Set a shell variable.
    ///
    /// This is equivalent to running `name=value` in the shell.
    pub async fn set_var(&mut self, name: &str, value: &str) -> Result<(), RuntimeError> {
        self.instance.set_var(name, value).await
    }

    /// Get the exit code from the last executed command ($?).
    pub async fn last_exit_code(&mut self) -> Result<i32, RuntimeError> {
        self.instance.last_exit_code().await
    }

    /// Capture a snapshot of the current shell session.
    ///
    /// Captures the interpreter state (variables, functions, aliases, shell
    /// options, the working directory, traps, the directory stack) and, when the
    /// shell uses in-memory storage, the VFS contents. Snapshots can only be
    /// taken between `execute` calls, not while a script is running.
    ///
    /// Restore the snapshot into this (or another) shell with [`Self::restore`].
    pub async fn snapshot(&mut self) -> Result<Snapshot, RuntimeError> {
        let shell_state = self.instance.snapshot().await?;
        let vfs = self.snapshot_vfs().await?;
        Ok(Snapshot::new(shell_state, vfs))
    }

    /// Restore a previously captured [`Snapshot`].
    ///
    /// Replaces the interpreter state with the snapshot's (builtins are
    /// re-attached automatically) and, if the snapshot captured VFS contents,
    /// restores the in-memory filesystem in place. Returns an error if the
    /// snapshot is malformed or its format version is incompatible.
    pub async fn restore(&mut self, snapshot: &Snapshot) -> Result<(), RuntimeError> {
        self.instance.restore(&snapshot.shell_state).await?;
        if let Some(vfs_bytes) = &snapshot.vfs {
            let vsnap: eryx_vfs::InMemorySnapshot = rmp_serde::from_slice(vfs_bytes)
                .map_err(|e| RuntimeError::Snapshot(format!("malformed VFS snapshot: {e}")))?;
            self.in_memory_vfs()
                .ok_or_else(|| {
                    RuntimeError::Snapshot(
                        "snapshot has VFS contents but this shell's storage is not in-memory"
                            .to_string(),
                    )
                })?
                .restore(&vsnap)
                .await;
        }
        Ok(())
    }

    /// Fork this shell into an independent copy.
    ///
    /// The returned shell starts with a deep copy of this shell's interpreter
    /// state **and** VFS contents, but is fully isolated: subsequent changes to
    /// either shell (variables, files, ...) do not affect the other. The fork
    /// reuses the same executor, mounts, tool handler, and limits.
    ///
    /// Requires the shell to use in-memory VFS storage (the default); returns an
    /// error otherwise.
    pub async fn fork(&mut self) -> Result<Shell, RuntimeError> {
        let snapshot = self.snapshot().await?;

        // Build an independent VFS seeded from the snapshot.
        let vfs_bytes = snapshot.vfs.as_ref().ok_or_else(|| {
            RuntimeError::Snapshot("fork requires in-memory VFS storage".to_string())
        })?;
        let vsnap: eryx_vfs::InMemorySnapshot = rmp_serde::from_slice(vfs_bytes)
            .map_err(|e| RuntimeError::Snapshot(format!("malformed VFS snapshot: {e}")))?;
        let forked_vfs = DynVfsStorage::new(InMemoryStorage::from_snapshot(&vsnap));

        // Fresh instance with the same config, then replay the interpreter state.
        let mut instance = build_shell_instance(&self.config, &forked_vfs).await?;
        instance.restore(&snapshot.shell_state).await?;

        Ok(Shell {
            instance,
            vfs: forked_vfs,
            default_limits: self.default_limits.clone(),
            config: self.config.clone(),
        })
    }

    /// Serialize the VFS contents if the storage is in-memory.
    async fn snapshot_vfs(&self) -> Result<Option<Vec<u8>>, RuntimeError> {
        match self.in_memory_vfs() {
            Some(storage) => {
                let vsnap = storage.snapshot().await;
                let bytes = rmp_serde::to_vec_named(&vsnap)
                    .map_err(|e| RuntimeError::Snapshot(e.to_string()))?;
                Ok(Some(bytes))
            }
            None => Ok(None),
        }
    }

    /// Downcast the shell's VFS to `InMemoryStorage`, if that's what it is.
    fn in_memory_vfs(&self) -> Option<&InMemoryStorage> {
        self.vfs
            .inner()
            .as_any()
            .and_then(|a| a.downcast_ref::<InMemoryStorage>())
    }
}

/// Stub Shell type when embedded-shell feature is disabled.
#[cfg(not(feature = "embedded-shell"))]
pub struct Shell {
    _private: (),
}

#[cfg(not(feature = "embedded-shell"))]
impl Shell {
    /// Create a new shell builder with default settings.
    pub fn builder() -> ShellBuilder {
        ShellBuilder::new()
    }
}

#[cfg(test)]
#[cfg(feature = "embedded-shell")]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_shell_builder_default() {
        let shell = Shell::builder()
            .build()
            .await
            .expect("Failed to build shell");

        // Should have created successfully
        assert!(format!("{:?}", shell).contains("Shell"));
    }

    #[tokio::test]
    async fn test_shell_basic_execution() {
        let mut shell = Shell::builder()
            .build()
            .await
            .expect("Failed to build shell");
        let limits = ResourceLimits::default();

        let result = shell
            .execute("echo hello", &limits)
            .await
            .expect("Execute failed");

        assert_eq!(result.exit_code, 0);
        assert!(String::from_utf8_lossy(&result.stdout).contains("hello"));
    }

    #[tokio::test]
    async fn test_shell_variable_persistence() {
        let mut shell = Shell::builder()
            .build()
            .await
            .expect("Failed to build shell");
        let limits = ResourceLimits::default();

        // Set a variable
        let result = shell
            .execute("myvar=hello_world", &limits)
            .await
            .expect("Execute failed");
        assert_eq!(result.exit_code, 0);

        // Variable should persist to next call
        let result = shell
            .execute("echo $myvar", &limits)
            .await
            .expect("Execute failed");
        assert_eq!(result.exit_code, 0);
        let stdout = String::from_utf8_lossy(&result.stdout);
        assert!(
            stdout.contains("hello_world"),
            "Expected 'hello_world' in stdout: {:?}",
            stdout
        );
    }

    #[tokio::test]
    async fn test_snapshot_restore_same_shell() {
        let mut shell = Shell::builder().build().await.unwrap();
        let limits = ResourceLimits::default();

        shell.execute("x=42", &limits).await.unwrap();
        shell
            .execute(r#"greet() { echo "hi $1 $x"; }"#, &limits)
            .await
            .unwrap();

        let snap = shell.snapshot().await.unwrap();

        // Mutate state after the snapshot...
        shell.execute("x=999", &limits).await.unwrap();

        // ...then restore: x is back to 42 and the function is intact.
        shell.restore(&snap).await.unwrap();
        let result = shell.execute("greet world", &limits).await.unwrap();
        assert_eq!(result.exit_code, 0);
        let stdout = String::from_utf8_lossy(&result.stdout);
        assert!(stdout.contains("hi world 42"), "stdout: {stdout:?}");
    }

    #[tokio::test]
    async fn test_snapshot_restore_into_fresh_shell() {
        let limits = ResourceLimits::default();

        // Capture state in shell A, serialize to bytes.
        let mut a = Shell::builder().build().await.unwrap();
        a.execute("name=conch", &limits).await.unwrap();
        a.execute(r#"shout() { echo "${name}!"; }"#, &limits)
            .await
            .unwrap();
        let bytes = a.snapshot().await.unwrap().to_bytes().unwrap();

        // Restore into a brand-new shell B. Running `shout` here also confirms
        // builtins (echo) are re-attached after restore.
        let mut b = Shell::builder().build().await.unwrap();
        let snap = Snapshot::from_bytes(&bytes).unwrap();
        b.restore(&snap).await.unwrap();
        let result = b.execute("shout", &limits).await.unwrap();
        assert_eq!(result.exit_code, 0);
        assert_eq!(String::from_utf8_lossy(&result.stdout).trim(), "conch!");
    }

    #[tokio::test]
    async fn test_snapshot_from_bytes_rejects_garbage() {
        assert!(Snapshot::from_bytes(b"not a valid snapshot").is_err());
    }

    #[tokio::test]
    async fn test_snapshot_restore_includes_vfs() {
        let mut shell = Shell::builder().build().await.unwrap();
        let limits = ResourceLimits::default();

        // Create a file in the VFS via the shell.
        shell
            .execute("echo original > /scratch/note.txt", &limits)
            .await
            .unwrap();

        let snap = shell.snapshot().await.unwrap();
        assert!(snap.has_vfs(), "snapshot should capture in-memory VFS");

        // Mutate the file after the snapshot.
        shell
            .execute("echo changed > /scratch/note.txt", &limits)
            .await
            .unwrap();
        let changed = shell
            .execute("cat /scratch/note.txt", &limits)
            .await
            .unwrap();
        assert!(String::from_utf8_lossy(&changed.stdout).contains("changed"));

        // Restore: the file contents revert to the snapshot.
        shell.restore(&snap).await.unwrap();
        let restored = shell
            .execute("cat /scratch/note.txt", &limits)
            .await
            .unwrap();
        assert!(
            String::from_utf8_lossy(&restored.stdout).contains("original"),
            "stdout: {:?}",
            String::from_utf8_lossy(&restored.stdout)
        );
    }

    #[tokio::test]
    async fn test_fork_is_isolated() {
        let limits = ResourceLimits::default();

        let mut parent = Shell::builder().build().await.unwrap();
        parent.execute("shared=base", &limits).await.unwrap();
        parent
            .execute("echo base > /scratch/f.txt", &limits)
            .await
            .unwrap();

        // Fork inherits state + files.
        let mut child = parent.fork().await.unwrap();
        let inherited_var = child.execute("echo $shared", &limits).await.unwrap();
        assert!(String::from_utf8_lossy(&inherited_var.stdout).contains("base"));
        let inherited_file = child.execute("cat /scratch/f.txt", &limits).await.unwrap();
        assert!(String::from_utf8_lossy(&inherited_file.stdout).contains("base"));

        // Diverge both sides.
        child.execute("shared=child", &limits).await.unwrap();
        child
            .execute("echo child > /scratch/f.txt", &limits)
            .await
            .unwrap();
        parent.execute("shared=parent", &limits).await.unwrap();

        // Child's changes don't leak to parent...
        let p_var = parent.execute("echo $shared", &limits).await.unwrap();
        assert_eq!(String::from_utf8_lossy(&p_var.stdout).trim(), "parent");
        let p_file = parent.execute("cat /scratch/f.txt", &limits).await.unwrap();
        assert_eq!(String::from_utf8_lossy(&p_file.stdout).trim(), "base");

        // ...and parent's changes don't leak to child.
        let c_var = child.execute("echo $shared", &limits).await.unwrap();
        assert_eq!(String::from_utf8_lossy(&c_var.stdout).trim(), "child");
        let c_file = child.execute("cat /scratch/f.txt", &limits).await.unwrap();
        assert_eq!(String::from_utf8_lossy(&c_file.stdout).trim(), "child");
    }

    #[tokio::test]
    async fn test_snapshot_restore_vfs_into_fresh_shell() {
        let limits = ResourceLimits::default();

        // Capture a session (state + files) in shell A.
        let mut a = Shell::builder().build().await.unwrap();
        a.execute("greeting=hi", &limits).await.unwrap();
        a.execute("echo hello > /scratch/a.txt", &limits)
            .await
            .unwrap();
        let bytes = a.snapshot().await.unwrap().to_bytes().unwrap();

        // Restore the whole session into a brand-new shell B.
        let mut b = Shell::builder().build().await.unwrap();
        b.restore(&Snapshot::from_bytes(&bytes).unwrap())
            .await
            .unwrap();

        // Both the variable and the file came across.
        let var = b.execute("echo $greeting", &limits).await.unwrap();
        assert!(String::from_utf8_lossy(&var.stdout).contains("hi"));
        let file = b.execute("cat /scratch/a.txt", &limits).await.unwrap();
        assert!(String::from_utf8_lossy(&file.stdout).contains("hello"));
    }

    #[tokio::test]
    async fn test_shell_function_persistence() {
        let mut shell = Shell::builder()
            .build()
            .await
            .expect("Failed to build shell");
        let limits = ResourceLimits::default();

        // Define a function
        let result = shell
            .execute("greet() { echo \"Hello, $1!\"; }", &limits)
            .await
            .expect("Execute failed");
        assert_eq!(result.exit_code, 0);

        // Function should persist
        let result = shell
            .execute("greet World", &limits)
            .await
            .expect("Execute failed");
        assert_eq!(result.exit_code, 0);
        let stdout = String::from_utf8_lossy(&result.stdout);
        assert!(
            stdout.contains("Hello, World!"),
            "Expected greeting in stdout: {:?}",
            stdout
        );
    }

    #[tokio::test]
    async fn test_shell_get_set_var() {
        let mut shell = Shell::builder()
            .build()
            .await
            .expect("Failed to build shell");

        // Set via API
        shell
            .set_var("api_var", "api_value")
            .await
            .expect("set_var failed");

        // Get via API
        let value = shell.get_var("api_var").await.expect("get_var failed");
        assert_eq!(value, Some("api_value".to_string()));

        // Should be visible in shell
        let limits = ResourceLimits::default();
        let result = shell
            .execute("echo $api_var", &limits)
            .await
            .expect("Execute failed");
        let stdout = String::from_utf8_lossy(&result.stdout);
        assert!(stdout.contains("api_value"), "stdout: {:?}", stdout);
    }

    #[tokio::test]
    async fn test_shell_vfs_write_read() {
        let mut shell = Shell::builder()
            .build()
            .await
            .expect("Failed to build shell");
        let limits = ResourceLimits::default();

        // Write to VFS from host
        shell
            .vfs()
            .write("/scratch/test.txt", b"hello world")
            .await
            .expect("VFS write failed");

        // Verify the file exists in VFS
        let exists = shell
            .vfs()
            .exists("/scratch/test.txt")
            .await
            .expect("VFS exists check failed");
        assert!(exists, "File should exist in VFS after write");

        // Read via shell command
        let result = shell
            .execute("cat /scratch/test.txt", &limits)
            .await
            .expect("Execute failed");

        let stdout = String::from_utf8_lossy(&result.stdout);
        let stderr = String::from_utf8_lossy(&result.stderr);

        assert_eq!(
            result.exit_code, 0,
            "Expected exit 0, got {}. stdout: {}, stderr: {}",
            result.exit_code, stdout, stderr
        );
        assert!(
            stdout.contains("hello world"),
            "Expected stdout to contain 'hello world', got: '{}', stderr: '{}'",
            stdout,
            stderr
        );
    }

    #[tokio::test]
    async fn test_shell_isolation() {
        // Create two shells
        let mut shell1 = Shell::builder()
            .build()
            .await
            .expect("Failed to build shell1");
        let mut shell2 = Shell::builder()
            .build()
            .await
            .expect("Failed to build shell2");
        let limits = ResourceLimits::default();

        // Set different values in each shell
        shell1
            .execute("x=from_shell1", &limits)
            .await
            .expect("Execute failed");
        shell2
            .execute("x=from_shell2", &limits)
            .await
            .expect("Execute failed");

        // Each should see its own value
        let result1 = shell1
            .execute("echo $x", &limits)
            .await
            .expect("Execute failed");
        let stdout1 = String::from_utf8_lossy(&result1.stdout);
        assert!(
            stdout1.contains("from_shell1"),
            "shell1 should see its own value: {:?}",
            stdout1
        );

        let result2 = shell2
            .execute("echo $x", &limits)
            .await
            .expect("Execute failed");
        let stdout2 = String::from_utf8_lossy(&result2.stdout);
        assert!(
            stdout2.contains("from_shell2"),
            "shell2 should see its own value: {:?}",
            stdout2
        );
    }

    #[tokio::test]
    async fn test_shell_custom_vfs_path() {
        let mut shell = Shell::builder()
            .vfs_path("/data", DirPerms::all(), FilePerms::all())
            .vfs_path("/config", DirPerms::READ, FilePerms::READ)
            .build()
            .await
            .expect("Failed to build shell");

        // Write to /data (should work)
        shell
            .vfs()
            .write("/data/test.txt", b"test content")
            .await
            .expect("VFS write to /data failed");

        let limits = ResourceLimits::default();
        let result = shell
            .execute("cat /data/test.txt", &limits)
            .await
            .expect("Execute failed");
        assert_eq!(result.exit_code, 0);
        assert!(String::from_utf8_lossy(&result.stdout).contains("test content"));
    }

    #[tokio::test]
    async fn test_shell_with_custom_storage() {
        // Create shell with explicit storage
        let storage = InMemoryStorage::new();
        let mut shell = Shell::builder()
            .vfs(storage)
            .build()
            .await
            .expect("Failed to build shell");

        let limits = ResourceLimits::default();

        // Write and read
        shell
            .vfs()
            .write("/scratch/test.txt", b"custom storage")
            .await
            .expect("VFS write failed");

        let result = shell
            .execute("cat /scratch/test.txt", &limits)
            .await
            .expect("Execute failed");

        assert_eq!(result.exit_code, 0);
        assert!(String::from_utf8_lossy(&result.stdout).contains("custom storage"));
    }

    #[tokio::test]
    async fn test_shell_last_exit_code() {
        let mut shell = Shell::builder()
            .build()
            .await
            .expect("Failed to build shell");
        let limits = ResourceLimits::default();

        // Run successful command
        shell
            .execute("true", &limits)
            .await
            .expect("Execute failed");
        let code = shell.last_exit_code().await.expect("last_exit_code failed");
        assert_eq!(code, 0);

        // Run failing command
        shell
            .execute("false", &limits)
            .await
            .expect("Execute failed");
        let code = shell.last_exit_code().await.expect("last_exit_code failed");
        assert_eq!(code, 1);
    }
}
