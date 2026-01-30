//! Conch: Virtual Filesystem Shell Sandbox
//!
//! Conch provides a sandboxed shell environment for executing commands with a
//! hybrid virtual filesystem that combines:
//!
//! - **VFS storage**: In-memory or custom storage for orchestrator-controlled paths
//! - **Real filesystem**: cap-std secured mounts for host directory access
//!
//! ## State Persistence
//!
//! Unlike stateless shell execution, the [`Shell`] maintains state across multiple
//! `execute` calls. Variables, functions, and aliases defined in one execution
//! persist to subsequent ones.
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
//! // State persists between execute calls
//! shell.execute("greeting=hello", &ResourceLimits::default()).await?;
//! let result = shell.execute("echo $greeting world", &ResourceLimits::default()).await?;
//! // result.stdout contains "hello world"
//!
//! // Functions persist too
//! shell.execute("greet() { echo \"Hello, $1!\"; }", &ResourceLimits::default()).await?;
//! shell.execute("greet World", &ResourceLimits::default()).await?;
//! ```

pub mod agent;
mod executor;
mod limits;
pub mod policy;
mod runtime;
mod shell;

#[cfg(test)]
mod tests;

pub mod ffi;

// Core Shell API
pub use shell::{DynVfsStorage, Mount, Shell, ShellBuilder};

// Executor (for advanced usage)
pub use executor::ComponentShellExecutor;

#[cfg(feature = "embedded-shell")]
pub use executor::ShellInstance;

// Tool handler types (for tool invocation from shell scripts)
pub use executor::{ToolHandler, ToolRequest, ToolResult};

// Resource limits
pub use limits::ResourceLimits;

// Runtime types
pub use runtime::{Conch, ExecutionResult, ExecutionStats, RuntimeError};

// Re-export eryx-vfs types for VFS storage
pub use eryx_vfs::{DirPerms, FilePerms, InMemoryStorage, VfsStorage};
