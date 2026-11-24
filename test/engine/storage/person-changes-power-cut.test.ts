// A change a person makes to rule configuration or a setting is on the disk before Rerail says it is made, so a power
// cut right after keeps it: the store's commits do not wait for the disk, and the repairs at the next start record a
// loss but bring nothing back. One case per kind of writer: the `rerail` command, which commits with FULL in a process
// of its own, and the daemon's API, which answers once the log is synced off its thread. Each makes the change, hears
// it succeed, then cuts the power as `power-cut.ts` does (everything after the last checkpoint and the last sync the
// writer's process made goes) and opens the database the next start opens.
import { spawnSync } from "node:child_process";
import { mkdtempSync, realpathSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { pathToFileURL } from "node:url";
import Database from "better-sqlite3";
import { afterEach, describe, expect, it } from "vitest";
import { createRerailApi, DatabaseApiStores, operations } from "../../../src/api/index.js";
import { openRerailDatabase, PACKAGE_ROOT, recordCheckout } from "../../../src/engine/index.js";
import { setRepositoryStarted } from "../../../src/runtime/index.js";
import { cutPowerInCopy } from "../../support/engine/power-cut.js";
import { recordLogSyncs } from "../../support/engine/log-syncs.js";
import { removeFolderSync } from "../../support/platform/remove-folder.js";

const folders: string[] = [];
afterEach(() => {
  for (const folder of folders.splice(0)) removeFolderSync(folder);
});

function folder(prefix: string): string {
  const made = realpathSync(mkdtempSync(join(tmpdir(), prefix)));
  folders.push(made);
  return made;
}

/** What `read` finds in the database the next start opens after a power cut now; the copy is removed after. */
function afterPowerCut<T>(databasePath: string, read: (database: Database.Database) => T, syncRecord?: string): T {
  const copy = cutPowerInCopy(databasePath, syncRecord);
  folders.push(dirname(copy));
  const database = new Database(copy);
  try {
    return read(database);
  } finally {
    database.close();
  }
}

describe("a person's change through a power cut right after Rerail said it was made", () => {
  it("keeps a checkout the rerail command stopped", () => {
    const home = folder("rerail-person-cli-");
    const checkout = folder("rerail-person-checkout-");
    expect(spawnSync("git", ["init", "--quiet", checkout]).status).toBe(0);
    const databasePath = join(home, "rerail.sqlite");
    // Rerail started in the checkout, as set-up leaves it. This connection stays open, as a running daemon's does, so
    // the command's own close leaves the log for the power cut instead of checkpointing it into the file.
    const database = openRerailDatabase(databasePath);
    try {
      setRepositoryStarted(database, recordCheckout(database, checkout).repositoryKey, true);
      const syncRecord = join(home, "log-syncs.jsonl");
      const stop = spawnSync(process.execPath, [
        // A URL: on Windows Node takes an absolute path for a drive letter's scheme.
        "--import", "tsx", "--import", pathToFileURL(join(PACKAGE_ROOT, "test/support/processes/record-log-syncs.ts")).href,
        join(PACKAGE_ROOT, "src/cli/cli.ts"), "--rerail-managed-child", "stop", "--repo", checkout, "--database", databasePath,
      ], { cwd: PACKAGE_ROOT, encoding: "utf8", env: { ...process.env, RERAIL_HOME: home, RERAIL_AUTOSTART: "0", RERAIL_TEST_LOG_SYNCS: syncRecord } });
      expect(stop.stderr).toBe("");
      expect(stop.stdout).toContain("Rerail is stopped in");
      expect(stop.status).toBe(0);
      const started = afterPowerCut(databasePath, kept => kept.prepare("SELECT started FROM checkouts").pluck().all(), syncRecord);
      expect(started).toEqual([0]);
    } finally {
      database.$client.close();
    }
  }, 60_000);

  it("keeps a setting the dashboard's API changed", async () => {
    recordLogSyncs();
    const databasePath = join(folder("rerail-person-api-"), "rerail.sqlite");
    const stores = new DatabaseApiStores(databasePath, {
      health: () => ({ pid: process.pid, database: { path: databasePath, exists: true }, native: { ok: true } }),
      origin: () => "http://127.0.0.1:4737",
    });
    try {
      const app = createRerailApi(stores, operations, { version: "0.0.0-test" });
      const set = async (presence: string): Promise<number> => (await app.request("/api/presentation-preferences", {
        method: "PUT", headers: { "content-type": "application/json" }, body: JSON.stringify({ repository: "", host: "", presence }),
      })).status;
      expect(await set("compact")).toBe(200);
      expect(await set("silent")).toBe(200);
      const presence = afterPowerCut(databasePath, kept => kept.prepare("SELECT presence FROM presentation_preferences").pluck().all());
      expect(presence).toEqual(["silent"]);
    } finally {
      stores.close();
    }
  });
});
