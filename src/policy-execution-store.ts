import { randomUUID } from "node:crypto";
import { and, count, desc, eq, sql, type SQL } from "drizzle-orm";
import type { SQLiteColumn } from "drizzle-orm/sqlite-core";
import { LazyReadonlyRerailDatabase, openRerailDatabase, type RerailDatabase } from "./database.js";
import type { ExecutablePolicyFinding } from "./executable-policy.js";
import type {
  PolicyExecutionRecorder,
  PolicyRunInput,
  PolicyRunRecord,
  PolicyRunResult,
  PolicyRunSummary,
  PolicyMode,
  PolicyScope,
  PolicyRunStatus,
} from "./policy-execution.js";
import type { RuleMode } from "./rule-mode.js";
import { policyRepositoryDiagnostics, policyRunResultPages, policyRuns, sessionsOfRepository, type PolicyRunRow } from "./schema.js";

export interface PolicyRunQuery {
  auditId?: string;
  sessionId?: string;
  status?: PolicyRunStatus;
  scope?: PolicyScope;
  mode?: PolicyMode;
  limit?: number;
  offset?: number;
}

export interface PolicyRunPage {
  audits: PolicyRunRecord[];
  total: number;
}

export interface RepositoryDiagnosticSummary {
  workspaceRoot: string;
  repositoryAudit: {
    sessionId: string;
    auditId: string | null;
    status: "running" | "completed" | "failed" | "interrupted";
    startedAt: number;
    finishedAt: number | null;
    error: string | null;
  } | null;
  audit: {
    auditId: string;
    scope: PolicyScope;
    status: PolicyRunStatus;
    startedAt: number;
    finishedAt: number | null;
    revision: number;
    catalogRevision: string | null;
    ruleRevisionCount: number;
    violationCount: number;
    failureCount: number;
  } | null;
}

export interface RepositoryDiagnosticQuery {
  workspaceRoot?: string;
  status?: "running" | "completed" | "failed" | "interrupted";
  limit?: number;
  offset?: number;
}

export interface RepositoryDiagnosticEntryQuery {
  auditId: string;
  kind?: "violation" | "failure";
  /** The mode of the rule that produced a violation; failures never match. */
  mode?: RuleMode;
  code?: string;
  path?: string;
  q?: string;
  limit?: number;
  offset?: number;
}

export interface RepositoryDiagnosticEntry {
  kind: "violation" | "failure";
  payload: unknown;
}

export interface RepositoryDiagnosticPage {
  repositories: RepositoryDiagnosticSummary[];
  total: number;
}

export interface RepositoryDiagnosticEntryPage {
  entries: RepositoryDiagnosticEntry[];
  total: number;
}

/** What one rule's audits in a time range say about it; absent evidence is zeros, never a guess. */
export interface RuleActivityTotals {
  distinctViolations: number;
  sessions: number;
  lastSeenAt: number | null;
  /** Distinct audits in range whose compiled rule set included this rule; 0 means never evaluated. */
  audits: number;
}

export interface FindingHistoryQuery {
  /** Epoch milliseconds; audits started at or after this instant. `0` means every audit. */
  since: number;
  ruleId?: string;
  limit?: number;
  offset?: number;
}

/** One violation with the audit that produced it; `at` is the audit's start. */
export interface FindingHistoryEntry {
  violation: ExecutablePolicyFinding;
  sessionId: string;
  auditId: string;
  at: number;
}

export interface FindingHistoryPage {
  violations: FindingHistoryEntry[];
  total: number;
}

export interface PolicyExecutionRunReader {
  get(runId: string): PolicyRunRecord | undefined;
  list(query?: PolicyRunQuery): PolicyRunPage;
  latestSummary(sessionId: string): PolicyRunSummary | undefined;
  listRepositoryDiagnostics(query?: RepositoryDiagnosticQuery): RepositoryDiagnosticPage;
  listRepositoryDiagnosticEntries(query: RepositoryDiagnosticEntryQuery): RepositoryDiagnosticEntryPage | undefined;
  ruleActivity(since: number): Map<string, RuleActivityTotals>;
  listFindingHistory(query: FindingHistoryQuery): FindingHistoryPage;
}

/** A repository's runs are the runs of its sessions; unscoped (`undefined`) readers see every run. */
type Repository = string | undefined;

function scopedSessions(column: SQLiteColumn, repositoryId: Repository): SQL {
  return repositoryId === undefined ? sql`1 = 1` : sql`${column} IN (${sessionsOfRepository(repositoryId)})`;
}

function conditions(query: PolicyRunQuery, repositoryId: Repository): SQL | undefined {
  return and(
    repositoryId === undefined ? undefined : scopedSessions(policyRuns.sessionId, repositoryId),
    query.auditId === undefined ? undefined : eq(policyRuns.auditId, query.auditId),
    query.sessionId === undefined ? undefined : eq(policyRuns.sessionId, query.sessionId),
    query.status === undefined ? undefined : eq(policyRuns.status, query.status),
    query.scope === undefined ? undefined : eq(policyRuns.scope, query.scope),
    query.mode === undefined ? undefined : eq(policyRuns.mode, query.mode),
  );
}

function paging(query: Pick<PolicyRunQuery, "limit" | "offset">): { limit: number; offset: number } {
  return {
    limit: Math.min(200, Math.max(1, Math.trunc(query.limit ?? 50))),
    offset: Math.max(0, Math.trunc(query.offset ?? 0)),
  };
}

function record(row: PolicyRunRow): PolicyRunRecord {
  return {
    auditId: row.auditId,
    sessionId: row.sessionId,
    workspaceRoot: row.workspaceRoot,
    scope: row.scope,
    mode: row.mode,
    revision: row.revision,
    catalogRevision: row.catalogRevision,
    ruleRevisions: row.ruleRevisions,
    ruleProvenance: row.ruleProvenance,
    paths: row.paths,
    startedAt: row.startedAt,
    finishedAt: row.finishedAt,
    status: row.status,
    violations: row.violations,
    failures: row.failures,
  };
}

function listPolicyRuns(database: RerailDatabase, query: PolicyRunQuery = {}, repositoryId?: Repository): PolicyRunPage {
  const where = conditions(query, repositoryId);
  const { limit, offset } = paging(query);
  const total = database.select({ total: count() }).from(policyRuns).where(where).get()!.total;
  const runs = database
    .select()
    .from(policyRuns)
    .where(where)
    .orderBy(desc(policyRuns.startedAt), desc(policyRuns.auditId))
    .limit(limit)
    .offset(offset)
    .all()
    .map(record);
  return { audits: runs, total };
}

function latestPolicyRunSummary(database: RerailDatabase, sessionId: string): PolicyRunSummary | undefined {
  return database.select({
    workspaceRoot: policyRuns.workspaceRoot,
    scope: policyRuns.scope,
    mode: policyRuns.mode,
    status: policyRuns.status,
    violationCount: sql<number>`json_array_length(${policyRuns.violations})`,
    failureCount: sql<number>`json_array_length(${policyRuns.failures})`,
  }).from(policyRuns).where(eq(policyRuns.sessionId, sessionId))
    .orderBy(desc(policyRuns.startedAt), desc(policyRuns.auditId)).limit(1).get();
}

function listRepositoryDiagnostics(
  database: RerailDatabase,
  query: RepositoryDiagnosticQuery = {},
  repositoryId?: Repository,
): RepositoryDiagnosticPage {
  const { limit, offset } = paging(query);
  const workspaceRoot = query.workspaceRoot ?? null;
  const status = query.status ?? null;
  const rows = database.all<{
    workspaceRoot: string;
    diagnosticSessionId: string | null;
    diagnosticRunId: string | null;
    diagnosticStatus: NonNullable<RepositoryDiagnosticSummary["repositoryAudit"]>["status"] | null;
    diagnosticStartedAt: number | null;
    diagnosticFinishedAt: number | null;
    diagnosticError: string | null;
    runId: string | null;
    runScope: PolicyScope | null;
    runStatus: PolicyRunStatus | null;
    runStartedAt: number | null;
    runFinishedAt: number | null;
    revision: number | null;
    catalogRevision: string | null;
    ruleRevisionCount: number | null;
    violationCount: number | null;
    failureCount: number | null;
    total: number;
  }>(sql`
    WITH repositories AS (
      SELECT workspace_root FROM ${policyRuns} WHERE ${scopedSessions(policyRuns.sessionId, repositoryId)}
      UNION
      SELECT workspace_root FROM ${policyRepositoryDiagnostics} WHERE ${scopedSessions(policyRepositoryDiagnostics.sessionId, repositoryId)}
    )
    SELECT
      repositories.workspace_root AS workspaceRoot,
      diagnostic.session_id AS diagnosticSessionId,
      diagnostic.audit_id AS diagnosticRunId,
      diagnostic.status AS diagnosticStatus,
      diagnostic.started_at AS diagnosticStartedAt,
      diagnostic.finished_at AS diagnosticFinishedAt,
      diagnostic.error AS diagnosticError,
      run.scope AS runScope,
      run.audit_id AS runId,
      run.status AS runStatus,
      run.started_at AS runStartedAt,
      run.finished_at AS runFinishedAt,
      run.revision AS revision,
      run.catalog_revision AS catalogRevision,
      json_array_length(run.rule_revisions_json) AS ruleRevisionCount,
      json_array_length(run.violations_json) AS violationCount,
      json_array_length(run.failures_json) AS failureCount,
      count(*) OVER () AS total
    FROM repositories
    LEFT JOIN ${policyRepositoryDiagnostics} diagnostic ON diagnostic.workspace_root = repositories.workspace_root
    LEFT JOIN ${policyRuns} run ON run.audit_id = diagnostic.audit_id
    WHERE (${workspaceRoot} IS NULL OR repositories.workspace_root = ${workspaceRoot})
      AND (${status} IS NULL OR diagnostic.status = ${status})
    ORDER BY repositories.workspace_root
    LIMIT ${limit} OFFSET ${offset}
  `);
  return {
    repositories: rows.map(row => ({
      workspaceRoot: row.workspaceRoot,
      repositoryAudit: row.diagnosticStatus === null ? null : {
        sessionId: row.diagnosticSessionId!,
        auditId: row.diagnosticRunId,
        status: row.diagnosticStatus,
        startedAt: row.diagnosticStartedAt!,
        finishedAt: row.diagnosticFinishedAt,
        error: row.diagnosticError,
      },
      audit: row.runId === null || row.runStatus === null ? null : {
        scope: row.runScope!,
        auditId: row.runId,
        status: row.runStatus,
        startedAt: row.runStartedAt!,
        finishedAt: row.runFinishedAt,
        revision: row.revision!,
        catalogRevision: row.catalogRevision,
        ruleRevisionCount: row.ruleRevisionCount ?? 0,
        violationCount: row.violationCount ?? 0,
        failureCount: row.failureCount ?? 0,
      },
    })),
    total: rows[0]?.total ?? 0,
  };
}

function listRepositoryDiagnosticEntries(
  database: RerailDatabase,
  query: RepositoryDiagnosticEntryQuery,
): RepositoryDiagnosticEntryPage | undefined {
  const diagnostic = database.select({ auditId: policyRepositoryDiagnostics.auditId }).from(policyRepositoryDiagnostics)
    .where(eq(policyRepositoryDiagnostics.auditId, query.auditId)).get();
  if (diagnostic === undefined) return undefined;
  const { limit, offset } = paging(query);
  const kind = query.kind ?? null;
  const mode = query.mode ?? null;
  const code = query.code?.trim() || null;
  const path = query.path?.trim() || null;
  const text = query.q?.trim() || null;
  const rows = database.all<{ kind: "violation" | "failure"; payload: string; total: number }>(sql`
    WITH entries AS (
      SELECT
        'violation' AS kind,
        finding.value AS payload,
        json_extract(finding.value, '$.mode') AS mode,
        json_extract(finding.value, '$.code') AS code,
        json_extract(finding.value, '$.path') AS path,
        json_extract(finding.value, '$.message') AS message,
        json_extract(finding.value, '$.ruleId') AS ruleId,
        json_extract(finding.value, '$.clauseId') AS clauseId
      FROM ${policyRuns} run, json_each(run.violations_json) finding
      WHERE run.audit_id = ${query.auditId}
      UNION ALL
      SELECT
        'failure' AS kind,
        failure.value AS payload,
        NULL AS mode,
        json_extract(failure.value, '$.code') AS code,
        json_extract(failure.value, '$.path') AS path,
        json_extract(failure.value, '$.message') AS message,
        json_extract(failure.value, '$.ruleId') AS ruleId,
        json_extract(failure.value, '$.clauseId') AS clauseId
      FROM ${policyRuns} run, json_each(run.failures_json) failure
      WHERE run.audit_id = ${query.auditId}
    )
    SELECT kind, payload, count(*) OVER () AS total
    FROM entries
    WHERE (${kind} IS NULL OR kind = ${kind})
      AND (${mode} IS NULL OR mode = ${mode})
      AND (${code} IS NULL OR code = ${code})
      AND (${path} IS NULL OR path LIKE '%' || ${path} || '%')
      AND (${text} IS NULL OR lower(
        coalesce(code, '') || ' ' || coalesce(path, '') || ' ' || coalesce(message, '') || ' ' ||
        coalesce(ruleId, '') || ' ' || coalesce(clauseId, '')
      ) LIKE '%' || lower(${text}) || '%')
    ORDER BY CASE kind WHEN 'violation' THEN 0 ELSE 1 END, code, path
    LIMIT ${limit} OFFSET ${offset}
  `);
  return {
    entries: rows.map(row => ({ kind: row.kind, payload: JSON.parse(row.payload) as unknown })),
    total: rows[0]?.total ?? 0,
  };
}

/**
 * The identity a violation keeps across audits: `violations_json` is each audit's current state, so an
 * unfixed violation is written again by every later audit and must not count more than once.
 * `json_extract` renders `span` as its JSON text, which is the serialized form the key needs.
 */
const violationKey = (column: string): SQL => sql.raw(`
  coalesce(json_extract(${column}, '$.clauseId'), '') || char(31)
  || coalesce(json_extract(${column}, '$.path'), '') || char(31)
  || coalesce(json_extract(${column}, '$.span'), '') || char(31)
  || coalesce(json_extract(${column}, '$.code'), '')
`);

/**
 * Per-rule evidence from the audits in one time range, keyed by `ruleId` — which is the catalog
 * rule id byte-for-byte. Rules with no evidence at all are absent; the caller reconciles against
 * the catalog so that absence never has to be inferred from a missing row.
 */
function ruleActivity(database: RerailDatabase, since: number, repositoryId?: Repository): Map<string, RuleActivityTotals> {
  const rows = database.all<{
    ruleId: string;
    distinctViolations: number;
    sessions: number;
    lastSeenAt: number | null;
    evaluations: number;
  }>(sql`
    WITH runs AS (
      SELECT audit_id, session_id, started_at, violations_json, rule_revisions_json
      FROM ${policyRuns}
      WHERE started_at >= ${since} AND ${scopedSessions(policyRuns.sessionId, repositoryId)}
    ),
    violations AS (
      SELECT
        json_extract(finding.value, '$.ruleId') AS ruleId,
        count(DISTINCT ${violationKey("finding.value")}) AS distinctViolations,
        count(DISTINCT run.session_id) AS sessions,
        max(run.started_at) AS lastSeenAt
      FROM runs run, json_each(run.violations_json) finding
      GROUP BY ruleId
    ),
    evaluated AS (
      SELECT json_extract(revision.value, '$.ruleId') AS ruleId, count(DISTINCT run.audit_id) AS evaluations
      FROM runs run, json_each(run.rule_revisions_json) revision
      GROUP BY ruleId
    ),
    measured AS (
      SELECT ruleId FROM violations
      UNION
      SELECT ruleId FROM evaluated
    )
    SELECT
      measured.ruleId AS ruleId,
      coalesce(violations.distinctViolations, 0) AS distinctViolations,
      coalesce(violations.sessions, 0) AS sessions,
      violations.lastSeenAt AS lastSeenAt,
      coalesce(evaluated.evaluations, 0) AS evaluations
    FROM measured
    LEFT JOIN violations ON violations.ruleId = measured.ruleId
    LEFT JOIN evaluated ON evaluated.ruleId = measured.ruleId
    WHERE measured.ruleId IS NOT NULL
  `);
  return new Map(rows.map(row => [row.ruleId, {
    distinctViolations: row.distinctViolations,
    sessions: row.sessions,
    lastSeenAt: row.lastSeenAt,
    audits: row.evaluations,
  }]));
}

/**
 * Violations newest first, one row per audit that produced them. Repeats within a single audit collapse;
 * repeats across audits stay, because the page shows a rule's history rather than its current state.
 */
function listFindingHistory(database: RerailDatabase, query: FindingHistoryQuery, repositoryId?: Repository): FindingHistoryPage {
  const { limit, offset } = paging(query);
  const ruleId = query.ruleId?.trim() || null;
  const matching = sql`
    WITH runs AS (
      SELECT audit_id, session_id, started_at, violations_json
      FROM ${policyRuns}
      WHERE started_at >= ${query.since} AND ${scopedSessions(policyRuns.sessionId, repositoryId)}
    ),
    entries AS (
      SELECT
        finding.value AS payload,
        run.audit_id AS runId,
        run.session_id AS sessionId,
        run.started_at AS at,
        json_extract(finding.value, '$.ruleId') AS ruleId,
        json_extract(finding.value, '$.id') AS findingId,
        row_number() OVER (
          PARTITION BY run.audit_id, json_extract(finding.value, '$.ruleId'), ${violationKey("finding.value")}
          ORDER BY finding.key
        ) AS copy
      FROM runs run, json_each(run.violations_json) finding
    ),
    matching AS (
      SELECT payload, runId, sessionId, at, findingId
      FROM entries
      WHERE copy = 1 AND (${ruleId} IS NULL OR ruleId = ${ruleId})
    )
  `;
  const rows = database.all<{ payload: string; runId: string; sessionId: string; at: number; total: number }>(sql`
    ${matching}
    SELECT payload, runId, sessionId, at, count(*) OVER () AS total
    FROM matching
    ORDER BY at DESC, findingId
    LIMIT ${limit} OFFSET ${offset}
  `);
  // The window total rides on returned rows, so a page past the end has to count on its own.
  const total = rows[0]?.total
    ?? database.all<{ total: number }>(sql`${matching} SELECT count(*) AS total FROM matching`)[0]?.total
    ?? 0;
  return {
    violations: rows.map(row => ({
      // The payload is this daemon's own serialized violation, re-read from the audit that wrote it.
      violation: JSON.parse(row.payload) as ExecutablePolicyFinding,
      sessionId: row.sessionId,
      auditId: row.runId,
      at: row.at,
    })),
    total,
  };
}

/** Durable, local-only execution bookkeeping. It never emits transcript events. */
export class SqlitePolicyExecutionStore implements PolicyExecutionRecorder, PolicyExecutionRunReader {
  readonly supportsResultPages = true;
  readonly path: string;
  readonly #database: RerailDatabase;

  constructor(path: string, database?: RerailDatabase) {
    this.path = path;
    this.#database = database ?? openRerailDatabase(path);
  }

  start(input: PolicyRunInput): string {
    const runId = randomUUID();
    this.#database
      .insert(policyRuns)
      .values({
        auditId: runId,
        sessionId: input.sessionId,
        workspaceRoot: input.workspaceRoot,
        scope: input.scope,
        mode: input.mode,
        revision: input.revision,
        catalogRevision: input.catalogRevision,
        ruleRevisions: input.ruleRevisions,
        paths: input.paths,
        ruleProvenance: input.ruleProvenance ?? [],
        startedAt: Date.now(),
        finishedAt: null,
        status: "running",
        violations: [],
        failures: [],
      })
      .run();
    return runId;
  }
  appendResultPage(
    runId: string,
    pageIndex: number,
    result: Pick<PolicyRunResult, "violations" | "failures">,
  ): void {
    this.#database.$client
      .prepare(
        "INSERT INTO audit_result_pages(audit_id,page_index,violations_json,failures_json) " +
          "SELECT ?,?,?,? WHERE EXISTS (SELECT 1 FROM audits WHERE audit_id=? AND status='running') " +
          "ON CONFLICT(audit_id,page_index) DO UPDATE SET violations_json=excluded.violations_json,failures_json=excluded.failures_json",
      )
      .run(
        runId,
        pageIndex,
        JSON.stringify(result.violations ?? []),
        JSON.stringify(result.failures ?? []),
        runId,
      );
  }

  finishResultPages(runId: string, status: PolicyRunStatus): void {
    this.#database.$client
      .prepare(
        "UPDATE audits SET status=?, finished_at=?, " +
          "violations_json=COALESCE((SELECT json_group_array(json(j.value)) FROM audit_result_pages p, json_each(p.violations_json) j WHERE p.audit_id=audits.audit_id ORDER BY p.page_index,j.key),'[]'), " +
          "failures_json=COALESCE((SELECT json_group_array(json(j.value)) FROM audit_result_pages p, json_each(p.failures_json) j WHERE p.audit_id=audits.audit_id ORDER BY p.page_index,j.key),'[]') " +
          "WHERE audit_id=? AND status='running'",
      )
      .run(status, Date.now(), runId);
    this.#database.delete(policyRunResultPages).where(eq(policyRunResultPages.auditId, runId)).run();
  }

  progress(runId: string, result: Pick<PolicyRunResult, "violations" | "failures">): void {
    this.#database
      .update(policyRuns)
      .set({
        ...(result.violations === undefined ? {} : { violations: result.violations }),
        ...(result.failures === undefined ? {} : { failures: result.failures }),
      })
      .where(and(eq(policyRuns.auditId, runId), eq(policyRuns.status, "running")))
      .run();
  }

  finish(runId: string, result: PolicyRunResult): void {
    // Interruption and database reset may win a race with an evaluator's late
    // completion. Only the first terminal transition owns the persisted result.
    this.#database
      .update(policyRuns)
      .set({
        status: result.status,
        violations: result.violations ?? [],
        failures: result.failures ?? [],
        finishedAt: Date.now(),
      })
      .where(and(eq(policyRuns.auditId, runId), eq(policyRuns.status, "running")))
      .run();
  }

  interrupt(sessionId: string): void {
    this.#database
      .update(policyRuns)
      .set({ status: "interrupted", finishedAt: Date.now() })
      .where(and(eq(policyRuns.sessionId, sessionId), eq(policyRuns.status, "running")))
      .run();
  }

  get(runId: string): PolicyRunRecord | undefined {
    const row = this.#database.select().from(policyRuns).where(eq(policyRuns.auditId, runId)).get();
    return row === undefined ? undefined : record(row);
  }
  list(query: PolicyRunQuery = {}): PolicyRunPage {
    return listPolicyRuns(this.#database, query);
  }
  listRepositoryDiagnostics(query: RepositoryDiagnosticQuery = {}): RepositoryDiagnosticPage {
    return listRepositoryDiagnostics(this.#database, query);
  }
  listRepositoryDiagnosticEntries(query: RepositoryDiagnosticEntryQuery): RepositoryDiagnosticEntryPage | undefined {
    return listRepositoryDiagnosticEntries(this.#database, query);
  }
  ruleActivity(since: number): Map<string, RuleActivityTotals> {
    return ruleActivity(this.#database, since);
  }
  listFindingHistory(query: FindingHistoryQuery): FindingHistoryPage {
    return listFindingHistory(this.#database, query);
  }

  latestSummary(sessionId: string): PolicyRunSummary | undefined {
    return latestPolicyRunSummary(this.#database, sessionId);
  }

  close(): void {
    this.#database.$client.close();
  }
}

/**
 * Read-only policy-run view that remains empty until a writer has created the database. With a
 * `repositoryId`, listings and aggregates cover only runs of that repository's sessions; a run or
 * session addressed by its id is found whichever repository recorded it.
 */
export class SqlitePolicyExecutionReader implements PolicyExecutionRunReader {
  readonly path: string;
  readonly #database: LazyReadonlyRerailDatabase;
  readonly #repositoryId: Repository;

  constructor(path: string, { repositoryId }: { repositoryId?: string } = {}) {
    this.path = path;
    this.#repositoryId = repositoryId;
    this.#database = new LazyReadonlyRerailDatabase(path);
  }

  get(runId: string): PolicyRunRecord | undefined {
    const database = this.#database.open();
    if (database === undefined) return undefined;
    const row = database.select().from(policyRuns).where(eq(policyRuns.auditId, runId)).get();
    return row === undefined ? undefined : record(row);
  }
  list(query: PolicyRunQuery = {}): PolicyRunPage {
    const database = this.#database.open();
    return database === undefined ? { audits: [], total: 0 } : listPolicyRuns(database, query, this.#repositoryId);
  }
  listRepositoryDiagnostics(query: RepositoryDiagnosticQuery = {}): RepositoryDiagnosticPage {
    const database = this.#database.open();
    return database === undefined ? { repositories: [], total: 0 } : listRepositoryDiagnostics(database, query, this.#repositoryId);
  }
  listRepositoryDiagnosticEntries(query: RepositoryDiagnosticEntryQuery): RepositoryDiagnosticEntryPage | undefined {
    const database = this.#database.open();
    return database === undefined ? undefined : listRepositoryDiagnosticEntries(database, query);
  }
  ruleActivity(since: number): Map<string, RuleActivityTotals> {
    const database = this.#database.open();
    return database === undefined ? new Map() : ruleActivity(database, since, this.#repositoryId);
  }
  listFindingHistory(query: FindingHistoryQuery): FindingHistoryPage {
    const database = this.#database.open();
    return database === undefined ? { violations: [], total: 0 } : listFindingHistory(database, query, this.#repositoryId);
  }

  latestSummary(sessionId: string): PolicyRunSummary | undefined {
    const database = this.#database.open();
    return database === undefined ? undefined : latestPolicyRunSummary(database, sessionId);
  }

  close(): void {
    this.#database.close();
  }
}
