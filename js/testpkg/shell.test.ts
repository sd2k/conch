import { describe, expect, it } from "vitest";

// The jco-generated module has async initialization via top-level await.
// The `execute` function is synchronous once the module is loaded.
// On success, it returns the exit code (number).
// On error (shell initialization failure), it throws a ComponentError.
import { execute } from "@bsull/conch";

describe("conch-shell", () => {
  describe("basic execution", () => {
    it("executes echo command", () => {
      const exitCode = execute("echo hello");
      expect(exitCode).toBe(0);
    });

    it("executes true command", () => {
      const exitCode = execute("true");
      expect(exitCode).toBe(0);
    });

    it("executes false command", () => {
      const exitCode = execute("false");
      expect(exitCode).toBe(1);
    });
  });

  describe("variables", () => {
    it("handles variable assignment and expansion", () => {
      const exitCode = execute("x=hello; echo $x");
      expect(exitCode).toBe(0);
    });
  });

  describe("arithmetic", () => {
    it("evaluates arithmetic expressions", () => {
      const exitCode = execute("echo $((2 + 2))");
      expect(exitCode).toBe(0);
    });
  });

  describe("control flow", () => {
    it("handles if statements", () => {
      const exitCode = execute("if true; then echo yes; fi");
      expect(exitCode).toBe(0);
    });

    it("handles for loops", () => {
      const exitCode = execute("for i in 1 2 3; do echo $i; done");
      expect(exitCode).toBe(0);
    });
  });

  describe("pipes", () => {
    it("handles simple pipes", () => {
      const exitCode = execute("echo hello | cat");
      expect(exitCode).toBe(0);
    });
  });

  describe("builtins", () => {
    it("runs test command", () => {
      const exitCode = execute("test 1 -eq 1");
      expect(exitCode).toBe(0);
    });

    it("runs test command (failure)", () => {
      const exitCode = execute("test 1 -eq 2");
      expect(exitCode).toBe(1);
    });
  });
});
