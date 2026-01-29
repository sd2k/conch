# @bsull/conch

A sandboxed bash-compatible shell running in WebAssembly, for browser and Node.js environments.

## Installation

```bash
npm install @bsull/conch
```

## Usage

```javascript
import { execute } from '@bsull/conch';

// Execute a shell script
const exitCode = execute('echo "Hello, World!"');
console.log('Exit code:', exitCode); // 0

// Script errors return non-zero exit codes (not exceptions)
const result = execute('false');
console.log(result); // 1

// Shell initialization errors throw ComponentError
try {
  execute('some command');
} catch (e) {
  console.error('Shell failed to initialize:', e.message);
}
```

## API

### `execute(script: string): number`

Execute a shell script in the sandbox.

- **script**: The bash-compatible shell script to execute
- **Returns**: The exit code (0 for success, non-zero for failure)
- **Throws**: `ComponentError` if the shell itself failed to initialize (not for script errors)

Note: The module uses top-level await for initialization. The `execute` function is synchronous once the module is loaded.

### Available Commands

The shell includes these built-in commands:

- **I/O**: `echo`, `printf`, `cat`, `read`
- **File operations**: `head`, `tail`, `wc`, `tee`
- **Text processing**: `grep`, `sed`, `awk`, `sort`, `uniq`, `cut`, `tr`
- **JSON**: `jq` (via jaq)
- **Utilities**: `test`, `true`, `false`, `pwd`, `cd`, `export`, `set`
- **Flow control**: `if`, `for`, `while`, `case`, functions

## Virtual Filesystem

By default, the shell operates in an empty virtual filesystem. You can set up files before execution:

```javascript
import { execute } from '@bsull/conch';
import { setFileData, fromPaths } from '@bsull/conch/vfs';

// Set up the VFS BEFORE any execute() calls
setFileData(fromPaths({
  '/data/input.txt': 'hello world',
  '/config/settings.json': '{"debug": true}'
}));

// Now the shell can access these files
execute('cat /data/input.txt');  // prints: hello world
```

**Important**: The shell caches its filesystem on first execution. Call `setFileData()` before any `execute()` calls.

### Node.js Setup

In Node.js, the VFS requires aliasing imports to use browser shims. With Vite/Vitest:

```javascript
// vitest.config.ts
import { defineConfig } from "vitest/config";
import { resolve } from "path";

const browserShimPath = resolve(
  __dirname,
  "node_modules/@bytecodealliance/preview2-shim/lib/browser"
);

export default defineConfig({
  resolve: {
    alias: {
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
```

Requires Node.js 19+ (for `globalThis.crypto`).

## Limitations

- **No network access**: Network operations are not available.
- **Single-threaded**: The shell runs in a single-threaded environment.
- **Memory limits**: Subject to WebAssembly memory constraints.

## Browser Support

This package uses the WebAssembly Component Model and requires a modern browser with WebAssembly support. The `@bytecodealliance/preview2-shim` package provides WASI Preview 2 compatibility.

## License

Apache-2.0
