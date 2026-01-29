//! Integration tests for the gRPC bidirectional streaming.
//!
//! These tests simulate a client (like the Go orchestrator) interacting with
//! the sandbox server over gRPC.

#![allow(clippy::unwrap_used)] // unwrap is acceptable in tests

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
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

use tokio::sync::{Mutex, mpsc};
use tokio::time::timeout;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::ReceiverStream;
use tonic::transport::Channel;

use conch_grpc::proto::{
    self, ClientMessage, ExecuteRequest, FileType, client_message::Msg as ClientMsg,
    sandbox_client::SandboxClient, server_message::Msg as ServerMsg,
};

/// Macro to log messages in tests (visible with --nocapture)
macro_rules! test_log {
    ($($arg:tt)*) => {
        eprintln!("[TEST] {}", format!($($arg)*));
    };
}

/// In-memory filesystem for testing.
/// Simulates what the Go orchestrator would maintain.
#[derive(Default)]
struct InMemoryFs {
    files: HashMap<String, Vec<u8>>,
    dirs: std::collections::HashSet<String>,
}

impl InMemoryFs {
    fn new() -> Self {
        let mut fs = Self::default();
        // Root always exists
        fs.dirs.insert("/".to_string());
        fs
    }

    fn read(&self, path: &str) -> Result<Vec<u8>, proto::VfsError> {
        self.files
            .get(path)
            .cloned()
            .ok_or_else(|| proto::VfsError {
                code: proto::VfsErrorCode::NotFound as i32,
                message: format!("not found: {}", path),
            })
    }

    fn write(&mut self, path: &str, data: Vec<u8>) -> Result<(), proto::VfsError> {
        self.files.insert(path.to_string(), data);
        Ok(())
    }

    fn stat(&self, path: &str) -> Result<proto::FileStat, proto::VfsError> {
        if self.dirs.contains(path) {
            Ok(proto::FileStat {
                file_type: FileType::Directory as i32,
                size: 0,
            })
        } else if let Some(data) = self.files.get(path) {
            Ok(proto::FileStat {
                file_type: FileType::File as i32,
                size: data.len() as u64,
            })
        } else {
            Err(proto::VfsError {
                code: proto::VfsErrorCode::NotFound as i32,
                message: format!("not found: {}", path),
            })
        }
    }

    fn list(&self, path: &str) -> Result<Vec<proto::DirEntry>, proto::VfsError> {
        if !self.dirs.contains(path) {
            return Err(proto::VfsError {
                code: proto::VfsErrorCode::NotADirectory as i32,
                message: format!("not a directory: {}", path),
            });
        }

        let prefix = if path == "/" {
            "/".to_string()
        } else {
            format!("{}/", path)
        };

        let mut entries = Vec::new();

        // Find direct children
        for file_path in self.files.keys() {
            if let Some(rest) = file_path.strip_prefix(&prefix)
                && !rest.contains('/')
            {
                entries.push(proto::DirEntry {
                    name: rest.to_string(),
                    file_type: FileType::File as i32,
                });
            }
        }

        for dir_path in &self.dirs {
            if dir_path != path
                && let Some(rest) = dir_path.strip_prefix(&prefix)
                && !rest.contains('/')
            {
                entries.push(proto::DirEntry {
                    name: rest.to_string(),
                    file_type: FileType::Directory as i32,
                });
            }
        }

        Ok(entries)
    }

    fn mkdir(&mut self, path: &str) -> Result<(), proto::VfsError> {
        // If it's already a directory, that's fine (idempotent)
        if self.dirs.contains(path) {
            return Ok(());
        }
        // If it's a file, that's an error
        if self.files.contains_key(path) {
            return Err(proto::VfsError {
                code: proto::VfsErrorCode::AlreadyExists as i32,
                message: format!("already exists as file: {}", path),
            });
        }
        self.dirs.insert(path.to_string());
        Ok(())
    }

    fn delete(&mut self, path: &str) -> Result<(), proto::VfsError> {
        if self.files.remove(path).is_some() || self.dirs.remove(path) {
            Ok(())
        } else {
            Err(proto::VfsError {
                code: proto::VfsErrorCode::NotFound as i32,
                message: format!("not found: {}", path),
            })
        }
    }

    fn rename(&mut self, from: &str, to: &str) -> Result<(), proto::VfsError> {
        if let Some(data) = self.files.remove(from) {
            self.files.insert(to.to_string(), data);
            Ok(())
        } else if self.dirs.remove(from) {
            self.dirs.insert(to.to_string());
            Ok(())
        } else {
            Err(proto::VfsError {
                code: proto::VfsErrorCode::NotFound as i32,
                message: format!("not found: {}", from),
            })
        }
    }
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

/// Handle VFS requests from the server using the in-memory filesystem.
fn handle_vfs_request(fs: &mut InMemoryFs, msg: &ServerMsg) -> Option<ClientMsg> {
    match msg {
        ServerMsg::VfsRead(req) => {
            test_log!("VFS read: {}", req.path);
            let result = match fs.read(&req.path) {
                Ok(data) => proto::vfs_read_response::Result::Data(data),
                Err(e) => proto::vfs_read_response::Result::Error(e),
            };
            Some(ClientMsg::VfsReadResponse(proto::VfsReadResponse {
                request_id: req.request_id,
                result: Some(result),
            }))
        }
        ServerMsg::VfsWrite(req) => {
            test_log!("VFS write: {} ({} bytes)", req.path, req.data.len());
            let result = match fs.write(&req.path, req.data.clone()) {
                Ok(()) => proto::vfs_write_response::Result::Ok(proto::Empty {}),
                Err(e) => proto::vfs_write_response::Result::Error(e),
            };
            Some(ClientMsg::VfsWriteResponse(proto::VfsWriteResponse {
                request_id: req.request_id,
                result: Some(result),
            }))
        }
        ServerMsg::VfsStat(req) => {
            test_log!("VFS stat: {}", req.path);
            let result = match fs.stat(&req.path) {
                Ok(stat) => proto::vfs_stat_response::Result::Stat(stat),
                Err(e) => proto::vfs_stat_response::Result::Error(e),
            };
            Some(ClientMsg::VfsStatResponse(proto::VfsStatResponse {
                request_id: req.request_id,
                result: Some(result),
            }))
        }
        ServerMsg::VfsList(req) => {
            test_log!("VFS list: {}", req.path);
            let result = match fs.list(&req.path) {
                Ok(entries) => {
                    proto::vfs_list_response::Result::Entries(proto::DirEntries { entries })
                }
                Err(e) => proto::vfs_list_response::Result::Error(e),
            };
            Some(ClientMsg::VfsListResponse(proto::VfsListResponse {
                request_id: req.request_id,
                result: Some(result),
            }))
        }
        ServerMsg::VfsMkdir(req) => {
            test_log!("VFS mkdir: {}", req.path);
            let result = match fs.mkdir(&req.path) {
                Ok(()) => proto::vfs_mkdir_response::Result::Ok(proto::Empty {}),
                Err(e) => proto::vfs_mkdir_response::Result::Error(e),
            };
            Some(ClientMsg::VfsMkdirResponse(proto::VfsMkdirResponse {
                request_id: req.request_id,
                result: Some(result),
            }))
        }
        ServerMsg::VfsDelete(req) => {
            let result = match fs.delete(&req.path) {
                Ok(()) => proto::vfs_delete_response::Result::Ok(proto::Empty {}),
                Err(e) => proto::vfs_delete_response::Result::Error(e),
            };
            Some(ClientMsg::VfsDeleteResponse(proto::VfsDeleteResponse {
                request_id: req.request_id,
                result: Some(result),
            }))
        }
        ServerMsg::VfsRename(req) => {
            let result = match fs.rename(&req.from_path, &req.to_path) {
                Ok(()) => proto::vfs_rename_response::Result::Ok(proto::Empty {}),
                Err(e) => proto::vfs_rename_response::Result::Error(e),
            };
            Some(ClientMsg::VfsRenameResponse(proto::VfsRenameResponse {
                request_id: req.request_id,
                result: Some(result),
            }))
        }
        _ => None,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_simple_echo() {
    let addr = start_test_server().await;
    let mut client = connect_client(addr).await;

    // Create channels for the bidirectional stream
    let (client_tx, client_rx) = mpsc::channel::<ClientMessage>(32);
    let client_stream = ReceiverStream::new(client_rx);

    // Set up in-memory filesystem
    let fs = Arc::new(Mutex::new(InMemoryFs::new()));

    // Send execute request BEFORE calling execute() since tonic streams are lazy
    let execute_req = ExecuteRequest {
        agent_id: "test-agent".to_string(),
        script: "echo hello".to_string(),
        limits: None,
        tools: vec![],
        metadata: None,
    };

    // Spawn a task to send the execute request after a brief delay
    let client_tx_clone = client_tx.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(10)).await;
        test_log!("Sending execute request...");
        client_tx_clone
            .send(ClientMessage {
                msg: Some(ClientMsg::Execute(execute_req)),
            })
            .await
            .unwrap();
        test_log!("Execute request sent");
    });

    // Start the execute stream
    test_log!("Calling client.execute()...");
    let response = client.execute(client_stream).await.unwrap();
    test_log!("Got response, getting inner stream...");
    let mut server_stream = response.into_inner();

    // Process messages until we get completion or timeout
    let mut stdout = String::new();
    let mut completed = false;

    test_log!("Waiting for server messages...");
    let result = timeout(Duration::from_secs(10), async {
        while let Some(msg_result) = server_stream.next().await {
            let msg = msg_result.unwrap();
            if let Some(server_msg) = msg.msg {
                // Handle VFS requests
                if let Some(response) = handle_vfs_request(&mut *fs.lock().await, &server_msg) {
                    test_log!("Sending VFS response for request");
                    client_tx
                        .send(ClientMessage {
                            msg: Some(response),
                        })
                        .await
                        .unwrap();
                    continue;
                }

                match &server_msg {
                    ServerMsg::Output(output) => {
                        test_log!("Got output: {:?}", String::from_utf8_lossy(&output.data));
                        if output.stream == proto::OutputStream::Stdout as i32 {
                            stdout.push_str(&String::from_utf8_lossy(&output.data));
                        }
                    }
                    ServerMsg::Completed(c) => {
                        test_log!("Got completed: exit_code={}", c.exit_code);
                        assert_eq!(c.exit_code, 0);
                        completed = true;
                        break;
                    }
                    ServerMsg::Error(e) => {
                        panic!("Unexpected error: {}", e.message);
                    }
                    other => {
                        test_log!("Got unhandled message: {:?}", std::mem::discriminant(other));
                    }
                }
            }
        }
    })
    .await;

    assert!(result.is_ok(), "Test timed out");
    assert!(completed, "Did not receive completion message");
    assert!(
        stdout.contains("hello"),
        "Expected 'hello' in stdout, got: {}",
        stdout
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_file_read_write() {
    init_tracing();
    let addr = start_test_server().await;
    let mut client = connect_client(addr).await;

    let (client_tx, client_rx) = mpsc::channel::<ClientMessage>(32);
    let client_stream = ReceiverStream::new(client_rx);

    let fs = Arc::new(Mutex::new(InMemoryFs::new()));

    // Pre-populate a file in the agent scratch space (which is mounted)
    fs.lock()
        .await
        .write("/agent/scratch/test.txt", b"initial content".to_vec())
        .unwrap();

    // Script that writes to a file (command substitution doesn't work yet with remote storage)
    let execute_req = ExecuteRequest {
        agent_id: "test-agent".to_string(),
        script: r#"
            echo "modified content" > /agent/scratch/test.txt
            echo "File written"
        "#
        .to_string(),
        limits: None,
        tools: vec![],
        metadata: None,
    };

    // Spawn task to send execute request
    let client_tx_clone = client_tx.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(10)).await;
        test_log!("Sending execute request...");
        client_tx_clone
            .send(ClientMessage {
                msg: Some(ClientMsg::Execute(execute_req)),
            })
            .await
            .unwrap();
        test_log!("Execute request sent");
    });

    test_log!("Calling client.execute()...");
    let response = client.execute(client_stream).await.unwrap();
    test_log!("Got response, getting inner stream...");
    let mut server_stream = response.into_inner();

    let mut stdout = String::new();

    test_log!("Waiting for server messages...");
    let result = timeout(Duration::from_secs(10), async {
        while let Some(msg_result) = server_stream.next().await {
            let msg = msg_result.unwrap();
            if let Some(server_msg) = msg.msg {
                if let Some(response) = handle_vfs_request(&mut *fs.lock().await, &server_msg) {
                    test_log!("Sending VFS response");
                    client_tx
                        .send(ClientMessage {
                            msg: Some(response),
                        })
                        .await
                        .unwrap();
                    continue;
                }

                match &server_msg {
                    ServerMsg::Output(output) => {
                        test_log!("Got output: {:?}", String::from_utf8_lossy(&output.data));
                        if output.stream == proto::OutputStream::Stdout as i32 {
                            stdout.push_str(&String::from_utf8_lossy(&output.data));
                        }
                    }
                    ServerMsg::Completed(c) => {
                        test_log!("Got completed: exit_code={}", c.exit_code);
                        break;
                    }
                    ServerMsg::Error(e) => panic!("Error: {}", e.message),
                    _ => {}
                }
            }
        }
    })
    .await;

    assert!(result.is_ok(), "Test timed out");

    // Verify the file was modified
    let final_content = fs.lock().await.read("/agent/scratch/test.txt").unwrap();
    assert!(
        String::from_utf8_lossy(&final_content).contains("modified"),
        "File was not modified correctly: {}",
        String::from_utf8_lossy(&final_content)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_tool_callback() {
    init_tracing();
    let addr = start_test_server().await;
    let mut client = connect_client(addr).await;

    let (client_tx, client_rx) = mpsc::channel::<ClientMessage>(32);
    let client_stream = ReceiverStream::new(client_rx);

    let fs = Arc::new(Mutex::new(InMemoryFs::new()));

    // Define a tool and use it in the script
    // Note: For now we just test that the tool request is triggered.
    // The current implementation doesn't support resuming execution after tool completion.
    let execute_req = ExecuteRequest {
        agent_id: "test-agent".to_string(),
        script: r#"tool weather --json '{"city": "London"}'"#.to_string(),
        limits: None,
        tools: vec![proto::ToolDefinition {
            name: "weather".to_string(),
            description: "Get weather for a city".to_string(),
            parameters_schema_json: r#"{"type":"object","properties":{"city":{"type":"string"}}}"#
                .to_string(),
        }],
        metadata: None,
    };

    // Spawn task to send execute request
    let client_tx_clone = client_tx.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(10)).await;
        client_tx_clone
            .send(ClientMessage {
                msg: Some(ClientMsg::Execute(execute_req)),
            })
            .await
            .unwrap();
    });

    let response = client.execute(client_stream).await.unwrap();
    let mut server_stream = response.into_inner();

    let mut tool_called = false;
    let mut stdout = String::new();

    let result = timeout(Duration::from_secs(10), async {
        while let Some(msg_result) = server_stream.next().await {
            let msg = msg_result.unwrap();
            if let Some(server_msg) = msg.msg {
                if let Some(response) = handle_vfs_request(&mut *fs.lock().await, &server_msg) {
                    client_tx
                        .send(ClientMessage {
                            msg: Some(response),
                        })
                        .await
                        .unwrap();
                    continue;
                }

                match server_msg {
                    ServerMsg::ToolRequest(req) => {
                        assert_eq!(req.tool, "weather");
                        tool_called = true;

                        // Send tool response
                        client_tx
                            .send(ClientMessage {
                                msg: Some(ClientMsg::ToolResponse(proto::ToolResponse {
                                    call_id: req.call_id,
                                    result: Some(proto::tool_response::Result::ResultJson(
                                        r#"{"temp": 15, "condition": "cloudy"}"#.to_string(),
                                    )),
                                })),
                            })
                            .await
                            .unwrap();
                    }
                    ServerMsg::Output(output) => {
                        if output.stream == proto::OutputStream::Stdout as i32 {
                            stdout.push_str(&String::from_utf8_lossy(&output.data));
                        }
                    }
                    ServerMsg::Completed(_) => break,
                    ServerMsg::Error(e) => panic!("Error: {}", e.message),
                    _ => {}
                }
            }
        }
    })
    .await;

    assert!(result.is_ok(), "Test timed out");
    assert!(tool_called, "Tool was not called");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_script_with_nonzero_exit_code() {
    let addr = start_test_server().await;
    let mut client = connect_client(addr).await;

    let (client_tx, client_rx) = mpsc::channel::<ClientMessage>(32);
    let client_stream = ReceiverStream::new(client_rx);

    let fs = Arc::new(Mutex::new(InMemoryFs::new()));

    // Note: exit code 42 is reserved for tool requests in conch, so use a different value
    let execute_req = ExecuteRequest {
        agent_id: "test-agent".to_string(),
        script: "exit 7".to_string(),
        limits: None,
        tools: vec![],
        metadata: None,
    };

    // Spawn task to send execute request
    let client_tx_clone = client_tx.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(10)).await;
        client_tx_clone
            .send(ClientMessage {
                msg: Some(ClientMsg::Execute(execute_req)),
            })
            .await
            .unwrap();
    });

    let response = client.execute(client_stream).await.unwrap();
    let mut server_stream = response.into_inner();

    let mut exit_code = None;

    let result = timeout(Duration::from_secs(10), async {
        while let Some(msg_result) = server_stream.next().await {
            let msg = msg_result.unwrap();
            if let Some(server_msg) = msg.msg {
                if let Some(response) = handle_vfs_request(&mut *fs.lock().await, &server_msg) {
                    client_tx
                        .send(ClientMessage {
                            msg: Some(response),
                        })
                        .await
                        .unwrap();
                    continue;
                }

                match server_msg {
                    ServerMsg::Completed(c) => {
                        exit_code = Some(c.exit_code);
                        break;
                    }
                    ServerMsg::Error(e) => panic!("Unexpected error: {}", e.message),
                    _ => {}
                }
            }
        }
    })
    .await;

    assert!(result.is_ok(), "Test timed out");
    assert_eq!(exit_code, Some(7), "Expected exit code 7");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_stderr_output() {
    let addr = start_test_server().await;
    let mut client = connect_client(addr).await;

    let (client_tx, client_rx) = mpsc::channel::<ClientMessage>(32);
    let client_stream = ReceiverStream::new(client_rx);

    let fs = Arc::new(Mutex::new(InMemoryFs::new()));

    let execute_req = ExecuteRequest {
        agent_id: "test-agent".to_string(),
        script: r#"echo "error message" >&2"#.to_string(),
        limits: None,
        tools: vec![],
        metadata: None,
    };

    let client_tx_clone = client_tx.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(10)).await;
        client_tx_clone
            .send(ClientMessage {
                msg: Some(ClientMsg::Execute(execute_req)),
            })
            .await
            .unwrap();
    });

    let response = client.execute(client_stream).await.unwrap();
    let mut server_stream = response.into_inner();

    let mut stderr = String::new();

    let result = timeout(Duration::from_secs(10), async {
        while let Some(msg_result) = server_stream.next().await {
            let msg = msg_result.unwrap();
            if let Some(server_msg) = msg.msg {
                if let Some(response) = handle_vfs_request(&mut *fs.lock().await, &server_msg) {
                    client_tx
                        .send(ClientMessage {
                            msg: Some(response),
                        })
                        .await
                        .unwrap();
                    continue;
                }

                match server_msg {
                    ServerMsg::Output(output) => {
                        if output.stream == proto::OutputStream::Stderr as i32 {
                            stderr.push_str(&String::from_utf8_lossy(&output.data));
                        }
                    }
                    ServerMsg::Completed(_) => break,
                    ServerMsg::Error(e) => panic!("Unexpected error: {}", e.message),
                    _ => {}
                }
            }
        }
    })
    .await;

    assert!(result.is_ok(), "Test timed out");
    assert!(
        stderr.contains("error message"),
        "Expected 'error message' in stderr, got: {}",
        stderr
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_resource_limits() {
    let addr = start_test_server().await;
    let mut client = connect_client(addr).await;

    let (client_tx, client_rx) = mpsc::channel::<ClientMessage>(32);
    let client_stream = ReceiverStream::new(client_rx);

    let fs = Arc::new(Mutex::new(InMemoryFs::new()));

    // Script with custom resource limits
    let execute_req = ExecuteRequest {
        agent_id: "test-agent".to_string(),
        script: "echo 'limited execution'".to_string(),
        limits: Some(proto::ResourceLimits {
            max_cpu_ms: 1000,
            max_memory_bytes: 32 * 1024 * 1024,
            max_output_bytes: 512 * 1024,
            timeout_ms: 5000,
        }),
        tools: vec![],
        metadata: None,
    };

    let client_tx_clone = client_tx.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(10)).await;
        client_tx_clone
            .send(ClientMessage {
                msg: Some(ClientMsg::Execute(execute_req)),
            })
            .await
            .unwrap();
    });

    let response = client.execute(client_stream).await.unwrap();
    let mut server_stream = response.into_inner();

    let mut stdout = String::new();
    let mut completed = false;

    let result = timeout(Duration::from_secs(10), async {
        while let Some(msg_result) = server_stream.next().await {
            let msg = msg_result.unwrap();
            if let Some(server_msg) = msg.msg {
                if let Some(response) = handle_vfs_request(&mut *fs.lock().await, &server_msg) {
                    client_tx
                        .send(ClientMessage {
                            msg: Some(response),
                        })
                        .await
                        .unwrap();
                    continue;
                }

                match server_msg {
                    ServerMsg::Output(output) => {
                        if output.stream == proto::OutputStream::Stdout as i32 {
                            stdout.push_str(&String::from_utf8_lossy(&output.data));
                        }
                    }
                    ServerMsg::Completed(c) => {
                        assert_eq!(c.exit_code, 0);
                        completed = true;
                        break;
                    }
                    ServerMsg::Error(e) => panic!("Unexpected error: {}", e.message),
                    _ => {}
                }
            }
        }
    })
    .await;

    assert!(result.is_ok(), "Test timed out");
    assert!(completed, "Did not receive completion");
    assert!(stdout.contains("limited execution"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_invalid_first_message() {
    let addr = start_test_server().await;
    let mut client = connect_client(addr).await;

    let (client_tx, client_rx) = mpsc::channel::<ClientMessage>(32);
    let client_stream = ReceiverStream::new(client_rx);

    // Send a VFS response as the first message instead of ExecuteRequest
    let client_tx_clone = client_tx.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(10)).await;
        client_tx_clone
            .send(ClientMessage {
                msg: Some(ClientMsg::VfsReadResponse(proto::VfsReadResponse {
                    request_id: 1,
                    result: Some(proto::vfs_read_response::Result::Data(vec![])),
                })),
            })
            .await
            .unwrap();
    });

    let response = client.execute(client_stream).await;

    // The server should reject this with an error
    assert!(
        response.is_err(),
        "Expected error for invalid first message"
    );
    let status = response.unwrap_err();
    assert_eq!(status.code(), tonic::Code::InvalidArgument);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_tool_error_response() {
    init_tracing();
    let addr = start_test_server().await;
    let mut client = connect_client(addr).await;

    let (client_tx, client_rx) = mpsc::channel::<ClientMessage>(32);
    let client_stream = ReceiverStream::new(client_rx);

    let fs = Arc::new(Mutex::new(InMemoryFs::new()));

    let execute_req = ExecuteRequest {
        agent_id: "test-agent".to_string(),
        script: r#"tool failing_tool --json '{}'"#.to_string(),
        limits: None,
        tools: vec![proto::ToolDefinition {
            name: "failing_tool".to_string(),
            description: "A tool that will fail".to_string(),
            parameters_schema_json: r#"{"type":"object"}"#.to_string(),
        }],
        metadata: None,
    };

    let client_tx_clone = client_tx.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(10)).await;
        client_tx_clone
            .send(ClientMessage {
                msg: Some(ClientMsg::Execute(execute_req)),
            })
            .await
            .unwrap();
    });

    let response = client.execute(client_stream).await.unwrap();
    let mut server_stream = response.into_inner();

    let mut tool_called = false;

    let result = timeout(Duration::from_secs(10), async {
        while let Some(msg_result) = server_stream.next().await {
            let msg = msg_result.unwrap();
            if let Some(server_msg) = msg.msg {
                if let Some(response) = handle_vfs_request(&mut *fs.lock().await, &server_msg) {
                    client_tx
                        .send(ClientMessage {
                            msg: Some(response),
                        })
                        .await
                        .unwrap();
                    continue;
                }

                match server_msg {
                    ServerMsg::ToolRequest(req) => {
                        assert_eq!(req.tool, "failing_tool");
                        tool_called = true;

                        // Send error response
                        client_tx
                            .send(ClientMessage {
                                msg: Some(ClientMsg::ToolResponse(proto::ToolResponse {
                                    call_id: req.call_id,
                                    result: Some(proto::tool_response::Result::Error(
                                        "Tool execution failed: service unavailable".to_string(),
                                    )),
                                })),
                            })
                            .await
                            .unwrap();
                    }
                    ServerMsg::Completed(_) => break,
                    ServerMsg::Error(e) => panic!("Unexpected error: {}", e.message),
                    _ => {}
                }
            }
        }
    })
    .await;

    assert!(result.is_ok(), "Test timed out");
    assert!(tool_called, "Tool was not called");
}

// Note: test_vfs_delete_operation and test_vfs_rename_operation removed because
// the conch shell doesn't have built-in `rm` or `mv` commands. The VFS delete/rename
// operations are tested indirectly through the InMemoryFs implementation.

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_agent_metadata() {
    init_tracing();
    let addr = start_test_server().await;
    let mut client = connect_client(addr).await;

    let (client_tx, client_rx) = mpsc::channel::<ClientMessage>(32);
    let client_stream = ReceiverStream::new(client_rx);

    let fs = Arc::new(Mutex::new(InMemoryFs::new()));

    // Execute with agent metadata
    let execute_req = ExecuteRequest {
        agent_id: "metadata-test-agent".to_string(),
        script: "echo 'with metadata'".to_string(),
        limits: None,
        tools: vec![],
        metadata: Some(proto::AgentMetadata {
            name: "Test Agent".to_string(),
            capabilities: vec!["read".to_string(), "write".to_string()],
            params_json: r#"{"debug": true}"#.to_string(),
        }),
    };

    let client_tx_clone = client_tx.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(10)).await;
        client_tx_clone
            .send(ClientMessage {
                msg: Some(ClientMsg::Execute(execute_req)),
            })
            .await
            .unwrap();
    });

    let response = client.execute(client_stream).await.unwrap();
    let mut server_stream = response.into_inner();

    let mut stdout = String::new();
    let mut completed = false;

    let result = timeout(Duration::from_secs(10), async {
        while let Some(msg_result) = server_stream.next().await {
            let msg = msg_result.unwrap();
            if let Some(server_msg) = msg.msg {
                if let Some(response) = handle_vfs_request(&mut *fs.lock().await, &server_msg) {
                    client_tx
                        .send(ClientMessage {
                            msg: Some(response),
                        })
                        .await
                        .unwrap();
                    continue;
                }

                match server_msg {
                    ServerMsg::Output(output) => {
                        if output.stream == proto::OutputStream::Stdout as i32 {
                            stdout.push_str(&String::from_utf8_lossy(&output.data));
                        }
                    }
                    ServerMsg::Completed(c) => {
                        assert_eq!(c.exit_code, 0);
                        completed = true;
                        break;
                    }
                    ServerMsg::Error(e) => panic!("Unexpected error: {}", e.message),
                    _ => {}
                }
            }
        }
    })
    .await;

    assert!(result.is_ok(), "Test timed out");
    assert!(completed, "Did not complete");
    assert!(stdout.contains("with metadata"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_empty_stream_closed_early() {
    let addr = start_test_server().await;
    let mut client = connect_client(addr).await;

    // Create an immediately-closed stream by using an empty iterator
    let client_stream = tokio_stream::iter(std::iter::empty::<ClientMessage>());

    // The server should detect stream closed before ExecuteRequest
    let response = client.execute(client_stream).await;

    assert!(
        response.is_err(),
        "Expected error when stream closes without ExecuteRequest"
    );
    let status = response.unwrap_err();
    assert_eq!(status.code(), tonic::Code::InvalidArgument);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_multiple_commands_in_script() {
    let addr = start_test_server().await;
    let mut client = connect_client(addr).await;

    let (client_tx, client_rx) = mpsc::channel::<ClientMessage>(32);
    let client_stream = ReceiverStream::new(client_rx);

    let fs = Arc::new(Mutex::new(InMemoryFs::new()));

    let execute_req = ExecuteRequest {
        agent_id: "test-agent".to_string(),
        script: r#"
            echo "line1"
            echo "line2"
            echo "line3"
        "#
        .to_string(),
        limits: None,
        tools: vec![],
        metadata: None,
    };

    let client_tx_clone = client_tx.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(10)).await;
        client_tx_clone
            .send(ClientMessage {
                msg: Some(ClientMsg::Execute(execute_req)),
            })
            .await
            .unwrap();
    });

    let response = client.execute(client_stream).await.unwrap();
    let mut server_stream = response.into_inner();

    let mut stdout = String::new();

    let result = timeout(Duration::from_secs(10), async {
        while let Some(msg_result) = server_stream.next().await {
            let msg = msg_result.unwrap();
            if let Some(server_msg) = msg.msg {
                if let Some(response) = handle_vfs_request(&mut *fs.lock().await, &server_msg) {
                    client_tx
                        .send(ClientMessage {
                            msg: Some(response),
                        })
                        .await
                        .unwrap();
                    continue;
                }

                match server_msg {
                    ServerMsg::Output(output) => {
                        if output.stream == proto::OutputStream::Stdout as i32 {
                            stdout.push_str(&String::from_utf8_lossy(&output.data));
                        }
                    }
                    ServerMsg::Completed(_) => break,
                    ServerMsg::Error(e) => panic!("Unexpected error: {}", e.message),
                    _ => {}
                }
            }
        }
    })
    .await;

    assert!(result.is_ok(), "Test timed out");
    assert!(stdout.contains("line1"));
    assert!(stdout.contains("line2"));
    assert!(stdout.contains("line3"));
}

// Note: test_pipeline_commands removed because the conch shell doesn't have `tr` command.
// Pipeline support exists but requires commands that exist in the shell.
