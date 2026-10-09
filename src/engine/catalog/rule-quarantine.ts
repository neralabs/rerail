import { LANGUAGE_NOT_SUPPORTED } from "../code-rule/code-rule-language-support.js";
import { codeRuleSchema, type CodeRule } from "../code-rule/code-rule-schema.js";
import type { RuleKind } from "../storage/schema.js";
import { ruleDocumentDigest } from "../stable-json.js";

/** Digest of a stored rule body exactly as it sits in the row, valid or not. */
export function storedDocumentDigest(kind: RuleKind, rule: unknown): string {
  // ruleDocumentDigest only serializes the body; it reads nothing typed beyond `id` for its error message.
  return ruleDocumentDigest({ kind, rule } as Parameters<typeof ruleDocumentDigest>[0]);
}

export type StoredCodeRuleRead = { ok: true; rule: CodeRule } | { ok: false; reason: string };

/** Validate a stored code-rule body against the current schema. Never throws. */
export function readStoredCodeRule(stored: unknown): StoredCodeRuleRead {
  const parsed = codeRuleSchema.safeParse(stored);
  if (parsed.success) return { ok: true, rule: parsed.data };
  const issue = parsed.error.issues[0];
  return { ok: false, reason: issue === undefined ? "invalid policy document" : `${issue.path.join("/") || "(root)"}: ${issue.message}` };
}

/**
 * A stored row left out of every read and evaluation: one that fails validation, or, with `code`
 * `language-not-supported`, a valid code rule the engine cannot read yet (kept inactive, never a failure).
 */
export interface QuarantinedRule {
  ruleId: string;
  kind: RuleKind;
  /** `global`, `project:<id>`, or the per-project store's row scope. */
  owner: string;
  reason: string;
  code?: typeof LANGUAGE_NOT_SUPPORTED;
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
