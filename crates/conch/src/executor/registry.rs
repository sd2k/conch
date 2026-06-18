//! Component registry for mapping command names to WASI components.
//!
//! The registry provides a simple `HashMap`-based lookup from command names
//! to WASM component bytes. When the shell encounters an unknown command,
//! it looks up the name here to find a component to instantiate.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// An entry in the component registry, holding the original WASM bytes.
///
/// The raw bytes are stored so that child executors with different engine
/// configurations (e.g., wasip3 with component-model-async) can compile
/// the component for their own engine.
#[derive(Clone, Debug)]
pub enum RegistryEntry {
    Wasm(Arc<Vec<u8>>),
    CWasm(Arc<Vec<u8>>),
}

/// A registry mapping command names to WASI component bytes.
///
/// Components are stored as raw WASM bytes and compiled on demand for
/// the target engine. This allows the child executor (which uses a
/// different engine with wasip3 support) to compile components as needed.
///
/// # Example
///
/// ```rust,ignore
/// let mut registry = ComponentRegistry::new();
/// registry.register("upper", &wasm_bytes);
///
/// let shell = Shell::builder()
///     .component_registry(registry)
///     .build()
///     .await?;
/// ```
#[derive(Clone, Default)]
pub struct ComponentRegistry {
    entries: HashMap<String, RegistryEntry>,
    /// Default sandbox root mounted at `/` for spawned commands. `None` means
    /// the host's real root (`/`) is mounted — the previous hardcoded fallback.
    default_sandbox_root: Option<PathBuf>,
    /// Per-command sandbox root overrides, taking precedence over the default.
    command_sandbox_roots: HashMap<String, PathBuf>,
}

impl std::fmt::Debug for ComponentRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ComponentRegistry")
            .field("count", &self.entries.len())
            .finish()
    }
}

impl ComponentRegistry {
    /// Create a new empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a command name with raw WASM component bytes.
    pub fn register_wasm(&mut self, name: impl Into<String>, wasm_bytes: Vec<u8>) {
        self.entries
            .insert(name.into(), RegistryEntry::Wasm(Arc::new(wasm_bytes)));
    }

    /// Register a command name with raw CWASM component bytes.
    pub fn register_cwasm(&mut self, name: impl Into<String>, cwasm_bytes: Vec<u8>) {
        self.entries
            .insert(name.into(), RegistryEntry::CWasm(Arc::new(cwasm_bytes)));
    }

    /// Look up the WASM bytes for a command name.
    ///
    /// Crate-internal: returns [`RegistryEntry`], which is not part of the
    /// public API (the host calls this when spawning a child component).
    pub(crate) fn get_bytes(&self, name: &str) -> Option<&RegistryEntry> {
        self.entries.get(name)
    }

    /// Check if a command name is registered.
    pub fn contains(&self, name: &str) -> bool {
        self.entries.contains_key(name)
    }

    /// Set the default sandbox root mounted at `/` for every spawned command.
    ///
    /// Builder-style; replaces the old hardcoded `/tmp/gh-root`. When unset,
    /// the host's real root (`/`) is mounted (the previous fallback behaviour).
    #[must_use]
    pub fn with_sandbox_root(mut self, root: impl Into<PathBuf>) -> Self {
        self.default_sandbox_root = Some(root.into());
        self
    }

    /// Set the default sandbox root mounted at `/` for every spawned command.
    pub fn set_sandbox_root(&mut self, root: impl Into<PathBuf>) {
        self.default_sandbox_root = Some(root.into());
    }

    /// Override the sandbox root for a single command, taking precedence over
    /// the default. Lets one registry host commands with different sandboxes.
    pub fn set_command_sandbox_root(&mut self, name: impl Into<String>, root: impl Into<PathBuf>) {
        self.command_sandbox_roots.insert(name.into(), root.into());
    }

    /// Resolve the sandbox root for a command: its per-command override if set,
    /// otherwise the registry default (`None` => mount the real `/`).
    pub fn sandbox_root(&self, name: &str) -> Option<&Path> {
        self.command_sandbox_roots
            .get(name)
            .or(self.default_sandbox_root.as_ref())
            .map(PathBuf::as_path)
    }

    /// Get the number of registered components.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Check if the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// A shared, thread-safe component registry.
pub type SharedRegistry = Arc<ComponentRegistry>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sandbox_root_defaults_to_none() {
        let registry = ComponentRegistry::new();
        assert_eq!(registry.sandbox_root("gh"), None);
    }

    #[test]
    fn sandbox_root_uses_default_for_all_commands() {
        let registry = ComponentRegistry::new().with_sandbox_root("/tmp/root");
        assert_eq!(registry.sandbox_root("gh"), Some(Path::new("/tmp/root")));
        assert_eq!(
            registry.sandbox_root("anything"),
            Some(Path::new("/tmp/root"))
        );
    }

    #[test]
    fn command_override_takes_precedence_over_default() {
        let mut registry = ComponentRegistry::new().with_sandbox_root("/tmp/default");
        registry.set_command_sandbox_root("gh", "/tmp/gh-root");
        assert_eq!(registry.sandbox_root("gh"), Some(Path::new("/tmp/gh-root")));
        // Other commands still fall back to the default.
        assert_eq!(
            registry.sandbox_root("curl"),
            Some(Path::new("/tmp/default"))
        );
    }

    #[test]
    fn command_override_works_without_a_default() {
        let mut registry = ComponentRegistry::new();
        registry.set_command_sandbox_root("gh", "/tmp/gh-root");
        assert_eq!(registry.sandbox_root("gh"), Some(Path::new("/tmp/gh-root")));
        assert_eq!(registry.sandbox_root("curl"), None);
    }
}
