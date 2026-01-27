//! tool builtin - invoke external tools from the sandbox
//!
//! The `tool` builtin allows agents to invoke external tools that are executed
//! by the orchestrator. This is the bridge between sandbox execution and
//! external capabilities like web search, code editing, etc.
//!
//! # Syntax
//!
//! ```bash
//! tool <name> [--param value]... [--json '{"key": "value"}']
//! ```
//!
//! # Examples
//!
//! ```bash
//! # Basic invocation with named parameters
//! tool web_search --query "rust async patterns"
//!
//! # With JSON parameters
//! tool code_edit --json '{"file": "src/main.rs", "instruction": "add error handling"}'
//!
//! # Piping data to a tool
//! cat /workspace/data.json | tool analyze_data --format json
//! ```
//!
//! # Output
//!
//! The tool builtin writes a JSON request to stdout describing the tool invocation.
//! In a full agent sandbox, this triggers a yield to the orchestrator.

use std::collections::HashMap;
use std::io::{Read, Write};

use brush_core::{ExecutionContext, ExecutionResult, ShellExtensions, builtins, error};

/// Exit code used to signal a tool invocation request.
///
/// When the sandbox sees this exit code, it parses stdout as a tool request
/// JSON and yields to the orchestrator.
pub const TOOL_REQUEST_EXIT_CODE: u8 = 42;

/// The tool builtin command.
pub struct ToolCommand;

/// Parsed tool invocation request.
#[derive(Debug)]
struct ToolRequest {
    /// Tool name to invoke.
    tool: String,
    /// Parameters for the tool.
    params: serde_json::Value,
    /// Stdin data piped to the tool (if any).
    stdin: Option<Vec<u8>>,
}

impl builtins::SimpleCommand for ToolCommand {
    fn get_content(
        _name: &str,
        content_type: builtins::ContentType,
        _options: &builtins::ContentOptions,
    ) -> Result<String, brush_core::Error> {
        match content_type {
            builtins::ContentType::DetailedHelp => {
                Ok("Invoke an external tool from the sandbox.\n\n\
                 Usage: tool <name> [--param value]... [--json '{...}']\n\n\
                 The tool command requests execution of an external tool by the orchestrator.\n\
                 Parameters can be passed as --key value pairs or as a JSON object with --json.\n\n\
                 Examples:\n\
                   tool web_search --query \"rust async\"\n\
                   tool code_edit --json '{\"file\": \"main.rs\", \"instruction\": \"fix bug\"}'\n\
                   cat data.json | tool analyze --format json"
                    .into())
            }
            builtins::ContentType::ShortUsage => {
                Ok("tool <name> [--param value]... [--json '{...}']".into())
            }
            builtins::ContentType::ShortDescription => Ok("tool - invoke an external tool".into()),
            builtins::ContentType::ManPage => error::unimp("man page not yet implemented"),
        }
    }

    fn execute<SE: ShellExtensions, I: Iterator<Item = S>, S: AsRef<str>>(
        context: ExecutionContext<'_, SE>,
        args: I,
    ) -> Result<ExecutionResult, brush_core::Error> {
        let args: Vec<String> = args.map(|s| s.as_ref().to_string()).collect();

        // Parse the tool request
        let request = match parse_tool_args(&args) {
            Ok(req) => req,
            Err(e) => {
                writeln!(context.stderr(), "tool: {}", e)?;
                return Ok(ExecutionResult::new(1));
            }
        };

        // Check if there's stdin data
        let stdin_data = {
            let mut stdin = context.stdin();
            let mut buf = Vec::new();
            // Only read stdin if it's not a TTY (i.e., something is piped)
            // In WASM, we can try to read and check if we get data
            match stdin.read_to_end(&mut buf) {
                Ok(0) => None,
                Ok(_) => Some(buf),
                Err(_) => None,
            }
        };

        let request = ToolRequest {
            tool: request.tool,
            params: request.params,
            stdin: stdin_data,
        };

        // Output the request as JSON to stdout
        // The orchestrator (AgentSandbox) will:
        // 1. Detect the exit code 42
        // 2. Parse stdout as a ToolRequest
        // 3. Write to /tools/pending/<call_id>/request.json
        let output = build_request_json(&request);
        writeln!(context.stdout(), "{}", output)?;
        context.stdout().flush()?;

        // Exit with TOOL_REQUEST_EXIT_CODE to signal yield to orchestrator
        Ok(ExecutionResult::new(TOOL_REQUEST_EXIT_CODE))
    }
}

/// Parse tool arguments into a ToolRequest.
fn parse_tool_args(args: &[String]) -> Result<ToolRequest, String> {
    let mut iter = args.iter().skip(1); // Skip "tool" itself

    // First argument must be the tool name
    let tool = iter
        .next()
        .ok_or_else(|| "missing tool name".to_string())?
        .clone();

    if tool.starts_with('-') {
        return Err(format!("expected tool name, got option: {}", tool));
    }

    let mut params: HashMap<String, serde_json::Value> = HashMap::new();
    let mut json_params: Option<serde_json::Value> = None;

    while let Some(arg) = iter.next() {
        if arg == "--json" {
            // Next argument is a JSON object
            let json_str = iter
                .next()
                .ok_or_else(|| "--json requires a JSON argument".to_string())?;

            let parsed: serde_json::Value =
                serde_json::from_str(json_str).map_err(|e| format!("invalid JSON: {}", e))?;

            if !parsed.is_object() {
                return Err("--json argument must be a JSON object".to_string());
            }

            json_params = Some(parsed);
        } else if let Some(key) = arg.strip_prefix("--") {
            // --key value pair
            if key.is_empty() {
                return Err("empty parameter name".to_string());
            }

            let value = iter
                .next()
                .ok_or_else(|| format!("--{} requires a value", key))?;

            // Try to parse as JSON value, fall back to string
            let json_value = parse_value_as_json(value);
            params.insert(key.to_string(), json_value);
        } else {
            return Err(format!("unexpected argument: {}", arg));
        }
    }

    // Merge --json params with individual params (individual params take precedence)
    let final_params = if let Some(mut json_obj) = json_params {
        if let Some(obj) = json_obj.as_object_mut() {
            for (k, v) in params {
                obj.insert(k, v);
            }
        }
        json_obj
    } else {
        serde_json::Value::Object(params.into_iter().collect())
    };

    Ok(ToolRequest {
        tool,
        params: final_params,
        stdin: None, // Filled in later
    })
}

/// Try to parse a value as JSON, falling back to a string.
fn parse_value_as_json(value: &str) -> serde_json::Value {
    // Try parsing as JSON first
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(value) {
        return v;
    }

    // Otherwise, treat as string
    serde_json::Value::String(value.to_string())
}

/// Build the JSON output for a tool request.
fn build_request_json(request: &ToolRequest) -> String {
    let mut obj = serde_json::json!({
        "tool": request.tool,
        "params": request.params,
    });

    if let Some(stdin) = &request.stdin {
        // Include stdin as base64 if binary, or as string if valid UTF-8
        if let Ok(s) = std::str::from_utf8(stdin) {
            obj["stdin"] = serde_json::Value::String(s.to_string());
        } else {
            // For binary data, we'd use base64, but for now just note it
            obj["stdin_bytes"] = serde_json::Value::Number(stdin.len().into());
        }
    }

    serde_json::to_string_pretty(&obj).unwrap_or_else(|_| "{}".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_basic_tool() {
        let args = vec!["tool".to_string(), "web_search".to_string()];
        let req = parse_tool_args(&args).expect("parse failed");
        assert_eq!(req.tool, "web_search");
        assert!(req.params.as_object().map_or(false, |o| o.is_empty()));
    }

    #[test]
    fn test_parse_with_params() {
        let args = vec![
            "tool".to_string(),
            "web_search".to_string(),
            "--query".to_string(),
            "rust async".to_string(),
            "--num_results".to_string(),
            "10".to_string(),
        ];
        let req = parse_tool_args(&args).expect("parse failed");
        assert_eq!(req.tool, "web_search");
        assert_eq!(req.params["query"], "rust async");
        assert_eq!(req.params["num_results"], 10); // Parsed as number
    }

    #[test]
    fn test_parse_with_json() {
        let args = vec![
            "tool".to_string(),
            "code_edit".to_string(),
            "--json".to_string(),
            r#"{"file": "main.rs", "line": 42}"#.to_string(),
        ];
        let req = parse_tool_args(&args).expect("parse failed");
        assert_eq!(req.tool, "code_edit");
        assert_eq!(req.params["file"], "main.rs");
        assert_eq!(req.params["line"], 42);
    }

    #[test]
    fn test_parse_json_with_override() {
        let args = vec![
            "tool".to_string(),
            "test".to_string(),
            "--json".to_string(),
            r#"{"a": 1, "b": 2}"#.to_string(),
            "--b".to_string(),
            "3".to_string(),
        ];
        let req = parse_tool_args(&args).expect("parse failed");
        assert_eq!(req.params["a"], 1);
        assert_eq!(req.params["b"], 3); // Overridden
    }

    #[test]
    fn test_parse_missing_tool_name() {
        let args = vec!["tool".to_string()];
        let err = parse_tool_args(&args).expect_err("should fail");
        assert!(err.contains("missing tool name"));
    }

    #[test]
    fn test_parse_tool_name_is_option() {
        let args = vec!["tool".to_string(), "--query".to_string()];
        let err = parse_tool_args(&args).expect_err("should fail");
        assert!(err.contains("expected tool name"));
    }

    #[test]
    fn test_parse_missing_param_value() {
        let args = vec![
            "tool".to_string(),
            "test".to_string(),
            "--param".to_string(),
        ];
        let err = parse_tool_args(&args).expect_err("should fail");
        assert!(err.contains("requires a value"));
    }

    #[test]
    fn test_parse_invalid_json() {
        let args = vec![
            "tool".to_string(),
            "test".to_string(),
            "--json".to_string(),
            "not json".to_string(),
        ];
        let err = parse_tool_args(&args).expect_err("should fail");
        assert!(err.contains("invalid JSON"));
    }

    #[test]
    fn test_parse_value_as_json_number() {
        assert_eq!(parse_value_as_json("42"), serde_json::json!(42));
        assert_eq!(parse_value_as_json("3.14"), serde_json::json!(3.14));
    }

    #[test]
    fn test_parse_value_as_json_bool() {
        assert_eq!(parse_value_as_json("true"), serde_json::json!(true));
        assert_eq!(parse_value_as_json("false"), serde_json::json!(false));
    }

    #[test]
    fn test_parse_value_as_json_null() {
        assert_eq!(parse_value_as_json("null"), serde_json::json!(null));
    }

    #[test]
    fn test_parse_value_as_json_string() {
        assert_eq!(
            parse_value_as_json("hello world"),
            serde_json::json!("hello world")
        );
    }

    #[test]
    fn test_parse_value_as_json_array() {
        assert_eq!(
            parse_value_as_json("[1, 2, 3]"),
            serde_json::json!([1, 2, 3])
        );
    }

    #[test]
    fn test_build_request_json() {
        let req = ToolRequest {
            tool: "test".to_string(),
            params: serde_json::json!({"key": "value"}),
            stdin: None,
        };
        let json = build_request_json(&req);
        assert!(json.contains("\"tool\": \"test\""));
        assert!(json.contains("\"key\": \"value\""));
    }

    #[test]
    fn test_build_request_json_with_stdin() {
        let req = ToolRequest {
            tool: "test".to_string(),
            params: serde_json::json!({}),
            stdin: Some(b"hello world".to_vec()),
        };
        let json = build_request_json(&req);
        assert!(json.contains("\"stdin\": \"hello world\""));
    }
}
