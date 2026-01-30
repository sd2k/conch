import { describe, expect, it } from "vitest";

import { execute } from "@bsull/conch";
import { setFileData, getFileData } from "@bsull/conch/vfs";

describe("virtual filesystem", () => {
  // Set up VFS once before any tests run - the shell caches preopens on first use
  setFileData({
    dir: {
      "hello.txt": { source: "Hello, World!" },
      "goodbye.txt": { source: "Goodbye!" },
      data: {
        dir: {
          "test.txt": { source: "nested content" },
          "numbers.txt": { source: "1\n2\n3\n" },
        },
      },
    },
  });

  it("VFS is set up correctly", () => {
    console.log("VFS state:", getFileData());
  });

  it("can read a file at root", () => {
    const result = execute("cat /hello.txt");
    expect(result.exitCode).toBe(0);
    expect(result.stdout).toBe("Hello, World!");
  });

  it("can read another file at root", () => {
    const result = execute("cat /goodbye.txt");
    expect(result.exitCode).toBe(0);
    expect(result.stdout).toBe("Goodbye!");
  });

  it("can read nested file", () => {
    const result = execute("cat /data/test.txt");
    expect(result.exitCode).toBe(0);
    expect(result.stdout).toBe("nested content");
  });

  it("can read another nested file", () => {
    const result = execute("cat /data/numbers.txt");
    expect(result.exitCode).toBe(0);
    expect(result.stdout).toBe("1\n2\n3\n");
  });
});
