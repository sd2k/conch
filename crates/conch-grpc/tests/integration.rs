//! Integration tests for the gRPC bidirectional streaming.
//!
//! These tests simulate a client (like the Go orchestrator) interacting with
//! the sandbox server over gRPC. The server uses in-memory VFS storage, so
//! the client only needs to handle tool request/response messages.

#![allow(clippy::unwrap_used)] // unwrap is acceptable in tests

use std::net::SocketAddr;
use std::sync::Once;
use std::time::Duration;

static INIT_TRACING: Once = Once::new();

fn init_tracing() {
    INIT_TRACING.call_once(|| {
        tracing_subscriber::fmt()
            .with_env_filter("conch_grpc=debug,conch=debug")
            .with_test_writer()
            .init();
    });
}

use tokio::sync::mpsc;
use tokio::time::timeout;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::ReceiverStream;
use tonic::transport::Channel;

use conch_grpc::proto::{
    self, ClientMessage, ExecuteRequest, client_message::Msg as ClientMsg,
    sandbox_client::SandboxClient, server_message::Msg as ServerMsg,
};

/// Macro to log messages in tests (visible with --nocapture)
macro_rules! test_log {
    ($($arg:tt)*) => {
        eprintln!("[TEST] {}", format!($($arg)*));
    };
}

/// Start the test server and return its address.
async fn start_test_server() -> SocketAddr {
    test_log!("Starting test server...");
    let addr: SocketAddr = "[::1]:0".parse().unwrap();

    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    let actual_addr = listener.local_addr().unwrap();
    test_log!("Server bound to {}", actual_addr);

    let service = conch_grpc::SandboxService::new();

    tokio::spawn(async move {
        test_log!("Server task starting...");
        let result = tonic::transport::Server::builder()
            .add_service(proto::sandbox_server::SandboxServer::new(service))
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await;
        test_log!("Server task ended: {:?}", result);
    });

    // Give the server a moment to start
    tokio::time::sleep(Duration::from_millis(50)).await;
    test_log!("Server started");

    actual_addr
}

/// Connect a client to the server.
async fn connect_client(addr: SocketAddr) -> SandboxClient<Channel> {
    test_log!("Connecting client to {}...", addr);
    let endpoint = format!("http://{}", addr);
    let client = SandboxClient::connect(endpoint).await.unwrap();
    test_log!("Client connected");
    client
}

/// Helper: run a script and collect stdout, stderr, and exit code.
/// Optionally handles tool requests via the provided callback.
async fn run_script(
    client: &mut SandboxClient<Channel>,
    req: ExecuteRequest,
    tool_handler: Option<Box<dyn Fn(&proto::ToolRequest) -> proto::tool_response::Result + Send>>,
) -> (String, String, Option<i32>, bool) {
    let (client_tx, client_rx) = mpsc::channel::<ClientMessage>(32);
    let client_stream = ReceiverStream::new(client_rx);

    // Spawn task to send execute request
    let client_tx_clone = client_tx.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(10)).await;
        client_tx_clone
            .send(ClientMessage {
                msg: Some(ClientMsg::Execute(req)),
            })
            .await
            .unwrap();
    });

    let response = client.execute(client_stream).await.unwrap();
    let mut server_stream = response.into_inner();

    let mut stdout = String::new();
    let mut stderr = String::new();
    let mut exit_code = None;
    let mut tool_called = false;

    let result = timeout(Duration::from_secs(10), async {
        while let Some(msg_result) = server_stream.next().await {
            let msg = msg_result.unwrap();
            if let Some(server_msg) = msg.msg {
                match server_msg {
                    ServerMsg::ToolRequest(req) => {
                        tool_called = true;
                        let result = if let Some(ref handler) = tool_handler {
                            handler(&req)
                        } else {
                            proto::tool_response::Result::Error(
                                "no handler configured".to_string(),
                            )
                        };
                        client_tx
                            .send(ClientMessage {
                                msg: Some(ClientMsg::ToolResponse(proto::ToolResponse {
                                    request_id: req.request_id,
                                    result: Some(result),
                                })),
                            })
                            .await
                            .unwrap();
                    }
                    ServerMsg::Output(output) => {
                        let text = String::from_utf8_lossy(&output.data).to_string();
                        if output.stream == proto::OutputStream::Stdout as i32 {
                            stdout.push_str(&text);
                        } else {
                            stderr.push_str(&text);
                        }
                    }
                    ServerMsg::Result(r) => {
                        // Also collect from the result message
                        if !r.stdout.is_empty() && stdout.is_empty() {
                            stdout = r.stdout;
                        }
                        if !r.stderr.is_empty() && stderr.is_empty() {
                            stderr = r.stderr;
                        }
                        exit_code = Some(r.exit_code);
                        break;
                    }
                }
            }
        }
    })
    .await;

    assert!(result.is_ok(), "Test timed out");
    (stdout, stderr, exit_code, tool_called)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_simple_echo() {
    let addr = start_test_server().await;
    let mut client = connect_client(addr).await;

    let req = ExecuteRequest {
        script: "echo hello".to_string(),
        tools: vec![],
        limits: None,
        files: vec![],
        network_config: None,
    };

    let (stdout, _, exit_code, _) = run_script(&mut client, req, None).await;

    assert_eq!(exit_code, Some(0));
    assert!(stdout.contains("hello"), "Expected 'hello' in stdout, got: {}", stdout);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_script_with_nonzero_exit_code() {
    let addr = start_test_server().await;
    let mut client = connect_client(addr).await;

    let req = ExecuteRequest {
        script: "exit 7".to_string(),
        tools: vec![],
        limits: None,
        files: vec![],
        network_config: None,
    };

    let (_, _, exit_code, _) = run_script(&mut client, req, None).await;

    assert_eq!(exit_code, Some(7), "Expected exit code 7");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_stderr_output() {
    let addr = start_test_server().await;
    let mut client = connect_client(addr).await;

    let req = ExecuteRequest {
        script: r#"echo "error message" >&2"#.to_string(),
        tools: vec![],
        limits: None,
        files: vec![],
        network_config: None,
    };

    let (_, stderr, _, _) = run_script(&mut client, req, None).await;

    assert!(
        stderr.contains("error message"),
        "Expected 'error message' in stderr, got: {}",
        stderr
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_multiple_commands() {
    let addr = start_test_server().await;
    let mut client = connect_client(addr).await;

    let req = ExecuteRequest {
        script: r#"
            echo "line1"
            echo "line2"
            echo "line3"
        "#
        .to_string(),
        tools: vec![],
        limits: None,
        files: vec![],
        network_config: None,
    };

    let (stdout, _, _, _) = run_script(&mut client, req, None).await;

    assert!(stdout.contains("line1"));
    assert!(stdout.contains("line2"));
    assert!(stdout.contains("line3"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_tool_callback() {
    init_tracing();
    let addr = start_test_server().await;
    let mut client = connect_client(addr).await;

    let req = ExecuteRequest {
        script: r#"tool weather --json '{"city": "London"}'"#.to_string(),
        tools: vec![proto::ToolDeclaration {
            name: "weather".to_string(),
            description: "Get weather for a city".to_string(),
            parameters_schema_json: r#"{"type":"object","properties":{"city":{"type":"string"}}}"#
                .to_string(),
        }],
        limits: None,
        files: vec![],
        network_config: None,
    };

    let handler: Box<dyn Fn(&proto::ToolRequest) -> proto::tool_response::Result + Send> =
        Box::new(|req| {
            assert_eq!(req.name, "weather");
            proto::tool_response::Result::JsonResult(
                r#"{"temp": 15, "condition": "cloudy"}"#.to_string(),
            )
        });

    let (stdout, _, _, tool_called) = run_script(&mut client, req, Some(handler)).await;

    assert!(tool_called, "Tool was not called");
    assert!(
        stdout.contains("temp") || stdout.contains("cloudy"),
        "Expected tool result in stdout, got: {}",
        stdout
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_tool_error_response() {
    init_tracing();
    let addr = start_test_server().await;
    let mut client = connect_client(addr).await;

    let req = ExecuteRequest {
        script: r#"tool failing_tool --json '{}'"#.to_string(),
        tools: vec![proto::ToolDeclaration {
            name: "failing_tool".to_string(),
            description: "A tool that will fail".to_string(),
            parameters_schema_json: r#"{"type":"object"}"#.to_string(),
        }],
        limits: None,
        files: vec![],
        network_config: None,
    };

    let handler: Box<dyn Fn(&proto::ToolRequest) -> proto::tool_response::Result + Send> =
        Box::new(|_| {
            proto::tool_response::Result::Error(
                "Tool execution failed: service unavailable".to_string(),
            )
        });

    let (_, _, _, tool_called) = run_script(&mut client, req, Some(handler)).await;

    assert!(tool_called, "Tool was not called");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_resource_limits() {
    let addr = start_test_server().await;
    let mut client = connect_client(addr).await;

    let req = ExecuteRequest {
        script: "echo 'limited execution'".to_string(),
        tools: vec![],
        limits: Some(proto::ResourceLimits {
            execution_timeout_ms: 5000,
            max_memory_bytes: 32 * 1024 * 1024,
            max_output_bytes: 512 * 1024,
            max_fuel: 0,
        }),
        files: vec![],
        network_config: None,
    };

    let (stdout, _, exit_code, _) = run_script(&mut client, req, None).await;

    assert_eq!(exit_code, Some(0));
    assert!(stdout.contains("limited execution"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_supporting_files() {
    let addr = start_test_server().await;
    let mut client = connect_client(addr).await;

    let req = ExecuteRequest {
        script: "cat /agent/scratch/data.txt".to_string(),
        tools: vec![],
        limits: None,
        files: vec![proto::SupportingFile {
            name: "data.txt".to_string(),
            content: b"hello from supporting file".to_vec(),
        }],
        network_config: None,
    };

    let (stdout, _, exit_code, _) = run_script(&mut client, req, None).await;

    assert_eq!(exit_code, Some(0));
    assert!(
        stdout.contains("hello from supporting file"),
        "Expected supporting file content in stdout, got: {}",
        stdout
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_invalid_first_message() {
    let addr = start_test_server().await;
    let mut client = connect_client(addr).await;

    let (client_tx, client_rx) = mpsc::channel::<ClientMessage>(32);
    let client_stream = ReceiverStream::new(client_rx);

    // Send a ToolResponse as the first message instead of ExecuteRequest
    let client_tx_clone = client_tx.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(10)).await;
        client_tx_clone
            .send(ClientMessage {
                msg: Some(ClientMsg::ToolResponse(proto::ToolResponse {
                    request_id: "1".to_string(),
                    result: Some(proto::tool_response::Result::JsonResult("{}".to_string())),
                })),
            })
            .await
            .unwrap();
    });

    let response = client.execute(client_stream).await;

    assert!(
        response.is_err(),
        "Expected error for invalid first message"
    );
    let status = response.unwrap_err();
    assert_eq!(status.code(), tonic::Code::InvalidArgument);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_empty_stream_closed_early() {
    let addr = start_test_server().await;
    let mut client = connect_client(addr).await;

    // Create an immediately-closed stream
    let client_stream = tokio_stream::iter(std::iter::empty::<ClientMessage>());

    let response = client.execute(client_stream).await;

    assert!(
        response.is_err(),
        "Expected error when stream closes without ExecuteRequest"
    );
    let status = response.unwrap_err();
    assert_eq!(status.code(), tonic::Code::InvalidArgument);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_file_write_and_read() {
    let addr = start_test_server().await;
    let mut client = connect_client(addr).await;

    // Test that the sandbox has a writable filesystem (in-memory)
    let req = ExecuteRequest {
        script: r#"
            echo "written content" > /agent/scratch/test.txt
            cat /agent/scratch/test.txt
        "#
        .to_string(),
        tools: vec![],
        limits: None,
        files: vec![],
        network_config: None,
    };

    let (stdout, _, exit_code, _) = run_script(&mut client, req, None).await;

    assert_eq!(exit_code, Some(0));
    assert!(
        stdout.contains("written content"),
        "Expected written content in stdout, got: {}",
        stdout
    );
}
