import { defineConfig } from "vitest/config";
import { resolve } from "path";

// Resolve paths to the browser shims in conch-shell's node_modules
const shimBase = resolve(__dirname, "../conch-shell/node_modules/@bytecodealliance/preview2-shim/lib/browser");

export default defineConfig({
  resolve: {
    alias: {
      // Force browser shims for @bytecodealliance/preview2-shim
      // This ensures the VFS-compatible browser shims are used even in Node.js
      "@bytecodealliance/preview2-shim/cli": resolve(shimBase, "cli.js"),
      "@bytecodealliance/preview2-shim/clocks": resolve(shimBase, "clocks.js"),
      "@bytecodealliance/preview2-shim/filesystem": resolve(shimBase, "filesystem.js"),
      "@bytecodealliance/preview2-shim/io": resolve(shimBase, "io.js"),
      "@bytecodealliance/preview2-shim/random": resolve(shimBase, "random.js"),
    },
  },
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
    exclude: ["**/node_modules/**"],
  },
});
