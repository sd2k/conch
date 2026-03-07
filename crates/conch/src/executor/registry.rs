//! Component registry for mapping command names to WASI components.
//!
//! The registry provides a simple `HashMap`-based lookup from command names
//! to WASM component bytes. When the shell encounters an unknown command,
//! it looks up the name here to find a component to instantiate.

use std::collections::HashMap;
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
    pub fn get_bytes(&self, name: &str) -> Option<&RegistryEntry> {
        self.entries.get(name)
    }

    /// Check if a command name is registered.
    pub fn contains(&self, name: &str) -> bool {
        self.entries.contains_key(name)
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
