/**
 * Stdin configuration for conch-shell
 *
 * This module provides functions to configure stdin for the shell.
 * By default, stdin returns empty data (EOF) when read, which is the
 * correct behavior when no input is piped.
 *
 * The browser shims from @bytecodealliance/preview2-shim have an
 * unimplemented stdin that returns undefined, causing errors. This
 * module provides a proper default and allows setting custom stdin data.
 */

import { _setStdin } from "./shims/cli.js";

/**
 * @typedef {Object} StdinHandler
 * @property {function(BigInt): Uint8Array} blockingRead - Read up to len bytes
 * @property {function(): void} subscribe - Subscribe to stdin (for polling)
 */

/**
 * Buffer for stdin data that can be read by the shell.
 * @type {Uint8Array}
 */
let stdinBuffer = new Uint8Array(0);
let stdinOffset = 0;

/**
 * Default stdin handler that returns data from the buffer, or empty if exhausted.
 * @type {StdinHandler}
 */
const defaultStdinHandler = {
  blockingRead(len) {
    const remaining = stdinBuffer.length - stdinOffset;
    if (remaining <= 0) {
      // EOF - return empty array
      return new Uint8Array(0);
    }
    const toRead = Math.min(Number(len), remaining);
    const result = stdinBuffer.slice(stdinOffset, stdinOffset + toRead);
    stdinOffset += toRead;
    return result;
  },
  subscribe() {
    // No-op for now - polling not implemented
  },
  [Symbol.dispose || Symbol.for("dispose")]() {
    // No-op
  },
};

/**
 * Initialize stdin with the default empty handler.
 * Call this before execute() to ensure stdin reads don't crash.
 */
export function initStdin() {
  stdinBuffer = new Uint8Array(0);
  stdinOffset = 0;
  _setStdin(defaultStdinHandler);
}

/**
 * Set stdin data that will be returned when the shell reads from stdin.
 *
 * @param {string|Uint8Array} data - Data to provide as stdin
 *
 * @example
 * // Provide string input
 * setStdinData("hello world\n");
 *
 * // Provide binary input
 * setStdinData(new Uint8Array([0x48, 0x69]));
 */
export function setStdinData(data) {
  if (typeof data === "string") {
    stdinBuffer = new TextEncoder().encode(data);
  } else {
    stdinBuffer = data;
  }
  stdinOffset = 0;
  _setStdin(defaultStdinHandler);
}

/**
 * Clear stdin data (reset to empty).
 */
export function clearStdin() {
  stdinBuffer = new Uint8Array(0);
  stdinOffset = 0;
}

// Auto-initialize stdin with empty handler on module load
initStdin();
