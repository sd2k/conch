/**
 * Conch Shell - JavaScript API
 *
 * This module provides a high-level API for the conch-shell WebAssembly component.
 */

import { ConchShellShell } from "./interfaces/conch-shell-shell.js";

/**
 * The low-level shell interface from jco-generated bindings.
 */
export const shell: typeof ConchShellShell;

/**
 * Result from executing a shell command.
 */
export interface ExecutionResult {
  /** The exit code (0 for success, non-zero for failure) */
  exitCode: number;
  /** Captured standard output */
  stdout: string;
  /** Captured standard error */
  stderr: string;
}

/**
 * Tool invocation request from the shell.
 */
export interface ToolRequest {
  /** The tool name (e.g., "web_search", "code_edit") */
  tool: string;
  /** JSON-encoded parameters */
  params: string;
  /** Optional stdin data piped to the tool */
  stdin?: Uint8Array;
}

/**
 * Tool invocation result from the host.
 */
export interface ToolResult {
  /** Whether the tool succeeded */
  success: boolean;
  /** Output from the tool (stdout on success, error message on failure) */
  output: string;
}

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
  /**
   * Create a new Shell instance.
   *
   * The shell starts with a clean environment. Each Shell instance
   * has its own isolated state.
   */
  constructor();

  /**
   * Execute a shell script and capture output.
   *
   * The script is interpreted as bash-compatible shell code.
   * Variables, functions, and aliases defined here persist for
   * subsequent execute() calls on this instance.
   *
   * @param script - The shell script to execute
   * @returns The exit code and captured stdout/stderr
   * @throws If the shell itself fails (not script errors)
   */
  execute(script: string): ExecutionResult;

  /**
   * Get a shell variable's value.
   *
   * @param name - The variable name (without $)
   * @returns The value, or undefined if not set
   */
  getVar(name: string): string | undefined;

  /**
   * Set a shell variable.
   *
   * This is equivalent to running `name=value` in the shell.
   *
   * @param name - The variable name
   * @param value - The value to set
   */
  setVar(name: string, value: string): void;

  /**
   * Get the exit code from the last executed command ($?).
   *
   * @returns The last exit code
   */
  lastExitCode(): number;

  /**
   * Dispose of the shell instance and release WASM resources.
   *
   * After calling dispose(), the shell instance cannot be used.
   * This is optional - the garbage collector will clean up eventually,
   * but calling dispose() explicitly frees resources immediately.
   */
  dispose(): void;

  /**
   * Support for explicit resource management (using declaration).
   */
  [Symbol.dispose](): void;
}

/**
 * Execute a shell script in a fresh, stateless instance.
 *
 * This function creates a new Shell instance for each call, so state
 * does not persist between calls. Use the Shell class directly if you
 * need state persistence.
 *
 * @param script - The shell script to execute
 * @returns The exit code and captured stdout/stderr
 * @throws If the shell fails to initialize
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
export function execute(script: string): ExecutionResult;
