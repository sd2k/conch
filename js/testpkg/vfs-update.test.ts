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

    const result = execute("cat /initial.txt");
    expect(result.exitCode).toBe(0);
    expect(result.stdout).toBe("initial content");
  });

  it("can read files added after first execute", () => {
    // Update VFS with a new file (initial.txt should still be there due to syncInPlace)
    setFileData({
      dir: {
        "initial.txt": { source: "initial content" },
        "added-after.txt": { source: "added after first execute" },
      },
    });

    const result = execute("cat /added-after.txt");
    expect(result.exitCode).toBe(0);
    expect(result.stdout).toBe("added after first execute");
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
    const result = execute("grep UPDATED /initial.txt");
    expect(result.exitCode).toBe(0);
    expect(result.stdout).toBe("UPDATED content\n");
  });

  it("can use updateFile helper", () => {
    updateFile("/helper-test.txt", "created via updateFile");
    const result = execute("cat /helper-test.txt");
    expect(result.exitCode).toBe(0);
    expect(result.stdout).toBe("created via updateFile");
  });

  it("can delete files via deletePath", () => {
    updateFile("/to-delete.txt", "this will be deleted");

    // Verify it exists
    let result = execute("cat /to-delete.txt");
    expect(result.exitCode).toBe(0);
    expect(result.stdout).toBe("this will be deleted");

    // Delete it
    const deleted = deletePath("/to-delete.txt");
    expect(deleted).toBe(true);

    // Verify it's gone
    result = execute("cat /to-delete.txt");
    expect(result.exitCode).toBe(1); // File not found
  });
});
