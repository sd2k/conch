/**
 * Virtual Filesystem API for conch-shell
 *
 * This module provides functions to set up and manipulate the virtual filesystem
 * that the shell operates in. The VFS is an in-memory filesystem that persists
 * for the lifetime of the module.
 *
 * IMPORTANT: The shell caches its filesystem preopens on first execution.
 * You must call setFileData() BEFORE any execute() calls to set up your VFS.
 * Calling setFileData() after execute() will not update the shell's view of
 * the filesystem.
 *
 * In browsers, this works automatically via the preview2-shim browser build.
 * In Node.js, you need to alias @bytecodealliance/preview2-shim/* imports to
 * the browser shims (see vitest.config.ts for an example). Node 19+ provides
 * globalThis.crypto which the browser shims require.
 */

// Import VFS functions from the filesystem module.
// In browsers, this resolves to the browser shim with in-memory VFS.
// In Node.js tests, vitest aliases this to the browser shim.
import {
  _setFileData,
  _getFileData,
  _setCwd,
} from "@bytecodealliance/preview2-shim/filesystem";

/**
 * @typedef {Object} FileEntry
 * @property {Uint8Array|string} source - File contents
 */

/**
 * @typedef {Object} DirEntry
 * @property {Object.<string, FileEntry|DirEntry>} dir - Directory contents
 */

/**
 * @typedef {Object} VfsData
 * @property {Object.<string, FileEntry|DirEntry>} dir - Root directory contents
 */

/**
 * Initialize the virtual filesystem with the given data structure.
 *
 * @param {VfsData} data - The filesystem structure
 *
 * @example
 * // Create a simple filesystem
 * setFileData({
 *   dir: {
 *     'hello.txt': { source: 'Hello, World!' },
 *     'data': {
 *       dir: {
 *         'numbers.txt': { source: '1\n2\n3\n' }
 *       }
 *     }
 *   }
 * });
 */
export function setFileData(data) {
  _setFileData(data);
}

/**
 * Get the current filesystem state as a JSON string.
 * Note: Binary file contents will be serialized.
 *
 * @returns {string} JSON representation of the filesystem
 */
export function getFileData() {
  return _getFileData();
}

/**
 * Set the current working directory.
 *
 * @param {string} path - The path to set as cwd (e.g., '/data')
 */
export function setCwd(path) {
  _setCwd(path);
}

/**
 * Helper to create a file entry from a string.
 *
 * @param {string} content - The file content
 * @returns {FileEntry}
 */
export function file(content) {
  return { source: content };
}

/**
 * Helper to create a file entry from binary data.
 *
 * @param {Uint8Array} data - The binary content
 * @returns {FileEntry}
 */
export function binaryFile(data) {
  return { source: data };
}

/**
 * Helper to create a directory entry.
 *
 * @param {Object.<string, FileEntry|DirEntry>} contents - Directory contents
 * @returns {DirEntry}
 */
export function dir(contents) {
  return { dir: contents };
}

/**
 * Create a complete VFS structure from a flat path map.
 *
 * @param {Object.<string, string|Uint8Array>} files - Map of paths to contents
 * @returns {VfsData}
 *
 * @example
 * const vfs = fromPaths({
 *   '/hello.txt': 'Hello!',
 *   '/data/numbers.txt': '1\n2\n3\n',
 *   '/data/nested/deep.txt': 'deep file'
 * });
 * setFileData(vfs);
 */
export function fromPaths(files) {
  const root = { dir: {} };

  for (const [path, content] of Object.entries(files)) {
    const parts = path.split("/").filter((p) => p.length > 0);
    let current = root;

    for (let i = 0; i < parts.length - 1; i++) {
      const part = parts[i];
      if (!current.dir[part]) {
        current.dir[part] = { dir: {} };
      }
      current = current.dir[part];
    }

    const filename = parts[parts.length - 1];
    current.dir[filename] = {
      source: typeof content === "string" ? content : content,
    };
  }

  return root;
}
