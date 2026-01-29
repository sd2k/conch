//! gRPC server implementation for the Sandbox service.

use std::pin::Pin;

use std::time::Duration;

use tokio::sync::mpsc;
use tokio_stream::{Stream, StreamExt, wrappers::ReceiverStream};
use tonic::{Request, Response, Status, Streaming};

use conch::ResourceLimits;
use conch::agent::{AgentSandbox, ExecutionOutcome, ToolDefinition, ToolResult};

use crate::proto::{
    self, ClientMessage, ExecuteRequest, ServerMessage, client_message::Msg as ClientMsg,
    server_message::Msg as ServerMsg,
};
use crate::remote_storage::RemoteStorage;

/// The Sandbox gRPC service implementation.
#[derive(Clone, Debug)]
pub struct SandboxService {
    // Could hold shared configuration here in the future
}

impl SandboxService {
    /// Create a new sandbox service.
    pub fn new() -> Self {
        Self {}
    }
}

impl Default for SandboxService {
    fn default() -> Self {
        Self::new()
    }
}

type ExecuteStream = Pin<Box<dyn Stream<Item = Result<ServerMessage, Status>> + Send>>;

#[tonic::async_trait]
impl proto::sandbox_server::Sandbox for SandboxService {
    type ExecuteStream = ExecuteStream;

    async fn execute(
        &self,
        request: Request<Streaming<ClientMessage>>,
    ) -> Result<Response<Self::ExecuteStream>, Status> {
        let mut client_stream = request.into_inner();

        // Wait for the initial ExecuteRequest
        let execute_req = match client_stream.next().await {
            Some(Ok(msg)) => match msg.msg {
                Some(ClientMsg::Execute(req)) => req,
                _ => {
                    return Err(Status::invalid_argument(
                        "first message must be ExecuteRequest",
                    ));
                }
            },
            Some(Err(e)) => return Err(e),
            None => {
                return Err(Status::invalid_argument(
                    "stream closed before ExecuteRequest",
                ));
            }
        };

        // Create channels for bidirectional communication
        let (server_tx, server_rx) = mpsc::channel::<ServerMessage>(32);

        // Spawn the execution task
        let server_tx_clone = server_tx.clone();
        tokio::spawn(async move {
            if let Err(e) = run_execution(execute_req, client_stream, server_tx_clone).await {
                tracing::error!("Execution error: {}", e);
            }
        });

        // Return the server stream
        let stream = ReceiverStream::new(server_rx);
        Ok(Response::new(Box::pin(stream.map(Ok)) as ExecuteStream))
    }
}

/// Run the execution with the given request and streams.
async fn run_execution(
    req: ExecuteRequest,
    mut client_stream: Streaming<ClientMessage>,
    server_tx: mpsc::Sender<ServerMessage>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Create the remote storage backed by the gRPC stream
    let storage = RemoteStorage::new(server_tx.clone());
    let response_handler = storage.response_handler();

    // Convert proto types to conch types
    let limits = convert_limits(&req.limits);
    let tools: Vec<ToolDefinition> = req.tools.iter().map(convert_tool).collect();

    // IMPORTANT: Spawn the response handler task BEFORE building the sandbox,
    // because build_with_storage makes VFS calls that need responses from the client.
    let (tool_response_tx, mut tool_response_rx) = mpsc::channel::<proto::ToolResponse>(8);

    tokio::spawn({
        async move {
            while let Some(Ok(msg)) = client_stream.next().await {
                // Check if it's a VFS response
                if response_handler.handle(&msg).await {
                    continue;
                }

                // Check if it's a tool response
                if let Some(ClientMsg::ToolResponse(resp)) = msg.msg {
                    let _ = tool_response_tx.send(resp).await;
                }
            }
        }
    });

    // Build the agent sandbox with remote storage
    let mut builder = AgentSandbox::builder(&req.agent_id);

    if let Some(ref metadata) = req.metadata {
        if !metadata.name.is_empty() {
            builder = builder.name(&metadata.name);
        }
        for cap in &metadata.capabilities {
            builder = builder.capability(cap);
        }
        if let Ok(params) = serde_json::from_str(&metadata.params_json) {
            builder = builder.params(params);
        }
    }

    for tool in tools {
        builder = builder.tool(tool);
    }

    // Build with the remote storage - this makes VFS calls!
    tracing::debug!("Building sandbox for agent {}", req.agent_id);
    let sandbox = builder
        .build_with_storage(storage)
        .await
        .map_err(|e| format!("failed to build sandbox: {}", e))?;

    // Execute the script
    tracing::debug!("Executing script");
    let outcome = sandbox
        .execute_with_tools(&req.script, &limits)
        .await
        .map_err(|e| format!("execution failed: {}", e))?;

    match outcome {
        ExecutionOutcome::Completed(result) => {
            // Send stdout if any
            if !result.stdout.is_empty() {
                let _ = server_tx
                    .send(ServerMessage {
                        msg: Some(ServerMsg::Output(proto::Output {
                            stream: proto::OutputStream::Stdout.into(),
                            data: result.stdout.clone(),
                        })),
                    })
                    .await;
            }

            // Send stderr if any
            if !result.stderr.is_empty() {
                let _ = server_tx
                    .send(ServerMessage {
                        msg: Some(ServerMsg::Output(proto::Output {
                            stream: proto::OutputStream::Stderr.into(),
                            data: result.stderr.clone(),
                        })),
                    })
                    .await;
            }

            // Send completion
            let _ = server_tx
                .send(ServerMessage {
                    msg: Some(ServerMsg::Completed(proto::Completed {
                        exit_code: result.exit_code,
                        truncated: result.truncated,
                        stats: Some(proto::ExecutionStats {
                            cpu_time_ms: result.stats.cpu_time_ms,
                            wall_time_ms: result.stats.wall_time_ms,
                            memory_bytes: result.stats.peak_memory_bytes,
                        }),
                    })),
                })
                .await;
        }
        ExecutionOutcome::ToolRequest(tool_req) => {
            // Send tool request to client
            let _ = server_tx
                .send(ServerMessage {
                    msg: Some(ServerMsg::ToolRequest(proto::ToolRequest {
                        call_id: tool_req.call_id.clone(),
                        tool: tool_req.tool.clone(),
                        params_json: tool_req.params.to_string(),
                        stdin: tool_req.stdin.map(|s| s.into_bytes()).unwrap_or_default(),
                    })),
                })
                .await;

            // Wait for tool response
            if let Some(resp) = tool_response_rx.recv().await {
                // Write the result back to the sandbox
                let result = match resp.result {
                    Some(proto::tool_response::Result::ResultJson(json)) => {
                        match serde_json::from_str(&json) {
                            Ok(value) => ToolResult::success(value),
                            Err(_) => ToolResult::success(serde_json::Value::String(json)),
                        }
                    }
                    Some(proto::tool_response::Result::Error(err)) => ToolResult::error(err),
                    None => ToolResult::error("empty tool response"),
                };

                if let Err(e) = sandbox.write_tool_result(&tool_req.call_id, result).await {
                    tracing::warn!("Failed to write tool result: {}", e);
                }

                // For now, send completed after tool response
                // In a full implementation, we'd resume execution
                let _ = server_tx
                    .send(ServerMessage {
                        msg: Some(ServerMsg::Completed(proto::Completed {
                            exit_code: 0,
                            truncated: false,
                            stats: None,
                        })),
                    })
                    .await;
            } else {
                // Client disconnected before sending tool response
                let _ = server_tx
                    .send(ServerMessage {
                        msg: Some(ServerMsg::Error(proto::Error {
                            message: "client disconnected before tool response".to_string(),
                            retryable: true,
                        })),
                    })
                    .await;
            }
        }
    }

    Ok(())
}

fn convert_limits(proto_limits: &Option<proto::ResourceLimits>) -> ResourceLimits {
    match proto_limits {
        Some(l) => ResourceLimits {
            max_cpu_ms: if l.max_cpu_ms > 0 { l.max_cpu_ms } else { 5000 },
            max_memory_bytes: if l.max_memory_bytes > 0 {
                l.max_memory_bytes
            } else {
                64 * 1024 * 1024
            },
            max_output_bytes: if l.max_output_bytes > 0 {
                l.max_output_bytes
            } else {
                1024 * 1024
            },
            timeout: Duration::from_millis(if l.timeout_ms > 0 {
                l.timeout_ms
            } else {
                30000
            }),
        },
        None => ResourceLimits::default(),
    }
}

fn convert_tool(proto_tool: &proto::ToolDefinition) -> ToolDefinition {
    let schema =
        serde_json::from_str(&proto_tool.parameters_schema_json).unwrap_or(serde_json::json!({}));
    ToolDefinition::new(&proto_tool.name, &proto_tool.description, schema)
}

/// Server configuration and runner.
#[derive(Debug)]
pub struct SandboxServer {
    addr: std::net::SocketAddr,
}

impl SandboxServer {
    /// Create a new server bound to the given address.
    pub fn new(addr: std::net::SocketAddr) -> Self {
        Self { addr }
    }

    /// Run the server until shutdown signal.
    pub async fn run(self) -> Result<(), Box<dyn std::error::Error>> {
        let service = SandboxService::new();

        tracing::info!("Starting gRPC server on {}", self.addr);

        tonic::transport::Server::builder()
            .add_service(proto::sandbox_server::SandboxServer::new(service))
            .serve_with_shutdown(self.addr, shutdown_signal())
            .await?;

        tracing::info!("gRPC server shut down");
        Ok(())
    }
}

async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(e) = tokio::signal::ctrl_c().await {
            tracing::error!("Failed to install Ctrl+C handler: {}", e);
        }
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(e) => {
                tracing::error!("Failed to install SIGTERM handler: {}", e);
                // Fall through to let ctrl_c handle shutdown
                std::future::pending::<()>().await;
            }
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {
            tracing::info!("Received Ctrl+C, initiating graceful shutdown");
        }
        _ = terminate => {
            tracing::info!("Received SIGTERM, initiating graceful shutdown");
        }
    }
}
