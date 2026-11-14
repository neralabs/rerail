import { mkdtempSync, statSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import Database from "better-sqlite3";
import { afterEach, describe, expect, it, vi } from "vitest";
import { RerailDaemon } from "../../src/daemon/index.js";
import { openRerailDatabase } from "../../src/engine/index.js";
import { removeFolder } from "../support/platform/remove-folder.js";

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

async function until(condition: () => boolean, everyMs = 200): Promise<void> {
  const deadline = Date.now() + MOVE_MS;
  while (!condition()) {
    if (Date.now() > deadline) throw new Error("timed out");
    await new Promise(resolve => setTimeout(resolve, everyMs));
  }
}

/** Spies on this thread's stderr, which is the daemon log: each write so far, read at any time. */
function spyOnStderr(): { writes: () => string[] } {
  const stderr = vi.spyOn(process.stderr, "write").mockReturnValue(true);
  cleanups.push(() => stderr.mockRestore());
  return { writes: () => stderr.mock.calls.map(([chunk]) => String(chunk)) };
}

/** What the daemon wrote to its log about its move of audit rule provenance. */
function moveLines(writes: readonly string[]): string[] {
  return writes.filter(write => write.startsWith("rerail daemon: moving audit rule provenance"));
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
 * A Rerail home whose database holds `audits` audits as Rerail before the upgrade recorded them, each with the same
 * provenance of `rules` rules inline, started before the file ran the step (`started_at` 1 up); the database's path and
 * that provenance's text.
 */
function databaseWithInlineAudits({ audits = 20, rules = 300 } = {}): { home: string; databasePath: string; provenance: string } {
  const home = mkdtempSync(join(tmpdir(), "rerail-provenance-daemon-"));
  cleanups.push(() => removeFolder(home));
  const databasePath = join(home, "rerail.sqlite");
  openRerailDatabase(databasePath).$client.close();
  const provenance = JSON.stringify(Array.from({ length: rules }, (_, index) => ({
    ruleId: `typescript/rule-${index}`, revision: 1, payloadHash: `sha256:${index}`, applicability: "global", origin: "user",
  })));
  const older = new Database(databasePath);
  try {
    const insert = older.prepare(`INSERT INTO audits (audit_id, session_id, workspace_root, scope, mode, revision, catalog_revision,
      rule_revisions_json, rule_provenance_json, paths_json, started_at, finished_at, status, violations_json, failures_json)
      VALUES (?, 'session-a', '/home/dev/app', 'touched-files', 'steer', 1, 'catalog', '[]', ?, '[]', ?, ?, 'completed', '[]', '[]')`);
    older.transaction(() => {
      for (let index = 1; index <= audits; index += 1) insert.run(`older-${index}`, provenance, index, index + 1);
    })();
  } finally {
    older.close();
  }
  expect(fileState(databasePath).inline).toBe(audits);
  return { home, databasePath, provenance };
}

/** How many audits name the stored copy of `provenance`: those the move took, each reading it as before. */
function namingStoredCopy(path: string, provenance: string): number {
  const database = new Database(path, { readonly: true });
  try {
    return database.prepare(`SELECT count(*) FROM audits JOIN audit_rule_provenances ON provenance_id = rule_provenance_id
      WHERE audit_rule_provenances.rule_provenance_json = ?`).pluck().get(provenance) as number;
  } finally {
    database.close();
  }
}

/** Whether every audit has moved and the move has emptied the WAL its batches filled: the move ended. */
function moved(databasePath: string): boolean {
  return fileState(databasePath).inline === 0 && walEmpty(databasePath);
}

/** Whether the database's WAL is absent or empty. */
function walEmpty(databasePath: string): boolean {
  try {
    return statSync(`${databasePath}-wal`).size === 0;
  } catch {
    return true;
  }
}

describe("the daemon's move of audit rule provenance", () => {
  it("moves the provenance audits recorded before the upgrade hold inline once it starts, and leaves the pages it frees in the file", async () => {
    const { home, databasePath } = databaseWithInlineAudits();

    const daemon = await RerailDaemon.start({ version: "test", databasePath, workspaceRoot: home, port: 0, remoteSink: false });
    cleanups.push(() => daemon.close());

    await until(() => moved(databasePath));
    // No `VACUUM`: new history reuses those pages, and `rerail scrub` gives them back to the disk.
    expect(fileState(databasePath).freePages).toBeGreaterThan(0);
  }, TEST_MS);

  it("keeps its thread free while it moves, however long the move's checkpoint takes", async () => {
    const { home, databasePath } = databaseWithInlineAudits();
    // The checkpoint after the last batch holds the thread running it 3 s, as one batch of 100 audits did on a loaded
    // Windows machine. Were the move on the daemon's thread, nothing else would run there meanwhile: a hook's health
    // probe gives up after 2 s, and the hook then runs without the daemon.
    const pragma = Database.prototype.pragma;
    const held = new Int32Array(new SharedArrayBuffer(4));
    const slowCheckpoint = vi.spyOn(Database.prototype, "pragma").mockImplementation(function (this: Database.Database, source: string, options?: Database.PragmaOptions) {
      const result = pragma.call(this, source, options);
      if (source === "wal_checkpoint(PASSIVE)") Atomics.wait(held, 0, 0, 3_000);
      return result;
    });
    cleanups.push(() => slowCheckpoint.mockRestore());

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

  it("stops quietly while a batch waits for the write lock another process holds, and the next start moves the rest", async () => {
    const { home, databasePath, provenance } = databaseWithInlineAudits({ audits: 500, rules: 20 });
    const log = spyOnStderr();
    const other = new Database(databasePath);
    cleanups.push(() => {
      other.close();
    });

    const daemon = await RerailDaemon.start({ version: "test", databasePath, workspaceRoot: home, port: 0, remoteSink: false });
    cleanups.push(() => daemon.close());
    // Once the first batch has moved, another process takes the write lock before the next batch, which then finds it
    // held and tries again after each pause; the daemon stops a second and a half into those tries.
    await until(() => namingStoredCopy(databasePath, provenance) > 0, 10);
    other.exec("BEGIN IMMEDIATE");
    const left = fileState(databasePath).inline;
    await new Promise(resolve => setTimeout(resolve, 1_500));
    try {
      await daemon.close();
    } finally {
      other.exec("ROLLBACK");
    }

    expect(moveLines(log.writes())).toEqual([]);
    expect(left).toBeGreaterThan(0);
    expect(fileState(databasePath).inline).toBe(left);
    const next = await RerailDaemon.start({ version: "test", databasePath, workspaceRoot: home, port: 0, remoteSink: false });
    cleanups.push(() => next.close());
    await until(() => moved(databasePath));
    expect(namingStoredCopy(databasePath, provenance)).toBe(500);
    expect(moveLines(log.writes())).toEqual([]);
  }, 2 * TEST_MS);

  it("writes a failure of the move's thread to the daemon log with the stack of where the thread threw it", async () => {
    const { home, databasePath } = databaseWithInlineAudits();
    // The first batch fails as it stores the shared copy, on the move's thread.
    const refused = "this test refuses to store a rule provenance";
    const database = new Database(databasePath);
    try {
      database.exec(`CREATE TRIGGER refuse_provenance BEFORE INSERT ON audit_rule_provenances BEGIN SELECT RAISE(ABORT, '${refused}'); END`);
    } finally {
      database.close();
    }
    const log = spyOnStderr();

    const daemon = await RerailDaemon.start({ version: "test", databasePath, workspaceRoot: home, port: 0, remoteSink: false });
    cleanups.push(() => daemon.close());
    await until(() => moveLines(log.writes()).length > 0);

    const lines = moveLines(log.writes());
    expect(lines).toHaveLength(1);
    expect(lines[0]).toMatch(/\n +at moveBatch \(.*rule-provenance-move\.[jt]s:\d+:\d+\)\n/u);
    expect(lines[0]).toMatch(/\n +at whenFree \(.*rule-provenance-move\.[jt]s:\d+:\d+\)\n/u);
    expect(lines[0]).toMatch(/^rerail daemon: moving audit rule provenance failed: /u);
    expect(lines[0]).toContain(`${refused}\n`);
    // The audits read as before: each still holds its provenance in its row.
    expect(fileState(databasePath).inline).toBe(20);
  }, TEST_MS);
});

