import { mkdir, mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import Database from "better-sqlite3";
import { afterEach, describe, expect, it } from "vitest";
import { SqliteAuditControlStore } from "../../../src/engine/audit/audit-control.js";
import { SqliteAuditStore, type RuleActivityTotals } from "../../../src/engine/audit/audit-store.js";
import type { CodeRuleViolation } from "../../../src/engine/code-rule/code-rule.js";
import { openRerailDatabase, resetRepositoryHistory } from "../../../src/engine/storage/database.js";
import { RERAIL_BASELINE_SCHEMA_VERSION } from "../../../src/engine/storage/migrations.js";
import { RERAIL_BASELINE_SCHEMA_STATEMENTS } from "../../../src/engine/storage/baseline-schema.js";
import { SqliteSessionReader, SqliteSessionStore } from "../../../src/runtime/index.js";

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
    WITH in_range AS (SELECT audit_id, session_id, started_at, mode, violations_json, rule_revisions_json FROM audits audit
      WHERE started_at >= ? AND ${scopeSql(repositories)}),
    violations AS (
      SELECT json_extract(violation.value, '$.ruleId') AS ruleId,
        count(DISTINCT ${KEY("violation.value")}) AS distinctViolations,
        count(DISTINCT CASE WHEN audit.mode = 'log' OR json_extract(violation.value, '$.mode') = 'log' THEN ${KEY("violation.value")} END) AS loggedViolations,
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
      SELECT CASE WHEN audit.mode = 'log' OR max(coalesce(json_extract(violation.value, '$.mode') = 'log', 0))
          OVER (PARTITION BY audit.audit_id, json_extract(violation.value, '$.ruleId'), ${KEY("violation.value")}) = 1
        THEN json_set(violation.value, '$.mode', 'log') ELSE violation.value END AS payload,
        audit.audit_id AS auditId, audit.session_id AS sessionId, audit.started_at AS at,
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
 * interruption, a scan's cumulative progress), plain SQL the way an older Rerail sharing the file writes, and audits
 * re-dated after the fact.
 */
function writeHistory(path: string, client: Database.Database, seed: number): void {
  const next = random(seed);
  const store = new SqliteAuditStore(path);
  try {
    for (let index = 0; index < 60; index += 1) {
      const sessionId = SESSIONS[Math.floor(next() * SESSIONS.length)]![0];
      const way = Math.floor(next() * 5);
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
      } else if (way === 3) {
        // A scan: each progress write repeats everything so far and adds more, copies of earlier entries included.
        let soFar: CodeRuleViolation[] = [];
        for (let chunk = 0; chunk < 4; chunk += 1) {
          soFar = [...soFar, ...violations(next)];
          store.progress(auditId, { violations: soFar });
        }
        store.finish(auditId, { status: "completed", violations: soFar, failures: [] });
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
  it.each([1, 2, 3, 4, 5, 6])("answers activity and history as reading every audit would (seed %i)", async (seed) => {
    const path = await temporaryDatabase();
    const database = openRerailDatabase(path);
    try {
      openSessions(path);
      writeHistory(path, database.$client, seed);
      expectMatchesOracle(path, database.$client);
      expect(database.$client.prepare("SELECT count(*) AS n FROM audit_violations_staging").get()).toEqual({ n: 0 });

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

  // A violation reported twice in one audit, steered then logged, is logged: the copy kept is the first, but every copy's mode counts.
  it.each(["in one write", "appended by a later progress write"])("counts a violation as logged when a later copy in the same audit was logged (%s)", async (how) => {
    const path = await temporaryDatabase();
    const database = openRerailDatabase(path);
    const store = new SqliteAuditStore(path);
    try {
      openSessions(path);
      const auditId = store.start({
        sessionId: "s1", workspaceRoot: "/w", scope: "repository", mode: "steer", revision: 1, catalogRevision: null,
        ruleRevisions: [{ ruleId: "rule-a", revision: 1 }], paths: [],
      });
      const copy = (mode: "steer" | "log", id: string) => ({
        id, ruleId: "rule-a", clauseId: "clause", code: "even", mode, message: "m", path: "src/a.ts", captures: {}, variables: {},
      }) as CodeRuleViolation;
      if (how === "in one write") store.finish(auditId, { status: "completed", violations: [copy("steer", "first"), copy("log", "second")], failures: [] });
      else {
        store.progress(auditId, { violations: [copy("steer", "first")] });
        store.progress(auditId, { violations: [copy("steer", "first"), copy("log", "second")] });
      }
      expect(store.ruleActivity(0).get("rule-a")).toMatchObject({ distinctViolations: 1, loggedViolations: 1 });
      expect(store.ruleActivity(0)).toEqual(oracleActivity(database.$client, 0));
      // The first copy is the one history shows.
      expect(store.listViolationHistory({ since: 0 }).violations.map(entry => entry.violation.id)).toEqual(["first"]);
    } finally {
      store.close();
      database.$client.close();
    }
  });

  describe("a Log-mode audit", () => {
    const steerRule = { id: "v", ruleId: "rule-a", clauseId: "clause", code: "even", mode: "steer", message: "m", path: "src/a.ts", captures: {}, variables: {} } as CodeRuleViolation;
    const logAudit = (store: SqliteAuditStore): void => {
      const auditId = store.start({
        sessionId: "s1", workspaceRoot: "/w", scope: "repository", mode: "log", revision: 1, catalogRevision: null,
        ruleRevisions: [{ ruleId: "rule-a", revision: 1 }], paths: [],
      });
      store.finish(auditId, { status: "completed", violations: [steerRule] });
    };

    it("records a Steer rule's violation as logged, because it told the agent nothing", async () => {
      const path = await temporaryDatabase();
      const database = openRerailDatabase(path);
      const store = new SqliteAuditStore(path);
      const sessions = new SqliteSessionReader(path);
      try {
        openSessions(path);
        logAudit(store);
        expect(store.ruleActivity(0).get("rule-a")).toMatchObject({ distinctViolations: 1, loggedViolations: 1 });
        expect(store.listViolationHistory({ since: 0 }).violations[0]!.violation.mode).toBe("log");
        expect(sessions.sessionPage({ limit: 50 }).sessions.find(row => row.sessionId === "s1")).toMatchObject({ violationCount: 1, steeredViolations: 0 });
      } finally {
        sessions.close();
        store.close();
        database.$client.close();
      }
    });

    it("is corrected when a file written with the earlier triggers is upgraded, and new audits use the new ones", async () => {
      const path = await temporaryDatabase();
      const database = openRerailDatabase(path);
      const store = new SqliteAuditStore(path);
      try {
        openSessions(path);
        logAudit(store);
        // As an earlier build left it: the row logged by the rule's mode, the trigger not reading the audit's, the step not run.
        const client = database.$client;
        client.exec(`UPDATE audit_violations SET logged = 0; UPDATE rule_violation_rollup SET last_logged_at = NULL;
          DROP TRIGGER audits_history_insert; CREATE TRIGGER audits_history_insert AFTER INSERT ON audits BEGIN SELECT 1; END;
          DELETE FROM schema_migrations WHERE id = 'audit-history-log-mode-audits';`);
      } finally {
        store.close();
        database.$client.close();
      }
      const upgraded = openRerailDatabase(path);
      const after = new SqliteAuditStore(path);
      try {
        expect(after.ruleActivity(0).get("rule-a")).toMatchObject({ distinctViolations: 1, loggedViolations: 1 });
        const trigger = upgraded.$client.prepare("SELECT sql FROM sqlite_master WHERE name = 'audits_history_insert'").get() as { sql: string };
        expect(trigger.sql).toContain("NEW.mode = 'log'");
      } finally {
        after.close();
        upgraded.$client.close();
      }
    });
  });

  describe("a repository's baseline scan", () => {
    const found = (id: string) => ({ id, ruleId: "rule-a", clauseId: "clause", code: "even", mode: "steer", message: "m", path: `src/${id}.ts`, captures: {}, variables: {} }) as CodeRuleViolation;
    const audit = (store: SqliteAuditStore, mode: "log" | "steer", ...ids: string[]): string => {
      const auditId = store.start({
        sessionId: "s1", workspaceRoot: "/w", scope: mode === "log" ? "repository" : "touched-files", mode, revision: 1, catalogRevision: null,
        ruleRevisions: [{ ruleId: "rule-a", revision: 1 }], paths: [],
      });
      store.finish(auditId, { status: "completed", violations: ids.map(found) });
      return auditId;
    };

    /** The session that happened to start first, holding the repository's baseline; `work` is what it found itself. */
    async function scanned(work: readonly string[]) {
      const path = await temporaryDatabase();
      const database = openRerailDatabase(path);
      const store = new SqliteAuditStore(path);
      const controls = new SqliteAuditControlStore(path);
      const sessions = new SqliteSessionReader(path);
      openSessions(path);
      controls.finishRepositoryAudit("s1", "/w", audit(store, "log", "old-1", "old-2"), "completed");
      if (work.length > 0) audit(store, "steer", ...work);
      const row = () => sessions.sessionPage({ limit: 50 }).sessions.find(session => session.sessionId === "s1")!;
      const close = () => { sessions.close(); controls.close(); store.close(); database.$client.close(); };
      return { store, row, close };
    }

    it("is not the work of the session that happened to start first", async () => {
      const { store, row, close } = await scanned([]);
      try {
        expect(row()).toMatchObject({ violationCount: 0, steeredViolations: 0 });
        expect(store.listViolationHistory({ since: 0, sessionId: "s1" })).toMatchObject({ total: 0, violations: [] });
        expect(store.sessionsWithViolations(0)).toBe(0);
      } finally {
        close();
      }
    });

    it("leaves what that session found itself counted", async () => {
      const { store, row, close } = await scanned(["mine"]);
      try {
        expect(row()).toMatchObject({ violationCount: 1, steeredViolations: 1 });
        expect(store.listViolationHistory({ since: 0, sessionId: "s1" }).violations.map(entry => entry.violation.id)).toEqual(["mine"]);
        expect(store.sessionsWithViolations(0)).toBe(1);
      } finally {
        close();
      }
    });

    it("stays in the rule's history, marked as the baseline, and is counted for the repository", async () => {
      const { store, close } = await scanned(["mine"]);
      try {
        const history = store.listViolationHistory({ since: 0, ruleId: "rule-a" });
        expect(history.violations.map(entry => [entry.violation.id, entry.baseline])).toEqual([["mine", false], ["old-1", true], ["old-2", true]]);
        expect(store.baselineViolations(0)).toBe(2);
        expect(store.baselineViolations(0, [REPOSITORY_B])).toBe(0);
      } finally {
        close();
      }
    });

    it("marks the baseline a file already holds when it is upgraded, not a scan someone asked for", async () => {
      const path = await temporaryDatabase();
      const database = openRerailDatabase(path);
      const store = new SqliteAuditStore(path);
      let baseline: string;
      try {
        openSessions(path);
        baseline = audit(store, "log", "old");
        const asked = audit(store, "log", "asked");
        const link = database.$client.prepare(`INSERT INTO repository_audits (workspace_root, session_id, audit_id, status, started_at)
          VALUES (?, 's1', ?, 'completed', 1)`);
        link.run("/w", baseline);
        link.run("/w2", asked);
        database.$client.prepare(`INSERT INTO audit_requests (id, session_id, workspace_root, scope, mode, paths_json, status, created_at, audit_ids_json)
          VALUES ('r', 's1', '/w2', 'repository', 'log', '[]', 'completed', 1, json_array(?))`).run(asked);
        database.$client.exec("UPDATE audits SET baseline = 0; DELETE FROM schema_migrations WHERE id = 'audits-baseline';");
      } finally {
        store.close();
        database.$client.close();
      }
      const upgraded = openRerailDatabase(path);
      try {
        expect(upgraded.$client.prepare("SELECT audit_id AS auditId FROM audits WHERE baseline = 1").all()).toEqual([{ auditId: baseline }]);
      } finally {
        upgraded.$client.close();
      }
    });
  });

  it("follows an older writer that replaces an audit with INSERT OR REPLACE, renames it, then deletes it", async () => {
    const path = await temporaryDatabase();
    const database = openRerailDatabase(path);
    const client = database.$client;
    try {
      openSessions(path);
      const write = (verb: string, auditId: string, entries: CodeRuleViolation[]) => client.prepare(`${verb} INTO audits (audit_id, session_id,
        workspace_root, scope, mode, revision, catalog_revision, rule_revisions_json, rule_provenance_json, paths_json, started_at,
        finished_at, status, violations_json, failures_json)
        VALUES (?, 's1', '/w', 'touched-files', 'steer', 1, NULL, '[{"ruleId":"rule-a","revision":1}]', '[]', '[]', 5000, NULL, 'completed', ?, '[]')`)
        .run(auditId, JSON.stringify(entries));
      const flagged = (ruleId: string) => ({ id: ruleId, ruleId, clauseId: "c", code: "X", mode: "steer", message: "m", path: "a.ts", captures: {}, variables: {} }) as CodeRuleViolation;
      const derived = (table: string) => client.prepare(`SELECT count(*) AS n FROM ${table}`).get();
      const rollupRules = () => client.prepare("SELECT rule_id FROM rule_violation_rollup ORDER BY rule_id").pluck().all();

      write("INSERT", "a1", [flagged("rule-a")]);
      write("INSERT OR REPLACE", "a1", []);
      expectMatchesOracle(path, client);
      expect([derived("audit_violations"), rollupRules()]).toEqual([{ n: 0 }, []]);

      write("INSERT OR REPLACE", "a1", [flagged("rule-b")]);
      expectMatchesOracle(path, client);
      expect(rollupRules()).toEqual(["rule-b"]);

      client.prepare("UPDATE audits SET audit_id = 'a2' WHERE audit_id = 'a1'").run();
      expectMatchesOracle(path, client);
      expect(client.prepare("SELECT DISTINCT audit_id FROM audit_violations UNION SELECT audit_id FROM audit_rule_set_uses").pluck().all()).toEqual(["a2"]);

      client.prepare("DELETE FROM audits").run();
      for (const table of ["audit_violations", "audit_violations_staging", "rule_violation_rollup", "audit_rule_set_uses"]) {
        expect(derived(table), table).toEqual({ n: 0 });
      }
    } finally {
      database.$client.close();
    }
  });

  it("counts a session whose only violations name no rule among the sessions with violations", async () => {
    const path = await temporaryDatabase();
    const database = openRerailDatabase(path);
    const store = new SqliteAuditStore(path);
    try {
      openSessions(path);
      const auditId = store.start({
        sessionId: "s3", workspaceRoot: "/w", scope: "repository", mode: "steer", revision: 1, catalogRevision: null, ruleRevisions: [], paths: [],
      });
      store.finish(auditId, { status: "completed", violations: [{ id: "no-rule", clauseId: "c", code: "X", mode: "steer", message: "m", captures: {}, variables: {} } as unknown as CodeRuleViolation], failures: [] });

      expect(store.sessionsWithViolations(0)).toBe(1);
      expect(store.sessionsWithViolations(0, [REPOSITORY_B])).toBe(1);
      expect(store.sessionsWithViolations(0, [REPOSITORY_A])).toBe(0);
      expect(store.listViolationHistory({ since: 0, sessionId: "s3" }).total).toBe(1);
    } finally {
      store.close();
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
