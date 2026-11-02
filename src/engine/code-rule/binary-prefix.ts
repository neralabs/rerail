import { constants } from "node:fs";
import { type FileHandle, open } from "node:fs/promises";

/** How much of an oversized unlisted file is checked for a NUL byte (Git's binary test), as it is never read whole. */
const BINARY_PREFIX_BYTES = 8000;

/** Whether an oversized file (an open one, or one at a path, never followed through a symlink) holds a NUL byte in its first `BINARY_PREFIX_BYTES`. */
export async function binaryPrefix(file: FileHandle | string): Promise<boolean> {
  const handle = typeof file === "string" ? await open(file, constants.O_RDONLY | constants.O_NOFOLLOW) : file;
  try {
    const { buffer, bytesRead } = await handle.read(Buffer.alloc(BINARY_PREFIX_BYTES), 0, BINARY_PREFIX_BYTES, 0);
    return buffer.subarray(0, bytesRead).includes(0);
  } finally {
    if (handle !== file) await handle.close();
  }
}
