//! WASM executor for running shell scripts.
//!
//! This module provides the [`ComponentShellExecutor`] which uses the
//! wasip2 component model to run shell scripts in a WASM sandbox.
//!
//! ## Shell Instances
//!
//! The executor supports creating persistent [`ShellInstance`]s that maintain
//! state (variables, functions, aliases) across multiple `execute` calls.
//! Each instance has its own isolated filesystem and WASM memory.

mod component;

pub use component::{ComponentShellExecutor, ToolHandler, ToolRequest, ToolResult};

#[cfg(feature = "embedded-shell")]
pub use component::ShellInstance;
