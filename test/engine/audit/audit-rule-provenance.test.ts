import { createHash } from "node:crypto";
import { readFileSync, statSync } from "node:fs";
import { mkdir, mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import Database from "better-sqlite3";
import { afterEach, describe, expect, it, vi } from "vitest";
import { SqliteAuditStore } from "../../../src/engine/audit/audit-store.js";
import type {
  AuditInput, AuditRecord, AuditRuleProvenance, LegacyAuditRuleProvenance, RepositoryAuditRuleProvenance,
} from "../../../src/engine/audit/audit.js";
import { eraseFreedPages, openRerailDatabase, resetRepositoryHistory } from "../../../src/engine/storage/database.js";
import { RERAIL_BASELINE_SCHEMA_STATEMENTS } from "../../../src/engine/storage/baseline-schema.js";
import { RERAIL_BASELINE_SCHEMA_VERSION, RERAIL_MIGRATIONS, RULE_PROVENANCE_STEP } from "../../../src/engine/storage/migrations.js";
import {
  moveRuleProvenances, moveRuleProvenancesInWorker, type RuleProvenanceMoveOptions,
} from "../../../src/engine/storage/rule-provenance-move.js";
import { tableColumns } from "../../../src/engine/storage/table-probe.js";
import { removeFolder } from "../../support/platform/remove-folder.js";
import { SqliteAuditStore as MainAuditStore } from "../support/main-audit-store.js";

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
    // The older Rerail's two audits stay in their rows; the recorded ones moved.
    expect(inlineAudits(path)).toBe(2);
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
    // The stored copy's worth and less than a second copy: the 30 inline copies whose pages the move freed held 30 times
    // as many.
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

    // No VACUUM: the file keeps its size, most of it free pages, and every audit reads its provenance as before.
    expect(statSync(path).size).toBe(before);
    expect(pageBytes(path).free).toBeGreaterThan(before / 2);
    const store = new SqliteAuditStore(path);
    try {
      for (const { auditId, text } of written) expect(JSON.stringify(store.get(auditId)?.ruleProvenance), auditId).toBe(text);
      // New audits of the same rules fill the free pages before the file grows.
      for (let index = 0; index < 30; index += 1) store.finish(store.start(auditInput("/home/dev/app", app)), { status: "completed" });
    } finally {
      store.close();
    }
    expect(statSync(path).size).toBe(before);

    // `rerail scrub` rewrites the file (`eraseFreedPages`): it shrinks to its pages in use, a small part of what it was.
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
