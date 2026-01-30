// IO shim - always uses browser implementation for consistent behavior
// This re-exports the browser shim to ensure VFS works in both Node.js and browsers
export * from "../node_modules/@bytecodealliance/preview2-shim/lib/browser/io.js";
