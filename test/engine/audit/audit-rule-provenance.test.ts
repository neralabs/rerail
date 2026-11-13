import type * as ChildProcessModule from "node:child_process";
import type { SpawnOptions } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, readFileSync, statfsSync, statSync } from "node:fs";
import { mkdir, mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import Database from "better-sqlite3";
import { afterEach, describe, expect, it, vi } from "vitest";
import { SqliteAuditStore } from "../../../src/engine/audit/audit-store.js";
import type {
  AuditInput, AuditRecord, AuditRuleProvenance, LegacyAuditRuleProvenance, RepositoryAuditRuleProvenance,
} from "../../../src/engine/audit/audit.js";
import { openRerailDatabase, resetRepositoryHistory } from "../../../src/engine/storage/database.js";
import { RERAIL_BASELINE_SCHEMA_STATEMENTS } from "../../../src/engine/storage/baseline-schema.js";
import { RERAIL_BASELINE_SCHEMA_VERSION, RERAIL_MIGRATIONS, RULE_PROVENANCE_STEP } from "../../../src/engine/storage/migrations.js";
import {
  moveRuleProvenances, moveRuleProvenancesInWorker, type RuleProvenanceMoveOptions,
} from "../../../src/engine/storage/rule-provenance-move.js";
import { tableColumns } from "../../../src/engine/storage/table-probe.js";
import { type ThreadWatch, vacuumGivingWay } from "../../../src/engine/storage/vacuum-giving-way.js";
import { removeFolder } from "../../support/platform/remove-folder.js";
import { slowVacuumNext, untilExists } from "../../support/processes/slow-vacuum.js";
import { SqliteAuditStore as MainAuditStore } from "../support/main-audit-store.js";

// Every export calls the real one; a test that needs a disk with little free space overrides `statfsSync`.
vi.mock("node:fs", { spy: true });

/**
 * Runs, once, as the next process `vacuumGivingWay` starts on this thread is spawned, before it can take the write lock:
 * another connection that commits as an attempt begins. Every other spawn reaches the real child_process.
 */
const vacuumStart = vi.hoisted(() => ({ before: undefined as (() => void) | undefined }));
vi.mock("node:child_process", async () => {
  const actual = await vi.importActual<typeof ChildProcessModule>("node:child_process");
  return {
    ...actual,
    spawn: (command: string, args: readonly string[], options: SpawnOptions) => {
      const before = vacuumStart.before;
      vacuumStart.before = undefined;
      before?.();
      return actual.spawn(command, args, options);
    },
  };
});

const temporaryDirectories: string[] = [];

afterEach(async () => {
  vi.mocked(statfsSync).mockReset();
  vacuumStart.before = undefined;
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

/** Writes one audit per provenance as an older Rerail does, provenance inline; each audit's id and provenance text. */
function writeInlineAudits(path: string, provenances: readonly AuditRuleProvenance[][]): { auditId: string; text: string }[] {
  const written = provenances.map((provenance, index) => ({ auditId: `older-${index}`, text: JSON.stringify(provenance) }));
  const older = new Database(path);
  try {
    // One transaction: one sync for hundreds of audits.
    older.transaction(() => {
      written.forEach(({ auditId, text }, index) => older.prepare(INLINE_AUDIT).run(auditId, "session-a", "/home/dev/app", text, index, index + 1));
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

/** Leaves `bytes` of free pages in the file, as deleting that much history does. */
function freePages(path: string, bytes: number): void {
  const database = new Database(path);
  try {
    database.exec(`CREATE TABLE filler (data); INSERT INTO filler VALUES (randomblob(${bytes})); DROP TABLE filler`);
  } finally {
    database.close();
  }
}

/** Adds about `bytes` of pages in use to the file: a table of 1 MB random blobs that stays. */
function fillPages(path: string, bytes: number): void {
  const database = new Database(path);
  try {
    database.exec(`CREATE TABLE IF NOT EXISTS filler_kept (data);
      WITH RECURSIVE blob(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM blob WHERE n < ${Math.ceil(bytes / 1_000_000)})
      INSERT INTO filler_kept SELECT randomblob(1000000) FROM blob`);
  } finally {
    database.close();
  }
}

/** `VACUUM` on a connection of its own, as `rerail scrub` and `rerail reset` run it. */
function vacuumOn(path: string): void {
  const other = new Database(path);
  try {
    other.exec("VACUUM");
  } finally {
    other.close();
  }
}

/** One write on `database`: what it threw, if anything, and how long it took. */
function timedWrite(database: Database.Database): { failure: string | undefined; waitedMs: number } {
  const began = performance.now();
  try {
    database.exec("CREATE TABLE IF NOT EXISTS daemon_writes (at); INSERT INTO daemon_writes VALUES (1)");
    return { failure: undefined, waitedMs: performance.now() - began };
  } catch (error) {
    return { failure: (error as { code?: string }).code ?? String(error), waitedMs: performance.now() - began };
  }
}

/** An audit an older Rerail sharing the file records now, its provenance in its row; when it began. */
function recordAsOlder(path: string, auditId: string, provenance: readonly AuditRuleProvenance[]): number {
  const older = new Database(path);
  try {
    const now = Date.now();
    older.prepare(INLINE_AUDIT).run(auditId, "session-a", "/home/dev/app", JSON.stringify(provenance), now, now + 1);
    return now;
  } finally {
    older.close();
  }
}

/** The start of the line a move writes while it waits for an older Rerail sharing the file. */
const WAITS_FOR_OLDER = "moving the rule provenance of audits recorded before this Rerail's upgrade waits until";

/**
 * A file with about 8 MB in use and 8 MB free, and a connection on it as the move's, which `vacuumGivingWay` runs on;
 * the VACUUMs of the processes it starts hold the write lock `holdMs` first, creating `held` (`slowVacuumNext`) as the
 * first takes it. `close` closes the connection and puts the environment back.
 */
async function fileToVacuum(holdMs: number): Promise<{ path: string; held: string; client: Database.Database; close: () => void }> {
  const path = await temporaryDatabase();
  openRerailDatabase(path).$client.close();
  fillPages(path, 8_000_000);
  freePages(path, 8_000_000);
  const held = join(dirname(path), "vacuum-held");
  const restore = slowVacuumNext({ held, holdMs });
  const client = openRerailDatabase(path).$client;
  client.pragma("busy_timeout = 0");
  return {
    path, held, client, close: () => {
      restore();
      client.close();
    },
  };
}

/** The steps an older Rerail runs when it opens the file: this build's, without the one this change adds. */
const OLDER_STEPS = RERAIL_MIGRATIONS.filter(step => step.id !== RULE_PROVENANCE_STEP);

/**
 * The audits `auditIds` name as an older Rerail sharing the file reads them: `main`'s audit store (its read path, in
 * `main-audit-store.ts`), on the file opened with the older build's steps.
 */
function readAsOlder(path: string, auditIds: readonly string[]): (AuditRecord | undefined)[] {
  const older = new MainAuditStore(path, openRerailDatabase(path, { migrations: OLDER_STEPS }));
  try {
    return auditIds.map(auditId => older.get(auditId));
  } finally {
    older.close();
  }
}

/** Each audit's rule provenance as JSON text; undefined for an audit not found. */
function provenanceTexts(audits: readonly (AuditRecord | undefined)[]): (string | undefined)[] {
  return audits.map(audit => audit && JSON.stringify(audit.ruleProvenance));
}

/** Each audit record without its rule provenance; undefined for an audit not found. */
function withoutProvenance(audits: readonly (AuditRecord | undefined)[]): (Omit<AuditRecord, "ruleProvenance"> | undefined)[] {
  return audits.map(audit => {
    if (audit === undefined) return undefined;
    const { ruleProvenance: _provenance, ...rest } = audit;
    return rest;
  });
}

/** What this thread wrote to stderr while `run` ran, and what `run` resolved to. */
async function stderrWhile<T>(run: () => Promise<T>): Promise<{ result: T; logged: string }> {
  const stderr = vi.spyOn(process.stderr, "write").mockReturnValue(true);
  try {
    const result = await run();
    return { result, logged: stderr.mock.calls.map(([chunk]) => String(chunk)).join("") };
  } finally {
    stderr.mockRestore();
  }
}

/** Spies on this thread's stderr: what it has written so far, read at any time, and how to put it back. */
function spyOnStderr(): { logged: () => string; restore: () => void } {
  const stderr = vi.spyOn(process.stderr, "write").mockReturnValue(true);
  return { logged: () => stderr.mock.calls.map(([chunk]) => String(chunk)).join(""), restore: () => stderr.mockRestore() };
}

/** Resolves once `holds` returns true, read every 50 ms; throws after `timeoutMs`. */
async function until(holds: () => boolean, timeoutMs: number): Promise<void> {
  const deadline = performance.now() + timeoutMs;
  while (!holds()) {
    if (performance.now() > deadline) throw new Error(`still not so after ${timeoutMs} ms`);
    await new Promise(resolve => setTimeout(resolve, 50));
  }
}

/** The start of the line a `VACUUM` that gives way writes once it has waited its bound without committing. */
const NOT_SHRUNK = "has not shrunk yet";

/**
 * Makes every disk report `times` the bytes of the file's pages in use as free (`statfsSync`), read each time the move
 * asks; how to undo it.
 */
function diskOfPagesInUse(path: string, times: number): () => void {
  vi.mocked(statfsSync).mockImplementation(() => {
    const bavail = Math.floor((times * bytesInUse(path)) / 4096);
    return { type: 0, bsize: 4096, frsize: 4096, blocks: 1 << 20, bfree: bavail, bavail, files: 0, ffree: 0 };
  });
  return () => vi.mocked(statfsSync).mockReset();
}

/**
 * The daemon's move of the provenance audits hold inline, without its pause between batches, and timed by a clock that
 * never moves: each batch then takes twice the audits of the one before, up to `perBatch`, however loaded the machine.
 */
function move(path: string, options: RuleProvenanceMoveOptions = {}): Promise<number> {
  return moveRuleProvenances(path, { pause: () => Promise.resolve(), now: () => 0, ...options });
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
    database.exec("DROP TABLE IF EXISTS audit_rule_provenances");
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

/**
 * Upgrades a file of the build before that holds 30 audits' provenance inline and moves it, with what `prepare` sets up
 * (it returns how to undo it) in place during the move: the file's size before, and what the move wrote to stderr.
 */
async function moveLogging(
  prepare: (path: string) => () => void,
  pause?: () => Promise<void>,
): Promise<{ path: string; size: number; logged: string }> {
  const path = await fileOfTheBuildBefore();
  writeInlineAudits(path, Array.from({ length: 30 }, () => provenanceOf("/home/dev/app", 200)));
  const size = statSync(path).size;
  const release = prepare(path);
  const stderr = vi.spyOn(process.stderr, "write").mockReturnValue(true);
  try {
    expect(await move(path, pause === undefined ? {} : { pause })).toBe(30);
    return { path, size, logged: stderr.mock.calls.map(([chunk]) => String(chunk)).join("") };
  } finally {
    stderr.mockRestore();
    release();
  }
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

      // Another checkout's provenance is its own copy.
      const worktree = provenanceOf("/home/dev/app-worktree", 200);
      const other = store.start(auditInput("/home/dev/app-worktree", worktree));
      const none = store.start(auditInput("/home/dev/app", []));
      expect(bytesInUse(path) - empty).toBeLessThan(3 * size);

      for (const auditId of audited) expect(store.get(auditId)?.ruleProvenance).toEqual(provenance);
      expect(store.get(other)?.ruleProvenance).toEqual(worktree);
      expect(store.get(none)?.ruleProvenance).toEqual([]);
      const listed = new Map(store.list({ limit: 200 }).audits.map(audit => [audit.auditId, audit.ruleProvenance]));
      expect(listed.size).toBe(22);
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
      expect(await move(path)).toBe(16);
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
      // leaves it there, where the older Rerail reads it too.
      const sharing = new Database(path);
      try {
        const now = Date.now();
        sharing.prepare(INLINE_AUDIT).run("older-after", "session-a", "/home/dev/app", JSON.stringify(worktree), now + 1, now + 2);
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

    expect(await move(path)).toBe(4);
    const store = new SqliteAuditStore(path);
    let ids: string[];
    let current: (AuditRecord | undefined)[];
    try {
      const later = store.start(auditInput("/home/dev/app", app));
      store.finish(later, { status: "completed" });
      ids = [...olderIds, later];
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
    expect(provenanceTexts(current)).toEqual([...written.map(({ text }) => text), JSON.stringify(app)]);
  });

  it("leaves each audit's provenance inline on the open that upgrades, and moves it a batch per write transaction", async () => {
    const path = await fileOfTheBuildBefore();
    const written = writeInlineAudits(path, Array.from({ length: 30 }, () => provenanceOf("/home/dev/app", 200)));

    openRerailDatabase(path).$client.close();
    // The open added the step's table and column, and moved nothing.
    expect(inlineAudits(path)).toBe(30);

    // An older Rerail recording audits that never waits for the lock: between two batches it is free.
    const older = new Database(path, { timeout: 0 });
    const store = new SqliteAuditStore(path);
    try {
      let pauses = 0;
      const moved = await move(path, {
        perBatch: 10,
        pause: () => {
          pauses += 1;
          const now = Date.now();
          older.prepare(INLINE_AUDIT).run(`between-${pauses}`, "session-a", "/home/dev/app", "[]", now + 1, now + 2);
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
      older.close();
    }
  });

  it("moves nothing while an older Rerail sharing the file has recorded an audit within the window, so it reads every recorded audit's provenance, and moves once the window has passed", async () => {
    const path = await fileOfTheBuildBefore();
    const app = provenanceOf("/home/dev/app", 200);
    const written = writeInlineAudits(path, Array.from({ length: 30 }, () => app));
    const recorded = written.map(({ auditId }) => auditId);
    // This build upgrades the file; then an older Rerail sharing it records an audit, its provenance in its row.
    openRerailDatabase(path).$client.close();
    const at = recordAsOlder(path, "older-after", app);
    const day = 24 * 3_600_000;

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

    // The first batch moved 10; the move stopped there and ran no VACUUM.
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
    openRerailDatabase(path).$client.close();
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
    expect(waits[1]).toContain(`waits until ${new Date(second + 3_000).toISOString()}: a Rerail older than this one recorded an `
      + `audit at ${new Date(second).toISOString()}`);
    expect(waits[1]).toContain("The move starts once no older Rerail has recorded an audit for 3 s.");
    // The older Rerail's two audits stay in their rows; the recorded ones moved, and the file gave the space back.
    expect(inlineAudits(path)).toBe(2);
    expect(pageBytes(path).free).toBe(0);
  }, 60_000);

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

  it("zeroes the pages the move frees, so none keeps a copy of what it moved when no VACUUM follows", async () => {
    const path = await fileOfTheBuildBefore();
    const marker = "/home/dev/zeroed-by-the-move";
    const provenance = provenanceOf(marker, 200);
    writeInlineAudits(path, Array.from({ length: 30 }, () => provenance));
    const occurrences = (text: string): number => text.split(marker).length - 1;
    // Too little disk for `VACUUM`: the move empties the WAL into the file and leaves the pages it freed there.
    vi.mocked(statfsSync).mockImplementation(() => ({ type: 0, bsize: 4096, frsize: 4096, blocks: 1 << 20, bfree: 16, bavail: 16, files: 0, ffree: 0 }));
    const stderr = vi.spyOn(process.stderr, "write").mockReturnValue(true);
    try {
      expect(await move(path)).toBe(30);
    } finally {
      stderr.mockRestore();
    }

    expect(pageBytes(path).free).toBeGreaterThan(0);
    expect(statSync(`${path}-wal`).size).toBe(0);
    // The stored copy's worth and less than a second copy: the 30 inline copies whose pages the move freed held 30 times
    // as many.
    const perCopy = occurrences(JSON.stringify(provenance));
    const inFile = occurrences(readFileSync(path).toString("latin1"));
    expect(inFile).toBeGreaterThanOrEqual(perCopy / 2);
    expect(inFile).toBeLessThan(2 * perCopy);
  });

  it("gives the space the move freed back to the disk once it ends, and no later move rewrites the file", async () => {
    const path = await fileOfTheBuildBefore();
    const written = writeInlineAudits(path, Array.from({ length: 30 }, () => provenanceOf("/home/dev/app", 200)));
    const before = statSync(path).size;

    expect(await move(path)).toBe(30);

    // The file is its pages in use, a small part of what it was, and every audit reads its provenance as before.
    expect(pageBytes(path).free).toBe(0);
    expect(statSync(path).size).toBe(bytesInUse(path));
    expect(statSync(path).size).toBeLessThan(before / 5);
    const store = new SqliteAuditStore(path);
    try {
      for (const { auditId, text } of written) expect(JSON.stringify(store.get(auditId)?.ruleProvenance), auditId).toBe(text);
    } finally {
      store.close();
    }

    // Space that deleted history frees later stays in the file for new history: no later move rewrites it.
    freePages(path, 1_000_000);
    const free = pageBytes(path).free;
    expect(free).toBeGreaterThan(900_000);
    expect(await move(path)).toBe(0);
    expect(pageBytes(path).free).toBe(free);
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
    expect(pageBytes(path).free).toBe(0);
    const store = new SqliteAuditStore(path);
    try {
      for (const { auditId, text } of written) expect(JSON.stringify(store.get(auditId)?.ruleProvenance), auditId).toBe(text);
    } finally {
      store.close();
    }
    // Nothing is left: the next run reads that and starts no thread.
    expect(await moveRuleProvenancesInWorker(path)).toBe(0);
  }, 60_000);

  it("ends its VACUUM at once when stopped, leaves the file whole, and a later run finishes the shrink", async () => {
    const path = await fileOfTheBuildBefore();
    // About 80 MB of provenance, more than the 64 MB of free space a later run leaves to new history, beside 100 MB of
    // other history: what the move frees can be a small part of a file that has recorded much else.
    const written = writeInlineAudits(path, Array.from({ length: 800 }, () => provenanceOf("/home/dev/app", 200)));
    fillPages(path, 100_000_000);
    // A VACUUM that would hold the write lock 30 s, as one of a large file on a loaded machine does.
    const held = join(dirname(path), "vacuum-held");
    const stop = new AbortController();
    const restore = slowVacuumNext({ held, holdMs: 30_000 });
    let moving: Promise<number>;
    try {
      moving = moveRuleProvenancesInWorker(path, stop.signal);
    } finally {
      restore();
    }
    await untilExists(held, 60_000);

    const asked = performance.now();
    stop.abort();
    expect(await moving).toBe(800);
    // The daemon's close waits for this. Run on the move's thread, the VACUUM held it the whole 30 s.
    expect(performance.now() - asked).toBeLessThan(2_000);

    // SQLite dropped the VACUUM: the write lock is free, the file is whole, and every audit reads its provenance.
    const writer = new Database(path);
    try {
      writer.pragma("busy_timeout = 0");
      writer.exec("BEGIN IMMEDIATE; ROLLBACK");
      expect(writer.pragma("integrity_check", { simple: true })).toBe("ok");
    } finally {
      writer.close();
    }
    expect(inlineAudits(path)).toBe(0);
    const store = new SqliteAuditStore(path);
    try {
      for (const { auditId, text } of written) expect(JSON.stringify(store.get(auditId)?.ruleProvenance), auditId).toBe(text);
    } finally {
      store.close();
    }

    // The stop left the space the move freed in the file. The next daemon start, with nothing left to move, gives it
    // back: its thread runs the VACUUM, giving way as before.
    expect(pageBytes(path).free).toBeGreaterThan(64_000_000);
    expect(await moveRuleProvenancesInWorker(path)).toBe(0);
    expect(pageBytes(path).free).toBe(0);
    expect(statSync(path).size).toBe(bytesInUse(path));
  }, 180_000);

  it("lets a VACUUM that has committed run to its end when the thread it gives way to stops answering", async () => {
    const path = await temporaryDatabase();
    openRerailDatabase(path).$client.close();
    // 64 MB in use and 8 MB free: the WAL the VACUUM fills holds the 64 MB until its checkpoint.
    fillPages(path, 64_000_000);
    freePages(path, 8_000_000);
    // The process creates `committed` once its VACUUM has committed, then waits 1 s before the checkpoint its commit
    // runs: the moment a write the daemon's thread began during the VACUUM is still asleep in its busy handler.
    const committed = join(dirname(path), "vacuum-committed");
    const restore = slowVacuumNext({ committed, lingerMs: 1_000 });
    const client = openRerailDatabase(path).$client;
    client.pragma("busy_timeout = 0");
    // The watched thread stops answering in that moment, once.
    let stalledFor: number | undefined;
    const watch: ThreadWatch = stalled => {
      const poll = setInterval(() => {
        if (stalledFor !== undefined || !existsSync(committed)) return;
        const began = performance.now();
        stalled();
        stalledFor = performance.now() - began;
      }, 5);
      return () => clearInterval(poll);
    };
    let logged: string;
    try {
      ({ logged } = await stderrWhile(() => vacuumGivingWay(client, path, {
        watch, signal: undefined, blocked: () => undefined, waitedLong: () => undefined, noticeMs: 600_000,
      })));
    } finally {
      restore();
      client.close();
    }

    // The process ran to its end, and no line says the VACUUM gave way. Ended there, the TRUNCATE checkpoint after it
    // copied the 64 MB into the database file while it held the write lock, on the thread that had stopped answering.
    const seen = { stalled: stalledFor !== undefined, ranToItsEnd: existsSync(`${committed}.done`), gaveWay: logged.includes("gave way") };
    expect(seen, `the stall held the thread ${Math.round(stalledFor ?? -1)} ms`)
      .toEqual({ stalled: true, ranToItsEnd: true, gaveWay: false });
    expect(stalledFor).toBeLessThan(100);
    expect(pageBytes(path).free).toBe(0);
  }, 120_000);

  it("ends its VACUUM when the thread it gives way to stalls, though another connection shrank the file as its process started", async () => {
    // The first VACUUM an attempt's process runs holds the write lock 12 s before it runs, past the 5 s busy timeout of
    // every Rerail writer.
    const { path, held, client, close } = await fileToVacuum(12_000);
    // As the attempt starts its process, before that process can take the lock, another connection gives the free pages
    // back, as `rerail scrub` or `rerail reset` does, and history deleted since frees 1 MB: the file now has fewer pages
    // than when the attempt began, and some free, so the attempt's process runs its VACUUM.
    vacuumStart.before = () => {
      vacuumOn(path);
      freePages(path, 1_000_000);
    };
    // The watched thread stalls once that process holds the lock, as when a write of its own waits for it; that write
    // then runs with the busy timeout the daemon's stores have.
    const daemonWrite = new Database(path, { timeout: 5_000 });
    let wrote: { failure: string | undefined; waitedMs: number } | undefined;
    const watch: ThreadWatch = stalled => {
      const poll = setInterval(() => {
        if (wrote !== undefined || !existsSync(held)) return;
        stalled();
        wrote = timedWrite(daemonWrite);
      }, 5);
      return () => clearInterval(poll);
    };
    let logged: string;
    try {
      ({ logged } = await stderrWhile(() => vacuumGivingWay(client, path, {
        watch, signal: undefined, blocked: () => undefined, waitedLong: () => undefined, noticeMs: 600_000,
      })));
    } finally {
      daemonWrite.close();
      close();
    }

    // The stall ended the attempt's process, which held the lock, and the write went through inside its busy timeout.
    // Taking the other connection's shrink for its own commit, the attempt let the process hold the lock 12 s, and the
    // write failed with SQLITE_BUSY after 5 s.
    const seen = {
      failure: wrote?.failure,
      inBusyTimeout: (wrote?.waitedMs ?? Number.POSITIVE_INFINITY) < 5_000,
      gaveWay: logged.includes("VACUUM gave way (the daemon's thread did not answer"),
    };
    expect(seen, `the write waited ${Math.round(wrote?.waitedMs ?? -1)} ms`).toEqual({ failure: undefined, inBusyTimeout: true, gaveWay: true });
    expect(pageBytes(path).free).toBe(0);
  }, 120_000);

  it("runs no VACUUM when another connection gave the free pages back as its process started", async () => {
    // A VACUUM the attempt's process runs creates `held` as it takes the write lock, and holds it 12 s.
    const { path, held, client, close } = await fileToVacuum(12_000);
    // As the attempt starts its process, another connection gives every free page back, as `rerail scrub` does.
    vacuumStart.before = () => vacuumOn(path);
    let logged: string;
    try {
      ({ logged } = await stderrWhile(() => vacuumGivingWay(client, path, {
        watch: () => () => undefined, signal: undefined, blocked: () => undefined, waitedLong: () => undefined, noticeMs: 600_000,
      })));
    } finally {
      close();
    }

    // The process found no free page once it held the lock, and ran no VACUUM. Before, it ran one, which rewrote the file
    // a second time and held the lock throughout.
    expect({ vacuumed: existsSync(held), logged }).toEqual({ vacuumed: false, logged: "" });
    expect(pageBytes(path).free).toBe(0);
  }, 60_000);

  it("says once, naming rerail scrub, when writes keep its VACUUM from starting past its bound, and VACUUMs once they stop", async () => {
    const path = await fileOfTheBuildBefore();
    writeInlineAudits(path, Array.from({ length: 30 }, () => provenanceOf("/home/dev/app", 200)));
    // Another connection commits every 300 ms for 5 s, as a busy daemon does: the file is never quiet for 2 s meanwhile.
    const writer = new Database(path);
    writer.exec("CREATE TABLE steady (at)");
    const steady = setInterval(() => writer.prepare("INSERT INTO steady VALUES (?)").run(performance.now()), 300);
    const quiet = setTimeout(() => clearInterval(steady), 5_000);
    let logged: string;
    try {
      ({ logged } = await stderrWhile(async () => {
        // A thread that always answers: nothing gives way here, only the wait for a quiet file.
        expect(await move(path, { watch: () => () => undefined, waitNotice: 2_500 })).toBe(30);
      }));
    } finally {
      clearTimeout(quiet);
      clearInterval(steady);
      writer.close();
    }

    const told = logged.split("\n").filter(line => line.includes("rerail scrub"));
    expect(told).toHaveLength(1);
    expect(told[0]).toContain(`rerail: ${path} has not shrunk yet`);
    expect(told[0]).toContain("in 2.5 s no other connection left the file alone for the 2 s its VACUUM waits for");
    expect(pageBytes(path).free).toBe(0);
  }, 60_000);

  it("says how many times its VACUUM gave way when it says, past its bound, that the file has not shrunk", async () => {
    const path = await fileOfTheBuildBefore();
    writeInlineAudits(path, Array.from({ length: 30 }, () => provenanceOf("/home/dev/app", 200)));
    // The watched thread stops answering as each attempt starts, until the line is written: each attempt gives way before
    // its process runs VACUUM, and the next waits for twice the quiet (2 s, then 4 s, then 8 s), so the bound of 10 s
    // passes while the third waits. Once the line is written, the next attempt runs to its end.
    const stderr = spyOnStderr();
    const watch: ThreadWatch = stalled => {
      if (!stderr.logged().includes(NOT_SHRUNK)) stalled();
      return () => undefined;
    };
    let logged = "";
    try {
      expect(await move(path, { watch, waitNotice: 10_000 })).toBe(30);
    } finally {
      logged = stderr.logged();
      stderr.restore();
    }

    const lines = logged.split("\n");
    const toldAt = lines.findIndex(line => line.includes(NOT_SHRUNK));
    const gaveWay = lines.slice(0, toldAt).filter(line => line.includes("VACUUM gave way (")).length;
    expect(gaveWay).toBeGreaterThan(0);
    expect(lines[toldAt]).toContain(`in 10 s its VACUUM gave way ${gaveWay === 1 ? "once" : `${gaveWay} times`}, and runs `
      + `again once no other connection has written to the file for ${2 * 2 ** gaveWay} s. The daemon keeps trying; run `
      + "`rerail scrub`");
    expect(lines.filter(line => line.includes(NOT_SHRUNK))).toHaveLength(1);
    expect(pageBytes(path).free).toBe(0);
  }, 60_000);

  it("says its VACUUM is running when an attempt runs as its bound passes, after one that gave way", async () => {
    const path = await fileOfTheBuildBefore();
    writeInlineAudits(path, Array.from({ length: 30 }, () => provenanceOf("/home/dev/app", 200)));
    // The first VACUUM a process runs holds the write lock 30 s before it runs, as one of a large file on a loaded
    // machine does. The watched thread stops answering as the first attempt starts, which gives way before its process
    // runs VACUUM, and answers through the second, which starts after 4 s of quiet and holds the lock past the bound of
    // 10 s. A stop ends it once the line is written.
    const held = join(dirname(path), "vacuum-held");
    const restore = slowVacuumNext({ held, holdMs: 30_000 });
    let attempts = 0;
    const watch: ThreadWatch = stalled => {
      if ((attempts += 1) === 1) stalled();
      return () => undefined;
    };
    const stop = new AbortController();
    const stderr = spyOnStderr();
    let logged = "";
    try {
      const moving = move(path, { watch, waitNotice: 10_000, signal: stop.signal });
      await until(() => stderr.logged().includes(NOT_SHRUNK), 60_000);
      await untilExists(held, 60_000);
      stop.abort();
      expect(await moving).toBe(30);
    } finally {
      logged = stderr.logged();
      stderr.restore();
      restore();
    }

    const told = logged.split("\n").filter(line => line.includes(NOT_SHRUNK));
    expect(told).toHaveLength(1);
    expect(told[0]).toContain("in 10 s its VACUUM gave way once; it is running again now. The daemon keeps trying; run "
      + "`rerail scrub`");
    expect(attempts).toBe(2);
    expect(logged).toContain("Rerail stopped before VACUUM finished");
  }, 90_000);

  it("reads the free disk space again before each attempt of a VACUUM that gives way", async () => {
    const path = await fileOfTheBuildBefore();
    writeInlineAudits(path, Array.from({ length: 30 }, () => provenanceOf("/home/dev/app", 200)));
    // Room when the move first asks (one read per disk); 64 KB from the next read on, as when the disk fills before the
    // first quiet moment comes.
    const disks = statSync(dirname(path)).dev === statSync(tmpdir()).dev ? 1 : 2;
    let reads = 0;
    vi.mocked(statfsSync).mockImplementation(() => {
      const bavail = (reads += 1) <= disks ? 1 << 30 : 16;
      return { type: 0, bsize: 4096, frsize: 4096, blocks: 1 << 30, bfree: bavail, bavail, files: 0, ffree: 0 };
    });

    const { logged } = await stderrWhile(async () => {
      expect(await move(path, { watch: () => () => undefined })).toBe(30);
    });
    expect(logged).toContain(`rerail: ${path} keeps its size`);
    expect(logged).toContain("of free disk space, less than the");
    expect(pageBytes(path).free).toBeGreaterThan(0);
  }, 30_000);

  it.skipIf(!existsSync("/dev/shm"))("measures the disk of the folder SQLite writes VACUUM's copy to, when SQLITE_TMPDIR names one", async () => {
    // A folder on a disk of its own (memory), as SQLite takes SQLITE_TMPDIR before TMPDIR.
    const sqliteTemporary = await mkdtemp("/dev/shm/rerail-sqlite-temporary-");
    temporaryDirectories.push(sqliteTemporary);
    const before = process.env.SQLITE_TMPDIR;
    process.env.SQLITE_TMPDIR = sqliteTemporary;
    try {
      const { logged } = await moveLogging(() => {
        // That disk has 64 KB free; every other has plenty.
        vi.mocked(statfsSync).mockImplementation(folder => {
          const bavail = folder === sqliteTemporary ? 16 : 1 << 30;
          return { type: 0, bsize: 4096, frsize: 4096, blocks: 1 << 30, bfree: bavail, bavail, files: 0, ffree: 0 };
        });
        return () => vi.mocked(statfsSync).mockReset();
      });
      expect(logged).toContain(`${sqliteTemporary} has 0.1 MB of free disk space`);
    } finally {
      if (before === undefined) delete process.env.SQLITE_TMPDIR;
      else process.env.SQLITE_TMPDIR = before;
    }
  });

  it("leaves the free space in the file when there is no provenance to move", async () => {
    const path = await fileOfTheBuildBefore();
    writeInlineAudits(path, [[], []]);
    freePages(path, 1_000_000);
    const free = pageBytes(path).free;

    expect(await move(path)).toBe(0);

    // The step's table and index take a few of the free pages; the rest stay.
    expect(pageBytes(path).free).toBeGreaterThan(free - 64 * 1024);
  });

  it("keeps the file's size, and says why, when another connection reads the database through the move", async () => {
    const { path, size, logged } = await moveLogging(path => {
      // A read begun before the upgrade: the WAL the move fills cannot be emptied while it lasts.
      const reader = new Database(path, { readonly: true });
      reader.exec("BEGIN");
      reader.prepare("SELECT count(*) FROM audits").get();
      return () => reader.close();
    });
    expect(logged).toContain(`rerail: ${path} keeps its size`);
    expect(logged).toContain("another connection was using it. Run `rerail scrub` to give the free space back.");
    expect(statSync(path).size).toBeGreaterThanOrEqual(size);
    expect(pageBytes(path).free).toBeGreaterThan(size / 2);
  }, 30_000);

  it("waits for another connection's read between tries, never inside SQLite, so its thread keeps running", async () => {
    const { result: { logged }, longest } = await withLongestStall(() => moveLogging(path => {
      // A read held through the move: the checkpoint before `VACUUM` finds it on every try.
      const reader = new Database(path, { readonly: true });
      reader.exec("BEGIN");
      reader.prepare("SELECT count(*) FROM audits").get();
      return () => reader.close();
    }, () => new Promise(resolve => setTimeout(resolve, 20))));
    expect(logged).toContain("another connection was using it");
    // The thread running the move takes a stop between two tries, and the daemon's close waits for that. Waiting inside
    // SQLite, the checkpoint held the thread for its 5 s busy timeout.
    expect(longest).toBeLessThan(2_000);
  }, 30_000);

  it("keeps the file's size, and says why, when the disk has less free space than the pages VACUUM copies", async () => {
    const { path, size, logged } = await moveLogging(() => {
      vi.mocked(statfsSync).mockImplementation(() => ({ type: 0, bsize: 4096, frsize: 4096, blocks: 1 << 20, bfree: 16, bavail: 16, files: 0, ffree: 0 }));
      return () => vi.mocked(statfsSync).mockReset();
    });
    expect(logged).toContain(`rerail: ${path} keeps its size`);
    expect(logged).toContain("free disk space");
    expect(statSync(path).size).toBeGreaterThanOrEqual(size);
    expect(pageBytes(path).free).toBeGreaterThan(size / 2);
  });

  it("runs VACUUM when the disk has room for the pages in use, though less than the file's size", async () => {
    // Three times the pages in use: room for the copy in the temporary folder and the one through the WAL, on one disk.
    const { path, size, logged } = await moveLogging(path => diskOfPagesInUse(path, 3));
    expect(logged).not.toContain("keeps its size");
    expect(pageBytes(path).free).toBe(0);
    expect(statSync(path).size).toBe(bytesInUse(path));
    expect(statSync(path).size).toBeLessThan(size / 5);
  });

  it("skips VACUUM when one disk holds the database's folder and the temporary folder and has room for one copy only", async () => {
    const { path, size, logged } = await moveLogging(path => {
      expect(statSync(dirname(path)).dev).toBe(statSync(tmpdir()).dev);
      return diskOfPagesInUse(path, 1.5);
    });
    expect(logged).toContain(`rerail: ${path} keeps its size`);
    expect(logged).toContain("free disk space");
    expect(statSync(path).size).toBeGreaterThanOrEqual(size);
  });

  it.each([
    ["before", 1, "keeps its size", "its WAL could not be emptied (disk I/O error)"],
    ["after", 2, "shrinks to ", "emptying its WAL after VACUUM failed (disk I/O error)"],
  ])("ends the move it committed, and says why, when emptying the WAL %s VACUUM fails", async (_, failing, line, reason) => {
    const pragma = Database.prototype.pragma;
    let checkpoints = 0;
    const { path, logged } = await moveLogging(() => {
      // A checkpoint that throws rather than reports busy, as one that cannot truncate the WAL on a full disk does.
      const spy = vi.spyOn(Database.prototype, "pragma").mockImplementation(function (this: Database.Database, source: string, options?: Database.PragmaOptions) {
        if (source === "wal_checkpoint(TRUNCATE)" && (checkpoints += 1) === failing) throw new Database.SqliteError("disk I/O error", "SQLITE_IOERR");
        return pragma.call(this, source, options);
      });
      return () => spy.mockRestore();
    });
    expect(checkpoints).toBe(failing);
    expect(logged).toContain(`rerail: ${path} ${line}`);
    expect(logged).toContain(reason);
    expect(inlineAudits(path)).toBe(0);
  });

  it("says the file shrinks later when another connection reads it as the VACUUM ends", async () => {
    const path = await fileOfTheBuildBefore();
    writeInlineAudits(path, Array.from({ length: 30 }, () => provenanceOf("/home/dev/app", 200)));
    const readers: Database.Database[] = [];
    const stderr = vi.spyOn(process.stderr, "write").mockReturnValue(true);
    let logged = "";
    try {
      const moved = await move(path, {
        pause: () => {
          // Before the batch that finds nothing left: a read that begins on an empty WAL lets the checkpoint before
          // `VACUUM` run, and keeps the one after it from writing the rewritten file into the database file.
          if (readers.length === 0 && inlineAudits(path) === 0) {
            const checkpointer = new Database(path);
            try {
              checkpointer.pragma("wal_checkpoint(TRUNCATE)");
            } finally {
              checkpointer.close();
            }
            const reader = new Database(path, { readonly: true });
            readers.push(reader);
            reader.exec("BEGIN");
            reader.prepare("SELECT count(*) FROM audits").get();
          }
          return Promise.resolve();
        },
      });
      expect(moved).toBe(30);
      logged = stderr.mock.calls.map(([chunk]) => String(chunk)).join("");
    } finally {
      stderr.mockRestore();
      for (const reader of readers) reader.close();
    }
    expect(logged).toContain(`rerail: ${path} shrinks to `);
    expect(logged).toContain("another connection was reading it as VACUUM ended");

    // Once the read ends, a checkpoint writes the rewritten file into the database file, as the line says.
    const later = new Database(path);
    try {
      later.pragma("wal_checkpoint(TRUNCATE)");
    } finally {
      later.close();
    }
    expect(pageBytes(path).free).toBe(0);
    expect(statSync(path).size).toBe(bytesInUse(path));
  }, 30_000);

  it("says the file has shrunk and only its WAL waits when another connection begins a read just after the VACUUM", async () => {
    const path = await fileOfTheBuildBefore();
    writeInlineAudits(path, Array.from({ length: 30 }, () => provenanceOf("/home/dev/app", 200)));
    const readers: Database.Database[] = [];
    const exec = Database.prototype.exec;
    // A read that begins once `VACUUM` has committed reads the rewritten file, so it lets the checkpoint after it write
    // every frame into the database file, but not empty the WAL.
    const vacuum = vi.spyOn(Database.prototype, "exec").mockImplementation(function (this: Database.Database, source: string) {
      exec.call(this, source);
      if (source === "VACUUM") {
        const reader = new Database(path, { readonly: true });
        readers.push(reader);
        reader.exec("BEGIN");
        reader.prepare("SELECT count(*) FROM audits").get();
      }
      return this;
    });
    const stderr = vi.spyOn(process.stderr, "write").mockReturnValue(true);
    let logged = "";
    try {
      expect(await move(path)).toBe(30);
      logged = stderr.mock.calls.map(([chunk]) => String(chunk)).join("");
      // While the read lasts: the file is its pages in use already, and the WAL still holds the rewritten file.
      expect(readers).toHaveLength(1);
      expect(pageBytes(path).free).toBe(0);
      expect(statSync(path).size).toBe(bytesInUse(path));
      expect(statSync(`${path}-wal`).size).toBeGreaterThan(statSync(path).size);
    } finally {
      stderr.mockRestore();
      vacuum.mockRestore();
      for (const reader of readers) reader.close();
    }
    expect(logged).toContain(`rerail: ${path} shrank to `);
    expect(logged).toContain("but its WAL stays at ");
    expect(logged).not.toContain("at a later checkpoint");
  });

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
