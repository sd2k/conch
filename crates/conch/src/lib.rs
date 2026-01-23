//! Conch: Virtual Filesystem Shell Sandbox
//!
//! Conch exposes multi-agent tool call history as a POSIX-like virtual filesystem,
//! allowing agents to query and analyze their own and related agents' execution
//! context using familiar shell commands within a secure WASM sandbox.

mod executor;
mod limits;
mod runtime;
mod vfs;

#[cfg(test)]
mod tests;

pub mod ffi;

pub use executor::ComponentShellExecutor;
pub use limits::ResourceLimits;
pub use runtime::{Conch, ExecutionContext, ExecutionResult, ExecutionStats, RuntimeError};
pub use vfs::{AccessPolicy, ContextFs, ContextProvider, DirEntry, FsError, Metadata};
