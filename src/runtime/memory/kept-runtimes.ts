import { stat } from "node:fs/promises";

/**
 * How long a runtime no session uses stays open while the daemon is short of room: the 15 minutes every such runtime
 * stayed open whatever the memory, so that under pressure a /clear, or a session back after a short break, finds its
 * checkout's rules compiled as it did, and no runtime is held longer than it was. With room a runtime stays open.
 */
export const SHORT_OF_ROOM_KEEP_MS = 15 * 60_000;

/** What the kept runtimes ask of the engine service that holds the runtimes. */
export interface KeptRuntimesOptions {
  /** Whether the daemon needs the room the kept runtimes take (`memoryRoom` in memory-room.ts). */
  readonly needsRoom: () => boolean;
  /** Whether some session uses the runtime at a key. */
  readonly inUse: (key: string) => boolean;
  /** The checkout folder of the open runtime at a key; undefined once it has closed. */
  readonly folderOf: (key: string) => string | undefined;
  /** Closes the open runtime at a key, settling once it has closed; it collects nothing. */
  readonly close: (key: string) => Promise<void>;
  /** Runs a full collection, so the engine frees the compiled rules closed runtimes held: one after each batch of closes. */
  readonly collect: () => void;
  /** The clock a runtime's last leaving is noted by. */
  readonly now: () => number;
  /** Whether a folder was removed; {@link folderRemoved} by default. */
  readonly removed?: (folder: string) => Promise<boolean>;
}

/**
 * The runtimes no session uses, which the engine service keeps open so that their checkout's next session finds its
 * rules compiled and its checkout walked, each with the time its last session left it; and when each closes. A runtime
 * is kept only while no session uses it and no event's work runs in it: every event claims its runtime as it arrives
 * (`claim`) and releases it once its work has ended, and the release keeps it again, as left then, only once no session
 * uses it. So nothing here closes a runtime a session uses or an event runs in, and closing checks that once more.
 *
 * A session's leaving and a runtime's opening close nothing, so no close lands on a session's path. Room is made at the
 * minute's sweep (`sweep`): while the daemon is short of room, each runtime kept for {@link SHORT_OF_ROOM_KEEP_MS} or
 * longer closes; and before the heap watch restarts the daemon, every kept runtime closes (`closeAll`). Each batch of
 * closes ends with one full collection.
 */
export class KeptRuntimes {
  readonly #options: KeptRuntimesOptions;
  /** When each kept runtime's last session left it, by key. */
  readonly #kept = new Map<string, number>();
  /** How many events or opens are on their way to each runtime or running in it. */
  readonly #claims = new Map<string, number>();
  /** The look for removed checkout folders under way (`#lookForRemoved`), one at a time. */
  #looking: Promise<void> | undefined;
  #stopped = false;

  constructor(options: KeptRuntimesOptions) {
    this.#options = options;
  }

  /**
   * An event's work is on its way to the runtime at `key`: it is not kept, and nothing closes it, until the returned
   * release, which comes once that work has ended. The release keeps it, as left then, when no session uses it (`left`).
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
   * The runtime at `key` may have lost its last session: it is kept, as left now, once no session uses it and no
   * event's work runs in it.
   */
  left(key: string): void {
    if (this.#closable(key)) this.#kept.set(key, this.#options.now());
  }

  /** The runtime at `key` has closed. */
  forget(key: string): void {
    this.#kept.delete(key);
  }

  /**
   * The minute's sweep: while the daemon needs the room, each runtime kept for {@link SHORT_OF_ROOM_KEEP_MS} or longer
   * closes, the one left last too; and each kept runtime whose checkout folder was removed closes (`#lookForRemoved`).
   * The daemon's memory is read only while some runtime has been kept that long.
   */
  sweep(): void {
    const due = this.#keptSince(this.#options.now() - SHORT_OF_ROOM_KEEP_MS);
    if (due.length > 0 && this.#options.needsRoom()) void this.#close(due);
    this.#lookForRemoved();
  }

  /** Closes every kept runtime, however recently left, and settles once each has closed and been collected. */
  async closeAll(): Promise<void> {
    await this.#close([...this.#kept.keys()]);
  }

  /** The engine service is closing its runtimes itself: nothing is kept, and a look still under way closes nothing. */
  stop(): void {
    this.#stopped = true;
    this.#kept.clear();
  }

  /** The keys of the runtimes kept since `at` or earlier. */
  #keptSince(at: number): string[] {
    return [...this.#kept].filter(([, leftAt]) => leftAt <= at).map(([key]) => key);
  }

  #closable(key: string): boolean {
    if (this.#stopped || this.#options.folderOf(key) === undefined) return false;
    return !this.#claims.has(key) && !this.#options.inUse(key);
  }

  /** Closes each of `keys` still kept and closable, then collects once for them all. */
  async #close(keys: readonly string[]): Promise<void> {
    const closing = keys.filter(key => this.#kept.delete(key) && this.#closable(key)).map(key => this.#options.close(key));
    if (closing.length === 0) return;
    await Promise.allSettled(closing);
    this.#options.collect();
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
    for (const key of [...this.#kept.keys()]) {
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
