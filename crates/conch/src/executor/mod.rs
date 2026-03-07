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

#[cfg(feature = "embedded-shell")]
mod child;
mod component;
mod registry;

pub use component::{ComponentShellExecutor, ToolHandler, ToolRequest, ToolResult};
pub use registry::{ComponentRegistry, SharedRegistry};

#[cfg(feature = "embedded-shell")]
pub use component::ShellInstance;
