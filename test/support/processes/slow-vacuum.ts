import { existsSync } from "node:fs";

/** A file URL: NODE_OPTIONS splits on spaces, and a URL spells each one `%20`. */
const PRELOAD = new URL("./slow-vacuum-preload.mjs", import.meta.url).href;

/** What `slowVacuumNext` makes the VACUUMs of the threads and processes started next do (`slow-vacuum-preload.mjs`). */
export interface SlowVacuum {
  /** The first `holds` (default 1) VACUUMs hold the write lock `holdMs` before they run, each creating `held` (`.n`). */
  readonly held?: string;
  readonly holdMs?: number;
  readonly holds?: number;
  /** A VACUUM that has committed creates `committed`, waits `lingerMs` before its checkpoint, then creates `.done`. */
  readonly committed?: string;
  readonly lingerMs?: number;
}

/**
 * Slows the VACUUMs run by the threads and processes this one starts next as `slow` says: what a VACUUM of a large file
 * does to every other writer, without the large file. A thread or process copies the environment when it starts, so
 * call the returned function once the last one that needs it has started: it puts the environment back.
 */
export function slowVacuumNext(slow: SlowVacuum): () => void {
  const values: Record<string, string | undefined> = {
    NODE_OPTIONS: `${process.env.NODE_OPTIONS ?? ""} --import ${PRELOAD}`.trim(),
    RERAIL_TEST_VACUUM_HELD: slow.held,
    RERAIL_TEST_VACUUM_HOLD_MS: slow.holdMs === undefined ? undefined : String(slow.holdMs),
    RERAIL_TEST_VACUUM_HOLDS: slow.holds === undefined ? undefined : String(slow.holds),
    RERAIL_TEST_VACUUM_COMMITTED: slow.committed,
    RERAIL_TEST_VACUUM_LINGER_MS: slow.lingerMs === undefined ? undefined : String(slow.lingerMs),
  };
  const before = Object.fromEntries(Object.keys(values).map(name => [name, process.env[name]]));
  const put = (entries: Record<string, string | undefined>): void => {
    for (const [name, value] of Object.entries(entries)) {
      if (value === undefined) delete process.env[name];
      else process.env[name] = value;
    }
  };
  put(values);
  return () => put(before);
}

/** Resolves once `path` exists: a slowed VACUUM holds the write lock, or has committed, from then on. */
export async function untilExists(path: string, timeoutMs: number): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  while (!existsSync(path)) {
    if (Date.now() > deadline) throw new Error(`${path} did not appear within ${timeoutMs} ms`);
    await new Promise(resolve => setTimeout(resolve, 50));
  }
}
