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

describe("complex scripts (user-reported)", () => {
  it("handles variable expansion with loops", () => {
    const shell = new Shell();
    const result = shell.execute(`
echo "=== Shell Demo ==="
NAME="Grafana"
VERSION="12.4.0"
echo "   Product: $NAME $VERSION"
for num in 10 20 30; do
  echo "   Processing: $num"
done
STATUS="healthy"
if [ "$STATUS" = "healthy" ]; then
  echo "   System is $STATUS"
fi
REQUESTS=156
ERRORS=3
RATE=$((ERRORS * 100 / REQUESTS))
echo "   Error rate: $RATE%"
    `);

    expect(result.exitCode).toBe(0);
    expect(result.stdout).toContain("Shell Demo");
    expect(result.stdout).toContain("Grafana 12.4.0");
    expect(result.stdout).toContain("Processing: 10");
    expect(result.stdout).toContain("Processing: 20");
    expect(result.stdout).toContain("Processing: 30");
    expect(result.stdout).toContain("System is healthy");
    expect(result.stdout).toContain("Error rate: 1%");
  });

  it("handles file redirects", () => {
    const shell = new Shell();

    // Test basic write and read
    const write = shell.execute('echo "timestamp,value" > /data.txt');
    expect(write.exitCode).toBe(0);

    const append = shell.execute('echo "2026-02-03T10:00:00Z,42" >> /data.txt');
    expect(append.exitCode).toBe(0);

    const read = shell.execute("cat /data.txt");
    expect(read.exitCode).toBe(0);
    expect(read.stdout).toContain("timestamp,value");
    expect(read.stdout).toContain("2026-02-03T10:00:00Z,42");
  });

  it("handles mkdir -p", () => {
    const shell = new Shell();

    const mkdir = shell.execute("mkdir -p /scratch /artifacts /tmp");
    expect(mkdir.exitCode).toBe(0);

    // Verify directory was created by writing to it
    const write = shell.execute('echo "test" > /scratch/demo.txt');
    expect(write.exitCode).toBe(0);

    const read = shell.execute("cat /scratch/demo.txt");
    expect(read.exitCode).toBe(0);
    expect(read.stdout).toBe("test\n");
  });

  it("handles heredoc", () => {
    const shell = new Shell();

    const heredoc = shell.execute(`cat > /test.txt << 'EOF'
test content
line 2
EOF`);
    expect(heredoc.exitCode).toBe(0);

    const read = shell.execute("cat /test.txt");
    expect(read.exitCode).toBe(0);
    expect(read.stdout).toContain("test content");
    expect(read.stdout).toContain("line 2");
  });
});

describe("filesystem builtins", () => {
  describe("touch", () => {
    it("creates a new file", () => {
      const shell = new Shell();
      const touch = shell.execute("touch /newfile.txt");
      expect(touch.exitCode).toBe(0);

      // Verify file exists by catting it (should be empty)
      const cat = shell.execute("cat /newfile.txt");
      expect(cat.exitCode).toBe(0);
      expect(cat.stdout).toBe("");
    });

    it("does not error on existing file", () => {
      const shell = new Shell();
      shell.execute('echo "content" > /existing.txt');
      const touch = shell.execute("touch /existing.txt");
      expect(touch.exitCode).toBe(0);

      // Content should still be there
      const cat = shell.execute("cat /existing.txt");
      expect(cat.stdout).toContain("content");
    });

    it("-c does not create non-existent file", () => {
      const shell = new Shell();
      const touch = shell.execute("touch -c /nonexistent.txt");
      expect(touch.exitCode).toBe(0);

      // File should not exist
      const cat = shell.execute("cat /nonexistent.txt");
      expect(cat.exitCode).not.toBe(0);
    });
  });

  describe("rm", () => {
    it("removes a file", () => {
      const shell = new Shell();
      shell.execute('echo "content" > /rmfile.txt');

      // Verify file exists
      const catBefore = shell.execute("cat /rmfile.txt");
      expect(catBefore.exitCode).toBe(0);

      // Remove file
      const rm = shell.execute("rm /rmfile.txt");
      if (rm.exitCode !== 0) {
        console.log("rm failed:", rm.exitCode, rm.stderr, rm.stdout);
      }
      expect(rm.exitCode).toBe(0);

      // Verify file is gone
      const catAfter = shell.execute("cat /rmfile.txt");
      expect(catAfter.exitCode).not.toBe(0);
    });

    it("errors on non-existent file without -f", () => {
      const shell = new Shell();
      const rm = shell.execute("rm /nonexistent.txt");
      expect(rm.exitCode).toBe(1);
    });

    it("-f silences non-existent file error", () => {
      const shell = new Shell();
      const rm = shell.execute("rm -f /nonexistent.txt");
      expect(rm.exitCode).toBe(0);
    });

    it("-r removes directory recursively", () => {
      const shell = new Shell();
      shell.execute("mkdir -p /rmdir/sub");
      shell.execute('echo "content" > /rmdir/sub/file.txt');

      // Verify setup
      const cat = shell.execute("cat /rmdir/sub/file.txt");
      expect(cat.exitCode).toBe(0);

      const rm = shell.execute("rm -r /rmdir");
      expect(rm.exitCode).toBe(0);
      expect(rm.stderr).toBe("");

      // Verify directory is gone
      const ls = shell.execute("ls /rmdir");
      expect(ls.exitCode).not.toBe(0);
    });

    it("errors on directory without -r", () => {
      const shell = new Shell();
      shell.execute("mkdir -p /dirtest");

      const rm = shell.execute("rm /dirtest");
      expect(rm.exitCode).toBe(1);
      expect(rm.stderr).toContain("directory");
    });
  });

  describe("ls", () => {
    it("lists directory contents", () => {
      const shell = new Shell();
      shell.execute("mkdir -p /lsdir");
      shell.execute("touch /lsdir/file1.txt");
      shell.execute("touch /lsdir/file2.txt");

      const ls = shell.execute("ls /lsdir");
      expect(ls.exitCode).toBe(0);
      expect(ls.stdout).toContain("file1.txt");
      expect(ls.stdout).toContain("file2.txt");
    });

    it("-1 lists one per line", () => {
      const shell = new Shell();
      shell.execute("mkdir -p /ls1dir");
      shell.execute("touch /ls1dir/a.txt");
      shell.execute("touch /ls1dir/b.txt");

      const ls = shell.execute("ls -1 /ls1dir");
      expect(ls.exitCode).toBe(0);
      expect(ls.stdout).toBe("a.txt\nb.txt\n");
    });

    it("-a shows hidden files", () => {
      const shell = new Shell();
      shell.execute("mkdir -p /lsadir");
      shell.execute("touch /lsadir/.hidden");
      shell.execute("touch /lsadir/visible");

      const ls = shell.execute("ls /lsadir");
      expect(ls.stdout).not.toContain(".hidden");

      const lsa = shell.execute("ls -a /lsadir");
      expect(lsa.stdout).toContain(".hidden");
      expect(lsa.stdout).toContain("visible");
    });

    it("-l shows long format", () => {
      const shell = new Shell();
      shell.execute("mkdir -p /lsldir");
      shell.execute('echo "hello" > /lsldir/test.txt');

      const ls = shell.execute("ls -l /lsldir");
      expect(ls.exitCode).toBe(0);
      expect(ls.stdout).toContain("test.txt");
      // Long format includes size, permissions, etc.
      expect(ls.stdout).toMatch(/rw/);
    });
  });

  describe("cp", () => {
    it("copies a file", () => {
      const shell = new Shell();
      shell.execute('echo "original" > /source.txt');

      const cp = shell.execute("cp /source.txt /dest.txt");
      expect(cp.exitCode).toBe(0);

      const cat = shell.execute("cat /dest.txt");
      expect(cat.stdout).toContain("original");

      // Original should still exist
      const catOrig = shell.execute("cat /source.txt");
      expect(catOrig.stdout).toContain("original");
    });

    it("-r copies directory recursively", () => {
      const shell = new Shell();
      shell.execute("mkdir -p /cpdir/sub");
      shell.execute('echo "content" > /cpdir/sub/file.txt');

      const cp = shell.execute("cp -r /cpdir /cpdir-copy");
      expect(cp.exitCode).toBe(0);

      const cat = shell.execute("cat /cpdir-copy/sub/file.txt");
      expect(cat.exitCode).toBe(0);
      expect(cat.stdout).toContain("content");
    });

    it("errors on directory without -r", () => {
      const shell = new Shell();
      shell.execute("mkdir -p /cpdir2");

      const cp = shell.execute("cp /cpdir2 /cpdir2-copy");
      expect(cp.exitCode).toBe(1);
      expect(cp.stderr).toContain("-r");
    });
  });

  describe("mv", () => {
    it("moves a file", () => {
      const shell = new Shell();
      shell.execute('echo "content" > /mvfile.txt');

      // Verify file was created
      const catBefore = shell.execute("cat /mvfile.txt");
      expect(catBefore.exitCode).toBe(0);

      const mv = shell.execute("mv /mvfile.txt /mvfile-moved.txt");
      expect(mv.exitCode).toBe(0);

      // Source should be gone
      const catSource = shell.execute("cat /mvfile.txt");
      expect(catSource.exitCode).not.toBe(0);

      // Dest should exist with content
      const catDest = shell.execute("cat /mvfile-moved.txt");
      expect(catDest.exitCode).toBe(0);
      expect(catDest.stdout).toContain("content");
    });

    it("moves directory", () => {
      const shell = new Shell();
      shell.execute("mkdir -p /mvdir/sub");
      shell.execute('echo "content" > /mvdir/sub/file.txt');

      const mv = shell.execute("mv /mvdir /mvdir-moved");
      expect(mv.exitCode).toBe(0);

      // Source should be gone
      const lsSource = shell.execute("ls /mvdir");
      expect(lsSource.exitCode).not.toBe(0);

      // Dest should exist with content
      const catDest = shell.execute("cat /mvdir-moved/sub/file.txt");
      expect(catDest.exitCode).toBe(0);
      expect(catDest.stdout).toContain("content");
    });

    it("renames a file", () => {
      const shell = new Shell();
      shell.execute('echo "hello" > /rename-src.txt');

      const mv = shell.execute("mv /rename-src.txt /rename-dst.txt");
      expect(mv.exitCode).toBe(0);

      const cat = shell.execute("cat /rename-dst.txt");
      expect(cat.exitCode).toBe(0);
      expect(cat.stdout).toContain("hello");
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
