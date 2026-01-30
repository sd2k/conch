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
    const result = execute('tool web_search --query "rust async"');
    expect(result.exitCode).toBe(0);
    expect(lastToolRequest).not.toBeNull();
    expect(lastToolRequest!.tool).toBe("web_search");
    const params = JSON.parse(lastToolRequest!.params);
    expect(params.query).toBe("rust async");
    expect(result.stdout).toContain("web_search");
  });

  it("invokes tool with JSON params", () => {
    const result = execute(
      'tool code_edit --json \'{"file": "main.rs", "line": 42}\'',
    );
    expect(result.exitCode).toBe(0);
    expect(lastToolRequest).not.toBeNull();
    expect(lastToolRequest!.tool).toBe("code_edit");
    const params = JSON.parse(lastToolRequest!.params);
    expect(params.file).toBe("main.rs");
    expect(params.line).toBe(42);
  });

  it("fails without tool name", () => {
    const result = execute("tool");
    expect(result.exitCode).toBe(1);
    // Handler should not have been called
    expect(lastToolRequest).toBeNull();
    expect(result.stderr).toContain("missing tool name");
  });

  it("fails with option as tool name", () => {
    const result = execute("tool --query test");
    expect(result.exitCode).toBe(1);
    expect(lastToolRequest).toBeNull();
    expect(result.stderr).toContain("expected tool name");
  });

  it("receives piped stdin data", () => {
    const result = execute('echo "input data" | tool analyze --format json');
    expect(result.exitCode).toBe(0);
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

    const result = execute("tool failing_tool");
    expect(result.exitCode).toBe(1);
    expect(result.stderr).toContain("Tool failed");
  });

  it("handles multiple tool calls in sequence", () => {
    const calls: string[] = [];
    setToolHandler((request) => {
      calls.push(request.tool);
      return { success: true, output: `result-${request.tool}` };
    });

    // Run each tool call separately to avoid subshell issues
    expect(execute("tool first_tool").exitCode).toBe(0);
    expect(execute("tool second_tool").exitCode).toBe(0);
    expect(execute("tool third_tool").exitCode).toBe(0);
    expect(calls).toEqual(["first_tool", "second_tool", "third_tool"]);
  });

  it("returns tool output to shell via command substitution", () => {
    setToolHandler(() => {
      return { success: true, output: "tool result: success!" };
    });
    const result = execute('result=$(tool test_tool); echo "Got: $result"');
    expect(result.exitCode).toBe(0);
    expect(result.stdout).toContain("Got: tool result: success!");
  });
});
