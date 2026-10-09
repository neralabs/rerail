import { mkdir } from "node:fs/promises";
import { join } from "node:path";
import { RerailDaemon } from "../../src/daemon/index.js";
import { defaultRerailDatabasePath, defaultRerailHome } from "../../src/engine/index.js";

const daemons: RerailDaemon[] = [];

/** Exercise the real HTTP/native boundary while retaining deterministic failure injection in Node. */
export async function startNativeTestDaemon(cwd: string): Promise<RerailDaemon> {
  const home = defaultRerailHome();
  await mkdir(home, { recursive: true });
  const daemon = await RerailDaemon.start({
    version: "test",
    workspaceRoot: cwd,
    databasePath: defaultRerailDatabasePath(),
    stateFile: join(home, "daemon.json"),
    port: 0,
  });
  daemons.push(daemon);
  return daemon;
}

export async function closeNativeTestDaemons(): Promise<void> {
  await Promise.all(daemons.splice(0).map(daemon => daemon.close()));
}
