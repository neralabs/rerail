import { CODE_RULE_VIOLATION_ANCHOR_PATTERN, codeRuleSchema, type CodeRule } from "./code-rule-schema.js";
import type { RuleKind } from "./schema.js";
import { ruleDocumentDigest } from "./stable-json.js";

/**
 * Stored `rerail.policy/v1` rows can predate the current schema. Before the engine started rejecting them
 * (#2561) it accepted and ignored `ast.capture`, `regex.captures`, `violation.variables`, and any
 * `violation.anchor` string: every violation was reported at its witness span. Dropping those fields, and
 * anchoring a free-form anchor at `witness.span`, is therefore behavior-preserving. Normalization is
 * read-only: stored rows, their payload hashes, and their bundled source digests are never rewritten.
 */

type JsonObject = Record<string, unknown>;

function isObject(value: unknown): value is JsonObject {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function without(value: JsonObject, key: string): JsonObject {
  const { [key]: _dropped, ...rest } = value;
  return rest;
}

/** A query (`forbid`, or a `select_when` leaf) without the capture fields the engine used to ignore. */
function normalizeQuery(query: unknown, path: string, changes: string[]): unknown {
  if (!isObject(query)) return query;
  let next = query;
  const ast = query.ast;
  if (isObject(ast) && "capture" in ast) {
    next = { ...next, ast: without(ast, "capture") };
    changes.push(`${path}/ast/capture: dropped`);
  }
  const regex = query.regex;
  if (isObject(regex) && "captures" in regex) {
    next = { ...next, regex: without(regex, "captures") };
    changes.push(`${path}/regex/captures: dropped`);
  }
  const selectWhen = query.select_when;
  if (isObject(selectWhen)) {
    const select = normalizeQuery(selectWhen.select, `${path}/select_when/select`, changes);
    const exists = normalizeQuery(selectWhen.exists, `${path}/select_when/exists`, changes);
    if (select !== selectWhen.select || exists !== selectWhen.exists) next = { ...next, select_when: { ...selectWhen, select, exists } };
  }
  return next;
}

function normalizeViolation(violation: unknown, hasQuery: boolean, path: string, changes: string[]): unknown {
  if (!isObject(violation)) return violation;
  let next = violation;
  const anchor = violation.anchor;
  // require_each, budget, and llm violations carry no source span, so the engine accepts only witness.span there.
  const legacyAnchor = typeof anchor === "string" && (!CODE_RULE_VIOLATION_ANCHOR_PATTERN.test(anchor) || (!hasQuery && anchor !== "witness.span"));
  if (legacyAnchor) {
    next = { ...next, anchor: "witness.span" };
    changes.push(`${path}/anchor: ${anchor} -> witness.span`);
  }
  if ("variables" in violation) {
    next = without(next, "variables");
    changes.push(`${path}/variables: dropped`);
  }
  return next;
}

function normalizeClause(clause: unknown, path: string, changes: string[]): unknown {
  if (!isObject(clause)) return clause;
  let next = clause;
  const violation = normalizeViolation(clause.violation, clause.forbid !== undefined, `${path}/violation`, changes);
  if (violation !== clause.violation) next = { ...next, violation: violation };
  const forbid = normalizeQuery(clause.forbid, `${path}/forbid`, changes);
  if (forbid !== clause.forbid) next = { ...next, forbid };
  return next;
}

export interface LegacyCodeRuleNormalization {
  /** The input itself when nothing changed; otherwise a copy. The input is never mutated. */
  rule: unknown;
  /** One `<json path>: <what changed>` entry per rewrite, in document order. */
  changes: string[];
}

/** Rewrite the pre-#2561 fields of a stored code-rule document into their current equivalent. */
export function normalizeLegacyCodeRule(rule: unknown): LegacyCodeRuleNormalization {
  const changes: string[] = [];
  if (!isObject(rule) || !Array.isArray(rule.clauses)) return { rule, changes };
  const clauses = rule.clauses.map((clause: unknown, index) => normalizeClause(clause, `clauses/${index}`, changes));
  return changes.length === 0 ? { rule, changes } : { rule: { ...rule, clauses: clauses }, changes };
}

/** Digest of a stored rule body exactly as it sits in the row, valid or not. */
export function storedDocumentDigest(kind: RuleKind, rule: unknown): string {
  // ruleDocumentDigest only serializes the body; it reads nothing typed beyond `id` for its error message.
  return ruleDocumentDigest({ kind, rule } as Parameters<typeof ruleDocumentDigest>[0]);
}

export type StoredCodeRuleRead =
  | {
    ok: true;
    rule: CodeRule;
    changes: readonly string[];
    /**
     * The row's source digest, restated for the normalized body when the stored body was an untouched
     * bundled document: the row stays "unmodified" to seeding and to anything it is exported or migrated into.
     */
    sourceDigest: string | null;
  }
  | { ok: false; reason: string };

/** Normalize, then validate, a stored code-rule body. Never throws. */
export function readStoredCodeRule(stored: unknown, sourceDigest: string | null): StoredCodeRuleRead {
  const { rule, changes } = normalizeLegacyCodeRule(stored);
  const parsed = codeRuleSchema.safeParse(rule);
  if (!parsed.success) {
    const issue = parsed.error.issues[0];
    const reason = issue === undefined ? "invalid policy document" : `${issue.path.join("/") || "(root)"}: ${issue.message}`;
    return { ok: false, reason };
  }
  const untouched = changes.length > 0 && sourceDigest !== null && storedDocumentDigest("file", stored) === sourceDigest;
  return {
    ok: true,
    rule: parsed.data,
    changes,
    sourceDigest: untouched ? storedDocumentDigest("file", parsed.data) : sourceDigest,
  };
}

/** A stored row that failed validation and was left out of every read. */
export interface QuarantinedRule {
  ruleId: string;
  kind: RuleKind;
  /** `global`, `project:<id>`, or the per-project store's row scope. */
  owner: string;
  reason: string;
}

const reported = new Set<string>();

/** Warn on stderr, once per process for each catalog, row, and reason, that a row is being skipped. */
export function reportQuarantinedRule(catalogPath: string, quarantined: QuarantinedRule): void {
  const key = JSON.stringify([catalogPath, quarantined.owner, quarantined.ruleId, quarantined.reason]);
  if (reported.has(key)) return;
  reported.add(key);
  process.stderr.write(
    `rerail: skipping ${quarantined.kind} rule ${quarantined.ruleId} (${quarantined.owner}) in ${catalogPath}: ${quarantined.reason}\n`,
  );
}
