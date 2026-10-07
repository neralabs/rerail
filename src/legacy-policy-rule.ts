import { POLICY_FINDING_ANCHOR_PATTERN, policyRuleSchema, type PolicyRule } from "./policy-schema.js";
import type { RuleKind } from "./schema.js";
import { ruleDocumentDigest } from "./stable-json.js";

/**
 * Stored `rerail.policy/v1` rows can predate the current schema. Before the kernel started rejecting them
 * (#2561) it accepted and ignored `ast.capture`, `regex.captures`, `finding.variables`, and any
 * `finding.anchor` string: every finding was reported at its witness span. Dropping those fields, and
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

/** A query (`forbid`, or a `select_when` leaf) without the capture fields the kernel used to ignore. */
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

function normalizeFinding(finding: unknown, hasQuery: boolean, path: string, changes: string[]): unknown {
  if (!isObject(finding)) return finding;
  let next = finding;
  const anchor = finding.anchor;
  // require_each, budget, and llm findings carry no source span, so the kernel accepts only witness.span there.
  const legacyAnchor = typeof anchor === "string" && (!POLICY_FINDING_ANCHOR_PATTERN.test(anchor) || (!hasQuery && anchor !== "witness.span"));
  if (legacyAnchor) {
    next = { ...next, anchor: "witness.span" };
    changes.push(`${path}/anchor: ${anchor} -> witness.span`);
  }
  if ("variables" in finding) {
    next = without(next, "variables");
    changes.push(`${path}/variables: dropped`);
  }
  return next;
}

function normalizeCheck(check: unknown, path: string, changes: string[]): unknown {
  if (!isObject(check)) return check;
  let next = check;
  const finding = normalizeFinding(check.finding, check.forbid !== undefined, `${path}/finding`, changes);
  if (finding !== check.finding) next = { ...next, finding };
  const forbid = normalizeQuery(check.forbid, `${path}/forbid`, changes);
  if (forbid !== check.forbid) next = { ...next, forbid };
  return next;
}

export interface LegacyPolicyNormalization {
  /** The input itself when nothing changed; otherwise a copy. The input is never mutated. */
  rule: unknown;
  /** One `<json path>: <what changed>` entry per rewrite, in document order. */
  changes: string[];
}

/** Rewrite the pre-#2561 fields of a stored policy document into their current equivalent. */
export function normalizeLegacyPolicyRule(rule: unknown): LegacyPolicyNormalization {
  const changes: string[] = [];
  if (!isObject(rule) || !Array.isArray(rule.checks)) return { rule, changes };
  const checks = rule.checks.map((check: unknown, index) => normalizeCheck(check, `checks/${index}`, changes));
  return changes.length === 0 ? { rule, changes } : { rule: { ...rule, checks }, changes };
}

/** Digest of a stored rule body exactly as it sits in the row, valid or not. */
export function storedDocumentDigest(kind: RuleKind, rule: unknown): string {
  // ruleDocumentDigest only serializes the body; it reads nothing typed beyond `id` for its error message.
  return ruleDocumentDigest({ kind, rule } as Parameters<typeof ruleDocumentDigest>[0]);
}

export type StoredPolicyRead =
  | {
    ok: true;
    rule: PolicyRule;
    changes: readonly string[];
    /**
     * The row's source digest, restated for the normalized body when the stored body was an untouched
     * bundled document: the row stays "unmodified" to seeding and to anything it is exported or migrated into.
     */
    sourceDigest: string | null;
  }
  | { ok: false; reason: string };

/** Normalize, then validate, a stored policy body. Never throws. */
export function readStoredPolicyRule(stored: unknown, sourceDigest: string | null): StoredPolicyRead {
  const { rule, changes } = normalizeLegacyPolicyRule(stored);
  const parsed = policyRuleSchema.safeParse(rule);
  if (!parsed.success) {
    const issue = parsed.error.issues[0];
    const reason = issue === undefined ? "invalid policy document" : `${issue.path.join("/") || "(root)"}: ${issue.message}`;
    return { ok: false, reason };
  }
  const untouched = changes.length > 0 && sourceDigest !== null && storedDocumentDigest("policy", stored) === sourceDigest;
  return {
    ok: true,
    rule: parsed.data,
    changes,
    sourceDigest: untouched ? storedDocumentDigest("policy", parsed.data) : sourceDigest,
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
