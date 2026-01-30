/**
 * Stateful Shell API tests
 *
 * Tests for the new Shell class that maintains state across executions.
 * Variables, functions, and aliases defined in one execute() call
 * persist for subsequent calls on the same Shell instance.
 */
import { describe, expect, it } from "vitest";

import { Shell, execute } from "@bsull/conch";

describe("Shell class (stateful)", () => {
  describe("variable persistence", () => {
    it("persists variables across execute calls", () => {
      const shell = new Shell();

      shell.execute('greeting="Hello"');
      shell.execute('name="World"');
      const result = shell.execute('echo "$greeting, $name!"');

      expect(result.exitCode).toBe(0);
      expect(result.stdout).toBe("Hello, World!\n");
    });

    it("can increment a variable across calls", () => {
      const shell = new Shell();

      shell.execute("count=0");
      shell.execute("count=$((count + 1))");
      shell.execute("count=$((count + 1))");
      shell.execute("count=$((count + 1))");
      const result = shell.execute("test $count -eq 3");

      expect(result.exitCode).toBe(0);
    });
  });

  describe("function persistence", () => {
    it("persists functions across execute calls", () => {
      const shell = new Shell();

      shell.execute('greet() { echo "Hi, $1!"; }');
      const result = shell.execute("greet Alice");

      expect(result.exitCode).toBe(0);
      expect(result.stdout).toBe("Hi, Alice!\n");
    });

    it("can call a function multiple times", () => {
      const shell = new Shell();

      shell.execute("double() { echo $(($1 * 2)); }");

      const r1 = shell.execute("double 5");
      expect(r1.exitCode).toBe(0);
      expect(r1.stdout).toBe("10\n");

      const r2 = shell.execute("double 10");
      expect(r2.exitCode).toBe(0);
      expect(r2.stdout).toBe("20\n");

      const r3 = shell.execute("double 0");
      expect(r3.exitCode).toBe(0);
      expect(r3.stdout).toBe("0\n");
    });
  });

  describe("getVar and setVar", () => {
    it("can get a variable set via execute", () => {
      const shell = new Shell();

      shell.execute("myvar=hello");
      expect(shell.getVar("myvar")).toBe("hello");
    });

    it("returns undefined for unset variables", () => {
      const shell = new Shell();

      expect(shell.getVar("nonexistent")).toBeUndefined();
    });

    it("can set a variable and read it in execute", () => {
      const shell = new Shell();

      shell.setVar("injected", "from JS");
      const result = shell.execute('test "$injected" = "from JS"');

      expect(result.exitCode).toBe(0);
    });

    it("can overwrite a variable", () => {
      const shell = new Shell();

      shell.setVar("x", "first");
      expect(shell.getVar("x")).toBe("first");

      shell.setVar("x", "second");
      expect(shell.getVar("x")).toBe("second");
    });
  });

  describe("lastExitCode", () => {
    it("returns 0 after successful command", () => {
      const shell = new Shell();

      shell.execute("true");
      expect(shell.lastExitCode()).toBe(0);
    });

    it("returns 1 after false command", () => {
      const shell = new Shell();

      shell.execute("false");
      expect(shell.lastExitCode()).toBe(1);
    });

    it("returns custom exit code", () => {
      const shell = new Shell();

      shell.execute("exit 42");
      expect(shell.lastExitCode()).toBe(42);
    });

    it("updates after each execute", () => {
      const shell = new Shell();

      shell.execute("true");
      expect(shell.lastExitCode()).toBe(0);

      shell.execute("false");
      expect(shell.lastExitCode()).toBe(1);

      shell.execute("true");
      expect(shell.lastExitCode()).toBe(0);
    });
  });

  describe("stdout and stderr capture", () => {
    it("captures stdout", () => {
      const shell = new Shell();
      const result = shell.execute("echo hello world");

      expect(result.stdout).toBe("hello world\n");
      expect(result.stderr).toBe("");
    });

    it("captures stderr", () => {
      const shell = new Shell();
      const result = shell.execute("echo error >&2");

      expect(result.stdout).toBe("");
      expect(result.stderr).toBe("error\n");
    });

    it("captures both stdout and stderr", () => {
      const shell = new Shell();
      const result = shell.execute("echo out; echo err >&2");

      expect(result.stdout).toBe("out\n");
      expect(result.stderr).toBe("err\n");
    });
  });

  describe("instance isolation", () => {
    it("different Shell instances have separate state", () => {
      const shell1 = new Shell();
      const shell2 = new Shell();

      shell1.execute("x=shell1");
      shell2.execute("x=shell2");

      expect(shell1.getVar("x")).toBe("shell1");
      expect(shell2.getVar("x")).toBe("shell2");
    });

    it("functions in one shell are not visible in another", () => {
      const shell1 = new Shell();
      const shell2 = new Shell();

      shell1.execute("myfunc() { echo from_shell1; }");

      const r1 = shell1.execute("myfunc");
      expect(r1.exitCode).toBe(0);
      expect(r1.stdout).toBe("from_shell1\n");

      // shell2 doesn't have myfunc, so it should fail
      const r2 = shell2.execute("myfunc");
      expect(r2.exitCode).not.toBe(0);
    });
  });

  describe("dispose", () => {
    it("can dispose a shell instance", () => {
      const shell = new Shell();
      const result = shell.execute("echo hello");
      expect(result.stdout).toBe("hello\n");
      shell.dispose();
      // After dispose, using the shell should throw or be invalid
      // (implementation-dependent behavior)
    });
  });
});

describe("execute function (stateless)", () => {
  it("does not persist state between calls", () => {
    execute("stateless_var=hello");
    // This should fail because stateless_var is not set in a fresh instance
    const result = execute('test -n "$stateless_var"');
    expect(result.exitCode).toBe(1);
  });

  it("returns exit code 0 for successful commands", () => {
    expect(execute("true").exitCode).toBe(0);
    expect(execute("echo hello").exitCode).toBe(0);
  });

  it("returns non-zero exit code for failed commands", () => {
    expect(execute("false").exitCode).toBe(1);
  });

  it("captures stdout", () => {
    const result = execute("echo hello");
    expect(result.stdout).toBe("hello\n");
  });
});
