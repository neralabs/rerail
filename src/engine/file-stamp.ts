import type { Stats } from "node:fs";

/**
 * A file's device, inode, size and times, as one string: a write to the file, or a new file in its place, changes it. A
 * walk keeps one per file it sees, for every file of the checkout and in each session and walk at once, so the stamp is
 * built with `join`, which makes one flat string: a template literal makes a rope of ten strings, about 340 bytes where
 * the flat string takes about 70.
 */
export function fileStamp(info: Pick<Stats, "dev" | "ino" | "size" | "mtimeMs" | "ctimeMs">): string {
  return [info.dev, info.ino, info.size, info.mtimeMs, info.ctimeMs].join(":");
}
