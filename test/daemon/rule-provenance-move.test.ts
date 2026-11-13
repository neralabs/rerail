import { existsSync, mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import Database from "better-sqlite3";
import { afterEach, describe, expect, it, vi } from "vitest";
import { RerailDaemon } from "../../src/daemon/index.js";
import { codeRuleSchema, openRerailDatabase } from "../../src/engine/index.js";
import { ingestEngineEvent, RerailRuntime } from "../../src/runtime/index.js";
import { removeFolder } from "../support/platform/remove-folder.js";
import { slowVacuumNext, untilExists } from "../support/processes/slow-vacuum.js";
import { setUpTestRepository } from "../support/repository-rules.js";

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

/** As `MOVE_MS`, for a move whose `VACUUM` starts a process of its own, which loads its modules through `tsx` too. */
const SLOW_MOVE_MS = 2 * MOVE_MS;

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
  cleanups.push(() => removeFolder(home));
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

/**
 * The longest time this thread ran no timer while `run` ran: the longest gap between two ticks of a 10 ms timer,
 * counting the gap that ends as `run` resolves, which a tick after it would come too late to see.
 */
async function longestGapWhile(run: () => Promise<void>): Promise<number> {
  let longest = 0;
  let last = performance.now();
  const tick = (): void => {
    const now = performance.now();
    longest = Math.max(longest, now - last);
    last = now;
  };
  const ticks = setInterval(tick, 10);
  try {
    await run();
    tick();
  } finally {
    clearInterval(ticks);
  }
  return longest;
}

/** How many rows of `table` have `column` equal to `value`. */
function rowsWith(databasePath: string, table: string, column: string, value: string): number {
  const database = new Database(databasePath, { readonly: true });
  try {
    return database.prepare(`SELECT count(*) FROM ${table} WHERE ${column} = ?`).pluck().get(value) as number;
  } finally {
    database.close();
  }
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

  it("answers on its thread and records an edit's audit, then a session start, sent to it while the move's VACUUM holds the database", async () => {
    const { home, databasePath } = databaseWithInlineAudits();
    // The checkout sessions edit, apart from the database: one rule on its Python files, started as set-up leaves it.
    const checkout = join(home, "app");
    mkdirSync(checkout);
    setUpTestRepository(checkout, { library: [{ kind: "code", rule: NO_NEEDLE }] }, databasePath);
    // Until the edit's call is in, this thread commits to the file every 250 ms, so the move's VACUUM, which starts once
    // no other connection has committed for 2 s, cannot start before it.
    const busy = keptBusy(databasePath);
    cleanups.push(busy.stop);
    const runtimes = runtimesOfEngine();
    // The first two VACUUMs each hold the write lock 12 s, past the 5 s any Rerail writer waits for it: the VACUUM after
    // 4,000 audits held it 8.9 s on a loaded Windows machine.
    const held = join(home, "vacuum-held");
    const restore = slowVacuumNext({ held, holdMs: 12_000, holds: 2 });
    let daemon: RerailDaemon;
    try {
      daemon = await RerailDaemon.start({
        version: "test", databasePath, workspaceRoot: checkout, port: 0, remoteSink: false, nativeHealth: () => ({ ok: true, kernelVersion: "1" }),
      });
    } finally {
      restore();
    }
    cleanups.push(() => daemon.close());
    const engine = { socketPath: daemon.kernelSocketPath, accessToken: daemon.state().accessToken! };

    // A coding agent's session writes a file, as Claude Code's Write tool does: the session starts, its rules open (a
    // call made before that goes on unchecked), the call comes, and the file is written.
    const edited = join(checkout, "edited.py");
    await ingestEngineEvent(engine, { kind: "session-start", eventId: "start-edits", session: { host: "claude-code", sessionId: "edits", cwd: checkout } });
    await runtimes.only().codeRulesOpened("edits");
    await ingestEngineEvent(engine, {
      kind: "call", eventId: "write-call", sessionId: "edits", toolCallId: "write", toolName: "Write", input: { file_path: edited, content: "value = 1\n" },
    });
    writeFileSync(edited, "value = 1\n");
    busy.stop();
    expect(existsSync(held), "the move's VACUUM began while this thread still committed").toBe(false);

    // Each write below succeeds, and the thread answers within the 2 s a hook's health probe waits. Waiting out the lock,
    // each failed once SQLite's 5 s busy timeout ran out.
    const answered = { failure: undefined, heldTwoSeconds: false };

    // The call's result, sent to the daemon's engine socket as a hook sends it, while the first VACUUM holds the lock:
    // the daemon records the result and audits the edit through its own stores, on its thread.
    await untilExists(held, SLOW_MOVE_MS);
    const audit = await writtenWhileHeld(async () => {
      await ingestEngineEvent(engine, { kind: "result", eventId: "write-result", sessionId: "edits", toolCallId: "write", toolName: "Write", output: "written" });
      await until(() => auditStatuses(databasePath, "edits").join() === "completed");
    });
    expect(audit, `longest gap ${audit.longest} ms`).toMatchObject(answered);
    expect(rowsWith(databasePath, "audits", "session_id", "edits")).toBe(1);

    // That VACUUM gave way. The next one starts once no one has written for 4 s, and holds the lock too: a session start
    // sent to the daemon meets it, and the daemon records the session through its own store, on its thread.
    await untilExists(`${held}.2`, SLOW_MOVE_MS);
    const start = await writtenWhileHeld(() => ingestEngineEvent(engine, {
      kind: "session-start", eventId: "start-during-vacuum", session: { host: "claude-code", sessionId: "during-vacuum", cwd: checkout },
    }));
    expect(start, `longest gap ${start.longest} ms`).toMatchObject(answered);
    expect(rowsWith(databasePath, "sessions", "session_id", "during-vacuum")).toBe(1);
    // The third VACUUM runs once no one writes, and gives the space back.
    await until(() => moved(databasePath), SLOW_MOVE_MS);
  }, 4 * SLOW_MOVE_MS);
});

/** A rule on every Python file of the checkout: it steers away from a line holding `NEEDLE`. */
const NO_NEEDLE = codeRuleSchema.parse({
  format: "rerail.policy/v1", id: "test:no-needle", revision: 1, mode: "steer",
  scope: { include: ["**/*.py"], exclude: [] },
  clauses: [{
    id: "no-needle",
    violation: { code: "NEEDLE", anchor: "witness.span", message: "No needle" },
    forbid: { regex: { pattern: "NEEDLE", source: "file.content" } },
  }],
});

/** A connection on this thread that commits to the database every 250 ms until stopped. */
function keptBusy(databasePath: string): { stop: () => void } {
  const database = new Database(databasePath, { timeout: 5_000 });
  database.exec("CREATE TABLE IF NOT EXISTS test_commits (at)");
  const commits = setInterval(() => database.prepare("INSERT INTO test_commits VALUES (?)").run(Date.now()), 250);
  return {
    stop: () => {
      clearInterval(commits);
      if (database.open) database.close();
    },
  };
}

/**
 * The runtimes the daemon's engine socket hands events to, as each first ingests one; `only` is the one there is. A spy
 * that calls through, so a test can await a session's rules on it, as a native host's `await-code-rules` request does.
 */
function runtimesOfEngine(): { only: () => RerailRuntime } {
  const seen = new Set<RerailRuntime>();
  const ingest = RerailRuntime.prototype.ingest;
  const spy = vi.spyOn(RerailRuntime.prototype, "ingest").mockImplementation(function (this: RerailRuntime, event) {
    seen.add(this);
    return ingest.call(this, event);
  });
  cleanups.push(() => spy.mockRestore());
  return {
    only: () => {
      expect(seen.size).toBe(1);
      return [...seen][0]!;
    },
  };
}

/** The status of each audit of `sessionId`, in the order they started. */
function auditStatuses(databasePath: string, sessionId: string): string[] {
  const database = new Database(databasePath, { readonly: true });
  try {
    return database.prepare("SELECT status FROM audits WHERE session_id = ? ORDER BY started_at").pluck().all(sessionId) as string[];
  } finally {
    database.close();
  }
}

/**
 * What `write` met, run on this thread (the daemon's) while a VACUUM holds the write lock: what it threw, if anything,
 * whether the thread went 2 s or more without running a timer, and that longest gap.
 */
async function writtenWhileHeld(write: () => Promise<unknown>): Promise<{ failure: string | undefined; heldTwoSeconds: boolean; longest: number }> {
  let failure: string | undefined;
  const longest = await longestGapWhile(async () => {
    try {
      await write();
    } catch (error) {
      failure = (error as { code?: string }).code ?? String(error);
    }
  });
  return { failure, heldTwoSeconds: longest >= 2_000, longest: Math.round(longest) };
}
