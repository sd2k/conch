//! gRPC server implementation for the Sandbox service.

use std::pin::Pin;
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::{mpsc, oneshot};
use tokio_stream::{Stream, StreamExt, wrappers::ReceiverStream};
use tonic::{Request, Response, Status, Streaming};

use conch::ResourceLimits;
use conch::agent::{AgentSandbox, ToolDefinition};
use conch::{ToolHandler, ToolRequest, ToolResult, VfsStorage};

use crate::proto::{
    self, ClientMessage, ExecuteRequest, ServerMessage, client_message::Msg as ClientMsg,
    server_message::Msg as ServerMsg,
};

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

/// Tool handler that sends requests to the gRPC client and waits for responses.
struct GrpcToolHandler {
    /// Channel to send tool requests to the server (which forwards to client)
    request_tx: mpsc::Sender<(ToolRequest, oneshot::Sender<ToolResult>)>,
}

#[async_trait]
impl ToolHandler for GrpcToolHandler {
    async fn invoke(&self, request: ToolRequest) -> ToolResult {
        let (response_tx, response_rx) = oneshot::channel();

        // Send request through channel
        if self.request_tx.send((request, response_tx)).await.is_err() {
            return ToolResult {
                success: false,
                output: "Tool handler channel closed".to_string(),
            };
        }

        // Wait for response
        match response_rx.await {
            Ok(result) => result,
            Err(_) => ToolResult {
                success: false,
                output: "Tool response channel closed".to_string(),
            },
        }
    }
}

/// Run the execution with the given request and streams.
async fn run_execution(
    req: ExecuteRequest,
    mut client_stream: Streaming<ClientMessage>,
    server_tx: mpsc::Sender<ServerMessage>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Convert proto types to conch types
    let limits = convert_limits(&req.limits);
    let tools: Vec<ToolDefinition> = req.tools.iter().map(convert_tool).collect();

    // Create channel for tool requests
    let (tool_request_tx, mut tool_request_rx) =
        mpsc::channel::<(ToolRequest, oneshot::Sender<ToolResult>)>(8);

    // Create the tool handler
    let tool_handler = GrpcToolHandler {
        request_tx: tool_request_tx,
    };

    // Clone server_tx for the message handler task
    let server_tx_for_tools = server_tx.clone();

    // Spawn message handler task for tool request/response routing
    let message_handler = tokio::spawn({
        async move {
            // Map of pending tool requests: call_id -> response sender
            let mut pending_tools: std::collections::HashMap<String, oneshot::Sender<ToolResult>> =
                std::collections::HashMap::new();

            loop {
                tokio::select! {
                    // Handle outgoing tool requests
                    Some((request, response_tx)) = tool_request_rx.recv() => {
                        let call_id = format!("call-{}", pending_tools.len() + 1);

                        // Send tool request to client
                        let _ = server_tx_for_tools
                            .send(ServerMessage {
                                msg: Some(ServerMsg::ToolRequest(proto::ToolRequest {
                                    request_id: call_id.clone(),
                                    name: request.tool.clone(),
                                    arguments_json: request.params.clone(),
                                })),
                            })
                            .await;

                        // Store the response sender
                        pending_tools.insert(call_id, response_tx);
                    }

                    // Handle incoming client messages (tool responses only)
                    msg = client_stream.next() => {
                        match msg {
                            Some(Ok(msg)) => {
                                if let Some(ClientMsg::ToolResponse(resp)) = msg.msg
                                    && let Some(response_tx) = pending_tools.remove(&resp.request_id)
                                {
                                    let result = match resp.result {
                                        Some(proto::tool_response::Result::JsonResult(json)) => {
                                            ToolResult {
                                                success: true,
                                                output: json,
                                            }
                                        }
                                        Some(proto::tool_response::Result::Error(err)) => {
                                            ToolResult {
                                                success: false,
                                                output: err,
                                            }
                                        }
                                        None => ToolResult {
                                            success: false,
                                            output: "empty tool response".to_string(),
                                        },
                                    };
                                    let _ = response_tx.send(result);
                                }
                            }
                            Some(Err(e)) => {
                                tracing::error!("Client stream error: {}", e);
                                break;
                            }
                            None => {
                                tracing::debug!("Client stream ended");
                                break;
                            }
                        }
                    }
                }
            }
        }
    });

    // Build the agent sandbox with in-memory storage and tool handler
    let mut builder = AgentSandbox::builder(format!("conch-{}", uuid_v4()));

    for tool in tools {
        builder = builder.tool(tool);
    }

    // Set up the tool handler
    builder = builder.tool_handler(tool_handler);

    // Build with default in-memory storage
    tracing::debug!("Building sandbox");
    let mut sandbox = builder
        .build()
        .await
        .map_err(|e| format!("failed to build sandbox: {}", e))?;

    // Write supporting files into /agent/scratch/ (already mounted by AgentSandbox)
    if !req.files.is_empty() {
        let vfs: &dyn VfsStorage = sandbox.vfs();
        for file in &req.files {
            let path = format!("/agent/scratch/{}", file.name);
            vfs.write(&path, &file.content)
                .await
                .map_err(|e| format!("failed to write supporting file {}: {}", file.name, e))?;
        }
    }

    // Execute the script
    tracing::debug!("Executing script");
    let result = sandbox
        .execute(&req.script, &limits)
        .await
        .map_err(|e| format!("execution failed: {}", e))?;

    // Send stdout if any
    if !result.stdout.is_empty() {
        let _ = server_tx
            .send(ServerMessage {
                msg: Some(ServerMsg::Output(proto::OutputEvent {
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
                msg: Some(ServerMsg::Output(proto::OutputEvent {
                    stream: proto::OutputStream::Stderr.into(),
                    data: result.stderr.clone(),
                })),
            })
            .await;
    }

    // Send execution result
    let _ = server_tx
        .send(ServerMessage {
            msg: Some(ServerMsg::Result(proto::ExecuteResult {
                exit_code: result.exit_code,
                stdout: String::from_utf8_lossy(&result.stdout).to_string(),
                stderr: String::from_utf8_lossy(&result.stderr).to_string(),
                error: String::new(),
                stats: Some(proto::ExecutionStats {
                    cpu_time_ms: result.stats.cpu_time_ms,
                    wall_time_ms: result.stats.wall_time_ms,
                    peak_memory_bytes: result.stats.peak_memory_bytes,
                    fuel_consumed: 0, // TODO: expose from conch
                }),
            })),
        })
        .await;

    // Clean up the message handler
    message_handler.abort();

    Ok(())
}

fn convert_limits(proto_limits: &Option<proto::ResourceLimits>) -> ResourceLimits {
    match proto_limits {
        Some(l) => ResourceLimits {
            max_cpu_ms: if l.execution_timeout_ms > 0 {
                l.execution_timeout_ms
            } else {
                30000
            },
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
            timeout: Duration::from_millis(if l.execution_timeout_ms > 0 {
                l.execution_timeout_ms
            } else {
                30000
            }),
        },
        None => ResourceLimits::default(),
    }
}

fn convert_tool(proto_tool: &proto::ToolDeclaration) -> ToolDefinition {
    let schema =
        serde_json::from_str(&proto_tool.parameters_schema_json).unwrap_or(serde_json::json!({}));
    ToolDefinition::new(&proto_tool.name, &proto_tool.description, schema)
}

/// Generate a simple UUID v4 string (no external dependency needed).
fn uuid_v4() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{:032x}", nanos)
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
