//! Session snapshots: capture and restore shell state.
//!
//! A [`Snapshot`] captures the serialized guest shell state — variables,
//! functions, aliases, shell options, the working directory, traps, and the
//! directory stack. It is produced by [`Shell::snapshot`](crate::Shell::snapshot)
//! and applied with [`Shell::restore`](crate::Shell::restore).
//!
//! State is captured by serializing the brush interpreter's logical state
//! (serde / MessagePack), not by imaging wasm linear memory — so snapshots are
//! compact, inspectable, and decoupled from the wasm instance.
//!
//! Not yet captured: VFS contents (tracked for a follow-up), background jobs,
//! and open file descriptors (re-established on restore).

use crate::runtime::RuntimeError;

/// Container format version for the host-level snapshot envelope. Bumped when
/// the envelope layout changes incompatibly; [`Snapshot::from_bytes`] rejects
/// mismatches rather than mis-decoding.
const CONTAINER_VERSION: u32 = 1;

/// A captured snapshot of a shell session.
///
/// Treat the contents as opaque. Use [`Shell::snapshot`](crate::Shell::snapshot)
/// to create one, [`Shell::restore`](crate::Shell::restore) to apply it, and
/// [`Snapshot::to_bytes`]/[`Snapshot::from_bytes`] to persist or transport it.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct Snapshot {
    /// Envelope version (rejected on mismatch by `from_bytes`).
    version: u32,
    /// Opaque, guest-produced shell-state blob (MessagePack).
    pub(crate) shell_state: Vec<u8>,
    /// Serialized VFS contents (MessagePack of `eryx_vfs::InMemorySnapshot`), if
    /// the shell's storage is in-memory and could be captured. `None` for
    /// non-in-memory storage (e.g. real-filesystem mounts).
    pub(crate) vfs: Option<Vec<u8>>,
    /// Unix timestamp (milliseconds) when the snapshot was captured.
    created_unix_ms: u64,
}

impl std::fmt::Debug for Snapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Snapshot")
            .field("version", &self.version)
            .field("size_bytes", &self.shell_state.len())
            .field("created_unix_ms", &self.created_unix_ms)
            .finish()
    }
}

impl Snapshot {
    /// Wrap guest shell-state and optional VFS blobs in a versioned,
    /// timestamped envelope.
    pub(crate) fn new(shell_state: Vec<u8>, vfs: Option<Vec<u8>>) -> Self {
        let created_unix_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        Self {
            version: CONTAINER_VERSION,
            shell_state,
            vfs,
            created_unix_ms,
        }
    }

    /// Whether this snapshot captured VFS contents.
    pub fn has_vfs(&self) -> bool {
        self.vfs.is_some()
    }

    /// Serialize this snapshot to bytes for storage or transport.
    pub fn to_bytes(&self) -> Result<Vec<u8>, RuntimeError> {
        rmp_serde::to_vec_named(self).map_err(|e| RuntimeError::Snapshot(e.to_string()))
    }

    /// Deserialize a snapshot previously produced by [`Self::to_bytes`].
    ///
    /// Returns an error if the bytes are malformed or the envelope version is
    /// incompatible with this build.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, RuntimeError> {
        let snap: Self = rmp_serde::from_slice(bytes)
            .map_err(|e| RuntimeError::Snapshot(format!("malformed snapshot: {e}")))?;
        if snap.version != CONTAINER_VERSION {
            return Err(RuntimeError::Snapshot(format!(
                "incompatible snapshot container version {} (this build expects {})",
                snap.version, CONTAINER_VERSION
            )));
        }
        Ok(snap)
    }

    /// Unix timestamp (milliseconds) when this snapshot was captured.
    pub fn created_unix_ms(&self) -> u64 {
        self.created_unix_ms
    }

    /// Total size of the captured blobs (shell state + VFS) in bytes.
    pub fn size_bytes(&self) -> usize {
        self.shell_state.len() + self.vfs.as_ref().map_or(0, Vec::len)
    }
}
