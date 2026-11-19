import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { loadNativeBinding, LOCKS_DIR } from "../../../src/engine/index.js";

/**
 * Makes this process the daemon of `home` by the OS's account, as a daemon's entry does before it starts: it holds the
 * home's lock and records its pid beside it. A lookup keeps the record of a daemon that does not answer only while its
 * pid holds the home. Returns the release; a home this process holds already stays held, and that release does nothing.
 */
export function holdHomeAsDaemon(home: string): () => void {
  mkdirSync(join(home, LOCKS_DIR), { recursive: true });
  const lock = loadNativeBinding().tryLockExclusive(join(home, LOCKS_DIR, "daemon.lock"));
  if (lock === null) return () => undefined;
  writeFileSync(join(home, LOCKS_DIR, "daemon.json"), JSON.stringify({ pid: process.pid }));
  return () => lock.release();
}
