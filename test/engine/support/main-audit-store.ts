/**
 * How Rerail at `main` reads one audit, before an audit's rule provenance was stored once: the functions of `main`'s
 * `src/engine/audit/audit-store.ts` that `SqliteAuditStore.get` runs (`holdsUnlistedTable`, `unlistedFailures`,
 * `auditColumns`, `record`) and that class's constructor, `get` and `close`, each copied as it is, so a test can read a
 * file this build wrote or moved the way an older Rerail sharing the file reads it. `main`'s `list` and the support
 * bundle build each record from the same `auditColumns` and `record`; what they add (filters, paging, totals) reads no
 * provenance and is left out. The imports name this build's modules: this change edits none of them but `schema.ts`,
 * which adds a table and a column to `audits` that `record` never reads. Keep it as it is: it stands for the older
 * build, not for `src/`.
 */
import { eq, getTableColumns, sql, type SQL } from "drizzle-orm";
import { openRerailDatabase, type RerailDatabase } from "../../../src/engine/storage/database.js";
import { holdsTable } from "../../../src/engine/storage/table-probe.js";
import { blindFilePaths, type AuditRecord } from "../../../src/engine/audit/audit.js";
import { audits, type AuditRow } from "../../../src/engine/storage/schema.js";

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
/** An `audits` row with `unlistedFailures`, which `record` reads. */
function auditColumns(database: RerailDatabase) {
  return { ...getTableColumns(audits), unlistedFailureCount: unlistedFailures(database) };
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

/** Durable, local-only audit bookkeeping. It never emits session events. */
export class SqliteAuditStore {
  readonly path: string;
  readonly #database: RerailDatabase;

  constructor(path: string, database?: RerailDatabase) {
    this.path = path;
    this.#database = database ?? openRerailDatabase(path);
  }

  get(auditId: string): AuditRecord | undefined {
    const row = this.#database.select(auditColumns(this.#database)).from(audits).where(eq(audits.auditId, auditId)).get();
    return row === undefined ? undefined : record(row);
  }

  close(): void {
    this.#database.$client.close();
  }
}
