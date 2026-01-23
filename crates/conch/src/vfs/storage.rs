//! VfsStorage adapter for ContextFs.
//!
//! This module bridges our `ContextFs` to eryx-vfs's `VfsStorage` trait,
//! enabling WASI filesystem shadowing.

use std::sync::Arc;
use std::time::SystemTime;

use async_trait::async_trait;
use eryx_vfs::{DirEntry, Metadata, VfsError, VfsStorage};
use tokio::sync::RwLock;

use super::context::{AccessPolicy, ContextFs, ContextProvider, FsError};

/// Adapter that implements `VfsStorage` for `ContextFs`.
///
/// This allows the shell to access agent context via standard WASI
/// filesystem operations like `open`, `read`, `readdir`, etc.
pub struct ContextStorage {
    /// The underlying context filesystem.
    context_fs: Arc<ContextFs>,
    /// In-memory scratch space for /tmp and other writable paths.
    scratch: RwLock<std::collections::HashMap<String, Vec<u8>>>,
}

impl std::fmt::Debug for ContextStorage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ContextStorage").finish_non_exhaustive()
    }
}

impl ContextStorage {
    /// Create a new context storage adapter.
    pub fn new(provider: Arc<dyn ContextProvider>, policy: AccessPolicy) -> Self {
        Self {
            context_fs: Arc::new(ContextFs::new(provider, policy)),
            scratch: RwLock::new(std::collections::HashMap::new()),
        }
    }

    /// Create from an existing ContextFs.
    pub fn from_context_fs(context_fs: Arc<ContextFs>) -> Self {
        Self {
            context_fs,
            scratch: RwLock::new(std::collections::HashMap::new()),
        }
    }

    /// Route a path to either context fs or scratch space.
    fn route_path(&self, path: &str) -> PathRoute {
        let path = path.trim_start_matches('/');
        if path.starts_with("ctx/") || path == "ctx" {
            PathRoute::Context(format!("/{}", path))
        } else if path.starts_with("tmp/") || path == "tmp" {
            PathRoute::Scratch(path.to_string())
        } else {
            // Default: treat as context path
            PathRoute::Context(format!("/ctx/{}", path))
        }
    }
}

enum PathRoute {
    Context(String),
    Scratch(String),
}

/// Convert our FsError to eryx-vfs VfsError.
fn to_vfs_error(err: FsError) -> VfsError {
    match err {
        FsError::NotFound(p) => VfsError::NotFound(p),
        FsError::PermissionDenied(msg) => VfsError::PermissionDenied(msg),
        FsError::NotADirectory(p) => VfsError::NotDirectory(p),
        FsError::NotAFile(p) => VfsError::NotFile(p),
        FsError::ReadOnly => VfsError::PermissionDenied("read-only filesystem".to_string()),
        FsError::InvalidPath(p) => VfsError::InvalidPath(p),
        FsError::Io(e) => VfsError::Io(e.to_string()),
        FsError::Provider(msg) => VfsError::Storage(msg),
    }
}

#[async_trait]
impl VfsStorage for ContextStorage {
    async fn read(&self, path: &str) -> Result<Vec<u8>, VfsError> {
        match self.route_path(path) {
            PathRoute::Context(p) => self.context_fs.read(&p).await.map_err(to_vfs_error),
            PathRoute::Scratch(p) => {
                let scratch = self.scratch.read().await;
                scratch
                    .get(&p)
                    .cloned()
                    .ok_or_else(|| VfsError::NotFound(p))
            }
        }
    }

    async fn read_at(&self, path: &str, offset: u64, len: u64) -> Result<Vec<u8>, VfsError> {
        let data = self.read(path).await?;
        let offset = offset as usize;
        let len = len as usize;
        if offset >= data.len() {
            return Ok(Vec::new());
        }
        let end = (offset + len).min(data.len());
        Ok(data[offset..end].to_vec())
    }

    async fn write(&self, path: &str, data: &[u8]) -> Result<(), VfsError> {
        match self.route_path(path) {
            PathRoute::Context(p) => {
                // Context paths under /ctx/self/scratch are writable
                if p.starts_with("/ctx/self/scratch/") {
                    let scratch_path = p.strip_prefix("/ctx/self/scratch/").unwrap_or(&p);
                    // Delegate to context provider's scratch space
                    // For now, use our local scratch as fallback
                    let mut scratch = self.scratch.write().await;
                    scratch.insert(format!("ctx/self/scratch/{}", scratch_path), data.to_vec());
                    Ok(())
                } else {
                    Err(VfsError::PermissionDenied(
                        "read-only filesystem".to_string(),
                    ))
                }
            }
            PathRoute::Scratch(p) => {
                let mut scratch = self.scratch.write().await;
                scratch.insert(p, data.to_vec());
                Ok(())
            }
        }
    }

    async fn write_at(&self, path: &str, offset: u64, data: &[u8]) -> Result<(), VfsError> {
        // Read existing, modify, write back
        let mut existing = self.read(path).await.unwrap_or_default();
        let offset = offset as usize;
        if offset > existing.len() {
            existing.resize(offset, 0);
        }
        let end = offset + data.len();
        if end > existing.len() {
            existing.resize(end, 0);
        }
        existing[offset..end].copy_from_slice(data);
        self.write(path, &existing).await
    }

    async fn set_size(&self, path: &str, size: u64) -> Result<(), VfsError> {
        let mut data = self.read(path).await.unwrap_or_default();
        data.resize(size as usize, 0);
        self.write(path, &data).await
    }

    async fn delete(&self, path: &str) -> Result<(), VfsError> {
        match self.route_path(path) {
            PathRoute::Context(_) => Err(VfsError::PermissionDenied(
                "read-only filesystem".to_string(),
            )),
            PathRoute::Scratch(p) => {
                let mut scratch = self.scratch.write().await;
                scratch.remove(&p);
                Ok(())
            }
        }
    }

    async fn exists(&self, path: &str) -> Result<bool, VfsError> {
        match self.route_path(path) {
            PathRoute::Context(p) => self.context_fs.exists(&p).await.map_err(to_vfs_error),
            PathRoute::Scratch(p) => {
                let scratch = self.scratch.read().await;
                Ok(scratch.contains_key(&p) || p == "tmp")
            }
        }
    }

    async fn list(&self, path: &str) -> Result<Vec<DirEntry>, VfsError> {
        match self.route_path(path) {
            PathRoute::Context(p) => {
                let entries = self.context_fs.read_dir(&p).await.map_err(to_vfs_error)?;
                Ok(entries
                    .into_iter()
                    .map(|e| {
                        let now = SystemTime::now();
                        DirEntry {
                            name: e.name,
                            metadata: Metadata {
                                is_dir: e.is_dir,
                                size: 0, // We don't track size for directory listings
                                created: now,
                                modified: now,
                                accessed: now,
                            },
                        }
                    })
                    .collect())
            }
            PathRoute::Scratch(p) => {
                let scratch = self.scratch.read().await;
                let prefix = if p.is_empty() || p == "tmp" {
                    "tmp/".to_string()
                } else {
                    format!("{}/", p.trim_end_matches('/'))
                };

                let mut entries = Vec::new();
                let mut seen_dirs = std::collections::HashSet::new();

                for key in scratch.keys() {
                    if let Some(rest) = key.strip_prefix(&prefix) {
                        // Get the first component after the prefix
                        let name = rest.split('/').next().unwrap_or(rest);
                        let is_dir = rest.contains('/');

                        if is_dir {
                            if seen_dirs.insert(name.to_string()) {
                                entries.push(DirEntry {
                                    name: name.to_string(),
                                    metadata: Metadata::default(),
                                });
                            }
                        } else {
                            let data = scratch.get(key).map(|v| v.len()).unwrap_or(0);
                            entries.push(DirEntry {
                                name: name.to_string(),
                                metadata: Metadata {
                                    is_dir: false,
                                    size: data as u64,
                                    ..Metadata::default()
                                },
                            });
                        }
                    }
                }
                Ok(entries)
            }
        }
    }

    async fn stat(&self, path: &str) -> Result<Metadata, VfsError> {
        match self.route_path(path) {
            PathRoute::Context(p) => {
                let meta = self.context_fs.stat(&p).await.map_err(to_vfs_error)?;
                let now = SystemTime::now();
                Ok(Metadata {
                    is_dir: meta.is_dir,
                    size: meta.size,
                    created: now,
                    modified: meta
                        .modified
                        .map(|t| SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(t))
                        .unwrap_or(now),
                    accessed: now,
                })
            }
            PathRoute::Scratch(p) => {
                let scratch = self.scratch.read().await;
                if let Some(data) = scratch.get(&p) {
                    Ok(Metadata {
                        is_dir: false,
                        size: data.len() as u64,
                        ..Metadata::default()
                    })
                } else if p == "tmp" || scratch.keys().any(|k| k.starts_with(&format!("{}/", p))) {
                    Ok(Metadata {
                        is_dir: true,
                        ..Metadata::default()
                    })
                } else {
                    Err(VfsError::NotFound(p))
                }
            }
        }
    }

    async fn mkdir(&self, path: &str) -> Result<(), VfsError> {
        match self.route_path(path) {
            PathRoute::Context(_) => Err(VfsError::PermissionDenied(
                "read-only filesystem".to_string(),
            )),
            PathRoute::Scratch(_) => {
                // Directories are implicit in our scratch space
                Ok(())
            }
        }
    }

    async fn rmdir(&self, path: &str) -> Result<(), VfsError> {
        match self.route_path(path) {
            PathRoute::Context(_) => Err(VfsError::PermissionDenied(
                "read-only filesystem".to_string(),
            )),
            PathRoute::Scratch(p) => {
                let scratch = self.scratch.write().await;
                let prefix = format!("{}/", p.trim_end_matches('/'));
                let to_remove: Vec<_> = scratch
                    .keys()
                    .filter(|k| k.starts_with(&prefix))
                    .cloned()
                    .collect();
                if !to_remove.is_empty() {
                    return Err(VfsError::DirectoryNotEmpty(p));
                }
                Ok(())
            }
        }
    }

    async fn rename(&self, from: &str, to: &str) -> Result<(), VfsError> {
        // Only allow rename within scratch space
        match (self.route_path(from), self.route_path(to)) {
            (PathRoute::Scratch(from_p), PathRoute::Scratch(to_p)) => {
                let mut scratch = self.scratch.write().await;
                if let Some(data) = scratch.remove(&from_p) {
                    scratch.insert(to_p, data);
                    Ok(())
                } else {
                    Err(VfsError::NotFound(from_p))
                }
            }
            _ => Err(VfsError::PermissionDenied(
                "read-only filesystem".to_string(),
            )),
        }
    }

    fn mkdir_sync(&self, _path: &str) -> Result<(), VfsError> {
        // Directories are implicit
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vfs::context::MockContextProvider;

    #[tokio::test]
    async fn test_scratch_read_write() {
        let provider = Arc::new(MockContextProvider::new());
        let policy = AccessPolicy::default();
        let storage = ContextStorage::new(provider, policy);

        // Write to scratch
        storage.write("/tmp/test.txt", b"hello").await.unwrap();

        // Read back
        let data = storage.read("/tmp/test.txt").await.unwrap();
        assert_eq!(data, b"hello");
    }

    #[tokio::test]
    async fn test_context_path_routing() {
        let provider = Arc::new(MockContextProvider::new());
        let policy = AccessPolicy::default();
        let storage = ContextStorage::new(provider, policy);

        // Context paths should route to ContextFs
        assert!(storage.exists("/ctx/self").await.unwrap());
        assert!(storage.exists("/ctx/self/tools").await.unwrap());
    }

    #[tokio::test]
    async fn test_scratch_list() {
        let provider = Arc::new(MockContextProvider::new());
        let policy = AccessPolicy::default();
        let storage = ContextStorage::new(provider, policy);

        storage.write("/tmp/a.txt", b"a").await.unwrap();
        storage.write("/tmp/b.txt", b"b").await.unwrap();

        let entries = storage.list("/tmp").await.unwrap();
        assert_eq!(entries.len(), 2);
    }
}
