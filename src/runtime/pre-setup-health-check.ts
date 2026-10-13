import { readdirSync } from "node:fs";
import { defaultRerailDatabasePath, openRerailDatabase } from "../engine/storage/database.js";
import { repositoryIdentityLocation } from "../engine/catalog/repository-identity.js";
import { recordCheckout } from "../engine/catalog/repository-registry.js";
import { checkoutName, setStarted, type RepositorySetupIo } from "./repository-start.js";

/**
 * A Rerail release from before set-up (0.1.0-next.0) checks an update by running `rerail start --repo <folder>` in a
 * scratch folder, then expects the event stream to open a session there. Set-up would only print its first question
 * to that run, so the old release refused every update to this one.
 *
 * The run is recognised by everything that check does, and nothing a person or coding agent does: `start` with only
 * `--repo`, no terminal, `RERAIL_AUTOSTART=0` with its own `RERAIL_HOME` and `RERAIL_STORAGE_ROOT`, at an empty folder
 * outside Git. An empty folder has nothing to read and no rule pack fits it.
 */
export function isPreSetupHealthCheck(
  args: readonly string[],
  lane: RepositorySetupIo["lane"],
  workspace: string,
  env: NodeJS.ProcessEnv = process.env,
): boolean {
  if (args.length !== 2 || args[0] !== "--repo" || lane !== "agent") return false;
  if (env.RERAIL_AUTOSTART !== "0" || env.RERAIL_HOME === undefined || env.RERAIL_STORAGE_ROOT === undefined) return false;
  try {
    if (readdirSync(workspace).length > 0) return false;
  } catch {
    return false;
  }
  return !repositoryIdentityLocation(workspace).git;
}

/**
 * Start Rerail in the old health check's folder without setting it up: no rule packs, no coding agent connected and no
 * dashboard opened. Sessions are recorded there, as in any started folder that is not set up, and set-up still runs on
 * the next `rerail start`.
 */
export function startForPreSetupHealthCheck(workspace: string, io: RepositorySetupIo, databasePath = defaultRerailDatabasePath()): void {
  const database = openRerailDatabase(databasePath);
  try {
    setStarted(database, recordCheckout(database, workspace), true, false);
  } finally {
    database.$client.close();
  }
  io.say(`● Rerail is on in ${checkoutName(workspace, false)}, which is not set up yet.`);
}
