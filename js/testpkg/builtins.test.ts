import { describe, expect, it } from "vitest";
import { execute } from "@bsull/conch";

describe("builtins", () => {
  describe("I/O", () => {
    it("echo", () => {
      const exitCode = execute("echo hello");
      expect(exitCode).toBe(0);
    });

    it.todo("printf", () => {
      const exitCode = execute("printf '%s\\n' hello");
      expect(exitCode).toBe(0);
    });

    it("cat", () => {
      const exitCode = execute("echo hello | cat");
      expect(exitCode).toBe(0);
    });

    it("read", () => {
      const exitCode = execute("echo hello | read x && echo $x");
      expect(exitCode).toBe(0);
    });
  });

  describe("file operations", () => {
    it("head", () => {
      const exitCode = execute("echo -e 'line1\\nline2\\nline3' | head -n 1");
      expect(exitCode).toBe(0);
    });

    it("tail", () => {
      const exitCode = execute("echo -e 'line1\\nline2\\nline3' | tail -n 1");
      expect(exitCode).toBe(0);
    });

    it("wc", () => {
      const exitCode = execute("echo hello | wc -c");
      expect(exitCode).toBe(0);
    });

    it.todo("tee", () => {
      const exitCode = execute("echo hello | tee /dev/null");
      expect(exitCode).toBe(0);
    });
  });

  describe("text processing", () => {
    it("grep", () => {
      const exitCode = execute("echo hello | grep hello");
      expect(exitCode).toBe(0);
    });

    it("grep (no match)", () => {
      const exitCode = execute("echo hello | grep world");
      expect(exitCode).toBe(1);
    });

    it.todo("sed", () => {
      const exitCode = execute("echo hello | sed 's/hello/world/'");
      expect(exitCode).toBe(0);
    });

    it.todo("awk", () => {
      const exitCode = execute("echo 'a b c' | awk '{print $2}'");
      expect(exitCode).toBe(0);
    });

    it.todo("sort", () => {
      const exitCode = execute("echo -e 'b\\na\\nc' | sort");
      expect(exitCode).toBe(0);
    });

    it.todo("uniq", () => {
      const exitCode = execute("echo -e 'a\\na\\nb' | uniq");
      expect(exitCode).toBe(0);
    });

    it.todo("cut", () => {
      const exitCode = execute("echo 'a:b:c' | cut -d: -f2");
      expect(exitCode).toBe(0);
    });

    it.todo("tr", () => {
      const exitCode = execute("echo hello | tr 'a-z' 'A-Z'");
      expect(exitCode).toBe(0);
    });
  });

  describe("JSON", () => {
    it("jq (identity)", () => {
      const exitCode = execute("echo '{\"a\":1}' | jq '.'");
      expect(exitCode).toBe(0);
    });

    it("jq (field access)", () => {
      const exitCode = execute("echo '{\"a\":1}' | jq '.a'");
      expect(exitCode).toBe(0);
    });

    it("jq (array)", () => {
      const exitCode = execute("echo '[1,2,3]' | jq '.[0]'");
      expect(exitCode).toBe(0);
    });
  });

  describe("utilities", () => {
    it("test (true)", () => {
      const exitCode = execute("test 1 -eq 1");
      expect(exitCode).toBe(0);
    });

    it("test (false)", () => {
      const exitCode = execute("test 1 -eq 2");
      expect(exitCode).toBe(1);
    });

    it("true", () => {
      const exitCode = execute("true");
      expect(exitCode).toBe(0);
    });

    it("false", () => {
      const exitCode = execute("false");
      expect(exitCode).toBe(1);
    });

    it("pwd", () => {
      const exitCode = execute("pwd");
      expect(exitCode).toBe(0);
    });

    it("cd", () => {
      const exitCode = execute("cd / && pwd");
      expect(exitCode).toBe(0);
    });

    it("export", () => {
      const exitCode = execute("export FOO=bar && echo $FOO");
      expect(exitCode).toBe(0);
    });

    it("set", () => {
      const exitCode = execute("set -e && true");
      expect(exitCode).toBe(0);
    });
  });

  describe("flow control", () => {
    it("if/then/else", () => {
      const exitCode = execute("if true; then echo yes; else echo no; fi");
      expect(exitCode).toBe(0);
    });

    it("if/then/elif/else", () => {
      const exitCode = execute(
        "if false; then echo a; elif true; then echo b; else echo c; fi",
      );
      expect(exitCode).toBe(0);
    });

    it("for loop", () => {
      const exitCode = execute("for i in 1 2 3; do echo $i; done");
      expect(exitCode).toBe(0);
    });

    it("while loop", () => {
      const exitCode = execute("x=0; while test $x -lt 3; do x=$((x+1)); done");
      expect(exitCode).toBe(0);
    });

    it("case statement", () => {
      const exitCode = execute("case foo in foo) echo matched;; esac");
      expect(exitCode).toBe(0);
    });

    it("functions", () => {
      const exitCode = execute("greet() { echo hello $1; }; greet world");
      expect(exitCode).toBe(0);
    });
  });
});
