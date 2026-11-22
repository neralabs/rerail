/**
 * A file that exists and cannot be opened, as a person's own settings or another program can make one, for the tests of
 * what Rerail does with a file it may stat and not read.
 */
import { chmodSync, closeSync, constants, openSync } from "node:fs";

/**
 * Makes the file at `path` one that cannot be opened until what it returns runs: mode 000 on Linux and macOS (root
 * opens it all the same); on Windows held open by this process sharing nothing, as another program holding it locked
 * holds it, so every other open of it fails, a Git's and this process's own.
 */
export function unopenable(path: string): () => void {
  if (process.platform !== "win32") {
    chmodSync(path, 0o000);
    return () => chmodSync(path, 0o644);
  }
  // Node defines libuv's flag for an open that shares nothing only on Windows, and its types leave it out.
  const sharingNothing = (constants as unknown as Record<string, number | undefined>).UV_FS_O_EXLOCK ?? 0;
  const held = openSync(path, constants.O_RDONLY | sharingNothing);
  return () => closeSync(held);
}
