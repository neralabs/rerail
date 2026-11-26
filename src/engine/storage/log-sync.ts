import { open, type FileHandle } from "node:fs/promises";

/**
 * Puts on the disk every commit the write-ahead log of the database at `path` holds: the log file is opened, synced
 * and closed through Node's file system calls, which run on libuv's thread pool, so the caller's thread never waits
 * for the disk and no SQLite lock is taken. The store's writers commit without waiting for the disk
 * (`synchronous = NORMAL`, `prepareWriter`), so a commit sits in the operating system's cache until a checkpoint, the
 * system's own writeback or this sync writes it; once this resolves, a power cut or an operating system crash keeps
 * every commit made before it began. It changes when the log's pages reach the disk, never what they hold, so SQLite's
 * recovery reads the log as it would after any writeback. With no log file there is nothing to sync: the last
 * connection to close checkpoints the log into the database file, syncing both, and removes it.
 */
export async function syncHistoryLog(path: string): Promise<void> {
  let log: FileHandle;
  try {
    // Opened for writing, since Windows syncs only a file its handle may write; never created.
    log = await open(`${path}-wal`, "r+");
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "ENOENT") return;
    throw error;
  }
  try {
    await log.sync();
  } finally {
    await log.close();
  }
}
