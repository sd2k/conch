//! Conch: Virtual Filesystem Shell Sandbox
//!
//! Conch provides a sandboxed shell environment for executing commands with a
//! hybrid virtual filesystem that combines:
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
//! // Execute commands
//! let result = shell.execute(
//!     "cat /scratch/input.txt && ls /project/src",
//!     &ResourceLimits::default(),
//! ).await?;
//! ```

mod executor;
mod limits;
mod runtime;
mod shell;

#[cfg(test)]
mod tests;

pub mod ffi;

// Core Shell API
pub use shell::{Mount, Shell, ShellBuilder};

// Executor (for advanced usage)
pub use executor::ComponentShellExecutor;

// Resource limits
pub use limits::ResourceLimits;

// Runtime types
pub use runtime::{Conch, ExecutionResult, ExecutionStats, RuntimeError};

// Re-export eryx-vfs types for VFS storage
pub use eryx_vfs::{DirPerms, FilePerms, InMemoryStorage, VfsStorage};
