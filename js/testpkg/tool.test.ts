/**
 * Tool builtin tests
 *
 * These tests are currently excluded from the test run due to Rust-side issues:
 * 1. "Cannot start a runtime from within a runtime" - stdin reading uses block_on
 * 2. Stdin returns undefined when nothing is piped, causing byteLength error
 *
 * To run these tests manually: npx vitest run tool.test.ts --config vitest.tool.config.ts
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
