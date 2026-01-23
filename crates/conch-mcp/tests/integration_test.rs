//! Integration tests for the Conch MCP server.
//!
//! These tests spawn the actual MCP server binary and communicate with it
//! over stdio using JSON-RPC, catching issues like nested tokio runtimes
//! that unit tests would miss.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use serde_json::{Value, json};
use tempfile::TempDir;

/// Helper to spawn the MCP server process
struct McpServerProcess {
    child: Child,
}

impl McpServerProcess {
    fn spawn() -> Self {
        Self::spawn_with_args(&[])
    }

    fn spawn_with_args(args: &[&str]) -> Self {
        // Find the binary - try release first, then debug
        let binary = Self::find_binary();

        let child = Command::new(&binary)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap_or_else(|e| panic!("Failed to spawn MCP server at {:?}: {}", binary, e));

        Self { child }
    }

    fn find_binary() -> std::path::PathBuf {
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let workspace_root = std::path::Path::new(manifest_dir)
            .parent()
            .unwrap()
            .parent()
            .unwrap();

        // Try release build first
        let release_path = workspace_root
            .join("target")
            .join("release")
            .join("conch-mcp");
        if release_path.exists() {
            return release_path;
        }

        // Fall back to debug build
        let debug_path = workspace_root
            .join("target")
            .join("debug")
            .join("conch-mcp");
        if debug_path.exists() {
            return debug_path;
        }

        panic!(
            "Could not find conch-mcp binary. Run `cargo build -p conch-mcp` first.\n\
             Searched:\n  - {:?}\n  - {:?}",
            release_path, debug_path
        );
    }

    /// Send a JSON-RPC request and get the response
    fn request(&mut self, request: Value) -> Value {
        let stdin = self.child.stdin.as_mut().expect("stdin not captured");
        let stdout = self.child.stdout.as_mut().expect("stdout not captured");

        // Write the request as a single line
        let request_str = serde_json::to_string(&request).expect("serialize request");
        writeln!(stdin, "{}", request_str).expect("write request");
        stdin.flush().expect("flush stdin");

        // Read the response line
        let mut reader = BufReader::new(stdout);
        let mut response_line = String::new();

        // Use a simple timeout by reading with a deadline
        // In a real implementation we'd use async I/O, but for tests this is simpler
        reader.read_line(&mut response_line).expect("read response");

        serde_json::from_str(&response_line)
            .unwrap_or_else(|e| panic!("parse response '{}': {}", response_line.trim(), e))
    }

    /// Send a notification (no response expected)
    fn notify(&mut self, notification: Value) {
        let stdin = self.child.stdin.as_mut().expect("stdin not captured");
        let notification_str =
            serde_json::to_string(&notification).expect("serialize notification");
        writeln!(stdin, "{}", notification_str).expect("write notification");
        stdin.flush().expect("flush stdin");
    }
}

impl Drop for McpServerProcess {
    fn drop(&mut self) {
        // Try to kill the process gracefully
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Perform MCP initialization handshake
fn initialize(server: &mut McpServerProcess) -> Value {
    // Step 1: Send initialize request
    let init_request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {
                "name": "conch-mcp-test",
                "version": "0.1.0"
            }
        }
    });

    let init_response = server.request(init_request);

    // Verify we got a valid response
    assert_eq!(init_response["jsonrpc"], "2.0");
    assert_eq!(init_response["id"], 1);
    assert!(
        init_response.get("result").is_some(),
        "Expected result in initialize response, got: {}",
        init_response
    );

    // Step 2: Send initialized notification
    let initialized_notification = json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized"
    });
    server.notify(initialized_notification);

    // Give the server a moment to process
    std::thread::sleep(Duration::from_millis(50));

    init_response
}

#[test]
fn test_mcp_initialize() {
    let mut server = McpServerProcess::spawn();
    let response = initialize(&mut server);

    // Check server info in the response
    let result = &response["result"];
    assert!(
        result.get("serverInfo").is_some(),
        "Expected serverInfo in result"
    );
    assert!(
        result.get("capabilities").is_some(),
        "Expected capabilities in result"
    );

    // Verify tools capability is enabled
    let capabilities = &result["capabilities"];
    assert!(
        capabilities.get("tools").is_some(),
        "Expected tools capability"
    );
}

#[test]
fn test_mcp_list_tools() {
    let mut server = McpServerProcess::spawn();
    initialize(&mut server);

    // List available tools
    let list_tools_request = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/list",
        "params": {}
    });

    let response = server.request(list_tools_request);

    assert_eq!(response["jsonrpc"], "2.0");
    assert_eq!(response["id"], 2);
    assert!(
        response.get("result").is_some(),
        "Expected result, got: {}",
        response
    );

    let tools = response["result"]["tools"]
        .as_array()
        .expect("tools should be an array");
    assert!(!tools.is_empty(), "Expected at least one tool");

    // Find the execute tool
    let execute_tool = tools.iter().find(|t| t["name"] == "execute");
    assert!(execute_tool.is_some(), "Expected 'execute' tool");

    let execute_tool = execute_tool.unwrap();
    assert!(
        execute_tool.get("description").is_some(),
        "Tool should have description"
    );
    assert!(
        execute_tool.get("inputSchema").is_some(),
        "Tool should have inputSchema"
    );
}

#[test]
fn test_mcp_execute_echo() {
    let mut server = McpServerProcess::spawn();
    initialize(&mut server);

    // Call the execute tool
    let call_tool_request = json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "tools/call",
        "params": {
            "name": "execute",
            "arguments": {
                "command": "echo hello world"
            }
        }
    });

    let response = server.request(call_tool_request);

    assert_eq!(response["jsonrpc"], "2.0");
    assert_eq!(response["id"], 3);
    assert!(
        response.get("result").is_some(),
        "Expected result, got: {}",
        response
    );

    let result = &response["result"];
    let content = result["content"]
        .as_array()
        .expect("content should be an array");
    assert!(!content.is_empty(), "Expected content");

    // Check the text content contains our output
    let text = content[0]["text"].as_str().expect("text content");
    assert!(
        text.contains("hello world"),
        "Expected 'hello world' in output, got: {}",
        text
    );
}

#[test]
fn test_mcp_execute_pipe() {
    let mut server = McpServerProcess::spawn();
    initialize(&mut server);

    // Test a simple pipe
    let call_tool_request = json!({
        "jsonrpc": "2.0",
        "id": 4,
        "method": "tools/call",
        "params": {
            "name": "execute",
            "arguments": {
                "command": "echo -e 'foo\\nbar\\nbaz' | grep bar"
            }
        }
    });

    let response = server.request(call_tool_request);

    assert_eq!(response["jsonrpc"], "2.0");
    assert!(
        response.get("result").is_some(),
        "Expected result, got: {}",
        response
    );

    let text = response["result"]["content"][0]["text"]
        .as_str()
        .expect("text content");
    assert!(
        text.contains("bar"),
        "Expected 'bar' in grep output, got: {}",
        text
    );
}

#[test]
fn test_mcp_execute_jq() {
    let mut server = McpServerProcess::spawn();
    initialize(&mut server);

    // Test jq builtin
    let call_tool_request = json!({
        "jsonrpc": "2.0",
        "id": 5,
        "method": "tools/call",
        "params": {
            "name": "execute",
            "arguments": {
                "command": "echo '{\"name\":\"conch\"}' | jq -r .name"
            }
        }
    });

    let response = server.request(call_tool_request);

    assert!(
        response.get("result").is_some(),
        "Expected result, got: {}",
        response
    );

    let text = response["result"]["content"][0]["text"]
        .as_str()
        .expect("text content");
    assert!(
        text.contains("conch"),
        "Expected 'conch' in jq output, got: {}",
        text
    );
}

#[test]
fn test_mcp_execute_with_custom_limits() {
    let mut server = McpServerProcess::spawn();
    initialize(&mut server);

    // Call with custom limits
    let call_tool_request = json!({
        "jsonrpc": "2.0",
        "id": 6,
        "method": "tools/call",
        "params": {
            "name": "execute",
            "arguments": {
                "command": "echo test",
                "max_cpu_ms": 1000,
                "timeout_ms": 5000
            }
        }
    });

    let response = server.request(call_tool_request);

    assert!(
        response.get("result").is_some(),
        "Expected result, got: {}",
        response
    );

    let text = response["result"]["content"][0]["text"]
        .as_str()
        .expect("text content");
    assert!(text.contains("test"), "Expected 'test' in output");
}

#[test]
fn test_mcp_execute_exit_code() {
    let mut server = McpServerProcess::spawn();
    initialize(&mut server);

    // Test non-zero exit code
    let call_tool_request = json!({
        "jsonrpc": "2.0",
        "id": 7,
        "method": "tools/call",
        "params": {
            "name": "execute",
            "arguments": {
                "command": "false"
            }
        }
    });

    let response = server.request(call_tool_request);

    assert!(
        response.get("result").is_some(),
        "Expected result even for non-zero exit, got: {}",
        response
    );

    let text = response["result"]["content"][0]["text"]
        .as_str()
        .expect("text content");
    // The output should mention the exit code
    assert!(
        text.contains("exit code: 1") || text.contains("exit code"),
        "Expected exit code in output, got: {}",
        text
    );
}

#[test]
fn test_mcp_unknown_tool() {
    let mut server = McpServerProcess::spawn();
    initialize(&mut server);

    // Try to call a non-existent tool
    let call_tool_request = json!({
        "jsonrpc": "2.0",
        "id": 8,
        "method": "tools/call",
        "params": {
            "name": "nonexistent_tool",
            "arguments": {}
        }
    });

    let response = server.request(call_tool_request);

    // Should get an error response
    assert!(
        response.get("error").is_some(),
        "Expected error for unknown tool, got: {}",
        response
    );
}

#[test]
fn test_mcp_multiple_executions() {
    let mut server = McpServerProcess::spawn();
    initialize(&mut server);

    // Execute multiple commands in sequence
    for i in 0..5 {
        let call_tool_request = json!({
            "jsonrpc": "2.0",
            "id": 100 + i,
            "method": "tools/call",
            "params": {
                "name": "execute",
                "arguments": {
                    "command": format!("echo iteration {}", i)
                }
            }
        });

        let response = server.request(call_tool_request);

        assert!(
            response.get("result").is_some(),
            "Expected result for iteration {}, got: {}",
            i,
            response
        );

        let text = response["result"]["content"][0]["text"]
            .as_str()
            .expect("text content");
        assert!(
            text.contains(&format!("iteration {}", i)),
            "Expected 'iteration {}' in output, got: {}",
            i,
            text
        );
    }
}

#[test]
fn test_mcp_isolation_between_executions() {
    let mut server = McpServerProcess::spawn();
    initialize(&mut server);

    // Set a variable in first execution
    let set_var_request = json!({
        "jsonrpc": "2.0",
        "id": 200,
        "method": "tools/call",
        "params": {
            "name": "execute",
            "arguments": {
                "command": "MY_VAR=secret; echo $MY_VAR"
            }
        }
    });

    let response1 = server.request(set_var_request);
    let text1 = response1["result"]["content"][0]["text"]
        .as_str()
        .expect("text content");
    assert!(
        text1.contains("secret"),
        "First execution should see variable"
    );

    // Try to read the variable in second execution - should not persist
    let read_var_request = json!({
        "jsonrpc": "2.0",
        "id": 201,
        "method": "tools/call",
        "params": {
            "name": "execute",
            "arguments": {
                "command": "echo ${MY_VAR:-unset}"
            }
        }
    });

    let response2 = server.request(read_var_request);
    let text2 = response2["result"]["content"][0]["text"]
        .as_str()
        .expect("text content");
    assert!(
        text2.contains("unset"),
        "Variable should not persist between executions, got: {}",
        text2
    );
}

#[test]
fn test_mcp_mount_readonly() {
    // Create a temp directory with a test file
    let temp_dir = TempDir::new().expect("create temp dir");
    let test_file = temp_dir.path().join("test.txt");
    fs::write(&test_file, "hello from mounted file").expect("write test file");

    // Spawn server with a readonly mount
    let mount_arg = format!("/data:{}:ro", temp_dir.path().display());
    let mut server = McpServerProcess::spawn_with_args(&["--mount", &mount_arg]);
    initialize(&mut server);

    // Read the mounted file
    let call_tool_request = json!({
        "jsonrpc": "2.0",
        "id": 300,
        "method": "tools/call",
        "params": {
            "name": "execute",
            "arguments": {
                "command": "cat /data/test.txt"
            }
        }
    });

    let response = server.request(call_tool_request);

    assert!(
        response.get("result").is_some(),
        "Expected result, got: {}",
        response
    );

    let text = response["result"]["content"][0]["text"]
        .as_str()
        .expect("text content");
    assert!(
        text.contains("hello from mounted file"),
        "Expected file content, got: {}",
        text
    );
}

#[test]
fn test_mcp_mount_readwrite() {
    // Create a temp directory
    let temp_dir = TempDir::new().expect("create temp dir");

    // Spawn server with a readwrite mount
    let mount_arg = format!("/workspace:{}:rw", temp_dir.path().display());
    let mut server = McpServerProcess::spawn_with_args(&["--mount", &mount_arg]);
    initialize(&mut server);

    // Write a file to the mounted directory
    let call_tool_request = json!({
        "jsonrpc": "2.0",
        "id": 400,
        "method": "tools/call",
        "params": {
            "name": "execute",
            "arguments": {
                "command": "echo 'written by shell' > /workspace/output.txt && cat /workspace/output.txt"
            }
        }
    });

    let response = server.request(call_tool_request);

    assert!(
        response.get("result").is_some(),
        "Expected result, got: {}",
        response
    );

    let text = response["result"]["content"][0]["text"]
        .as_str()
        .expect("text content");
    assert!(
        text.contains("written by shell"),
        "Expected written content, got: {}",
        text
    );

    // Verify the file was actually written to the host filesystem
    let written_file = temp_dir.path().join("output.txt");
    assert!(
        written_file.exists(),
        "File should exist on host filesystem"
    );

    let content = fs::read_to_string(&written_file).expect("read written file");
    assert!(
        content.contains("written by shell"),
        "Host file should contain shell output, got: {}",
        content
    );
}

#[test]
fn test_mcp_tool_description_includes_mounts() {
    // Create a temp directory
    let temp_dir = TempDir::new().expect("create temp dir");

    // Spawn server with a mount
    let mount_arg = format!("/mydata:{}:ro", temp_dir.path().display());
    let mut server = McpServerProcess::spawn_with_args(&["--mount", &mount_arg]);
    initialize(&mut server);

    // List tools and check description
    let list_tools_request = json!({
        "jsonrpc": "2.0",
        "id": 500,
        "method": "tools/list",
        "params": {}
    });

    let response = server.request(list_tools_request);

    let tools = response["result"]["tools"].as_array().expect("tools array");
    let execute_tool = tools.iter().find(|t| t["name"] == "execute").unwrap();

    let description = execute_tool["description"].as_str().expect("description");

    assert!(
        description.contains("/mydata"),
        "Tool description should mention mount path, got: {}",
        description
    );
    assert!(
        description.contains("read-only"),
        "Tool description should mention mount mode, got: {}",
        description
    );
}
