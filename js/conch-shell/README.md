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

- **I/O**: `echo`, `cat`, `read`
- **File operations**: `head`, `tail`, `wc`
- **Text processing**: `grep`
- **JSON**: `jq` (via jaq)
- **Utilities**: `test`, `true`, `false`, `pwd`, `cd`, `export`, `set`
- **Flow control**: `if`, `for`, `while`, `case`, functions

## Virtual Filesystem

By default, the shell operates in an empty virtual filesystem. You can set up and update files at any time:

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

The VFS supports updates even after `execute()` has been called. This is achieved by mutating the underlying filesystem data in place, preserving the WASM shell's references.

## Browser & Node.js Support

This package works out of the box in both browsers and Node.js with the same behavior:

- Uses the in-memory virtual filesystem (VFS)
- `setFileData()`, `updateFile()`, and `deletePath()` work as expected
- No access to the real filesystem (sandboxed by design)
- No configuration or import aliasing required

**Requirements:** Node.js 19+ (for `globalThis.crypto`)

## Limitations

- **No real filesystem access**: The shell operates in a sandboxed virtual filesystem.
- **No network access**: Network operations are not available in the sandbox.
- **Single-threaded**: The shell runs in a single-threaded environment.
- **Memory limits**: Subject to WebAssembly memory constraints.
- **Some commands unavailable**: Commands like `ls`, `sed`, `awk`, `sort`, `printf` are not yet implemented.

## Platform Support

This package uses the WebAssembly Component Model and requires:

**Node.js:**
- Node.js 19+ (for `globalThis.crypto`)

**Browsers:**
- Chrome 119+
- Firefox 120+
- Safari 17+
- Edge 119+

## License

Apache-2.0