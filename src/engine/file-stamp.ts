import type { Stats } from "node:fs";

/** A file's device, inode, size and times, as one string: a write to the file, or a new file in its place, changes it. */
export function fileStamp(info: Stats): string {
  return `${info.dev}:${info.ino}:${info.size}:${info.mtimeMs}:${info.ctimeMs}`;
}
