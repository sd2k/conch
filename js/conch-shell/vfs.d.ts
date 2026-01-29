/**
 * Virtual Filesystem API for conch-shell
 *
 * This module provides an in-memory virtual filesystem for the shell.
 *
 * IMPORTANT: The shell caches its filesystem preopens on first execution.
 * You must call setFileData() BEFORE any execute() calls to set up your VFS.
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
 * Initialize the virtual filesystem with the given data structure.
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
