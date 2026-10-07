import { policyRuleSchema, type PolicyRule } from "./policy-schema.js";
import type { RuleKind } from "./schema.js";
import { ruleDocumentDigest } from "./stable-json.js";

/** Digest of a stored rule body exactly as it sits in the row, valid or not. */
export function storedDocumentDigest(kind: RuleKind, rule: unknown): string {
  // ruleDocumentDigest only serializes the body; it reads nothing typed beyond `id` for its error message.
  return ruleDocumentDigest({ kind, rule } as Parameters<typeof ruleDocumentDigest>[0]);
}

export type StoredPolicyRead = { ok: true; rule: PolicyRule } | { ok: false; reason: string };

/** Validate a stored policy body against the current schema. Never throws. */
export function readStoredPolicyRule(stored: unknown): StoredPolicyRead {
  const parsed = policyRuleSchema.safeParse(stored);
  if (parsed.success) return { ok: true, rule: parsed.data };
  const issue = parsed.error.issues[0];
  return { ok: false, reason: issue === undefined ? "invalid policy document" : `${issue.path.join("/") || "(root)"}: ${issue.message}` };
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
