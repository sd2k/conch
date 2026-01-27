//! Agent sandbox module for multi-agent systems.
//!
//! This module provides the [`AgentVfs`] filesystem structure and [`AgentSandbox`]
//! execution environment for agents in a multi-agent system.
//!
//! # Filesystem Layout
//!
//! Each agent has access to a virtual filesystem with the following structure:
//!
//! ```text
//! /agent/              - Current agent context (read-write)
//! ├── metadata.json    - Agent ID, name, capabilities
//! ├── params.json      - Parameters passed when spawned
//! ├── scratch/         - Temporary working directory
//! └── state/           - Persistent state across tool calls
//!
//! /tools/              - Tool definitions and results
//! ├── index.txt        - Quick reference (name + description)
//! ├── available/       - Full tool definitions (JSON schema)
//! ├── pending/         - Tools currently executing
//! └── history/         - Completed tool calls
//! ```
//!
//! # Example
//!
//! ```rust,ignore
//! use conch::agent::{AgentVfs, ToolDefinition};
//! use conch::InMemoryStorage;
//! use serde_json::json;
//!
//! let vfs = AgentVfs::builder("agent-123")
//!     .name("code-reviewer")
//!     .parent("agent-root")
//!     .tool(ToolDefinition::new("web_search", "Search the web", json!({})))
//!     .build(InMemoryStorage::new())
//!     .await?;
//!
//! // Agent metadata is automatically available
//! let metadata = vfs.read("/agent/metadata.json").await?;
//!
//! // Tools are available in /tools/
//! let index = vfs.read("/tools/index.txt").await?;
//! ```

mod sandbox;
mod tools;
mod vfs;

pub use sandbox::{AgentSandbox, AgentSandboxBuilder};
pub use tools::{ToolDefinition, ToolRegistry, ToolSummary, VecToolRegistry, generate_index_txt};
pub use vfs::{AgentMetadata, AgentVfs, AgentVfsBuilder};
