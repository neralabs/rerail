// The structure gate's only view of the repository: the tracked file list and file contents.
//
// A repo is `{ root, files, has, text, bytes, size, isBinary }`. `files` is the sorted list of
// repo-relative POSIX paths from `git ls-files` (tracked files still present in the working tree);
// the readers return null/0 for paths outside it. `loadRepo` reads the working tree lazily and caches;
// `memoryRepo` builds the same interface from an object of path → contents, for tests.

import { execFileSync } from "node:child_process";
import { readFileSync, statSync } from "node:fs";
import { join } from "node:path";

const BINARY_SNIFF_BYTES = 8000;

function compare(a, b) {
  return a < b ? -1 : a > b ? 1 : 0;
}

function makeRepo(root, files, readBytes, sizeOf) {
  const tracked = new Set(files);
  const bytesCache = new Map();
  const textCache = new Map();
  const bytes = (path) => {
    if (!tracked.has(path)) return null;
    if (!bytesCache.has(path)) bytesCache.set(path, readBytes(path));
    return bytesCache.get(path);
  };
  return {
    root,
    files,
    has: (path) => tracked.has(path),
    bytes,
    text(path) {
      if (!tracked.has(path)) return null;
      if (!textCache.has(path)) textCache.set(path, bytes(path).toString("utf8"));
      return textCache.get(path);
    },
    size: (path) => (tracked.has(path) ? sizeOf(path) : 0),
    isBinary(path) {
      const data = bytes(path);
      return data !== null && data.subarray(0, BINARY_SNIFF_BYTES).includes(0);
    },
  };
}

/** The tracked files of the git checkout at `root`. */
export function loadRepo(root) {
  const listed = execFileSync("git", ["ls-files", "-z"], { cwd: root, encoding: "utf8", maxBuffer: 1 << 28 })
    .split("\0")
    .filter((path) => path !== "");
  const sizes = new Map();
  for (const path of new Set(listed)) {
    try {
      const stat = statSync(join(root, path));
      if (stat.isFile()) sizes.set(path, stat.size);
    } catch {
      // Deleted in the working tree: not part of the checkout being checked.
    }
  }
  const files = [...sizes.keys()].sort(compare);
  return makeRepo(root, files, (path) => readFileSync(join(root, path)), (path) => sizes.get(path));
}

/** A repo whose tracked files are the keys of `entries` (contents: string or Buffer). */
export function memoryRepo(entries, root = "/memory") {
  const files = Object.keys(entries).sort(compare);
  const bytesOf = (path) => {
    const value = entries[path];
    return Buffer.isBuffer(value) ? value : Buffer.from(value, "utf8");
  };
  return makeRepo(root, files, bytesOf, (path) => bytesOf(path).length);
}

export { compare };
