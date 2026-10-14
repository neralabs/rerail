import { mkdir } from "node:fs/promises";
import { join } from "node:path";
import { RerailDaemon } from "../../src/daemon/index.js";
import { defaultRerailDatabasePath, defaultRerailHome } from "../../src/engine/index.js";

const daemons: RerailDaemon[] = [];

/** Exercise the real HTTP/native boundary while retaining deterministic failure injection in Node. A restart passes the port the daemon it replaces held. */
export async function startNativeTestDaemon(cwd: string, port = 0): Promise<RerailDaemon> {
  const home = defaultRerailHome();
  await mkdir(home, { recursive: true });
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
}
