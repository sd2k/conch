//! Shell API for executing commands in a sandboxed environment.
//!
//! The [`Shell`] provides a high-level interface for executing shell commands
//! in a WASM sandbox with a hybrid virtual filesystem that combines:
//!
//! - **VFS storage**: In-memory or custom storage for orchestrator-controlled paths
//! - **Real filesystem**: cap-std secured mounts for host directory access
//!
//! # Example
//!
//! ```rust,ignore
//! use conch::{Shell, Mount, ResourceLimits};
//!
//! // Create a shell with a real filesystem mount
//! let shell = Shell::builder()
//!     .mount("/project", "/home/user/code", Mount::readonly())
//!     .build()?;
//!
//! // Write data to VFS scratch area
//! shell.vfs().write("/scratch/input.txt", b"hello").await?;
//!
//! // Execute commands - they see both VFS and real filesystem
//! let result = shell.execute(
//!     "cat /scratch/input.txt && ls /project/src",
//!     &ResourceLimits::default(),
//! ).await?;
//! ```

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use eryx_vfs::{
    DirEntry, DirPerms, FilePerms, HybridVfsCtx, InMemoryStorage, Metadata, RealDir, VfsResult,
    VfsStorage,
};

use crate::executor::ComponentShellExecutor;
use crate::limits::ResourceLimits;
use crate::runtime::{ExecutionResult, RuntimeError};

/// A sized wrapper around `Arc<dyn VfsStorage>` that implements `VfsStorage`.
///
/// This allows us to use dynamic dispatch for VFS storage while still being
/// compatible with APIs that require `Sized` types (like `HybridVfsCtx<S>`).
#[derive(Clone)]
pub struct DynVfsStorage(Arc<dyn VfsStorage>);

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
/// let shell = Shell::builder()
///     .vfs(my_custom_storage)
///     .mount("/project", "/home/user/code", Mount::readonly())
///     .mount("/output", "/tmp/agent-output", Mount::readwrite())
///     .vfs_path("/data", DirPerms::READ, FilePerms::READ)
///     .build()?;
/// ```
pub struct ShellBuilder {
    vfs: Option<DynVfsStorage>,
    vfs_mounts: Vec<VfsMount>,
    real_mounts: Vec<RealMount>,
    executor: Option<ComponentShellExecutor>,
}

impl std::fmt::Debug for ShellBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ShellBuilder")
            .field("has_vfs", &self.vfs.is_some())
            .field("vfs_mounts", &self.vfs_mounts)
            .field("real_mounts", &self.real_mounts)
            .field("has_executor", &self.executor.is_some())
            .finish()
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

    /// Build the shell with the configured settings.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - A real filesystem mount path doesn't exist or can't be opened
    /// - The executor fails to initialize
    pub fn build(self) -> Result<Shell, RuntimeError> {
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
            None => {
                #[cfg(feature = "embedded-shell")]
                {
                    ComponentShellExecutor::embedded()?
                }
                #[cfg(not(feature = "embedded-shell"))]
                {
                    return Err(RuntimeError::Wasm(
                        "No executor provided and embedded-shell feature not enabled".to_string(),
                    ));
                }
            }
        };

        Ok(Shell {
            executor,
            vfs,
            vfs_mounts,
            real_mounts: self.real_mounts,
        })
    }
}

/// A sandboxed shell for executing commands.
///
/// The shell provides a hybrid filesystem that combines virtual storage (for
/// orchestrator-controlled data) with optional real filesystem mounts (for
/// host directory access via cap-std).
///
/// # Example
///
/// ```rust,ignore
/// let shell = Shell::builder()
///     .mount("/project", "/home/user/code", Mount::readonly())
///     .build()?;
///
/// // Write to VFS
/// shell.vfs().write("/scratch/data.txt", b"hello").await?;
///
/// // Execute commands
/// let result = shell.execute("cat /scratch/data.txt", &limits).await?;
/// ```
pub struct Shell {
    executor: ComponentShellExecutor,
    vfs: DynVfsStorage,
    vfs_mounts: Vec<VfsMount>,
    real_mounts: Vec<RealMount>,
}

impl std::fmt::Debug for Shell {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Shell")
            .field("vfs_mounts", &self.vfs_mounts)
            .field("real_mounts", &self.real_mounts)
            .finish_non_exhaustive()
    }
}

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
        Arc::clone(&self.vfs.0)
    }

    /// Execute a shell script.
    ///
    /// The script has access to:
    /// - VFS paths configured via `vfs_path()` (backed by VfsStorage)
    /// - Real filesystem paths configured via `mount()` (backed by cap-std)
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let result = shell.execute("echo hello | wc -c", &limits).await?;
    /// assert_eq!(result.exit_code, 0);
    /// println!("stdout: {}", String::from_utf8_lossy(&result.stdout));
    /// ```
    pub async fn execute(
        &self,
        script: &str,
        limits: &ResourceLimits,
    ) -> Result<ExecutionResult, RuntimeError> {
        // Build HybridVfsCtx for this execution
        // Wrap DynVfsStorage in Arc for HybridVfsCtx (DynVfsStorage is Sized + VfsStorage)
        let mut hybrid_ctx = HybridVfsCtx::new(Arc::new(self.vfs.clone()));

        // Add VFS mounts
        for mount in &self.vfs_mounts {
            hybrid_ctx.add_vfs_preopen(&mount.guest_path, mount.dir_perms, mount.file_perms);
        }

        // Add real filesystem mounts
        for mount in &self.real_mounts {
            let real_dir =
                RealDir::open_ambient(&mount.host_path, mount.dir_perms, mount.file_perms)
                    .map_err(|e| {
                        RuntimeError::Wasm(format!(
                            "Failed to open mount {}: {}",
                            mount.host_path.display(),
                            e
                        ))
                    })?;
            hybrid_ctx.add_real_preopen(&mount.guest_path, real_dir);
        }

        // Execute with hybrid VFS
        self.executor
            .execute_with_hybrid_vfs(script, limits, hybrid_ctx)
            .await
    }
}

#[cfg(test)]
#[cfg(feature = "embedded-shell")]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_shell_builder_default() {
        let shell = Shell::builder().build().expect("Failed to build shell");

        // Should have default /scratch mount
        assert_eq!(shell.vfs_mounts.len(), 1);
        assert_eq!(shell.vfs_mounts[0].guest_path, "/scratch");
    }

    #[tokio::test]
    async fn test_shell_basic_execution() {
        let shell = Shell::builder().build().expect("Failed to build shell");
        let limits = ResourceLimits::default();

        let result = shell
            .execute("echo hello", &limits)
            .await
            .expect("Execute failed");

        assert_eq!(result.exit_code, 0);
        assert!(String::from_utf8_lossy(&result.stdout).contains("hello"));
    }

    #[tokio::test]
    async fn test_shell_vfs_write_read() {
        let shell = Shell::builder().build().expect("Failed to build shell");
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

        // Read via shell command (the file we wrote from host)
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
    async fn test_shell_custom_vfs_path() {
        let shell = Shell::builder()
            .vfs_path("/data", DirPerms::all(), FilePerms::all())
            .vfs_path("/config", DirPerms::READ, FilePerms::READ)
            .build()
            .expect("Failed to build shell");

        // Should have the custom mounts (not the default /scratch)
        assert_eq!(shell.vfs_mounts.len(), 2);
        assert_eq!(shell.vfs_mounts[0].guest_path, "/data");
        assert_eq!(shell.vfs_mounts[1].guest_path, "/config");
    }

    #[tokio::test]
    async fn test_shell_with_custom_storage() {
        // Create shell with explicit storage
        let storage = InMemoryStorage::new();
        let shell = Shell::builder()
            .vfs(storage)
            .build()
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
}
