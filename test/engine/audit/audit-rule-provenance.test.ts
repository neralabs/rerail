import { createHash } from "node:crypto";
import { existsSync, readFileSync, statfsSync, statSync } from "node:fs";
import { mkdir, mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import Database from "better-sqlite3";
import { afterEach, describe, expect, it, vi } from "vitest";
import { SqliteAuditStore } from "../../../src/engine/audit/audit-store.js";
import type { AuditInput, AuditRuleProvenance, LegacyAuditRuleProvenance, RepositoryAuditRuleProvenance } from "../../../src/engine/audit/audit.js";
import { openRerailDatabase, resetRepositoryHistory } from "../../../src/engine/storage/database.js";
import { RERAIL_BASELINE_SCHEMA_STATEMENTS } from "../../../src/engine/storage/baseline-schema.js";
import { RERAIL_BASELINE_SCHEMA_VERSION, RULE_PROVENANCE_STEP } from "../../../src/engine/storage/migrations.js";
import { moveRuleProvenances, type RuleProvenanceMoveOptions } from "../../../src/engine/storage/rule-provenance-move.js";
import { tableColumns } from "../../../src/engine/storage/table-probe.js";

// Every export calls the real one; a test that needs a disk with little free space overrides `statfsSync`.
vi.mock("node:fs", { spy: true });

const temporaryDirectories: string[] = [];

afterEach(async () => {
  vi.mocked(statfsSync).mockReset();
  await Promise.all(temporaryDirectories.splice(0).map(path => rm(path, { recursive: true, force: true })));
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
    written.forEach(({ auditId, text }, index) => older.prepare(INLINE_AUDIT).run(auditId, "session-a", "/home/dev/app", text, index, index + 1));
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

/** The daemon's move of the provenance audits hold inline, without its pause between batches. */
function move(path: string, options: RuleProvenanceMoveOptions = {}): Promise<number> {
  return moveRuleProvenances(path, { pause: () => Promise.resolve(), ...options });
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
async function moveLogging(prepare: (path: string) => () => void): Promise<{ path: string; size: number; logged: string }> {
  const path = await fileOfTheBuildBefore();
  writeInlineAudits(path, Array.from({ length: 30 }, () => provenanceOf("/home/dev/app", 200)));
  const size = statSync(path).size;
  const release = prepare(path);
  const stderr = vi.spyOn(process.stderr, "write").mockReturnValue(true);
  try {
    expect(await move(path)).toBe(30);
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

  it("writes no zeroed copy of what the move frees through the WAL", async () => {
    const path = await fileOfTheBuildBefore();
    const provenance = provenanceOf("/home/dev/app", 200);
    writeInlineAudits(path, Array.from({ length: 30 }, () => provenance));
    const wal = (): number => (existsSync(`${path}-wal`) ? statSync(`${path}-wal`).size : 0);

    let largest = 0;
    const moved = await move(path, {
      perBatch: 10,
      pause: () => {
        largest = Math.max(largest, wal());
        return Promise.resolve();
      },
    });

    // The WAL holds the pages the batches changed. Zeroing what they freed would write each batch's 10 copies again.
    expect(moved).toBe(30);
    expect(largest).toBeGreaterThan(0);
    expect(largest).toBeLessThan(10 * Buffer.byteLength(JSON.stringify(provenance)));
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
    expect(logged).toContain("another connection was using it");
    expect(statSync(path).size).toBeGreaterThanOrEqual(size);
    expect(pageBytes(path).free).toBeGreaterThan(size / 2);
  }, 30_000);

  it("keeps the file's size, and says why, when the disk has less free space than the file", async () => {
    const { path, size, logged } = await moveLogging(() => {
      vi.mocked(statfsSync).mockImplementation(() => ({ type: 0, bsize: 4096, frsize: 4096, blocks: 1 << 20, bfree: 16, bavail: 16, files: 0, ffree: 0 }));
      return () => vi.mocked(statfsSync).mockReset();
    });
    expect(logged).toContain(`rerail: ${path} keeps its size`);
    expect(logged).toContain("free disk space");
    expect(statSync(path).size).toBeGreaterThanOrEqual(size);
    expect(pageBytes(path).free).toBeGreaterThan(size / 2);
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
});
