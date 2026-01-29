/**
 * Initialize stdin with the default empty handler.
 * Call this before execute() to ensure stdin reads don't crash.
 * This is called automatically when the module is imported.
 */
export function initStdin(): void;

/**
 * Set stdin data that will be returned when the shell reads from stdin.
 *
 * @param data - Data to provide as stdin (string or Uint8Array)
 */
export function setStdinData(data: string | Uint8Array): void;

/**
 * Clear stdin data (reset to empty).
 */
export function clearStdin(): void;
