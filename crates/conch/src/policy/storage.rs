//! Policy-enforcing VFS storage wrapper.

use std::sync::Arc;

use async_trait::async_trait;
use eryx_vfs::{DirEntry, Metadata, VfsError, VfsResult, VfsStorage};

#[cfg(test)]
use super::handler::CommandInfo;
use super::handler::{CommandTracker, Operation, PolicyDecision, PolicyHandler};

/// A VFS storage wrapper that enforces policy on all operations.
///
/// This wraps any `VfsStorage` implementation and checks all operations
/// against a `PolicyHandler` before allowing them to proceed.
///
/// ## Thread Safety
///
/// The `PolicyStorage` is `Send + Sync` and can be shared across threads.
/// The inner storage and policy handler must also be `Send + Sync`.
///
/// ## Example
///
/// ```rust,ignore
/// use conch::policy::{PolicyStorage, PolicyBuilder};
/// use eryx_vfs::InMemoryStorage;
/// use std::sync::Arc;
///
/// let storage = Arc::new(InMemoryStorage::new());
/// let policy = PolicyBuilder::new()
///     .allow_read("/agent/**")
///     .allow_write("/agent/scratch/**")
///     .build();
///
/// let policy_storage = PolicyStorage::new(storage, Arc::new(policy));
///
/// // This will succeed (allowed by policy)
/// policy_storage.read("/agent/params.json").await?;
///
/// // This will fail with PermissionDenied (not allowed by policy)
/// policy_storage.write("/agent/params.json", b"data").await?;
/// ```
pub struct PolicyStorage<S: VfsStorage + ?Sized, P: PolicyHandler + ?Sized> {
    inner: Arc<S>,
    policy: Arc<P>,
    command_tracker: Arc<CommandTracker>,
}

impl<S: VfsStorage + ?Sized, P: PolicyHandler + ?Sized> PolicyStorage<S, P> {
    /// Create a new policy storage wrapper.
    pub fn new(storage: Arc<S>, policy: Arc<P>) -> Self {
        Self {
            inner: storage,
            policy,
            command_tracker: Arc::new(CommandTracker::new()),
        }
    }

    /// Create a new policy storage wrapper with a command tracker.
    ///
    /// Use this when you want to share a command tracker with other components
    /// (e.g., the shell executor that sets the current command).
    pub fn with_command_tracker(
        storage: Arc<S>,
        policy: Arc<P>,
        command_tracker: Arc<CommandTracker>,
    ) -> Self {
        Self {
            inner: storage,
            policy,
            command_tracker,
        }
    }

    /// Get a reference to the command tracker.
    ///
    /// Use this to set the current command context before filesystem operations.
    pub fn command_tracker(&self) -> &Arc<CommandTracker> {
        &self.command_tracker
    }

    /// Get a reference to the inner storage.
    pub fn inner(&self) -> &Arc<S> {
        &self.inner
    }

    /// Get a reference to the policy handler.
    pub fn policy(&self) -> &Arc<P> {
        &self.policy
    }

    /// Check if an operation is allowed by the policy.
    fn check(&self, path: &str, operation: Operation) -> VfsResult<()> {
        let command = self.command_tracker.current();
        let decision = self.policy.check_access(path, operation, command.as_ref());

        match decision {
            PolicyDecision::Allow => Ok(()),
            PolicyDecision::Deny(reason) => {
                tracing::debug!(
                    path = %path,
                    operation = ?operation,
                    command = ?command,
                    reason = %reason,
                    "policy denied access"
                );
                Err(VfsError::PermissionDenied(reason))
            }
        }
    }
}

// Implement Clone manually since we need Arc
impl<S: VfsStorage + ?Sized, P: PolicyHandler + ?Sized> Clone for PolicyStorage<S, P> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
            policy: Arc::clone(&self.policy),
            command_tracker: Arc::clone(&self.command_tracker),
        }
    }
}

impl<S: VfsStorage + ?Sized, P: PolicyHandler + ?Sized> std::fmt::Debug for PolicyStorage<S, P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PolicyStorage")
            .field("command_tracker", &self.command_tracker)
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl<S: VfsStorage + ?Sized + 'static, P: PolicyHandler + ?Sized + 'static> VfsStorage
    for PolicyStorage<S, P>
{
    async fn read(&self, path: &str) -> VfsResult<Vec<u8>> {
        self.check(path, Operation::Read)?;
        self.inner.read(path).await
    }

    async fn read_at(&self, path: &str, offset: u64, len: u64) -> VfsResult<Vec<u8>> {
        self.check(path, Operation::Read)?;
        self.inner.read_at(path, offset, len).await
    }

    async fn write(&self, path: &str, data: &[u8]) -> VfsResult<()> {
        self.check(path, Operation::Write)?;
        self.inner.write(path, data).await
    }

    async fn write_at(&self, path: &str, offset: u64, data: &[u8]) -> VfsResult<()> {
        self.check(path, Operation::Write)?;
        self.inner.write_at(path, offset, data).await
    }

    async fn set_size(&self, path: &str, size: u64) -> VfsResult<()> {
        self.check(path, Operation::Write)?;
        self.inner.set_size(path, size).await
    }

    async fn delete(&self, path: &str) -> VfsResult<()> {
        self.check(path, Operation::Delete)?;
        self.inner.delete(path).await
    }

    async fn exists(&self, path: &str) -> VfsResult<bool> {
        // exists() is a read-like operation
        self.check(path, Operation::Stat)?;
        self.inner.exists(path).await
    }

    async fn list(&self, path: &str) -> VfsResult<Vec<DirEntry>> {
        self.check(path, Operation::List)?;
        self.inner.list(path).await
    }

    async fn stat(&self, path: &str) -> VfsResult<Metadata> {
        self.check(path, Operation::Stat)?;
        self.inner.stat(path).await
    }

    async fn mkdir(&self, path: &str) -> VfsResult<()> {
        self.check(path, Operation::Mkdir)?;
        self.inner.mkdir(path).await
    }

    async fn rmdir(&self, path: &str) -> VfsResult<()> {
        self.check(path, Operation::Rmdir)?;
        self.inner.rmdir(path).await
    }

    async fn rename(&self, from: &str, to: &str) -> VfsResult<()> {
        // Check both source and destination
        self.check(from, Operation::Rename)?;
        self.check(to, Operation::Rename)?;
        self.inner.rename(from, to).await
    }

    fn mkdir_sync(&self, path: &str) -> VfsResult<()> {
        // Sync operations also need policy checks
        let command = self.command_tracker.current();
        let decision = self
            .policy
            .check_access(path, Operation::Mkdir, command.as_ref());

        match decision {
            PolicyDecision::Allow => self.inner.mkdir_sync(path),
            PolicyDecision::Deny(reason) => Err(VfsError::PermissionDenied(reason)),
        }
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::policy::{Policy, PolicyBuilder};
    use eryx_vfs::InMemoryStorage;

    async fn setup_test_storage() -> (Arc<InMemoryStorage>, PolicyStorage<InMemoryStorage, Policy>)
    {
        let storage = Arc::new(InMemoryStorage::new());

        // Pre-create directories
        storage.mkdir("/agent").await.unwrap();
        storage.mkdir("/agent/scratch").await.unwrap();
        storage.mkdir("/tools").await.unwrap();
        storage.mkdir("/other").await.unwrap();

        // Write some test files
        storage.write("/agent/params.json", b"{}").await.unwrap();
        storage.write("/tools/index.json", b"[]").await.unwrap();
        storage.write("/other/secret.txt", b"secret").await.unwrap();

        let policy = PolicyBuilder::new()
            .allow_read("/agent/**")
            .allow_read("/tools/**")
            .allow_write("/agent/scratch/**")
            .build();

        let policy_storage = PolicyStorage::new(Arc::clone(&storage), Arc::new(policy));

        (storage, policy_storage)
    }

    #[tokio::test]
    async fn test_allowed_read() {
        let (_storage, policy_storage) = setup_test_storage().await;

        // Should be able to read from /agent
        let result = policy_storage.read("/agent/params.json").await;
        assert!(result.is_ok());

        // Should be able to read from /tools
        let result = policy_storage.read("/tools/index.json").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_denied_read() {
        let (_storage, policy_storage) = setup_test_storage().await;

        // Should NOT be able to read from /other
        let result = policy_storage.read("/other/secret.txt").await;
        assert!(result.is_err());
        match result {
            Err(VfsError::PermissionDenied(_)) => {}
            other => panic!("expected PermissionDenied, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_allowed_write() {
        let (_storage, policy_storage) = setup_test_storage().await;

        // Should be able to write to /agent/scratch
        let result = policy_storage
            .write("/agent/scratch/output.txt", b"data")
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_denied_write() {
        let (_storage, policy_storage) = setup_test_storage().await;

        // Should NOT be able to write to /agent (outside scratch)
        let result = policy_storage
            .write("/agent/params.json", b"modified")
            .await;
        assert!(result.is_err());
        match result {
            Err(VfsError::PermissionDenied(_)) => {}
            other => panic!("expected PermissionDenied, got {:?}", other),
        }

        // Should NOT be able to write to /tools
        let result = policy_storage.write("/tools/new.json", b"{}").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_command_context() {
        let (_storage, policy_storage) = setup_test_storage().await;

        // Set command context
        policy_storage
            .command_tracker()
            .begin_command(CommandInfo::new(
                "cat",
                vec!["/agent/params.json".to_string()],
            ));

        // Read should still work with command context
        let result = policy_storage.read("/agent/params.json").await;
        assert!(result.is_ok());

        policy_storage.command_tracker().end_command();
    }

    #[tokio::test]
    async fn test_list_allowed() {
        let (_storage, policy_storage) = setup_test_storage().await;

        // Should be able to list /agent
        let result = policy_storage.list("/agent").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_list_denied() {
        let (_storage, policy_storage) = setup_test_storage().await;

        // Should NOT be able to list /other
        let result = policy_storage.list("/other").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_stat_allowed() {
        let (_storage, policy_storage) = setup_test_storage().await;

        // Should be able to stat /agent
        let result = policy_storage.stat("/agent/params.json").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_mkdir_allowed() {
        let (_storage, policy_storage) = setup_test_storage().await;

        // Should be able to mkdir in /agent/scratch
        let result = policy_storage.mkdir("/agent/scratch/subdir").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_mkdir_denied() {
        let (_storage, policy_storage) = setup_test_storage().await;

        // Should NOT be able to mkdir in /agent (outside scratch)
        let result = policy_storage.mkdir("/agent/newdir").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_delete_denied() {
        let (_storage, policy_storage) = setup_test_storage().await;

        // Should NOT be able to delete from /agent
        let result = policy_storage.delete("/agent/params.json").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_rename_needs_both_paths() {
        let (storage, policy_storage) = setup_test_storage().await;

        // Create a file in scratch
        storage
            .write("/agent/scratch/source.txt", b"data")
            .await
            .unwrap();

        // Should be able to rename within scratch
        let result = policy_storage
            .rename("/agent/scratch/source.txt", "/agent/scratch/dest.txt")
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_rename_denied_destination() {
        let (storage, policy_storage) = setup_test_storage().await;

        // Create a file in scratch
        storage
            .write("/agent/scratch/source.txt", b"data")
            .await
            .unwrap();

        // Should NOT be able to rename from scratch to /agent
        let result = policy_storage
            .rename("/agent/scratch/source.txt", "/agent/moved.txt")
            .await;
        assert!(result.is_err());
    }
}
