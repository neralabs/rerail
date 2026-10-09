import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { PACKAGE_ROOT } from "../package-root.js";
import { loadNativeBinding, type NativeBinding } from "../native/native.js";
import { LANGUAGE_NOT_SUPPORTED, languageNotSupportedOf, type CodeRuleLanguageNotSupported } from "./code-rule-language-support.js";
import { codeRuleSchema, type CodeRule } from "./code-rule-schema.js";
import { stableJsonText } from "../stable-json.js";

/** `language-not-supported`: the rule names a language or capability the engine does not read yet; anything else is `invalid-rule`. */
export type CodeRuleCompileErrorCode = typeof LANGUAGE_NOT_SUPPORTED | "invalid-rule";

/** The engine rejected a code-rule document that the TypeScript schema accepted. */
export class CodeRuleCompileError extends Error {
  readonly ruleId: string;
  readonly reason: string;
  readonly code: CodeRuleCompileErrorCode;

  constructor(ruleId: string, reason: string) {
    super(`policy rule ${ruleId} does not compile: ${reason}`);
    this.name = "PolicyCompileError";
    this.ruleId = ruleId;
    this.reason = reason;
    this.code = languageNotSupportedOf(reason) === undefined ? "invalid-rule" : LANGUAGE_NOT_SUPPORTED;
  }
}

/** The native engine could not be loaded, so a code-rule write cannot be checked the way the live catalog will compile it. */
export class EngineUnavailableError extends Error {
  constructor(ruleId: string, reason: string) {
    super(`cannot validate policy rule ${ruleId}: native kernel unavailable: ${reason}`);
    this.name = "PolicyKernelUnavailableError";
  }
}

/** `<package>/native/src/code_rule_dsl`: the engine's `capabilities.json`, which code rules compile against, lives here. */
export function bundledCodeRuleDslDirectory(): string {
  return resolve(PACKAGE_ROOT, "native", "src", "code_rule_dsl");
}

let bundledCapabilities: string | undefined;

function capabilitiesJson(): string {
  bundledCapabilities ??= readFileSync(resolve(bundledCodeRuleDslDirectory(), "capabilities.json"), "utf8");
  return bundledCapabilities;
}

/** Why a document that JSON cannot represent (a cycle, a function) is no code rule. */
export const NOT_JSON_REPRESENTABLE = "the document is not JSON representable";

/**
 * Compile a schema-valid code rule exactly as the live catalog will (same source text, same bundled capabilities),
 * so a rule the engine rejects never reaches the catalog and fails every later refresh. Code-rule evaluation has
 * no fallback without the engine either, so an unavailable engine refuses the write rather than storing an
 * unchecked rule.
 */
export function assertCodeRuleCompiles(rule: CodeRule): void {
  const sourceText = stableJsonText(rule);
  if (sourceText === undefined) throw new CodeRuleCompileError(rule.id, NOT_JSON_REPRESENTABLE);
  let binding: NativeBinding;
  let capabilities: string;
  try {
    binding = loadNativeBinding();
    capabilities = capabilitiesJson();
  } catch (error) {
    throw new EngineUnavailableError(rule.id, error instanceof Error ? error.message : String(error));
  }
  try {
    binding.CodeRuleDslRuntime.compile(sourceText, "json", capabilities);
  } catch (error) {
    throw new CodeRuleCompileError(rule.id, error instanceof Error ? error.message : String(error));
  }
}

/** A code rule the catalog may store, and why it stays inactive when the engine does not read it yet. */
export interface StorableCodeRule {
  rule: CodeRule;
  inactive?: CodeRuleLanguageNotSupported;
}

/**
 * The schema's parse of `input`, compiled as the live catalog will compile it. A rule the engine refuses only as
 * `language-not-supported` is storable: the catalog keeps it, inactive, until the engine reads its language. Throws
 * the schema's `ZodError`, `CodeRuleCompileError` or `EngineUnavailableError`.
 */
export function storableCodeRule(input: unknown): StorableCodeRule {
  const rule = codeRuleSchema.parse(input);
  try {
    assertCodeRuleCompiles(rule);
  } catch (error) {
    const inactive = error instanceof CodeRuleCompileError ? languageNotSupportedOf(error.reason) : undefined;
    if (inactive === undefined) throw error;
    return { rule, inactive };
  }
  return { rule };
}
