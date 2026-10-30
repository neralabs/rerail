// Preflight for build:native: a checkout cloned without Git LFS holds text pointers in place of the files
// .gitattributes stores in LFS, and builds around them: the engine embeds the names table's pointer, and every session
// later fails on the model's checksum. Self-contained, as a fresh checkout has no dist/; `install --root` runs the same
// check through src/setup/lfs-pointers.ts.
import { existsSync, globSync, readFileSync, statSync } from "node:fs";
import { join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const checkoutRoot = resolve(fileURLToPath(new URL("../..", import.meta.url)));
const POINTER = Buffer.from("version https://git-lfs");
/** A pointer is three short lines; LFS content is far larger. */
const POINTER_MAX_BYTES = 1024;
export const LFS_REMEDY = "git lfs install && git lfs pull";

/** The paths `root/.gitattributes` stores in LFS, as globs relative to `root`. */
function lfsPatterns(root) {
  const attributes = join(root, ".gitattributes");
  if (!existsSync(attributes)) return [];
  const patterns = [];
  for (const raw of readFileSync(attributes, "utf8").split("\n")) {
    const line = raw.trim();
    if (line === "" || line.startsWith("#")) continue;
    const [pattern, ...settings] = line.split(/\s+/u);
    if (!settings.includes("filter=lfs")) continue;
    // Without a slash a pattern matches a file's name at any depth; with one it is anchored to the root.
    const anchored = pattern.replace(/^\//u, "");
    patterns.push(anchored.includes("/") ? anchored : `**/${anchored}`);
  }
  return patterns;
}

function isLfsPointer(path) {
  const stats = statSync(path, { throwIfNoEntry: false });
  if (stats === undefined || !stats.isFile() || stats.size > POINTER_MAX_BYTES) return false;
  return readFileSync(path).subarray(0, POINTER.length).equals(POINTER);
}

/** The files under `root` that `.gitattributes` stores in Git LFS and that hold only their pointer, sorted. */
export function lfsPointerFiles(root = checkoutRoot) {
  return globSync(lfsPatterns(root), { cwd: root })
    .map(path => path.replaceAll("\\", "/"))
    .filter(path => isLfsPointer(join(root, path)))
    .sort();
}

if (process.argv[1] !== undefined && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  const pointers = lfsPointerFiles();
  if (pointers.length > 0) {
    console.error(`Git LFS pointers in place of ${pointers.join(", ")}: run \`${LFS_REMEDY}\`, then build again`);
    process.exit(1);
  }
}
