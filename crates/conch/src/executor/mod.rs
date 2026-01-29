//! WASM executor for running shell scripts.
//!
//! This module provides the [`ComponentShellExecutor`] which uses the
//! wasip2 component model to run shell scripts in a WASM sandbox.
//!
//! The executor uses the `InstancePre` pattern to pre-link the component once,
//! then efficiently instantiate per execution call.

mod component;

pub use component::{ComponentShellExecutor, ToolHandler, ToolRequest, ToolResult};
