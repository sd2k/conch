//! Remote VFS storage backed by gRPC stream.
//!
//! This implements `VfsStorage` by sending requests to the client over a
//! bidirectional gRPC stream and waiting for responses.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::SystemTime;

use async_trait::async_trait;
use eryx_vfs::{DirEntry, Metadata, VfsError, VfsResult, VfsStorage};
use tokio::sync::{Mutex, mpsc, oneshot};

use crate::proto::{
    self, ClientMessage, ServerMessage, VfsErrorCode, client_message::Msg as ClientMsg,
    server_message::Msg as ServerMsg,
};

/// A pending VFS request waiting for a response.
struct PendingRequest {
    sender: oneshot::Sender<VfsResponse>,
}

impl std::fmt::Debug for PendingRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PendingRequest").finish_non_exhaustive()
    }
}

/// The response to a VFS request.
enum VfsResponse {
    Read(VfsResult<Vec<u8>>),
    Write(VfsResult<()>),
    Stat(VfsResult<Metadata>),
    List(VfsResult<Vec<DirEntry>>),
    Delete(VfsResult<()>),
    Mkdir(VfsResult<()>),
    Rename(VfsResult<()>),
}

/// Remote storage that delegates VFS operations to the gRPC client.
///
/// This is used by the sandbox to perform filesystem operations. Each operation
/// sends a request message to the client and waits for a response.
#[derive(Debug)]
pub struct RemoteStorage {
    /// Channel to send server messages to the client
    tx: mpsc::Sender<ServerMessage>,
    /// Pending requests awaiting responses, keyed by request_id
    pending: Arc<Mutex<HashMap<u64, PendingRequest>>>,
    /// Counter for generating unique request IDs
    next_request_id: AtomicU64,
}

impl RemoteStorage {
    /// Create a new remote storage.
    ///
    /// # Arguments
    /// * `tx` - Channel to send server messages to the client
    pub fn new(tx: mpsc::Sender<ServerMessage>) -> Self {
        Self {
            tx,
            pending: Arc::new(Mutex::new(HashMap::new())),
            next_request_id: AtomicU64::new(1),
        }
    }

    /// Get a handle for processing client responses.
    ///
    /// The returned handle should be used to dispatch incoming `ClientMessage`s
    /// to their corresponding pending requests.
    pub fn response_handler(&self) -> ResponseHandler {
        ResponseHandler {
            pending: Arc::clone(&self.pending),
        }
    }

    /// Generate a unique request ID.
    fn next_id(&self) -> u64 {
        self.next_request_id.fetch_add(1, Ordering::Relaxed)
    }

    /// Send a request and wait for the response.
    async fn request<F>(&self, handler: F) -> VfsResult<VfsResponse>
    where
        F: FnOnce(u64) -> ServerMsg,
    {
        let request_id = self.next_id();
        let (tx, rx) = oneshot::channel();

        // Register the pending request
        {
            let mut pending = self.pending.lock().await;
            pending.insert(request_id, PendingRequest { sender: tx });
        }

        // Send the request
        let msg = ServerMessage {
            msg: Some(handler(request_id)),
        };
        self.tx
            .send(msg)
            .await
            .map_err(|_| VfsError::Storage("gRPC stream closed".to_string()))?;

        // Wait for the response
        rx.await
            .map_err(|_| VfsError::Storage("response channel closed".to_string()))
    }
}

/// Handle for dispatching client responses to pending requests.
#[derive(Debug)]
pub struct ResponseHandler {
    pending: Arc<Mutex<HashMap<u64, PendingRequest>>>,
}

impl ResponseHandler {
    /// Dispatch a client message to its pending request.
    ///
    /// Returns `true` if the message was handled, `false` if it wasn't a VFS response.
    pub async fn handle(&self, msg: &ClientMessage) -> bool {
        let Some(ref client_msg) = msg.msg else {
            return false;
        };

        match client_msg {
            ClientMsg::VfsReadResponse(resp) => {
                self.complete(
                    resp.request_id,
                    VfsResponse::Read(convert_read_response(resp)),
                )
                .await;
                true
            }
            ClientMsg::VfsWriteResponse(resp) => {
                self.complete(
                    resp.request_id,
                    VfsResponse::Write(convert_write_response(resp)),
                )
                .await;
                true
            }
            ClientMsg::VfsStatResponse(resp) => {
                self.complete(
                    resp.request_id,
                    VfsResponse::Stat(convert_stat_response(resp)),
                )
                .await;
                true
            }
            ClientMsg::VfsListResponse(resp) => {
                self.complete(
                    resp.request_id,
                    VfsResponse::List(convert_list_response(resp)),
                )
                .await;
                true
            }
            ClientMsg::VfsDeleteResponse(resp) => {
                self.complete(
                    resp.request_id,
                    VfsResponse::Delete(convert_delete_response(resp)),
                )
                .await;
                true
            }
            ClientMsg::VfsMkdirResponse(resp) => {
                self.complete(
                    resp.request_id,
                    VfsResponse::Mkdir(convert_mkdir_response(resp)),
                )
                .await;
                true
            }
            ClientMsg::VfsRenameResponse(resp) => {
                self.complete(
                    resp.request_id,
                    VfsResponse::Rename(convert_rename_response(resp)),
                )
                .await;
                true
            }
            _ => false,
        }
    }

    async fn complete(&self, request_id: u64, response: VfsResponse) {
        let mut pending = self.pending.lock().await;
        if let Some(req) = pending.remove(&request_id) {
            let _ = req.sender.send(response);
        }
    }
}

// Convert proto responses to Rust results

fn convert_vfs_error(err: &proto::VfsError) -> VfsError {
    let code = VfsErrorCode::try_from(err.code).unwrap_or(VfsErrorCode::Unspecified);
    match code {
        VfsErrorCode::NotFound => VfsError::NotFound(err.message.clone()),
        VfsErrorCode::PermissionDenied => VfsError::PermissionDenied(err.message.clone()),
        VfsErrorCode::AlreadyExists => VfsError::AlreadyExists(err.message.clone()),
        VfsErrorCode::NotADirectory => VfsError::NotDirectory(err.message.clone()),
        VfsErrorCode::NotAFile => VfsError::NotFile(err.message.clone()),
        VfsErrorCode::IoError | VfsErrorCode::Unspecified => VfsError::Storage(err.message.clone()),
    }
}

fn convert_read_response(resp: &proto::VfsReadResponse) -> VfsResult<Vec<u8>> {
    use proto::vfs_read_response::Result as R;
    match &resp.result {
        Some(R::Data(data)) => Ok(data.clone()),
        Some(R::Error(err)) => Err(convert_vfs_error(err)),
        None => Err(VfsError::Storage("empty response".to_string())),
    }
}

fn convert_write_response(resp: &proto::VfsWriteResponse) -> VfsResult<()> {
    use proto::vfs_write_response::Result as R;
    match &resp.result {
        Some(R::Ok(_)) => Ok(()),
        Some(R::Error(err)) => Err(convert_vfs_error(err)),
        None => Err(VfsError::Storage("empty response".to_string())),
    }
}

fn convert_stat_response(resp: &proto::VfsStatResponse) -> VfsResult<Metadata> {
    use proto::vfs_stat_response::Result as R;
    match &resp.result {
        Some(R::Stat(stat)) => {
            let is_dir = matches!(
                proto::FileType::try_from(stat.file_type),
                Ok(proto::FileType::Directory)
            );
            let now = SystemTime::now();
            Ok(Metadata {
                is_dir,
                size: stat.size,
                created: now,
                modified: now,
                accessed: now,
            })
        }
        Some(R::Error(err)) => Err(convert_vfs_error(err)),
        None => Err(VfsError::Storage("empty response".to_string())),
    }
}

fn convert_list_response(resp: &proto::VfsListResponse) -> VfsResult<Vec<DirEntry>> {
    use proto::vfs_list_response::Result as R;
    match &resp.result {
        Some(R::Entries(entries)) => {
            let now = SystemTime::now();
            Ok(entries
                .entries
                .iter()
                .map(|e| {
                    let is_dir = matches!(
                        proto::FileType::try_from(e.file_type),
                        Ok(proto::FileType::Directory)
                    );
                    DirEntry {
                        name: e.name.clone(),
                        metadata: Metadata {
                            is_dir,
                            size: 0, // Size not included in list response
                            created: now,
                            modified: now,
                            accessed: now,
                        },
                    }
                })
                .collect())
        }
        Some(R::Error(err)) => Err(convert_vfs_error(err)),
        None => Err(VfsError::Storage("empty response".to_string())),
    }
}

fn convert_delete_response(resp: &proto::VfsDeleteResponse) -> VfsResult<()> {
    use proto::vfs_delete_response::Result as R;
    match &resp.result {
        Some(R::Ok(_)) => Ok(()),
        Some(R::Error(err)) => Err(convert_vfs_error(err)),
        None => Err(VfsError::Storage("empty response".to_string())),
    }
}

fn convert_mkdir_response(resp: &proto::VfsMkdirResponse) -> VfsResult<()> {
    use proto::vfs_mkdir_response::Result as R;
    match &resp.result {
        Some(R::Ok(_)) => Ok(()),
        Some(R::Error(err)) => Err(convert_vfs_error(err)),
        None => Err(VfsError::Storage("empty response".to_string())),
    }
}

fn convert_rename_response(resp: &proto::VfsRenameResponse) -> VfsResult<()> {
    use proto::vfs_rename_response::Result as R;
    match &resp.result {
        Some(R::Ok(_)) => Ok(()),
        Some(R::Error(err)) => Err(convert_vfs_error(err)),
        None => Err(VfsError::Storage("empty response".to_string())),
    }
}

#[async_trait]
impl VfsStorage for RemoteStorage {
    async fn read(&self, path: &str) -> VfsResult<Vec<u8>> {
        let path = path.to_string();
        let resp = self
            .request(|id| {
                ServerMsg::VfsRead(proto::VfsReadRequest {
                    request_id: id,
                    path: path.clone(),
                })
            })
            .await?;

        match resp {
            VfsResponse::Read(r) => r,
            _ => Err(VfsError::Storage("unexpected response type".to_string())),
        }
    }

    async fn read_at(&self, path: &str, offset: u64, len: u64) -> VfsResult<Vec<u8>> {
        // Read full file and slice - could optimize with proto extension later
        let data = self.read(path).await?;
        let start = offset as usize;
        let end = (offset + len) as usize;
        if start >= data.len() {
            Ok(Vec::new())
        } else {
            Ok(data[start..end.min(data.len())].to_vec())
        }
    }

    async fn write(&self, path: &str, data: &[u8]) -> VfsResult<()> {
        let data = data.to_vec();
        let path = path.to_string();
        let resp = self
            .request(|id| {
                ServerMsg::VfsWrite(proto::VfsWriteRequest {
                    request_id: id,
                    path: path.clone(),
                    data: data.clone(),
                })
            })
            .await?;

        match resp {
            VfsResponse::Write(r) => r,
            _ => Err(VfsError::Storage("unexpected response type".to_string())),
        }
    }

    async fn write_at(&self, path: &str, offset: u64, data: &[u8]) -> VfsResult<()> {
        // Read, modify, write - could optimize with proto extension later
        let mut existing = match self.read(path).await {
            Ok(d) => d,
            Err(VfsError::NotFound(_)) => Vec::new(),
            Err(e) => return Err(e),
        };

        let start = offset as usize;
        if start > existing.len() {
            existing.resize(start, 0);
        }
        let end = start + data.len();
        if end > existing.len() {
            existing.resize(end, 0);
        }
        existing[start..end].copy_from_slice(data);

        self.write(path, &existing).await
    }

    async fn set_size(&self, path: &str, size: u64) -> VfsResult<()> {
        // Read, truncate/extend, write
        let mut data = match self.read(path).await {
            Ok(d) => d,
            Err(VfsError::NotFound(_)) => Vec::new(),
            Err(e) => return Err(e),
        };

        data.resize(size as usize, 0);
        self.write(path, &data).await
    }

    async fn stat(&self, path: &str) -> VfsResult<Metadata> {
        let path = path.to_string();
        let resp = self
            .request(|id| {
                ServerMsg::VfsStat(proto::VfsStatRequest {
                    request_id: id,
                    path: path.clone(),
                })
            })
            .await?;

        match resp {
            VfsResponse::Stat(r) => r,
            _ => Err(VfsError::Storage("unexpected response type".to_string())),
        }
    }

    async fn list(&self, path: &str) -> VfsResult<Vec<DirEntry>> {
        let path = path.to_string();
        let resp = self
            .request(|id| {
                ServerMsg::VfsList(proto::VfsListRequest {
                    request_id: id,
                    path: path.clone(),
                })
            })
            .await?;

        match resp {
            VfsResponse::List(r) => r,
            _ => Err(VfsError::Storage("unexpected response type".to_string())),
        }
    }

    async fn delete(&self, path: &str) -> VfsResult<()> {
        let path = path.to_string();
        let resp = self
            .request(|id| {
                ServerMsg::VfsDelete(proto::VfsDeleteRequest {
                    request_id: id,
                    path: path.clone(),
                })
            })
            .await?;

        match resp {
            VfsResponse::Delete(r) => r,
            _ => Err(VfsError::Storage("unexpected response type".to_string())),
        }
    }

    async fn mkdir(&self, path: &str) -> VfsResult<()> {
        let path = path.to_string();
        let resp = self
            .request(|id| {
                ServerMsg::VfsMkdir(proto::VfsMkdirRequest {
                    request_id: id,
                    path: path.clone(),
                })
            })
            .await?;

        match resp {
            VfsResponse::Mkdir(r) => r,
            _ => Err(VfsError::Storage("unexpected response type".to_string())),
        }
    }

    async fn rmdir(&self, path: &str) -> VfsResult<()> {
        // Use delete for directories too
        self.delete(path).await
    }

    async fn rename(&self, from: &str, to: &str) -> VfsResult<()> {
        let from = from.to_string();
        let to = to.to_string();
        let resp = self
            .request(|id| {
                ServerMsg::VfsRename(proto::VfsRenameRequest {
                    request_id: id,
                    from_path: from.clone(),
                    to_path: to.clone(),
                })
            })
            .await?;

        match resp {
            VfsResponse::Rename(r) => r,
            _ => Err(VfsError::Storage("unexpected response type".to_string())),
        }
    }

    async fn exists(&self, path: &str) -> VfsResult<bool> {
        match self.stat(path).await {
            Ok(_) => Ok(true),
            Err(VfsError::NotFound(_)) => Ok(false),
            Err(e) => Err(e),
        }
    }

    fn mkdir_sync(&self, path: &str) -> VfsResult<()> {
        // Use block_in_place to run async code synchronously.
        // This is safe because we're in a multi-threaded tokio runtime.
        let path = path.to_string();
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(self.mkdir(&path))
        })
    }
}
