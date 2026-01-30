import { describe, expect, it } from "vitest";
import { setFileData, updateFile, deletePath } from "@bsull/conch/vfs";
import { execute } from "@bsull/conch";

describe("VFS updates after execute", () => {
  it("can read files set up before first execute", () => {
    setFileData({
      dir: {
        "initial.txt": { source: "initial content" },
      },
    });

    const exitCode = execute("cat /initial.txt");
    expect(exitCode).toBe(0);
  });

  it("can read files added after first execute", () => {
    // Update VFS with a new file (initial.txt should still be there due to syncInPlace)
    setFileData({
      dir: {
        "initial.txt": { source: "initial content" },
        "added-after.txt": { source: "added after first execute" },
      },
    });

    const exitCode = execute("cat /added-after.txt");
    expect(exitCode).toBe(0);
  });

  it("can read updated file content after execute", () => {
    // Update the content of initial.txt
    setFileData({
      dir: {
        "initial.txt": { source: "UPDATED content" },
        "added-after.txt": { source: "added after first execute" },
      },
    });

    // Use grep to verify the new content
    const exitCode = execute("grep UPDATED /initial.txt");
    expect(exitCode).toBe(0);
  });

  it("can use updateFile helper", () => {
    updateFile("/helper-test.txt", "created via updateFile");
    const exitCode = execute("cat /helper-test.txt");
    expect(exitCode).toBe(0);
  });

  it("can delete files via deletePath", () => {
    updateFile("/to-delete.txt", "this will be deleted");

    // Verify it exists
    let exitCode = execute("cat /to-delete.txt");
    expect(exitCode).toBe(0);

    // Delete it
    const deleted = deletePath("/to-delete.txt");
    expect(deleted).toBe(true);

    // Verify it's gone
    exitCode = execute("cat /to-delete.txt");
    expect(exitCode).toBe(1); // File not found
  });
});
