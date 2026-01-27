//! Continuation mechanism for tool yield/resume.
//!
//! When an agent invokes a tool via the `tool` builtin, execution yields
//! back to the orchestrator with a [`ToolRequest`]. The orchestrator executes
//! the tool and writes results using [`write_tool_result`].
//!
//! # Execution Flow
//!
//! 1. Agent script calls `tool <name> --param value`
//! 2. Tool builtin writes request to `/tools/pending/<call_id>/request.json`
//! 3. Tool builtin exits with code 42
//! 4. `execute_with_tools()` returns `ExecutionOutcome::ToolRequest`
//! 5. Orchestrator executes the tool externally
//! 6. Orchestrator calls `write_tool_result()` to record the result
//! 7. Agent can read result from `/tools/last_result.json` or `/tools/history/<call_id>/`

use eryx_vfs::VfsStorage;
use serde::{Deserialize, Serialize};

use crate::runtime::{ExecutionResult, RuntimeError};

/// Exit code that signals a tool invocation request.
pub const TOOL_REQUEST_EXIT_CODE: i32 = 42;

/// Outcome of executing a script in an agent sandbox.
#[derive(Debug)]
pub enum ExecutionOutcome {
    /// Script completed normally (or with an error exit code).
    Completed(ExecutionResult),

    /// Script is waiting for a tool to be executed.
    ToolRequest(ToolRequest),
}

/// A request to execute an external tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolRequest {
    /// Unique ID for this tool call (e.g., "call-001").
    pub call_id: String,
    /// Name of the tool to invoke.
    pub tool: String,
    /// Parameters for the tool.
    pub params: serde_json::Value,
    /// Stdin data piped to the tool (if any).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stdin: Option<String>,
    /// Number of bytes if stdin was binary (not UTF-8).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stdin_bytes: Option<usize>,
}

/// Result of a tool execution, to be written to the VFS.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ToolResult {
    /// Tool completed successfully with a JSON value.
    Success(serde_json::Value),
    /// Tool failed with an error message.
    Error {
        /// The error message describing what went wrong.
        error: String,
    },
}

impl ToolResult {
    /// Create a successful result.
    pub fn success(value: serde_json::Value) -> Self {
        Self::Success(value)
    }

    /// Create an error result.
    pub fn error(message: impl Into<String>) -> Self {
        Self::Error {
            error: message.into(),
        }
    }

    /// Check if this is a successful result.
    pub fn is_success(&self) -> bool {
        matches!(self, Self::Success(_))
    }
}

impl From<Result<serde_json::Value, String>> for ToolResult {
    fn from(result: Result<serde_json::Value, String>) -> Self {
        match result {
            Ok(value) => Self::Success(value),
            Err(error) => Self::Error { error },
        }
    }
}

/// Write a tool result to the VFS.
///
/// This moves the request from `/tools/pending/<call_id>/` to
/// `/tools/history/<call_id>/` and writes the result. It also updates
/// `/tools/last_result.json` so scripts can easily read the most recent result.
///
/// # Arguments
///
/// * `storage` - The VFS storage to write to
/// * `call_id` - The ID of the tool call being completed
/// * `result` - The result of the tool execution
///
/// # Errors
///
/// Returns an error if VFS operations fail.
pub async fn write_tool_result(
    storage: &dyn VfsStorage,
    call_id: &str,
    result: ToolResult,
) -> Result<(), RuntimeError> {
    let pending_dir = format!("/tools/pending/{call_id}");
    let history_dir = format!("/tools/history/{call_id}");

    // Create history directory
    storage
        .mkdir(&history_dir)
        .await
        .map_err(|e| RuntimeError::Vfs(e.to_string()))?;

    // Copy request to history
    let request_data = storage
        .read(&format!("{pending_dir}/request.json"))
        .await
        .map_err(|e| RuntimeError::Vfs(format!("Failed to read pending request: {e}")))?;
    storage
        .write(&format!("{history_dir}/request.json"), &request_data)
        .await
        .map_err(|e| RuntimeError::Vfs(e.to_string()))?;

    // Write response or error
    match &result {
        ToolResult::Success(value) => {
            let response_json = serde_json::to_string_pretty(value)
                .map_err(|e| RuntimeError::Vfs(format!("Failed to serialize response: {e}")))?;
            storage
                .write(
                    &format!("{history_dir}/response.json"),
                    response_json.as_bytes(),
                )
                .await
                .map_err(|e| RuntimeError::Vfs(e.to_string()))?;
        }
        ToolResult::Error { error } => {
            let error_json = serde_json::json!({ "error": error });
            let error_str = serde_json::to_string_pretty(&error_json)
                .map_err(|e| RuntimeError::Vfs(format!("Failed to serialize error: {e}")))?;
            storage
                .write(&format!("{history_dir}/error.json"), error_str.as_bytes())
                .await
                .map_err(|e| RuntimeError::Vfs(e.to_string()))?;
        }
    }

    // Write metadata
    let metadata = serde_json::json!({
        "call_id": call_id,
        "completed_at": chrono_lite_now(),
        "success": result.is_success(),
    });
    let metadata_json = serde_json::to_string_pretty(&metadata)
        .map_err(|e| RuntimeError::Vfs(format!("Failed to serialize metadata: {e}")))?;
    storage
        .write(
            &format!("{history_dir}/metadata.json"),
            metadata_json.as_bytes(),
        )
        .await
        .map_err(|e| RuntimeError::Vfs(e.to_string()))?;

    // Remove pending directory contents
    let _ = storage.delete(&format!("{pending_dir}/request.json")).await;
    let _ = storage.rmdir(&pending_dir).await;

    // Write to last_result.json for easy script access
    let last_result_json = match &result {
        ToolResult::Success(value) => serde_json::to_string_pretty(value),
        ToolResult::Error { error } => {
            serde_json::to_string_pretty(&serde_json::json!({ "error": error }))
        }
    }
    .map_err(|e| RuntimeError::Vfs(format!("Failed to serialize last result: {e}")))?;

    storage
        .write("/tools/last_result.json", last_result_json.as_bytes())
        .await
        .map_err(|e| RuntimeError::Vfs(e.to_string()))?;

    Ok(())
}

/// Parse a tool request from the pending directory.
///
/// This is useful for orchestrators that want to re-read a pending request
/// (e.g., after a restart) rather than relying on the initial tool invocation.
pub async fn parse_pending_request(
    storage: &dyn VfsStorage,
    call_id: &str,
) -> Result<ToolRequest, RuntimeError> {
    let request_path = format!("/tools/pending/{call_id}/request.json");
    let request_data = storage
        .read(&request_path)
        .await
        .map_err(|e| RuntimeError::Vfs(format!("Failed to read pending request: {e}")))?;

    let request: ToolRequest = serde_json::from_slice(&request_data)
        .map_err(|e| RuntimeError::Vfs(format!("Failed to parse pending request: {e}")))?;

    Ok(request)
}

/// Find the most recent pending tool request.
///
/// Returns the call_id of a pending request if one exists. This is useful
/// for orchestrators that want to discover pending requests (e.g., after
/// a restart or for debugging).
pub async fn find_pending_request(
    storage: &dyn VfsStorage,
) -> Result<Option<String>, RuntimeError> {
    let entries = storage
        .list("/tools/pending")
        .await
        .map_err(|e| RuntimeError::Vfs(e.to_string()))?;

    // Return the first (and should be only) pending request
    for entry in entries {
        if entry.metadata.is_dir {
            return Ok(Some(entry.name));
        }
    }

    Ok(None)
}

/// Get current time in RFC3339 format.
fn chrono_lite_now() -> String {
    use std::time::SystemTime;

    let now = SystemTime::now();
    let duration = now
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();

    let secs = duration.as_secs();
    let days_since_epoch = secs / 86400;
    let time_of_day = secs % 86400;

    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;

    let (year, month, day) = days_to_ymd(days_since_epoch);

    format!("{year:04}-{month:02}-{day:02}T{hours:02}:{minutes:02}:{seconds:02}Z")
}

/// Convert days since Unix epoch to year/month/day.
fn days_to_ymd(days: u64) -> (u32, u32, u32) {
    let days = days as i64;
    const DAYS_1970_TO_2000: i64 = 10957;
    let days_from_2000 = days - DAYS_1970_TO_2000;
    let mut year = 2000 + (days_from_2000 / 365) as u32;
    let mut day_of_year = days_from_2000 % 365;
    if day_of_year < 0 {
        year -= 1;
        day_of_year += 365;
    }

    let is_leap = year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
    let month_days: [u32; 12] = if is_leap {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };

    let mut month = 1u32;
    let mut remaining = day_of_year as u32;
    for &days_in_month in &month_days {
        if remaining < days_in_month {
            break;
        }
        remaining -= days_in_month;
        month += 1;
    }

    let day = remaining + 1;
    (year, month.min(12), day.clamp(1, 31))
}

#[cfg(test)]
mod tests {
    use super::*;
    use eryx_vfs::InMemoryStorage;

    #[test]
    fn test_tool_request_serialization() {
        let request = ToolRequest {
            call_id: "call-001".to_string(),
            tool: "web_search".to_string(),
            params: serde_json::json!({"query": "test"}),
            stdin: None,
            stdin_bytes: None,
        };

        let json = serde_json::to_string(&request).expect("serialize");
        let parsed: ToolRequest = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(parsed.call_id, "call-001");
        assert_eq!(parsed.tool, "web_search");
        assert_eq!(parsed.params["query"], "test");
    }

    #[test]
    fn test_tool_request_with_stdin() {
        let request = ToolRequest {
            call_id: "call-002".to_string(),
            tool: "analyze".to_string(),
            params: serde_json::json!({}),
            stdin: Some("hello world".to_string()),
            stdin_bytes: None,
        };

        let json = serde_json::to_string(&request).expect("serialize");
        assert!(json.contains("hello world"));
    }

    #[test]
    fn test_tool_result_success() {
        let result = ToolResult::success(serde_json::json!({"data": "test"}));
        assert!(result.is_success());

        let json = serde_json::to_string(&result).expect("serialize");
        assert!(json.contains("data"));
    }

    #[test]
    fn test_tool_result_error() {
        let result = ToolResult::error("something went wrong");
        assert!(!result.is_success());

        let json = serde_json::to_string(&result).expect("serialize");
        assert!(json.contains("something went wrong"));
    }

    #[test]
    fn test_tool_result_from_ok() {
        let result: ToolResult = Ok(serde_json::json!(42)).into();
        assert!(result.is_success());
    }

    #[test]
    fn test_tool_result_from_err() {
        let result: ToolResult = Err("oops".to_string()).into();
        assert!(!result.is_success());
    }

    #[tokio::test]
    async fn test_write_tool_result_success() {
        let storage = InMemoryStorage::new();

        // Create pending request structure
        storage.mkdir("/tools").await.unwrap();
        storage.mkdir("/tools/pending").await.unwrap();
        storage.mkdir("/tools/history").await.unwrap();
        storage.mkdir("/tools/pending/call-001").await.unwrap();

        let request = ToolRequest {
            call_id: "call-001".to_string(),
            tool: "test_tool".to_string(),
            params: serde_json::json!({"arg": "value"}),
            stdin: None,
            stdin_bytes: None,
        };
        let request_json = serde_json::to_string_pretty(&request).unwrap();
        storage
            .write(
                "/tools/pending/call-001/request.json",
                request_json.as_bytes(),
            )
            .await
            .unwrap();

        // Write successful result
        let result = ToolResult::success(serde_json::json!({"result": "success"}));
        write_tool_result(&storage, "call-001", result)
            .await
            .expect("write_tool_result failed");

        // Verify history was created
        let response = storage
            .read("/tools/history/call-001/response.json")
            .await
            .unwrap();
        let response_str = String::from_utf8_lossy(&response);
        assert!(response_str.contains("success"));

        // Verify last_result.json
        let last_result = storage.read("/tools/last_result.json").await.unwrap();
        let last_result_str = String::from_utf8_lossy(&last_result);
        assert!(last_result_str.contains("success"));

        // Verify pending was cleaned up
        assert!(
            storage
                .read("/tools/pending/call-001/request.json")
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn test_write_tool_result_error() {
        let storage = InMemoryStorage::new();

        // Create pending request structure
        storage.mkdir("/tools").await.unwrap();
        storage.mkdir("/tools/pending").await.unwrap();
        storage.mkdir("/tools/history").await.unwrap();
        storage.mkdir("/tools/pending/call-002").await.unwrap();

        let request = ToolRequest {
            call_id: "call-002".to_string(),
            tool: "failing_tool".to_string(),
            params: serde_json::json!({}),
            stdin: None,
            stdin_bytes: None,
        };
        let request_json = serde_json::to_string_pretty(&request).unwrap();
        storage
            .write(
                "/tools/pending/call-002/request.json",
                request_json.as_bytes(),
            )
            .await
            .unwrap();

        // Write error result
        let result = ToolResult::error("Tool execution failed");
        write_tool_result(&storage, "call-002", result)
            .await
            .expect("write_tool_result failed");

        // Verify error was written
        let error = storage
            .read("/tools/history/call-002/error.json")
            .await
            .unwrap();
        let error_str = String::from_utf8_lossy(&error);
        assert!(error_str.contains("Tool execution failed"));

        // Verify metadata shows failure
        let metadata = storage
            .read("/tools/history/call-002/metadata.json")
            .await
            .unwrap();
        let metadata_str = String::from_utf8_lossy(&metadata);
        assert!(metadata_str.contains("\"success\": false"));
    }

    #[tokio::test]
    async fn test_find_pending_request() {
        let storage = InMemoryStorage::new();

        storage.mkdir("/tools").await.unwrap();
        storage.mkdir("/tools/pending").await.unwrap();

        // Initially no pending requests
        assert!(find_pending_request(&storage).await.unwrap().is_none());

        // Add a pending request
        storage.mkdir("/tools/pending/call-003").await.unwrap();
        storage
            .write("/tools/pending/call-003/request.json", b"{}")
            .await
            .unwrap();

        // Now should find it
        let call_id = find_pending_request(&storage).await.unwrap();
        assert_eq!(call_id, Some("call-003".to_string()));
    }

    #[tokio::test]
    async fn test_parse_pending_request() {
        let storage = InMemoryStorage::new();

        storage.mkdir("/tools").await.unwrap();
        storage.mkdir("/tools/pending").await.unwrap();
        storage.mkdir("/tools/pending/call-004").await.unwrap();

        let request = ToolRequest {
            call_id: "call-004".to_string(),
            tool: "my_tool".to_string(),
            params: serde_json::json!({"key": "value"}),
            stdin: Some("input data".to_string()),
            stdin_bytes: None,
        };
        let request_json = serde_json::to_string(&request).unwrap();
        storage
            .write(
                "/tools/pending/call-004/request.json",
                request_json.as_bytes(),
            )
            .await
            .unwrap();

        let parsed = parse_pending_request(&storage, "call-004")
            .await
            .expect("parse failed");
        assert_eq!(parsed.call_id, "call-004");
        assert_eq!(parsed.tool, "my_tool");
        assert_eq!(parsed.params["key"], "value");
        assert_eq!(parsed.stdin, Some("input data".to_string()));
    }
}
