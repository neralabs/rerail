import { z } from "zod";
import { CodeRuleCompileError, EngineUnavailableError, storableCodeRule } from "./code-rule-compile.js";
import type { LANGUAGE_NOT_SUPPORTED } from "./code-rule-language-support.js";
import { codeRuleSchema, type CodeRule } from "./code-rule-schema.js";

/**
 * `@neralabs/rerail/rules`: whether a code-rule document is one Rerail stores, checked exactly as its own writes check
 * it. It loads the schema and the native engine only, never the daemon or a host extension.
 */

/**
 * A storable rule (`inactive` when the engine does not read its language, or a capability it uses, yet), or why it is
 * not one: the schema's `issues`, or the engine's compile error as `reason`.
 */
export type CodeRuleValidation =
  | { ok: true; rule: CodeRule; inactive?: { code: typeof LANGUAGE_NOT_SUPPORTED; reason: string } }
  | { ok: false; code: "invalid-rule"; reason: string; issues?: readonly z.core.$ZodIssue[] };

/** Schema, then the engine compile the live catalog uses. Throws EngineUnavailableError only. */
export function validateCodeRule(input: unknown): CodeRuleValidation {
  try {
    return { ok: true, ...storableCodeRule(input) };
  } catch (error) {
    if (error instanceof z.ZodError) return { ok: false, code: "invalid-rule", reason: z.prettifyError(error), issues: error.issues };
    if (error instanceof CodeRuleCompileError) return { ok: false, code: "invalid-rule", reason: error.reason };
    throw error;
  }
}

export { codeRuleSchema, type CodeRule, EngineUnavailableError };
