/**
 * Virtual Filesystem API for conch-shell
 *
 * This module provides an in-memory virtual filesystem for the shell.
 * The VFS supports updates even after execute() has been called.
 *
 * In browsers, this works automatically via the preview2-shim browser build.
 * In Node.js, you need to alias @bytecodealliance/preview2-shim/* to the
 * browser shims (Node 19+ provides globalThis.crypto). See the testpkg
 * vitest.config.ts for an example configuration.
 */

export interface FileEntry {
  source: Uint8Array | string;
}

export interface DirEntry {
  dir: Record<string, FileEntry | DirEntry>;
}

export interface VfsData {
  dir: Record<string, FileEntry | DirEntry>;
}

/**
 * Initialize or update the virtual filesystem with the given data structure.
 *
 * This function can be called multiple times, even after execute() has been
 * called. Subsequent calls will update the filesystem in place, preserving
 * the WASM shell's references to the data.
 */
export function setFileData(data: VfsData): void;

/**
 * Get the current filesystem state as a JSON string.
 */
export function getFileData(): string;

/**
 * Set the current working directory.
 */
export function setCwd(path: string): void;

/**
 * Update a single file's content in the VFS.
 * This is more efficient than setFileData for single-file updates.
 *
 * @param path - The file path (e.g., '/data/file.txt')
 * @param content - The new file content
 */
export function updateFile(path: string, content: string | Uint8Array): void;

/**
 * Delete a file or directory from the VFS.
 *
 * @param path - The path to delete (e.g., '/data/file.txt')
 * @returns True if the path was deleted, false if it didn't exist
 */
export function deletePath(path: string): boolean;

/**
 * Helper to create a file entry from a string.
 */
export function file(content: string): FileEntry;

/**
 * Helper to create a file entry from binary data.
 */
export function binaryFile(data: Uint8Array): FileEntry;

/**
 * Helper to create a directory entry.
 */
export function dir(contents: Record<string, FileEntry | DirEntry>): DirEntry;

/**
 * Create a complete VFS structure from a flat path map.
 */
export function fromPaths(files: Record<string, string | Uint8Array>): VfsData;
