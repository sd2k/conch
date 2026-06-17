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

/// Enable wasmtime's on-disk compilation cache on the given config.
///
/// Compiling the embedded shell (and child components) with cranelift takes
/// seconds. Because `cargo nextest` runs every test in its own process, an
/// executor created without a precompiled `.cwasm` would re-run that JIT for
/// each test. wasmtime's cache persists compiled artifacts to disk, keyed by
/// (wasmtime version + `Config` + module hash), so the compilation is paid
/// roughly once instead of per process.
///
/// The cache is transparent: a hit produces byte-identical compiled output to
/// a miss, so this only affects speed, never behavior, and it composes with the
/// existing `Engine::deserialize` (cwasm) fast path. Enabling it is best-effort
/// — if the cache directory can't be created we simply run without it.
fn enable_compilation_cache(config: &mut wasmtime::Config) {
    match wasmtime::Cache::new(wasmtime::CacheConfig::new()) {
        Ok(cache) => {
            config.cache(Some(cache));
        }
        Err(e) => {
            tracing::warn!("failed to enable wasmtime compilation cache: {e}");
        }
    }
}
