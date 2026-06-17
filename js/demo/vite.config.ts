import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";
import { resolve } from "node:path";

// @bsull/conch is installed as a `file:` symlink, so its dependency
// @bytecodealliance/preview2-shim lives under the package's own node_modules.
// The conch shims import the bare subpaths (e.g. ".../preview2-shim/filesystem");
// force them to the *browser* build, which is the one wired up for the VFS.
// (Mirrors js/testpkg/vitest.config.ts.)
const shimBase = resolve(
  __dirname,
  "../conch-shell/node_modules/@bytecodealliance/preview2-shim/lib/browser",
);

export default defineConfig({
  plugins: [svelte()],
  resolve: {
    alias: {
      "@bytecodealliance/preview2-shim/cli": resolve(shimBase, "cli.js"),
      "@bytecodealliance/preview2-shim/clocks": resolve(shimBase, "clocks.js"),
      "@bytecodealliance/preview2-shim/filesystem": resolve(shimBase, "filesystem.js"),
      "@bytecodealliance/preview2-shim/io": resolve(shimBase, "io.js"),
      "@bytecodealliance/preview2-shim/random": resolve(shimBase, "random.js"),
    },
  },
  // @bsull/conch loads its WASM core via top-level await, which esbuild's
  // dep pre-bundling can't handle. Serve it as native ESM instead.
  optimizeDeps: {
    exclude: ["@bsull/conch"],
  },
  assetsInclude: ["**/*.wasm"],
  build: {
    target: "esnext",
  },
  server: {
    fs: {
      // Allow serving files from the symlinked package outside this dir.
      allow: [".", resolve(__dirname, "../conch-shell")],
    },
  },
});
