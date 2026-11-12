import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import Database from "better-sqlite3";
import { afterEach, describe, expect, it, vi } from "vitest";
import { RerailDaemon } from "../../src/daemon/index.js";
import { openRerailDatabase } from "../../src/engine/index.js";

const cleanups: Array<() => Promise<void> | void> = [];
afterEach(async () => {
  for (const cleanup of cleanups.splice(0).reverse()) await cleanup();
});

/**
 * How long a test waits for the move to end. The move's thread loads its modules when it starts, from TypeScript source
 * through `tsx` as the tests run it, so on a slow Windows machine whose temporary folder starts empty the move can end
 * tens of seconds after the daemon starts. Each test's own timeout leaves room for the daemon's start and close.
 */
const MOVE_MS = 45_000;
const TEST_MS = MOVE_MS + 20_000;

async function until(condition: () => boolean, timeoutMs = MOVE_MS): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  while (!condition()) {
    if (Date.now() > deadline) throw new Error("timed out");
    await new Promise(resolve => setTimeout(resolve, 200));
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

/**
 * A Rerail home whose database holds 20 audits as Rerail before the upgrade recorded them, with their provenance inline,
 * started before the file ran the step (`started_at` 1 to 20); the database's path.
 */
function databaseWithInlineAudits(): { home: string; databasePath: string } {
  const home = mkdtempSync(join(tmpdir(), "rerail-provenance-daemon-"));
  cleanups.push(() => rmSync(home, { recursive: true, force: true }));
  const databasePath = join(home, "rerail.sqlite");
  openRerailDatabase(databasePath).$client.close();
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
  return { home, databasePath };
}

/** Whether every audit has moved and the file holds no free page: the move ended with its `VACUUM`. */
function moved(databasePath: string): boolean {
  const { inline, freePages } = fileState(databasePath);
  return inline === 0 && freePages === 0;
}

describe("the daemon's move of audit rule provenance", () => {
  it("moves the provenance audits recorded before the upgrade hold inline once it starts, and gives the space back", async () => {
    const { home, databasePath } = databaseWithInlineAudits();

    const daemon = await RerailDaemon.start({ version: "test", databasePath, workspaceRoot: home, port: 0, remoteSink: false });
    cleanups.push(() => daemon.close());

    await until(() => moved(databasePath));
  }, TEST_MS);

  it("keeps its thread free while it moves, however long the move's VACUUM takes", async () => {
    const { home, databasePath } = databaseWithInlineAudits();
    // A `VACUUM` that holds the thread running it 3 s, as the one after 4,000 audits did on a loaded Windows machine.
    // Were the move on the daemon's thread, nothing else would run there meanwhile: a hook's health probe gives up
    // after 2 s, and the hook then runs without the daemon.
    const exec = Database.prototype.exec;
    const held = new Int32Array(new SharedArrayBuffer(4));
    const slowVacuum = vi.spyOn(Database.prototype, "exec").mockImplementation(function (this: Database.Database, source: string) {
      exec.call(this, source);
      if (source === "VACUUM") Atomics.wait(held, 0, 0, 3_000);
      return this;
    });
    cleanups.push(() => slowVacuum.mockRestore());

    const daemon = await RerailDaemon.start({ version: "test", databasePath, workspaceRoot: home, port: 0, remoteSink: false });
    cleanups.push(() => daemon.close());
    let longest = 0;
    let last = performance.now();
    const tick = (): void => {
      const now = performance.now();
      longest = Math.max(longest, now - last);
      last = now;
    };
    const ticks = setInterval(tick, 10);
    try {
      await until(() => moved(databasePath));
      // The gap that ends here, which no later tick would see.
      tick();
    } finally {
      clearInterval(ticks);
    }
    expect(longest).toBeLessThan(2_000);
  }, TEST_MS);
});
