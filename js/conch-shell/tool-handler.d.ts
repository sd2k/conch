/**
 * Tool request from the shell's `tool` builtin.
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
 * Result returned to the shell after tool execution.
 */
export interface ToolResult {
  /** Whether the tool succeeded */
  success: boolean;
  /** Output on success, error message on failure */
  output: string;
}

/**
 * Tool handler function signature.
 */
export type ToolHandler = (request: ToolRequest) => ToolResult;

/**
 * Set the tool handler function that will be called when the shell
 * executes the `tool` builtin.
 */
export function setToolHandler(handler: ToolHandler): void;

/**
 * Get the current tool handler.
 */
export function getToolHandler(): ToolHandler;

/**
 * Default export - the invoke function called by the WASM component.
 */
export default function invokeTool(request: ToolRequest): ToolResult;
