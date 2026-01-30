# Conch Shell - JavaScript Bindings

This directory contains the JavaScript/TypeScript bindings for the conch-shell WebAssembly component, enabling it to run in browsers and Node.js.

## Architecture

The build process uses **jco** (JavaScript Component Tools) to transpile the WASM Component Model component into JavaScript that can run in any JavaScript environment:

```
conch-shell (Rust)
    ↓ cargo build --target wasm32-wasip2
WASM Component (wit-bindgen + WASI Preview 2)
    ↓ jco transpile
JavaScript module + WASM core files
```

The `@bytecodealliance/preview2-shim` package provides WASI Preview 2 compatibility for browsers and Node.js.

## Prerequisites

- Rust with `wasm32-wasip2` target: `rustup target add wasm32-wasip2`
- [mise](https://mise.jdx.dev/) for task running
- Node.js 20+
- jco: `npm install -g @bytecodealliance/jco`

## Building

Tasks are defined in `js/mise.toml` using mise's monorepo feature. You can run them from the project root with the `//js:` prefix, or from within the `js/` directory using the `:` shorthand.

From project root:
```bash
# Install jco
MISE_EXPERIMENTAL=1 mise run //js:deps

# Build the WASM component and transpile to JS
MISE_EXPERIMENTAL=1 mise run //js:build

# Run tests
MISE_EXPERIMENTAL=1 mise run //js:test

# Clean build artifacts
MISE_EXPERIMENTAL=1 mise run //js:clean
```

Or from within `js/`:
```bash
cd js
MISE_EXPERIMENTAL=1 mise run :deps
MISE_EXPERIMENTAL=1 mise run :build
MISE_EXPERIMENTAL=1 mise run :test
MISE_EXPERIMENTAL=1 mise run :clean
```

## Directory Structure

```
js/
├── README.md             # This file
├── conch-shell/          # Output npm package
│   ├── package.json
│   ├── README.md
│   ├── conch-shell.js    # Generated JS bindings
│   ├── conch-shell.d.ts  # Generated TypeScript types
│   ├── *.wasm            # WASM core files
│   └── interfaces/       # Generated WASI interface types
└── testpkg/              # Integration tests
    ├── package.json
    ├── tsconfig.json
    └── shell.test.ts
```

## Usage

```javascript
import { execute } from '@bsull/conch';

// Execute a shell script (synchronous after module initialization)
const exitCode = execute('echo "Hello from WASM!"');
console.log('Exit code:', exitCode);

// The shell supports pipes, variables, loops, etc.
execute('for i in 1 2 3; do echo $i; done');
execute('echo hello | cat');
execute('x=world; echo "Hello $x"');
```

## Virtual Filesystem

The shell operates in a virtual in-memory filesystem. You can set up and update files at any time, even after `execute()` has been called:

```javascript
import { execute } from '@bsull/conch';
import { setFileData, updateFile, deletePath, fromPaths } from '@bsull/conch/vfs';

// Set up the VFS
setFileData(fromPaths({
  '/data/input.txt': 'hello world',
  '/config/settings.json': '{"debug": true}'
}));

// Execute commands
execute('cat /data/input.txt');  // prints: hello world

// Update the VFS (works even after execute() has been called)
updateFile('/data/input.txt', 'updated content');
execute('cat /data/input.txt');  // prints: updated content

// Add new files
updateFile('/data/new-file.txt', 'new content');

// Delete files
deletePath('/data/input.txt');
```

The VFS supports dynamic updates by mutating the underlying filesystem data in place, preserving the WASM shell's references.

## Browser & Node.js Support

This package works out of the box in both browsers and Node.js with the same behavior:

- Uses the in-memory virtual filesystem (VFS)
- `setFileData()`, `updateFile()`, and `deletePath()` work as expected
- No access to the real filesystem (sandboxed by design)
- No configuration or import aliasing required

**Requirements:** Node.js 19+ (for `globalThis.crypto`)

## Limitations

1. **No real filesystem access**: The shell operates in a sandboxed virtual filesystem.

2. **No network access**: Network operations are not available in the sandbox.

3. **stdout/stderr**: Output goes to the WASI shim's stdout/stderr, which by default prints to `console.log`/`console.error`.

4. **Some commands unavailable**: Commands like `ls`, `sed`, `awk`, `sort`, `printf` are not yet implemented and will fail with "operation not supported on this platform".

5. **Single-threaded**: The shell runs in a single-threaded environment.

6. **Memory limits**: Subject to WebAssembly memory constraints.

## Publishing

```bash
MISE_EXPERIMENTAL=1 mise run //js:publish
```

This publishes the `@bsull/conch` package to npm.
