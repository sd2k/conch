import { describe, expect, it } from "vitest";

// The jco-generated module has async initialization via top-level await.
// The `execute` function is synchronous once the module is loaded.
// It returns an ExecutionResult with exitCode, stdout, and stderr.
import { execute } from "@bsull/conch";

describe("conch-shell", () => {
  describe("basic execution", () => {
    it("executes echo command", () => {
      const result = execute("echo hello");
      expect(result.exitCode).toBe(0);
      expect(result.stdout).toBe("hello\n");
      expect(result.stderr).toBe("");
    });

    it("executes true command", () => {
      const result = execute("true");
      expect(result.exitCode).toBe(0);
    });

    it("executes false command", () => {
      const result = execute("false");
      expect(result.exitCode).toBe(1);
    });
  });

  describe("variables", () => {
    it("handles variable assignment and expansion", () => {
      const result = execute("x=hello; echo $x");
      expect(result.exitCode).toBe(0);
      expect(result.stdout).toBe("hello\n");
    });
  });

  describe("arithmetic", () => {
    it("evaluates arithmetic expressions", () => {
      const result = execute("echo $((2 + 2))");
      expect(result.exitCode).toBe(0);
      expect(result.stdout).toBe("4\n");
    });
  });

  describe("control flow", () => {
    it("handles if statements", () => {
      const result = execute("if true; then echo yes; fi");
      expect(result.exitCode).toBe(0);
      expect(result.stdout).toBe("yes\n");
    });

    it("handles for loops", () => {
      const result = execute("for i in 1 2 3; do echo $i; done");
      expect(result.exitCode).toBe(0);
      expect(result.stdout).toBe("1\n2\n3\n");
    });
  });

  describe("pipes", () => {
    it("handles simple pipes", () => {
      const result = execute("echo hello | cat");
      expect(result.exitCode).toBe(0);
      expect(result.stdout).toBe("hello\n");
    });
  });

  describe("builtins", () => {
    it("runs test command", () => {
      const result = execute("test 1 -eq 1");
      expect(result.exitCode).toBe(0);
    });

    it("runs test command (failure)", () => {
      const result = execute("test 1 -eq 2");
      expect(result.exitCode).toBe(1);
    });
  });
});
