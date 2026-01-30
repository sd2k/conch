import { describe, expect, it } from "vitest";
import { execute } from "@bsull/conch";

describe("builtins", () => {
  describe("I/O", () => {
    it("echo", () => {
      const result = execute("echo hello");
      expect(result.exitCode).toBe(0);
      expect(result.stdout).toBe("hello\n");
    });

    it.todo("printf", () => {
      const result = execute("printf '%s\\n' hello");
      expect(result.exitCode).toBe(0);
    });

    it("cat", () => {
      const result = execute("echo hello | cat");
      expect(result.exitCode).toBe(0);
      expect(result.stdout).toBe("hello\n");
    });

    it("read", () => {
      const result = execute("echo hello | read x && echo $x");
      expect(result.exitCode).toBe(0);
    });
  });

  describe("file operations", () => {
    it("head", () => {
      const result = execute("echo -e 'line1\\nline2\\nline3' | head -n 1");
      expect(result.exitCode).toBe(0);
      expect(result.stdout).toBe("line1\n");
    });

    it("tail", () => {
      const result = execute("echo -e 'line1\\nline2\\nline3' | tail -n 1");
      expect(result.exitCode).toBe(0);
      expect(result.stdout).toBe("line3\n");
    });

    it("wc", () => {
      const result = execute("echo hello | wc -c");
      expect(result.exitCode).toBe(0);
      expect(result.stdout.trim()).toBe("6");
    });

    it.todo("tee", () => {
      const result = execute("echo hello | tee /dev/null");
      expect(result.exitCode).toBe(0);
    });
  });

  describe("text processing", () => {
    it("grep", () => {
      const result = execute("echo hello | grep hello");
      expect(result.exitCode).toBe(0);
      expect(result.stdout).toBe("hello\n");
    });

    it("grep (no match)", () => {
      const result = execute("echo hello | grep world");
      expect(result.exitCode).toBe(1);
    });

    it.todo("sed", () => {
      const result = execute("echo hello | sed 's/hello/world/'");
      expect(result.exitCode).toBe(0);
    });

    it.todo("awk", () => {
      const result = execute("echo 'a b c' | awk '{print $2}'");
      expect(result.exitCode).toBe(0);
    });

    it.todo("sort", () => {
      const result = execute("echo -e 'b\\na\\nc' | sort");
      expect(result.exitCode).toBe(0);
    });

    it.todo("uniq", () => {
      const result = execute("echo -e 'a\\na\\nb' | uniq");
      expect(result.exitCode).toBe(0);
    });

    it.todo("cut", () => {
      const result = execute("echo 'a:b:c' | cut -d: -f2");
      expect(result.exitCode).toBe(0);
    });

    it.todo("tr", () => {
      const result = execute("echo hello | tr 'a-z' 'A-Z'");
      expect(result.exitCode).toBe(0);
    });
  });

  describe("JSON", () => {
    it("jq (identity)", () => {
      const result = execute("echo '{\"a\":1}' | jq '.'");
      expect(result.exitCode).toBe(0);
      expect(result.stdout).toContain('"a"');
    });

    it("jq (field access)", () => {
      const result = execute("echo '{\"a\":1}' | jq '.a'");
      expect(result.exitCode).toBe(0);
      expect(result.stdout.trim()).toBe("1");
    });

    it("jq (array)", () => {
      const result = execute("echo '[1,2,3]' | jq '.[0]'");
      expect(result.exitCode).toBe(0);
      expect(result.stdout.trim()).toBe("1");
    });
  });

  describe("utilities", () => {
    it("test (true)", () => {
      const result = execute("test 1 -eq 1");
      expect(result.exitCode).toBe(0);
    });

    it("test (false)", () => {
      const result = execute("test 1 -eq 2");
      expect(result.exitCode).toBe(1);
    });

    it("true", () => {
      const result = execute("true");
      expect(result.exitCode).toBe(0);
    });

    it("false", () => {
      const result = execute("false");
      expect(result.exitCode).toBe(1);
    });

    it("pwd", () => {
      const result = execute("pwd");
      expect(result.exitCode).toBe(0);
      expect(result.stdout).toBe("/\n");
    });

    it("cd", () => {
      const result = execute("cd / && pwd");
      expect(result.exitCode).toBe(0);
      expect(result.stdout).toBe("/\n");
    });

    it("export", () => {
      const result = execute("export FOO=bar && echo $FOO");
      expect(result.exitCode).toBe(0);
      expect(result.stdout).toBe("bar\n");
    });

    it("set", () => {
      const result = execute("set -e && true");
      expect(result.exitCode).toBe(0);
    });
  });

  describe("flow control", () => {
    it("if/then/else", () => {
      const result = execute("if true; then echo yes; else echo no; fi");
      expect(result.exitCode).toBe(0);
      expect(result.stdout).toBe("yes\n");
    });

    it("if/then/elif/else", () => {
      const result = execute(
        "if false; then echo a; elif true; then echo b; else echo c; fi",
      );
      expect(result.exitCode).toBe(0);
      expect(result.stdout).toBe("b\n");
    });

    it("for loop", () => {
      const result = execute("for i in 1 2 3; do echo $i; done");
      expect(result.exitCode).toBe(0);
      expect(result.stdout).toBe("1\n2\n3\n");
    });

    it("while loop", () => {
      const result = execute("x=0; while test $x -lt 3; do x=$((x+1)); done");
      expect(result.exitCode).toBe(0);
    });

    it("case statement", () => {
      const result = execute("case foo in foo) echo matched;; esac");
      expect(result.exitCode).toBe(0);
      expect(result.stdout).toBe("matched\n");
    });

    it("functions", () => {
      const result = execute("greet() { echo hello $1; }; greet world");
      expect(result.exitCode).toBe(0);
      expect(result.stdout).toBe("hello world\n");
    });
  });
});
