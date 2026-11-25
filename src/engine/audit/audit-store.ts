import { createHash, randomUUID } from "node:crypto";
import { and, count, desc, eq, getTableColumns, getTableName, inArray, or, sql, type SQL } from "drizzle-orm";
import type { SQLiteColumn } from "drizzle-orm/sqlite-core";
import { z } from "zod";
import { LazyReadonlyRerailDatabase, openRerailDatabase, type RerailDatabase } from "../storage/database.js";
import { decodeCursor, encodeCursor } from "../storage/page-cursor.js";
import { flushRecordWrites, openRecordWriter, type RecordWriter } from "../storage/record-writes.js";
import { holdsTable } from "../storage/table-probe.js";
import { isStoredItem, ITEMS, nameOnce, storedList, type StoredItem, type StoredListLog } from "../storage/stored-list.js";
import type { CodeRuleViolation } from "../code-rule/code-rule.js";
import { scrubSecrets, scrubValue } from "../secret-scrub.js";
import {
  blindFilePaths,
  type AuditFailure,
  type AuditRecorder,
  type AuditInput,
  type AuditRecord,
  type AuditResult,
  type AuditSummary,
  type AuditMode,
  type AuditOrigin,
  type AuditScope,
  type AuditStatus,
  type AuditRuleProvenance,
} from "./audit.js";
import type { RuleMode } from "../catalog/rule-mode.js";
import {
  repositoryAudits,
  auditResultPages,
  auditUnlistedFailures,
  audits,
  auditRuleProvenances,
  auditRuleSetRules,
  auditRuleSetUses,
  auditViolations,
  checkouts,
  ruleViolationRollup,
  sessions,
  sessionsOfRepository,
  type AuditRow,
} from "../storage/schema.js";

/**
 * Violations as the database keeps them: each one's message, captured variables and fix text scrubbed of secrets
 * (`scrubSecrets`). What identifies a violation across audits (`violationKeySql`: clause, path, span, code) is kept
 * as it is, so history counts it as before. Steers render the live violations, never these.
 */
export function storedViolations(violations: readonly CodeRuleViolation[]): CodeRuleViolation[] {
  return violations.map(violation => {
    const message = scrubSecrets(violation.message);
    const variables = scrubValue(violation.variables);
    const fixText = violation.fix === undefined ? undefined : scrubSecrets(violation.fix.text);
    if (message === violation.message && variables === violation.variables && fixText === violation.fix?.text) return violation;
    return {
      ...violation, message, variables,
      ...(violation.fix === undefined || fixText === undefined ? {} : { fix: { ...violation.fix, text: fixText } }),
    };
  });
}

/** Failures as the database keeps them: each message scrubbed of secrets (`scrubSecrets`). */
export function storedFailures(failures: readonly AuditFailure[]): AuditFailure[] {
  return failures.map(failure => {
    const message = scrubSecrets(failure.message);
    return message === failure.message ? failure : { ...failure, message };
  });
}

/**
 * Databases known to hold `audit_unlisted_failures`. A read-only open applies no DDL, so a file only an earlier build
 * wrote lacks the table: its audits list every failure they kept, and none is counted beyond them.
 */
const HOLDS_UNLISTED_TABLE = new WeakSet<RerailDatabase>();
function holdsUnlistedTable(database: RerailDatabase): boolean {
  if (HOLDS_UNLISTED_TABLE.has(database)) return true;
  const holds = holdsTable(database.$client, "audit_unlisted_failures");
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
/**
 * An audit's rule provenance in a select from `audits`, as the text the database holds: the copy
 * `audit_rule_provenances` keeps, or the row's own for an audit an older build wrote. Written with table names, as
 * `unlistedFailures` is.
 */
function ruleProvenanceText(): SQL<string> {
  return sql<string>`coalesce((SELECT provenance.rule_provenance_json FROM audit_rule_provenances provenance
    WHERE provenance.provenance_id = audits.rule_provenance_id), audits.rule_provenance_json)`;
}

/**
 * The JSON lists an audit record reads from the database, in the order the failures that mark them come before the
 * audit's own: each one's column in `audits`, the clause id of the failure that marks it, its name in that failure and
 * in the log, whether that name is plural, and the JSON type of its items as SQLite names it (`json_each`'s `type`).
 */
const STORED_LISTS = {
  ruleRevisions: { column: audits.ruleRevisions, clauseId: "rule-revisions", name: "rule revisions", plural: true, item: "object" },
  ruleProvenance: { column: audits.ruleProvenance, clauseId: "rule-provenance", name: "rule provenance", plural: false, item: "object" },
  paths: { column: audits.paths, clauseId: "paths", name: "paths", plural: true, item: "text" },
  violations: { column: audits.violations, clauseId: "violations", name: "violations", plural: true, item: "object" },
  failures: { column: audits.failures, clauseId: "failures", name: "failures", plural: true, item: "object" },
} as const;
const LIST_NAMES = Object.keys(STORED_LISTS) as StoredList[];
type StoredList = keyof typeof STORED_LISTS;

/**
 * An `audits` row with each stored list as the text the database holds, which `record` parses (`readLists`), so one
 * audit's damaged text fails no page that holds it; and `unlistedFailures`.
 */
function auditColumns(database: RerailDatabase) {
  return {
    ...getTableColumns(audits),
    ruleRevisions: sql<string>`${audits.ruleRevisions}`, paths: sql<string>`${audits.paths}`,
    violations: sql<string>`${audits.violations}`, failures: sql<string>`${audits.failures}`,
    ruleProvenance: ruleProvenanceText(), unlistedFailureCount: unlistedFailures(database),
  };
}
/** An `audits` row as `auditColumns` selects it. */
type AuditReadRow = Omit<AuditRow, StoredList> & Record<StoredList, string> & { unlistedFailureCount: number };
/** The lists `record` reads from an `AuditReadRow`, each as its `AuditRecord` field holds it. */
type AuditLists = Pick<AuditRecord, StoredList>;

/** The code and stage of every failure `unreadable` makes; the enforcer's `spool` failures share them. */
const UNREADABLE = { code: "adapter-error", stage: "persistence" } as const;
/** The clause ids of the failures `unreadable` makes, one a list. */
const UNREADABLE_CLAUSES: ReadonlySet<string> = new Set(LIST_NAMES.map(list => STORED_LISTS[list].clauseId));
/**
 * The failure an audit lists before its own when the text the database holds for `list` is not a JSON array of its
 * items. A CHECK refuses text that is no JSON array for the rule provenance (in `audits` and in
 * `audit_rule_provenances`), the paths, the violations and the failures; none holds the rule revisions, which Rerail
 * only ever writes as an array, and none checks the items. Rerail writes only lists of its own items, so only a
 * damaged file or a hand edit leaves it.
 */
function unreadable(list: StoredList): AuditFailure {
  const { clauseId, name, plural, item } = STORED_LISTS[list];
  return {
    clauseId, ...UNREADABLE,
    message: `The audit's ${name} in the database ${plural ? "are" : "is"} not a JSON array of ${ITEMS[item]}, which only a damaged file or a hand edit leaves, so ${plural ? "they read" : "it reads"} as none.`,
  };
}

/**
 * Whether `failure` is one an audit record lists for a stored list it reads as none (`unreadable`): told by its code,
 * stage and clause id, never its message, which stays free to change.
 */
export function isUnreadableList(failure: AuditFailure): boolean {
  return failure.code === UNREADABLE.code && failure.stage === UNREADABLE.stage && UNREADABLE_CLAUSES.has(failure.clauseId);
}

/** The JSON array of `list`'s items `text` holds, as an audit record reads it, or why it holds none. */
export function parsedList(text: string, list: StoredList): unknown[] | string {
  return storedList(text, STORED_LISTS[list].item);
}

/** Where an audit record is read from: its database, and where the line that names a damaged list goes. */
interface AuditSource {
  readonly database: RerailDatabase;
  readonly log: StoredListLog;
}

/**
 * The line that names `row`'s audit and where its damaged `list` text is, once for each such text, on the error output
 * of the reader that read it: `daemon.log` for the API and a coding agent's `audits_list`, the command's own error
 * output for `rerail support-bundle`. Every audit naming a damaged stored copy of a rule provenance reads it, so the
 * line names the copy's row.
 */
function sayUnreadable({ database, log }: AuditSource, row: AuditReadRow, list: StoredList, why: string): void {
  const copy = list === "ruleProvenance" ? row.ruleProvenanceId : null;
  const { column, name, item } = STORED_LISTS[list];
  // The column and table names the schema gives, so the line names the place a person mends.
  const [where, place] = copy === null
    ? [`audit ${row.auditId} ${column.name}`, `the ${column.name} of its own row`]
    : [`copy ${copy}`, `${auditRuleProvenances.ruleProvenance.name} of ${getTableName(auditRuleProvenances)} row ${copy}, which every audit naming it reads,`];
  nameOnce(log, { file: database.$client.name, place: where }, `rerail: ${database.$client.name}: audit ${row.auditId} reads with no ${name}, and a failure `
    + `that says so: ${place} is not a JSON array of ${ITEMS[item]} (${why}).\n`);
}

/**
 * `row`'s stored lists, its failures after one for each list whose text is no JSON array of its items: such a list
 * reads as none, rather than failing every read of a page that holds the audit, and its failure and `sayUnreadable` say
 * so. Those failures come first, so the support bundle, which keeps an audit's first 100, keeps them; every other
 * reader of the record keeps every failure, and the session's summary only counts them.
 */
function readLists(source: AuditSource, row: AuditReadRow): AuditLists {
  const marks: AuditFailure[] = [];
  const read = (list: StoredList): unknown[] => {
    const parsed = parsedList(row[list], list);
    if (Array.isArray(parsed)) return parsed;
    sayUnreadable(source, row, list, parsed);
    marks.push(unreadable(list));
    return [];
  };
  // Read in `STORED_LISTS` order. Each JSON array is this daemon's own serialized list of what its field types.
  const ruleRevisions = read("ruleRevisions") as AuditLists["ruleRevisions"];
  const ruleProvenance = read("ruleProvenance") as AuditLists["ruleProvenance"];
  const paths = read("paths") as AuditLists["paths"];
  const violations = read("violations") as AuditLists["violations"];
  const failures = read("failures") as AuditLists["failures"];
  return { ruleRevisions, ruleProvenance, paths, violations, failures: [...marks, ...failures] };
}

/**
 * The id of `provenance` in `audit_rule_provenances`, stored first when no audit recorded it yet, so the audits of one
 * checkout's rules share one copy. Its key is the SHA-256 of its JSON text, as the daemon's move of older audits'
 * inline provenance keys it (`rule-provenance-move.ts`), so a later audit of the same rules names the copy the move
 * stored; a test of an upgraded file holds the two to that. For the caller's write transaction: no other writer can
 * store the same text between the lookup and the insert.
 */
function storedRuleProvenanceId(database: RerailDatabase, provenance: readonly AuditRuleProvenance[]): number {
  const json = JSON.stringify(provenance);
  const digest = createHash("sha256").update(json).digest("hex");
  const stored = database.select({ id: auditRuleProvenances.provenanceId }).from(auditRuleProvenances)
    .where(eq(auditRuleProvenances.digest, digest)).get();
  if (stored !== undefined) return stored.id;
  // The text the digest was taken of, rather than the column's JSON mode serializing the provenance again.
  const inserted = database.$client.prepare<[string, string]>("INSERT INTO audit_rule_provenances (digest, rule_provenance_json) VALUES (?, ?)")
    .run(digest, json);
  return Number(inserted.lastInsertRowid);
}

export interface AuditQuery {
  auditId?: string;
  sessionId?: string;
  status?: AuditStatus;
  scope?: AuditScope;
  mode?: AuditMode;
  origin?: AuditOrigin;
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

/**
 * One violation with the audit that produced it; `at` is the audit's start, `host` its session's coding agent. A
 * violation the repository's own audit recorded (`AuditOwner`) has neither.
 */
export interface ViolationHistoryEntry {
  violation: CodeRuleViolation;
  sessionId: string | null;
  auditId: string;
  at: number;
  host: string | null;
  /** The coding agent was not told: the audit ran in Log mode, or the violation's rule was in Log mode then. */
  logged: boolean;
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
  /** The folder each audit of `auditIds` ran in, which its violations' paths are relative to; an unknown id has none. */
  workspaceRoots(auditIds: readonly string[]): Map<string, string>;
  /** Distinct sessions with a violation from an audit started at or after `since`. */
  sessionsWithViolations(since: number, repositoryIds?: readonly string[]): number;
}

/**
 * A repository's runs are the runs of its sessions and its own (an audit no session owns, `AuditOwner`); unscoped
 * (`undefined`) readers see every run.
 */
type Repository = string | readonly string[] | undefined;

function scopedSessions(column: SQLiteColumn, repositoryId: Repository): SQL {
  return repositoryId === undefined ? sql`1 = 1` : sql`${column} IN (${sessionsOfRepository(repositoryId)})`;
}

/** `scopedSessions` for a table whose rows a session or the repository owns (`audits` and the history derived from it). */
function scopedOwner(session: SQLiteColumn, repository: SQLiteColumn, repositoryId: Repository): SQL {
  if (repositoryId === undefined) return sql`1 = 1`;
  const ids = typeof repositoryId === "string" ? [repositoryId] : [...repositoryId];
  const owned = ids.length === 0 ? sql`0` : inArray(repository, ids);
  return sql`(${scopedSessions(session, repositoryId)} OR (${session} IS NULL AND ${owned}))`;
}

/** The column each `AuditQuery` filter matches exactly. */
const EXACT_FILTER_COLUMNS = {
  auditId: audits.auditId, sessionId: audits.sessionId, status: audits.status,
  scope: audits.scope, mode: audits.mode, origin: audits.origin,
} as const;
const EXACT_FILTERS = Object.keys(EXACT_FILTER_COLUMNS) as Array<keyof typeof EXACT_FILTER_COLUMNS>;

function conditions(query: AuditQuery, repositoryId: Repository): SQL | undefined {
  return and(
    repositoryId === undefined ? undefined : scopedOwner(audits.sessionId, audits.repositoryId, repositoryId),
    ...EXACT_FILTERS.map(filter => (query[filter] === undefined ? undefined : eq(EXACT_FILTER_COLUMNS[filter], query[filter]))),
  );
}

function paging(query: Pick<AuditQuery, "limit" | "offset">): { limit: number; offset: number } {
  return {
    limit: Math.min(200, Math.max(1, Math.trunc(query.limit ?? 50))),
    offset: Math.max(0, Math.trunc(query.offset ?? 0)),
  };
}

function record(source: AuditSource, row: AuditReadRow): AuditRecord {
  const { ruleRevisions, ruleProvenance, paths, violations, failures } = readLists(source, row);
  const blindFiles = blindFilePaths(failures);
  return {
    auditId: row.auditId,
    sessionId: row.sessionId,
    repositoryId: row.repositoryId,
    recordedBy: row.recordedBy,
    workspaceRoot: row.workspaceRoot,
    scope: row.scope,
    mode: row.mode,
    origin: row.origin,
    revision: row.revision,
    catalogRevision: row.catalogRevision,
    ruleRevisions,
    ruleProvenance,
    paths,
    startedAt: row.startedAt,
    finishedAt: row.finishedAt,
    status: row.status,
    violations,
    failures,
    failureCount: failures.length + row.unlistedFailureCount,
    unlistedFailureCount: row.unlistedFailureCount,
    blindFiles,
    blindFileCount: blindFiles.length,
  };
}

/**
 * The owner columns of `input`'s audit: its session's, or for one the repository owns (`AuditOwner`) no session, the
 * repository its checkout is recorded under, and the session that runs it. Rerail records a worktree's checkout the
 * first time a session looks at it, so the walk that found the change has recorded it.
 */
function auditOwner(database: RerailDatabase, input: AuditInput): Pick<typeof audits.$inferInsert, "sessionId" | "repositoryId" | "recordedBy"> {
  if (input.owner !== "repository") return { sessionId: input.sessionId };
  const repository = database.select({ key: checkouts.repositoryKey }).from(checkouts).where(eq(checkouts.checkoutPath, input.workspaceRoot)).get();
  if (repository === undefined) throw new Error(`No repository is recorded for ${input.workspaceRoot}, so it cannot own an audit there`);
  return { sessionId: null, repositoryId: repository.key, recordedBy: input.sessionId };
}

function listAudits(source: AuditSource, query: AuditQuery = {}, repositoryId?: Repository): AuditPage {
  const { database } = source;
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
    .map(row => record(source, row));
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

/**
 * How many items `text` holds when it is a JSON array whose items are each of JSON type `item`, as `parsedList` reads
 * it, else NULL, in SQL. Text that does not parse is never walked, so it fails no query. One walk of the rest tests and
 * counts its items: an array's items have integer keys, an object's members text keys, and a lone value none. Only text
 * with nothing to walk is read a third time, to tell `[]` from `{}`, since each read of a 2 MB list shows in the cost
 * of the session's summary.
 */
function listLength(text: SQL | SQLiteColumn, item: StoredItem): SQL<number | null> {
  return sql<number | null>`(CASE WHEN NOT json_valid(${text}) THEN NULL ELSE (SELECT CASE
    WHEN count(*) = 0 THEN (CASE WHEN json_type(${text}) = 'array' THEN 0 END)
    WHEN max(entry.type <> ${item} OR typeof(entry.key) <> 'integer') THEN NULL ELSE count(*) END FROM json_each(${text}) entry) END)`;
}
/** 1 when `text` holds a JSON array whose items are each of JSON type `item`, as `parsedList` reads it, else 0, in SQL. */
function holdsList(text: SQL | SQLiteColumn, item: StoredItem): SQL<number> {
  return sql<number>`(${listLength(text, item)} IS NOT NULL)`;
}
/** The text an audit record reads `list` from, in a select from `audits`: the stored copy's for the rule provenance. */
function listText(list: StoredList): SQL | SQLiteColumn {
  return list === "ruleProvenance" ? ruleProvenanceText() : STORED_LISTS[list].column;
}
/**
 * How many of an audit's stored lists its record reads as none (`readLists`), the failures it lists for them, from
 * each list's test in SQL: 1 when it reads.
 */
function unreadableLists(held: Readonly<Record<StoredList, SQL>>): SQL<number> {
  return sql<number>`(${sql.join(LIST_NAMES.map(list => sql`(1 - ${held[list]})`), sql` + `)})`;
}

/**
 * Each stored list's length in a select from `audits`, as `listLength` tests it, under the list's name: NULL for a list
 * the record reads as none. A query tests each list once here and reads the result through `heldIn`.
 */
function listLengths(): SQL {
  return sql.join(LIST_NAMES.map(list => sql`${listLength(listText(list), STORED_LISTS[list].item)} AS ${sql.identifier(list)}`), sql`, `);
}
/** Each list's test in SQL (1 when it reads), from the `listLengths` of the row `alias` names. */
function heldIn(alias: string): Record<StoredList, SQL> {
  return Object.fromEntries(LIST_NAMES.map(list => [list, sql`(${sql.identifier(alias)}.${sql.identifier(list)} IS NOT NULL)`])) as Record<StoredList, SQL>;
}

/**
 * The session's latest audit's counts, each the audit record's: violations or failures that are no JSON array of
 * objects count as none, and each stored list that reads as none counts the failure its record lists, rather than
 * failing the session's page. It is read at every presentation request, so each list is tested once (`listLength`), in
 * a row SQLite makes once (`MATERIALIZED`) and the counts read.
 */
function latestAuditSummary(database: RerailDatabase, sessionId: string): AuditSummary | undefined {
  const held = heldIn("latest");
  const failures = sql`(CASE WHEN failures IS NULL THEN '[]' ELSE failures_json END)`;
  return database.get<AuditSummary | undefined>(sql`WITH latest AS MATERIALIZED (
      SELECT ${audits.workspaceRoot} AS workspaceRoot, ${audits.scope} AS scope, ${audits.mode} AS mode, ${audits.status} AS status,
        ${audits.failures} AS failures_json, ${unlistedFailures(database)} AS unlisted, ${listLengths()}
      FROM ${audits} WHERE ${audits.sessionId} = ${sessionId} ORDER BY ${audits.startedAt} DESC, ${audits.auditId} DESC LIMIT 1)
    SELECT workspaceRoot, scope, mode, status, coalesce(violations, 0) AS violationCount,
      coalesce(failures, 0) + ${unreadableLists(held)} + unlisted AS failureCount, unlisted AS unlistedFailureCount,
      (SELECT count(*) FROM json_each(${failures}) WHERE json_extract(value, '$.blind') IS NOT NULL) AS blindFileCount,
      (SELECT count(*) FROM json_each(${failures}) WHERE ${COVERAGE_GAP}) AS gapCount,
      (SELECT count(DISTINCT json_extract(value, '$.ruleId')) FROM json_each(${failures}) WHERE ${OUT_OF_WORK}) AS outOfWorkRuleCount,
      (SELECT count(DISTINCT json_extract(value, '$.ruleId')) FROM json_each(${failures}) WHERE ${FILE_WORK_STOPPED})
        AS fileWorkStoppedRuleCount
    FROM latest`);
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
      SELECT workspace_root FROM ${audits} WHERE ${scopedOwner(audits.sessionId, audits.repositoryId, repositoryId)}
      UNION
      SELECT workspace_root FROM ${repositoryAudits} WHERE ${scopedSessions(repositoryAudits.sessionId, repositoryId)}
    ),
    lengths AS MATERIALIZED (
      SELECT ${audits.auditId} AS audit_id, ${listLengths()} FROM ${audits}
      WHERE ${audits.auditId} IN (SELECT ${repositoryAudits.auditId} FROM ${repositoryAudits})
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
      coalesce(lengths.ruleRevisions, 0) AS ruleRevisionCount,
      coalesce(lengths.violations, 0) AS violationCount,
      coalesce(lengths.failures, 0) + ${unreadableLists(heldIn("lengths"))} + coalesce(${unlisted}, 0) AS failureCount,
      coalesce(${unlisted}, 0) AS unlistedFailureCount,
      count(*) OVER () AS total
    FROM repositories
    LEFT JOIN ${repositoryAudits} repositoryAudit ON repositoryAudit.workspace_root = repositories.workspace_root
    LEFT JOIN ${audits} audit ON audit.audit_id = repositoryAudit.audit_id
    LEFT JOIN lengths ON lengths.audit_id = audit.audit_id
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

/**
 * The failure entries a repository audit's entries page lists for the audit's stored lists its record reads as none,
 * the failures `unreadable` makes, in the columns of that page's `entries`: one row for each list `lists.<name>` holds
 * NULL for (`listLengths`).
 */
function unreadableEntries(): SQL {
  return sql.join(LIST_NAMES.map(list => {
    const failure = unreadable(list);
    return sql`SELECT 'failure' AS kind, ${JSON.stringify(failure)} AS payload, NULL AS mode, ${failure.code} AS code, NULL AS path,
      ${failure.message} AS message, NULL AS ruleId, ${failure.clauseId} AS clauseId FROM lists WHERE lists.${sql.identifier(list)} IS NULL`;
  }), sql` UNION ALL `);
}

/**
 * A repository audit's violations and failures, a page at a time. Each stored list is tested once (`listLengths`): one
 * its record reads as none lists no entry, where `json_each` would raise on it for the whole page, and the page lists the
 * failure the record lists for it instead (`unreadableEntries`).
 */
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
    WITH lists AS MATERIALIZED (SELECT ${listLengths()} FROM ${audits} WHERE ${audits.auditId} = ${query.auditId}),
    entries AS (
      SELECT
        'violation' AS kind,
        violation.value AS payload,
        json_extract(violation.value, '$.mode') AS mode,
        json_extract(violation.value, '$.code') AS code,
        json_extract(violation.value, '$.path') AS path,
        json_extract(violation.value, '$.message') AS message,
        json_extract(violation.value, '$.ruleId') AS ruleId,
        json_extract(violation.value, '$.clauseId') AS clauseId
      FROM ${audits} audit, lists, json_each(CASE WHEN lists.violations IS NULL THEN '[]' ELSE audit.violations_json END) violation
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
      FROM ${audits} audit, lists, json_each(CASE WHEN lists.failures IS NULL THEN '[]' ELSE audit.failures_json END) failure
      WHERE audit.audit_id = ${query.auditId}
      UNION ALL
      ${unreadableEntries()}
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
      WHERE ${ruleViolationRollup.lastSeenAt} >= ${since} AND ${scopedOwner(ruleViolationRollup.sessionId, ruleViolationRollup.repositoryId, repositoryId)}
      GROUP BY ${ruleViolationRollup.ruleId}
    ),
    uses AS (
      SELECT ${auditRuleSetUses.ruleSetId} AS ruleSetId, count(*) AS audits
      FROM ${auditRuleSetUses}
      WHERE ${auditRuleSetUses.startedAt} >= ${since} AND ${scopedOwner(auditRuleSetUses.sessionId, auditRuleSetUses.repositoryId, repositoryId)}
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

/**
 * Distinct sessions with a violation from an audit started at or after `since`. The rollup holds only violations
 * that name a rule, so the few stored without one are read from `audit_violations` (index on rule and start). The
 * repository's own violations belong to no session, so they count for none.
 */
function sessionsWithViolations(database: RerailDatabase, since: number, repositoryId?: Repository): number {
  return database.all<{ sessions: number }>(sql`
    SELECT count(*) AS sessions FROM (
      SELECT ${ruleViolationRollup.sessionId} AS session_id FROM ${ruleViolationRollup}
      WHERE ${ruleViolationRollup.lastSeenAt} >= ${since} AND ${ruleViolationRollup.sessionId} IS NOT NULL
        AND ${scopedSessions(ruleViolationRollup.sessionId, repositoryId)}
      UNION
      SELECT ${auditViolations.sessionId} FROM ${auditViolations}
      WHERE ${auditViolations.ruleId} IS NULL AND ${auditViolations.startedAt} >= ${since} AND ${auditViolations.sessionId} IS NOT NULL
        AND ${scopedSessions(auditViolations.sessionId, repositoryId)}
    )
  `)[0]?.sessions ?? 0;
}

/** A history row's place in newest-first order: audit start, then audit id (both descending), then report order. */
const historyCursorSchema = z.tuple([z.number().int(), z.string(), z.number().int().nonnegative()]);

/** A row of the history's page: its stored violation as JSON text, NULL where its audit's text does not parse. */
interface HistoryRow {
  payload: string | null;
  auditId: string;
  ordinal: number;
  sessionId: string | null;
  at: number;
  host: string | null;
  logged: number;
}

/**
 * The violation a history row reads: the JSON text of its audit's stored item at its place, when that is a JSON
 * object. None for a row whose audit's text does not parse (its payload NULL) or holds another JSON value there, or
 * no item at all.
 */
function rowViolation(row: HistoryRow): CodeRuleViolation | undefined {
  if (row.payload === null) return undefined;
  const value: unknown = JSON.parse(row.payload); // `->` gives JSON text, never anything else
  // The payload is this daemon's own serialized violation, re-read from the audit that wrote it.
  return isStoredItem(value, "object") ? value as CodeRuleViolation : undefined;
}

/**
 * The entries of the history's `rows` whose stored violation reads (`rowViolation`). A row whose audit's text holds
 * none there is left out, and the line that names the audit goes to the log once, so one damaged audit costs its own
 * violations, never the page.
 */
function readableHistory(source: AuditSource, rows: readonly HistoryRow[]): ViolationHistoryPage["violations"] {
  const entries: ViolationHistoryPage["violations"] = [];
  for (const row of rows) {
    const violation = rowViolation(row);
    if (violation === undefined) {
      sayHistoryLeftOut(source, row.auditId);
      continue;
    }
    entries.push({
      violation,
      sessionId: row.sessionId,
      auditId: row.auditId,
      at: row.at,
      host: row.host,
      logged: row.logged === 1,
    });
  }
  return entries;
}

/** The line that names an audit whose violations the history leaves out (`readableHistory`), once for each audit. */
function sayHistoryLeftOut({ database, log }: AuditSource, auditId: string): void {
  const file = database.$client.name;
  // The column name the schema gives, so the line names the place a person mends.
  const column = audits.violations.name;
  nameOnce(log, { file, place: `history audit ${auditId} ${column}` }, `rerail: ${file}: the violation history leaves out `
    + `violations of audit ${auditId}: the ${column} of its row holds no JSON object for them, which only a damaged file or a hand edit leaves.\n`);
}

/** The history rows `query` asks for, as a condition on `audit_violations`: its range, owner, rule and session. */
function historyMatching(query: ViolationHistoryQuery, repositoryId?: Repository): SQL | undefined {
  const ruleId = query.ruleId?.trim() || null;
  const sessionId = query.sessionId?.trim() || null;
  return and(
    sql`${auditViolations.startedAt} >= ${query.since}`,
    scopedOwner(auditViolations.sessionId, auditViolations.repositoryId, repositoryId),
    ruleId === null ? undefined : eq(auditViolations.ruleId, ruleId),
    sessionId === null ? undefined : eq(auditViolations.sessionId, sessionId),
  );
}

/** The history rows after `cursor`'s place in newest-first order, as a condition; none without a cursor. */
function historyContinuing(cursor: string | undefined): SQL | undefined {
  if (cursor === undefined) return undefined;
  const after = decodeCursor(cursor, historyCursorSchema);
  return sql`(
    ${auditViolations.startedAt} < ${after[0]}
    OR (${auditViolations.startedAt} = ${after[0]} AND (${auditViolations.auditId} < ${after[1]}
      OR (${auditViolations.auditId} = ${after[1]} AND ${auditViolations.ordinal} > ${after[2]})))
  )`;
}

/**
 * The page of `rows`, read `limit` and one more: the first `limit` rows' readable entries (`readableHistory`), and a
 * cursor after the last of those rows, readable or not, when the one more exists.
 */
function historyPage(source: AuditSource, rows: readonly HistoryRow[], limit: number, total: number): ViolationHistoryPage {
  const shown = rows.slice(0, limit);
  const last = shown.at(-1);
  return {
    violations: readableHistory(source, shown),
    total,
    nextCursor: rows.length > limit && last !== undefined ? encodeCursor([last.at, last.auditId, last.ordinal]) : null,
  };
}

/**
 * Violations newest first, one row per audit that produced them. Repeats within a single audit collapse;
 * repeats across audits stay, because the page shows a rule's history rather than its current state. Pages
 * continue from `cursor`; only the returned rows read their payload from `audits.violations_json`. A row whose payload
 * does not read is left out (`readableHistory`); the cursor and the total still count it, so a readable row's page,
 * place and total are those it has with every audit readable.
 */
function listViolationHistory(source: AuditSource, query: ViolationHistoryQuery, repositoryId?: Repository): ViolationHistoryPage {
  const { database } = source;
  const { limit } = paging(query);
  const matching = historyMatching(query, repositoryId);
  const continuing = historyContinuing(query.cursor);
  // The page and its total from one snapshot, so a commit between them cannot make them disagree. Each audit on the
  // page has its text tested once (`held`), and only a text that parses is read at each row's place, with `->`, the one
  // JSON call a row makes: `json_extract` raises on text that does not parse, and each further call on a row costs
  // another pass over the audit's text, a page of a 2 MB list twice its time.
  const { rows, total } = database.transaction(() => ({
    rows: database.all<HistoryRow>(sql`
    WITH page AS (
      SELECT ${auditViolations.auditId}, ${auditViolations.ordinal}, ${auditViolations.sessionId}, ${auditViolations.startedAt},
        ${auditViolations.logged}
      FROM ${auditViolations}
      WHERE ${and(matching, continuing)}
      ORDER BY ${auditViolations.startedAt} DESC, ${auditViolations.auditId} DESC, ${auditViolations.ordinal}
      LIMIT ${limit + 1}
    ),
    held AS MATERIALIZED (
      SELECT ${audits.auditId} AS audit_id, json_valid(${audits.violations}) AS readable FROM ${audits}
      WHERE ${audits.auditId} IN (SELECT audit_id FROM page)
    )
    SELECT
      (CASE WHEN held.readable THEN ${audits.violations} -> ('$[' || page.ordinal || ']') END) AS payload,
      page.audit_id AS auditId, page.ordinal AS ordinal, page.session_id AS sessionId, page.started_at AS at,
      ${sessions.host} AS host, page.logged AS logged
    FROM page
    JOIN held ON held.audit_id = page.audit_id
    JOIN ${audits} ON ${audits.auditId} = page.audit_id
    LEFT JOIN ${sessions} ON ${sessions.sessionId} = page.session_id
    ORDER BY page.started_at DESC, page.audit_id DESC, page.ordinal
  `),
    total: database.select({ total: count() }).from(auditViolations).where(matching).get()?.total ?? 0,
  }));
  return historyPage(source, rows, limit, total);
}

function workspaceRoots(database: RerailDatabase, auditIds: readonly string[]): Map<string, string> {
  // The ids go in as one JSON array, so a page of any size is one statement with one bound value.
  const rows = database.select({ auditId: audits.auditId, workspaceRoot: audits.workspaceRoot }).from(audits)
    .where(sql`${audits.auditId} IN (SELECT value FROM json_each(${JSON.stringify(auditIds)}))`).all();
  return new Map(rows.map(row => [row.auditId, row.workspaceRoot]));
}

/**
 * Durable, local-only audit bookkeeping. It never emits session events. Every write goes through the database's record
 * writer (`openRecordWriter`): made at once, or, while another process holds the write lock, in order once it is given
 * back, never waiting on the caller's thread; its times are taken when it is asked for. Every read here is of what this
 * process recorded, so it makes the waiting writes first (`#recorded`).
 */
export class SqliteAuditStore implements AuditRecorder, AuditReader {
  readonly supportsResultPages = true;
  readonly path: string;
  readonly #database: RerailDatabase;
  readonly #records: RecordWriter;

  constructor(path: string, database?: RerailDatabase) {
    this.path = path;
    this.#database = database ?? openRerailDatabase(path);
    this.#records = openRecordWriter(path);
  }

  start(input: AuditInput): string {
    const auditId = randomUUID();
    const provenance = input.ruleProvenance ?? [];
    const startedAt = Date.now();
    // In one immediate transaction (`RecordWrite`): the owner and provenance lookups and the insert take the write lock
    // together.
    this.#records.write(database => database
      .insert(audits)
      .values({
        auditId: auditId,
        ...auditOwner(database, input),
        workspaceRoot: input.workspaceRoot,
        scope: input.scope,
        mode: input.mode,
        origin: input.origin ?? "ongoing",
        revision: input.revision,
        catalogRevision: input.catalogRevision,
        ruleRevisions: input.ruleRevisions,
        paths: input.paths,
        ruleProvenance: [],
        // Every audit names a stored copy, `[]` included (one row for all that ran no rules): an audit that names
        // none is one a Rerail older than `audit-rule-provenance-once` wrote, and the daemon's move of older audits'
        // provenance waits while such audits come (`rule-provenance-move.ts`).
        ruleProvenanceId: storedRuleProvenanceId(database, provenance),
        startedAt,
        finishedAt: null,
        status: "running",
        violations: [],
        failures: [],
      })
      .run());
    return auditId;
  }
  appendResultPage(
    auditId: string,
    pageIndex: number,
    result: Pick<AuditResult, "violations" | "failures">,
  ): void {
    const violations = JSON.stringify(storedViolations(result.violations ?? []));
    const failures = JSON.stringify(storedFailures(result.failures ?? []));
    this.#records.write(database => database.$client
      .prepare(
        "INSERT INTO audit_result_pages(audit_id,page_index,violations_json,failures_json) " +
          "SELECT ?,?,?,? WHERE EXISTS (SELECT 1 FROM audits WHERE audit_id=? AND status='running') " +
          "ON CONFLICT(audit_id,page_index) DO UPDATE SET violations_json=excluded.violations_json,failures_json=excluded.failures_json",
      )
      .run(auditId, pageIndex, violations, failures, auditId), violations.length + failures.length);
  }

  finishResultPages(auditId: string, status: AuditStatus, unlistedFailureCount = 0): void {
    const finishedAt = Date.now();
    this.#records.write(database => {
      const finished = database.$client
        .prepare(
          "UPDATE audits SET status=?, finished_at=?, " +
            "violations_json=COALESCE((SELECT json_group_array(json(j.value)) FROM audit_result_pages p, json_each(p.violations_json) j WHERE p.audit_id=audits.audit_id ORDER BY p.page_index,j.key),'[]'), " +
            "failures_json=COALESCE((SELECT json_group_array(json(j.value)) FROM audit_result_pages p, json_each(p.failures_json) j WHERE p.audit_id=audits.audit_id ORDER BY p.page_index,j.key),'[]') " +
            "WHERE audit_id=? AND status='running'",
        )
        .run(status, finishedAt, auditId).changes > 0;
      if (finished) recordUnlisted(database, auditId, unlistedFailureCount);
      database.delete(auditResultPages).where(eq(auditResultPages.auditId, auditId)).run();
    });
  }

  progress(auditId: string, result: Pick<AuditResult, "violations" | "failures" | "unlistedFailureCount">): void {
    const violations = result.violations === undefined ? undefined : storedViolations(result.violations);
    const failures = result.failures === undefined ? undefined : storedFailures(result.failures);
    this.#records.write(database => {
      const running = database
        .update(audits)
        .set({ ...(violations === undefined ? {} : { violations }), ...(failures === undefined ? {} : { failures }) })
        .where(and(eq(audits.auditId, auditId), eq(audits.status, "running")))
        .run().changes > 0;
      if (running && failures !== undefined) recordUnlisted(database, auditId, result.unlistedFailureCount ?? 0);
    });
  }

  finish(auditId: string, result: AuditResult): void {
    const terminal = {
      status: result.status,
      violations: storedViolations(result.violations ?? []),
      failures: storedFailures(result.failures ?? []),
      finishedAt: Date.now(),
    };
    // Interruption and database reset may win a race with an evaluator's late
    // completion. Only the first terminal transition owns the persisted result.
    this.#records.write(database => {
      const finished = database
        .update(audits)
        .set(terminal)
        .where(and(eq(audits.auditId, auditId), eq(audits.status, "running")))
        .run().changes > 0;
      if (finished) recordUnlisted(database, auditId, result.unlistedFailureCount ?? 0);
    });
  }

  interrupt(sessionId: string): void {
    const running = and(eq(audits.status, "running"), or(eq(audits.sessionId, sessionId), eq(audits.recordedBy, sessionId)));
    const finishedAt = Date.now();
    // Read first: an update takes the write lock even when it matches no row, and every evaluator's open asks this of a
    // session that mostly runs nothing. A start still waiting for the lock is no row yet, so the writer then writes.
    const idle = (): boolean => this.#database.select({ auditId: audits.auditId }).from(audits).where(running).limit(1).get() === undefined;
    this.#records.writeUnless(idle, database => database.update(audits).set({ status: "interrupted", finishedAt }).where(running).run());
  }

  get(auditId: string): AuditRecord | undefined {
    const database = this.#recorded();
    const row = database.select(auditColumns(database)).from(audits).where(eq(audits.auditId, auditId)).get();
    return row === undefined ? undefined : record({ database, log: process.stderr }, row);
  }
  list(query: AuditQuery = {}, repositoryIds?: readonly string[]): AuditPage {
    return listAudits({ database: this.#recorded(), log: process.stderr }, query, repositoryIds);
  }
  listRepositoryAudits(query: RepositoryAuditQuery = {}, repositoryIds?: readonly string[]): RepositoryAuditPage {
    return listRepositoryAudits(this.#recorded(), query, repositoryIds);
  }
  listRepositoryAuditEntries(query: RepositoryAuditEntryQuery): RepositoryAuditEntryPage | undefined {
    return listRepositoryAuditEntries(this.#recorded(), query);
  }
  ruleActivity(since: number, repositoryIds?: readonly string[]): Map<string, RuleActivityTotals> {
    return ruleActivity(this.#recorded(), since, repositoryIds);
  }
  listViolationHistory(query: ViolationHistoryQuery, repositoryIds?: readonly string[]): ViolationHistoryPage {
    return listViolationHistory({ database: this.#recorded(), log: process.stderr }, query, repositoryIds);
  }
  workspaceRoots(auditIds: readonly string[]): Map<string, string> {
    return workspaceRoots(this.#recorded(), auditIds);
  }
  sessionsWithViolations(since: number, repositoryIds?: readonly string[]): number {
    return sessionsWithViolations(this.#recorded(), since, repositoryIds);
  }

  latestSummary(sessionId: string): AuditSummary | undefined {
    return latestAuditSummary(this.#recorded(), sessionId);
  }

  /**
   * The connection for a read of what this process recorded, its waiting writes made first: a scan's verdict reads the
   * audits it just made, which a holder of a moment in another process would otherwise hide.
   */
  #recorded(): RerailDatabase {
    flushRecordWrites(this.path);
    return this.#database;
  }

  close(): void {
    this.#records.close();
    this.#database.$client.close();
  }
}

/** Keeps how many failures an audit had beyond those it lists, replacing what it kept before; none when it lists all. */
function recordUnlisted(database: RerailDatabase, auditId: string, count: number): void {
  database.delete(auditUnlistedFailures).where(eq(auditUnlistedFailures.auditId, auditId)).run();
  if (count > 0) database.insert(auditUnlistedFailures).values({ auditId, count }).run();
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
  readonly #log: AuditSource["log"];

  /** `log`: where the line that names an audit whose stored lists do not read goes (`sayUnreadable`). */
  constructor(path: string, { repositoryId, log = process.stderr }: { repositoryId?: string | readonly string[]; log?: AuditSource["log"] } = {}) {
    this.path = path;
    this.#repositoryId = repositoryId;
    this.#log = log;
    this.#database = new LazyReadonlyRerailDatabase(path);
  }

  get(auditId: string): AuditRecord | undefined {
    const database = this.#database.open();
    if (database === undefined) return undefined;
    const row = database.select(auditColumns(database)).from(audits).where(eq(audits.auditId, auditId)).get();
    return row === undefined ? undefined : record({ database, log: this.#log }, row);
  }
  list(query: AuditQuery = {}, repositoryIds?: readonly string[]): AuditPage {
    const database = this.#database.open();
    return database === undefined ? { audits: [], total: 0 } : listAudits({ database, log: this.#log }, query, repositoryIds ?? this.#repositoryId);
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
    // A malformed cursor is refused whether or not any history exists yet.
    if (query.cursor !== undefined) decodeCursor(query.cursor, historyCursorSchema);
    const database = this.#database.open();
    return database === undefined
      ? { violations: [], total: 0, nextCursor: null }
      : listViolationHistory({ database, log: this.#log }, query, repositoryIds ?? this.#repositoryId);
  }
  workspaceRoots(auditIds: readonly string[]): Map<string, string> {
    const database = this.#database.open();
    return database === undefined ? new Map() : workspaceRoots(database, auditIds);
  }
  sessionsWithViolations(since: number, repositoryIds?: readonly string[]): number {
    const database = this.#database.open();
    return database === undefined ? 0 : sessionsWithViolations(database, since, repositoryIds ?? this.#repositoryId);
  }

  latestSummary(sessionId: string): AuditSummary | undefined {
    const database = this.#database.open();
    return database === undefined ? undefined : latestAuditSummary(database, sessionId);
  }

  /**
   * Of `auditIds`, those whose stored lists their records all read (`unreadableLists`), in one statement and with no list
   * parsed in JavaScript: each stored rule provenance copy they name is tested once, however many of them name it.
   */
  readableAudits(auditIds: readonly string[]): Set<string> {
    const ids = sql`(SELECT value FROM json_each(${JSON.stringify(auditIds)}))`;
    // A copy's test joined by its id, as `ruleProvenanceText` reads the copy; the audit's own text where it names none.
    const provenance = sql`coalesce(copies.held, ${holdsList(audits.ruleProvenance, "object")})`;
    const held = Object.fromEntries(LIST_NAMES.map(list => [
      list, list === "ruleProvenance" ? provenance : holdsList(listText(list), STORED_LISTS[list].item),
    ])) as Record<StoredList, SQL>;
    const rows = this.#database.open()?.all<{ auditId: string }>(sql`WITH copies AS MATERIALIZED (
        SELECT provenance_id, ${holdsList(sql`rule_provenance_json`, "object")} AS held FROM ${auditRuleProvenances}
        WHERE provenance_id IN (SELECT rule_provenance_id FROM ${audits} WHERE audit_id IN ${ids}))
      SELECT ${audits.auditId} AS auditId FROM ${audits} LEFT JOIN copies ON copies.provenance_id = ${audits.ruleProvenanceId}
      WHERE ${audits.auditId} IN ${ids} AND ${unreadableLists(held)} = 0`) ?? [];
    return new Set(rows.map(row => row.auditId));
  }

  close(): void {
    this.#database.close();
  }
}
