/**
 * Tool builtin tests
 *
 * The tool builtin allows agents to invoke external tools from the shell.
 * It calls the host-provided invoke-tool function (via setToolHandler) and
 * writes the result to stdout.
 */
import { describe, expect, it, beforeEach } from "vitest";

import { execute } from "@bsull/conch";
// Initialize stdin with empty handler to prevent crashes when reading stdin
import "@bsull/conch/stdin";
import {
  setToolHandler,
  type ToolRequest,
  type ToolResult,
} from "@bsull/conch/tool-handler";

describe("tool builtin", () => {
  // Track tool calls for assertions
  let lastToolRequest: ToolRequest | null = null;

  beforeEach(() => {
    lastToolRequest = null;
    // Set up a default handler that records the request
    setToolHandler((request) => {
      lastToolRequest = request;
      return {
        success: true,
        output: JSON.stringify({ tool: request.tool, received: true }),
      };
    });
  });

  it("invokes tool with basic params", () => {
    const exitCode = execute('tool web_search --query "rust async"');
    expect(exitCode).toBe(0);
    expect(lastToolRequest).not.toBeNull();
    expect(lastToolRequest!.tool).toBe("web_search");
    const params = JSON.parse(lastToolRequest!.params);
    expect(params.query).toBe("rust async");
  });

  it("invokes tool with JSON params", () => {
    const exitCode = execute(
      'tool code_edit --json \'{"file": "main.rs", "line": 42}\'',
    );
    expect(exitCode).toBe(0);
    expect(lastToolRequest).not.toBeNull();
    expect(lastToolRequest!.tool).toBe("code_edit");
    const params = JSON.parse(lastToolRequest!.params);
    expect(params.file).toBe("main.rs");
    expect(params.line).toBe(42);
  });

  it("fails without tool name", () => {
    const exitCode = execute("tool");
    expect(exitCode).toBe(1);
    // Handler should not have been called
    expect(lastToolRequest).toBeNull();
  });

  it("fails with option as tool name", () => {
    const exitCode = execute("tool --query test");
    expect(exitCode).toBe(1);
    expect(lastToolRequest).toBeNull();
  });

  it("receives piped stdin data", () => {
    const exitCode = execute('echo "input data" | tool analyze --format json');
    expect(exitCode).toBe(0);
    expect(lastToolRequest).not.toBeNull();
    expect(lastToolRequest!.tool).toBe("analyze");
    // stdin should contain the piped data
    expect(lastToolRequest!.stdin).toBeDefined();
    const stdinText = new TextDecoder().decode(lastToolRequest!.stdin);
    expect(stdinText).toBe("input data\n");
  });

  it("handles tool failure", () => {
    setToolHandler(() => {
      return {
        success: false,
        output: "Tool failed: something went wrong",
      };
    });

    const exitCode = execute("tool failing_tool");
    expect(exitCode).toBe(1);
  });

  it("handles multiple tool calls in sequence", () => {
    const calls: string[] = [];
    setToolHandler((request) => {
      calls.push(request.tool);
      return { success: true, output: `result-${request.tool}` };
    });

    // Run each tool call separately to avoid subshell issues
    expect(execute("tool first_tool")).toBe(0);
    expect(execute("tool second_tool")).toBe(0);
    expect(execute("tool third_tool")).toBe(0);
    expect(calls).toEqual(["first_tool", "second_tool", "third_tool"]);
  });

  // TODO: Command substitution causes tokio runtime issues in WASM
  // it("returns tool output to shell", () => {
  //   setToolHandler((request) => {
  //     return { success: true, output: "tool result: success!" };
  //   });
  //   const exitCode = execute('result=$(tool test_tool); echo "Got: $result"');
  //   expect(exitCode).toBe(0);
  // });
});
