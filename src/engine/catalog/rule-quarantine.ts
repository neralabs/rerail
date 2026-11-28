import { assertCodeRuleCompiles, CodeRuleCompileError, type InactiveCodeRule } from "../code-rule/code-rule-compile.js";
import { languageNotSupportedOf } from "../code-rule/code-rule-language-support.js";
import { codeRuleSchema, type CodeRule } from "../code-rule/code-rule-schema.js";
import { RULE_KINDS, type RuleKind } from "../storage/schema.js";
import type { StoredListLog } from "../storage/stored-list.js";
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
 * A stored row left out of every read and evaluation: one that fails validation, or, with a `code`, a valid code rule
 * kept inactive, never a failure: `language-not-supported`, one the engine cannot read yet, or `reads-left-out-token`,
 * an installed pack's rule this Rerail no longer runs.
 */
export interface QuarantinedRule {
  ruleId: string;
  kind: RuleKind;
  /** `global`, `project:<id>`, or the per-project store's row scope. */
  owner: string;
  reason: string;
  code?: InactiveCodeRule["code"];
}

/**
 * The JSON object `text`, a rule's stored document or a setting's stored options, holds, or why it holds none. Rerail
 * writes only objects there, so only a damaged file or a hand edit leaves anything else, and the reader quarantines
 * that one row rather than failing every read that holds it.
 */
export function storedObject(text: string): Record<string, unknown> | string {
  let value: unknown;
  try {
    value = JSON.parse(text);
  } catch (error) {
    return (error as SyntaxError).message; // JSON.parse throws nothing else
  }
  return isJsonObject(value) ? value : "it parses to another JSON value";
}

/** Whether a parsed JSON value is an object: not null, not an array. */
function isJsonObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** Where a reader names a stored row whose text does not read: the database file, and the log the line goes to. */
export interface StoredRowLog {
  readonly file: string;
  readonly log: StoredListLog;
}

/** The {@link StoredRowLog} of a reader of `database` whose lines go to its process's error output: `daemon.log` in the daemon. */
export function stderrRowLog(database: { readonly $client: { readonly name: string } }): StoredRowLog {
  return { file: database.$client.name, log: process.stderr };
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
