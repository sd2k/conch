// Filesystem shim - always uses browser implementation for consistent behavior
// This re-exports the browser shim to ensure VFS works in both Node.js and browsers
export * from "@bytecodealliance/preview2-shim/filesystem";
