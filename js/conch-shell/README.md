# @bsull/conch

A sandboxed bash-compatible shell running in WebAssembly, for browser and Node.js environments.

## Installation

```bash
npm install @bsull/conch
```

## Usage

### Stateful Shell (Recommended)

The `Shell` class maintains state across executions - variables, functions, and aliases persist between calls:

```javascript
import { Shell } from '@bsull/conch';

const shell = new Shell();

// Variables persist between calls
shell.execute('greeting="Hello"');
shell.execute('name="World"');
const result = shell.execute('echo "$greeting, $name!"');
console.log(result.stdout);  // "Hello, World!\n"

// Functions persist too
shell.execute('greet() { echo "Hi, $1!"; }');
const result2 = shell.execute('greet Alice');
console.log(result2.stdout);  // "Hi, Alice!\n"

// Direct variable access
shell.setVar('count', '42');
console.log(shell.getVar('count'));  // "42"

// Check exit codes
const result3 = shell.execute('false');
console.log(result3.exitCode);  // 1

// Clean up when done (optional - GC will handle it otherwise)
shell.dispose();
```

### Stateless Execution

For simple one-off commands, use the `execute()` function. Each call creates a fresh shell instance:

```javascript
import { execute } from '@bsull/conch';

// Execute a shell script and capture output
const result = execute('echo "Hello, World!"');
console.log(result.exitCode);  // 0
console.log(result.stdout);    // "Hello, World!\n"
console.log(result.stderr);    // ""

// Script errors return non-zero exit codes (not exceptions)
const result2 = execute('ls /nonexistent');
console.log(result2.exitCode);  // 1
console.log(result2.stderr);    // error message

// Note: state does NOT persist between calls
execute('x=42');
const result3 = execute('echo $x');
console.log(result3.stdout);  // "\n" (x is not set - fresh instance!)
```

## API

### `ExecutionResult`

Both `Shell.execute()` and `execute()` return an `ExecutionResult`:

```typescript
interface ExecutionResult {
  exitCode: number;  // 0 for success, non-zero for failure
  stdout: string;    // Captured standard output
  stderr: string;    // Captured standard error
}
```

### `Shell` Class

A stateful shell instance with persistent state.

```typescript
class Shell {
  constructor();
  execute(script: string): ExecutionResult;
  getVar(name: string): string | undefined;
  setVar(name: string, value: string): void;
  lastExitCode(): number;
  dispose(): void;
}
```

- **`constructor()`**: Create a new shell with a clean environment
- **`execute(script)`**: Run a script, returns `ExecutionResult` with exit code and captured output
- **`getVar(name)`**: Get a variable's value (without the `$`)
- **`setVar(name, value)`**: Set a variable (like `name=value` in shell)
- **`lastExitCode()`**: Get the last command's exit code (`$?`)
- **`dispose()`**: Release WASM resources (optional, GC handles cleanup)

### `execute(script: string): ExecutionResult`

Execute a script in a fresh, stateless instance. Equivalent to:

```javascript
const shell = new Shell();
try {
  return shell.execute(script);
} finally {
  shell.dispose();
}
```

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
import { Shell } from '@bsull/conch';
import { setFileData, updateFile, deletePath, fromPaths } from '@bsull/conch/vfs';

// Set up the VFS
setFileData(fromPaths({
  '/data/input.txt': 'hello world',
  '/config/settings.json': '{"debug": true}'
}));

const shell = new Shell();

// Execute commands and capture output
const result = shell.execute('cat /data/input.txt');
console.log(result.stdout);  // "hello world"

// Update the VFS (works even after execute() has been called)
updateFile('/data/input.txt', 'updated content');
const result2 = shell.execute('cat /data/input.txt');
console.log(result2.stdout);  // "updated content"

// Add new files
updateFile('/data/new-file.txt', 'new content');

// Delete files
deletePath('/data/input.txt');
```

The VFS supports updates even after `execute()` has been called. This is achieved by mutating the underlying filesystem data in place, preserving the WASM shell's references.

## Stdin Configuration

```javascript
import { Shell } from '@bsull/conch';
import { setStdinData, clearStdin } from '@bsull/conch/stdin';

const shell = new Shell();

// Provide input data
setStdinData('line 1\nline 2\nline 3\n');
const result = shell.execute('head -n 2');
console.log(result.stdout);  // "line 1\nline 2\n"

// Clear stdin
clearStdin();
```

## Tool Handler

The shell supports a `tool` builtin for AI/LLM integration:

```javascript
import { Shell } from '@bsull/conch';
import { setToolHandler } from '@bsull/conch/tool-handler';

setToolHandler((request) => {
  if (request.tool === 'web_search') {
    const params = JSON.parse(request.params);
    // Perform the search...
    return { success: true, output: JSON.stringify(results) };
  }
  return { success: false, output: `Unknown tool: ${request.tool}` };
});

const shell = new Shell();
const result = shell.execute('tool web_search --query "rust wasm"');
console.log(result.stdout);  // tool output
```

## Browser & Node.js Support

This package works out of the box in both browsers and Node.js with the same behavior:

- Uses the in-memory virtual filesystem (VFS)
- `setFileData()`, `updateFile()`, and `deletePath()` work as expected
- No access to the real filesystem (sandboxed by design)
- No configuration or import aliasing required

**Requirements:** Node.js 19+ (for `globalThis.crypto`)

## Limitations

- **No real filesystem access**: The shell operates in a sandboxed virtual filesystem
- **No network access**: Network operations are not available in the sandbox
- **Single-threaded**: The shell runs in a single-threaded environment
- **Memory limits**: Subject to WebAssembly memory constraints
- **Some commands unavailable**: Commands like `ls`, `sed`, `awk`, `sort`, `printf` are not yet implemented

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