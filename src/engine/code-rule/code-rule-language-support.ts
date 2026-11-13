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
  /** Where and why, as the engine words it: `/clauses/0/forbid/ast/language: \`<language>\` is not supported yet`. */
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

/** The transform kinds that read a capture's value with `value: true`; `toPrecision` always reads one. */
const VALUE_SOURCE_KINDS: ReadonlySet<string> = new Set(["replace", "substring", "convert"]);

const entriesOf = (value: Json): [string, Json][] => (isObject(value) ? Object.entries(value) : []);

const readsValue = (body: Json): boolean => isObject(body) && body.value === true;

/** Where the transform step `step` at `path` reads a value, and the key that does: `toPrecision` always, another kind with `value: true`. */
function valueRead(step: Json, path: string): { at: string; key: string } | undefined {
  const [only, ...others] = entriesOf(step);
  if (only === undefined || others.length > 0) return undefined;
  const [kind, body] = only;
  if (kind === "toPrecision") return { at: `${path}/toPrecision`, key: "toPrecision" };
  return VALUE_SOURCE_KINDS.has(kind) && readsValue(body) ? { at: `${path}/${kind}/value`, key: "value" } : undefined;
}

/** The first step of a query's `transform` that reads a value, if `language` has the value capability `not-supported-yet`. */
function unsupportedTransform(transform: Json, path: string, language: CodeRuleAstLanguage): CodeRuleLanguageNotSupported | undefined {
  if (codeRuleLanguageCapability(language, "value") !== "not-supported-yet") return undefined;
  // The engine compiles the variables in name order.
  for (const [name, step] of entriesOf(transform).sort(([a], [b]) => (a < b ? -1 : 1))) {
    const read = valueRead(step, `${path}/${name}`);
    if (read !== undefined) return { code: LANGUAGE_NOT_SUPPORTED, reason: `${read.at}: \`${read.key}\` is not supported yet for ${language} policies` };
  }
  return undefined;
}

/** Whether a query's or define's language, what its rules read, or what its `transform` reads is not supported yet. */
function unsupportedIn(holder: Record<string, Json>, path: string, rules: readonly string[]): CodeRuleLanguageNotSupported | undefined {
  const language = holder.language;
  if (typeof language !== "string") return undefined;
  if (isCodeRulePlannedLanguage(language)) return { code: LANGUAGE_NOT_SUPPORTED, reason: `${path}/language: \`${language}\` is not supported yet` };
  if (!isCodeRuleAstLanguage(language)) return undefined;
  for (const key of rules) {
    const found = holder[key] === undefined ? undefined : unsupportedPredicate(holder[key], `${path}/${key}`, language);
    if (found !== undefined) return found;
  }
  return unsupportedTransform(holder.transform, `${path}/transform`, language);
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
