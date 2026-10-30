import { existsSync, globSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";

/**
 * A checkout cloned without Git LFS holds small text pointers in place of the files `.gitattributes` stores in LFS:
 * the model's weights, the engine's names table, a committed parser. Everything builds and installs around them, and
 * then every session's attach fails on the model's checksum. `scripts/build/require-lfs.mjs` holds the build to the
 * same check.
 */
export const LFS_REMEDY = "git lfs install && git lfs pull";

const POINTER = Buffer.from("version https://git-lfs");
/** A pointer is three short lines; LFS content is far larger. */
const POINTER_MAX_BYTES = 1024;
/** Shipped in the package, which carries no `.gitattributes`. */
const SHIPPED_LFS_FILES = ["model/model.safetensors"];

/** The paths `root/.gitattributes` stores in LFS, as globs relative to `root`. */
function lfsPatterns(root: string): string[] {
  const attributes = join(root, ".gitattributes");
  if (!existsSync(attributes)) return [];
  const patterns: string[] = [];
  for (const raw of readFileSync(attributes, "utf8").split("\n")) {
    const line = raw.trim();
    if (line === "" || line.startsWith("#")) continue;
    const [pattern = "", ...settings] = line.split(/\s+/);
    if (!settings.includes("filter=lfs")) continue;
    // Without a slash a pattern matches a file's name at any depth; with one it is anchored to the root.
    patterns.push(pattern.replace(/^\//, "").includes("/") ? pattern.replace(/^\//, "") : `**/${pattern}`);
  }
  return patterns;
}

/** Whether the file at `path` is a Git LFS pointer rather than the content it stands for. */
export function isLfsPointer(path: string): boolean {
  const stats = statSync(path, { throwIfNoEntry: false });
  if (stats === undefined || !stats.isFile() || stats.size > POINTER_MAX_BYTES) return false;
  return readFileSync(path).subarray(0, POINTER.length).equals(POINTER);
}

/** The files under `root` stored in Git LFS that hold only their pointer, relative to `root`, sorted. */
export function lfsPointerFiles(root: string): string[] {
  const candidates = new Set(SHIPPED_LFS_FILES);
  for (const path of globSync(lfsPatterns(root), { cwd: root })) candidates.add(path.replaceAll("\\", "/"));
  return [...candidates].filter(path => isLfsPointer(join(root, path))).sort();
}

/** Refuses a package root whose Git LFS files are pointers, naming them and the command that fetches them. */
export function requireLfsContent(root: string): void {
  const pointers = lfsPointerFiles(root);
  if (pointers.length === 0) return;
  throw new Error(
    `${root} holds Git LFS pointers in place of ${pointers.join(", ")}: run \`${LFS_REMEDY}\` there, then rebuild (npm run build) and install again`,
  );
}
