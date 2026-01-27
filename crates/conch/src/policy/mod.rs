//! Policy enforcement for the Conch sandbox.
//!
//! This module provides a policy layer that wraps VFS storage to enforce
//! access control. The policy layer intercepts all filesystem operations
//! and checks them against configurable rules.
//!
//! ## Design Philosophy
//!
//! The policy system separates concerns:
//!
//! - **User approval**: Happens before script execution (high-level intent)
//! - **Policy enforcement**: Happens during execution (security boundaries)
//!
//! The policy doesn't prompt users during execution - it enforces pre-configured
//! rules. If an operation is denied, it fails with an error. This design:
//!
//! - Works in distributed systems (no mid-execution state serialization needed)
//! - Avoids per-operation confirmation fatigue
//! - Provides a clear security boundary regardless of script content
//!
//! ## Example
//!
//! ```rust,ignore
//! use conch::policy::{Policy, PolicyBuilder, Operation};
//!
//! // Create a policy that allows reading agent directories and writing to scratch
//! let policy = PolicyBuilder::new()
//!     .allow_read("/agent/**")
//!     .allow_read("/tools/**")
//!     .allow_read("/history/**")
//!     .allow_write("/agent/scratch/**")
//!     .build();
//!
//! // Wrap storage with policy enforcement
//! let policy_storage = PolicyStorage::new(storage, policy);
//! ```

mod handler;
mod storage;

pub use handler::{
    AllowAllPolicy, CommandInfo, CommandTracker, DenyAllPolicy, Operation, Policy, PolicyBuilder,
    PolicyDecision, PolicyHandler, agent_sandbox_policy, read_only_policy,
};
pub use storage::PolicyStorage;
