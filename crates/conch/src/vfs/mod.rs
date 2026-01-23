//! Virtual filesystem for agent context.
//!
//! This module provides:
//! - `ContextFs` - Core VFS implementation for agent context paths (`/ctx/...`)
//! - `ContextStorage` - Adapter implementing `eryx_vfs::VfsStorage` for WASI integration
//!
//! The VFS exposes agent context as a filesystem:
//! - `/ctx/self/tools/<id>/` - Tool call history
//! - `/ctx/self/messages/` - Conversation history
//! - `/ctx/self/scratch/` - Read-write workspace
//! - `/ctx/parent/tools/` - Parent agent's tools (with permission)

mod context;
mod storage;

pub use context::*;
pub use storage::ContextStorage;
