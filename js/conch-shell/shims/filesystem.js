// Filesystem shim - extends browser implementation with fixes for missing features
// Patches several stub methods in the base preview2-shim that just log to console

import {
  preopens as basePreopens,
  types as baseTypes,
  _setFileData,
  _getFileData,
  _setCwd,
} from "@bytecodealliance/preview2-shim/filesystem";

import { streams } from "@bytecodealliance/preview2-shim/io";

// =============================================================================
// Direct VFS manipulation without going through vfs.js
//
// The key insight: preview2-shim's _fileData is the actual VFS. The Descriptor
// objects have #entry pointing to parts of this _fileData. When we call
// _setFileData(newData), it replaces _fileData BUT old descriptors still have
// #entry pointing to the OLD _fileData.
//
// To make mutations visible to existing descriptors, we need to:
// 1. Get a reference to the SAME object that descriptors use
// 2. Mutate it in place
//
// Since we can't get a direct reference to _fileData, we use this trick:
// - Call _setFileData with OUR object BEFORE any WASM operations
// - Keep a reference to our object
// - Mutate our object directly
//
// This works because the shim is imported before any Shell is created.
// =============================================================================

// Our VFS data object - this will become _fileData after initialization
let _vfsData = { dir: {} };
let _vfsInitialized = false;

// Initialize immediately at import time
// This happens BEFORE any Shell is created, so no stale descriptors
_setFileData(_vfsData);
_vfsInitialized = true;

// Direct delete from our VFS data
function directDeletePath(path) {
  const parts = path.split("/").filter((p) => p.length > 0);
  if (parts.length === 0) return false;

  let current = _vfsData;
  for (let i = 0; i < parts.length - 1; i++) {
    const part = parts[i];
    if (!current.dir || !current.dir[part]) {
      return false;
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

// Direct update file in our VFS data
function directUpdateFile(path, content) {
  const parts = path.split("/").filter((p) => p.length > 0);
  let current = _vfsData;

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
  current.dir[filename] = { source: content };
}

// Direct create directory in our VFS data
function directCreateDir(path) {
  const parts = path.split("/").filter((p) => p.length > 0);
  let current = _vfsData;

  // Navigate/create all directories
  for (const part of parts) {
    if (!current.dir) {
      current.dir = {};
    }
    if (!current.dir[part]) {
      current.dir[part] = { dir: {} };
    }
    current = current.dir[part];
  }
}

const { OutputStream } = streams;

// Re-export the helpers
export { _setFileData, _getFileData, _setCwd };

// Get the original Descriptor class
const OriginalDescriptor = baseTypes.Descriptor;

// =============================================================================
// Patch appendViaStream - was just a stub that logged to console
// =============================================================================
OriginalDescriptor.prototype.appendViaStream = function () {
  // Use writeViaStream at the current file size to implement append
  try {
    const stat = this.stat();
    const offset = stat.size;
    return this.writeViaStream(offset);
  } catch (e) {
    console.warn("[filesystem] appendViaStream failed:", e);
    throw e;
  }
};

// =============================================================================
// Helper to check if a path exists and what type it is
// =============================================================================
function getPathInfo(path) {
  const parts = path.split("/").filter((p) => p.length > 0);

  let current = _vfsData;
  for (const part of parts) {
    if (!current || !current.dir || !current.dir[part]) {
      return { exists: false };
    }
    current = current.dir[part];
  }

  return {
    exists: true,
    isDirectory: !!current.dir,
    isFile: current.source !== undefined,
  };
}

// =============================================================================
// Patch unlinkFileAt - needed for rm
// =============================================================================
OriginalDescriptor.prototype.unlinkFileAt = function (path) {
  // Normalize path (handle both absolute and relative paths)
  const fullPath = path.startsWith("/") ? path : "/" + path;

  // Check if the path exists and is a file
  const info = getPathInfo(fullPath);
  if (!info.exists) {
    throw "no-entry";
  }

  if (info.isDirectory) {
    throw "is-directory";
  }

  // Delete directly from our VFS data
  if (!directDeletePath(fullPath)) {
    throw "no-entry";
  }
};

// =============================================================================
// Patch removeDirectoryAt - needed for rmdir
// =============================================================================
OriginalDescriptor.prototype.removeDirectoryAt = function (path) {
  const fullPath = path.startsWith("/") ? path : "/" + path;

  const info = getPathInfo(fullPath);
  if (!info.exists) {
    throw "no-entry";
  }

  if (!info.isDirectory) {
    throw "not-directory";
  }

  // Check if directory is empty
  const parts = fullPath.split("/").filter((p) => p.length > 0);
  let current = _vfsData;
  for (const part of parts) {
    current = current.dir[part];
  }
  if (Object.keys(current.dir).length > 0) {
    throw "not-empty";
  }

  // Delete directly from our VFS data
  if (!directDeletePath(fullPath)) {
    throw "no-entry";
  }
};

// =============================================================================
// Patch setTimesAt - needed for touch
// =============================================================================
OriginalDescriptor.prototype.setTimesAt = function (
  _pathFlags,
  path,
  dataAccessTimestamp,
  dataModificationTimestamp
) {
  // For now, just ensure the file exists (touch behavior)
  // The VFS doesn't actually track timestamps, but we can create the file
  try {
    this.statAt({}, path);
    // File exists, timestamps would be updated (no-op in our VFS)
  } catch (e) {
    if (e === "no-entry") {
      // Create the file (touch creates files that don't exist)
      this.openAt({}, path, { create: true }, {}, {});
    } else {
      throw e;
    }
  }
};

// =============================================================================
// Patch renameAt - needed for mv
// =============================================================================
OriginalDescriptor.prototype.renameAt = function (
  oldPath,
  newDescriptor,
  newPath
) {
  const fullOldPath = oldPath.startsWith("/") ? oldPath : "/" + oldPath;
  const fullNewPath = newPath.startsWith("/") ? newPath : "/" + newPath;

  // Check source exists
  const oldInfo = getPathInfo(fullOldPath);
  if (!oldInfo.exists) {
    throw "no-entry";
  }

  // Check destination
  const newInfo = getPathInfo(fullNewPath);
  if (newInfo.exists) {
    // If source is dir and dest is file, error
    if (oldInfo.isDirectory && newInfo.isFile) throw "not-directory";
    // If source is file and dest is dir, error
    if (oldInfo.isFile && newInfo.isDirectory) throw "is-directory";
  }

  // Get the entry directly from _vfsData
  const oldParts = fullOldPath.split("/").filter((p) => p.length > 0);
  let oldEntry = _vfsData;
  for (const part of oldParts) {
    oldEntry = oldEntry.dir[part];
  }

  // Save the original content/dir reference BEFORE deleting
  // Don't use JSON clone - it corrupts Uint8Array to plain objects
  const isFile = oldEntry.source !== undefined;
  const savedSource = oldEntry.source; // Keep reference to original content
  const savedDir = oldEntry.dir; // Keep reference to original dir

  // Delete old path
  if (!directDeletePath(fullOldPath)) {
    throw "no-entry";
  }

  // Create new path directly using the saved reference
  if (isFile) {
    directUpdateFile(fullNewPath, savedSource);
  } else if (savedDir !== undefined) {
    // It's a directory - recreate structure with original references
    directCreateDir(fullNewPath);
    recreateDirectoryWithRefs(fullNewPath, savedDir);
  }
};

// Helper to recreate a directory structure preserving original content references
function recreateDirectoryWithRefs(basePath, dirContents) {
  for (const [name, child] of Object.entries(dirContents)) {
    const childPath = basePath + "/" + name;
    if (child.source !== undefined) {
      // File - use the original source reference (preserves Uint8Array)
      directUpdateFile(childPath, child.source);
    } else if (child.dir !== undefined) {
      // Directory - recurse
      directCreateDir(childPath);
      recreateDirectoryWithRefs(childPath, child.dir);
    }
  }
}

// =============================================================================
// Patch setSize - needed for truncate
// NOTE: This is limited because we don't know which file this descriptor refers to
// =============================================================================
OriginalDescriptor.prototype.setSize = function (size) {
  // setSize is called on a file descriptor, but we don't have a way to know
  // which file it refers to without access to the private #entry field.
  // This is a no-op for now - truncation may not work as expected.
};

// Export the types (we patch the prototype, so original types work)
export const preopens = basePreopens;
export const types = baseTypes;
