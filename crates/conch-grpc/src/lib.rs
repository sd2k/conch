//! Conch gRPC Server
//!
//! A gRPC service that exposes Conch sandboxed shell execution with
//! bidirectional streaming for VFS operations and tool callbacks.
//!
//! # Architecture
//!
//! The server is stateless - all VFS state is owned by the client (e.g., a Go
//! orchestrator). Every filesystem operation (read, write, list, etc.) is sent
//! to the client over the gRPC stream, and the client responds with the data
//! or error.
//!
//! This design allows:
//! - Rust pods to be rolled/scaled freely
//! - State to persist in the client's storage (DB, Redis, etc.)
//! - The client to implement caching, access control, etc.
//!
//! # Example Flow
//!
//! ```text
//! Client                                    Server
//! │                                           │
//! │  ExecuteRequest{script: "cat /file"}      │
//! │ ─────────────────────────────────────────>│
//! │                                           │
//! │       VfsReadRequest{path: "/file"}       │
//! │<───────────────────────────────────────── │
//! │                                           │
//! │  VfsReadResponse{data: "contents"}        │
//! │ ─────────────────────────────────────────>│
//! │                                           │
//! │       Output{stdout: "contents"}          │
//! │<───────────────────────────────────────── │
//! │                                           │
//! │       Completed{exit_code: 0}             │
//! │<───────────────────────────────────────── │
//! ```

pub mod proto {
    #![allow(missing_docs)]
    #![allow(clippy::doc_markdown)]
    tonic::include_proto!("conch.v1");
}

mod remote_storage;
mod server;

pub use remote_storage::RemoteStorage;
pub use server::{SandboxServer, SandboxService};

// Re-export proto types for convenience
pub use proto::{
    ClientMessage, ExecuteRequest, ServerMessage, sandbox_client::SandboxClient,
    sandbox_server::SandboxServer as SandboxGrpcServer,
};
