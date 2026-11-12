import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import Database from "better-sqlite3";
import { afterEach, describe, expect, it } from "vitest";
import { RerailDaemon } from "../../src/daemon/index.js";
import { openRerailDatabase } from "../../src/engine/index.js";

const cleanups: Array<() => Promise<void> | void> = [];
afterEach(async () => {
  for (const cleanup of cleanups.splice(0).reverse()) await cleanup();
});

async function until(condition: () => boolean, timeoutMs = 15_000): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  while (!condition()) {
    if (Date.now() > deadline) throw new Error("timed out");
    await new Promise(resolve => setTimeout(resolve, 50));
  }
}

/** How many audits hold their rule provenance inline, and how many of the file's pages are free. */
function fileState(path: string): { inline: number; freePages: number } {
  const database = new Database(path, { readonly: true });
  try {
    const inline = database.prepare("SELECT count(*) FROM audits WHERE rule_provenance_json <> '[]'").pluck().get() as number;
    return { inline, freePages: database.pragma("freelist_count", { simple: true }) as number };
  } finally {
    database.close();
  }
}

describe("the daemon's move of audit rule provenance", () => {
  it("moves the provenance audits recorded before the upgrade hold inline once it starts, and gives the space back", async () => {
    const home = mkdtempSync(join(tmpdir(), "rerail-provenance-daemon-"));
    cleanups.push(() => rmSync(home, { recursive: true, force: true }));
    const databasePath = join(home, "rerail.sqlite");
    openRerailDatabase(databasePath).$client.close();
    // Audits as Rerail before the upgrade recorded them, with their provenance inline, started before the file ran the
    // step (`started_at` 1 to 20).
    const provenance = JSON.stringify(Array.from({ length: 300 }, (_, index) => ({
      ruleId: `typescript/rule-${index}`, revision: 1, payloadHash: `sha256:${index}`, applicability: "global", origin: "user",
    })));
    const older = new Database(databasePath);
    try {
      const insert = older.prepare(`INSERT INTO audits (audit_id, session_id, workspace_root, scope, mode, revision, catalog_revision,
        rule_revisions_json, rule_provenance_json, paths_json, started_at, finished_at, status, violations_json, failures_json)
        VALUES (?, 'session-a', '/home/dev/app', 'touched-files', 'steer', 1, 'catalog', '[]', ?, '[]', ?, ?, 'completed', '[]', '[]')`);
      for (let index = 1; index <= 20; index += 1) insert.run(`older-${index}`, provenance, index, index + 1);
    } finally {
      older.close();
    }
    expect(fileState(databasePath).inline).toBe(20);

    const daemon = await RerailDaemon.start({ version: "test", databasePath, workspaceRoot: home, port: 0, remoteSink: false });
    cleanups.push(() => daemon.close());

    await until(() => {
      const { inline, freePages } = fileState(databasePath);
      return inline === 0 && freePages === 0;
    });
  }, 30_000);
});
