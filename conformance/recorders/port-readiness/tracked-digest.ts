import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";

const sha256 = (bytes: string | Buffer): string => createHash("sha256").update(bytes).digest("hex");

/**
 * sha256 over each tracked file `git ls-files` lists under `pathspec` at `root`: its path and the sha256 of its bytes in
 * the working tree (a tracked file deleted there counts as deleted).
 */
export function trackedDigest(root: string, pathspec: string | readonly string[]): string {
  const listed = execFileSync("git", ["ls-files", "-z", "--", ...(typeof pathspec === "string" ? [pathspec] : pathspec)], { cwd: root, encoding: "utf8", maxBuffer: 1 << 28 })
    .split("\0")
    .filter((path) => path !== "")
    .sort((a, b) => (a < b ? -1 : a > b ? 1 : 0));
  return sha256([...new Set(listed)].map((path) => {
    const absolute = join(root, path);
    return `${path}\0${existsSync(absolute) ? sha256(readFileSync(absolute)) : "deleted"}`;
  }).join("\n"));
}
