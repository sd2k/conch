/**
 * Tool handler module for conch-shell
 *
 * This module provides the invoke-tool implementation that the shell calls
 * when the `tool` builtin is executed. Users can set a custom handler to
 * implement tool execution logic.
 *
 * @example
 * import { setToolHandler } from "@bsull/conch/tool-handler";
 * import { execute } from "@bsull/conch";
 *
 * setToolHandler((request) => {
 *   if (request.tool === "web_search") {
 *     // Do the search...
 *     return { success: true, output: JSON.stringify(results) };
 *   }
 *   return { success: false, output: `Unknown tool: ${request.tool}` };
 * });
 *
 * execute('result=$(tool web_search --query "rust"); echo $result');
 */

/**
 * @typedef {Object} ToolRequest
 * @property {string} tool - The tool name
 * @property {string} params - JSON-encoded parameters
 * @property {Uint8Array} [stdin] - Optional stdin data
 */

/**
 * @typedef {Object} ToolResult
 * @property {boolean} success - Whether the tool succeeded
 * @property {string} output - Output on success, error message on failure
 */

/**
 * @callback ToolHandler
 * @param {ToolRequest} request - The tool request
 * @returns {ToolResult} The result
 */

/** @type {ToolHandler} */
let currentHandler = (request) => {
  return {
    success: false,
    output: `No tool handler set. Cannot execute tool: ${request.tool}`,
  };
};

/**
 * Set the tool handler function.
 *
 * @param {ToolHandler} handler - Function to handle tool requests
 */
export function setToolHandler(handler) {
  currentHandler = handler;
}

/**
 * Get the current tool handler.
 *
 * @returns {ToolHandler}
 */
export function getToolHandler() {
  return currentHandler;
}

/**
 * Default export - the invoke function called by the WASM component.
 * This is mapped via jco's --map option.
 *
 * @param {ToolRequest} request
 * @returns {ToolResult}
 */
export default function invokeTool(request) {
  return currentHandler(request);
}
