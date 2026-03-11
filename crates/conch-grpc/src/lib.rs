//! Conch gRPC Server
//!
//! A gRPC service that exposes Conch sandboxed shell execution with
//! bidirectional streaming for tool callbacks.
//!
//! # Architecture
//!
//! The server uses in-memory VFS storage. Each execution gets a fresh sandbox
//! with an isolated filesystem. Supporting files can be pre-seeded via the
//! `SupportingFile` message, and tools are invoked via bidirectional gRPC
//! streaming.
//!
//! # Example Flow
//!
//! ```text
//! Client                                    Server
//! │                                           │
//! │  ExecuteRequest{script, tools, files}     │
//! │ ─────────────────────────────────────────>│
//! │                                           │
//! │       ToolRequest{name, args_json}        │
//! │<───────────────────────────────────────── │
//! │                                           │
//! │  ToolResponse{json_result}                │
//! │ ─────────────────────────────────────────>│
//! │                                           │
//! │       Output{stdout: "..."}               │
//! │<───────────────────────────────────────── │
//! │                                           │
//! │       ExecuteResult{exit_code: 0}         │
//! │<───────────────────────────────────────── │
//! ```

pub mod proto {
    #![allow(missing_docs)]
    #![allow(clippy::doc_markdown)]
    tonic::include_proto!("conch.v1");
}

mod server;

pub use server::{SandboxServer, SandboxService};

// Re-export proto types for convenience
pub use proto::{
    ClientMessage, ExecuteRequest, ServerMessage, sandbox_client::SandboxClient,
    sandbox_server::SandboxServer as SandboxGrpcServer,
};
