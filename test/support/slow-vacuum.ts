import { existsSync } from "node:fs";

/** A file URL: NODE_OPTIONS splits on spaces, and a URL spells each one `%20`. */
const PRELOAD = new URL("./slow-vacuum-preload.mjs", import.meta.url).href;

/**
 * Makes the first VACUUM run by a thread or process this one starts next hold the write lock `holdMs` before it runs,
 * and create `held` once it holds it (`slow-vacuum-preload.mjs`): what a VACUUM of a large file does to every other
 * writer, without the large file. A thread or process copies the environment when it starts, so call the returned
 * function right after starting it: it puts the environment back.
 */
export function slowVacuumNext(held: string, holdMs: number): () => void {
  const values: Record<string, string> = {
    NODE_OPTIONS: `${process.env.NODE_OPTIONS ?? ""} --import ${PRELOAD}`.trim(),
    RERAIL_TEST_VACUUM_HELD: held,
    RERAIL_TEST_VACUUM_HOLD_MS: String(holdMs),
  };
  const before = Object.fromEntries(Object.keys(values).map(name => [name, process.env[name]]));
  Object.assign(process.env, values);
  return () => {
    for (const [name, value] of Object.entries(before)) {
      if (value === undefined) delete process.env[name];
      else process.env[name] = value;
    }
  };
}

/** Resolves once `path` exists: a slowed VACUUM holds the write lock from then on. */
export async function untilExists(path: string, timeoutMs: number): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  while (!existsSync(path)) {
    if (Date.now() > deadline) throw new Error(`${path} did not appear within ${timeoutMs} ms`);
    await new Promise(resolve => setTimeout(resolve, 50));
  }
}
