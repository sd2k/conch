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
    const exitCode = execute("cat /hello.txt");
    expect(exitCode).toBe(0);
  });

  it("can read another file at root", () => {
    const exitCode = execute("cat /goodbye.txt");
    expect(exitCode).toBe(0);
  });

  it("can read nested file", () => {
    const exitCode = execute("cat /data/test.txt");
    expect(exitCode).toBe(0);
  });

  it("can read another nested file", () => {
    const exitCode = execute("cat /data/numbers.txt");
    expect(exitCode).toBe(0);
  });
});
