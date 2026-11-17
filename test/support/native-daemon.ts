import { mkdirSync, writeFileSync } from "node:fs";
import { mkdir } from "node:fs/promises";
import { join } from "node:path";
import { RerailDaemon } from "../../src/daemon/index.js";
import { defaultRerailDatabasePath, defaultRerailHome, loadNativeBinding, LOCKS_DIR } from "../../src/engine/index.js";

const daemons: RerailDaemon[] = [];
const homeLocks: Array<{ release(): void }> = [];

/**
 * Makes this process the daemon of `home` by the OS's account, as a daemon's entry does before it starts: it holds the
 * home's lock and records its pid beside it. A lookup keeps the record of a daemon that does not answer only while its
 * pid holds the home. A home this process holds already stays held.
 */
function holdHome(home: string): void {
  mkdirSync(join(home, LOCKS_DIR), { recursive: true });
  const lock = loadNativeBinding().tryLockExclusive(join(home, LOCKS_DIR, "daemon.lock"));
  if (lock === null) return;
  homeLocks.push(lock);
  writeFileSync(join(home, LOCKS_DIR, "daemon.json"), JSON.stringify({ pid: process.pid }));
}

/** Exercise the real HTTP/native boundary while retaining deterministic failure injection in Node. A restart passes the port the daemon it replaces held. */
export async function startNativeTestDaemon(cwd: string, port = 0): Promise<RerailDaemon> {
  const home = defaultRerailHome();
  await mkdir(home, { recursive: true });
  holdHome(home);
  const daemon = await RerailDaemon.start({
    version: "test",
    workspaceRoot: cwd,
    databasePath: defaultRerailDatabasePath(),
    stateFile: join(home, "daemon.json"),
    port,
  });
  daemons.push(daemon);
  return daemon;
}

export async function closeNativeTestDaemons(): Promise<void> {
  await Promise.all(daemons.splice(0).map(daemon => daemon.close()));
  for (const lock of homeLocks.splice(0)) lock.release();
}
