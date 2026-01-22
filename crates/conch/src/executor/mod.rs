//! WASM executors for running shell scripts.
//!
//! This module provides two executor implementations:
//!
//! - [`CoreShellExecutor`]: Uses wasip1 core modules (legacy, for compatibility)
//! - [`ComponentShellExecutor`]: Uses wasip2 component model (preferred)
//!
//! Both executors use the `InstancePre` pattern to pre-link modules once,
//! then efficiently instantiate per execution call.

mod component;
mod core;

pub use component::ComponentShellExecutor;
pub use core::CoreShellExecutor;
