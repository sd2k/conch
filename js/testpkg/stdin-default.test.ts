import { describe, expect, it } from "vitest";
import { Shell } from "@bsull/conch";

// Regression test for the stdin-not-initialized bug.
//
// `new Shell()` used to leave the browser preview2-shim's unimplemented stdin
// stub in place, which returns `undefined`. Any command that reads stdin
// without piped input (cat/grep/wc/jq/read) then threw
// "Cannot read properties of undefined (reading 'byteLength')" mid-execute.
// That host-side throw unwinds the wasm stack without dropping the Instance's
// RefCell borrow, poisoning the shell so every *subsequent* command panicked
// with "RefCell already borrowed". `@bsull/conch` now installs a default
// empty/EOF stdin handler at load, so reads return EOF cleanly.
describe("default stdin", () => {
  it("reads EOF (not undefined) when no stdin is provided", () => {
    const shell = new Shell();
    // Must not throw, and must not brick the instance.
    const r = shell.execute("cat");
    expect(typeof r.exitCode).toBe("number");
    expect(r.stderr).not.toContain("byteLength");
    shell.dispose();
  });

  it("does not poison the shell after a stdin read (no RefCell panic)", () => {
    const shell = new Shell();
    shell.execute("cat"); // reads stdin -> EOF
    // Previously panicked with "RefCell already borrowed" here.
    const r = shell.execute("echo alive");
    expect(r.exitCode).toBe(0);
    expect(r.stdout).toBe("alive\n");
    shell.dispose();
  });
});
