import { spawnSync } from "node:child_process";
import { randomUUID } from "node:crypto";
import { chmodSync, existsSync, linkSync, mkdirSync, renameSync, rmSync } from "node:fs";
import Database from "better-sqlite3";
import { basename, dirname, join, resolve } from "node:path";
import { deserialize, serialize } from "node:v8";
import { and, eq, ne, sql } from "drizzle-orm";
import { drizzle, type BetterSQLite3Database } from "drizzle-orm/better-sqlite3";
import { RULE_MODES, type RuleMode } from "./rule-mode.js";
import type { RuleDocument } from "./rule-store.js";
import * as schema from "./schema.js";
import { RERAIL_SCHEMA_STATEMENTS } from "./schema-sql.js";
import { RERAIL_V14_SCHEMA_STATEMENTS } from "./schema-v14-sql.js";
import { enableWal } from "./sqlite-wal.js";
import { GIT_EXECUTABLE, readRepositoryId, repositoryIdentityLocation } from "./repository-identity.js";
import { ruleDocumentDigest, stableJsonText } from "./stable-json.js";
import { defaultRerailHome } from "./rerail-home.js";

export type RerailDatabase = BetterSQLite3Database<typeof schema> & { readonly $client: Database.Database };

export interface OpenRerailDatabaseOptions {
  /** Open an existing database without write access; the schema is not applied. */
  readonly?: boolean;
}

const LEGACY_TABLES = ["transcript_sessions", "transcript_events", "policy_replay_events", "policy_program_artifacts"];

/** The one Rerail database: rules and every repository's history, in the Rerail home. */
export const RERAIL_DATABASE_FILE = "rerail.sqlite";
/** The machine rule catalog before rules and history shared a file; adopted once, then left in place. */
export const LEGACY_RULE_CATALOG_FILE = "rules.sqlite";
/** A checkout's history before it moved into the Rerail home; its legacy `rules` table is still imported. */
export const LEGACY_CHECKOUT_DATABASE = join(".rerail", "transcripts.sqlite");

/** `$RERAIL_HOME/rerail.sqlite`, with the Rerail home defaulting to `~/.rerail`. */
export function defaultRerailDatabasePath(environment: NodeJS.ProcessEnv = process.env): string {
  return join(defaultRerailHome(environment), RERAIL_DATABASE_FILE);
}

/** Where a checkout kept its own database before the Rerail home held all history. */
export function legacyCheckoutDatabasePath(workspaceRoot: string): string {
  return join(resolve(workspaceRoot), LEGACY_CHECKOUT_DATABASE);
}

const SCHEMA_REVISION_TABLE = "rerail_schema_revision";
const SCHEMA_REVISION_TABLE_DDL =
  "CREATE TABLE IF NOT EXISTS `rerail_schema_revision` (singleton INTEGER PRIMARY KEY CHECK(singleton = 1), revision INTEGER NOT NULL)";

function recordSchemaRevision(database: RerailDatabase, revision: number): void {
  database.run(sql.raw(SCHEMA_REVISION_TABLE_DDL));
  database.run(sql.raw(
    `INSERT INTO \`${SCHEMA_REVISION_TABLE}\` (singleton, revision) VALUES (1, ${revision}) `
    + "ON CONFLICT(singleton) DO UPDATE SET revision = MAX(revision, excluded.revision)",
  ));
}

/**
 * Steps from an older stamped version to the next one. Each entry
 * runs before the current DDL is (re)applied. Version 0 is a v1 database
 * written before the stamp existed.
 */
type SchemaUpgrade = (database: RerailDatabase) => void;

/** The `CREATE TABLE` statement for `table` in `statements`. */
function tableStatement(statements: readonly string[], table: string, version: string): string {
  const statement = statements.find(candidate => candidate.startsWith(`CREATE TABLE IF NOT EXISTS \`${table}\` (`));
  if (statement === undefined) throw new Error(`Rerail ${version} schema is missing ${table} DDL`);
  return statement;
}

/** Steps below 14 build their tables as v14 defined them, so every older database reaches the v14 shape first. */
function v14TableStatement(table: string): string {
  return tableStatement(RERAIL_V14_SCHEMA_STATEMENTS, table, "v14");
}

function tableNames(database: RerailDatabase): Set<string> {
  return new Set(database.all<{ name: string }>(sql`SELECT name FROM sqlite_master WHERE type = 'table'`).map(row => row.name));
}

function tableColumns(database: RerailDatabase, table: string): Set<string> {
  return new Set(database.all<{ name: string }>(sql`SELECT name FROM pragma_table_info(${table})`).map(row => row.name));
}

/** Upgrade either historical kind-based shape without disturbing an already-current table. */
const migratePolicyExecutionAxes: SchemaUpgrade = database => {
  const columns = database.all<{ name: string }>(sql`PRAGMA table_info(policy_runs)`).map(row => row.name);
  if (!columns.includes("kind")) return;
  database.run(sql.raw("ALTER TABLE `policy_runs` RENAME TO `policy_runs_legacy`"));
  database.run(sql.raw(v14TableStatement("policy_runs")));
  database.run(sql.raw(
    "INSERT INTO `policy_runs` (" +
      "`run_id`, `session_id`, `workspace_root`, `scope`, `mode`, `revision`, `rule_provenance_json`, `paths_json`, " +
      "`started_at`, `finished_at`, `status`, `findings_json`, `failures_json`" +
    ") SELECT " +
      "`run_id`, `session_id`, `workspace_root`, " +
      "CASE `kind` " +
        "WHEN 'full-scan' THEN 'repository' WHEN 'scoped-scan' THEN 'touched-files' " +
        "WHEN 'ratchet' THEN 'ai-output' WHEN 'shadow' THEN 'repository' ELSE 'touched-files' END, " +
      "CASE `kind` WHEN 'generation' THEN 'resteer' ELSE 'log' END, " +
      "`revision`, '[]', `scope_json`, `started_at`, `finished_at`, `status`, `findings_json`, `failures_json` " +
    "FROM `policy_runs_legacy` WHERE `kind` <> 'delivery'",
  ));
  database.run(sql.raw("DROP TABLE `policy_runs_legacy`"));
};

/**
 * Both schema v7 lines existed independently before the branches merged: main
 * added scoped-rule provenance while the scan-control branch added immutable
 * catalog revisions and repository diagnostics. Every entry is idempotent so
 * databases stamped by either line converge on v9.
 */
const migrateMergedPolicyMetadata: SchemaUpgrade = database => {
  const columns = new Set(database.all<{ name: string }>(sql`PRAGMA table_info(policy_runs)`).map(row => row.name));
  if (!columns.has("catalog_revision")) database.run(sql.raw("ALTER TABLE `policy_runs` ADD COLUMN `catalog_revision` text"));
  if (!columns.has("rule_revisions_json"))
    database.run(sql.raw("ALTER TABLE `policy_runs` ADD COLUMN `rule_revisions_json` text NOT NULL DEFAULT '[]'"));
  if (!columns.has("rule_provenance_json"))
    database.run(sql.raw("ALTER TABLE `policy_runs` ADD COLUMN `rule_provenance_json` text NOT NULL DEFAULT '[]'"));

  const tables = new Set(database.all<{ name: string }>(sql`SELECT name FROM sqlite_master WHERE type = 'table'`).map(row => row.name));
  if (tables.has("policy_settings"))
    database.run(sql.raw("UPDATE `policy_settings` SET `settings_json` = json_remove(`settings_json`, '$.initialMode')"));
  database.run(sql.raw(v14TableStatement("policy_repository_diagnostics")));
  database.run(sql.raw(
    "INSERT OR IGNORE INTO `policy_repository_diagnostics` " +
      "(`workspace_root`, `session_id`, `run_id`, `status`, `started_at`, `finished_at`, `error`) " +
    "SELECT p.`workspace_root`, p.`session_id`, p.`run_id`, " +
      "CASE p.`status` WHEN 'running' THEN 'interrupted' WHEN 'superseded' THEN 'interrupted' ELSE p.`status` END, " +
      "p.`started_at`, COALESCE(p.`finished_at`, " + Date.now() + "), " +
      "CASE WHEN p.`status` IN ('running', 'superseded') THEN 'Historic repository diagnostic did not complete' ELSE NULL END " +
    "FROM `policy_runs` p WHERE p.`scope` = 'repository' AND p.`mode` = 'log' AND NOT EXISTS (" +
      "SELECT 1 FROM `policy_runs` earlier WHERE earlier.`workspace_root` = p.`workspace_root` " +
        "AND earlier.`scope` = 'repository' AND earlier.`mode` = 'log' " +
        "AND (earlier.`started_at` < p.`started_at` OR (earlier.`started_at` = p.`started_at` AND earlier.`run_id` < p.`run_id`))" +
    ")",
  ));
};

/** The checkout-local opt-out that v16 replaced; read once by the upgrade, never again. */
const RETIRED_DISABLED_MARKER = join(".rerail", "disabled");

function knownRepositoryId(path: string): string | undefined {
  try {
    return readRepositoryId(path);
  } catch {
    return undefined;
  }
}

function hasTable(database: RerailDatabase, name: string): boolean {
  return database.get<{ name: string } | undefined>(sql`SELECT name FROM sqlite_master WHERE type = 'table' AND name = ${name}`) !== undefined;
}

/** Create one table of the current schema ahead of the full DDL, for an upgrade that writes to it. */
function createSchemaTable(database: RerailDatabase, name: string): void {
  const statement = RERAIL_SCHEMA_STATEMENTS.find(candidate => candidate.startsWith(`CREATE TABLE IF NOT EXISTS \`${name}\``));
  if (statement === undefined) throw new Error(`Rerail schema is missing ${name} DDL`);
  database.run(sql.raw(statement));
}

function latestSessionRowid(database: RerailDatabase): number {
  if (!hasTable(database, "sessions")) return 0;
  return database.get<{ latest: number | null }>(sql`SELECT max(rowid) AS latest FROM sessions`)?.latest ?? 0;
}

/** The last session rowid the start carry-over read. Releases before v16 never write it. */
function sessionsCarriedOver(database: RerailDatabase): number {
  if (!hasTable(database, "repository_start_carry_over")) return 0;
  return database.select({ rowid: schema.repositoryStartCarryOver.sessionRowid })
    .from(schema.repositoryStartCarryOver).get()?.rowid ?? 0;
}

/** The watermark is set, not maximized: after a reset deletes the newest sessions, rowids are reused below it. */
function recordSessionsCarriedOver(database: RerailDatabase, rowid: number): void {
  createSchemaTable(database, "repository_start_carry_over");
  database.insert(schema.repositoryStartCarryOver).values({ singleton: 1, sessionRowid: rowid })
    .onConflictDoUpdate({ target: schema.repositoryStartCarryOver.singleton, set: { sessionRowid: rowid } }).run();
}

/**
 * Whether sessions changed since the carry-over last ran: newer ones, or a reset that removed the newest.
 * A reader that finds one opens writable.
 */
export function repositoryStartCarryOverPending(database: RerailDatabase): boolean {
  return latestSessionRowid(database) !== sessionsCarriedOver(database);
}

/** Add `sessions.start_gated` to a database written before it; older writers leave every row 0. */
function addStartGatedColumn(database: RerailDatabase): void {
  const columns = new Set(database.all<{ name: string }>(sql`PRAGMA table_info(sessions)`).map(row => row.name));
  if (columns.size > 0 && !columns.has("start_gated")) {
    database.run(sql.raw("ALTER TABLE `sessions` ADD COLUMN `start_gated` integer DEFAULT false NOT NULL"));
  }
}

/**
 * Each repository's checkouts by id, from sessions an older writer recorded after `afterRowid`. The
 * identity file names a repository for sessions recorded before attribution, which releases before v13
 * never wrote.
 */
function sessionCheckouts(database: RerailDatabase, afterRowid: number): Map<string, string> {
  const checkouts = new Map<string, string>();
  if (!hasTable(database, "sessions")) return checkouts;
  const rows = database.selectDistinct({ cwd: schema.sessions.cwd, repositoryId: schema.sessions.repositoryId })
    .from(schema.sessions)
    .where(and(sql`${schema.sessions}.rowid > ${afterRowid}`, eq(schema.sessions.startGated, false))).all();
  for (const row of rows) {
    const repositoryId = row.repositoryId ?? knownRepositoryId(row.cwd);
    if (repositoryId !== undefined) checkouts.set(row.cwd, repositoryId);
  }
  return checkouts;
}

/** Whether any worktree of `repositoryId` has its own on/off: the repository was set up there. */
function hasCheckoutStart(database: RerailDatabase, repositoryId: string): boolean {
  return hasTable(database, "checkout_starts") && database.select({ repositoryId: schema.checkoutStarts.repositoryId })
    .from(schema.checkoutStarts).where(eq(schema.checkoutStarts.repositoryId, repositoryId)).limit(1).get() !== undefined;
}

/** The linked worktree whose top folder holds `path`, or undefined for a main checkout or a folder Git cannot read. */
function linkedWorktreeRoot(path: string): string | undefined {
  try {
    const location = repositoryIdentityLocation(path);
    return location.main ? undefined : location.workspaceRoot;
  } catch {
    return undefined;
  }
}

/** Turn off the one checkout that carried the retired marker: a linked worktree's own row, else the repository's. */
function stopMarkedCheckout(database: RerailDatabase, path: string, repositoryId: string, now: number): void {
  const worktree = linkedWorktreeRoot(path);
  if (worktree === undefined) {
    database.update(schema.repositoryStarts)
      .set({ state: "stopped", stoppedAt: now, updatedAt: now })
      .where(eq(schema.repositoryStarts.repositoryId, repositoryId)).run();
    return;
  }
  createSchemaTable(database, "checkout_starts");
  database.insert(schema.checkoutStarts)
    .values({ repositoryId, checkoutPath: worktree, state: "stopped", stoppedAt: now, updatedAt: now })
    .onConflictDoNothing().run();
}

/**
 * Every checkout Git lists for the repository `path` belongs to: the main checkout and each linked worktree.
 * The retired `rerail disable` wrote its marker without registering the checkout anywhere, so a worktree
 * disabled before it recorded a session is found only here. Empty where Git cannot read `path`.
 */
function gitWorktrees(path: string): string[] {
  if (!existsSync(path)) return [];
  const listed = spawnSync(GIT_EXECUTABLE, ["-C", path, "worktree", "list", "--porcelain"], { encoding: "utf8", timeout: 10_000 });
  if (listed.status !== 0) return [];
  return listed.stdout.split("\n").filter(line => line.startsWith("worktree ")).map(line => line.slice("worktree ".length));
}

/** Insert `started` for each repository without any start state; returns those inserted. Existing rows are kept. */
function insertStarted(database: RerailDatabase, repositoryIds: Iterable<string>, now: number): Set<string> {
  const added = new Set<string>();
  for (const repositoryId of repositoryIds) {
    if (hasCheckoutStart(database, repositoryId)) continue;
    const inserted = database.insert(schema.repositoryStarts)
      .values({ repositoryId, state: "started", startedAt: now, updatedAt: now })
      .onConflictDoNothing().run();
    if (inserted.changes > 0) added.add(repositoryId);
  }
  return added;
}

/** The known checkouts of the `added` repositories, plus every worktree Git lists for each, by repository. */
function checkoutsToInspect(checkouts: ReadonlyMap<string, string>, added: ReadonlySet<string>): Map<string, string> {
  const inspected = new Map([...checkouts].filter(([, repositoryId]) => added.has(repositoryId)));
  const listed = new Set<string>();
  for (const [path, repositoryId] of [...inspected]) {
    if (listed.has(repositoryId)) continue;
    const worktrees = gitWorktrees(path);
    if (worktrees.length > 0) listed.add(repositoryId);
    for (const worktree of worktrees) inspected.set(worktree, repositoryId);
  }
  return inspected;
}

/**
 * Start every repository in `checkouts` that has no start state, in the main checkout or any worktree.
 * Existing rows, a person's choice, are never changed. A checkout of a newly found repository that
 * carries the retired `.rerail/disabled` marker is stopped instead; in a linked worktree, only that worktree.
 */
function startUnrecorded(database: RerailDatabase, checkouts: ReadonlyMap<string, string>): void {
  const now = Date.now();
  const added = insertStarted(database, new Set(checkouts.values()), now);
  for (const [path, repositoryId] of checkoutsToInspect(checkouts, added)) {
    if (existsSync(join(path, RETIRED_DISABLED_MARKER))) stopMarkedCheckout(database, path, repositoryId, now);
  }
}

/**
 * v16 makes Rerail off until started in each repository. Nobody is switched off by the upgrade: a
 * repository with recorded sessions, or one a runtime already registered in the catalog, counts as
 * started. A checkout that carried the retired `.rerail/disabled` marker is stopped: the repository, or
 * only that worktree when the marker sits in a linked worktree.
 *
 * The backfill runs whenever the table is missing, not only at step 15: a build from a parallel line can
 * stamp a later revision without ever creating `repository_starts`, and skipping it would switch every
 * repository off.
 *
 * After it, every writable open carries over the sessions an older release recorded since the last pass.
 * A release before v16 records in every repository and ignores this table, so a rollback followed by
 * another update leaves the repositories used in between with history and no state. This release, by
 * contrast, records wherever a checkout counts as started, which includes a flagged development
 * workspace (`NERA_RERAIL_ENABLED=true`) that has no row; the event stream and the daemon kernel record
 * such a session in the host database. Every session this release records is marked `start_gated`,
 * and only unmarked ones are carried over, so neither the flag nor a seeded mock session switches a
 * repository on. A repository with only a worktree's own row was set up there and is skipped.
 */
const migrateRepositoryStarts: SchemaUpgrade = database => {
  addStartGatedColumn(database);
  const latest = latestSessionRowid(database);
  if (!hasTable(database, "repository_starts")) {
    createSchemaTable(database, "repository_starts");
    const checkouts = sessionCheckouts(database, 0);
    if (hasTable(database, "rule_repositories")) {
      const rows = database.select().from(schema.ruleRepositories).where(ne(schema.ruleRepositories.lastKnownPath, "")).all();
      for (const row of rows) checkouts.set(row.lastKnownPath, row.repositoryId);
    }
    startUnrecorded(database, checkouts);
  } else {
    const carriedOver = sessionsCarriedOver(database);
    if (latest === carriedOver) return;
    // Below the watermark only after a reset; those rowids will be handed out again.
    if (latest > carriedOver) startUnrecorded(database, sessionCheckouts(database, carriedOver));
  }
  recordSessionsCarriedOver(database, latest);
};

/**
 * v13 attributes sessions to a repository now that every repository shares one database. A file without a sessions
 * table gets the column from the current DDL instead.
 */
const addSessionRepository: SchemaUpgrade = database => {
  const columns = tableColumns(database, "sessions");
  if (columns.size > 0 && !columns.has("repository_id")) database.run(sql.raw("ALTER TABLE `sessions` ADD COLUMN `repository_id` text"));
};

/** The rule kinds every schema before v15 stored. */
type LegacyRuleKind = "policy" | "cosine";
/** The v11 finding severities and cosine deliveries, by the rule mode each became in v14. */
const LEGACY_SEVERITY_MODES: Readonly<Record<string, RuleMode>> = { advisory: "log", warning: "steer", error: "steer" };
const LEGACY_DELIVERY_MODES: Readonly<Record<string, RuleMode>> = { observe: "log", steer: "steer", followUp: "steer", nextTurn: "steer" };

/** A rule body as v11 stored it: `finding.severity` on each policy check, `action.delivery` on a cosine rule, no `mode`. */
interface LegacyRule {
  id: string;
  checks?: { finding?: { severity?: unknown; [key: string]: unknown }; [key: string]: unknown }[];
  action?: { delivery?: unknown; [key: string]: unknown };
  [key: string]: unknown;
}

interface ModedRule extends LegacyRule {
  mode: RuleMode;
}

interface LegacyRuleRow {
  id: string;
  kind: LegacyRuleKind;
  revision: number;
  source_digest: string | null;
  format: string | null;
  description: string | null;
  scope_json: string | null;
  checks_json: string | null;
  reference: string | null;
  match_json: string | null;
  sample_json: string | null;
  action_json: string | null;
  repeat_json: string | null;
  [column: string]: unknown;
}

function legacyMode(modes: Readonly<Record<string, RuleMode>>, value: unknown, subject: string, field: string): RuleMode {
  if (typeof value === "string" && Object.hasOwn(modes, value)) return modes[value]!;
  throw new Error(`Cannot upgrade ${subject} to rule modes: ${field} ${JSON.stringify(value)} has no rule mode`);
}

/** A policy takes the strongest mode among its reporting checks' severities; a define check has no finding. */
function modedPolicy(rule: LegacyRule): ModedRule {
  let mode: RuleMode | undefined;
  const checks = (rule.checks ?? []).map(check => {
    if (check.finding === undefined) return check;
    const { severity, ...finding } = check.finding;
    const checkMode = legacyMode(LEGACY_SEVERITY_MODES, severity, `rule ${rule.id}`, "finding severity");
    if (mode === undefined || RULE_MODES.indexOf(checkMode) > RULE_MODES.indexOf(mode)) mode = checkMode;
    return { ...check, finding };
  });
  if (mode === undefined) throw new Error(`Cannot upgrade rule ${rule.id} to rule modes: none of its checks reports a finding`);
  return { ...rule, mode, checks };
}

function modedCosine(rule: LegacyRule): ModedRule {
  const { delivery, ...action } = rule.action ?? {};
  return { ...rule, mode: legacyMode(LEGACY_DELIVERY_MODES, delivery, `rule ${rule.id}`, "action delivery"), action };
}

function modedRule(kind: LegacyRuleKind, rule: LegacyRule): ModedRule {
  return kind === "policy" ? modedPolicy(rule) : modedCosine(rule);
}

/** Digest of a rule document under any kind a schema has stored; `ruleDocumentDigest` only serializes it. */
function documentDigest(kind: string, rule: unknown): string {
  return ruleDocumentDigest({ kind, rule } as unknown as RuleDocument);
}

/** The body `rowToStoredRule` reads back from a v11 row: NULL optional columns are omitted, not null. */
function legacyRowRule(row: LegacyRuleRow): LegacyRule {
  if (row.kind === "policy") {
    return {
      format: row.format,
      id: row.id,
      revision: row.revision,
      ...(row.description === null ? {} : { description: row.description }),
      ...(row.scope_json === null ? {} : { scope: JSON.parse(row.scope_json) }),
      checks: JSON.parse(row.checks_json ?? "null"),
    };
  }
  return {
    id: row.id,
    ...(row.reference === null ? {} : { reference: row.reference }),
    match: JSON.parse(row.match_json ?? "null"),
    sample: JSON.parse(row.sample_json ?? "null"),
    action: JSON.parse(row.action_json ?? "null"),
    ...(row.repeat_json === null ? {} : { repeat: JSON.parse(row.repeat_json) }),
  };
}

/** Bundled provenance follows the rewrite only where the row still held the bundled content. */
function restampedSourceDigest(sourceDigest: string | null, before: string, after: string): string | null {
  return sourceDigest === before ? after : sourceDigest;
}

/**
 * v14 folds policy `finding.severity` and cosine `action.delivery` into one rule-level `mode`, a NOT NULL
 * column SQLite can add only by rebuilding the table (the rebuild also carries v12's relaxed cosine CHECK).
 * Stored rule bodies, catalog documents, and recorded findings are rewritten once; digests are recomputed so
 * an unedited bundled rule keeps following the bundle. Every phase skips rows already in the v14 shape.
 */
const migrateRuleModes: SchemaUpgrade = database => {
  const tables = new Set(database.all<{ name: string }>(sql`SELECT name FROM sqlite_master WHERE type = 'table'`).map(row => row.name));
  const columns = database.all<{ name: string }>(sql`PRAGMA table_info(rules)`).map(row => row.name);
  if (!columns.includes("mode")) rebuildModedRules(database);
  if (tables.has("catalog_rules")) rewriteCatalogRuleModes(database);
  if (tables.has("policy_runs")) rewriteFindingModes(database, "policy_runs");
  if (tables.has("policy_run_result_pages")) rewriteFindingModes(database, "policy_run_result_pages");
};

function rebuildModedRules(database: RerailDatabase): void {
  database.run(sql.raw("ALTER TABLE `rules` RENAME TO `rules_legacy`"));
  database.run(sql.raw(v14TableStatement("rules")));
  const current = new Set(database.all<{ name: string }>(sql`PRAGMA table_info(rules)`).map(row => row.name));
  for (const row of database.all<LegacyRuleRow>(sql.raw("SELECT * FROM `rules_legacy`"))) {
    const legacy = legacyRowRule(row);
    const rule = modedRule(row.kind, legacy);
    const values: Record<string, unknown> = {
      ...row,
      mode: rule.mode,
      ...(row.kind === "policy" ? { checks_json: JSON.stringify(rule.checks) } : { action_json: JSON.stringify(rule.action) }),
      source_digest: restampedSourceDigest(row.source_digest, documentDigest(row.kind, legacy), documentDigest(row.kind, rule)),
    };
    const names = Object.keys(values).filter(name => current.has(name));
    const columnList = sql.raw(names.map(name => `\`${name}\``).join(", "));
    const valueList = sql.join(names.map(name => sql`${values[name]}`), sql.raw(", "));
    database.run(sql`INSERT INTO rules (${columnList}) VALUES (${valueList})`);
  }
  // Dropping the old table drops its index too; the current DDL recreates it after the upgrades.
  database.run(sql.raw("DROP TABLE `rules_legacy`"));
}

function rewriteCatalogRuleModes(database: RerailDatabase): void {
  const rows = database.all<{ owner_kind: string; owner_id: string; rule_id: string; document_json: string; source_digest: string | null }>(
    sql`SELECT owner_kind, owner_id, rule_id, document_json, source_digest FROM catalog_rules`,
  );
  for (const row of rows) {
    const document: { kind: LegacyRuleKind; rule: LegacyRule } = JSON.parse(row.document_json);
    if ("mode" in document.rule) continue;
    const rule = modedRule(document.kind, document.rule);
    const digest = documentDigest(document.kind, rule);
    database.run(sql`
      UPDATE catalog_rules
      SET document_json = ${JSON.stringify({ ...document, rule })}, payload_hash = ${digest},
        source_digest = ${restampedSourceDigest(row.source_digest, documentDigest(document.kind, document.rule), digest)}
      WHERE owner_kind = ${row.owner_kind} AND owner_id = ${row.owner_id} AND rule_id = ${row.rule_id}
    `);
  }
}

/** Rewrites recorded findings in `table`, a run's whole result or one of its result pages, keyed by `rowid`. */
function rewriteFindingModes(database: RerailDatabase, table: "policy_runs" | "policy_run_result_pages"): void {
  const rows = database.all<{ row: number; run_id: string; findings_json: string }>(sql.raw(`
    SELECT rowid AS row, run_id, findings_json FROM \`${table}\`
    WHERE EXISTS (SELECT 1 FROM json_each(\`${table}\`.findings_json) AS finding WHERE json_type(finding.value, '$.severity') IS NOT NULL)
  `));
  for (const row of rows) {
    const findings: { policyId?: unknown; [key: string]: unknown }[] = JSON.parse(row.findings_json);
    const moded = findings.map(finding => {
      const subject = `rule ${String(finding.policyId)} in policy run ${row.run_id}`;
      return Object.fromEntries(Object.entries(finding).map(([key, value]) => (
        key === "severity" ? ["mode", legacyMode(LEGACY_SEVERITY_MODES, value, subject, "finding severity")] : [key, value]
      )));
    });
    database.run(sql`UPDATE ${sql.identifier(table)} SET findings_json = ${JSON.stringify(moded)} WHERE rowid = ${row.row}`);
  }
}

/** The values v14 stored, by the value each became in v15. */
const V15_RULE_KINDS: Readonly<Record<string, string>> = { policy: "file", cosine: "conversation" };
const V15_RULE_ORIGINS: Readonly<Record<string, string>> = { local: "user", migrated: "user", nera: "user", bundled: "rerail" };
const V15_EVENT_KINDS: Readonly<Record<string, string>> = { "tool-call": "call", "tool-result": "result" };
const V14_POLICY_FORMAT = "rerail.policy/v1";
const V15_POLICY_FORMAT = "rerail.policy/v1";
const V15_VIOLATION_KEYS: Readonly<Record<string, string>> = { checkId: "clauseId", policyId: "ruleId" };
const V15_COVERAGE_KEYS: Readonly<Record<string, string>> = {
  cosineRuleSignature: "conversationRuleSignature",
  policyRuleSignature: "fileRuleSignature",
};
/** Rows each rewrite reads at a time; better-sqlite3 cannot write while a read is still iterating. */
const V15_REWRITE_BATCH = 200;

type JsonObject = Record<string, unknown>;

function isJsonObject(value: unknown): value is JsonObject {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** `value` with the keys `renames` names replaced where they stand, so the key order is kept. */
function renamedKeys(value: JsonObject, renames: Readonly<Record<string, string>>): JsonObject {
  return Object.fromEntries(Object.entries(value).map(([key, child]) => [Object.hasOwn(renames, key) ? renames[key]! : key, child]));
}

function v15Kind(kind: string): string {
  return Object.hasOwn(V15_RULE_KINDS, kind) ? V15_RULE_KINDS[kind]! : kind;
}

/** A v14 policy body as v15 writes it: the v15 format, `checks` as `clauses`, each clause's `finding` as `violation`. */
function v15PolicyBody(rule: unknown): unknown {
  if (!isJsonObject(rule)) return rule;
  const renamed = renamedKeys(rule, { checks: "clauses" });
  return {
    ...renamed,
    ...(renamed.format === V14_POLICY_FORMAT ? { format: V15_POLICY_FORMAT } : {}),
    ...(Array.isArray(renamed.clauses) ? { clauses: renamed.clauses.map(v15Clause) } : {}),
  };
}

function v15Clause(clause: unknown): unknown {
  return isJsonObject(clause) ? renamedKeys(clause, { finding: "violation" }) : clause;
}

/** Recorded violations and failures name their clause and rule as `clauseId` and `ruleId`. */
function v15Violations(json: string): string {
  const entries: unknown = JSON.parse(json);
  if (!Array.isArray(entries)) return json;
  return JSON.stringify(entries.map(entry => (isJsonObject(entry) ? renamedKeys(entry, V15_VIOLATION_KEYS) : entry)));
}

/** Recorded provenance keeps its digests; origins take their v15 value and the sync fields are gone. */
function v15Provenance(json: string): string {
  const entries: unknown = JSON.parse(json);
  if (!Array.isArray(entries)) return json;
  return JSON.stringify(entries.map(entry => {
    if (!isJsonObject(entry)) return entry;
    const { syncState: _syncState, upstreamRuleId: _upstreamRuleId, ...kept } = entry;
    const origin = typeof kept.origin === "string" && Object.hasOwn(V15_RULE_ORIGINS, kept.origin) ? V15_RULE_ORIGINS[kept.origin] : kept.origin;
    return { ...kept, origin };
  }));
}

interface V14RuleRow extends LegacyRuleRow {
  mode: string;
}

/** The body the v14 `rules` reader rebuilt from a row: `mode` included, NULL optional columns omitted. */
function v14StoredRule(row: V14RuleRow): JsonObject {
  if (row.kind === "policy") {
    return {
      format: row.format,
      id: row.id,
      revision: row.revision,
      mode: row.mode,
      ...(row.description === null ? {} : { description: row.description }),
      ...(row.scope_json === null ? {} : { scope: JSON.parse(row.scope_json) }),
      checks: JSON.parse(row.checks_json ?? "null"),
    };
  }
  return {
    id: row.id,
    mode: row.mode,
    ...(row.reference === null ? {} : { reference: row.reference }),
    match: JSON.parse(row.match_json ?? "null"),
    sample: JSON.parse(row.sample_json ?? "null"),
    action: JSON.parse(row.action_json ?? "null"),
    ...(row.repeat_json === null ? {} : { repeat: JSON.parse(row.repeat_json) }),
  };
}

function currentTableStatement(table: string): string {
  return tableStatement(RERAIL_SCHEMA_STATEMENTS, table, `v${schema.RERAIL_SCHEMA_VERSION}`);
}

/** Read `table` in rowid order, a batch at a time, handing each row to `rewrite` once. */
function forEachRow<Row extends { row: number }>(
  database: RerailDatabase,
  select: (after: number) => ReturnType<typeof sql>,
  rewrite: (row: Row) => void,
): void {
  let after = Number.MIN_SAFE_INTEGER;
  for (;;) {
    const rows = database.all<Row>(select(after));
    for (const row of rows) rewrite(row);
    if (rows.length < V15_REWRITE_BATCH) return;
    after = rows.at(-1)!.row;
  }
}

function insertRow(database: RerailDatabase, table: string, values: Readonly<Record<string, unknown>>): void {
  const names = Object.keys(values);
  const columnList = sql.raw(names.map(name => `\`${name}\``).join(", "));
  const valueList = sql.join(names.map(name => sql`${values[name]}`), sql.raw(", "));
  database.run(sql`INSERT INTO ${sql.identifier(table)} (${columnList}) VALUES (${valueList})`);
}

/**
 * The checkout `rules` table carries kind and format CHECKs, so v15 rebuilds it. `rules` is no foreign-key parent,
 * so renaming it aside is safe. A bundled row keeps following the bundle: its source digest moves with the rewrite
 * only where it equalled the digest of the body the v14 reader rebuilt.
 */
function rebuildRerailRules(database: RerailDatabase): void {
  if (!tableColumns(database, "rules").has("checks_json")) return;
  database.run(sql.raw("ALTER TABLE `rules` RENAME TO `rules_legacy`"));
  database.run(sql.raw(currentTableStatement("rules")));
  const current = tableColumns(database, "rules");
  for (const row of database.all<V14RuleRow>(sql.raw("SELECT * FROM `rules_legacy`"))) {
    const kind = v15Kind(row.kind);
    const before = v14StoredRule(row);
    const after = row.kind === "policy" ? v15PolicyBody(before) as JsonObject : before;
    const { checks_json: checksJson, ...columns } = row;
    const values: Record<string, unknown> = {
      ...columns,
      kind,
      ...(row.kind === "policy"
        ? { format: after.format, clauses_json: checksJson === null ? null : JSON.stringify(after.clauses) }
        : {}),
      source_digest: restampedSourceDigest(row.source_digest, documentDigest(row.kind, before), documentDigest(kind, after)),
    };
    insertRow(database, "rules", Object.fromEntries(Object.entries(values).filter(([name]) => current.has(name))));
  }
  // Dropping the old table drops its index too; the current DDL recreates it after the upgrades.
  database.run(sql.raw("DROP TABLE `rules_legacy`"));
}

/** Catalog documents carry `{ kind, rule }`; the payload hash is always the digest of the stored document. */
function rewriteRerailCatalog(database: RerailDatabase, tables: ReadonlySet<string>): void {
  if (tables.has("rule_catalog_identities")) {
    database.run(sql.raw(
      "UPDATE `rule_catalog_identities` SET `kind` = CASE `kind` WHEN 'policy' THEN 'file' ELSE 'conversation' END " +
      "WHERE `kind` IN ('policy', 'cosine')",
    ));
  }
  if (!tables.has("catalog_rules")) return;
  const rows = database.all<{ owner_kind: string; owner_id: string; rule_id: string; kind: string; document_json: string; source_digest: string | null }>(
    sql`SELECT owner_kind, owner_id, rule_id, kind, document_json, source_digest FROM catalog_rules WHERE kind IN ('policy', 'cosine')`,
  );
  for (const row of rows) {
    const document: unknown = JSON.parse(row.document_json);
    const before = isJsonObject(document) ? document : { kind: row.kind, rule: document };
    const beforeKind = typeof before.kind === "string" ? before.kind : row.kind;
    const after = { ...before, kind: v15Kind(beforeKind), rule: beforeKind === "policy" ? v15PolicyBody(before.rule) : before.rule };
    const digest = documentDigest(after.kind, after.rule);
    database.run(sql`
      UPDATE catalog_rules
      SET kind = ${v15Kind(row.kind)}, document_json = ${JSON.stringify(after)}, payload_hash = ${digest},
        source_digest = ${restampedSourceDigest(row.source_digest, documentDigest(beforeKind, before.rule), digest)}
      WHERE owner_kind = ${row.owner_kind} AND owner_id = ${row.owner_id} AND rule_id = ${row.rule_id}
    `);
  }
  database.run(sql.raw(
    "UPDATE `catalog_rules` SET `origin` = CASE `origin` WHEN 'bundled' THEN 'rerail' ELSE 'user' END " +
    "WHERE `origin` IN ('local', 'migrated', 'nera', 'bundled')",
  ));
  const columns = tableColumns(database, "catalog_rules");
  for (const column of ["sync_state", "upstream_rule_id"]) {
    if (columns.has(column)) database.run(sql.raw(`ALTER TABLE \`catalog_rules\` DROP COLUMN \`${column}\``));
  }
}

/**
 * Audits replace policy runs. `policy_runs` is the foreign-key parent of its result pages, so it is never renamed:
 * renaming it rewrites the child's reference, and dropping the renamed parent would cascade into the pages. The new
 * tables are built and filled parent first, and the old ones dropped child first. A page whose run is gone has no
 * audit to belong to and is dropped with its table.
 */
function copyRerailAudits(database: RerailDatabase, tables: ReadonlySet<string>): void {
  if (!tables.has("policy_runs") && !tables.has("policy_run_result_pages")) return;
  database.run(sql.raw(currentTableStatement("audits")));
  if (tables.has("policy_runs")) forEachRow<{ row: number; [column: string]: unknown }>(
    database,
    after => sql`SELECT rowid AS row, * FROM policy_runs WHERE rowid > ${after} ORDER BY rowid LIMIT ${V15_REWRITE_BATCH}`,
    run => insertRow(database, "audits", {
      audit_id: run.run_id,
      session_id: run.session_id,
      workspace_root: run.workspace_root,
      scope: run.scope,
      mode: run.mode === "resteer" ? "steer" : run.mode,
      revision: run.revision,
      catalog_revision: run.catalog_revision,
      rule_revisions_json: run.rule_revisions_json,
      rule_provenance_json: v15Provenance(run.rule_provenance_json as string),
      paths_json: run.paths_json,
      started_at: run.started_at,
      finished_at: run.finished_at,
      status: run.status,
      violations_json: v15Violations(run.findings_json as string),
      failures_json: v15Violations(run.failures_json as string),
    }),
  );
  if (tables.has("policy_run_result_pages")) {
    database.run(sql.raw(currentTableStatement("audit_result_pages")));
    forEachRow<{ row: number; run_id: string; page_index: number; findings_json: string; failures_json: string }>(
      database,
      after => sql`
        SELECT rowid AS row, run_id, page_index, findings_json, failures_json FROM policy_run_result_pages
        WHERE rowid > ${after} AND run_id IN (SELECT audit_id FROM audits) ORDER BY rowid LIMIT ${V15_REWRITE_BATCH}
      `,
      page => insertRow(database, "audit_result_pages", {
        audit_id: page.run_id,
        page_index: page.page_index,
        violations_json: v15Violations(page.findings_json),
        failures_json: v15Violations(page.failures_json),
      }),
    );
    database.run(sql.raw("DROP TABLE `policy_run_result_pages`"));
  }
  if (tables.has("policy_runs")) database.run(sql.raw("DROP TABLE `policy_runs`"));
}

/** Audit settings, repository audits, and audit requests hold no foreign keys; their rows copy across in SQL. */
function copyRerailAuditControls(database: RerailDatabase, tables: ReadonlySet<string>): void {
  if (tables.has("policy_settings")) {
    database.run(sql.raw(currentTableStatement("audit_settings")));
    database.run(sql.raw(
      "INSERT INTO `audit_settings` (`workspace_root`, `settings_json`, `updated_at`) " +
      "SELECT `workspace_root`, CASE WHEN json_valid(`settings_json`) AND json_extract(`settings_json`, '$.mode') = 'resteer' " +
        "THEN json_set(`settings_json`, '$.mode', 'steer') ELSE `settings_json` END, `updated_at` FROM `policy_settings`",
    ));
    database.run(sql.raw("DROP TABLE `policy_settings`"));
  }
  if (tables.has("policy_repository_diagnostics")) {
    database.run(sql.raw(currentTableStatement("repository_audits")));
    database.run(sql.raw(
      "INSERT INTO `repository_audits` (`workspace_root`, `session_id`, `audit_id`, `status`, `started_at`, `finished_at`, `error`) " +
      "SELECT `workspace_root`, `session_id`, `run_id`, `status`, `started_at`, `finished_at`, `error` FROM `policy_repository_diagnostics`",
    ));
    database.run(sql.raw("DROP TABLE `policy_repository_diagnostics`"));
  }
  if (tables.has("policy_scan_requests")) {
    database.run(sql.raw(currentTableStatement("audit_requests")));
    database.run(sql.raw(
      "INSERT INTO `audit_requests` (`id`, `session_id`, `workspace_root`, `scope`, `mode`, `paths_json`, `status`, " +
        "`created_at`, `started_at`, `finished_at`, `audit_ids_json`, `error`) " +
      "SELECT `id`, `session_id`, `workspace_root`, `scope`, CASE `mode` WHEN 'resteer' THEN 'steer' ELSE `mode` END, " +
        "`paths_json`, `status`, `created_at`, `started_at`, `finished_at`, `run_ids_json`, `error` FROM `policy_scan_requests`",
    ));
    database.run(sql.raw("DROP TABLE `policy_scan_requests`"));
  }
}

/** A V8 payload with its keys renamed; a payload that is not an object, or does not decode, is kept as it is. */
function v15EventPayload(payload: Buffer, rewrite: (value: JsonObject) => JsonObject | undefined): Buffer | undefined {
  let value: unknown;
  try {
    value = deserialize(payload);
  } catch {
    return undefined;
  }
  const rewritten = isJsonObject(value) ? rewrite(value) : undefined;
  return rewritten === undefined ? undefined : serialize(rewritten);
}

/**
 * Tool events take the kinds `call` and `result`, in the column and in both stored copies of the payload (the payload
 * is the whole adapter event). Rule-coverage payloads name their signatures by the v15 kinds. Event ids never change:
 * adapters derive them from the native ids and deduplicate on them.
 */
function rewriteRerailEvents(database: RerailDatabase, tables: ReadonlySet<string>): void {
  if (!tables.has("events")) return;
  forEachRow<{ row: number; kind: string; payload_v8: Buffer }>(
    database,
    after => sql`
      SELECT sequence AS row, kind, payload_v8 FROM events
      WHERE kind IN ('tool-call', 'tool-result') AND payload_v8 IS NOT NULL AND sequence > ${after}
      ORDER BY sequence LIMIT ${V15_REWRITE_BATCH}
    `,
    event => {
      const payload = v15EventPayload(event.payload_v8, value => (value.kind === event.kind ? { ...value, kind: V15_EVENT_KINDS[event.kind] } : undefined));
      if (payload !== undefined) database.run(sql`UPDATE events SET payload_v8 = ${payload} WHERE sequence = ${event.row}`);
    },
  );
  database.run(sql.raw(
    "UPDATE `events` SET " +
      "`payload_json` = CASE WHEN json_valid(`payload_json`) AND json_extract(`payload_json`, '$.kind') = `kind` " +
        "THEN json_set(`payload_json`, '$.kind', CASE `kind` WHEN 'tool-call' THEN 'call' ELSE 'result' END) ELSE `payload_json` END, " +
      "`kind` = CASE `kind` WHEN 'tool-call' THEN 'call' ELSE 'result' END " +
    "WHERE `kind` IN ('tool-call', 'tool-result')",
  ));
  forEachRow<{ row: number; payload_json: string; payload_v8: Buffer | null }>(
    database,
    after => sql`
      SELECT sequence AS row, payload_json, payload_v8 FROM events
      WHERE kind = 'rule-coverage' AND sequence > ${after} AND CASE WHEN json_valid(payload_json)
        THEN json_type(payload_json, '$.cosineRuleSignature') IS NOT NULL OR json_type(payload_json, '$.policyRuleSignature') IS NOT NULL
        ELSE 0 END
      ORDER BY sequence LIMIT ${V15_REWRITE_BATCH}
    `,
    event => {
      const coverage: unknown = JSON.parse(event.payload_json);
      if (!isJsonObject(coverage)) return;
      const payloadJson = stableJsonText(renamedKeys(coverage, V15_COVERAGE_KEYS)) ?? event.payload_json;
      const payloadV8 = event.payload_v8 === null
        ? undefined
        : v15EventPayload(event.payload_v8, value => renamedKeys(value, V15_COVERAGE_KEYS));
      database.run(sql`
        UPDATE events SET payload_json = ${payloadJson}, payload_v8 = ${payloadV8 ?? event.payload_v8} WHERE sequence = ${event.row}
      `);
    },
  );
}

/**
 * v15 adopts the Rerail contract: rule documents are `rerail.policy/v1` with `clauses` and `violation`, rule kinds
 * are `file` and `conversation`, catalog origins are `user` and `rerail` with no sync state, policy runs are audits
 * whose violations name `ruleId` and `clauseId`, the run mode `resteer` is `steer`, and tool events are `call` and
 * `result`. It runs alike in the Rerail database, in a legacy `rules.sqlite` adopted into it, and in a legacy checkout
 * `transcripts.sqlite` whose rules are imported, never validates what it rewrites, and every phase skips data already
 * in the v15 shape or tables that do not exist. Recorded provenance digests and catalog revisions stay as they were
 * computed.
 */
const migrateRerailContract: SchemaUpgrade = database => {
  // Unreleased builds stamped v13 (rule modes) and v14 (this contract) before taking main's repository attribution,
  // so such a file reaches this step without step 12.
  addSessionRepository(database);
  const tables = tableNames(database);
  if (tables.has("rules")) rebuildRerailRules(database);
  rewriteRerailCatalog(database, tables);
  copyRerailAudits(database, tables);
  copyRerailAuditControls(database, tables);
  rewriteRerailEvents(database, tables);
};

const SCHEMA_UPGRADES: Readonly<Record<number, SchemaUpgrade>> = {
  1: database => {
    // A v1 file another process already widened must not fail here; SQLite has no ADD COLUMN IF NOT EXISTS.
    const columns = new Set(database.all<{ name: string }>(sql`PRAGMA table_info(rules)`).map(row => row.name));
    if (!columns.has("source_digest")) database.run(sql.raw("ALTER TABLE `rules` ADD COLUMN `source_digest` text"));
  },
  2: database => {
    // v3 compiles the enabled policy rows in memory; the persisted catalog cache is gone.
    database.run(sql.raw("DROP TABLE IF EXISTS `policy_dsl_rules`"));
    database.run(sql.raw("DROP TABLE IF EXISTS `policy_dsl_catalog`"));
  },
  3: database => {
    // v4 adds independent execution history. It intentionally has no session
    // foreign key, so a policy run can outlive/reset independently of sessions.
    database.run(sql.raw(v14TableStatement("policy_runs")));
  },
  4: migratePolicyExecutionAxes,
  5: migratePolicyExecutionAxes,
  6: migrateMergedPolicyMetadata,
  7: migrateMergedPolicyMetadata,
  8: migrateMergedPolicyMetadata,
  9: database => {
    database.run(sql.raw(v14TableStatement("presentation_preferences")));
  },
  10: database => {
    database.run(sql.raw(v14TableStatement("policy_run_result_pages")));
  },
  // Rule modes rebuild `rules` into the v14 DDL, which also carries v12's relaxed cosine CHECK,
  // so they replace that rebuild at step 11. Step 13 repeats them for files already at v13.
  11: migrateRuleModes,
  12: addSessionRepository,
  13: migrateRuleModes,
  14: migrateRerailContract,
  // v16 makes Rerail off until started; see `migrateRepositoryStarts`, which also runs outside this table.
  15: migrateRepositoryStarts,
};

/** Open the Node-owned SQLite database without applying Rerail's project schema. */
export function openSqliteDatabase(path: string, options: OpenRerailDatabaseOptions = {}): RerailDatabase {
  const readonly = options.readonly === true;
  if (!readonly) mkdirSync(dirname(path), { recursive: true, mode: 0o700 });
  const client = new Database(path, { readonly });
  const database = drizzle(client, { schema });
  database.run(sql`PRAGMA busy_timeout = 5000`);
  if (!readonly) {
    chmodSync(path, 0o600);
    enableWal(() => database.run(sql`PRAGMA journal_mode = WAL`));
    database.run(sql`PRAGMA foreign_keys = ON`);
    database.run(sql`PRAGMA synchronous = FULL`);
  }
  return database;
}

/** Move a pre-schema database (and its WAL/shm sidecars) aside so the caller can start fresh. */
function archiveLegacyDatabase(path: string): string {
  const archived = `${path}.pre-schema-${new Date().toISOString().replace(/[-:]/gu, "").replace(/\.\d+Z$/u, "Z")}`;
  renameSync(path, archived);
  for (const suffix of ["-wal", "-shm"]) {
    if (existsSync(`${path}${suffix}`)) renameSync(`${path}${suffix}`, `${archived}${suffix}`);
  }
  return archived;
}

function schemaRevisionOnDisk(
  database: RerailDatabase,
  userVersion: number,
  hasRevisionTable: boolean,
  unstampedPreFloor: boolean,
): number {
  if (!hasRevisionTable || userVersion < schema.RERAIL_SCHEMA_COMPATIBILITY_FLOOR) {
    return unstampedPreFloor ? 10 : userVersion;
  }
  return database.all<{ revision: number }>(sql.raw(`SELECT revision FROM \`${SCHEMA_REVISION_TABLE}\` WHERE singleton = 1`))[0]?.revision ?? userVersion;
}

/**
 * A `rerail.sqlite` that does not exist yet beside a legacy `rules.sqlite` starts as a
 * transactionally consistent copy of it (`VACUUM INTO`), published with a no-clobber link so
 * concurrent openers agree on one copy. The legacy file is only read and is left in place;
 * once `rerail.sqlite` exists it is never consulted again.
 */
function adoptLegacyRuleCatalog(path: string): void {
  if (basename(path) !== RERAIL_DATABASE_FILE || existsSync(path)) return;
  const legacy = join(dirname(path), LEGACY_RULE_CATALOG_FILE);
  if (!existsSync(legacy)) return;
  mkdirSync(dirname(path), { recursive: true, mode: 0o700 });
  const temporary = `${path}.${process.pid}.${randomUUID()}.adopt`;
  try {
    const source = new Database(legacy, { readonly: true, fileMustExist: true });
    try {
      source.pragma("busy_timeout = 5000");
      source.prepare("VACUUM INTO ?").run(temporary);
    } finally {
      source.close();
    }
    try {
      chmodSync(temporary, 0o600);
      linkSync(temporary, path);
      process.stderr.write(`rerail: copied the rule catalog from ${legacy} into ${path}; ${legacy} is no longer read\n`);
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== "EEXIST") throw error;
    }
  } finally {
    // A failed VACUUM INTO can leave a partial copy, or none at all.
    rmSync(temporary, { force: true });
  }
}

/**
 * Open the project database, creating the directory, file, and schema as needed.
 * Writers own the file mode (0600) and directory mode (0700) they create.
 *
 * A database written before schema v1 is renamed to `<path>.pre-schema-<timestamp>`
 * and a fresh one takes its place: a session must not fail to start because of an
 * old file, and the archive keeps the old transcripts readable. `PRAGMA user_version`
 * records the oldest compatible reader; additive revisions are recorded separately
 * and reached through `SCHEMA_UPGRADES`.
 */
export function openRerailDatabase(path: string, options: OpenRerailDatabaseOptions = {}): RerailDatabase {
  if (options.readonly !== true) adoptLegacyRuleCatalog(path);
  const database = openSqliteDatabase(path, options);
  if (options.readonly === true) return database;
  // Inspection, upgrade, DDL, and stamp run under one write lock: two processes opening a
  // v1 file at once must not both see the column missing and both try to add it.
  database.run(sql.raw("BEGIN IMMEDIATE"));
  try {
    const tables = database.all<{ name: string }>(sql`SELECT name FROM sqlite_master WHERE type = 'table'`).map(row => row.name);
    const legacy = LEGACY_TABLES.filter(name => tables.includes(name));
    if (legacy.length > 0) {
      database.run(sql.raw("ROLLBACK"));
      database.$client.close();
      const archived = archiveLegacyDatabase(path);
      process.stderr.write(
        `rerail: ${path} predates schema v${schema.RERAIL_SCHEMA_VERSION} (found ${legacy.join(", ")}); moved it to ${archived} and started a fresh database\n`,
      );
      return openRerailDatabase(path, options);
    }
    const userVersion = database.all<{ user_version: number }>(sql`PRAGMA user_version`)[0]?.user_version ?? 0;
    const hasRevisionTable = tables.includes(SCHEMA_REVISION_TABLE);
    // Before the compatibility-floor stamp, parallel lines reused version numbers: v11 added either
    // structural cosine rows or result pages, and an unreleased single-database build stamped v12 for
    // repository attribution on top of the cosine line. Replay the idempotent upgrades from 10 so any
    // of those histories converges on the current revision under the shared floor.
    const unstampedPreFloor = (userVersion === 11 || userVersion === 12) && !hasRevisionTable;
    const floorOnDisk = unstampedPreFloor ? 10 : userVersion;
    let revisionOnDisk = schemaRevisionOnDisk(database, userVersion, hasRevisionTable, unstampedPreFloor);
    // An empty file needs only the DDL; an unstamped file with tables is v1, written before the stamp existed.
    if (tables.length === 0) revisionOnDisk = schema.RERAIL_SCHEMA_VERSION;
    else if (revisionOnDisk === 0) revisionOnDisk = 1;
    if (floorOnDisk > schema.RERAIL_SCHEMA_VERSION) {
      throw new Error(`${path} requires a newer Rerail: its compatibility floor is v${floorOnDisk}, this Rerail is v${schema.RERAIL_SCHEMA_VERSION}`);
    }
    for (let step = revisionOnDisk; step < schema.RERAIL_SCHEMA_VERSION; step += 1) {
      SCHEMA_UPGRADES[step]?.(database);
    }
    // Independent of the stamped revision; see `migrateRepositoryStarts`. An empty file has nothing to carry over.
    if (tables.length > 0) migrateRepositoryStarts(database);
    for (const statement of RERAIL_SCHEMA_STATEMENTS) database.run(sql.raw(statement));
    recordSchemaRevision(database, Math.max(revisionOnDisk, schema.RERAIL_SCHEMA_VERSION));
    database.run(sql.raw(`PRAGMA user_version = ${Math.max(floorOnDisk, schema.RERAIL_SCHEMA_COMPATIBILITY_FLOOR)}`));
    database.run(sql.raw("COMMIT"));
  } catch (error) {
    try {
      database.run(sql.raw("ROLLBACK"));
    } catch {
      // Already rolled back, or the connection failed before the transaction began.
    }
    database.$client.close();
    throw error;
  }
  return database;
}

/** Opens a read-only database lazily so viewers can start before the writer creates it. */
export class LazyReadonlyRerailDatabase {
  readonly path: string;
  #database: RerailDatabase | undefined;

  constructor(path: string) {
    this.path = path;
  }

  open(): RerailDatabase | undefined {
    if (this.#database !== undefined) return this.#database;
    if (!existsSync(this.path)) return undefined;
    this.#database = openRerailDatabase(this.path, { readonly: true });
    return this.#database;
  }

  close(): void {
    this.#database?.$client.close();
    this.#database = undefined;
  }
}

export interface HistoryResetReport {
  /** Ended sessions removed together with all of their history. */
  sessions: number;
  /** Sessions that were still open (`ended_at IS NULL`): their rows and runtime ownership stay, their history goes. */
  openSessions: number;
}

/**
 * `rerail reset`: delete one repository's execution history in ONE write transaction. Events,
 * work links, audits, audit requests, and repository audits of the repository's sessions go;
 * ended sessions go with them. Still-running sessions keep their row and runtime ownership
 * claim so live processes carry on recording. Rules, other repositories' history, audit
 * settings, and presentation preferences are untouched. The file is never unlinked or
 * rebuilt, so every connection holding it open keeps working.
 */
export function resetRepositoryHistory(path: string, repositoryId: string): HistoryResetReport {
  const database = openRerailDatabase(path);
  try {
    return database.transaction(() => {
      const counts = database.all<{ ended: number; open: number }>(sql`
        SELECT count(*) FILTER (WHERE ended_at IS NOT NULL) AS ended, count(*) FILTER (WHERE ended_at IS NULL) AS open
        FROM ${schema.sessions} WHERE ${schema.sessions.repositoryId} = ${repositoryId}
      `)[0]!;
      for (const table of schema.SESSION_HISTORY_TABLES) {
        database.run(sql`DELETE FROM ${table} WHERE ${table.sessionId} IN (${schema.sessionsOfRepository(repositoryId)})`);
      }
      const ended = schema.sessionsOfRepository(repositoryId, { ended: true });
      database.run(sql`DELETE FROM ${schema.policySessionOwners} WHERE ${schema.policySessionOwners.sessionId} IN (${ended})`);
      database.run(sql`DELETE FROM ${schema.sessions} WHERE ${schema.sessions.sessionId} IN (${ended})`);
      return { sessions: counts.ended, openSessions: counts.open };
    }, { behavior: "immediate" });
  } finally {
    database.$client.close();
  }
}
