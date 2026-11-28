/**
 * The Rerail database's migration path. The Drizzle tables in `schema.ts` are the current shape: a new file
 * gets it from the generated DDL. An existing file reaches it through the steps below, run in list order the
 * next time a writer opens it.
 *
 * `PRAGMA user_version` is the file's compatibility floor: the oldest schema version a build must read to
 * open it. A build refuses a file whose floor is above its own `RERAIL_SCHEMA_VERSION`. A step raises the
 * floor only when an older build would misread the file after it. Older builds read a new nullable column,
 * table or index correctly, because Drizzle names the columns it reads and writes, so such a step sets no
 * `floor` and an older build keeps opening the file. Which steps a file has run is recorded in
 * `schema_migrations`; an id this build does not know (a newer build's step) is left alone.
 */
import { sql } from "drizzle-orm";
import type { RerailDatabase } from "./database.js";
import { schemaMigrations } from "./schema.js";

/** One change to the shape of an existing table. Ids and order never change once released. */
export interface Migration {
  /** Unique and stable: `schema_migrations` records it once the step has run. */
  readonly id: string;
  /**
   * The schema version a build must read to open the file after this step, for a change an older build
   * would misread (above `RERAIL_BASELINE_SCHEMA_VERSION`). Omitted for a change older builds read correctly.
   */
  readonly floor?: number;
  /**
   * Brings the file from the shape before this step to the shape after it, inside the open's write
   * transaction, where foreign keys are enforced and `PRAGMA foreign_keys` cannot change: a step that rebuilds a
   * table others reference keeps them valid (or sets `PRAGMA defer_foreign_keys`); `rebuildTable` is only for a
   * table nothing references. It must tolerate a table this step changes not existing yet: the generated DDL that
   * runs after the steps creates it in its current shape. `addColumn` and `rebuildTable` do both.
   */
  up(database: RerailDatabase): void;
}

/** The step that keeps each rule provenance once; `rule-provenance-move.ts` moves what audits held before it. */
export const RULE_PROVENANCE_STEP = "audit-rule-provenance-once";

/**
 * The first schema version with a migration path. Files stamped 1 come from releases before it: 0.1.0-next.1 (main
 * before the migration path) wrote the baseline's shape without `schema_migrations`, and such a file is adopted at the
 * baseline (`database.ts`); 0.1.0-next.0 wrote another shape, which is refused, as is any file stamped 2 (development
 * builds only).
 */
export const RERAIL_BASELINE_SCHEMA_VERSION = 3;

/** This build's steps, oldest first. Append only. */
export const RERAIL_MIGRATIONS: readonly Migration[] = [
  {
    // Rule settings gain `suppressions` and `config_json`, and a row may set only those: the
    // `rule_settings_value` CHECK changes, which SQLite can only do by rebuilding the table.
    // No floor, by choice: builds at the baseline keep opening the file, so agent hosts on
    // different installed versions keep sharing the one database. They read only `on` and
    // `mode`: they ignore a row's `suppressions` and `config_json` (the rule runs with its
    // default options and honours suppression comments), and when one resets that rule's or
    // pack's on/off and mode, its `writeSetting` deletes the row and both values with it.
    id: "rule-settings-suppressions-config",
    up(database) {
      rebuildTable(
        database,
        "rule_settings",
        `CREATE TABLE \`rule_settings\` (
	\`owner_kind\` text NOT NULL,
	\`owner_id\` text NOT NULL,
	\`folder\` text DEFAULT '' NOT NULL,
	\`target_kind\` text NOT NULL,
	\`target_id\` text NOT NULL,
	\`on\` integer,
	\`mode\` text,
	\`suppressions\` text,
	\`config_json\` text,
	\`updated_at\` integer NOT NULL,
	PRIMARY KEY(\`owner_kind\`, \`owner_id\`, \`folder\`, \`target_kind\`, \`target_id\`),
	CONSTRAINT "rule_settings_owner_kind" CHECK("rule_settings"."owner_kind" IN ('repository', 'checkout')),
	CONSTRAINT "rule_settings_target_kind" CHECK("rule_settings"."target_kind" IN ('rule', 'pack')),
	CONSTRAINT "rule_settings_mode" CHECK("rule_settings"."mode" IS NULL OR "rule_settings"."mode" IN ('log', 'steer', 'block')),
	CONSTRAINT "rule_settings_suppressions" CHECK("rule_settings"."suppressions" IS NULL OR "rule_settings"."suppressions" IN ('honour', 'ignore')),
	CONSTRAINT "rule_settings_value" CHECK("rule_settings"."on" IS NOT NULL OR "rule_settings"."mode" IS NOT NULL OR "rule_settings"."suppressions" IS NOT NULL OR "rule_settings"."config_json" IS NOT NULL)
)`,
      );
    },
  },
  {
    // The derived audit history the dashboard reads (`audit-history.ts`), filled from the audits a file already
    // holds. Steps run before the DDL, so the tables and the backfill are written out here, frozen as this step
    // introduced them; the triggers that keep the tables current come with the DDL right after. No floor: an older
    // build never reads these tables, and its writes to `audits` reach them through the triggers.
    id: "audit-history-derived-tables",
    up(database) {
      if (database.all(sql`SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'audits'`).length === 0) return;
      for (const statement of AUDIT_HISTORY_TABLES) database.run(sql.raw(statement));
      for (const statement of AUDIT_HISTORY_BACKFILL) database.run(sql.raw(statement));
    },
  },
  {
    // A repository's `follow_gitignore`, on by default, so files Git ignores leave its audits. No floor: an older
    // build never reads the column, and its inserts take the default.
    id: "repositories-follow-gitignore",
    up(database) {
      addColumn(database, "repositories", "follow_gitignore", "integer DEFAULT true NOT NULL");
    },
  },
  {
    // Rules created by you, and packs Rerail does not publish, run in Log only until a person approves their content
    // (`rule-approval.ts`). The library rules a file already holds were in force before approvals existed, so each is
    // recorded approved at the content it has; only new rules and later edits wait. Steps run before the DDL, so the
    // table is created here as this step introduced it. No floor: an older build never reads the table, and runs
    // every rule as before.
    id: "rule-approvals",
    up(database) {
      database.run(sql.raw(RULE_APPROVALS_TABLE));
      if (database.all(sql`SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'library_rules'`).length === 0) return;
      database.run(sql.raw(`INSERT OR IGNORE INTO rule_approvals (subject_kind, subject_id, digest, approved_at)
        SELECT 'rule', rule_id, payload_hash, updated_at FROM library_rules`));
    },
  },
  {
    // A violation that an audit in Log mode recorded is `logged` whatever its rule's mode, because that audit tells the
    // coding agent nothing. The three triggers that derive `audit_violations.logged` are dropped, and the DDL right
    // after creates them again with the audit's mode in it. The rows already derived are repaired, and so are their
    // rollup rows, whose triggers run on insert and delete but not on this update. No floor: an older build reads
    // `logged` as before, and it creates a trigger only when the trigger is missing, so it keeps these.
    id: "audit-violations-logged-by-audit-mode",
    up(database) {
      for (const trigger of ["audits_history_insert", "audits_history_violations_append", "audits_history_violations_update"])
        database.run(sql.raw(`DROP TRIGGER IF EXISTS \`${trigger}\``));
      if (database.all(sql`SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'audit_violations'`).length === 0) return;
      for (const statement of LOGGED_BY_AUDIT_MODE_REPAIR) database.run(sql.raw(statement));
    },
  },
  {
    // An audit names its rule provenance in `audit_rule_provenances`, which keeps each distinct one once, instead of
    // holding a copy in `rule_provenance_json` (221 KB for 767 rules, written again by every audited edit). Steps run
    // before the DDL, so the table is created here as this step introduced it; then the column. The audits a file
    // holds keep their provenance inline, where the audit store reads it, until the daemon moves it in short
    // transactions (`rule-provenance-move.ts`): moving it here would hold the open's write lock for the whole history
    // (about 10 s for 4,000 audits), and other writers fail once they have waited 5 s. No floor, by choice: builds at
    // the baseline keep opening the file, so agent hosts on different installed versions keep sharing it. An older
    // build writes its audits' provenance inline as before, and this build reads it there; for the audits this build
    // wrote or moved it reads `[]`, which only its audit API and remote sink show, as its dashboard reads no
    // provenance. The step records the last audit the file holds (`audit_rule_provenance_step`): the move takes the
    // audits up to that one, in the order they were recorded, whatever the clock read when each began. An older build
    // never reads that table.
    id: RULE_PROVENANCE_STEP,
    up(database) {
      if (database.all(sql`SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'audits'`).length === 0) return;
      for (const statement of AUDIT_RULE_PROVENANCES_TABLE) database.run(sql.raw(statement));
      addColumn(database, "audits", "rule_provenance_id", "integer");
      database.run(sql.raw("INSERT INTO audit_rule_provenance_step (last_audit_id) SELECT audit_id FROM audits ORDER BY rowid DESC LIMIT 1"));
    },
  },
  {
    // A repository's `merge_assist`, off by default: only a person turns Merge Assist on. No floor: an older build
    // never reads the column, and its inserts take the default. Its tables need no step; the DDL creates them.
    id: "repositories-merge-assist",
    up(database) {
      addColumn(database, "repositories", "merge_assist", "integer DEFAULT false NOT NULL");
    },
  },
  {
    // A rule setting's `origin`: `lint-config` for one the repository's lint config gave (set-up, `rerail packs
    // calibrate`), null for a person's or an agent's, which calibrating leaves alone. No floor: an older build never
    // reads the column. Its writes name only the columns it knows, so a row it inserts has no origin, and its upsert
    // of an existing row keeps the row's origin: a calibrated row an older build's dashboard changed still reads as
    // calibrated, and the next calibration may replace that change. Its delete takes the origin with the row.
    id: "rule-settings-origin",
    up(database) {
      addColumn(database, "rule_settings", "origin", "text");
    },
  },
  {
    // An audit's `origin`: `ongoing`, or `scan` for one someone asked for, whose violations the dashboard leaves out.
    // The four triggers that derive history from `audits` are dropped, and the DDL right after creates them again
    // skipping scans. The scans a file already holds are marked where it still names them (`markRecordedScans`). No
    // floor: an older build never reads the column, its inserts take the default, and it creates a trigger only when
    // the trigger is missing, so it keeps these.
    id: "audits-origin",
    up(database) {
      for (const trigger of ["audits_history_insert", "audits_history_violations_append", "audits_history_violations_update", "audits_history_rule_set_update"])
        database.run(sql.raw(`DROP TRIGGER IF EXISTS \`${trigger}\``));
      addColumn(database, "audits", "origin", `text DEFAULT 'ongoing' NOT NULL CONSTRAINT "audits_origin" CHECK("audits"."origin" IN ('ongoing', 'scan'))`);
      markRecordedScans(database);
    },
  },
  {
    // A rule's `title`, of either kind, which the dashboard shows. No floor: an older build never reads the column; its
    // inserts leave it null, and its updates of a row keep the title the row had.
    id: "rules-title",
    up(database) {
      addColumn(database, "rules", "title", "text");
    },
  },
  {
    // An audit may belong to the repository instead of a session: a change the walk of the other worktrees found that
    // no session made is recorded once, under the repository, with no `session_id` (`audits.repository_id`,
    // `audits.recorded_by`). `session_id` loses its NOT NULL in `audits` and the derived tables, and the derived tables
    // gain the owner's `repository_id`. The history triggers are dropped first, so no rename rewrites them, and the DDL
    // right after creates them again matching either owner. `audits` is not rebuilt: its rows are large (1.5 GB for
    // 6,000 audits), and copying them and building its indexes again held the open's write lock for 18 s, where other
    // writers give up after 5 s. Its new columns are added in place, and its NOT NULL is dropped in place
    // (`dropNotNull`). The derived tables' rows are small, so they are rebuilt. No floor: an older build writes
    // audits with a session as before, which the new triggers derive as before; its repository-scoped reads match
    // `session_id IN (...)` and so leave the repository's audits out, and it creates a trigger only when the trigger
    // is missing, so it keeps these. Its unscoped reads return them with no session, and its dashboard reads unscoped
    // while it knows no repository: there its History by rule and rule panel fail on such a violation (they read every
    // violation's session id). A floor would lock that build out of the file instead, its hooks included.
    id: "audits-repository-owner",
    up(database) {
      for (const trigger of AUDIT_HISTORY_TRIGGERS) database.run(sql.raw(`DROP TRIGGER IF EXISTS \`${trigger}\``));
      addColumn(database, "audits", "repository_id", "text");
      addColumn(database, "audits", "recorded_by", "text");
      dropNotNull(database, "audits", "`session_id` text NOT NULL");
      for (const [table, ddl] of OWNED_HISTORY_TABLES) rebuildTable(database, table, ddl);
    },
  },
  {
    id: "analysis-scope-release-retry",
    up(database) {
      addColumn(database, "analysis_scope_releases", "next_attempt_at", "integer DEFAULT 0 NOT NULL");
    },
  },
  {
    id: "analysis-scope-probe-failures",
    up(database) {
      addColumn(database, "analysis_scope_release_probes", "attempts", "integer DEFAULT 0 NOT NULL");
      addColumn(database, "analysis_scope_release_probes", "last_error", "text");
    },
  },
];

/**
 * `audits-origin`: marks the scans a file recorded before audits had an origin, and removes what was derived from them
 * (`SCAN_HISTORY_REPAIR`). Those are the audits its scan requests name (`audit_ids_json`) and the one each checkout's
 * `repository_audits` row names now. That row keeps one audit id: a scan request finished later, or a retry of an
 * interrupted or failed first repository audit, replaced it, and the first audit it named stays `ongoing`. Nothing
 * else tells such an audit apart: an ongoing audit of the repository scope looks the same.
 */
function markRecordedScans(database: RerailDatabase): void {
  const tables = new Set(database.all<{ name: string }>(sql`SELECT name FROM sqlite_master WHERE type = 'table'`).map(row => row.name));
  if (!tables.has("audits")) return;
  if (tables.has("audit_requests")) database.run(sql.raw(MARK_REQUESTED_SCANS));
  if (tables.has("repository_audits")) database.run(sql.raw(MARK_REPOSITORY_AUDIT_SCANS));
  if (tables.has("audit_violations")) for (const statement of SCAN_HISTORY_REPAIR) database.run(sql.raw(statement));
}

/** `audits-origin`: the audits `rerail scan --session` (an audit request) recorded are scans. */
const MARK_REQUESTED_SCANS = `UPDATE audits SET origin = 'scan' WHERE audit_id IN (
    SELECT requested.value FROM audit_requests request, json_each(request.audit_ids_json) requested)`;

/** `audits-origin`: the first repository audit each checkout's row still names is a scan. */
const MARK_REPOSITORY_AUDIT_SCANS = `UPDATE audits SET origin = 'scan'
    WHERE audit_id IN (SELECT audit_id FROM repository_audits WHERE audit_id IS NOT NULL)`;

/**
 * `audits-origin`'s repair: the derived rows of the audits it marked scans go, and the rollup rows of their violations
 * are recomputed from the sightings left (or dropped). It recomputes them itself: the rollup triggers are missing when
 * this open's `audit-history-derived-tables` filled the tables.
 */
const SCAN_HISTORY_REPAIR: readonly string[] = [
  `CREATE TEMP TABLE scan_violation_keys AS
    SELECT DISTINCT violation.session_id, violation.rule_id, violation.violation_key
    FROM audits audit JOIN audit_violations violation ON violation.audit_id = audit.audit_id
    WHERE audit.origin = 'scan' AND violation.rule_id IS NOT NULL`,
  "DELETE FROM audit_violations WHERE audit_id IN (SELECT audit_id FROM audits WHERE origin = 'scan')",
  "DELETE FROM audit_rule_set_uses WHERE audit_id IN (SELECT audit_id FROM audits WHERE origin = 'scan')",
  `DELETE FROM rule_violation_rollup
    WHERE (session_id, rule_id, violation_key) IN (SELECT session_id, rule_id, violation_key FROM temp.scan_violation_keys)`,
  `INSERT INTO rule_violation_rollup (session_id, rule_id, violation_key, last_seen_at, last_logged_at)
    SELECT kept.session_id, kept.rule_id, kept.violation_key, max(kept.started_at), max(CASE WHEN kept.logged = 1 THEN kept.started_at END)
    FROM temp.scan_violation_keys scanned
    JOIN audit_violations kept
      ON kept.session_id = scanned.session_id AND kept.rule_id = scanned.rule_id AND kept.violation_key = scanned.violation_key
    GROUP BY kept.session_id, kept.rule_id, kept.violation_key`,
  "DROP TABLE temp.scan_violation_keys",
];

/**
 * `audit_rule_provenances` and its digest index, and `audit_rule_provenance_step`, as `audit-rule-provenance-once`
 * introduced them.
 */
const AUDIT_RULE_PROVENANCES_TABLE: readonly string[] = [
  "CREATE TABLE IF NOT EXISTS `audit_rule_provenances` (\n\t`provenance_id` integer PRIMARY KEY NOT NULL,\n\t`digest` text NOT NULL,\n\t`rule_provenance_json` text NOT NULL,\n\tCONSTRAINT \"audit_rule_provenances_json\" CHECK(json_type(\"audit_rule_provenances\".\"rule_provenance_json\") = 'array')\n)",
  "CREATE UNIQUE INDEX IF NOT EXISTS `audit_rule_provenances_digest_unique` ON `audit_rule_provenances` (`digest`)",
  "CREATE TABLE IF NOT EXISTS `audit_rule_provenance_step` (\n\t`last_audit_id` text PRIMARY KEY NOT NULL\n)",
];

/**
 * `audit-violations-logged-by-audit-mode`'s repair: every row of an audit in Log mode becomes `logged`, and each
 * rollup row of those violations takes the latest of them as its latest logged sighting if it is later.
 */
const LOGGED_BY_AUDIT_MODE_REPAIR: readonly string[] = [
  `UPDATE audit_violations SET logged = 1
    WHERE logged = 0 AND audit_id IN (SELECT audit_id FROM audits WHERE mode = 'log')`,
  `UPDATE rule_violation_rollup SET last_logged_at = max(coalesce(last_logged_at, latest.started_at), latest.started_at)
    FROM (
      SELECT violation.session_id, violation.rule_id, violation.violation_key, max(violation.started_at) AS started_at
      FROM audits audit JOIN audit_violations violation ON violation.audit_id = audit.audit_id
      WHERE audit.mode = 'log' AND violation.rule_id IS NOT NULL
      GROUP BY violation.session_id, violation.rule_id, violation.violation_key
    ) latest
    WHERE rule_violation_rollup.session_id = latest.session_id AND rule_violation_rollup.rule_id = latest.rule_id
      AND rule_violation_rollup.violation_key = latest.violation_key`,
];

/** `rule_approvals` as `rule-approvals` introduced it. */
const RULE_APPROVALS_TABLE = "CREATE TABLE IF NOT EXISTS `rule_approvals` (\n\t`subject_kind` text NOT NULL,\n\t`subject_id` text NOT NULL,\n\t`digest` text NOT NULL,\n\t`approved_at` integer NOT NULL,\n\tPRIMARY KEY(`subject_kind`, `subject_id`, `digest`),\n\tCONSTRAINT \"rule_approvals_subject_kind\" CHECK(\"rule_approvals\".\"subject_kind\" IN ('rule', 'pack'))\n)";

/** The derived audit history tables as `audit-history-derived-tables` introduced them. */
const AUDIT_HISTORY_TABLES: readonly string[] = [
  "CREATE TABLE IF NOT EXISTS `audit_violations` (\n\t`audit_id` text NOT NULL,\n\t`ordinal` integer NOT NULL,\n\t`session_id` text NOT NULL,\n\t`started_at` integer NOT NULL,\n\t`rule_id` text,\n\t`violation_key` text NOT NULL,\n\t`logged` integer NOT NULL,\n\tPRIMARY KEY(`audit_id`, `ordinal`)\n)",
  "CREATE INDEX IF NOT EXISTS `audit_violations_session_started` ON `audit_violations` (`session_id`,`started_at`)",
  "CREATE INDEX IF NOT EXISTS `audit_violations_started` ON `audit_violations` (`started_at`)",
  "CREATE INDEX IF NOT EXISTS `audit_violations_rule_started` ON `audit_violations` (`rule_id`,`started_at`)",
  "CREATE INDEX IF NOT EXISTS `audit_violations_key` ON `audit_violations` (`session_id`,`rule_id`,`violation_key`,`started_at`)",
  "CREATE INDEX IF NOT EXISTS `audit_violations_key_logged` ON `audit_violations` (`session_id`,`rule_id`,`violation_key`,`started_at`) WHERE \"audit_violations\".\"logged\" = 1",
  "CREATE TABLE IF NOT EXISTS `rule_violation_rollup` (\n\t`session_id` text NOT NULL,\n\t`rule_id` text NOT NULL,\n\t`violation_key` text NOT NULL,\n\t`last_seen_at` integer NOT NULL,\n\t`last_logged_at` integer,\n\tPRIMARY KEY(`session_id`, `rule_id`, `violation_key`)\n)",
  "CREATE INDEX IF NOT EXISTS `rule_violation_rollup_last_seen` ON `rule_violation_rollup` (`last_seen_at`)",
  "CREATE TABLE IF NOT EXISTS `audit_rule_sets` (\n\t`rule_set_id` integer PRIMARY KEY NOT NULL,\n\t`rule_revisions_json` text NOT NULL\n)",
  "CREATE UNIQUE INDEX IF NOT EXISTS `audit_rule_sets_rule_revisions_json_unique` ON `audit_rule_sets` (`rule_revisions_json`)",
  "CREATE TABLE IF NOT EXISTS `audit_rule_set_rules` (\n\t`rule_set_id` integer NOT NULL,\n\t`rule_id` text NOT NULL,\n\tPRIMARY KEY(`rule_set_id`, `rule_id`)\n)",
  "CREATE TABLE IF NOT EXISTS `audit_rule_set_uses` (\n\t`audit_id` text PRIMARY KEY NOT NULL,\n\t`session_id` text NOT NULL,\n\t`started_at` integer NOT NULL,\n\t`rule_set_id` integer NOT NULL\n)",
  "CREATE INDEX IF NOT EXISTS `audit_rule_set_uses_session_started` ON `audit_rule_set_uses` (`session_id`,`started_at`,`rule_set_id`)",
  "CREATE INDEX IF NOT EXISTS `audit_rule_set_uses_started` ON `audit_rule_set_uses` (`started_at`,`rule_set_id`)",
  "CREATE TABLE IF NOT EXISTS `audit_violations_staging` (\n\t`audit_id` text NOT NULL,\n\t`ordinal` integer NOT NULL,\n\t`session_id` text NOT NULL,\n\t`started_at` integer NOT NULL,\n\t`rule_id` text,\n\t`violation_key` text NOT NULL,\n\t`logged` integer NOT NULL,\n\tPRIMARY KEY(`audit_id`, `ordinal`)\n)",
];

/**
 * The audits a file holds, replayed into those tables as the triggers would have built them: one row per distinct
 * (rule, violation key) in each audit at its first copy, `logged` when any copy was in Log mode; each key's latest
 * sighting; each audit's rule set. The violation key is `audit-history.ts`'s `violationKeySql`.
 */
const AUDIT_HISTORY_BACKFILL: readonly string[] = [
  `INSERT INTO audit_violations (audit_id, ordinal, session_id, started_at, rule_id, violation_key, logged)
    SELECT entry.audit_id, entry.ordinal, entry.session_id, entry.started_at, entry.rule_id, entry.violation_key, entry.logged
    FROM (
      SELECT
        audit.audit_id AS audit_id,
        audit.session_id AS session_id,
        audit.started_at AS started_at,
        violation.key AS ordinal,
        json_extract(violation.value, '$.ruleId') AS rule_id,
        coalesce(json_extract(violation.value, '$.clauseId'), '') || char(31) || coalesce(json_extract(violation.value, '$.path'), '')
          || char(31) || coalesce(json_extract(violation.value, '$.span'), '') || char(31) || coalesce(json_extract(violation.value, '$.code'), '')
          AS violation_key,
        row_number() OVER copies AS copy,
        max(coalesce(json_extract(violation.value, '$.mode') = 'log', 0)) OVER copies AS logged
      FROM audits audit, json_each(audit.violations_json) violation
      WINDOW copies AS (PARTITION BY audit.audit_id, json_extract(violation.value, '$.ruleId'),
        coalesce(json_extract(violation.value, '$.clauseId'), '') || char(31) || coalesce(json_extract(violation.value, '$.path'), '')
          || char(31) || coalesce(json_extract(violation.value, '$.span'), '') || char(31) || coalesce(json_extract(violation.value, '$.code'), '')
        ORDER BY violation.key ROWS BETWEEN UNBOUNDED PRECEDING AND UNBOUNDED FOLLOWING)
    ) entry
    WHERE entry.copy = 1`,
  `INSERT INTO rule_violation_rollup (session_id, rule_id, violation_key, last_seen_at, last_logged_at)
    SELECT session_id, rule_id, violation_key, max(started_at), max(CASE WHEN logged = 1 THEN started_at END)
    FROM audit_violations WHERE rule_id IS NOT NULL
    GROUP BY session_id, rule_id, violation_key`,
  "INSERT INTO audit_rule_sets (rule_revisions_json) SELECT DISTINCT rule_revisions_json FROM audits",
  `INSERT INTO audit_rule_set_rules (rule_set_id, rule_id)
    SELECT DISTINCT rule_set.rule_set_id, json_extract(revision.value, '$.ruleId')
    FROM audit_rule_sets rule_set, json_each(rule_set.rule_revisions_json) revision
    WHERE json_extract(revision.value, '$.ruleId') IS NOT NULL`,
  `INSERT INTO audit_rule_set_uses (audit_id, session_id, started_at, rule_set_id)
    SELECT audit.audit_id, audit.session_id, audit.started_at, rule_set.rule_set_id
    FROM audits audit JOIN audit_rule_sets rule_set ON rule_set.rule_revisions_json = audit.rule_revisions_json`,
];

/** `audits-repository-owner`: every trigger that names `audits` or a derived history table, as `audit-history.ts` creates them. */
const AUDIT_HISTORY_TRIGGERS = [
  "audit_violations_rollup_insert", "audit_violations_rollup_delete", "audits_history_insert", "audits_history_violations_append",
  "audits_history_violations_update", "audits_history_rule_set_update", "audits_history_delete", "audits_rule_provenance_delete",
] as const;

/** The derived audit history tables as `audits-repository-owner` introduced them. */
const OWNED_HISTORY_TABLES: ReadonlyArray<readonly [string, string]> = [
  ["audit_violations", "CREATE TABLE IF NOT EXISTS `audit_violations` (\n\t`audit_id` text NOT NULL,\n\t`ordinal` integer NOT NULL,\n\t`session_id` text,\n\t`started_at` integer NOT NULL,\n\t`rule_id` text,\n\t`violation_key` text NOT NULL,\n\t`logged` integer NOT NULL,\n\t`repository_id` text,\n\tPRIMARY KEY(`audit_id`, `ordinal`)\n)"],
  ["audit_violations_staging", "CREATE TABLE IF NOT EXISTS `audit_violations_staging` (\n\t`audit_id` text NOT NULL,\n\t`ordinal` integer NOT NULL,\n\t`session_id` text,\n\t`started_at` integer NOT NULL,\n\t`rule_id` text,\n\t`violation_key` text NOT NULL,\n\t`logged` integer NOT NULL,\n\t`repository_id` text,\n\tPRIMARY KEY(`audit_id`, `ordinal`)\n)"],
  ["rule_violation_rollup", "CREATE TABLE IF NOT EXISTS `rule_violation_rollup` (\n\t`session_id` text,\n\t`rule_id` text NOT NULL,\n\t`violation_key` text NOT NULL,\n\t`last_seen_at` integer NOT NULL,\n\t`last_logged_at` integer,\n\t`repository_id` text\n)"],
  ["audit_rule_set_uses", "CREATE TABLE IF NOT EXISTS `audit_rule_set_uses` (\n\t`audit_id` text PRIMARY KEY NOT NULL,\n\t`session_id` text,\n\t`started_at` integer NOT NULL,\n\t`rule_set_id` integer NOT NULL,\n\t`repository_id` text\n)"],
];

/** The schema version `migrations` leave a file at: the baseline, or the highest floor a step raises it to. */
export function schemaVersionOf(migrations: readonly Migration[]): number {
  return Math.max(RERAIL_BASELINE_SCHEMA_VERSION, ...migrations.map(migration => migration.floor ?? RERAIL_BASELINE_SCHEMA_VERSION));
}

/** The schema version this build writes and the newest one it reads. */
export const RERAIL_SCHEMA_VERSION = schemaVersionOf(RERAIL_MIGRATIONS);

const IDENTIFIER = /^[a-z_][a-z0-9_]*$/u;

function identifier(name: string): string {
  if (!IDENTIFIER.test(name)) throw new Error(`Not a schema identifier: ${name}`);
  return `\`${name}\``;
}

/**
 * Adds `column` to `table` with `definition` (`text`, `integer DEFAULT 0 NOT NULL`, ...), unless the table does
 * not exist yet or already has the column. SQLite adds a column in place without copying the table; a
 * `NOT NULL` column needs a `DEFAULT`.
 */
export function addColumn(database: RerailDatabase, table: string, column: string, definition: string): void {
  const columns = database.all<{ name: string }>(sql`SELECT name FROM pragma_table_info(${table})`);
  if (columns.length === 0 || columns.some(existing => existing.name === column)) return;
  database.run(sql.raw(`ALTER TABLE ${identifier(table)} ADD COLUMN ${identifier(column)} ${definition}`));
}

/** The steps of `migrations` an existing file has not recorded, in list order; every step for a file without the record. */
export function pendingMigrations(database: RerailDatabase, migrations: readonly Migration[]): readonly Migration[] {
  const recordsMigrations = database.all(sql`SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'schema_migrations'`).length > 0;
  const applied = new Set(
    recordsMigrations ? database.select({ id: schemaMigrations.id }).from(schemaMigrations).all().map(row => row.id) : [],
  );
  return migrations.filter(migration => !applied.has(migration.id));
}

/**
 * Replaces `table` with the one `ddl` creates (a `CREATE TABLE` under the same name, written out in the step so
 * it never follows later schema changes) and copies every row, column by column, unless the table does not exist
 * yet. A column the old table lacks takes its default. For a table with no indexes or triggers of its own and no
 * foreign keys pointing at it: those would not survive the swap.
 */
export function rebuildTable(database: RerailDatabase, table: string, ddl: string): void {
  const columns = database.all<{ name: string }>(sql`SELECT name FROM pragma_table_info(${table})`);
  if (columns.length === 0) return;
  const previous = `${table}_before_rebuild`;
  database.run(sql.raw(`ALTER TABLE ${identifier(table)} RENAME TO ${identifier(previous)}`));
  database.run(sql.raw(ddl));
  const copied = columns.map(column => identifier(column.name)).join(", ");
  database.run(sql.raw(`INSERT INTO ${identifier(table)} (${copied}) SELECT ${copied} FROM ${identifier(previous)}`));
  database.run(sql.raw(`DROP TABLE ${identifier(previous)}`));
}

/**
 * Drops a column's NOT NULL from `table` in place, unless the table does not exist yet or its definition no longer
 * holds `column` (the column as its definition reads, `` `name` type NOT NULL ``): SQLite's documented procedure for a
 * change that leaves every row and index valid (https://www.sqlite.org/lang_altertable.html#otheralter), which rewrites
 * neither. Only the stored definition changes, so the columns stay in the order the file has them, whichever steps
 * added them. Defensive mode refuses an edit of `sqlite_schema`, so it is lifted for the edit alone; the schema
 * version moves on, so every other connection to the file reads the new definition.
 */
function dropNotNull(database: RerailDatabase, table: string, column: string): void {
  const client = database.$client;
  const stored = client.prepare("SELECT sql FROM sqlite_schema WHERE type = 'table' AND name = ?").pluck().get(table) as string | undefined;
  if (stored === undefined || !stored.includes(column)) return;
  if (stored.indexOf(column) !== stored.lastIndexOf(column)) throw new Error(`${table} defines ${column} more than once`);
  const version = client.pragma("schema_version", { simple: true }) as number;
  client.unsafeMode(true);
  try {
    client.pragma("writable_schema = ON");
    client.prepare("UPDATE sqlite_schema SET sql = ? WHERE type = 'table' AND name = ?").run(stored.replace(column, column.replace(/ NOT NULL$/u, "")), table);
    client.pragma(`schema_version = ${version + 1}`);
  } finally {
    client.pragma("writable_schema = RESET");
    client.unsafeMode(false);
  }
}

/**
 * Brings a file a writer holds under `BEGIN IMMEDIATE` to the current shape and returns the floor to stamp.
 * A new file (`fresh`) gets the DDL and every step recorded as run without running it. An existing file
 * runs the steps it has not recorded, in order, then the DDL, which only creates what is missing.
 */
export function migrate(
  database: RerailDatabase,
  { fresh, floor }: { fresh: boolean; floor: number },
  statements: readonly string[],
  migrations: readonly Migration[],
): number {
  const pending = fresh ? [] : pendingMigrations(database, migrations);
  for (const migration of pending) migration.up(database);
  for (const statement of statements) database.run(sql.raw(statement));
  const recorded = fresh ? migrations : pending;
  const appliedAt = Date.now();
  for (const migration of recorded) database.insert(schemaMigrations).values({ id: migration.id, appliedAt }).run();
  return fresh ? schemaVersionOf(migrations) : Math.max(floor, schemaVersionOf(pending));
}
