import { mkdir, mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import Database from "better-sqlite3";
import { afterEach, describe, expect, it } from "vitest";
import { SqliteAuditStore, type RuleActivityTotals } from "../../../src/engine/audit/audit-store.js";
import type { CodeRuleViolation } from "../../../src/engine/code-rule/code-rule.js";
import { openRerailDatabase, resetRepositoryHistory } from "../../../src/engine/storage/database.js";
import { RERAIL_BASELINE_SCHEMA_VERSION } from "../../../src/engine/storage/migrations.js";
import { RERAIL_BASELINE_SCHEMA_STATEMENTS } from "../../../src/engine/storage/baseline-schema.js";
import { SqliteSessionStore } from "../../../src/runtime/session-store.js";

const temporaryDirectories: string[] = [];

afterEach(async () => {
  await Promise.all(temporaryDirectories.splice(0).map(path => rm(path, { recursive: true, force: true })));
});

async function temporaryDatabase(): Promise<string> {
  const root = await mkdtemp(join(tmpdir(), "rerail-audit-history-"));
  temporaryDirectories.push(root);
  const path = join(root, "home", "rerail.sqlite");
  await mkdir(dirname(path), { recursive: true });
  return path;
}

/*
 * The oracle: history read straight from `audits`, the way it was before the derived tables. Every answer the
 * derived tables give must equal it, whoever wrote the audits and however they changed since.
 */
const KEY = (value: string): string => `coalesce(json_extract(${value}, '$.clauseId'), '') || char(31)
  || coalesce(json_extract(${value}, '$.path'), '') || char(31)
  || coalesce(json_extract(${value}, '$.span'), '') || char(31)
  || coalesce(json_extract(${value}, '$.code'), '')`;

const scopeSql = (repositories: readonly string[] | undefined): string => repositories === undefined
  ? "1 = 1"
  : `audit.session_id IN (SELECT session_id FROM sessions WHERE repository_id IN (${repositories.map(id => `'${id}'`).join(", ")}))`;

function oracleActivity(client: Database.Database, since: number, repositories?: readonly string[]): Map<string, RuleActivityTotals> {
  const rows = client.prepare(`
    WITH in_range AS (SELECT audit_id, session_id, started_at, violations_json, rule_revisions_json FROM audits audit
      WHERE started_at >= ? AND ${scopeSql(repositories)}),
    violations AS (
      SELECT json_extract(violation.value, '$.ruleId') AS ruleId,
        count(DISTINCT ${KEY("violation.value")}) AS distinctViolations,
        count(DISTINCT CASE WHEN json_extract(violation.value, '$.mode') = 'log' THEN ${KEY("violation.value")} END) AS loggedViolations,
        count(DISTINCT audit.session_id) AS sessions, max(audit.started_at) AS lastSeenAt
      FROM in_range audit, json_each(audit.violations_json) violation GROUP BY ruleId),
    audited AS (
      SELECT json_extract(revision.value, '$.ruleId') AS ruleId, count(DISTINCT audit.audit_id) AS audits
      FROM in_range audit, json_each(audit.rule_revisions_json) revision GROUP BY ruleId),
    measured AS (SELECT ruleId FROM violations UNION SELECT ruleId FROM audited)
    SELECT measured.ruleId AS ruleId, coalesce(violations.distinctViolations, 0) AS distinctViolations,
      coalesce(violations.sessions, 0) AS sessions, violations.lastSeenAt AS lastSeenAt,
      coalesce(audited.audits, 0) AS audits, coalesce(violations.loggedViolations, 0) AS loggedViolations
    FROM measured LEFT JOIN violations ON violations.ruleId = measured.ruleId LEFT JOIN audited ON audited.ruleId = measured.ruleId
    WHERE measured.ruleId IS NOT NULL
  `).all(since) as Array<RuleActivityTotals & { ruleId: string }>;
  return new Map(rows.map(({ ruleId, ...totals }) => [ruleId, totals]));
}

interface HistoryRow { auditId: string; sessionId: string; at: number; ruleId: string | null; payload: string }

function oracleHistory(client: Database.Database, since: number, repositories?: readonly string[]): HistoryRow[] {
  return client.prepare(`
    SELECT payload, auditId, sessionId, at, ruleId FROM (
      SELECT violation.value AS payload, audit.audit_id AS auditId, audit.session_id AS sessionId, audit.started_at AS at,
        json_extract(violation.value, '$.ruleId') AS ruleId,
        row_number() OVER (PARTITION BY audit.audit_id, json_extract(violation.value, '$.ruleId'), ${KEY("violation.value")} ORDER BY violation.key) AS copy
      FROM audits audit, json_each(audit.violations_json) violation
      WHERE audit.started_at >= ? AND ${scopeSql(repositories)}
    ) WHERE copy = 1
  `).all(since) as HistoryRow[];
}

/** Every page of the derived history, walked by cursor, as the oracle's rows. */
function pagedHistory(store: SqliteAuditStore, query: { since: number; ruleId?: string; sessionId?: string }, repositories?: readonly string[]) {
  const rows: HistoryRow[] = [];
  const totals = new Set<number>();
  let cursor: string | undefined;
  for (;;) {
    const page = store.listViolationHistory({ ...query, limit: 7, ...(cursor === undefined ? {} : { cursor }) }, repositories);
    totals.add(page.total);
    rows.push(...page.violations.map(({ violation, auditId, sessionId, at }) => ({
      auditId, sessionId, at, ruleId: violation.ruleId ?? null, payload: JSON.stringify(violation),
    })));
    if (page.nextCursor === null) return { rows, totals };
    cursor = page.nextCursor;
  }
}

const rowKey = (row: HistoryRow): string => `${row.auditId}|${row.at}|${JSON.stringify(JSON.parse(row.payload))}`;

/** Deterministic, so a failure reproduces. */
function random(seed: number): () => number {
  let state = seed;
  return () => {
    state = (state * 1_103_515_245 + 12_345) % 2_147_483_648;
    return state / 2_147_483_648;
  };
}

const RULES = ["rule-a", "rule-b", "rule-c", "rule-d"] as const;
const REPOSITORY_A = "remote:example/a";
const REPOSITORY_B = "remote:example/b";
const SESSIONS = [["s1", REPOSITORY_A], ["s2", REPOSITORY_A], ["s3", REPOSITORY_B]] as const;

/** Violations drawn from a small pool, so audits repeat each other and themselves; some carry no rule id. */
function violations(next: () => number): CodeRuleViolation[] {
  return Array.from({ length: Math.floor(next() * 6) }, () => {
    const index = Math.floor(next() * 8);
    const ruleId = index === 7 ? undefined : RULES[index % RULES.length]!;
    return {
      id: `violation-${index}-${Math.floor(next() * 3)}`,
      ...(ruleId === undefined ? {} : { ruleId }),
      clauseId: "clause",
      code: index % 2 === 0 ? "even" : "odd",
      mode: next() < 0.3 ? "log" : "steer",
      message: "message",
      path: `src/file-${index % 3}.ts`,
      span: { startByte: index, endByte: index + 1, startLine: 1, startColumn: index, endLine: 1, endColumn: index + 1 },
      captures: {},
      variables: {},
    } as CodeRuleViolation;
  });
}

const revisions = (next: () => number) => RULES.filter(() => next() < 0.6).map(ruleId => ({ ruleId, revision: 1 }));

/**
 * Audits written every way history gets written: this build's store (start, progress, finish, paged results,
 * interruption), plain SQL the way an older Rerail sharing the file writes, and audits re-dated after the fact.
 */
function writeHistory(path: string, client: Database.Database, seed: number): void {
  const next = random(seed);
  const store = new SqliteAuditStore(path);
  try {
    for (let index = 0; index < 60; index += 1) {
      const sessionId = SESSIONS[Math.floor(next() * SESSIONS.length)]![0];
      const way = Math.floor(next() * 4);
      if (way === 0) {
        client.prepare(`INSERT INTO audits (audit_id, session_id, workspace_root, scope, mode, revision, catalog_revision,
          rule_revisions_json, rule_provenance_json, paths_json, started_at, finished_at, status, violations_json, failures_json)
          VALUES (?, ?, '/w', 'touched-files', 'steer', 1, NULL, ?, '[]', '[]', ?, NULL, 'completed', ?, '[]')`)
          .run(`older-${index}`, sessionId, JSON.stringify(revisions(next)), Math.floor(next() * 10_000), JSON.stringify(violations(next)));
        continue;
      }
      const auditId = store.start({
        sessionId, workspaceRoot: "/w", scope: "repository", mode: "steer", revision: 1, catalogRevision: null,
        ruleRevisions: revisions(next), paths: [], ruleProvenance: [],
      });
      if (way === 1) {
        store.progress(auditId, { violations: violations(next) });
        store.finish(auditId, { status: next() < 0.2 ? "interrupted" : "completed", violations: violations(next), failures: [] });
      } else if (way === 2) {
        store.appendResultPage(auditId, 0, { violations: violations(next), failures: [] });
        store.appendResultPage(auditId, 1, { violations: violations(next), failures: [] });
        store.finishResultPages(auditId, "completed");
      } else {
        // Left running: its progress so far is history too.
        store.progress(auditId, { violations: violations(next) });
      }
      client.prepare("UPDATE audits SET started_at = ? WHERE audit_id = ?").run(Math.floor(next() * 10_000), auditId);
    }
  } finally {
    store.close();
  }
}

function expectMatchesOracle(path: string, client: Database.Database): void {
  const store = new SqliteAuditStore(path);
  try {
    for (const repositories of [undefined, [REPOSITORY_A], [REPOSITORY_B], []] as const) {
      for (const since of [0, 2_500, 9_000, 20_000]) {
        expect(store.ruleActivity(since, repositories), `activity since ${since} in ${repositories}`).toEqual(oracleActivity(client, since, repositories));
        const expected = oracleHistory(client, since, repositories);
        const { rows, totals } = pagedHistory(store, { since }, repositories);
        expect([...totals]).toEqual([expected.length]);
        expect(rows.map(rowKey).sort()).toEqual(expected.map(rowKey).sort());
        expect(new Set(rows.map(rowKey)).size).toBe(rows.length);
        expect(rows.map(row => row.at)).toEqual(rows.map(row => row.at).sort((a, b) => b - a));
        expect(store.sessionsWithViolations(since, repositories)).toBe(new Set(expected.map(row => row.sessionId)).size);
        for (const ruleId of ["rule-a", "rule-d"]) {
          expect(pagedHistory(store, { since, ruleId }, repositories).rows.map(rowKey).sort())
            .toEqual(expected.filter(row => row.ruleId === ruleId).map(rowKey).sort());
        }
        expect(pagedHistory(store, { since, sessionId: "s2" }, repositories).rows.map(rowKey).sort())
          .toEqual(expected.filter(row => row.sessionId === "s2").map(rowKey).sort());
      }
    }
  } finally {
    store.close();
  }
}

function openSessions(path: string): void {
  const sessions = new SqliteSessionStore(path);
  try {
    for (const [sessionId, repository] of SESSIONS) sessions.openSession({ host: "codex", sessionId, cwd: "/w" }, 1, repository);
    for (const [sessionId] of SESSIONS) sessions.closeSession(sessionId, 2);
  } finally {
    sessions.close();
  }
}

describe("derived audit history", () => {
  it.each([1, 2, 3])("answers activity and history as reading every audit would (seed %i)", async (seed) => {
    const path = await temporaryDatabase();
    const database = openRerailDatabase(path);
    try {
      openSessions(path);
      writeHistory(path, database.$client, seed);
      expectMatchesOracle(path, database.$client);

      // `rerail reset` deletes one repository's audits; what was derived from them goes too.
      resetRepositoryHistory(path, REPOSITORY_A);
      expectMatchesOracle(path, database.$client);
      for (const table of ["audit_violations", "rule_violation_rollup", "audit_rule_set_uses"]) {
        expect(database.$client.prepare(`SELECT count(*) AS n FROM ${table} WHERE session_id IN ('s1', 's2')`).get(), table).toEqual({ n: 0 });
      }
    } finally {
      database.$client.close();
    }
  });

  it("fills the derived history from the audits a file already holds when it is upgraded, then keeps it current", async () => {
    const path = await temporaryDatabase();
    const created = new Database(path);
    try {
      for (const statement of RERAIL_BASELINE_SCHEMA_STATEMENTS) created.exec(statement);
      created.pragma(`user_version = ${RERAIL_BASELINE_SCHEMA_VERSION}`);
    } finally {
      created.close();
    }
    const baseline = new Database(path);
    try {
      for (const [sessionId, repository] of SESSIONS) {
        baseline.prepare("INSERT INTO sessions (session_id, host, cwd, repository_id, started_at, ended_at) VALUES (?, 'codex', '/w', ?, 1, 2)")
          .run(sessionId, repository);
      }
      // Audits only an older Rerail wrote: the file has no derived tables or triggers yet.
      for (let index = 0; index < 40; index += 1) {
        const next = random(index + 10);
        baseline.prepare(`INSERT INTO audits (audit_id, session_id, workspace_root, scope, mode, revision, catalog_revision,
          rule_revisions_json, rule_provenance_json, paths_json, started_at, finished_at, status, violations_json, failures_json)
          VALUES (?, ?, '/w', 'touched-files', 'steer', 1, NULL, ?, '[]', '[]', ?, NULL, 'completed', ?, '[]')`)
          .run(`before-${index}`, SESSIONS[index % SESSIONS.length]![0], JSON.stringify(revisions(next)), Math.floor(next() * 10_000), JSON.stringify(violations(next)));
      }
    } finally {
      baseline.close();
    }

    const database = openRerailDatabase(path);
    try {
      expectMatchesOracle(path, database.$client);
      writeHistory(path, database.$client, 4);
      expectMatchesOracle(path, database.$client);
    } finally {
      database.$client.close();
    }
  });
});
