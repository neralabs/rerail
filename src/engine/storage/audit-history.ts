/**
 * Keeps the derived audit history tables (`audit_violations`, `rule_violation_rollup`, `audit_rule_sets`,
 * `audit_rule_set_rules`, `audit_rule_set_uses`; see `schema.ts`) in step with `audits`. They are SQLite triggers
 * rather than code in the audit writer because every Rerail that shares the file writes `audits`, an older one
 * included, and only a trigger sees all of them.
 *
 * The triggers are part of the DDL applied on every open (`schema-sql.generated.ts`), created `IF NOT EXISTS`: a
 * build that changes one must drop it in a migration step so the new body is created. The statements avoid conflict
 * clauses (`OR IGNORE`, upserts), which an outer `INSERT OR REPLACE` on `audits` would override.
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
 * The `audit_violations` rows of the audits read through `audit` (its `audit_id`, `session_id`, `started_at` and
 * `violations_json`), from `tables` (empty in a trigger, where `audit` is `NEW`): repeats of a (rule, key) within one
 * audit collapse onto the first copy.
 */
function auditViolationRowsSql(audit: string, tables: string): string {
  const key = violationKeySql("violation.value");
  const copies = `PARTITION BY ${audit}.audit_id, json_extract(violation.value, '$.ruleId'), ${key}`;
  return `SELECT entry.audit_id, entry.ordinal, entry.session_id, entry.started_at, entry.rule_id, entry.violation_key, entry.logged
    FROM (
      SELECT
        ${audit}.audit_id AS audit_id,
        ${audit}.session_id AS session_id,
        ${audit}.started_at AS started_at,
        violation.key AS ordinal,
        json_extract(violation.value, '$.ruleId') AS rule_id,
        ${key} AS violation_key,
        row_number() OVER (${copies} ORDER BY violation.key) AS copy,
        max(coalesce(json_extract(violation.value, '$.mode') = 'log', 0)) OVER (${copies}) AS logged
      FROM ${tables === "" ? "" : `${tables}, `}json_each(${audit}.violations_json) violation
    ) entry
    WHERE entry.copy = 1`;
}

const VIOLATION_COLUMNS = "audit_violations (audit_id, ordinal, session_id, started_at, rule_id, violation_key, logged)";

const INSERT_VIOLATIONS = (audit: string): string => `INSERT INTO ${VIOLATION_COLUMNS} ${auditViolationRowsSql(audit, "")}`;

/** Creates the rule set of `audit`'s `rule_revisions_json` and its members unless it exists, then records the audit's use of it. */
const USE_RULE_SET = (audit: string): string => `
  INSERT INTO audit_rule_sets (rule_revisions_json)
    SELECT ${audit}.rule_revisions_json WHERE NOT EXISTS (SELECT 1 FROM audit_rule_sets WHERE rule_revisions_json = ${audit}.rule_revisions_json);
  INSERT INTO audit_rule_set_rules (rule_set_id, rule_id)
    SELECT DISTINCT rule_set.rule_set_id, json_extract(revision.value, '$.ruleId')
    FROM audit_rule_sets rule_set, json_each(rule_set.rule_revisions_json) revision
    WHERE rule_set.rule_revisions_json = ${audit}.rule_revisions_json
      AND json_extract(revision.value, '$.ruleId') IS NOT NULL
      AND NOT EXISTS (SELECT 1 FROM audit_rule_set_rules member WHERE member.rule_set_id = rule_set.rule_set_id);
  INSERT INTO audit_rule_set_uses (audit_id, session_id, started_at, rule_set_id)
    SELECT ${audit}.audit_id, ${audit}.session_id, ${audit}.started_at, rule_set_id
    FROM audit_rule_sets WHERE rule_revisions_json = ${audit}.rule_revisions_json;`;

const sameKey = (row: string): string =>
  `session_id = ${row}.session_id AND rule_id = ${row}.rule_id AND violation_key = ${row}.violation_key`;

/** A new sighting can only move its rollup row's maxima forward: raise them in place, or add the row. */
const RAISE_ROLLUP = `
  UPDATE rule_violation_rollup SET
    last_seen_at = max(last_seen_at, NEW.started_at),
    last_logged_at = CASE WHEN NEW.logged = 1 THEN max(coalesce(last_logged_at, NEW.started_at), NEW.started_at) ELSE last_logged_at END
  WHERE ${sameKey("NEW")};
  INSERT INTO rule_violation_rollup (session_id, rule_id, violation_key, last_seen_at, last_logged_at)
    SELECT NEW.session_id, NEW.rule_id, NEW.violation_key, NEW.started_at, CASE WHEN NEW.logged = 1 THEN NEW.started_at END
    WHERE NOT EXISTS (SELECT 1 FROM rule_violation_rollup WHERE ${sameKey("NEW")});`;

/** A removed sighting may have held a maximum: recompute the row from what is left (index-only), or drop it. */
const RECOMPUTE_ROLLUP = `
  DELETE FROM rule_violation_rollup WHERE ${sameKey("OLD")};
  INSERT INTO rule_violation_rollup (session_id, rule_id, violation_key, last_seen_at, last_logged_at)
    SELECT OLD.session_id, OLD.rule_id, OLD.violation_key, latest.seen, latest.logged FROM (
      SELECT
        (SELECT max(started_at) FROM audit_violations WHERE ${sameKey("OLD")}) AS seen,
        (SELECT max(started_at) FROM audit_violations WHERE ${sameKey("OLD")} AND logged = 1) AS logged
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
  ${INSERT_VIOLATIONS("NEW")};${USE_RULE_SET("NEW")}
  END`,
  `CREATE TRIGGER IF NOT EXISTS \`audits_history_violations_update\` AFTER UPDATE OF audit_id, session_id, started_at, violations_json ON \`audits\`
  WHEN ${changed("violations_json")}
  BEGIN
  DELETE FROM audit_violations WHERE audit_id = OLD.audit_id;
  ${INSERT_VIOLATIONS("NEW")};
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
];

/**
 * Fills the derived tables from the audits a file already holds: the migration step that introduces them runs it once,
 * before the triggers exist. Equivalent to replaying every audit through the triggers.
 */
export const AUDIT_HISTORY_BACKFILL_STATEMENTS: readonly string[] = [
  `INSERT INTO ${VIOLATION_COLUMNS} ${auditViolationRowsSql("audit", "audits audit")}`,
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
