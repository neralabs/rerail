import {
  isCodeRuleAstLanguage,
  isCodeRulePlannedLanguage,
  codeRuleLanguageCapability,
  type CodeRuleAstLanguage,
  type CodeRuleSemanticCapability,
} from "./code-rule-languages.js";

/**
 * The stable code of a rule the engine cannot read yet: its `language` is one the registry plans, or it reads a
 * capability its language has `not-supported-yet`. Compiling refuses it with this code (the engine's message starts
 * with it); the catalog keeps the rule stored but inactive, with the reason.
 */
export const LANGUAGE_NOT_SUPPORTED = "language-not-supported";

export interface CodeRuleLanguageNotSupported {
  code: typeof LANGUAGE_NOT_SUPPORTED;
  /** Where and why, as the engine words it: `/clauses/0/forbid/ast/language: \`vue\` is not supported yet`. */
  reason: string;
}

/** The predicates a language reads only with a capability, as the engine maps them. */
export const CODE_RULE_PREDICATE_CAPABILITIES: Readonly<Record<string, CodeRuleSemanticCapability>> = {
  resolves: "scope",
  binds: "scope",
  value: "value",
  compare: "value",
  arguments: "arguments",
  calls: "calls",
  formatArgs: "formatArgs",
};

/** The `language-not-supported` reason an engine compile error carries, if it is one. */
export function languageNotSupportedOf(message: string): CodeRuleLanguageNotSupported | undefined {
  const prefix = `${LANGUAGE_NOT_SUPPORTED}: `;
  const at = message.indexOf(prefix);
  return at < 0 ? undefined : { code: LANGUAGE_NOT_SUPPORTED, reason: message.slice(at + prefix.length) };
}

type Json = unknown;

const isObject = (value: Json): value is Record<string, Json> => typeof value === "object" && value !== null && !Array.isArray(value);

/** The first predicate under `value` whose capability `language` has `not-supported-yet`. */
function unsupportedPredicate(value: Json, path: string, language: CodeRuleAstLanguage): CodeRuleLanguageNotSupported | undefined {
  const children: [string, Json][] = Array.isArray(value) ? value.map((child, index) => [String(index), child]) : isObject(value) ? Object.entries(value) : [];
  for (const [key, child] of children) {
    const capability = isObject(value) ? CODE_RULE_PREDICATE_CAPABILITIES[key] : undefined;
    if (capability !== undefined && codeRuleLanguageCapability(language, capability) === "not-supported-yet") {
      return { code: LANGUAGE_NOT_SUPPORTED, reason: `${path}/${key}: \`${key}\` is not supported yet for ${language} policies` };
    }
    const found = unsupportedPredicate(child, `${path}/${key}`, language);
    if (found !== undefined) return found;
  }
  return undefined;
}

/** Whether a query's or define's language, or what its rules read, is not supported yet. */
function unsupportedIn(holder: Record<string, Json>, path: string, rules: readonly string[]): CodeRuleLanguageNotSupported | undefined {
  const language = holder.language;
  if (typeof language !== "string") return undefined;
  if (isCodeRulePlannedLanguage(language)) return { code: LANGUAGE_NOT_SUPPORTED, reason: `${path}/language: \`${language}\` is not supported yet` };
  if (!isCodeRuleAstLanguage(language)) return undefined;
  for (const key of rules) {
    const found = holder[key] === undefined ? undefined : unsupportedPredicate(holder[key], `${path}/${key}`, language);
    if (found !== undefined) return found;
  }
  return undefined;
}

/**
 * Why a code-rule document is not supported yet, or undefined when the engine reads every language and capability it
 * names. It reads the registry as the engine's compile does, so the reason is the one compiling gives.
 */
export function codeRuleLanguageNotSupported(document: Json, path = ""): CodeRuleLanguageNotSupported | undefined {
  const children: [string, Json][] = Array.isArray(document) ? document.map((child, index) => [String(index), child]) : isObject(document) ? Object.entries(document) : [];
  for (const [key, child] of children) {
    const at = `${path}/${key}`;
    const found =
      isObject(document) && key === "ast" && isObject(child)
        ? unsupportedIn(child, at, ["rule", "utils", "constraints"])
        : isObject(document) && key === "define" && isObject(child)
          ? unsupportedIn(child, at, ["utils"])
          : undefined;
    if (found !== undefined) return found;
    const nested = codeRuleLanguageNotSupported(child, at);
    if (nested !== undefined) return nested;
  }
  return undefined;
}
