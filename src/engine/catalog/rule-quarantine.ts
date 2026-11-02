import { assertCodeRuleCompiles, CodeRuleCompileError } from "../code-rule/code-rule-compile.js";
import { LANGUAGE_NOT_SUPPORTED, languageNotSupportedOf } from "../code-rule/code-rule-language-support.js";
import { codeRuleSchema, type CodeRule } from "../code-rule/code-rule-schema.js";
import { RULE_KINDS, type RuleKind } from "../storage/schema.js";
import { ruleDocumentDigest } from "../stable-json.js";

/** Digest of a stored rule body exactly as it sits in the row, valid or not. */
export function storedDocumentDigest(kind: RuleKind, rule: unknown): string {
  // ruleDocumentDigest only serializes the body; it reads nothing typed beyond `id` for its error message.
  return ruleDocumentDigest({ kind, rule } as Parameters<typeof ruleDocumentDigest>[0]);
}

export type StoredCodeRuleRead = { ok: true; rule: CodeRule } | { ok: false; reason: string };

/**
 * Validate a stored code-rule body as a write validates it: the current schema, then the engine compile. Other
 * processes and plain SQL write these rows too, so a body the engine refuses is quarantined with the engine's reason.
 * One it refuses only as `language-not-supported` reads, as the write stored it: kept inactive. Without the engine the
 * schema alone decides, and the catalog, which needs the engine to run any rule, refuses the rest. Never throws.
 */
export function readStoredCodeRule(stored: unknown): StoredCodeRuleRead {
  const parsed = codeRuleSchema.safeParse(stored);
  if (!parsed.success) {
    const issue = parsed.error.issues[0];
    return { ok: false, reason: issue === undefined ? "invalid policy document" : `${issue.path.join("/") || "(root)"}: ${issue.message}` };
  }
  try {
    assertCodeRuleCompiles(parsed.data);
  } catch (error) {
    // The compile throws only CodeRuleCompileError or, without the engine, EngineUnavailableError.
    if (error instanceof CodeRuleCompileError && languageNotSupportedOf(error.reason) === undefined) return { ok: false, reason: error.reason };
  }
  return { ok: true, rule: parsed.data };
}

const isRuleKind = (value: unknown): value is RuleKind => (RULE_KINDS as readonly unknown[]).includes(value);

/**
 * Why a row whose kind is neither `code` nor `conversation` is skipped, and the kind it is reported under: the one
 * its stored document records, else `code` (the only other kind a Rerail ever wrote is the code rule kind's old
 * name). Undefined for a known kind. A database written before the kind checks can hold any kind, since the DDL
 * does not add a check to a table that already exists.
 */
export function unknownRuleKind(kind: string, document: unknown): { kind: RuleKind; reason: string } | undefined {
  if (isRuleKind(kind)) return undefined;
  const recorded = typeof document === "object" && document !== null && "kind" in document ? document.kind : undefined;
  return { kind: isRuleKind(recorded) ? recorded : "code", reason: `unknown rule kind ${kind}` };
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
