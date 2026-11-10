import { rm } from "node:fs/promises";

/**
 * Removes the folder `path` and everything in it, once nothing holds it. Windows refuses to remove a file or folder a
 * process still has open (`EBUSY`, `EPERM`), and a SQLite connection, a file watcher or an exited child releases its
 * handle a moment after it reports itself closed. So the removal is retried, as `rm` does with `maxRetries`, waiting
 * 100 ms longer before each try, about 5 s in all.
 */
export async function removeFolder(path: string): Promise<void> {
  await rm(path, { recursive: true, force: true, maxRetries: 10, retryDelay: 100 });
}
