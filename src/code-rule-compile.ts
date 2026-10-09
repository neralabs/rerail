import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { loadNativeBinding, type NativeBinding } from "./native.js";
import { LANGUAGE_NOT_SUPPORTED, languageNotSupportedOf } from "./code-rule-language-support.js";
import type { CodeRule } from "./code-rule-schema.js";
import { stableJsonText } from "./stable-json.js";

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
  const moduleDirectory = dirname(fileURLToPath(import.meta.url));
  return resolve(moduleDirectory, "..", "native", "src", "code_rule_dsl");
}

let bundledCapabilities: string | undefined;

function capabilitiesJson(): string {
  bundledCapabilities ??= readFileSync(resolve(bundledCodeRuleDslDirectory(), "capabilities.json"), "utf8");
  return bundledCapabilities;
}

/**
 * Compile a schema-valid code rule exactly as the live catalog will (same source text, same bundled capabilities),
 * so a rule the engine rejects never reaches the catalog and fails every later refresh. Code-rule evaluation has
 * no fallback without the engine either, so an unavailable engine refuses the write rather than storing an
 * unchecked rule.
 */
export function assertCodeRuleCompiles(rule: CodeRule): void {
  const sourceText = stableJsonText(rule);
  if (sourceText === undefined) throw new CodeRuleCompileError(rule.id, "the document is not JSON representable");
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

/**
 * `assertCodeRuleCompiles`, except that a rule refused only as `language-not-supported` passes: the catalog stores it,
 * inactive, until the engine reads its language.
 */
export function assertCodeRuleStorable(rule: CodeRule): void {
  try {
    assertCodeRuleCompiles(rule);
  } catch (error) {
    if (error instanceof CodeRuleCompileError && error.code === LANGUAGE_NOT_SUPPORTED) return;
    throw error;
  }
}
