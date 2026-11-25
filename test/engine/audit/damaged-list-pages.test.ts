import { mkdir, mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import Database from "better-sqlite3";
import { afterEach, describe, expect, it, vi } from "vitest";
import { SqliteAuditReader, SqliteAuditStore, type AuditReader } from "../../../src/engine/audit/audit-store.js";
import type { CodeRuleViolation } from "../../../src/engine/code-rule/code-rule.js";
import { removeFolder } from "../../support/platform/remove-folder.js";

const temporaryDirectories: string[] = [];

afterEach(async () => {
  await Promise.all(temporaryDirectories.splice(0).map(path => removeFolder(path)));
});

async function temporaryDatabase(): Promise<string> {
  const root = await mkdtemp(join(tmpdir(), "rerail-damaged-list-"));
  temporaryDirectories.push(root);
  const path = join(root, "home", "rerail.sqlite");
  await mkdir(dirname(path), { recursive: true });
  return path;
}

/** Spies on this thread's stderr, where the store writes the line that names a damaged row: what it holds so far. */
function spyOnStderr(): { logged: () => string; restore: () => void } {
  const stderr = vi.spyOn(process.stderr, "write").mockReturnValue(true);
  return { logged: () => stderr.mock.calls.map(([chunk]) => String(chunk)).join(""), restore: () => stderr.mockRestore() };
}

const violation = (id: string): CodeRuleViolation => ({
  id, ruleId: `rule-${id.slice(-1)}`, clauseId: "clause", code: "code", mode: "steer", message: "m", path: `src/${id}.ts`,
  captures: {}, variables: {},
}) as CodeRuleViolation;

/**
 * Four finished audits of one session, started at 1 to 4 s, each with three violations `a<n>-0` to `a<n>-2`, and one
 * repository audit naming each; the audit ids, oldest first.
 */
function fourAudits(path: string): string[] {
  const store = new SqliteAuditStore(path);
  const client = new Database(path);
  try {
    return [1, 2, 3, 4].map(n => {
      const auditId = store.start({
        sessionId: "session-a", workspaceRoot: `/w${n}`, scope: "repository", mode: "steer", revision: 1, catalogRevision: null,
        ruleRevisions: [{ ruleId: "rule-0", revision: 1 }], paths: ["src/a.ts"],
      });
      store.finish(auditId, {
        status: "completed", violations: [0, 1, 2].map(k => violation(`a${n}-${k}`)),
        failures: [{ clauseId: "c", code: "adapter-error", message: `failure of a${n}`, path: "src/a.ts" }],
      });
      client.prepare("UPDATE audits SET started_at = ? WHERE audit_id = ?").run(n * 1_000, auditId);
      client.prepare(`INSERT INTO repository_audits (workspace_root, session_id, audit_id, status, started_at)
        VALUES (?, 'session-a', ?, 'completed', ?)`).run(`/w${n}`, auditId, n * 1_000);
      return auditId;
    });
  } finally {
    client.close();
    store.close();
  }
}

/**
 * Writes `text` over one audit's `column`, as a damaged file leaves it: the triggers that read the column on an update
 * (the history's) are set aside for the one write and put back, and the CHECKs are off for it, as the issue's steps do.
 */
function damage(path: string, auditId: string, column: string, text: string): void {
  const client = new Database(path);
  try {
    client.pragma("ignore_check_constraints = ON");
    const triggers = client.prepare("SELECT name, sql FROM sqlite_master WHERE type = 'trigger' AND tbl_name = 'audits' AND sql LIKE ?")
      .all(`%${column}%`) as Array<{ name: string; sql: string }>;
    client.transaction(() => {
      for (const trigger of triggers) client.exec(`DROP TRIGGER "${trigger.name}"`);
      client.prepare(`UPDATE audits SET ${column} = ? WHERE audit_id = ?`).run(text, auditId);
      for (const trigger of triggers) client.exec(trigger.sql);
    })();
  } finally {
    client.close();
  }
}

/** One page of the history as the walk saw it: the violations' ids, its total and its cursor. */
interface HistoryPage { ids: string[]; total: number; nextCursor: string | null }

/** Every page of the session's history, `limit` at a time, walked by cursor. */
function walk(reader: AuditReader, limit: number): HistoryPage[] {
  const pages: HistoryPage[] = [];
  let cursor: string | undefined;
  do {
    const page = reader.listViolationHistory({ since: 0, sessionId: "session-a", limit, ...(cursor === undefined ? {} : { cursor }) });
    pages.push({ ids: page.violations.map(entry => entry.violation.id), total: page.total, nextCursor: page.nextCursor });
    cursor = page.nextCursor ?? undefined;
  } while (cursor !== undefined);
  return pages;
}

/** Each page size, so the damaged rows fall at a page's start, inside it, at its end and on the row past it. */
const LIMITS = [1, 2, 3, 4, 5, 6, 7, 11, 12, 13];

describe("the violation history with an audit whose stored violations do not read", () => {
  it.each([
    ["does not parse", '[{"ruleId":"x"', ["a3-0", "a3-1", "a3-2"]],
    ["holds another JSON value in one place", JSON.stringify([violation("a3-0"), null, violation("a3-2")]), ["a3-1"]],
    ["holds a string in one place", JSON.stringify([violation("a3-0"), violation("a3-1"), "a3-2"]), ["a3-2"]],
  ])("answers every page when one audit's violations_json %s, leaving out what does not read, and every other row on the page, at the cursor and with the total it has undamaged", async (_, text, lost) => {
    const path = await temporaryDatabase();
    const auditIds = fourAudits(path);
    const store = new SqliteAuditStore(path);
    const undamaged = new Map(LIMITS.map(limit => [limit, walk(store, limit)]));
    store.close();
    expect(undamaged.get(13)).toEqual([{ ids: ["a4-0", "a4-1", "a4-2", "a3-0", "a3-1", "a3-2", "a2-0", "a2-1", "a2-2", "a1-0", "a1-1", "a1-2"], total: 12, nextCursor: null }]);
    damage(path, auditIds[2]!, "violations_json", text);
    const stderr = spyOnStderr();
    const damagedStore = new SqliteAuditStore(path);
    const reader = new SqliteAuditReader(path);
    try {
      for (const limit of LIMITS) {
        const expected = undamaged.get(limit)!.map(page => ({ ...page, ids: page.ids.filter(id => !lost.includes(id)) }));
        expect(walk(damagedStore, limit), `limit ${limit}`).toEqual(expected);
        expect(walk(reader, limit), `limit ${limit}, read-only`).toEqual(expected);
      }
      // One line for the audit, however many pages and readers met it; the column is the schema's.
      const lines = stderr.logged().split("\n").filter(line => line.includes(auditIds[2]!));
      expect(lines).toEqual([`rerail: ${path}: the violation history leaves out violations of audit ${auditIds[2]}: the violations_json `
        + "of its row holds no JSON object for them, which only a damaged file or a hand edit leaves."]);
    } finally {
      reader.close();
      damagedStore.close();
      stderr.restore();
    }
  });

  it("writes the line to the log a read-only reader is given, and reads the damaged text where it lay", async () => {
    const path = await temporaryDatabase();
    const auditIds = fourAudits(path);
    damage(path, auditIds[0]!, "violations_json", "[{");
    const lines: string[] = [];
    const reader = new SqliteAuditReader(path, { log: { write: (line: string) => lines.push(line) > 0 } });
    try {
      expect(walk(reader, 20)[0]!.ids).toEqual(["a4-0", "a4-1", "a4-2", "a3-0", "a3-1", "a3-2", "a2-0", "a2-1", "a2-2"]);
      expect(lines).toHaveLength(1);
      expect(lines[0]).toContain(`violations of audit ${auditIds[0]}`);
    } finally {
      reader.close();
    }
    const client = new Database(path, { readonly: true });
    try {
      expect(client.prepare("SELECT violations_json FROM audits WHERE audit_id = ?").pluck().get(auditIds[0])).toBe("[{");
    } finally {
      client.close();
    }
  });
});

describe("the violation history's cost", () => {
  it("tests each audit's text once a page, and reads each row with one JSON call", async () => {
    const path = await temporaryDatabase();
    fourAudits(path);
    const store = new SqliteAuditStore(path);
    const probe = new Database(path, { readonly: true });
    const reads = vi.spyOn(Object.getPrototypeOf(probe.prepare("SELECT 1")) as Database.Statement, "all");
    let statements: Array<{ source: string; params: unknown[] }> = [];
    try {
      expect(walk(store, 5).flatMap(page => page.ids)).toHaveLength(12);
      statements = reads.mock.contexts.map((statement, index) => ({ source: (statement as Database.Statement).source, params: reads.mock.calls[index]! }))
        .filter(statement => statement.source.includes("audit_violations") && statement.source.includes("json_valid"));
    } finally {
      reads.mockRestore();
      store.close();
    }
    try {
      expect(statements).toHaveLength(3);
      const { source, params } = statements[0]!;
      // Each JSON call on a row is one more pass over the audit's whole text: one test of the text, made once for each
      // audit on the page in a row SQLite makes once, and one read of each row's place.
      const calls = (probe.prepare(`EXPLAIN ${source}`).all(...params) as Array<{ p4: unknown }>)
        .map(step => String(step.p4)).filter(p4 => /^(json\w*|->>?)\(/u.test(p4));
      expect(calls.sort()).toEqual(["->(2)", "json_valid(1)"]);
      const plan = probe.prepare(`EXPLAIN QUERY PLAN ${source}`).all(...params) as Array<{ detail: string }>;
      expect(plan.map(step => step.detail)).toContain("MATERIALIZE held");
    } finally {
      probe.close();
    }
  });
});

/** The failure an audit's record lists before its own for a stored list it reads as none, as #989 made it. */
const unreadable = (clauseId: string, name: string) => ({
  clauseId, code: "adapter-error", stage: "persistence",
  message: `The audit's ${name} in the database are not a JSON array of objects, which only a damaged file or a hand edit leaves, so they read as none.`,
});

describe("the repository audit pages with an audit whose stored lists do not read", () => {
  it.each([
    // The page orders failures by code, then path: the one that marks a list names no path, so it comes first.
    ["violations_json", "violations", "violations", { violationCount: 0, failureCount: 2 }, ["violations", "failure of a3"]],
    ["failures_json", "failures", "failures", { violationCount: 3, failureCount: 1 }, ["a3-0", "a3-1", "a3-2", "failures"]],
    ["rule_revisions_json", "rule-revisions", "rule revisions", { ruleRevisionCount: 0, violationCount: 3, failureCount: 2 },
      ["a3-0", "a3-1", "a3-2", "rule-revisions", "failure of a3"]],
  ])("answers the list and the audit's entries when its %s does not parse, counting and listing it as its record does", async (column, clauseId, name, counts, entries) => {
    const path = await temporaryDatabase();
    const auditIds = fourAudits(path);
    damage(path, auditIds[2]!, column, '[{"ruleId":"x"');
    const stderr = spyOnStderr();
    const store = new SqliteAuditStore(path);
    const reader = new SqliteAuditReader(path);
    try {
      for (const source of [store, reader]) {
        const listed = source.listRepositoryAudits({ limit: 10 });
        expect(listed.total).toBe(4);
        const audit = (root: string) => listed.repositories.find(repository => repository.workspaceRoot === root)?.audit;
        expect(audit("/w3")).toMatchObject(counts);
        // The record counts the same, and every other audit as stored.
        expect(source.get(auditIds[2]!)).toMatchObject({ failureCount: counts.failureCount, violations: counts.violationCount === 0 ? [] : expect.any(Array) });
        expect(audit("/w2")).toMatchObject({ ruleRevisionCount: 1, violationCount: 3, failureCount: 1 });
        const page = source.listRepositoryAuditEntries({ auditId: auditIds[2]!, limit: 20 })!;
        expect(page.total).toBe(entries.length);
        expect(page.entries.map(({ payload }) => {
          const entry = payload as { id?: string; message: string; clauseId: string };
          return entry.id ?? (entry.message.startsWith("failure of") ? entry.message : entry.clauseId);
        })).toEqual(entries);
        expect(page.entries.find(entry => (entry.payload as { stage?: string }).stage === "persistence")?.payload)
          .toEqual(unreadable(clauseId, name));
        expect(source.listRepositoryAuditEntries({ auditId: auditIds[1]!, limit: 20 })!.total).toBe(4);
      }
    } finally {
      reader.close();
      store.close();
      stderr.restore();
    }
  });

  it("pages, filters and counts an intact audit's entries as before", async () => {
    const path = await temporaryDatabase();
    const auditIds = fourAudits(path);
    const store = new SqliteAuditStore(path);
    try {
      const ids = (query: Partial<Parameters<AuditReader["listRepositoryAuditEntries"]>[0]>) => {
        const page = store.listRepositoryAuditEntries({ auditId: auditIds[0]!, ...query })!;
        return [page.total, page.entries.map(({ kind, payload }) => (kind === "violation" ? (payload as { id: string }).id : kind))];
      };
      expect(ids({ limit: 20 })).toEqual([4, ["a1-0", "a1-1", "a1-2", "failure"]]);
      expect(ids({ limit: 2, offset: 1 })).toEqual([4, ["a1-1", "a1-2"]]);
      expect(ids({ kind: "failure" })).toEqual([1, ["failure"]]);
      expect(ids({ q: "a1-2" })).toEqual([1, ["a1-2"]]);
    } finally {
      store.close();
    }
  });
});
