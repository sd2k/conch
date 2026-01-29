import { defineConfig } from "vitest/config";
import { resolve } from "path";

// Path to the preview2-shim browser lib directory
const browserShimPath = resolve(
  __dirname,
  "node_modules/@bsull/conch/node_modules/@bytecodealliance/preview2-shim/lib/browser",
);

export default defineConfig({
  test: {
    environment: "node",
    // Run tests serially since VFS is global state
    pool: "forks",
    poolOptions: {
      forks: {
        singleFork: true,
      },
    },
    sequence: {
      shuffle: false,
    },
    // Exclude tool builtin tests until Rust-side stdin issues are fixed
    exclude: ["**/tool.test.ts", "**/node_modules/**"],
  },
  resolve: {
    alias: {
      // Alias all preview2-shim imports to use browser shims instead of Node.js shims.
      // This allows the VFS (in-memory filesystem) to work in Node.js tests.
      // Node 19+ has globalThis.crypto which the browser shims require.
      "@bytecodealliance/preview2-shim/cli": `${browserShimPath}/cli.js`,
      "@bytecodealliance/preview2-shim/clocks": `${browserShimPath}/clocks.js`,
      "@bytecodealliance/preview2-shim/filesystem": `${browserShimPath}/filesystem.js`,
      "@bytecodealliance/preview2-shim/io": `${browserShimPath}/io.js`,
      "@bytecodealliance/preview2-shim/random": `${browserShimPath}/random.js`,
      "@bytecodealliance/preview2-shim/sockets": `${browserShimPath}/sockets.js`,
      "@bytecodealliance/preview2-shim/http": `${browserShimPath}/http.js`,
      "@bytecodealliance/preview2-shim": `${browserShimPath}/index.js`,
    },
  },
});
