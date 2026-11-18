import { stat } from "node:fs/promises";

/** What the kept runtimes ask of the engine service that holds the runtimes. */
export interface KeptRuntimesOptions {
  /** Whether the daemon needs the room the kept runtimes take (`memoryRoom` in memory-room.ts). */
  readonly needsRoom: () => boolean;
  /** Whether some session uses the runtime at a key. */
  readonly inUse: (key: string) => boolean;
  /** The checkout folder of the open runtime at a key; undefined once it has closed. */
  readonly folderOf: (key: string) => string | undefined;
  /** Closes the open runtime at a key, settling once it has closed. */
  readonly close: (key: string) => Promise<void>;
  /** Whether a folder was removed; {@link folderRemoved} by default. */
  readonly removed?: (folder: string) => Promise<boolean>;
}

/**
 * The runtimes no session uses, which the engine service keeps open so that their checkout's next session finds its
 * rules compiled and its checkout walked, in the order their last session left them; and when each closes. A runtime
 * is kept only while no session uses it and no use of it is on its way: every event and open for it claims it as it
 * arrives (`claim`), which takes it out, and its release keeps it again only once no session uses it. So nothing here
 * closes a runtime a session uses or is about to use, and closing checks that once more. A session's leaving closes
 * nothing, since it frees nothing a new allocation needs: room is made where memory is about to be taken, as a runtime
 * opens (`beforeOpen`), at the minute's sweep (`sweep`), and before the heap watch restarts the daemon (`closeAll`). At the sweep the one left most recently stays, so a
 * /clear or a new session in the same checkout a moment later finds it, until another runtime opens.
 */
export class KeptRuntimes {
  readonly #options: KeptRuntimesOptions;
  /** The kept runtimes' keys, the one whose last session left longest ago first. */
  readonly #kept = new Set<string>();
  /** How many uses of each runtime are on their way: an event or an open queued for it. */
  readonly #claims = new Map<string, number>();
  /** The look for removed checkout folders under way (`#lookForRemoved`), one at a time. */
  #looking: Promise<void> | undefined;
  #stopped = false;

  constructor(options: KeptRuntimesOptions) {
    this.#options = options;
  }

  /**
   * A use of the runtime at `key` is on its way: it is not kept, and nothing closes it, until the returned release. The
   * release keeps it, as the one left most recently, when no session uses it by then (`left`).
   */
  claim(key: string): () => void {
    this.#kept.delete(key);
    this.#claims.set(key, (this.#claims.get(key) ?? 0) + 1);
    let released = false;
    return () => {
      if (released) return;
      released = true;
      const remaining = (this.#claims.get(key) ?? 1) - 1;
      if (remaining > 0) this.#claims.set(key, remaining);
      else this.#claims.delete(key);
      this.left(key);
    };
  }

  /**
   * The runtime at `key` may have lost its last session: it is kept, as the one left most recently, once no session
   * uses it and no use is on its way.
   */
  left(key: string): void {
    if (!this.#closable(key)) return;
    this.#kept.delete(key);
    this.#kept.add(key);
  }

  /** The runtime at `key` has closed. */
  forget(key: string): void {
    this.#kept.delete(key);
  }

  /**
   * A runtime is about to open and take memory: while the daemon needs the room, every kept runtime closes, the one left
   * most recently too.
   */
  beforeOpen(): void {
    if (this.#kept.size > 0 && this.#options.needsRoom()) void this.#close([...this.#kept]);
  }

  /**
   * The minute's sweep: while the daemon needs the room, every kept runtime but the one left most recently closes; and
   * each kept runtime whose checkout folder was removed closes (`#lookForRemoved`). The daemon's memory is read only
   * while two or more are kept.
   */
  sweep(): void {
    if (this.#kept.size > 1 && this.#options.needsRoom()) void this.#close([...this.#kept].slice(0, -1));
    this.#lookForRemoved();
  }

  /** Closes every kept runtime, the one left most recently too, and settles once each has closed. */
  async closeAll(): Promise<void> {
    await this.#close([...this.#kept]);
  }

  /** The engine service is closing its runtimes itself: nothing is kept, and a look still under way closes nothing. */
  stop(): void {
    this.#stopped = true;
    this.#kept.clear();
  }

  #closable(key: string): boolean {
    if (this.#stopped || this.#options.folderOf(key) === undefined) return false;
    return !this.#claims.has(key) && !this.#options.inUse(key);
  }

  async #close(keys: readonly string[]): Promise<void> {
    const closing: Array<Promise<void>> = [];
    for (const key of keys) {
      if (!this.#kept.delete(key) || !this.#closable(key)) continue;
      closing.push(this.#options.close(key));
    }
    await Promise.all(closing);
  }

  /**
   * Looks whether each kept runtime's checkout folder was removed, off the event loop and one folder at a time, so a
   * share that does not answer holds one of libuv's threads and no hook; while a look runs, the next sweep starts none.
   */
  #lookForRemoved(): void {
    if (this.#looking !== undefined || this.#kept.size === 0) return;
    this.#looking = this.#closeRemoved().finally(() => {
      this.#looking = undefined;
    });
  }

  async #closeRemoved(): Promise<void> {
    const removed = this.#options.removed ?? folderRemoved;
    for (const key of [...this.#kept]) {
      const folder = this.#options.folderOf(key);
      if (folder !== undefined && await removed(folder)) await this.#close([key]);
    }
  }
}

/**
 * Whether `folder` was removed: its stat finds no such file or folder. Any other failure, such as a refusal to read it
 * or a share that errs, says nothing about removal, so the folder counts as still there.
 */
export async function folderRemoved(folder: string): Promise<boolean> {
  try {
    await stat(folder);
    return false;
  } catch (error) {
    const code = (error as NodeJS.ErrnoException).code;
    return code === "ENOENT" || code === "ENOTDIR";
  }
}
