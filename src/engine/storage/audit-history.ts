/**
 * Keeps the derived audit history tables (`audit_violations`, `rule_violation_rollup`, `audit_rule_sets`,
 * `audit_rule_set_rules`, `audit_rule_set_uses`, and the trigger-only scratch table `audit_violations_staging`; see
 * `schema.ts`) in step with `audits`, and deletes a stored rule provenance (`audit_rule_provenances`) no audit names
 * any more. They are SQLite triggers rather than code in the audit writer because every Rerail that shares the file
 * writes `audits`, an older one included, and only a trigger sees all of them.
 *
 * The triggers are part of the DDL applied on every open (`schema-sql.generated.ts`), created `IF NOT EXISTS`: a
 * build that changes one must drop it in a migration step so the new body is created. The statements avoid conflict
 * clauses (`OR IGNORE`, upserts), which an outer `INSERT OR REPLACE` on `audits` would override. Such a REPLACE
 * deletes the old row without running `audits_history_delete` (SQLite runs no delete trigger for it while
 * `recursive_triggers` is off, its default), so the insert trigger clears what an audit of that id left first. The
 * backfill that fills the tables for audits written before them is frozen in its migration step (`migrations.ts`).
 *
 * Only ongoing audits (`AuditOrigin`) reach the tables: a scan's violations are the checkout's, found when someone
 * asked, not the session's, so the dashboard counts neither them nor the scan as an audit of its rules. The scan
 * keeps its result in `audits`, where `rerail scan` reads it.
 *
 * Each derived row carries its audit's owner: `session_id`, or with none the repository `repository_id` names
 * (`schema.ts`). A row of one kind is matched by its own key with `=`, in a statement of its own (`ownerKeys`), so each
 * lookup takes its index: an `IS` or an `OR` over both kinds would read the whole table.
 */

/**
 * The identity a violation keeps across audits within its rule: `violations_json` is each audit's current state,
 * so an unfixed violation is written again by every later audit and must not count more than once. `json_extract`
 * renders `span` as its JSON text, the serialized form the key needs.
 */
export function violationKeySql(value: string): string {
  return `coalesce(json_extract(${value}, '$.clauseId'), '') || char(31)`
    + ` || coalesce(json_extract(${value}, '$.path'), '') || char(31)`
    + ` || coalesce(json_extract(${value}, '$.span'), '') || char(31)`
    + ` || coalesce(json_extract(${value}, '$.code'), '')`;
}

/**
 * Each of `audit`'s violations (`audit` names its row in `from`, which reads `json_each(<audit>.violations_json)` as
 * `violation`) where `where` holds: its `ordinal`, `rule_id` and `violation_key`, which `copy` of that (rule, key) in
 * the audit it is, and `logged`. A violation is logged, the coding agent not told, when the audit ran in Log mode, where
 * it tells the agent nothing whatever its rules' modes, or when any copy's rule was in Log mode. The audit's `mode`
 * never changes after its insert. The history triggers keep the first copy of each; the remote sink sends a scan's
 * violations by the same rows.
 */
export function violationCopiesSql(audit: string, from: string, where: string): string {
  const key = violationKeySql("violation.value");
  return `SELECT
        violation.key AS ordinal,
        json_extract(violation.value, '$.ruleId') AS rule_id,
        ${key} AS violation_key,
        row_number() OVER copies AS copy,
        ${audit}.mode = 'log' OR max(coalesce(json_extract(violation.value, '$.mode') = 'log', 0)) OVER copies AS logged
      FROM ${from}
      WHERE ${where}
      -- The whole partition: with ORDER BY alone the frame would end at the current row, and the first copy, the one
      -- kept, would see only its own rule's mode.
      WINDOW copies AS (PARTITION BY json_extract(violation.value, '$.ruleId'), ${key}
        ORDER BY violation.key ROWS BETWEEN UNBOUNDED PRECEDING AND UNBOUNDED FOLLOWING)`;
}

/** Whether `audit` is one the derived tables hold: an ongoing audit. Its `origin` never changes after its insert. */
const ONGOING = (audit: string): string => `${audit}.origin = 'ongoing'`;

/**
 * The `audit_violations` rows of the audit a trigger holds as `NEW`, from its entries at ordinal `from` on: repeats
 * of a (rule, key) among them collapse onto the first copy (`violationCopiesSql`). None for a scan (`ONGOING`).
 */
const violationRows = (from: string): string => `SELECT NEW.audit_id, entry.ordinal, NEW.session_id, NEW.started_at, entry.rule_id, entry.violation_key, entry.logged,
    NEW.repository_id
    FROM (
      ${violationCopiesSql("NEW", "json_each(NEW.violations_json) violation", `violation.key >= ${from} AND ${ONGOING("NEW")}`)}
    ) entry
    WHERE entry.copy = 1`;

const VIOLATION_COLUMNS = "(audit_id, ordinal, session_id, started_at, rule_id, violation_key, logged, repository_id)";
const STAGED_COLUMNS = "staged.audit_id, staged.ordinal, staged.session_id, staged.started_at, staged.rule_id, staged.violation_key, staged.logged, staged.repository_id";

const INSERT_VIOLATIONS = `INSERT INTO audit_violations ${VIOLATION_COLUMNS} ${violationRows("0")}`;

/**
 * Whether an update only appended to the audit's violations: same audit, session and start, and `OLD.violations_json`
 * is a prefix of the new array's text. JSON values delimit themselves, so the new array then begins with the old
 * entries, unchanged; a writer that formats the text another way just takes `REPLACE_VIOLATIONS`.
 */
const APPENDED = `OLD.audit_id = NEW.audit_id AND OLD.session_id IS NEW.session_id AND OLD.started_at IS NEW.started_at
  AND length(NEW.violations_json) > length(OLD.violations_json)
  AND (OLD.violations_json = '[]'
    OR substr(NEW.violations_json, 1, length(OLD.violations_json)) = substr(OLD.violations_json, 1, length(OLD.violations_json) - 1) || ',')`;

/** How many entries the audit had before the append; a constant subquery, so SQLite evaluates it once per statement. */
const OLD_LENGTH = "(SELECT json_array_length(OLD.violations_json))";

/**
 * An append (`APPENDED`), as a running scan's cumulative `progress` writes are: only the new entries are read. A
 * (rule, key) new to the audit gets its row; one the audit already had keeps its first copy, and becomes `logged`
 * when a new copy was in Log mode (its row is replaced, so the rollup sees the change).
 */
const APPEND_VIOLATIONS = `
  INSERT INTO audit_violations_staging ${VIOLATION_COLUMNS} ${violationRows(OLD_LENGTH)};
  INSERT INTO audit_violations_staging ${VIOLATION_COLUMNS}
    SELECT kept.audit_id, kept.ordinal, kept.session_id, kept.started_at, kept.rule_id, kept.violation_key, 1, kept.repository_id
    FROM audit_violations_staging staged
    JOIN audit_violations kept INDEXED BY audit_violations_audit_key
      ON kept.audit_id = staged.audit_id AND kept.rule_id IS staged.rule_id AND kept.violation_key = staged.violation_key
    WHERE staged.audit_id = NEW.audit_id AND staged.logged = 1 AND kept.logged = 0;
  DELETE FROM audit_violations WHERE audit_id = NEW.audit_id AND ordinal IN (
    SELECT ordinal FROM audit_violations_staging WHERE audit_id = NEW.audit_id AND ordinal < ${OLD_LENGTH});
  INSERT INTO audit_violations ${VIOLATION_COLUMNS}
    SELECT ${STAGED_COLUMNS} FROM audit_violations_staging staged
    WHERE staged.audit_id = NEW.audit_id AND staged.ordinal < ${OLD_LENGTH};
  INSERT INTO audit_violations ${VIOLATION_COLUMNS}
    SELECT ${STAGED_COLUMNS} FROM audit_violations_staging staged
    WHERE staged.audit_id = NEW.audit_id AND staged.ordinal >= ${OLD_LENGTH} AND NOT EXISTS (
      SELECT 1 FROM audit_violations kept INDEXED BY audit_violations_audit_key
      WHERE kept.audit_id = NEW.audit_id AND kept.rule_id IS staged.rule_id AND kept.violation_key = staged.violation_key);
  DELETE FROM audit_violations_staging WHERE audit_id = NEW.audit_id;`;

/**
 * Any other change to an existing audit's violations: its rows are computed into `audit_violations_staging`, then
 * only the rows that differ are deleted and inserted, so the rollup triggers run for the changes alone.
 */
const REPLACE_VIOLATIONS = `
  INSERT INTO audit_violations_staging ${VIOLATION_COLUMNS} ${violationRows("0")};
  DELETE FROM audit_violations WHERE audit_id = OLD.audit_id AND NOT EXISTS (
    SELECT 1 FROM audit_violations_staging staged
    WHERE staged.audit_id = NEW.audit_id AND staged.audit_id = audit_violations.audit_id AND staged.ordinal = audit_violations.ordinal
      AND staged.session_id IS audit_violations.session_id AND staged.repository_id IS audit_violations.repository_id
      AND staged.started_at = audit_violations.started_at
      AND staged.rule_id IS audit_violations.rule_id AND staged.violation_key = audit_violations.violation_key
      AND staged.logged = audit_violations.logged);
  INSERT INTO audit_violations ${VIOLATION_COLUMNS}
    SELECT ${STAGED_COLUMNS} FROM audit_violations_staging staged
    WHERE staged.audit_id = NEW.audit_id
      AND NOT EXISTS (SELECT 1 FROM audit_violations kept WHERE kept.audit_id = staged.audit_id AND kept.ordinal = staged.ordinal);
  DELETE FROM audit_violations_staging WHERE audit_id = NEW.audit_id;`;

/**
 * Creates the rule set of `audit`'s `rule_revisions_json` and its members unless it exists, then records the audit's
 * use of it; nothing for a scan (`ONGOING`).
 */
const USE_RULE_SET = (audit: string): string => `
  INSERT INTO audit_rule_sets (rule_revisions_json)
    SELECT ${audit}.rule_revisions_json WHERE ${ONGOING(audit)}
      AND NOT EXISTS (SELECT 1 FROM audit_rule_sets WHERE rule_revisions_json = ${audit}.rule_revisions_json);
  INSERT INTO audit_rule_set_rules (rule_set_id, rule_id)
    SELECT DISTINCT rule_set.rule_set_id, json_extract(revision.value, '$.ruleId')
    FROM audit_rule_sets rule_set, json_each(rule_set.rule_revisions_json) revision
    WHERE rule_set.rule_revisions_json = ${audit}.rule_revisions_json
      AND json_extract(revision.value, '$.ruleId') IS NOT NULL
      AND NOT EXISTS (SELECT 1 FROM audit_rule_set_rules member WHERE member.rule_set_id = rule_set.rule_set_id);
  INSERT INTO audit_rule_set_uses (audit_id, session_id, started_at, rule_set_id, repository_id)
    SELECT ${audit}.audit_id, ${audit}.session_id, ${audit}.started_at, rule_set_id, ${audit}.repository_id
    FROM audit_rule_sets WHERE rule_revisions_json = ${audit}.rule_revisions_json AND ${ONGOING(audit)};`;

/**
 * The rows with `row`'s owner, rule and violation key, as two conditions: a session's rows, and the repository's (no
 * session). The one that is not `row`'s kind holds for no row, and costs nothing, as its first term is a constant.
 */
const ownerKeys = (row: string): readonly [string, string] => {
  const ruleKey = `rule_id = ${row}.rule_id AND violation_key = ${row}.violation_key`;
  return [
    `${row}.session_id IS NOT NULL AND session_id = ${row}.session_id AND ${ruleKey}`,
    `${row}.session_id IS NULL AND session_id IS NULL AND repository_id = ${row}.repository_id AND ${ruleKey}`,
  ];
};

/** A new sighting can only move its rollup row's maxima forward: raise them in place, or add the row. */
const RAISE_ROLLUP = `${ownerKeys("NEW").map(key => `
  UPDATE rule_violation_rollup SET
    last_seen_at = max(last_seen_at, NEW.started_at),
    last_logged_at = CASE WHEN NEW.logged = 1 THEN max(coalesce(last_logged_at, NEW.started_at), NEW.started_at) ELSE last_logged_at END
  WHERE ${key};`).join("")}
  INSERT INTO rule_violation_rollup (session_id, rule_id, violation_key, last_seen_at, last_logged_at, repository_id)
    SELECT NEW.session_id, NEW.rule_id, NEW.violation_key, NEW.started_at, CASE WHEN NEW.logged = 1 THEN NEW.started_at END, NEW.repository_id
    WHERE ${ownerKeys("NEW").map(key => `NOT EXISTS (SELECT 1 FROM rule_violation_rollup WHERE ${key})`).join(" AND ")};`;

/** The latest start of `OLD`'s owner, rule and key in `audit_violations` where `extra` holds (index-only); null for none. */
const latestSighting = (extra: string): string =>
  `coalesce(${ownerKeys("OLD").map(key => `(SELECT max(started_at) FROM audit_violations WHERE ${key}${extra})`).join(", ")})`;

/** A removed sighting may have held a maximum: recompute the row from what is left (index-only), or drop it. */
const RECOMPUTE_ROLLUP = `${ownerKeys("OLD").map(key => `
  DELETE FROM rule_violation_rollup WHERE ${key};`).join("")}
  INSERT INTO rule_violation_rollup (session_id, rule_id, violation_key, last_seen_at, last_logged_at, repository_id)
    SELECT OLD.session_id, OLD.rule_id, OLD.violation_key, latest.seen, latest.logged, OLD.repository_id FROM (
      SELECT ${latestSighting("")} AS seen, ${latestSighting(" AND logged = 1")} AS logged
    ) latest
    WHERE latest.seen IS NOT NULL;`;

/** An update that changed what a derived row is made of: the audit's identity, session, start, or `column`. */
const changed = (column: string): string => ["audit_id", "session_id", "started_at", column]
  .map(name => `OLD.${name} IS NOT NEW.${name}`).join(" OR ");

/** Applied, in order, after the tables on every open; see the module comment before changing one. */
export const AUDIT_HISTORY_TRIGGER_STATEMENTS: readonly string[] = [
  `CREATE TRIGGER IF NOT EXISTS \`audit_violations_rollup_insert\` AFTER INSERT ON \`audit_violations\`
  WHEN NEW.rule_id IS NOT NULL
  BEGIN${RAISE_ROLLUP}
  END`,
  `CREATE TRIGGER IF NOT EXISTS \`audit_violations_rollup_delete\` AFTER DELETE ON \`audit_violations\`
  WHEN OLD.rule_id IS NOT NULL
  BEGIN${RECOMPUTE_ROLLUP}
  END`,
  `CREATE TRIGGER IF NOT EXISTS \`audits_history_insert\` AFTER INSERT ON \`audits\`
  BEGIN
  DELETE FROM audit_violations WHERE audit_id = NEW.audit_id;
  DELETE FROM audit_rule_set_uses WHERE audit_id = NEW.audit_id;
  ${INSERT_VIOLATIONS};${USE_RULE_SET("NEW")}
  END`,
  `CREATE TRIGGER IF NOT EXISTS \`audits_history_violations_append\` AFTER UPDATE OF violations_json ON \`audits\`
  WHEN ${APPENDED}
  BEGIN${APPEND_VIOLATIONS}
  END`,
  `CREATE TRIGGER IF NOT EXISTS \`audits_history_violations_update\` AFTER UPDATE OF audit_id, session_id, started_at, violations_json ON \`audits\`
  WHEN (${changed("violations_json")}) AND NOT (${APPENDED})
  BEGIN${REPLACE_VIOLATIONS}
  END`,
  `CREATE TRIGGER IF NOT EXISTS \`audits_history_rule_set_update\` AFTER UPDATE OF audit_id, session_id, started_at, rule_revisions_json ON \`audits\`
  WHEN ${changed("rule_revisions_json")}
  BEGIN
  DELETE FROM audit_rule_set_uses WHERE audit_id = OLD.audit_id;${USE_RULE_SET("NEW")}
  END`,
  `CREATE TRIGGER IF NOT EXISTS \`audits_history_delete\` AFTER DELETE ON \`audits\`
  BEGIN
  DELETE FROM audit_violations WHERE audit_id = OLD.audit_id;
  DELETE FROM audit_rule_set_uses WHERE audit_id = OLD.audit_id;
  END`,
  // A stored rule provenance goes with the last audit that names it, so `rerail reset` leaves none of a repository's
  // behind, whichever build deletes the audits.
  `CREATE TRIGGER IF NOT EXISTS \`audits_rule_provenance_delete\` AFTER DELETE ON \`audits\`
  WHEN OLD.rule_provenance_id IS NOT NULL
  BEGIN
  DELETE FROM audit_rule_provenances WHERE provenance_id = OLD.rule_provenance_id
    AND NOT EXISTS (SELECT 1 FROM audits WHERE rule_provenance_id = OLD.rule_provenance_id);
  END`,
];
