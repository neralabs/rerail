import { randomUUID } from "node:crypto";
import { and, count, desc, eq, getTableColumns, sql, type SQL } from "drizzle-orm";
import type { SQLiteColumn } from "drizzle-orm/sqlite-core";
import { z } from "zod";
import { LazyReadonlyRerailDatabase, openRerailDatabase, type RerailDatabase } from "../storage/database.js";
import { decodeCursor, encodeCursor } from "../storage/page-cursor.js";
import type { CodeRuleViolation } from "../code-rule/code-rule.js";
import {
  blindFilePaths,
  type AuditRecorder,
  type AuditInput,
  type AuditRecord,
  type AuditResult,
  type AuditSummary,
  type AuditMode,
  type AuditScope,
  type AuditStatus,
} from "./audit.js";
import type { RuleMode } from "../catalog/rule-mode.js";
import {
  repositoryAudits,
  auditResultPages,
  auditUnlistedFailures,
  audits,
  auditRuleSetRules,
  auditRuleSetUses,
  auditViolations,
  ruleViolationRollup,
  sessions,
  sessionsOfRepository,
  type AuditRow,
} from "../storage/schema.js";

/**
 * Databases known to hold `audit_unlisted_failures`. A read-only open applies no DDL, so a file only an earlier build
 * wrote lacks the table: its audits list every failure they kept, and none is counted beyond them.
 */
const HOLDS_UNLISTED_TABLE = new WeakSet<RerailDatabase>();
function holdsUnlistedTable(database: RerailDatabase): boolean {
  if (HOLDS_UNLISTED_TABLE.has(database)) return true;
  const holds = database.$client
    .prepare("SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'audit_unlisted_failures'")
    .get() !== undefined;
  if (holds) HOLDS_UNLISTED_TABLE.add(database);
  return holds;
}
/**
 * How many failures an audit had beyond those it lists (`audit_unlisted_failures`), in a select from `audits`. Written
 * with table names: Drizzle renders a single-table select's columns without them, which would leave the subquery
 * comparing `audit_unlisted_failures.audit_id` with itself.
 */
function unlistedFailures(database: RerailDatabase): SQL<number> {
  return holdsUnlistedTable(database)
    ? sql<number>`coalesce((SELECT unlisted.count FROM audit_unlisted_failures unlisted
      WHERE unlisted.audit_id = audits.audit_id), 0)`
    : sql<number>`0`;
}
/** An `audits` row with `unlistedFailures`, which `record` reads. */
function auditColumns(database: RerailDatabase) {
  return { ...getTableColumns(audits), unlistedFailureCount: unlistedFailures(database) };
}

export interface AuditQuery {
  auditId?: string;
  sessionId?: string;
  status?: AuditStatus;
  scope?: AuditScope;
  mode?: AuditMode;
  limit?: number;
  offset?: number;
}

export interface AuditPage {
  audits: AuditRecord[];
  total: number;
}

export interface RepositoryAuditSummary {
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
    scope: AuditScope;
    status: AuditStatus;
    startedAt: number;
    finishedAt: number | null;
    revision: number;
    catalogRevision: string | null;
    ruleRevisionCount: number;
    violationCount: number;
    /** Every failure the audit had, listed or not; `unlistedFailureCount` of them its list leaves out. */
    failureCount: number;
    unlistedFailureCount: number;
  } | null;
}

export interface RepositoryAuditQuery {
  workspaceRoot?: string;
  status?: "running" | "completed" | "failed" | "interrupted";
  limit?: number;
  offset?: number;
}

export interface RepositoryAuditEntryQuery {
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

export interface RepositoryAuditEntry {
  kind: "violation" | "failure";
  payload: unknown;
}

export interface RepositoryAuditPage {
  repositories: RepositoryAuditSummary[];
  total: number;
}

export interface RepositoryAuditEntryPage {
  entries: RepositoryAuditEntry[];
  total: number;
}

/** What one rule's audits in a time range say about it; absent evidence is zeros, never a guess. */
export interface RuleActivityTotals {
  distinctViolations: number;
  sessions: number;
  lastSeenAt: number | null;
  /** Distinct audits in range whose compiled rule set included this rule; 0 means never evaluated. */
  audits: number;
  /** Of `distinctViolations`, those recorded in Log mode: the mode at the time, not the rule's mode now. */
  loggedViolations: number;
}

export interface ViolationHistoryQuery {
  /** Epoch milliseconds; audits started at or after this instant. `0` means every audit. */
  since: number;
  ruleId?: string;
  sessionId?: string;
  limit?: number;
  /** The previous page's `nextCursor`; the first page without one. */
  cursor?: string;
}

/** One violation with the audit that produced it; `at` is the audit's start, `host` its session's coding agent. */
export interface ViolationHistoryEntry {
  violation: CodeRuleViolation;
  sessionId: string;
  auditId: string;
  at: number;
  host: string | null;
}

export interface ViolationHistoryPage {
  violations: ViolationHistoryEntry[];
  /** Every violation the query matches, on every page. */
  total: number;
  /** Where the next page starts; `null` on the last page. */
  nextCursor: string | null;
}

/**
 * `repositoryIds`, where a method takes it, narrows that call to the runs of sessions recorded under those ids (a
 * repository key and the clone ids of its checkouts); it replaces a reader's own scope.
 */
export interface AuditReader {
  get(auditId: string): AuditRecord | undefined;
  list(query?: AuditQuery, repositoryIds?: readonly string[]): AuditPage;
  latestSummary(sessionId: string): AuditSummary | undefined;
  listRepositoryAudits(query?: RepositoryAuditQuery, repositoryIds?: readonly string[]): RepositoryAuditPage;
  listRepositoryAuditEntries(query: RepositoryAuditEntryQuery): RepositoryAuditEntryPage | undefined;
  ruleActivity(since: number, repositoryIds?: readonly string[]): Map<string, RuleActivityTotals>;
  listViolationHistory(query: ViolationHistoryQuery, repositoryIds?: readonly string[]): ViolationHistoryPage;
  /** Distinct sessions with a violation from an audit started at or after `since`. */
  sessionsWithViolations(since: number, repositoryIds?: readonly string[]): number;
}

/** A repository's runs are the runs of its sessions; unscoped (`undefined`) readers see every run. */
type Repository = string | readonly string[] | undefined;

function scopedSessions(column: SQLiteColumn, repositoryId: Repository): SQL {
  return repositoryId === undefined ? sql`1 = 1` : sql`${column} IN (${sessionsOfRepository(repositoryId)})`;
}

function conditions(query: AuditQuery, repositoryId: Repository): SQL | undefined {
  return and(
    repositoryId === undefined ? undefined : scopedSessions(audits.sessionId, repositoryId),
    query.auditId === undefined ? undefined : eq(audits.auditId, query.auditId),
    query.sessionId === undefined ? undefined : eq(audits.sessionId, query.sessionId),
    query.status === undefined ? undefined : eq(audits.status, query.status),
    query.scope === undefined ? undefined : eq(audits.scope, query.scope),
    query.mode === undefined ? undefined : eq(audits.mode, query.mode),
  );
}

function paging(query: Pick<AuditQuery, "limit" | "offset">): { limit: number; offset: number } {
  return {
    limit: Math.min(200, Math.max(1, Math.trunc(query.limit ?? 50))),
    offset: Math.max(0, Math.trunc(query.offset ?? 0)),
  };
}

function record(row: AuditRow & { unlistedFailureCount: number }): AuditRecord {
  const blindFiles = blindFilePaths(row.failures);
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
    failureCount: row.failures.length + row.unlistedFailureCount,
    unlistedFailureCount: row.unlistedFailureCount,
    blindFiles,
    blindFileCount: blindFiles.length,
  };
}

function listAudits(database: RerailDatabase, query: AuditQuery = {}, repositoryId?: Repository): AuditPage {
  const where = conditions(query, repositoryId);
  const { limit, offset } = paging(query);
  const total = database.select({ total: count() }).from(audits).where(where).get()!.total;
  const page = database
    .select(auditColumns(database))
    .from(audits)
    .where(where)
    .orderBy(desc(audits.startedAt), desc(audits.auditId))
    .limit(limit)
    .offset(offset)
    .all()
    .map(record);
  return { audits: page, total };
}

/** `isOutOfWork` over one failure of `json_each`, in SQL. */
const OUT_OF_WORK = sql.raw(`(json_extract(value, '$.blind') IS NULL AND json_extract(value, '$.code') = 'budget-exceeded'
  AND json_extract(value, '$.ruleId') IS NOT NULL AND coalesce(json_extract(value, '$.stage'), '') NOT IN ('read', 'scan'))`);
/** `isCoverageGap` over one failure of `json_each`, in SQL. */
const COVERAGE_GAP = sql.raw(`(json_extract(value, '$.blind') = 'budget' OR (json_extract(value, '$.blind') IS NULL
  AND json_extract(value, '$.code') IN ('budget-exceeded', 'file-budget-exceeded') AND json_extract(value, '$.ruleId') IS NOT NULL
  AND coalesce(json_extract(value, '$.stage'), '') NOT IN ('read', 'scan')))`);
/** A rule the file's work stopped (`file-budget-exceeded`), over one failure of `json_each`, in SQL. */
const FILE_WORK_STOPPED = sql.raw(`(json_extract(value, '$.blind') IS NULL AND json_extract(value, '$.code') = 'file-budget-exceeded'
  AND json_extract(value, '$.ruleId') IS NOT NULL AND coalesce(json_extract(value, '$.stage'), '') NOT IN ('read', 'scan'))`);

function latestAuditSummary(database: RerailDatabase, sessionId: string): AuditSummary | undefined {
  return database.select({
    workspaceRoot: audits.workspaceRoot,
    scope: audits.scope,
    mode: audits.mode,
    status: audits.status,
    violationCount: sql<number>`json_array_length(${audits.violations})`,
    failureCount: sql<number>`json_array_length(${audits.failures}) + ${unlistedFailures(database)}`,
    unlistedFailureCount: unlistedFailures(database),
    blindFileCount: sql<number>`(SELECT count(*) FROM json_each(${audits.failures}) WHERE json_extract(value, '$.blind') IS NOT NULL)`,
    gapCount: sql<number>`(SELECT count(*) FROM json_each(${audits.failures}) WHERE ${COVERAGE_GAP})`,
    outOfWorkRuleCount: sql<number>`(SELECT count(DISTINCT json_extract(value, '$.ruleId')) FROM json_each(${audits.failures})
      WHERE ${OUT_OF_WORK})`,
    fileWorkStoppedRuleCount: sql<number>`(SELECT count(DISTINCT json_extract(value, '$.ruleId')) FROM json_each(${audits.failures})
      WHERE ${FILE_WORK_STOPPED})`,
  }).from(audits).where(eq(audits.sessionId, sessionId))
    .orderBy(desc(audits.startedAt), desc(audits.auditId)).limit(1).get();
}

function listRepositoryAudits(
  database: RerailDatabase,
  query: RepositoryAuditQuery = {},
  repositoryId?: Repository,
): RepositoryAuditPage {
  const { limit, offset } = paging(query);
  const workspaceRoot = query.workspaceRoot ?? null;
  const status = query.status ?? null;
  // The audit's failures past those it lists; a file only older builds wrote has no such table.
  const unlisted = holdsUnlistedTable(database)
    ? sql`(SELECT ${auditUnlistedFailures.count} FROM ${auditUnlistedFailures} WHERE ${auditUnlistedFailures.auditId} = audit.audit_id)`
    : sql`NULL`;
  const rows = database.all<{
    workspaceRoot: string;
    repositoryAuditSessionId: string | null;
    repositoryAuditId: string | null;
    repositoryAuditStatus: NonNullable<RepositoryAuditSummary["repositoryAudit"]>["status"] | null;
    repositoryAuditStartedAt: number | null;
    repositoryAuditFinishedAt: number | null;
    repositoryAuditError: string | null;
    auditId: string | null;
    auditScope: AuditScope | null;
    auditStatus: AuditStatus | null;
    auditStartedAt: number | null;
    auditFinishedAt: number | null;
    revision: number | null;
    catalogRevision: string | null;
    ruleRevisionCount: number | null;
    violationCount: number | null;
    failureCount: number | null;
    unlistedFailureCount: number | null;
    total: number;
  }>(sql`
    WITH repositories AS (
      SELECT workspace_root FROM ${audits} WHERE ${scopedSessions(audits.sessionId, repositoryId)}
      UNION
      SELECT workspace_root FROM ${repositoryAudits} WHERE ${scopedSessions(repositoryAudits.sessionId, repositoryId)}
    )
    SELECT
      repositories.workspace_root AS workspaceRoot,
      repositoryAudit.session_id AS repositoryAuditSessionId,
      repositoryAudit.audit_id AS repositoryAuditId,
      repositoryAudit.status AS repositoryAuditStatus,
      repositoryAudit.started_at AS repositoryAuditStartedAt,
      repositoryAudit.finished_at AS repositoryAuditFinishedAt,
      repositoryAudit.error AS repositoryAuditError,
      audit.scope AS auditScope,
      audit.audit_id AS auditId,
      audit.status AS auditStatus,
      audit.started_at AS auditStartedAt,
      audit.finished_at AS auditFinishedAt,
      audit.revision AS revision,
      audit.catalog_revision AS catalogRevision,
      json_array_length(audit.rule_revisions_json) AS ruleRevisionCount,
      json_array_length(audit.violations_json) AS violationCount,
      json_array_length(audit.failures_json) + coalesce(${unlisted}, 0) AS failureCount,
      coalesce(${unlisted}, 0) AS unlistedFailureCount,
      count(*) OVER () AS total
    FROM repositories
    LEFT JOIN ${repositoryAudits} repositoryAudit ON repositoryAudit.workspace_root = repositories.workspace_root
    LEFT JOIN ${audits} audit ON audit.audit_id = repositoryAudit.audit_id
    WHERE (${workspaceRoot} IS NULL OR repositories.workspace_root = ${workspaceRoot})
      AND (${status} IS NULL OR repositoryAudit.status = ${status})
    ORDER BY repositories.workspace_root
    LIMIT ${limit} OFFSET ${offset}
  `);
  return {
    repositories: rows.map(row => ({
      workspaceRoot: row.workspaceRoot,
      repositoryAudit: row.repositoryAuditStatus === null ? null : {
        sessionId: row.repositoryAuditSessionId!,
        auditId: row.repositoryAuditId,
        status: row.repositoryAuditStatus,
        startedAt: row.repositoryAuditStartedAt!,
        finishedAt: row.repositoryAuditFinishedAt,
        error: row.repositoryAuditError,
      },
      audit: row.auditId === null || row.auditStatus === null ? null : {
        scope: row.auditScope!,
        auditId: row.auditId,
        status: row.auditStatus,
        startedAt: row.auditStartedAt!,
        finishedAt: row.auditFinishedAt,
        revision: row.revision!,
        catalogRevision: row.catalogRevision,
        ruleRevisionCount: row.ruleRevisionCount ?? 0,
        violationCount: row.violationCount ?? 0,
        failureCount: row.failureCount ?? 0,
        unlistedFailureCount: row.unlistedFailureCount ?? 0,
      },
    })),
    total: rows[0]?.total ?? 0,
  };
}

function listRepositoryAuditEntries(
  database: RerailDatabase,
  query: RepositoryAuditEntryQuery,
): RepositoryAuditEntryPage | undefined {
  const repositoryAudit = database.select({ auditId: repositoryAudits.auditId }).from(repositoryAudits)
    .where(eq(repositoryAudits.auditId, query.auditId)).get();
  if (repositoryAudit === undefined) return undefined;
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
        violation.value AS payload,
        json_extract(violation.value, '$.mode') AS mode,
        json_extract(violation.value, '$.code') AS code,
        json_extract(violation.value, '$.path') AS path,
        json_extract(violation.value, '$.message') AS message,
        json_extract(violation.value, '$.ruleId') AS ruleId,
        json_extract(violation.value, '$.clauseId') AS clauseId
      FROM ${audits} audit, json_each(audit.violations_json) violation
      WHERE audit.audit_id = ${query.auditId}
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
      FROM ${audits} audit, json_each(audit.failures_json) failure
      WHERE audit.audit_id = ${query.auditId}
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
 * Per-rule evidence from the audits in one time range, keyed by `ruleId` — which is the catalog
 * rule id byte-for-byte. Rules with no evidence at all are absent; the caller reconciles against
 * the catalog so that absence never has to be inferred from a missing row. Read from the derived
 * tables (`audit-history.ts`): a range `[since, now)` holds a violation exactly when its rollup row
 * was last seen at or after `since`, and an audit's rules are those of its rule set.
 */
function ruleActivity(database: RerailDatabase, since: number, repositoryId?: Repository): Map<string, RuleActivityTotals> {
  const rows = database.all<{
    ruleId: string;
    distinctViolations: number;
    sessions: number;
    lastSeenAt: number | null;
    audits: number;
    loggedViolations: number;
  }>(sql`
    WITH violations AS (
      SELECT
        ${ruleViolationRollup.ruleId} AS ruleId,
        count(DISTINCT ${ruleViolationRollup.violationKey}) AS distinctViolations,
        count(DISTINCT CASE WHEN ${ruleViolationRollup.lastLoggedAt} >= ${since} THEN ${ruleViolationRollup.violationKey} END) AS loggedViolations,
        count(DISTINCT ${ruleViolationRollup.sessionId}) AS sessions,
        max(${ruleViolationRollup.lastSeenAt}) AS lastSeenAt
      FROM ${ruleViolationRollup}
      WHERE ${ruleViolationRollup.lastSeenAt} >= ${since} AND ${scopedSessions(ruleViolationRollup.sessionId, repositoryId)}
      GROUP BY ${ruleViolationRollup.ruleId}
    ),
    uses AS (
      SELECT ${auditRuleSetUses.ruleSetId} AS ruleSetId, count(*) AS audits
      FROM ${auditRuleSetUses}
      WHERE ${auditRuleSetUses.startedAt} >= ${since} AND ${scopedSessions(auditRuleSetUses.sessionId, repositoryId)}
      GROUP BY ${auditRuleSetUses.ruleSetId}
    ),
    audited AS (
      SELECT ${auditRuleSetRules.ruleId} AS ruleId, sum(uses.audits) AS audits
      FROM uses JOIN ${auditRuleSetRules} ON ${auditRuleSetRules.ruleSetId} = uses.ruleSetId
      GROUP BY ${auditRuleSetRules.ruleId}
    ),
    measured AS (
      SELECT ruleId FROM violations
      UNION
      SELECT ruleId FROM audited
    )
    SELECT
      measured.ruleId AS ruleId,
      coalesce(violations.distinctViolations, 0) AS distinctViolations,
      coalesce(violations.sessions, 0) AS sessions,
      violations.lastSeenAt AS lastSeenAt,
      coalesce(audited.audits, 0) AS audits,
      coalesce(violations.loggedViolations, 0) AS loggedViolations
    FROM measured
    LEFT JOIN violations ON violations.ruleId = measured.ruleId
    LEFT JOIN audited ON audited.ruleId = measured.ruleId
  `);
  return new Map(rows.map(row => [row.ruleId, {
    distinctViolations: row.distinctViolations,
    sessions: row.sessions,
    lastSeenAt: row.lastSeenAt,
    audits: row.audits,
    loggedViolations: row.loggedViolations,
  }]));
}

function sessionsWithViolations(database: RerailDatabase, since: number, repositoryId?: Repository): number {
  return database.all<{ sessions: number }>(sql`
    SELECT count(DISTINCT ${ruleViolationRollup.sessionId}) AS sessions FROM ${ruleViolationRollup}
    WHERE ${ruleViolationRollup.lastSeenAt} >= ${since} AND ${scopedSessions(ruleViolationRollup.sessionId, repositoryId)}
  `)[0]?.sessions ?? 0;
}

/** A history row's place in newest-first order: audit start, then audit id (both descending), then report order. */
const historyCursorSchema = z.tuple([z.number().int(), z.string(), z.number().int().nonnegative()]);

/**
 * Violations newest first, one row per audit that produced them. Repeats within a single audit collapse;
 * repeats across audits stay, because the page shows a rule's history rather than its current state. Pages
 * continue from `cursor`; only the returned rows read their payload from `audits.violations_json`.
 */
function listViolationHistory(database: RerailDatabase, query: ViolationHistoryQuery, repositoryId?: Repository): ViolationHistoryPage {
  const { limit } = paging(query);
  const ruleId = query.ruleId?.trim() || null;
  const sessionId = query.sessionId?.trim() || null;
  const matching = and(
    sql`${auditViolations.startedAt} >= ${query.since}`,
    scopedSessions(auditViolations.sessionId, repositoryId),
    ruleId === null ? undefined : eq(auditViolations.ruleId, ruleId),
    sessionId === null ? undefined : eq(auditViolations.sessionId, sessionId),
  );
  const after = query.cursor === undefined ? undefined : decodeCursor(query.cursor, historyCursorSchema);
  const continuing = after === undefined ? undefined : sql`(
    ${auditViolations.startedAt} < ${after[0]}
    OR (${auditViolations.startedAt} = ${after[0]} AND (${auditViolations.auditId} < ${after[1]}
      OR (${auditViolations.auditId} = ${after[1]} AND ${auditViolations.ordinal} > ${after[2]})))
  )`;
  // The page and its total from one snapshot, so a commit between them cannot make them disagree.
  const { rows, total } = database.transaction(() => ({
    rows: database.all<{ payload: string; auditId: string; ordinal: number; sessionId: string; at: number; host: string | null }>(sql`
    SELECT
      json_extract(${audits.violations}, '$[' || page.ordinal || ']') AS payload,
      page.audit_id AS auditId, page.ordinal AS ordinal, page.session_id AS sessionId, page.started_at AS at,
      ${sessions.host} AS host
    FROM (
      SELECT ${auditViolations.auditId}, ${auditViolations.ordinal}, ${auditViolations.sessionId}, ${auditViolations.startedAt}
      FROM ${auditViolations}
      WHERE ${and(matching, continuing)}
      ORDER BY ${auditViolations.startedAt} DESC, ${auditViolations.auditId} DESC, ${auditViolations.ordinal}
      LIMIT ${limit + 1}
    ) page
    JOIN ${audits} ON ${audits.auditId} = page.audit_id
    LEFT JOIN ${sessions} ON ${sessions.sessionId} = page.session_id
    ORDER BY page.started_at DESC, page.audit_id DESC, page.ordinal
  `),
    total: database.select({ total: count() }).from(auditViolations).where(matching).get()?.total ?? 0,
  }));
  const shown = rows.slice(0, limit);
  const last = shown.at(-1);
  return {
    violations: shown.map(row => ({
      // The payload is this daemon's own serialized violation, re-read from the audit that wrote it.
      violation: JSON.parse(row.payload) as CodeRuleViolation,
      sessionId: row.sessionId,
      auditId: row.auditId,
      at: row.at,
      host: row.host,
    })),
    total,
    nextCursor: rows.length > limit && last !== undefined ? encodeCursor([last.at, last.auditId, last.ordinal]) : null,
  };
}

/** Durable, local-only audit bookkeeping. It never emits session events. */
export class SqliteAuditStore implements AuditRecorder, AuditReader {
  readonly supportsResultPages = true;
  readonly path: string;
  readonly #database: RerailDatabase;

  constructor(path: string, database?: RerailDatabase) {
    this.path = path;
    this.#database = database ?? openRerailDatabase(path);
  }

  start(input: AuditInput): string {
    const auditId = randomUUID();
    this.#database
      .insert(audits)
      .values({
        auditId: auditId,
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
    return auditId;
  }
  appendResultPage(
    auditId: string,
    pageIndex: number,
    result: Pick<AuditResult, "violations" | "failures">,
  ): void {
    this.#database.$client
      .prepare(
        "INSERT INTO audit_result_pages(audit_id,page_index,violations_json,failures_json) " +
          "SELECT ?,?,?,? WHERE EXISTS (SELECT 1 FROM audits WHERE audit_id=? AND status='running') " +
          "ON CONFLICT(audit_id,page_index) DO UPDATE SET violations_json=excluded.violations_json,failures_json=excluded.failures_json",
      )
      .run(
        auditId,
        pageIndex,
        JSON.stringify(result.violations ?? []),
        JSON.stringify(result.failures ?? []),
        auditId,
      );
  }

  finishResultPages(auditId: string, status: AuditStatus, unlistedFailureCount = 0): void {
    this.#database.$client.transaction(() => {
      const finished = this.#database.$client
        .prepare(
          "UPDATE audits SET status=?, finished_at=?, " +
            "violations_json=COALESCE((SELECT json_group_array(json(j.value)) FROM audit_result_pages p, json_each(p.violations_json) j WHERE p.audit_id=audits.audit_id ORDER BY p.page_index,j.key),'[]'), " +
            "failures_json=COALESCE((SELECT json_group_array(json(j.value)) FROM audit_result_pages p, json_each(p.failures_json) j WHERE p.audit_id=audits.audit_id ORDER BY p.page_index,j.key),'[]') " +
            "WHERE audit_id=? AND status='running'",
        )
        .run(status, Date.now(), auditId).changes > 0;
      if (finished) this.#recordUnlisted(auditId, unlistedFailureCount);
      this.#database.delete(auditResultPages).where(eq(auditResultPages.auditId, auditId)).run();
    })();
  }

  progress(auditId: string, result: Pick<AuditResult, "violations" | "failures" | "unlistedFailureCount">): void {
    this.#database.$client.transaction(() => {
      const running = this.#database
        .update(audits)
        .set({
          ...(result.violations === undefined ? {} : { violations: result.violations }),
          ...(result.failures === undefined ? {} : { failures: result.failures }),
        })
        .where(and(eq(audits.auditId, auditId), eq(audits.status, "running")))
        .run().changes > 0;
      if (running && result.failures !== undefined) this.#recordUnlisted(auditId, result.unlistedFailureCount ?? 0);
    })();
  }

  finish(auditId: string, result: AuditResult): void {
    // Interruption and database reset may win a race with an evaluator's late
    // completion. Only the first terminal transition owns the persisted result.
    this.#database.$client.transaction(() => {
      const finished = this.#database
        .update(audits)
        .set({
          status: result.status,
          violations: result.violations ?? [],
          failures: result.failures ?? [],
          finishedAt: Date.now(),
        })
        .where(and(eq(audits.auditId, auditId), eq(audits.status, "running")))
        .run().changes > 0;
      if (finished) this.#recordUnlisted(auditId, result.unlistedFailureCount ?? 0);
    })();
  }

  /** Keeps how many failures an audit had beyond those it lists, replacing what it kept before; none when it lists all. */
  #recordUnlisted(auditId: string, count: number): void {
    this.#database.delete(auditUnlistedFailures).where(eq(auditUnlistedFailures.auditId, auditId)).run();
    if (count > 0) this.#database.insert(auditUnlistedFailures).values({ auditId, count }).run();
  }

  interrupt(sessionId: string): void {
    this.#database
      .update(audits)
      .set({ status: "interrupted", finishedAt: Date.now() })
      .where(and(eq(audits.sessionId, sessionId), eq(audits.status, "running")))
      .run();
  }

  get(auditId: string): AuditRecord | undefined {
    const row = this.#database.select(auditColumns(this.#database)).from(audits).where(eq(audits.auditId, auditId)).get();
    return row === undefined ? undefined : record(row);
  }
  list(query: AuditQuery = {}, repositoryIds?: readonly string[]): AuditPage {
    return listAudits(this.#database, query, repositoryIds);
  }
  listRepositoryAudits(query: RepositoryAuditQuery = {}, repositoryIds?: readonly string[]): RepositoryAuditPage {
    return listRepositoryAudits(this.#database, query, repositoryIds);
  }
  listRepositoryAuditEntries(query: RepositoryAuditEntryQuery): RepositoryAuditEntryPage | undefined {
    return listRepositoryAuditEntries(this.#database, query);
  }
  ruleActivity(since: number, repositoryIds?: readonly string[]): Map<string, RuleActivityTotals> {
    return ruleActivity(this.#database, since, repositoryIds);
  }
  listViolationHistory(query: ViolationHistoryQuery, repositoryIds?: readonly string[]): ViolationHistoryPage {
    return listViolationHistory(this.#database, query, repositoryIds);
  }
  sessionsWithViolations(since: number, repositoryIds?: readonly string[]): number {
    return sessionsWithViolations(this.#database, since, repositoryIds);
  }

  latestSummary(sessionId: string): AuditSummary | undefined {
    return latestAuditSummary(this.#database, sessionId);
  }

  close(): void {
    this.#database.$client.close();
  }
}

/**
 * Read-only audit view that remains empty until a writer has created the database. With a
 * `repositoryId` (or several), listings and aggregates cover only runs of those sessions; a run or
 * session addressed by its id is found whichever repository recorded it.
 */
export class SqliteAuditReader implements AuditReader {
  readonly path: string;
  readonly #database: LazyReadonlyRerailDatabase;
  readonly #repositoryId: Repository;

  constructor(path: string, { repositoryId }: { repositoryId?: string | readonly string[] } = {}) {
    this.path = path;
    this.#repositoryId = repositoryId;
    this.#database = new LazyReadonlyRerailDatabase(path);
  }

  get(auditId: string): AuditRecord | undefined {
    const database = this.#database.open();
    if (database === undefined) return undefined;
    const row = database.select(auditColumns(database)).from(audits).where(eq(audits.auditId, auditId)).get();
    return row === undefined ? undefined : record(row);
  }
  list(query: AuditQuery = {}, repositoryIds?: readonly string[]): AuditPage {
    const database = this.#database.open();
    return database === undefined ? { audits: [], total: 0 } : listAudits(database, query, repositoryIds ?? this.#repositoryId);
  }
  listRepositoryAudits(query: RepositoryAuditQuery = {}, repositoryIds?: readonly string[]): RepositoryAuditPage {
    const database = this.#database.open();
    return database === undefined
      ? { repositories: [], total: 0 }
      : listRepositoryAudits(database, query, repositoryIds ?? this.#repositoryId);
  }
  listRepositoryAuditEntries(query: RepositoryAuditEntryQuery): RepositoryAuditEntryPage | undefined {
    const database = this.#database.open();
    return database === undefined ? undefined : listRepositoryAuditEntries(database, query);
  }
  ruleActivity(since: number, repositoryIds?: readonly string[]): Map<string, RuleActivityTotals> {
    const database = this.#database.open();
    return database === undefined ? new Map() : ruleActivity(database, since, repositoryIds ?? this.#repositoryId);
  }
  listViolationHistory(query: ViolationHistoryQuery, repositoryIds?: readonly string[]): ViolationHistoryPage {
    const database = this.#database.open();
    return database === undefined
      ? { violations: [], total: 0, nextCursor: null }
      : listViolationHistory(database, query, repositoryIds ?? this.#repositoryId);
  }
  sessionsWithViolations(since: number, repositoryIds?: readonly string[]): number {
    const database = this.#database.open();
    return database === undefined ? 0 : sessionsWithViolations(database, since, repositoryIds ?? this.#repositoryId);
  }

  latestSummary(sessionId: string): AuditSummary | undefined {
    const database = this.#database.open();
    return database === undefined ? undefined : latestAuditSummary(database, sessionId);
  }

  close(): void {
    this.#database.close();
  }
}
