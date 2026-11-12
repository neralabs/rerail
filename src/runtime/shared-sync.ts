import { open, type FileHandle } from "node:fs/promises";

/**
 * A disk sync that writers share. A sync already running may have begun before a caller's write, so the caller waits
 * for the next one, and every caller that asks while a sync runs shares that next one: one sync runs and one waits,
 * however many writers ask.
 */
export class SharedSync {
  readonly #sync: () => Promise<void>;
  #running: Promise<void> | undefined;
  /** Begins once `#running` ends; shared by every caller since `#running` began. */
  #next: Promise<void> | undefined;

  constructor(sync: () => Promise<void>) {
    this.#sync = sync;
  }

  /** Settles once a sync that began after this call has ended, and rejects when that sync failed. */
  sync(): Promise<void> {
    if (this.#next !== undefined) return this.#next;
    const running = this.#running;
    if (running === undefined) return this.#begin();
    const next = running.then(() => undefined, () => undefined).then(() => {
      this.#next = undefined;
      return this.#begin();
    });
    this.#next = next;
    return next;
  }

  #begin(): Promise<void> {
    const running = this.#sync().finally(() => {
      if (this.#running === running) this.#running = undefined;
    });
    this.#running = running;
    return running;
  }
}

/**
 * Syncs the file at `path` to disk from Node's thread pool, through a handle opened for this sync alone. The sync
 * (fsync on Linux and macOS, FlushFileBuffers on Windows) writes back every change to the file, whichever handle made
 * it.
 */
export async function syncFile(path: string): Promise<void> {
  // Read and write: Windows flushes a file only through a handle that may write it.
  const handle: FileHandle = await open(path, "r+");
  try {
    await handle.sync();
  } finally {
    await handle.close();
  }
}
