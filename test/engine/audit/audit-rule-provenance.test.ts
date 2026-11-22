import { createHash } from "node:crypto";
import { readFileSync, statSync } from "node:fs";
import { mkdir, mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { setTimeout as sleep } from "node:timers/promises";
import Database from "better-sqlite3";
import { afterEach, describe, expect, it, vi } from "vitest";
import { SqliteAuditReader, SqliteAuditStore } from "../../../src/engine/audit/audit-store.js";
import type {
  AuditFailure, AuditInput, AuditRecord, AuditRuleProvenance, LegacyAuditRuleProvenance, RepositoryAuditRuleProvenance,
} from "../../../src/engine/audit/audit.js";
import { eraseFreedPages, openRerailDatabase, resetRepositoryHistory } from "../../../src/engine/storage/database.js";
import { RERAIL_BASELINE_SCHEMA_STATEMENTS } from "../../../src/engine/storage/baseline-schema.js";
import { RERAIL_BASELINE_SCHEMA_VERSION, RERAIL_MIGRATIONS, RULE_PROVENANCE_STEP } from "../../../src/engine/storage/migrations.js";
import {
  moveRuleProvenances, moveRuleProvenancesInWorker, type RuleProvenanceMoveOptions,
} from "../../../src/engine/storage/rule-provenance-move.js";
import { tableColumns } from "../../../src/engine/storage/table-probe.js";
import { removeFolder } from "../../support/platform/remove-folder.js";
import { SqliteAuditStore as MainAuditStore, type MainAuditRecord } from "../support/main-audit-store.js";

const temporaryDirectories: string[] = [];

afterEach(async () => {
  await Promise.all(temporaryDirectories.splice(0).map(path => removeFolder(path)));
});

async function temporaryDatabase(): Promise<string> {
  const root = await mkdtemp(join(tmpdir(), "rerail-rule-provenance-"));
  temporaryDirectories.push(root);
  const path = join(root, "home", "rerail.sqlite");
  await mkdir(dirname(path), { recursive: true });
  return path;
}

const REPOSITORY = "remote:github.com/example/app";

/**
 * A rule provenance as a checkout's catalog records it (`code-rule-dsl-catalog.ts`): `rules` rules of a pack, each on at
 * the repository's root and every fifth also placed, off and in Log, in one folder by the checkout and the repository.
 */
function provenanceOf(checkoutPath: string, rules: number): RepositoryAuditRuleProvenance[] {
  return Array.from({ length: rules }, (_, index) => ({
    ruleId: `typescript/rule-${String(index).padStart(3, "0")}`,
    revision: 1 + (index % 3),
    payloadHash: `sha256:${createHash("sha256").update(`rule-${index}`).digest("hex")}`,
    source: { kind: "pack", id: "typescript", packId: "typescript" },
    repositoryKey: REPOSITORY,
    checkoutPath,
    placement: [
      {
        folder: "", on: true, mode: "steer", onFrom: { by: "pack-start", packId: "typescript" }, modeFrom: { by: "rule-default" },
        suppressions: "honour", suppressionsFrom: { by: "rule-default" },
      },
      ...(index % 5 === 0
        ? [{
          folder: "web", on: false, mode: "log" as const,
          onFrom: { by: "checkout" as const, checkoutPath, folder: "web", target: "rule" as const },
          modeFrom: { by: "repository" as const, folder: "web", target: "pack" as const },
        }]
        : []),
    ],
  }));
}

/** A provenance recorded before repository-owned rules. */
const LEGACY: LegacyAuditRuleProvenance[] = Array.from({ length: 12 }, (_, index) => ({
  ruleId: `legacy-${index}`, revision: 1, payloadHash: `sha256:legacy-${index}`, applicability: index % 2 === 0 ? "global" : "project",
  ...(index % 2 === 0 ? {} : { projectId: "project-a" }), origin: "user",
}));

function auditInput(workspaceRoot: string, ruleProvenance: AuditRuleProvenance[], sessionId = "session-a"): AuditInput {
  return { sessionId, workspaceRoot, scope: "touched-files", mode: "steer", revision: 1, catalogRevision: "catalog", ruleRevisions: [], paths: ["src/a.ts"], ruleProvenance };
}

/** The file's pages that hold something (every page but those on the free list) and those that do not, in bytes. */
function pageBytes(path: string): { inUse: number; free: number } {
  const database = new Database(path, { readonly: true });
  try {
    const pragma = (name: string): number => database.pragma(name, { simple: true }) as number;
    const free = pragma("freelist_count") * pragma("page_size");
    return { inUse: pragma("page_count") * pragma("page_size") - free, free };
  } finally {
    database.close();
  }
}

function bytesInUse(path: string): number {
  return pageBytes(path).inUse;
}

/** An audit as Rerail before this change wrote one, and as an older Rerail sharing the file still does: provenance inline. */
const INLINE_AUDIT = `INSERT INTO audits (audit_id, session_id, workspace_root, scope, mode, revision, catalog_revision,
  rule_revisions_json, rule_provenance_json, paths_json, started_at, finished_at, status, violations_json, failures_json)
  VALUES (?, ?, ?, 'touched-files', 'steer', 1, 'catalog', '[]', ?, '[]', ?, ?, 'completed', '[]', '[]')`;

/**
 * Writes one audit per provenance as an older Rerail does, provenance inline, the first beginning at `from` (epoch ms)
 * and each later one a millisecond after the one before; each audit's id and provenance text.
 */
function writeInlineAudits(path: string, provenances: readonly AuditRuleProvenance[][], from = 0): { auditId: string; text: string }[] {
  const written = provenances.map((provenance, index) => ({ auditId: `older-${index}`, text: JSON.stringify(provenance) }));
  const older = new Database(path);
  try {
    // One transaction: one sync for hundreds of audits.
    older.transaction(() => {
      written.forEach(({ auditId, text }, index) => older.prepare(INLINE_AUDIT).run(auditId, "session-a", "/home/dev/app", text, from + index, from + index + 1));
    })();
  } finally {
    older.close();
  }
  return written;
}

/** How many audits hold their provenance inline. */
function inlineAudits(path: string): number {
  const database = new Database(path, { readonly: true });
  try {
    return database.prepare("SELECT count(*) FROM audits WHERE rule_provenance_json <> '[]'").pluck().get() as number;
  } finally {
    database.close();
  }
}

/** The text of each stored copy of a rule provenance, sorted. */
function storedTexts(path: string): string[] {
  const database = new Database(path, { readonly: true });
  try {
    return (database.prepare("SELECT rule_provenance_json FROM audit_rule_provenances").pluck().all() as string[]).sort();
  } finally {
    database.close();
  }
}

/** How many audits name no stored copy of their provenance, as a Rerail older than this one writes each audit. */
function auditsNamingNoCopy(path: string): number {
  const database = new Database(path, { readonly: true });
  try {
    return database.prepare("SELECT count(*) FROM audits WHERE rule_provenance_id IS NULL").pluck().get() as number;
  } finally {
    database.close();
  }
}

/** An audit an older Rerail sharing the file records at `now` (epoch ms), its provenance in its row; when it began. */
function recordAsOlder(path: string, auditId: string, provenance: readonly AuditRuleProvenance[], now = Date.now()): number {
  const older = new Database(path);
  try {
    older.prepare(INLINE_AUDIT).run(auditId, "session-a", "/home/dev/app", JSON.stringify(provenance), now, now + 1);
    return now;
  } finally {
    older.close();
  }
}

/**
 * Upgrades a file of the build before with this build's open, which runs the step that keeps each provenance once, and
 * records the step as run at `stepAt` (epoch ms), the time the move compares each audit's start with. A test then dates
 * its audits from that number: the open stamps the step with the clock once its DDL ends, and on a fast file system,
 * such as a temporary folder in memory, that is the millisecond the test dated its audits from.
 */
function upgradeAt(path: string, stepAt: number): void {
  openRerailDatabase(path).$client.close();
  const database = new Database(path);
  try {
    database.prepare("UPDATE schema_migrations SET applied_at = ? WHERE id = ?").run(stepAt, RULE_PROVENANCE_STEP);
  } finally {
    database.close();
  }
}

/** Dates audit `auditId` as begun at `at` (epoch ms), as a clock that read `at` when it was recorded did. */
function beganAt(path: string, auditId: string, at: number): void {
  const database = new Database(path);
  try {
    database.prepare("UPDATE audits SET started_at = ? WHERE audit_id = ?").run(at, auditId);
  } finally {
    database.close();
  }
}

/** Runs `statement` on the file, as a file in that state was written. */
function rewrite(path: string, statement: string): void {
  const database = new Database(path);
  try {
    database.exec(statement);
  } finally {
    database.close();
  }
}

/** When the file ran the step that keeps each provenance once (epoch ms). */
function stepAppliedAt(path: string): number {
  const database = new Database(path, { readonly: true });
  try {
    return database.prepare("SELECT applied_at FROM schema_migrations WHERE id = ?").pluck().get(RULE_PROVENANCE_STEP) as number;
  } finally {
    database.close();
  }
}

/**
 * Records `count` audits in one statement, so SQLite copies `text` without a round trip per audit: `<prefix>-<n>`, each
 * with `text` in its row and naming `provenanceId` (null, as a Rerail older than this one writes each audit), the first
 * beginning at `from` (epoch ms) and each later one a millisecond after the one before.
 */
function recordMany(path: string, { prefix, count, from, text, provenanceId = null }: {
  prefix: string; count: number; from: number; text: string; provenanceId?: number | null;
}): void {
  const database = new Database(path);
  try {
    database.prepare(`WITH RECURSIVE n(i) AS (SELECT 0 UNION ALL SELECT i + 1 FROM n WHERE i + 1 < ?)
      INSERT INTO audits (audit_id, session_id, workspace_root, scope, mode, revision, catalog_revision, rule_revisions_json,
        rule_provenance_json, rule_provenance_id, paths_json, started_at, finished_at, status, violations_json, failures_json)
      SELECT ? || '-' || i, 'session-b', '/home/dev/app', 'touched-files', 'steer', 1, 'catalog', '[]', ?, ?, '[]', ? + i,
        ? + i + 1, 'completed', '[]', '[]' FROM n`).run(count, prefix, text, provenanceId, from, from);
  } finally {
    database.close();
  }
}

/** The id of the stored copy of `[]`, which this build's store names for every audit that ran no rules, stored first. */
function storedEmptyProvenance(path: string): number {
  const database = new Database(path);
  try {
    const digest = createHash("sha256").update("[]").digest("hex");
    database.prepare("INSERT OR IGNORE INTO audit_rule_provenances (digest, rule_provenance_json) VALUES (?, '[]')").run(digest);
    return database.prepare("SELECT provenance_id FROM audit_rule_provenances WHERE digest = ?").pluck().get(digest) as number;
  } finally {
    database.close();
  }
}

/**
 * Times the statements this thread prepares from here on that read audits (`FROM audits`): each `get`, `all` and `run`,
 * and each row `iterate` gives. The time a write takes to commit is left out: it waits on the disk, whatever a statement
 * reads. Puts `prepare` back on `restore`. A move's thread prepares its own statements, which this does not see.
 */
function timeAuditReads(): { totalMs: () => number; restore: () => void } {
  let total = 0;
  const timed = <T>(run: () => T): T => {
    const began = performance.now();
    try {
      return run();
    } finally {
      total += performance.now() - began;
    }
  };
  const prepare = Database.prototype.prepare;
  const spy = vi.spyOn(Database.prototype, "prepare").mockImplementation(function (this: Database.Database, source: string) {
    const statement = prepare.call(this, source);
    if (!/\bFROM audits\b/u.test(source)) return statement;
    for (const kind of ["get", "all", "run"] as const) {
      const original = statement[kind].bind(statement) as (...values: unknown[]) => unknown;
      Object.assign(statement, { [kind]: (...values: unknown[]) => timed(() => original(...values)) });
    }
    const iterate = statement.iterate.bind(statement) as (...values: unknown[]) => IterableIterator<unknown>;
    Object.assign(statement, {
      iterate: (...values: unknown[]) => {
        const rows = iterate(...values);
        return {
          [Symbol.iterator]() {
            return this;
          },
          next: () => timed(() => rows.next()),
          return: (value?: unknown) => rows.return?.(value) ?? { done: true, value },
        };
      },
    });
    return statement;
  });
  return { totalMs: () => total, restore: () => spy.mockRestore() };
}

/**
 * Makes the reads of the move's check before a batch, on its writer (the statements on `audits_rule_provenance` that
 * read from no rowid on: `takes` and `lastOlderWrite`), throw the error SQLite gives a read while another process
 * recovers the WAL, while `busy` is on; how many times they threw it, and how to put `prepare` back.
 */
function busyChecks(): { setBusy: (busy: boolean) => void; answered: () => number; restore: () => void } {
  let busy = false;
  let answered = 0;
  const prepare = Database.prototype.prepare;
  const spy = vi.spyOn(Database.prototype, "prepare").mockImplementation(function (this: Database.Database, source: string) {
    const statement = prepare.call(this, source);
    if (this.readonly || !source.includes("INDEXED BY audits_rule_provenance") || /rowid >/u.test(source)) return statement;
    for (const kind of ["get", "iterate"] as const) {
      const original = statement[kind].bind(statement) as (...values: unknown[]) => unknown;
      Object.assign(statement, {
        [kind]: (...values: unknown[]) => {
          if (!busy) return original(...values);
          answered += 1;
          throw new Database.SqliteError("database is locked", "SQLITE_BUSY_RECOVERY");
        },
      });
    }
    return statement;
  });
  return { setBusy: value => { busy = value; }, answered: () => answered, restore: () => spy.mockRestore() };
}

/** The start of the line a move writes while it waits for an older Rerail sharing the file. */
const WAITS_FOR_OLDER = "moving the rule provenance of audits recorded before this Rerail's upgrade waits until";

/**
 * Writes `text` as audit `auditId`'s provenance in its row, past the CHECK on `audits` that refuses text that is not a
 * JSON array, as only a damaged file or a hand edit can.
 */
function damageProvenance(path: string, auditId: string, text: string): void {
  const database = new Database(path);
  try {
    database.pragma("ignore_check_constraints = ON");
    database.prepare("UPDATE audits SET rule_provenance_json = ? WHERE audit_id = ?").run(text, auditId);
  } finally {
    database.close();
  }
}

/** What `read` gives back as JSON text, or what it throws, as the error's name and message. */
function readOutcome(read: () => unknown): string {
  try {
    return JSON.stringify(read());
  } catch (error) {
    return error instanceof Error ? `${error.name}: ${error.message}` : String(error);
  }
}

/** The steps an older Rerail runs when it opens the file: this build's, without the one this change adds. */
const OLDER_STEPS = RERAIL_MIGRATIONS.filter(step => step.id !== RULE_PROVENANCE_STEP);

/**
 * The audits `auditIds` name as an older Rerail sharing the file reads them: `main`'s audit store (its read path, in
 * `main-audit-store.ts`), on the file opened with the older build's steps.
 */
function readAsOlder(path: string, auditIds: readonly string[]): (MainAuditRecord | undefined)[] {
  const older = new MainAuditStore(path, openRerailDatabase(path, { migrations: OLDER_STEPS }));
  try {
    return auditIds.map(auditId => older.get(auditId));
  } finally {
    older.close();
  }
}

/** Each audit's rule provenance as JSON text; undefined for an audit not found. */
function provenanceTexts(audits: readonly (MainAuditRecord | undefined)[]): (string | undefined)[] {
  return audits.map(audit => audit && JSON.stringify(audit.ruleProvenance));
}

/**
 * Each audit record without its rule provenance, and without the `origin` an older Rerail never reads; undefined for
 * an audit not found.
 */
function withoutProvenance(audits: readonly (MainAuditRecord | undefined)[]): (Omit<MainAuditRecord, "ruleProvenance"> | undefined)[] {
  return audits.map(audit => {
    if (audit === undefined) return undefined;
    const { ruleProvenance: _provenance, origin: _origin, ...rest } = audit as MainAuditRecord & { origin?: unknown };
    return rest;
  });
}

/** Spies on this thread's stderr: what it has written so far, read at any time, and how to put it back. */
function spyOnStderr(): { logged: () => string; restore: () => void } {
  const stderr = vi.spyOn(process.stderr, "write").mockReturnValue(true);
  return { logged: () => stderr.mock.calls.map(([chunk]) => String(chunk)).join(""), restore: () => stderr.mockRestore() };
}

/**
 * The daemon's move of the provenance audits hold inline, without its pause between batches, and timed by a clock that
 * never moves: each batch then takes twice the audits of the one before, up to `perBatch`, however loaded the machine.
 */
function move(path: string, options: RuleProvenanceMoveOptions = {}): Promise<number> {
  return moveRuleProvenances(path, { pause: () => Promise.resolve(), now: () => 0, ...options });
}

/** What a later daemon start of the move did (`laterStart`). */
interface LaterStart {
  readonly moved: number;
  readonly waits: readonly number[];
  readonly logged: string;
  readonly readMs: number;
}

/**
 * A later daemon start of the move on `path`, by the time `clock` gives: what it resolved to, each wait for an older
 * Rerail it asked its timer for (the first ends the start, as a stop would), what it wrote to stderr, and how long this
 * thread's reads of audits took.
 */
async function laterStart(path: string, clock: () => number): Promise<LaterStart> {
  const stop = new AbortController();
  const waits: number[] = [];
  const timer = (ms: number): Promise<void> => {
    waits.push(ms);
    stop.abort();
    return Promise.reject(new Error("stopped"));
  };
  const stderr = spyOnStderr();
  const timed = timeAuditReads();
  try {
    const moved = await moveRuleProvenancesInWorker(path, stop.signal, { clock, timer });
    return { moved, waits, logged: stderr.logged(), readMs: timed.totalMs() };
  } finally {
    timed.restore();
    stderr.restore();
  }
}

/**
 * What `run` resolves to, and the longest time this thread ran no timer meanwhile: the longest gap between two ticks of
 * a 10 ms timer, counting the gap that ends as `run` resolves, which a tick after it would come too late to see.
 */
async function withLongestStall<T>(run: () => Promise<T>): Promise<{ result: T; longest: number }> {
  let longest = 0;
  let last = performance.now();
  const tick = (): void => {
    const now = performance.now();
    longest = Math.max(longest, now - last);
    last = now;
  };
  const ticks = setInterval(tick, 10);
  try {
    const result = await run();
    tick();
    return { result, longest };
  } finally {
    clearInterval(ticks);
  }
}

/** The step that keeps each provenance once, and what it adds to a file. */
const STEP = RULE_PROVENANCE_STEP;

/** A file in the shape the build before this change wrote: this build's, without what `STEP` adds or its record of it. */
async function fileOfTheBuildBefore(): Promise<string> {
  const path = await temporaryDatabase();
  openRerailDatabase(path).$client.close();
  const database = new Database(path);
  try {
    database.exec("DROP TRIGGER IF EXISTS audits_rule_provenance_delete; DROP INDEX IF EXISTS audits_rule_provenance");
    if (tableColumns(database, "audits").includes("rule_provenance_id")) database.exec("ALTER TABLE audits DROP COLUMN rule_provenance_id");
    database.exec("DROP TABLE IF EXISTS audit_rule_provenances; DROP TABLE IF EXISTS audit_rule_provenance_step");
    database.prepare("DELETE FROM schema_migrations WHERE id = ?").run(STEP);
    // Nothing else of the step is left, so a later change to it shows up here rather than as a file older than claimed.
    expect(database.prepare("SELECT name FROM sqlite_master WHERE name LIKE '%provenance%' OR sql LIKE '%rule_provenance_id%'").all()).toEqual([]);
  } finally {
    database.close();
  }
  return path;
}

/** A file created at the migration baseline (0.1.0's shape), before any step. */
async function fileAtTheBaseline(): Promise<string> {
  const path = await temporaryDatabase();
  const database = new Database(path);
  try {
    for (const statement of RERAIL_BASELINE_SCHEMA_STATEMENTS) database.exec(statement);
    database.pragma(`user_version = ${RERAIL_BASELINE_SCHEMA_VERSION}`);
  } finally {
    database.close();
  }
  return path;
}

describe("audit rule provenance", () => {
  it("keeps one copy of a rule provenance however many audits record it", async () => {
    const path = await temporaryDatabase();
    const store = new SqliteAuditStore(path);
    try {
      const provenance = provenanceOf("/home/dev/app", 200);
      const size = Buffer.byteLength(JSON.stringify(provenance));
      const empty = bytesInUse(path);
      const audited: string[] = [];
      for (let index = 0; index < 20; index += 1) {
        const auditId = store.start(auditInput("/home/dev/app", provenance));
        store.finish(auditId, { status: "completed" });
        audited.push(auditId);
      }
      // One copy and twenty audits' own rows, not twenty copies.
      expect(bytesInUse(path) - empty).toBeLessThan(2 * size);

      // Another checkout's provenance is its own copy. Audits that ran no rules name one stored copy of `[]`, so an
      // audit that names none is one a Rerail older than this one wrote.
      const worktree = provenanceOf("/home/dev/app-worktree", 200);
      const other = store.start(auditInput("/home/dev/app-worktree", worktree));
      const none = store.start(auditInput("/home/dev/app", []));
      store.finish(store.start(auditInput("/home/dev/app", [])), { status: "completed" });
      expect(bytesInUse(path) - empty).toBeLessThan(3 * size);
      expect(storedTexts(path)).toEqual([JSON.stringify(provenance), JSON.stringify(worktree), "[]"].sort());
      expect(auditsNamingNoCopy(path)).toBe(0);

      for (const auditId of audited) expect(store.get(auditId)?.ruleProvenance).toEqual(provenance);
      expect(store.get(other)?.ruleProvenance).toEqual(worktree);
      expect(store.get(none)?.ruleProvenance).toEqual([]);
      const listed = new Map(store.list({ limit: 200 }).audits.map(audit => [audit.auditId, audit.ruleProvenance]));
      expect(listed.size).toBe(23);
      for (const auditId of audited) expect(listed.get(auditId)).toEqual(provenance);
      expect(listed.get(other)).toEqual(worktree);
      expect(listed.get(none)).toEqual([]);
    } finally {
      store.close();
    }
  });

  it.each([
    ["the build before this change", fileOfTheBuildBefore],
    ["Rerail at the migration baseline", fileAtTheBaseline],
  ])("reads every audit's provenance unchanged once a file %s wrote is upgraded and moved, and keeps each provenance once", async (_, olderFile) => {
    const path = await olderFile();
    const app = provenanceOf("/home/dev/app", 200);
    const worktree = provenanceOf("/home/dev/app-worktree", 200);
    // Audits sharing one provenance, another checkout's, one recorded before repository-owned rules, and none.
    const written = writeInlineAudits(path, [
      ...Array.from({ length: 12 }, () => app),
      ...Array.from({ length: 3 }, () => worktree),
      LEGACY,
      [],
      [],
    ]);
    const before = bytesInUse(path);
    // What the file holds beyond one copy of each distinct provenance.
    const distinct = new Set(written.map(({ text }) => text));
    const extra = written.reduce((total, { text }) => total + Buffer.byteLength(text), 0)
      - [...distinct].reduce((total, text) => total + Buffer.byteLength(text), 0);

    const store = new SqliteAuditStore(path);
    const readsAsWritten = (): void => {
      for (const { auditId, text } of written) expect(JSON.stringify(store.get(auditId)?.ruleProvenance), auditId).toBe(text);
      const listed = new Map(store.list({ limit: 200 }).audits.map(audit => [audit.auditId, JSON.stringify(audit.ruleProvenance)]));
      expect(listed).toEqual(new Map(written.map(({ auditId, text }) => [auditId, text])));
    };
    try {
      // The upgrade leaves each provenance inline, where the store reads it, until the move.
      readsAsWritten();
      expect(await move(path)).toBe(18);
      // The move's pages went through the WAL; it is emptied once the move ends, not kept that size while the store
      // holds the file.
      expect(statSync(`${path}-wal`).size).toBe(0);
      readsAsWritten();
      // The upgrade adds tables and indexes of its own to a baseline file, hence the margin.
      expect(before - bytesInUse(path)).toBeGreaterThan(0.7 * extra);

      // A later audit of the same rules takes no copy of its own.
      const upgraded = bytesInUse(path);
      const later = store.start(auditInput("/home/dev/app", app));
      expect(store.get(later)?.ruleProvenance).toEqual(app);
      expect(bytesInUse(path) - upgraded).toBeLessThan(Buffer.byteLength(JSON.stringify(app)) / 2);

      // An older Rerail sharing the file writes its provenance inline still, and it reads back the same. A later move
      // leaves it there, where the older Rerail reads it too. It began a millisecond after the step, whatever the clock
      // read meanwhile.
      const sharing = new Database(path);
      try {
        const after = stepAppliedAt(path) + 1;
        sharing.prepare(INLINE_AUDIT).run("older-after", "session-a", "/home/dev/app", JSON.stringify(worktree), after, after + 1);
      } finally {
        sharing.close();
      }
      expect(store.get("older-after")?.ruleProvenance).toEqual(worktree);
      expect(await move(path)).toBe(0);
      expect(inlineAudits(path)).toBe(1);
    } finally {
      store.close();
    }
  });

  it("lets an older Rerail open the file and read every field of each audit, with [] as the provenance of those this build moved or wrote", async () => {
    const path = await fileOfTheBuildBefore();
    const app = provenanceOf("/home/dev/app", 200);
    // Audits sharing one provenance, one recorded before repository-owned rules, and one with none.
    const written = writeInlineAudits(path, [app, app, app, LEGACY, []]);
    const olderIds = written.map(({ auditId }) => auditId);
    // This build's open runs the step and moves nothing: the older build still reads each provenance in its row.
    openRerailDatabase(path).$client.close();
    const before = readAsOlder(path, olderIds);
    expect(provenanceTexts(before)).toEqual(written.map(({ text }) => text));

    expect(await move(path)).toBe(5);
    const store = new SqliteAuditStore(path);
    let ids: string[];
    let current: (AuditRecord | undefined)[];
    try {
      // A later audit of the same rules, and one that ran none.
      const later = store.start(auditInput("/home/dev/app", app));
      store.finish(later, { status: "completed" });
      const none = store.start(auditInput("/home/dev/app", []));
      store.finish(none, { status: "completed" });
      ids = [...olderIds, later, none];
      current = ids.map(auditId => store.get(auditId));
    } finally {
      store.close();
    }

    // The older build reads `[]` as the provenance of each audit this build moved or wrote, and every other field as it
    // read it before the move and as this build reads it.
    const after = readAsOlder(path, ids);
    expect(provenanceTexts(after)).toEqual(ids.map(() => "[]"));
    expect(withoutProvenance(after.slice(0, olderIds.length))).toEqual(withoutProvenance(before));
    expect(withoutProvenance(after)).toEqual(withoutProvenance(current));
    expect(provenanceTexts(current)).toEqual([...written.map(({ text }) => text), JSON.stringify(app), "[]"]);
  });

  it("leaves each audit's provenance inline on the open that upgrades, and moves it a batch per write transaction", async () => {
    const path = await fileOfTheBuildBefore();
    const written = writeInlineAudits(path, Array.from({ length: 30 }, () => provenanceOf("/home/dev/app", 200)));

    openRerailDatabase(path).$client.close();
    // The open added the step's table and column, and moved nothing.
    expect(inlineAudits(path)).toBe(30);

    // Another process writing that never waits for the lock: between two batches it is free. It records sessions, since
    // an older Rerail's audit would stop the move.
    const writer = new Database(path, { timeout: 0 });
    const store = new SqliteAuditStore(path);
    try {
      let pauses = 0;
      const moved = await move(path, {
        perBatch: 10,
        pause: () => {
          pauses += 1;
          writer.prepare("INSERT INTO sessions (session_id, host, cwd, repository_id, started_at, ended_at) VALUES (?, 'codex', '/w', ?, 1, 2)")
            .run(`between-${pauses}`, REPOSITORY);
          return Promise.resolve();
        },
      });
      // A pause before each of three batches of 10, and before the one that finds none left.
      expect(moved).toBe(30);
      expect(pauses).toBe(4);
      expect(inlineAudits(path)).toBe(0);
      for (const { auditId, text } of written) expect(JSON.stringify(store.get(auditId)?.ruleProvenance), auditId).toBe(text);
    } finally {
      store.close();
      writer.close();
    }
  });

  it("moves nothing while an older Rerail sharing the file has recorded an audit within the window, so it reads every recorded audit's provenance, and moves once the window has passed", async () => {
    const path = await fileOfTheBuildBefore();
    const app = provenanceOf("/home/dev/app", 200);
    const written = writeInlineAudits(path, Array.from({ length: 30 }, () => app));
    const recorded = written.map(({ auditId }) => auditId);
    // This build upgrades the file a day after those audits began; then an older Rerail sharing it records an audit, its
    // provenance in its row. The clock is the test's own from here on.
    const day = 24 * 3_600_000;
    upgradeAt(path, day);
    const at = recordAsOlder(path, "older-after", app, day + 1);

    // Six days on, the move takes nothing: the older Rerail still reads each recorded audit's provenance in its row.
    expect(await move(path, { clock: () => at + 6 * day })).toBe(0);
    expect(provenanceTexts(readAsOlder(path, recorded))).toEqual(written.map(({ text }) => text));
    expect(inlineAudits(path)).toBe(31);

    // Eight days on, with no audit of an older Rerail for the week since, the move takes the recorded audits and leaves
    // the older Rerail's own in its row, where both versions read it.
    expect(await move(path, { clock: () => at + 8 * day })).toBe(30);
    expect(provenanceTexts(readAsOlder(path, ["older-after"]))).toEqual([JSON.stringify(app)]);
    expect(inlineAudits(path)).toBe(1);
    const store = new SqliteAuditStore(path);
    try {
      for (const { auditId, text } of written) expect(JSON.stringify(store.get(auditId)?.ruleProvenance), auditId).toBe(text);
    } finally {
      store.close();
    }
  });

  it("moves nothing until a week after the last audit recorded before the upgrade, with only this build running, and every audit reads in full meanwhile", async () => {
    const path = await fileOfTheBuildBefore();
    const app = provenanceOf("/home/dev/app", 200);
    // The build before records 30 audits in the hour before this build upgrades the file, and runs no more.
    const from = Date.now() - 3_600_000;
    const written = writeInlineAudits(path, Array.from({ length: 30 }, () => app), from);
    const last = from + written.length - 1;
    const store = new SqliteAuditStore(path);
    const day = 24 * 3_600_000;
    try {
      // This build records audits from the upgrade on, two of them with no rules on: each names its stored copy, an
      // empty one included, so none of them holds the move back.
      for (const provenance of [app, [], []]) store.finish(store.start(auditInput("/home/dev/app", provenance)), { status: "completed" });

      // An hour on, and a millisecond before the week since the last of them ends, the move takes nothing: a Rerail
      // that recorded them may still run beside this one, and it reads each recorded audit's provenance in its row.
      for (const later of [3_600_000, 7 * day - 1]) expect(await move(path, { clock: () => last + later })).toBe(0);
      expect(provenanceTexts(readAsOlder(path, written.map(({ auditId }) => auditId)))).toEqual(written.map(({ text }) => text));
      expect(inlineAudits(path)).toBe(30);

      // A week after the last of them, the move takes them all, and this build reads each in full.
      expect(await move(path, { clock: () => last + 7 * day })).toBe(30);
      expect(inlineAudits(path)).toBe(0);
      for (const { auditId, text } of written) expect(JSON.stringify(store.get(auditId)?.ruleProvenance), auditId).toBe(text);
    } finally {
      store.close();
    }
  });

  it("moves nothing while an older Rerail sharing the file records audits that run no rules, so it reads every recorded audit's provenance, and moves a week after its last", async () => {
    const path = await fileOfTheBuildBefore();
    const app = provenanceOf("/home/dev/app", 200);
    const from = Date.now() - 3_600_000;
    const written = writeInlineAudits(path, Array.from({ length: 30 }, () => app), from);
    const recorded = written.map(({ auditId }) => auditId);
    const last = from + written.length - 1;
    openRerailDatabase(path).$client.close();
    const day = 24 * 3_600_000;

    // The Rerail that recorded them runs on beside this one with no rules on, in a checkout with none or with a catalog
    // it could not prepare: each audit it records holds `[]` in its row, as this build's rule-less audits did before.
    // It records one 6 days after the last recorded audit, and another 6 days after that; a day after each, past the
    // week since the audit before, the move takes nothing, and it reads every recorded audit's provenance in its row.
    let previous = last;
    for (const index of [0, 1]) {
      previous = recordAsOlder(path, `ruleless-${index}`, [], previous + 6 * day);
      expect(await move(path, { clock: () => previous + day })).toBe(0);
      expect(provenanceTexts(readAsOlder(path, recorded))).toEqual(written.map(({ text }) => text));
    }
    expect(await move(path, { clock: () => previous + 7 * day - 1 })).toBe(0);
    expect(inlineAudits(path)).toBe(30);

    // It records none for a week: the move takes the recorded audits. Both versions read its own audits as `[]`.
    expect(await move(path, { clock: () => previous + 7 * day })).toBe(30);
    expect(provenanceTexts(readAsOlder(path, ["ruleless-0", "ruleless-1"]))).toEqual(["[]", "[]"]);
    const store = new SqliteAuditStore(path);
    try {
      for (const { auditId, text } of written) expect(JSON.stringify(store.get(auditId)?.ruleProvenance), auditId).toBe(text);
      expect(provenanceTexts(["ruleless-0", "ruleless-1"].map(auditId => store.get(auditId)))).toEqual(["[]", "[]"]);
    } finally {
      store.close();
    }
  });

  it("moves nothing while a Rerail that ran before the upgrade records an audit at least once a week, whenever in the week it comes", async () => {
    const path = await fileOfTheBuildBefore();
    const app = provenanceOf("/home/dev/app", 200);
    const from = Date.now() - 3_600_000;
    const written = writeInlineAudits(path, Array.from({ length: 30 }, () => app), from);
    const recorded = written.map(({ auditId }) => auditId);
    openRerailDatabase(path).$client.close();
    const day = 24 * 3_600_000;

    // The build before runs on beside this one, and records its next audit an hour after the upgrade, then six days,
    // then six days and 23 hours after the one before. Each time, a millisecond before that audit comes, the move takes
    // nothing, and the build before reads every recorded audit's provenance in its row.
    let previous = Date.now();
    for (const [index, gap] of [3_600_000, 6 * day, 7 * day - 3_600_000].entries()) {
      expect(await move(path, { clock: () => previous + gap - 1 })).toBe(0);
      expect(provenanceTexts(readAsOlder(path, recorded))).toEqual(written.map(({ text }) => text));
      previous = recordAsOlder(path, `beside-${index}`, app, previous + gap);
    }
    expect(await move(path, { clock: () => previous + 7 * day - 1 })).toBe(0);

    // It records none for a week: the move takes the recorded audits and leaves its own three in their rows.
    expect(await move(path, { clock: () => previous + 7 * day })).toBe(30);
    expect(inlineAudits(path)).toBe(3);
  });

  it("says once in its log, naming the last audit recorded before the upgrade, why it waits and until when, then moves", async () => {
    const path = await fileOfTheBuildBefore();
    // The build before records 30 audits from 2 s ago, and this build upgrades the file a second after the first.
    const from = Date.now() - 2_000;
    const written = writeInlineAudits(path, Array.from({ length: 30 }, () => provenanceOf("/home/dev/app", 200)), from);
    const last = from + written.length - 1;
    upgradeAt(path, from + 1_000);
    const stderr = spyOnStderr();
    let logged = "";
    try {
      // A window of 3 s: the move waits until 3 s after the last audit recorded before the upgrade.
      expect(await moveRuleProvenancesInWorker(path, undefined, { olderWindowMs: 3_000 })).toBe(30);
    } finally {
      logged = stderr.logged();
      stderr.restore();
    }

    const waits = logged.split("\n").filter(line => line.includes(WAITS_FOR_OLDER));
    expect(waits).toEqual([`rerail: ${path}: ${WAITS_FOR_OLDER} ${new Date(last + 3_000).toISOString()}: the last audit a `
      + "Rerail older than this one recorded, which reads each audit's rule provenance only from the audit's own row, began at "
      + `${new Date(last).toISOString()}. The move starts once no such audit has begun for 3 s.`]);
    expect(inlineAudits(path)).toBe(0);
  }, 60_000);

  it("stops before its next batch once an older Rerail sharing the file records an audit, leaving the audits not yet moved where it reads them", async () => {
    const path = await fileOfTheBuildBefore();
    const written = writeInlineAudits(path, Array.from({ length: 30 }, () => provenanceOf("/home/dev/app", 200)));
    openRerailDatabase(path).$client.close();
    let pauses = 0;
    const moved = await move(path, {
      perBatch: 10,
      // Before the second batch, an older Rerail records an audit.
      pause: () => {
        if ((pauses += 1) === 2) recordAsOlder(path, "older-during", provenanceOf("/home/dev/app", 20));
        return Promise.resolve();
      },
    });

    // The first batch moved 10; the move stopped there.
    expect(moved).toBe(10);
    expect(pauses).toBe(2);
    const older = provenanceTexts(readAsOlder(path, written.map(({ auditId }) => auditId)));
    expect(older.slice(0, 10)).toEqual(written.slice(0, 10).map(() => "[]"));
    expect(older.slice(10)).toEqual(written.slice(10).map(({ text }) => text));
    expect(inlineAudits(path)).toBe(21);
    expect(pageBytes(path).free).toBeGreaterThan(0);
  });

  it("waits on its calling thread, saying why and until when, while an older Rerail sharing the file records audits, then moves", async () => {
    const path = await fileOfTheBuildBefore();
    const app = provenanceOf("/home/dev/app", 200);
    writeInlineAudits(path, Array.from({ length: 30 }, () => app));
    // This build upgraded the file a second ago; the older Rerail's audits begin after that.
    upgradeAt(path, Date.now() - 1_000);
    recordAsOlder(path, "beside-1", app);
    const stderr = spyOnStderr();
    let second: number;
    let logged = "";
    try {
      // A window of 3 s: the move waits until 3 s after the older Rerail's audit, and another audit 1.5 s in makes it
      // wait again, until 3 s after that one.
      const moving = moveRuleProvenancesInWorker(path, undefined, { olderWindowMs: 3_000 });
      await new Promise(resolve => setTimeout(resolve, 1_500));
      second = recordAsOlder(path, "beside-2", app);
      expect(await moving).toBe(30);
    } finally {
      logged = stderr.logged();
      stderr.restore();
    }

    const waits = logged.split("\n").filter(line => line.includes(WAITS_FOR_OLDER));
    expect(waits).toHaveLength(2);
    expect(waits[1]).toContain(`waits until ${new Date(second + 3_000).toISOString()}: the last audit a Rerail older than `
      + `this one recorded, which reads each audit's rule provenance only from the audit's own row, began at `
      + `${new Date(second).toISOString()}.`);
    expect(waits[1]).toContain("The move starts once no such audit has begun for 3 s.");
    // The older Rerail's two audits stay in their rows; the recorded ones moved.
    expect(inlineAudits(path)).toBe(2);
  }, 60_000);

  it("waits at most an hour a pass for an audit dated past its clock, reading the file again after each, and says so once", async () => {
    const path = await fileOfTheBuildBefore();
    writeInlineAudits(path, [provenanceOf("/home/dev/app", 20)]);
    openRerailDatabase(path).$client.close();
    // An older Rerail recorded an audit 30 days after the clock this test moves, its provenance in its row, as when a
    // clock ran ahead while it recorded and was then set right. The week after it ends past the longest timer Node
    // keeps (2^31 - 1 ms, about 24.9 days); Node fires a longer one after 1 ms.
    const day = 24 * 3_600_000;
    let now = Date.now();
    const ahead = recordAsOlder(path, "older-ahead", provenanceOf("/home/dev/app", 20), now + 30 * day);
    const stop = new AbortController();
    // The timer the move waits on: it moves the clock by what it is asked, and ends the wait on its fifth pass.
    const asked: number[] = [];
    const timer = (ms: number, signal: AbortSignal | undefined): Promise<void> => {
      asked.push(ms);
      now += ms;
      if (asked.length === 5) stop.abort();
      return signal?.aborted === true ? Promise.reject(new Error("stopped")) : Promise.resolve();
    };
    const warnings: string[] = [];
    const warned = (warning: Error): void => {
      warnings.push(warning.name);
    };
    process.on("warning", warned);
    // A second on the real clock ends a move that does not take this test's timer.
    const deadline = setTimeout(() => stop.abort(), 1_000);
    const stderr = spyOnStderr();
    let logged = "";
    try {
      expect(await moveRuleProvenancesInWorker(path, stop.signal, { clock: () => now, timer })).toBe(0);
    } finally {
      logged = stderr.logged();
      stderr.restore();
      clearTimeout(deadline);
      process.off("warning", warned);
    }

    // One line, naming the audit and the end of the week after it, however many passes.
    const waits = logged.split("\n").filter(line => line.includes(WAITS_FOR_OLDER));
    expect(waits.length, "wait lines").toBe(1);
    expect(waits[0]).toContain(`waits until ${new Date(ahead + 7 * day).toISOString()}`);
    expect(warnings).toEqual([]);
    // Five passes of an hour each, the file and the clock read again after each; nothing moved.
    expect(asked).toEqual(Array.from({ length: 5 }, () => 3_600_000));
    expect(inlineAudits(path)).toBe(2);
  });

  it("fails with what ended its wait for an older Rerail when that was not a stop", async () => {
    const path = await fileOfTheBuildBefore();
    writeInlineAudits(path, [provenanceOf("/home/dev/app", 20)]);
    openRerailDatabase(path).$client.close();
    recordAsOlder(path, "older-beside", provenanceOf("/home/dev/app", 20));
    // The timer the wait sleeps on fails, and no stop came: the move fails with that, and the daemon logs it.
    const broken = new Error("the timer failed");
    const stderr = spyOnStderr();
    try {
      await expect(moveRuleProvenancesInWorker(path, new AbortController().signal, { timer: () => Promise.reject(broken) }))
        .rejects.toBe(broken);
    } finally {
      stderr.restore();
    }
    expect(inlineAudits(path)).toBe(2);
  });

  it("sizes each batch from how long the one before held the write lock", async () => {
    /** How many audits each batch of a move of 30 moved, timed by `now`. */
    const batchSizes = async (now: () => number): Promise<number[]> => {
      const path = await fileOfTheBuildBefore();
      writeInlineAudits(path, Array.from({ length: 30 }, () => provenanceOf("/home/dev/app", 20)));
      const left: number[] = [];
      const pause = (): Promise<void> => {
        left.push(inlineAudits(path));
        return Promise.resolve();
      };
      expect(await move(path, { now, pause })).toBe(30);
      return left.slice(1).map((count, index) => left[index]! - count);
    };

    // Each batch takes 200 ms by this clock, twice the 100 ms a batch aims for: the next one moves half as many audits,
    // and at least one.
    let clock = 0;
    expect(await batchSizes(() => (clock += 200))).toEqual([10, 5, 2, ...Array.from({ length: 13 }, () => 1)]);
    // Each batch takes no time: the next one moves twice as many.
    expect(await batchSizes(() => 0)).toEqual([10, 20]);
  });

  it("zeroes the pages the move frees, so the file keeps no copy of what it moved", async () => {
    const path = await fileOfTheBuildBefore();
    const marker = "/home/dev/zeroed-by-the-move";
    const provenance = provenanceOf(marker, 200);
    writeInlineAudits(path, Array.from({ length: 30 }, () => provenance));
    const occurrences = (text: string): number => text.split(marker).length - 1;

    // The move empties the WAL into the file and leaves the pages it freed there.
    expect(await move(path)).toBe(30);

    expect(pageBytes(path).free).toBeGreaterThan(0);
    expect(statSync(`${path}-wal`).size).toBe(0);
    // The stored copy's worth and less than a second copy: the 30 inline copies whose pages the move freed held 30
    // times as many.
    const perCopy = occurrences(JSON.stringify(provenance));
    const inFile = occurrences(readFileSync(path).toString("latin1"));
    expect(inFile).toBeGreaterThanOrEqual(perCopy / 2);
    expect(inFile).toBeLessThan(2 * perCopy);
  });

  it("leaves the pages it frees in the file for new history, which rerail scrub's rewrite gives back to the disk", async () => {
    const path = await fileOfTheBuildBefore();
    const app = provenanceOf("/home/dev/app", 200);
    const written = writeInlineAudits(path, Array.from({ length: 30 }, () => app));
    const before = statSync(path).size;

    expect(await move(path)).toBe(30);

    // No VACUUM: the file keeps the pages the move freed, more than half of it, and grows only by the shared copy it
    // stored before freeing any. Every audit reads its provenance as before.
    const moved = statSync(path).size;
    expect(moved).toBeGreaterThanOrEqual(before);
    expect(pageBytes(path).free).toBeGreaterThan(moved / 2);
    const store = new SqliteAuditStore(path);
    try {
      for (const { auditId, text } of written) expect(JSON.stringify(store.get(auditId)?.ruleProvenance), auditId).toBe(text);
      // New audits of the same rules fill the free pages, and the file keeps its size.
      for (let index = 0; index < 30; index += 1) store.finish(store.start(auditInput("/home/dev/app", app)), { status: "completed" });
    } finally {
      store.close();
    }
    expect(statSync(path).size).toBe(moved);

    // `rerail scrub` rewrites the file (`eraseFreedPages`): it shrinks to its pages in use, a small part of what it
    // was.
    const scrub = openRerailDatabase(path);
    try {
      expect(eraseFreedPages(scrub)).toBe(true);
    } finally {
      scrub.$client.close();
    }
    expect(pageBytes(path).free).toBe(0);
    expect(statSync(path).size).toBe(bytesInUse(path));
    expect(statSync(path).size).toBeLessThan(before / 5);
  });

  it("takes no write lock once nothing is left to move", async () => {
    const path = await fileOfTheBuildBefore();
    writeInlineAudits(path, [provenanceOf("/home/dev/app", 20)]);
    expect(await move(path)).toBe(1);

    // Another connection holds the write lock: a move that took it would wait out its 5 s busy timeout, then fail.
    const writer = new Database(path);
    try {
      writer.exec("BEGIN IMMEDIATE");
      expect(await move(path)).toBe(0);
      writer.exec("ROLLBACK");
    } finally {
      writer.close();
    }
  }, 15_000);

  it("leaves an audit whose provenance in its row is not a JSON array as it is, moves every other, and says so once", async () => {
    const path = await fileOfTheBuildBefore();
    const written = writeInlineAudits(path, Array.from({ length: 600 }, () => provenanceOf("/home/dev/app", 20)));
    const damaged = "older-299";
    damageProvenance(path, damaged, '[{"ruleId":"typescript/broken","revision":3');
    openRerailDatabase(path).$client.close();
    const before = readOutcome(() => readAsOlder(path, [damaged])[0]?.ruleProvenance);
    const stderr = spyOnStderr();
    let logged = "";
    try {
      expect(await move(path)).toBe(599);
      // Each later run finds nothing left to move, on its calling thread and on the move's own, and says nothing.
      expect(await move(path)).toBe(0);
      expect(await moveRuleProvenancesInWorker(path)).toBe(0);
    } finally {
      logged = stderr.logged();
      stderr.restore();
    }

    // One line naming the audit, with no stack. A batch that failed on it ended the move there at every start.
    expect(logged).toBe(`rerail: ${path}: moving the rule provenance of audits recorded before this Rerail's upgrade `
      + `leaves audit ${damaged} as it is: the provenance in its row is not a JSON array, so no stored copy can hold it, `
      + "and the audit reads as it did.\n");
    expect(auditsNamingNoCopy(path)).toBe(1);
    expect(inlineAudits(path)).toBe(1);
    // The build before fails on it, before the move and after. This build reads it with no provenance and a failure
    // that says so, and every other audit as written.
    expect(before).toMatch(/^SyntaxError: /u);
    const quiet = spyOnStderr();
    const store = new SqliteAuditStore(path);
    try {
      expect(store.get(damaged)).toMatchObject({ ruleProvenance: [], failures: [UNREADABLE_PROVENANCE] });
      for (const { auditId, text } of written.filter(({ auditId }) => auditId !== damaged)) {
        expect(JSON.stringify(store.get(auditId)?.ruleProvenance), auditId).toBe(text);
      }
    } finally {
      store.close();
      quiet.restore();
    }
    expect(readOutcome(() => readAsOlder(path, [damaged])[0]?.ruleProvenance)).toBe(before);
  }, 60_000);

  it("reads no row of the audits an older Rerail recorded after the step, in its last batch or on its calling thread once nothing is left", async () => {
    const path = await fileOfTheBuildBefore();
    writeInlineAudits(path, Array.from({ length: 20 }, () => provenanceOf("/home/dev/app", 20)));
    openRerailDatabase(path).$client.close();
    // An older Rerail sharing the file then records 2,000 audits, each with its provenance in its row: about 200 KB,
    // with the audit's start after it.
    const text = JSON.stringify(provenanceOf("/home/dev/app", 700));
    expect(text.length).toBeGreaterThan(190_000);
    recordMany(path, { prefix: "older-after", count: 2_000, from: stepAppliedAt(path) + 1, text });
    // Eight days on, so the move does not wait for them.
    const clock = (): number => Date.now() + 8 * 24 * 3_600_000;

    // What the move's reads of audits take, its batches and the checks before each: reading the start of each of those
    // 2,000 rows took 125 ms in one batch, under its write lock, on Linux and 470 ms on Windows.
    const moving = timeAuditReads();
    try {
      expect(await move(path, { clock })).toBe(20);
    } finally {
      moving.restore();
    }
    expect(moving.totalMs()).toBeLessThan(25);
    expect(inlineAudits(path)).toBe(2_000);

    // The older Rerail goes on recording: its next audit begins an hour before the clock reads, so a start that found an
    // audit left to move would wait a week for it. A later start finds nothing left to move: it neither waits nor starts
    // a thread, and says nothing. A check that read past the last audit the step recorded would find the older Rerail's
    // audits. On the calling thread, the daemon's, which answers hooks, reading the start of each of those rows took 18
    // to 1,134 ms on a real daemon's start.
    recordAsOlder(path, "older-later", provenanceOf("/home/dev/app", 20), clock() - 3_600_000);
    const later = await laterStart(path, clock);
    expect(later.moved).toBe(0);
    expect(later.waits).toEqual([]);
    expect(later.logged).not.toContain(WAITS_FOR_OLDER);
    expect(later.readMs).toBeLessThan(20);
  }, 120_000);

  it("reads one audit row on its calling thread while it waits for an older Rerail, however many audits this build recorded since", async () => {
    const path = await fileOfTheBuildBefore();
    writeInlineAudits(path, Array.from({ length: 20 }, () => provenanceOf("/home/dev/app", 20)));
    openRerailDatabase(path).$client.close();
    // An older Rerail sharing the file records an audit; this build records 50,000 after it, each without rules.
    const at = recordAsOlder(path, "older-beside", provenanceOf("/home/dev/app", 20));
    recordMany(path, { prefix: "ruleless", count: 50_000, from: at + 1, text: "[]", provenanceId: storedEmptyProvenance(path) });
    const stop = new AbortController();
    // The timer the wait sleeps on ends the wait on its first pass.
    const timer = (): Promise<void> => {
      stop.abort();
      return Promise.reject(new Error("stopped"));
    };
    const stderr = spyOnStderr();
    const timed = timeAuditReads();
    let logged = "";
    try {
      expect(await moveRuleProvenancesInWorker(path, stop.signal, { timer })).toBe(0);
    } finally {
      timed.restore();
      logged = stderr.logged();
      stderr.restore();
    }

    expect(logged).toContain(`began at ${new Date(at).toISOString()}.`);
    // Reading the start of every audit in the week, and the row of each that began after the older Rerail's, took
    // 69 ms on Linux and 195 ms on Windows.
    expect(timed.totalMs()).toBeLessThan(20);
    expect(inlineAudits(path)).toBe(21);
  }, 60_000);

  it("moves every audit recorded before the step, one begun after the time the step recorded among them, and reads one audit row a check on its calling thread", async () => {
    const path = await fileOfTheBuildBefore();
    const written = writeInlineAudits(path, Array.from({ length: 30 }, () => provenanceOf("/home/dev/app", 20)));
    // This build upgrades the file a day on. The clock went back between the audits and that open, as an NTP
    // correction sets it: the 16th audit recorded before the step began an hour after the time the step recorded.
    const day = 24 * 3_600_000;
    upgradeAt(path, day);
    beganAt(path, "older-15", day + 3_600_000);
    // An older Rerail sharing the file then records an audit its clock dates before the step, and 2,000 more, each with
    // its provenance in its row: about 200 KB, with the audit's start after it.
    const behind = provenanceOf("/home/dev/app-behind", 20);
    recordAsOlder(path, "older-behind", behind, day - 1);
    const text = JSON.stringify(provenanceOf("/home/dev/app", 700));
    recordMany(path, { prefix: "older-after", count: 2_000, from: day + 1, text });
    // Nine days on, so the move does not wait for them.
    const clock = (): number => 10 * day;

    // The move runs on a thread of its own; on the calling thread, the daemon's, which answers hooks, the start's check
    // and the check once the move's thread ends each read one audit row at most. Bounded by the time the step recorded
    // alone, the move would stop at the 16th: it and the 14 after it would keep their copy for good.
    const moving = timeAuditReads();
    try {
      expect(await moveRuleProvenancesInWorker(path, undefined, { clock })).toBe(30);
    } finally {
      moving.restore();
    }
    expect(moving.totalMs()).toBeLessThan(20);
    // Every audit recorded before the step reads as written. The older Rerail's stay in their rows, where it reads
    // them, the one its clock dated before the step included.
    expect(inlineAudits(path)).toBe(2_001);
    const store = new SqliteAuditStore(path);
    try {
      for (const audit of written) expect(JSON.stringify(store.get(audit.auditId)?.ruleProvenance), audit.auditId).toBe(audit.text);
    } finally {
      store.close();
    }
    expect(provenanceTexts(readAsOlder(path, ["older-behind"]))).toEqual([JSON.stringify(behind)]);

    // The older Rerail goes on recording: its next audit begins an hour before the clock reads, so a start that found an
    // audit left to move would wait a week for it. A later start finds nothing left to move: it neither waits nor starts
    // a thread, and says nothing. A check that read past the last audit the step recorded would find the older Rerail's
    // audits, the one its clock dated before the step first.
    recordAsOlder(path, "older-later", behind, clock() - 3_600_000);
    const later = await laterStart(path, clock);
    expect(later.moved).toBe(0);
    expect(later.waits).toEqual([]);
    expect(later.logged).not.toContain(WAITS_FOR_OLDER);
    expect(later.readMs).toBeLessThan(20);
  }, 120_000);

  it.each([
    ["that ran the step before the step recorded it", "DELETE FROM audit_rule_provenance_step", 30],
    ["whose recorded audit a reset of its repository's history deleted", "DELETE FROM audits WHERE audit_id = 'older-29'", 29],
  ])("takes the audits up to the first that began after the step, as before, and a later start finds none left, in a file %s", async (_, forget, recorded) => {
    const path = await fileOfTheBuildBefore();
    const app = provenanceOf("/home/dev/app", 20);
    writeInlineAudits(path, Array.from({ length: 30 }, () => app));
    const day = 24 * 3_600_000;
    upgradeAt(path, day);
    rewrite(path, forget);
    // An older Rerail sharing the file records an audit a millisecond after the time the step recorded.
    recordAsOlder(path, "older-after", app, day + 1);

    expect(await move(path, { clock: () => 10 * day })).toBe(recorded);
    expect(auditsNamingNoCopy(path)).toBe(1);
    expect(provenanceTexts(readAsOlder(path, ["older-after"]))).toEqual([JSON.stringify(app)]);

    // The older Rerail records its next audit an hour before the clock reads, so a start that found an audit left to
    // move would wait a week for it. A later start stops at the first audit that began after the step, and finds none.
    recordAsOlder(path, "older-later", app, 10 * day - 3_600_000);
    const later = await laterStart(path, () => 10 * day);
    expect(later.moved).toBe(0);
    expect(later.waits).toEqual([]);
    expect(later.logged).not.toContain(WAITS_FOR_OLDER);
  });

  it("ends at once, moving nothing, when a stop comes before it opens its writer while another connection holds the write lock", async () => {
    const path = await fileOfTheBuildBefore();
    writeInlineAudits(path, Array.from({ length: 30 }, () => provenanceOf("/home/dev/app", 20)));
    openRerailDatabase(path).$client.close();
    const writer = new Database(path);
    writer.exec("BEGIN IMMEDIATE");
    const stop = new AbortController();
    stop.abort();
    let tookMs: number;
    try {
      const began = performance.now();
      expect(await move(path, { signal: stop.signal })).toBe(0);
      tookMs = performance.now() - began;
    } finally {
      writer.exec("ROLLBACK");
      writer.close();
    }
    // A writer opened as `openRerailDatabase` opens one begins a write transaction, which waited inside SQLite for the
    // lock 5 s, where no stop reaches it, and then failed.
    expect(tookMs).toBeLessThan(2_000);
    expect(inlineAudits(path)).toBe(30);
  }, 15_000);

  it("moves on a thread of its own, which a stop ends before its first batch, and a later run moves the rest", async () => {
    const path = await fileOfTheBuildBefore();
    const written = writeInlineAudits(path, Array.from({ length: 30 }, () => provenanceOf("/home/dev/app", 200)));

    const stop = new AbortController();
    const stopped = moveRuleProvenancesInWorker(path, stop.signal);
    stop.abort();
    expect(await stopped).toBe(0);
    expect(inlineAudits(path)).toBe(30);

    expect(await moveRuleProvenancesInWorker(path)).toBe(30);
    expect(inlineAudits(path)).toBe(0);
    const store = new SqliteAuditStore(path);
    try {
      for (const { auditId, text } of written) expect(JSON.stringify(store.get(auditId)?.ruleProvenance), auditId).toBe(text);
    } finally {
      store.close();
    }
    // Nothing is left: the next run reads that and starts no thread.
    expect(await moveRuleProvenancesInWorker(path)).toBe(0);
  }, 60_000);

  it("ends with what it moved once a stop comes while a batch waits for another connection's write lock, and a later run moves the rest", async () => {
    const path = await fileOfTheBuildBefore();
    const written = writeInlineAudits(path, Array.from({ length: 30 }, () => provenanceOf("/home/dev/app", 200)));
    const writer = new Database(path);
    const stop = new AbortController();
    let pauses = 0;
    try {
      // After the first batch another connection takes the write lock, so the second finds it held, and the stop comes
      // in the pause before that batch's next try.
      const moved = await move(path, {
        perBatch: 10,
        signal: stop.signal,
        pause: () => {
          pauses += 1;
          if (pauses === 2) writer.exec("BEGIN IMMEDIATE");
          if (pauses === 3) stop.abort();
          return Promise.resolve();
        },
      });
      expect(moved).toBe(10);
      expect(pauses).toBe(3);
      expect(inlineAudits(path)).toBe(20);
    } finally {
      if (writer.inTransaction) writer.exec("ROLLBACK");
      writer.close();
    }

    expect(await move(path)).toBe(20);
    const store = new SqliteAuditStore(path);
    try {
      for (const { auditId, text } of written) expect(JSON.stringify(store.get(auditId)?.ruleProvenance), auditId).toBe(text);
    } finally {
      store.close();
    }
  });

  it("waits as long as another connection holds the write lock before a batch, then moves the rest", async () => {
    const path = await fileOfTheBuildBefore();
    const written = writeInlineAudits(path, Array.from({ length: 30 }, () => provenanceOf("/home/dev/app", 20)));
    const writer = new Database(path);
    let pauses = 0;
    try {
      // After the first batch another connection takes the write lock and holds it for 100 of the move's pauses, past
      // the 5 s every Rerail connection waits for it, as `rerail scrub` holds it 6.8 to 17.7 s on a large file.
      const moved = await move(path, {
        perBatch: 10,
        pause: () => {
          pauses += 1;
          if (pauses === 2) writer.exec("BEGIN IMMEDIATE");
          if (pauses === 102) writer.exec("ROLLBACK");
          return Promise.resolve();
        },
      });
      expect(moved).toBe(30);
      expect(pauses).toBeGreaterThan(102);
    } finally {
      if (writer.inTransaction) writer.exec("ROLLBACK");
      writer.close();
    }

    expect(inlineAudits(path)).toBe(0);
    const store = new SqliteAuditStore(path);
    try {
      for (const { auditId, text } of written) expect(JSON.stringify(store.get(auditId)?.ruleProvenance), auditId).toBe(text);
    } finally {
      store.close();
    }
  });

  it("pauses as for a busy batch while SQLite answers the check before a batch busy, then moves the rest, failing nothing", async () => {
    const path = await fileOfTheBuildBefore();
    const written = writeInlineAudits(path, Array.from({ length: 30 }, () => provenanceOf("/home/dev/app", 20)));
    openRerailDatabase(path).$client.close();
    const checks = busyChecks();
    let pauses = 0;
    let moved: number;
    try {
      // After the first batch, SQLite answers the check before each batch busy for 10 of the move's pauses, as it
      // answers a read while another process recovers the WAL.
      moved = await move(path, {
        perBatch: 10,
        pause: () => {
          pauses += 1;
          if (pauses === 2) checks.setBusy(true);
          if (pauses === 12) checks.setBusy(false);
          return Promise.resolve();
        },
      });
    } finally {
      checks.restore();
    }

    // The move went on after those pauses. Read outside the batch's handling of a busy answer, the first one ended the
    // move, and the daemon logged that as a failure.
    expect(moved).toBe(30);
    expect(checks.answered()).toBe(10);
    expect(inlineAudits(path)).toBe(0);
    const store = new SqliteAuditStore(path);
    try {
      for (const { auditId, text } of written) expect(JSON.stringify(store.get(auditId)?.ruleProvenance), auditId).toBe(text);
    } finally {
      store.close();
    }
  });

  it("ends at once with what it moved when a stop comes while SQLite answers the check before a batch busy, and a later run moves the rest", async () => {
    const path = await fileOfTheBuildBefore();
    writeInlineAudits(path, Array.from({ length: 30 }, () => provenanceOf("/home/dev/app", 20)));
    openRerailDatabase(path).$client.close();
    const checks = busyChecks();
    const stop = new AbortController();
    let pauses = 0;
    let moved: number;
    let ended = false;
    // Whether the move had ended by the time the event loop could run anything after the stop: a timer, I/O or the
    // immediate that reads it. Ending at once is ending within the stop's turn, and that holds however busy the machine
    // is; a bound in milliseconds also timed the close of the move's connection and the machine's load.
    const endedAtStop = Promise.withResolvers<boolean>();
    try {
      // The move's own pause, 150 ms, which a stop ends. After the first batch the check before each batch is answered
      // busy; the stop comes while the third pause that follows runs, so a move that waited out that pause, or anything
      // else, would not have ended in the stop's turn.
      moved = await move(path, {
        perBatch: 10,
        signal: stop.signal,
        pause: async () => {
          pauses += 1;
          if (pauses === 2) checks.setBusy(true);
          if (pauses === 4) {
            setImmediate(() => {
              stop.abort();
              setImmediate(() => endedAtStop.resolve(ended));
            });
          }
          await sleep(150, undefined, { signal: stop.signal }).catch(() => undefined);
        },
      }).finally(() => {
        ended = true;
      });
    } finally {
      checks.restore();
    }

    expect(moved).toBe(10);
    expect(pauses).toBe(4);
    expect(checks.answered()).toBe(2);
    expect(await endedAtStop.promise).toBe(true);
    expect(inlineAudits(path)).toBe(20);
    expect(await move(path)).toBe(20);
  });

  it("empties the WAL between tries while another connection reads, never waiting inside SQLite, so its thread keeps running", async () => {
    const path = await fileOfTheBuildBefore();
    writeInlineAudits(path, Array.from({ length: 30 }, () => provenanceOf("/home/dev/app", 200)));
    // A read begun before the upgrade and held through the move: the checkpoint after the last batch finds it on every
    // try, and leaves the WAL to SQLite's later checkpoints.
    const reader = new Database(path, { readonly: true });
    reader.exec("BEGIN");
    reader.prepare("SELECT count(*) FROM audits").get();
    let longest: number;
    try {
      const stalled = await withLongestStall(() => move(path, { pause: () => new Promise(resolve => setTimeout(resolve, 20)) }));
      expect(stalled.result).toBe(30);
      longest = stalled.longest;
      expect(statSync(`${path}-wal`).size).toBeGreaterThan(0);
    } finally {
      reader.close();
    }
    // The thread running the move takes a stop between two tries, and the daemon's close waits for that. Waiting inside
    // SQLite, the checkpoint held the thread for its 5 s busy timeout.
    expect(longest).toBeLessThan(2_000);
    expect(inlineAudits(path)).toBe(0);
  }, 30_000);

  it("leaves nothing of a repository's provenance in the file once its history is reset", async () => {
    const path = await temporaryDatabase();
    const marker = "/checkouts/reset-marker";
    const store = new SqliteAuditStore(path);
    const database = openRerailDatabase(path);
    try {
      const insertSession = database.$client.prepare(
        "INSERT INTO sessions (session_id, host, cwd, repository_id, started_at, ended_at) VALUES (?, 'codex', '/w', ?, 1, 2)",
      );
      insertSession.run("session-reset", "remote:example/reset");
      insertSession.run("session-kept", "remote:example/kept");
      const reset = provenanceOf(marker, 50);
      const kept = provenanceOf("/checkouts/kept", 50);
      for (let index = 0; index < 3; index += 1) {
        store.finish(store.start(auditInput(marker, reset, "session-reset")), { status: "completed" });
      }
      const keptAudit = store.start(auditInput("/checkouts/kept", kept, "session-kept"));
      store.finish(keptAudit, { status: "completed" });
    } finally {
      store.close();
      database.$client.close();
    }
    expect(readFileSync(path).toString("latin1")).toContain(marker);

    resetRepositoryHistory(path, "remote:example/reset");

    expect(readFileSync(path).toString("latin1")).not.toContain(marker);
    const reader = new SqliteAuditStore(path);
    try {
      expect(reader.list().audits.map(audit => audit.ruleProvenance)).toEqual([provenanceOf("/checkouts/kept", 50)]);
    } finally {
      reader.close();
    }
  });

  it("keeps a stored provenance while another repository's audit names it, and drops it with the last one", async () => {
    const path = await temporaryDatabase();
    // A session of another repository that audits an edit in this checkout records the same provenance.
    const shared = provenanceOf("/checkouts/shared", 50);
    const store = new SqliteAuditStore(path);
    const database = openRerailDatabase(path);
    const kept: string[] = [];
    try {
      const insertSession = database.$client.prepare(
        "INSERT INTO sessions (session_id, host, cwd, repository_id, started_at, ended_at) VALUES (?, 'codex', '/w', ?, 1, 2)",
      );
      insertSession.run("session-reset", "remote:example/reset");
      insertSession.run("session-kept", "remote:example/kept");
      store.finish(store.start(auditInput("/checkouts/shared", shared, "session-reset")), { status: "completed" });
      for (let index = 0; index < 2; index += 1) {
        const auditId = store.start(auditInput("/checkouts/shared", shared, "session-kept"));
        store.finish(auditId, { status: "completed" });
        kept.push(auditId);
      }
    } finally {
      store.close();
      database.$client.close();
    }
    const storedCopies = (): number => {
      const reader = new Database(path, { readonly: true });
      try {
        return reader.prepare("SELECT count(*) FROM audit_rule_provenances").pluck().get() as number;
      } finally {
        reader.close();
      }
    };
    expect(storedCopies()).toBe(1);

    resetRepositoryHistory(path, "remote:example/reset");

    // The reset repository's audit named the copy too; the other repository's audits still name it.
    expect(storedCopies()).toBe(1);
    const reader = new SqliteAuditStore(path);
    try {
      for (const auditId of kept) expect(reader.get(auditId)?.ruleProvenance, auditId).toEqual(shared);
      expect(reader.list().audits.map(audit => audit.ruleProvenance)).toEqual([shared, shared]);
    } finally {
      reader.close();
    }

    resetRepositoryHistory(path, "remote:example/kept");
    expect(storedCopies()).toBe(0);
  });
});

/**
 * The failure an audit lists after its own when the rule provenance the database holds for it is not a JSON array, so
 * it reads as none.
 */
const UNREADABLE_PROVENANCE = {
  clauseId: "rule-provenance", code: "adapter-error", stage: "persistence",
  message: "The audit's rule provenance in the database is not a JSON array, which only a damaged file or a hand edit leaves, so it reads as none.",
};

/**
 * Six finished audits this build records, oldest first: three of one checkout's rules, then three of another's, each
 * naming the stored copy of its provenance.
 */
function sixAudits(path: string): { auditIds: string[]; expected: Map<string, AuditRuleProvenance[]> } {
  const first = provenanceOf("/home/dev/app", 3);
  const second = provenanceOf("/home/dev/app-worktree", 4);
  const store = new SqliteAuditStore(path);
  try {
    const recorded = [first, first, first, second, second, second].map(provenance => {
      const auditId = store.start(auditInput("/home/dev/app", provenance));
      store.finish(auditId, { status: "completed" });
      return [auditId, provenance] as const;
    });
    return { auditIds: recorded.map(([auditId]) => auditId), expected: new Map(recorded) };
  } finally {
    store.close();
  }
}

/**
 * Writes `text` over an audit's own provenance and makes it name no stored copy, past the CHECK on `audits`, as the
 * issue's steps do by hand: only a damaged file or a hand edit leaves such text.
 */
function damageAuditRow(path: string, auditId: string, text: string): void {
  const database = new Database(path);
  try {
    database.pragma("ignore_check_constraints = ON");
    database.prepare("UPDATE audits SET rule_provenance_json = ?, rule_provenance_id = NULL WHERE audit_id = ?").run(text, auditId);
  } finally {
    database.close();
  }
}

/** Writes `text` over the stored copy of the provenance `auditId` names, past its CHECK; the copy's id. */
function damageStoredCopy(path: string, auditId: string, text: string): number {
  const database = new Database(path);
  try {
    database.pragma("ignore_check_constraints = ON");
    const copy = database.prepare("SELECT rule_provenance_id FROM audits WHERE audit_id = ?").pluck().get(auditId) as number;
    database.prepare("UPDATE audit_rule_provenances SET rule_provenance_json = ? WHERE provenance_id = ?").run(text, copy);
    return copy;
  } finally {
    database.close();
  }
}

/** The message `JSON.parse` fails with on `text`, as this Node writes it. */
function parseFailure(text: string): string {
  try {
    JSON.parse(text);
    return "parsed";
  } catch (error) {
    return (error as Error).message;
  }
}

describe("an audit whose rule provenance does not read", () => {
  it("lists every audit of each page that holds one whose provenance in its row does not parse, that one with no provenance and a failure that says so", async () => {
    const path = await temporaryDatabase();
    const { auditIds, expected } = sixAudits(path);
    const damaged = auditIds[4]!;
    damageAuditRow(path, damaged, '[{"ruleId":"typescript/broken","revision":3');
    expected.set(damaged, []);
    const stderr = spyOnStderr();
    const store = new SqliteAuditStore(path);
    const reader = new SqliteAuditReader(path);
    try {
      const listed = store.list({ limit: 200 }).audits;
      expect(new Map(listed.map(audit => [audit.auditId, audit.ruleProvenance]))).toEqual(expected);
      const record = listed.find(audit => audit.auditId === damaged);
      expect(record).toMatchObject({ failures: [UNREADABLE_PROVENANCE], failureCount: 1, unlistedFailureCount: 0, blindFiles: [] });
      expect(listed.filter(audit => audit.failures.length > 0)).toHaveLength(1);
      // Each page of two answers, the one holding it among them; together they hold every audit.
      const paged = [0, 2, 4].flatMap(offset => store.list({ limit: 2, offset }).audits.map(audit => audit.auditId));
      expect(paged.sort()).toEqual([...auditIds].sort());
      expect(store.get(damaged)).toEqual(record);
      expect(store.get(auditIds[0]!)?.ruleProvenance).toEqual(expected.get(auditIds[0]!));
      // The read-only reader the remote outbox and the support bundle read through reads the same.
      expect(reader.list({ limit: 200 }).audits).toEqual(listed);
      expect(reader.get(damaged)).toEqual(record);
    } finally {
      reader.close();
      store.close();
      stderr.restore();
    }
  });

  it("reads every audit that names a stored copy whose text does not parse with no provenance and a failure that says so, and every other as stored", async () => {
    const path = await temporaryDatabase();
    const { auditIds, expected } = sixAudits(path);
    damageStoredCopy(path, auditIds[0]!, '[{"ruleId":"typescript/broken"');
    for (const auditId of auditIds.slice(0, 3)) expected.set(auditId, []);
    const stderr = spyOnStderr();
    const store = new SqliteAuditStore(path);
    try {
      const listed = store.list({ limit: 200 }).audits;
      expect(new Map(listed.map(audit => [audit.auditId, audit.ruleProvenance]))).toEqual(expected);
      expect(new Map(listed.map(audit => [audit.auditId, audit.failures]))).toEqual(new Map(auditIds.map((auditId, index) => [
        auditId, index < 3 ? [UNREADABLE_PROVENANCE] : [],
      ])));
      expect(store.get(auditIds[1]!)?.failures).toEqual([UNREADABLE_PROVENANCE]);
    } finally {
      store.close();
      stderr.restore();
    }
  });

  it.each(['{"ruleId":"typescript/broken"}', "null", '"[]"', "3"])(
    "reads provenance that parses as %s, which is no JSON array, as none, with a failure that says so",
    async text => {
      const path = await temporaryDatabase();
      const { auditIds } = sixAudits(path);
      damageAuditRow(path, auditIds[2]!, text);
      const stderr = spyOnStderr();
      const store = new SqliteAuditStore(path);
      try {
        expect(store.get(auditIds[2]!)).toMatchObject({ ruleProvenance: [], failures: [UNREADABLE_PROVENANCE], failureCount: 1 });
      } finally {
        store.close();
        stderr.restore();
      }
    },
  );

  it("names in its log, once for each damaged text, the audit that read it with no provenance and where that text is", async () => {
    const path = await temporaryDatabase();
    const { auditIds } = sixAudits(path);
    const text = '[{"ruleId":"typescript/broken","revision":3';
    damageAuditRow(path, auditIds[4]!, text);
    const copy = damageStoredCopy(path, auditIds[0]!, "{}");
    const stderr = spyOnStderr();
    const store = new SqliteAuditStore(path);
    const reader = new SqliteAuditReader(path);
    let logged = "";
    try {
      store.get(auditIds[1]!);
      store.list({ limit: 200 });
      // Later reads of either text, on this connection or another, say nothing more.
      store.list({ limit: 200 });
      store.get(auditIds[4]!);
      reader.list({ limit: 200 });
    } finally {
      logged = stderr.logged();
      reader.close();
      store.close();
      stderr.restore();
    }
    expect(logged).toBe(
      `rerail: ${path}: audit ${auditIds[1]} reads with no rule provenance, and a failure that says so: rule_provenance_json `
      + `of audit_rule_provenances row ${copy}, which every audit naming it reads, is not a JSON array (it parses to another JSON value).\n`
      + `rerail: ${path}: audit ${auditIds[4]} reads with no rule provenance, and a failure that says so: the rule_provenance_json `
      + `of its own row is not a JSON array (${parseFailure(text)}).\n`,
    );
  });
});

/** Each other list an `audits` row holds, by its column, and what the failure an audit lists for it calls it. */
const STORED_LISTS = {
  ruleRevisions: { column: "rule_revisions_json", clauseId: "rule-revisions", name: "rule revisions" },
  paths: { column: "paths_json", clauseId: "paths", name: "paths" },
  violations: { column: "violations_json", clauseId: "violations", name: "violations" },
  failures: { column: "failures_json", clauseId: "failures", name: "failures" },
} as const;
type StoredList = keyof typeof STORED_LISTS;
const LISTS = Object.keys(STORED_LISTS) as StoredList[];

/** The failure an audit lists after its own when the text the database holds for its `list` is not a JSON array. */
function unreadableList(list: StoredList): AuditFailure {
  return {
    clauseId: STORED_LISTS[list].clauseId, code: "adapter-error", stage: "persistence",
    message: `The audit's ${STORED_LISTS[list].name} in the database are not a JSON array, which only a damaged file or a hand edit leaves, so they read as none.`,
  };
}

const VIOLATION = {
  id: "violation-1", ruleId: "typescript/rule-000", clauseId: "clause-a", code: "forbidden-call", mode: "steer" as const,
  message: "forbidden call", path: "src/a.ts", captures: {}, variables: {},
};
const FAILURE: AuditFailure = { clauseId: "clause-b", code: "adapter-error", message: "failed", stage: "evaluate" };

/** Three finished audits of one session, each with a rule revision, a path, a violation and a failure; their ids. */
function threeAudits(path: string): string[] {
  const store = new SqliteAuditStore(path);
  try {
    return [0, 1, 2].map(() => {
      const auditId = store.start({ ...auditInput("/home/dev/app", provenanceOf("/home/dev/app", 2)), ruleRevisions: [{ ruleId: "typescript/rule-000", revision: 1 }] });
      store.finish(auditId, { status: "completed", violations: [VIOLATION], failures: [FAILURE] });
      return auditId;
    });
  } finally {
    store.close();
  }
}

/**
 * Writes `text` over one list of an audit's row, past the CHECK on `audits`. The history triggers parse the rule
 * revisions and the violations on UPDATE and refuse such text, so those that name the column are set aside for the
 * write and put back: the state a damaged file leaves.
 */
function damageList(path: string, auditId: string, list: StoredList, text: string): void {
  const { column } = STORED_LISTS[list];
  const database = new Database(path);
  try {
    database.pragma("ignore_check_constraints = ON");
    const triggers = database.prepare("SELECT name, sql FROM sqlite_master WHERE type = 'trigger' AND tbl_name = 'audits' AND sql LIKE ?")
      .all(`%${column}%`) as Array<{ name: string; sql: string }>;
    database.transaction(() => {
      for (const trigger of triggers) database.exec(`DROP TRIGGER "${trigger.name}"`);
      database.prepare(`UPDATE audits SET ${column} = ? WHERE audit_id = ?`).run(text, auditId);
      for (const trigger of triggers) database.exec(trigger.sql);
    })();
  } finally {
    database.close();
  }
}

/** The audit `auditId` as a store on `path` reads it. */
function readAudit(path: string, auditId: string): AuditRecord | undefined {
  const store = new SqliteAuditStore(path);
  try {
    return store.get(auditId);
  } finally {
    store.close();
  }
}

describe("an audit whose other stored lists do not read", () => {
  it.each(LISTS)("lists every audit of each page that holds one whose %s do not parse, that one with none and a failure that says so", async list => {
    const path = await temporaryDatabase();
    const auditIds = threeAudits(path);
    const before = readAudit(path, auditIds[1]!)!;
    const healthy = readAudit(path, auditIds[0]!);
    damageList(path, auditIds[1]!, list, '[{"ruleId":"typescript/broken"');
    const failures = list === "failures" ? [unreadableList(list)] : [FAILURE, unreadableList(list)];
    const stderr = spyOnStderr();
    const store = new SqliteAuditStore(path);
    const reader = new SqliteAuditReader(path);
    try {
      const listed = store.list({ limit: 200 }).audits;
      expect(listed.map(audit => audit.auditId).sort()).toEqual([...auditIds].sort());
      const record = listed.find(audit => audit.auditId === auditIds[1]);
      expect(record).toEqual({ ...before, [list]: list === "failures" ? failures : [], failures, failureCount: failures.length });
      expect(listed.find(audit => audit.auditId === auditIds[0])).toEqual(healthy);
      expect(store.get(auditIds[1]!)).toEqual(record);
      expect(reader.list({ limit: 200 }).audits).toEqual(listed);
      expect(reader.get(auditIds[1]!)).toEqual(record);
    } finally {
      reader.close();
      store.close();
      stderr.restore();
    }
  });

  // An object with a member, since SQLite's JSON functions walk an object's members as they walk an array's items.
  const BROKEN = '[{"ruleId":"typescript/broken"';
  const OBJECT = '{"blind":"budget"}';
  it.each([
    ["ruleRevisions", BROKEN], ["ruleProvenance", BROKEN], ["paths", OBJECT], ["violations", BROKEN], ["violations", OBJECT],
    ["failures", BROKEN], ["failures", OBJECT],
  ] as const)("answers the session's summary when its latest audit's %s read as %s, with the counts of its record", async (list, text) => {
    const path = await temporaryDatabase();
    threeAudits(path);
    const latest = new SqliteAuditStore(path);
    const latestId = latest.list({ limit: 1 }).audits[0]!.auditId;
    latest.close();
    const healthy = { workspaceRoot: "/home/dev/app", scope: "touched-files", mode: "steer", status: "completed", violationCount: 1,
      failureCount: 1, unlistedFailureCount: 0, blindFileCount: 0, gapCount: 0, outOfWorkRuleCount: 0, fileWorkStoppedRuleCount: 0 };
    if (list === "ruleProvenance") damageAuditRow(path, latestId, text);
    else damageList(path, latestId, list, text);
    const stderr = spyOnStderr();
    const store = new SqliteAuditStore(path);
    const reader = new SqliteAuditReader(path);
    try {
      const record = store.get(latestId)!;
      expect(record.failureCount).toBe(list === "failures" ? 1 : 2);
      expect(store.latestSummary("session-a")).toEqual({ ...healthy, violationCount: record.violations.length, failureCount: record.failureCount });
      expect(reader.latestSummary("session-a")).toEqual(store.latestSummary("session-a"));
    } finally {
      reader.close();
      store.close();
      stderr.restore();
    }
  });

  it("names in its log, once for each damaged list, two of one audit among them, the audit that read it with none and the column that holds it", async () => {
    const path = await temporaryDatabase();
    const auditIds = threeAudits(path);
    const text = '[{"ruleId":"typescript/broken"';
    damageList(path, auditIds[0]!, "paths", text);
    damageList(path, auditIds[0]!, "failures", "null");
    damageList(path, auditIds[1]!, "violations", "{}");
    const stderr = spyOnStderr();
    const store = new SqliteAuditStore(path);
    const reader = new SqliteAuditReader(path);
    let logged = "";
    try {
      store.list({ limit: 200 });
      store.list({ limit: 200 });
      store.get(auditIds[0]!);
      reader.list({ limit: 200 });
    } finally {
      logged = stderr.logged();
      reader.close();
      store.close();
      stderr.restore();
    }
    expect(logged.split("\n").sort()).toEqual([
      "",
      `rerail: ${path}: audit ${auditIds[0]} reads with no paths, and a failure that says so: the paths_json of its own row is `
      + `not a JSON array (${parseFailure(text)}).`,
      `rerail: ${path}: audit ${auditIds[0]} reads with no failures, and a failure that says so: the failures_json of its `
      + "own row is not a JSON array (it parses to another JSON value).",
      `rerail: ${path}: audit ${auditIds[1]} reads with no violations, and a failure that says so: the violations_json of its `
      + "own row is not a JSON array (it parses to another JSON value).",
    ].sort());
  });
});
