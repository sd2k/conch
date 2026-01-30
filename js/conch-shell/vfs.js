/**
 * Virtual Filesystem API for conch-shell
 *
 * This module provides functions to set up and manipulate the virtual filesystem
 * that the shell operates in. The VFS is an in-memory filesystem that persists
 * for the lifetime of the module.
 *
 * The VFS supports updates even after execute() has been called. This is achieved
 * by mutating the underlying filesystem data in place rather than replacing it,
 * which allows the WASM shell's cached references to remain valid.
 *
 * In browsers, this works automatically via the preview2-shim browser build.
 * In Node.js, you need to alias @bytecodealliance/preview2-shim/* imports to
 * the browser shims (see vitest.config.ts for an example). Node 19+ provides
 * globalThis.crypto which the browser shims require.
 */

// Import VFS functions from our filesystem shim.
// This always uses browser implementations for consistent VFS behavior
// in both Node.js and browsers.
import { _setFileData, _getFileData, _setCwd } from "./shims/filesystem.js";

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

// Track the root VFS data object so we can mutate it in place
// This is the SAME object that gets passed to _setFileData and stored in the Descriptor
let _rootData = null;
let _initialized = false;

/**
 * Recursively sync target to match source by mutating target in place.
 * This preserves object references while updating contents.
 *
 * @param {Object} target - The object to mutate
 * @param {Object} source - The source data to sync from
 */
function syncInPlace(target, source) {
  // Remove keys from target that don't exist in source
  for (const key of Object.keys(target)) {
    if (!(key in source)) {
      delete target[key];
    }
  }

  // Update/add keys from source
  for (const [key, value] of Object.entries(source)) {
    if (value === null || value === undefined) {
      target[key] = value;
    } else if (ArrayBuffer.isView(value)) {
      // Typed arrays (Uint8Array, etc.) - assign directly
      target[key] = value;
    } else if (typeof value === "object") {
      // Object - check if we can recurse
      if (
        target[key] &&
        typeof target[key] === "object" &&
        !ArrayBuffer.isView(target[key])
      ) {
        // Both are plain objects, recurse to preserve references
        syncInPlace(target[key], value);
      } else {
        // Target doesn't have this key as an object, or it's a typed array
        // We need to create a new object but then sync into it for nested structures
        if (value.dir !== undefined || value.source !== undefined) {
          // This is a VFS entry (file or directory)
          target[key] = {};
          syncInPlace(target[key], value);
        } else {
          target[key] = value;
        }
      }
    } else {
      // Primitive value
      target[key] = value;
    }
  }
}

/**
 * Initialize or update the virtual filesystem with the given data structure.
 *
 * This function can be called multiple times, even after execute() has been
 * called. Subsequent calls will update the filesystem in place, preserving
 * the WASM shell's references to the data.
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
 *
 * // Later, update the filesystem (works even after execute())
 * setFileData({
 *   dir: {
 *     'hello.txt': { source: 'Updated content' },
 *     'new-file.txt': { source: 'New file!' }
 *   }
 * });
 */
export function setFileData(data) {
  if (!_initialized) {
    // First call - create our root data object and pass it to preview2-shim
    // We create our own object so we maintain a reference to it for future mutations
    _rootData = { dir: {} };
    syncInPlace(_rootData, data);
    _setFileData(_rootData);
    _initialized = true;
  } else {
    // Subsequent calls - mutate _rootData in place to preserve WASM references
    // The Descriptor holds a reference to _rootData, so mutations are visible
    syncInPlace(_rootData, data);
  }
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
 * Update a single file's content in the VFS.
 * This is more efficient than setFileData for single-file updates.
 *
 * @param {string} path - The file path (e.g., '/data/file.txt')
 * @param {string|Uint8Array} content - The new file content
 */
export function updateFile(path, content) {
  if (!_initialized || !_rootData) {
    // Initialize with this single file
    setFileData(fromPaths({ [path]: content }));
    return;
  }

  const parts = path.split("/").filter((p) => p.length > 0);
  let current = _rootData;

  // Navigate to parent directory, creating dirs as needed
  for (let i = 0; i < parts.length - 1; i++) {
    const part = parts[i];
    if (!current.dir) {
      current.dir = {};
    }
    if (!current.dir[part]) {
      current.dir[part] = { dir: {} };
    }
    current = current.dir[part];
  }

  // Set the file
  if (!current.dir) {
    current.dir = {};
  }
  const filename = parts[parts.length - 1];
  if (current.dir[filename]) {
    // Update existing file in place
    current.dir[filename].source = content;
  } else {
    // Create new file
    current.dir[filename] = { source: content };
  }
}

/**
 * Delete a file or directory from the VFS.
 *
 * @param {string} path - The path to delete (e.g., '/data/file.txt')
 * @returns {boolean} True if the path was deleted, false if it didn't exist
 */
export function deletePath(path) {
  if (!_initialized || !_rootData) {
    return false;
  }

  const parts = path.split("/").filter((p) => p.length > 0);
  if (parts.length === 0) {
    return false; // Can't delete root
  }

  let current = _rootData;

  // Navigate to parent directory
  for (let i = 0; i < parts.length - 1; i++) {
    const part = parts[i];
    if (!current.dir || !current.dir[part]) {
      return false; // Path doesn't exist
    }
    current = current.dir[part];
  }

  const name = parts[parts.length - 1];
  if (current.dir && name in current.dir) {
    delete current.dir[name];
    return true;
  }
  return false;
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
