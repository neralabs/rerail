import { stat } from "node:fs/promises";

/**
 * How long a runtime no session uses stays open before anything but the heap watch's relief or the daemon's stop may
 * close it: the 15 minutes every such runtime stayed open whatever the memory or its folder, so that a /clear, or a
 * session back after a short break, finds its checkout's rules compiled as it did. Past it, the runtime closes while the
 * daemon is short of room, so none is held longer than it was then, and with room once its checkout folder was removed:
 * a folder gone for a moment, moved and moved back or on a share that dropped, costs nobody the rules kept for it within
 * the time they always were.
 */
export const UNUSED_KEEP_MS = 15 * 60_000;

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
 * is kept only while no session uses it and no event's work runs in it (`#closable`): every event claims its runtime as
 * it arrives (`claim`) and releases it once its work has ended, and the release keeps it again, as left then, only once
 * no session uses it. So nothing here closes a runtime a session uses or an event runs in, and closing checks that once
 * more.
 *
 * A session's leaving and a runtime's opening close nothing, so no close lands on a session's path. Room is made at the
 * minute's sweep (`sweep`), which closes no runtime sooner than {@link UNUSED_KEEP_MS} after its last session left, as
 * none closed sooner: each runtime kept that long closes while the daemon is short of room, and with room once its
 * checkout folder was removed. Before the heap watch restarts the daemon, every kept runtime closes (`closeAll`). Each
 * batch of closes ends with one full collection.
 */
export class KeptRuntimes {
  readonly #options: KeptRuntimesOptions;
  /**
   * When each runtime was last left, by key: it is kept while it is closable, and a session that takes it back leaves its
   * time here until it leaves again (`left`).
   */
  readonly #kept = new Map<string, number>();
  /** How many events or opens are on their way to each runtime or running in it. */
  readonly #claims = new Map<string, number>();
  /** Set while a look for removed checkout folders runs (`#lookForRemoved`), one at a time. */
  #looking = false;
  #stopped = false;

  constructor(options: KeptRuntimesOptions) {
    this.#options = options;
  }

  /**
   * An event's work is on its way to the runtime at `key`: it is not kept, and nothing closes it, until the returned
   * release, which comes once that work has ended. The release keeps it, as left then, when no session uses it (`left`).
   */
  claim(key: string): () => void {
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

  /**
   * The minute's sweep: each runtime kept for {@link UNUSED_KEEP_MS} or longer closes, the one left last too,
   * while the daemon needs the room, and with room once its checkout folder was removed (`#lookForRemoved`). The
   * daemon's memory is read, and folders are looked at, only while some runtime has been kept that long. Settles once
   * what it closed has been collected.
   */
  async sweep(): Promise<void> {
    const due = this.#keptSince(this.#options.now() - UNUSED_KEEP_MS);
    if (due.length === 0) return;
    if (this.#options.needsRoom()) await this.#close(due);
    else await this.#lookForRemoved(due);
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

  /** The keys of the runtimes kept since `at` or earlier: left then, and closable now. */
  #keptSince(at: number): string[] {
    return [...this.#kept].filter(([key, leftAt]) => leftAt <= at && this.#closable(key)).map(([key]) => key);
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
   * Looks whether the checkout folder of each of `keys` was removed, and closes the runtimes of those that were: off the
   * event loop and one folder at a time, so a share that does not answer holds one of libuv's threads and no hook; while
   * a look runs, the next sweep starts none.
   */
  async #lookForRemoved(keys: readonly string[]): Promise<void> {
    if (this.#looking) return;
    this.#looking = true;
    try {
      await this.#closeRemoved(keys);
    } finally {
      this.#looking = false;
    }
  }

  async #closeRemoved(keys: readonly string[]): Promise<void> {
    const removed = this.#options.removed ?? folderRemoved;
    for (const key of keys) {
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
