/**
 * Conch Shell - JavaScript API
 *
 * This module provides a high-level API for the conch-shell WebAssembly component.
 *
 * Two usage patterns are supported:
 *
 * 1. Stateful Shell (recommended):
 *    ```js
 *    import { Shell } from '@bsull/conch';
 *    const shell = new Shell();
 *    shell.execute('x=42');
 *    const result = shell.execute('echo $x');
 *    console.log(result.stdout);  // "42\n"
 *    ```
 *
 * 2. Stateless execute (backward compatible):
 *    ```js
 *    import { execute } from '@bsull/conch';
 *    const result = execute('echo hello');
 *    console.log(result.stdout);  // "hello\n"
 *    ```
 */

// Import the jco-generated bindings
import { shell } from "./conch-shell.js";
import { _setStdout, _setStderr } from "./shims/cli.js";

// Re-export the low-level Instance class for advanced use cases
export { shell };

const symbolDispose = Symbol.dispose ?? Symbol.for("dispose");
const textDecoder = new TextDecoder();

/**
 * Create an output stream handler that captures to a buffer array.
 * @param {string[]} buffer - Array to push output chunks to
 */
function createCaptureHandler(buffer) {
  return {
    write(contents) {
      buffer.push(textDecoder.decode(contents));
    },
    blockingFlush() {},
    [symbolDispose]() {},
  };
}

/**
 * Result from executing a shell command.
 * @typedef {Object} ExecutionResult
 * @property {number} exitCode - The exit code (0 for success)
 * @property {string} stdout - Captured standard output
 * @property {string} stderr - Captured standard error
 */

/**
 * A stateful shell instance that maintains state across executions.
 *
 * Variables, functions, and aliases defined in one execute() call
 * persist for subsequent calls on the same Shell instance.
 *
 * @example
 * const shell = new Shell();
 *
 * // Variables persist
 * shell.execute('greeting="Hello"');
 * const result = shell.execute('echo "$greeting World"');
 * console.log(result.stdout);  // "Hello World\n"
 *
 * // Functions persist
 * shell.execute('greet() { echo "Hi, $1!"; }');
 * const result2 = shell.execute('greet Alice');
 * console.log(result2.stdout);  // "Hi, Alice!\n"
 *
 * // Direct variable access
 * shell.setVar('name', 'Bob');
 * console.log(shell.getVar('name'));  // "Bob"
 */
export class Shell {
  #instance;

  /**
   * Create a new Shell instance.
   *
   * The shell starts with a clean environment. Each Shell instance
   * has its own isolated state.
   */
  constructor() {
    this.#instance = new shell.Instance();
  }

  /**
   * Execute a shell script and capture output.
   *
   * The script is interpreted as bash-compatible shell code.
   * Variables, functions, and aliases defined here persist for
   * subsequent execute() calls on this instance.
   *
   * @param {string} script - The shell script to execute
   * @returns {ExecutionResult} The exit code and captured stdout/stderr
   * @throws {Error} If the shell itself fails (not script errors)
   *
   * @example
   * const result = shell.execute('echo hello');
   * console.log(result.exitCode);  // 0
   * console.log(result.stdout);    // "hello\n"
   * console.log(result.stderr);    // ""
   */
  execute(script) {
    // Set up capture buffers
    const stdoutBuffer = [];
    const stderrBuffer = [];

    _setStdout(createCaptureHandler(stdoutBuffer));
    _setStderr(createCaptureHandler(stderrBuffer));

    try {
      const exitCode = this.#instance.execute(script);
      return {
        exitCode,
        stdout: stdoutBuffer.join(""),
        stderr: stderrBuffer.join(""),
      };
    } catch (e) {
      // Even on error, return any captured output
      return {
        exitCode: -1,
        stdout: stdoutBuffer.join(""),
        stderr: stderrBuffer.join("") + (e.message || String(e)),
      };
    }
  }

  /**
   * Get a shell variable's value.
   *
   * @param {string} name - The variable name (without $)
   * @returns {string|undefined} The value, or undefined if not set
   *
   * @example
   * shell.execute('x=42');
   * console.log(shell.getVar('x'));  // "42"
   * console.log(shell.getVar('y'));  // undefined
   */
  getVar(name) {
    return this.#instance.getVar(name);
  }

  /**
   * Set a shell variable.
   *
   * This is equivalent to running `name=value` in the shell.
   *
   * @param {string} name - The variable name
   * @param {string} value - The value to set
   *
   * @example
   * shell.setVar('greeting', 'Hello');
   * shell.execute('echo $greeting');  // stdout: "Hello\n"
   */
  setVar(name, value) {
    this.#instance.setVar(name, value);
  }

  /**
   * Get the exit code from the last executed command ($?).
   *
   * @returns {number} The last exit code
   *
   * @example
   * shell.execute('true');
   * console.log(shell.lastExitCode());  // 0
   * shell.execute('false');
   * console.log(shell.lastExitCode());  // 1
   */
  lastExitCode() {
    return this.#instance.lastExitCode();
  }

  /**
   * Dispose of the shell instance and release WASM resources.
   *
   * After calling dispose(), the shell instance cannot be used.
   * This is optional - the garbage collector will clean up eventually,
   * but calling dispose() explicitly frees resources immediately.
   */
  dispose() {
    if (this.#instance && this.#instance[symbolDispose]) {
      this.#instance[symbolDispose]();
    }
    this.#instance = null;
  }

  /**
   * Support for explicit resource management (using declaration).
   */
  [Symbol.dispose]() {
    this.dispose();
  }
}

/**
 * Execute a shell script in a fresh, stateless instance.
 *
 * This function creates a new Shell instance for each call, so state
 * does not persist between calls. Use the Shell class directly if you
 * need state persistence.
 *
 * @param {string} script - The shell script to execute
 * @returns {ExecutionResult} The exit code and captured stdout/stderr
 * @throws {Error} If the shell fails to initialize
 *
 * @example
 * const result = execute('echo hello');
 * console.log(result.exitCode);  // 0
 * console.log(result.stdout);    // "hello\n"
 *
 * // State does NOT persist
 * execute('x=42');
 * const result2 = execute('echo $x');
 * console.log(result2.stdout);  // "\n" (x is not set)
 */
export function execute(script) {
  const shellInstance = new Shell();
  try {
    return shellInstance.execute(script);
  } finally {
    shellInstance.dispose();
  }
}
