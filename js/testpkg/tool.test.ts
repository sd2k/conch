/**
 * Tool builtin tests
 *
 * The tool builtin allows agents to invoke external tools from the shell.
 * It outputs a JSON request to stdout and exits with code 42 to signal
 * the orchestrator to execute the tool.
 */
import { describe, expect, it } from "vitest";

import { execute } from "@bsull/conch";

describe("tool builtin", () => {
  // The tool builtin exits with code 42 to signal a tool request
  const TOOL_REQUEST_EXIT_CODE = 42;

  it("invokes tool with basic params", () => {
    const exitCode = execute('tool web_search --query "rust async"');
    expect(exitCode).toBe(TOOL_REQUEST_EXIT_CODE);
  });

  it("invokes tool with JSON params", () => {
    const exitCode = execute(
      'tool code_edit --json \'{"file": "main.rs", "line": 42}\'',
    );
    expect(exitCode).toBe(TOOL_REQUEST_EXIT_CODE);
  });

  it("fails without tool name", () => {
    const exitCode = execute("tool");
    expect(exitCode).toBe(1);
  });

  it("fails with option as tool name", () => {
    const exitCode = execute("tool --query test");
    expect(exitCode).toBe(1);
  });
});
