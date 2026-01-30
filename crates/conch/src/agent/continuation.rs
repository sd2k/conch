//! Tool request and result types for agent sandboxes.
//!
//! This module provides types for serializing tool requests and results to the VFS.
//! With the callback-based tool handler approach, these types are primarily used
//! for VFS history recording and compatibility with external systems.
//!
//! # Note
//!
//! The old yield/resume mechanism using exit code 42 has been removed.
//! Tool invocation now uses the callback-based `ToolHandler` trait from
//! `crate::executor`.

use eryx_vfs::VfsStorage;
use serde::{Deserialize, Serialize};

use crate::runtime::RuntimeError;

/// A request to execute an external tool.
///
/// This type is used for:
/// - Serializing tool requests to VFS history
/// - Compatibility with external orchestration systems
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolRequest {
    /// Unique ID for this tool call (e.g., "call-001").
    pub call_id: String,
    /// Name of the tool to invoke.
    pub tool: String,
    /// Parameters for the tool (JSON-encoded).
    pub params: serde_json::Value,
    /// Stdin data piped to the tool (if any).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stdin: Option<String>,
    /// Number of bytes if stdin was binary (not UTF-8).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stdin_bytes: Option<usize>,
}

/// Result of a tool execution, for VFS serialization.
///
/// This type is used for:
/// - Writing tool results to VFS history
/// - Compatibility with external orchestration systems
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

/// Write a tool result to the VFS history.
///
/// This records a tool call in `/tools/history/<call_id>/` and updates
/// `/tools/last_result.json` for easy script access.
///
/// # Arguments
///
/// * `storage` - The VFS storage to write to
/// * `call_id` - The ID of the tool call
/// * `request` - The original tool request
/// * `result` - The result of the tool execution
///
/// # Errors
///
/// Returns an error if VFS operations fail.
pub async fn write_tool_history(
    storage: &dyn VfsStorage,
    call_id: &str,
    request: &ToolRequest,
    result: &ToolResult,
) -> Result<(), RuntimeError> {
    let history_dir = format!("/tools/history/{call_id}");

    // Create history directory
    storage
        .mkdir(&history_dir)
        .await
        .map_err(|e| RuntimeError::Vfs(e.to_string()))?;

    // Write request
    let request_json = serde_json::to_string_pretty(request)
        .map_err(|e| RuntimeError::Vfs(format!("Failed to serialize request: {e}")))?;
    storage
        .write(
            &format!("{history_dir}/request.json"),
            request_json.as_bytes(),
        )
        .await
        .map_err(|e| RuntimeError::Vfs(e.to_string()))?;

    // Write response or error
    match result {
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
        "tool": request.tool,
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

    // Write to last_result.json for easy script access
    let last_result_json = match result {
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
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

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
}
