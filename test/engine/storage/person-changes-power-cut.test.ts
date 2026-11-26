// A change a person makes to rule configuration or a setting is on the disk before Rerail says it is made, so a power
// cut right after keeps it: the store's commits do not wait for the disk, and the repairs at the next start record a
// loss but bring nothing back. One case per kind of writer: the `rerail` command, which commits with FULL in a process
// of its own, and the daemon's API, which answers once the log is synced off its thread. Each makes the change, hears
// it succeed, then cuts the power as `power-cut.ts` does (everything after the last checkpoint and the last sync the
// writer's process made goes) and opens the database the next start opens. A sync that fails after the change
// committed is answered as a change made and not confirmed on the disk, over HTTP and MCP alike.
import { spawnSync } from "node:child_process";
import { mkdtempSync, realpathSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { pathToFileURL } from "node:url";
import Database from "better-sqlite3";
import { afterEach, describe, expect, it, vi } from "vitest";
import { z } from "zod";
import { createRerailApi, createRerailMcpHandler, DatabaseApiStores, operations } from "../../../src/api/index.js";
import { openRerailDatabase, PACKAGE_ROOT, recordCheckout } from "../../../src/engine/index.js";
import { setRepositoryStarted } from "../../../src/runtime/index.js";
import { cutPowerInCopy } from "../../support/engine/power-cut.js";
import { recordLogSyncs } from "../../support/engine/log-syncs.js";
import { removeFolderSync } from "../../support/platform/remove-folder.js";
import { drivenWatch } from "../../support/platform/driven-watch.js";

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

describe("a person's change the disk did not confirm", () => {
  // A write a coding agent may make over MCP: the API's own preference write is a person's alone.
  const agentWrite: Parameters<typeof createRerailMcpHandler>[1][number] = {
    id: "presence_set", method: "PUT", path: "/api/presence", summary: "Sets the default presence.",
    body: z.object({ presence: z.enum(["compact", "silent"]) }), output: z.object({}),
    handler: (stores, input) => {
      stores.preferences().upsert({ repository: "", host: "", presence: input.presence as "compact" | "silent" });
      return {};
    },
  };
  const said = "the change was made, but Rerail could not confirm it is on the disk: a power cut may still undo it, and trying it again may make it twice (EIO: i/o error, fsync)";

  it("is answered as made and not confirmed, over HTTP and MCP, and the change stream hears it", async () => {
    const databasePath = join(folder("rerail-person-unsynced-"), "rerail.sqlite");
    const stores = new DatabaseApiStores(databasePath, {
      health: () => ({ pid: process.pid, database: { path: databasePath, exists: true }, native: { ok: true } }),
      origin: () => "http://127.0.0.1:4737",
      // No folder notice and no beat: what the stream hears is the stores' own notice of a write.
      changeFeed: { beatMs: 3_600_000, watch: drivenWatch().watch },
    });
    try {
      const notices: string[] = [];
      stores.changes().listen(notice => notices.push(notice));
      // The log's sync fails once the change committed: EIO, or on Windows another program holding the file.
      vi.spyOn(stores, "syncToDisk").mockRejectedValue(new Error("EIO: i/o error, fsync"));
      const presence = (): unknown[] => {
        const database = new Database(databasePath, { readonly: true });
        try {
          return database.prepare("SELECT presence FROM presentation_preferences").pluck().all();
        } finally {
          database.close();
        }
      };
      const answer = await createRerailApi(stores, [agentWrite], { version: "0.0.0-test" }).request("/api/presence", {
        method: "PUT", headers: { "content-type": "application/json" }, body: JSON.stringify({ presence: "compact" }),
      });
      expect(answer.status).toBe(503);
      expect(await answer.json()).toEqual({ error: "not-on-disk", message: said });
      expect(presence()).toEqual(["compact"]);
      await vi.waitFor(() => expect(notices).toEqual(["data"]));
      const mcp = createRerailMcpHandler(stores, [agentWrite], { version: "0.0.0-test" });
      const call = await mcp(new Request("http://localhost:4737/mcp", {
        method: "POST",
        headers: { host: "localhost:4737", "content-type": "application/json", accept: "application/json, text/event-stream" },
        body: JSON.stringify({ jsonrpc: "2.0", id: 1, method: "tools/call", params: { name: "presence_set", arguments: { presence: "silent" } } }),
      }));
      const { result } = await call.json() as { result: { isError?: boolean; content: { text: string }[] } };
      expect(result).toEqual({ isError: true, content: [{ type: "text", text: `not-on-disk: ${said}` }] });
      expect(presence()).toEqual(["silent"]);
      await vi.waitFor(() => expect(notices).toEqual(["data", "data"]));
    } finally {
      stores.close();
    }
  });
});
