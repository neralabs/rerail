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
type Found = CodeRuleLanguageNotSupported | undefined;

const isObject = (value: Json): value is Record<string, Json> => typeof value === "object" && value !== null && !Array.isArray(value);

const byKey = ([a]: [string, unknown], [b]: [string, unknown]): number => (a < b ? -1 : a > b ? 1 : 0);

const entriesOf = (value: Json): [string, Json][] => (isObject(value) ? Object.entries(value) : []);

/** An object's entries in key order, as the engine reads every object (its JSON maps are ordered by key). */
const sortedEntries = (value: Json): [string, Json][] => entriesOf(value).sort(byKey);

/** The first of `items` for which `find` finds something. */
function first<T>(items: Iterable<T>, find: (item: T) => Found): Found {
  for (const item of items) {
    const found = find(item);
    if (found !== undefined) return found;
  }
  return undefined;
}

const notYet = (at: string, key: string, language: CodeRuleAstLanguage): CodeRuleLanguageNotSupported =>
  ({ code: LANGUAGE_NOT_SUPPORTED, reason: `${at}: \`${key}\` is not supported yet for ${language} policies` });

/** The first predicate under `value` whose capability `language` has `not-supported-yet`, in the engine's order. */
function unsupportedPredicate(value: Json, path: string, language: CodeRuleAstLanguage): Found {
  if (Array.isArray(value)) return first(value.entries(), ([index, child]) => unsupportedPredicate(child, `${path}/${index}`, language));
  return first(sortedEntries(value), ([key, child]) => unsupportedKey(key, child, `${path}/${key}`, language));
}

/** `key` at `at`, if it is a predicate `language` does not read yet, or else the first such predicate under it. */
function unsupportedKey(key: string, child: Json, at: string, language: CodeRuleAstLanguage): Found {
  const capability = Object.hasOwn(CODE_RULE_PREDICATE_CAPABILITIES, key) ? CODE_RULE_PREDICATE_CAPABILITIES[key] : undefined;
  if (capability !== undefined && codeRuleLanguageCapability(language, capability) === "not-supported-yet") return notYet(at, key, language);
  return unsupportedPredicate(child, at, language);
}

/** Named rules (utilities, constraints) by name: each is walked as a rule, and its name is no predicate. */
const unsupportedNamed = (rules: Json, path: string, language: CodeRuleAstLanguage): Found =>
  first(sortedEntries(rules), ([name, rule]) => unsupportedPredicate(rule, `${path}/${name}`, language));

/** The transform kinds that read a capture's value with `value: true`; `toPrecision` always reads one. */
const VALUE_SOURCE_KINDS: ReadonlySet<string> = new Set(["replace", "substring", "convert"]);

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
function unsupportedTransform(transform: Json, path: string, language: CodeRuleAstLanguage): Found {
  if (codeRuleLanguageCapability(language, "value") !== "not-supported-yet") return undefined;
  return first(sortedEntries(transform), ([name, step]) => {
    const read = valueRead(step, `${path}/${name}`);
    return read === undefined ? undefined : notYet(read.at, read.key, language);
  });
}

/** A query's or define's `language`, if the registry plans it. */
function plannedLanguage(holder: Record<string, Json>, path: string): Found {
  const language = holder.language;
  return typeof language === "string" && isCodeRulePlannedLanguage(language)
    ? { code: LANGUAGE_NOT_SUPPORTED, reason: `${path}/language: \`${language}\` is not supported yet` }
    : undefined;
}

const astLanguage = (holder: Record<string, Json>): CodeRuleAstLanguage | undefined =>
  typeof holder.language === "string" && isCodeRuleAstLanguage(holder.language) ? holder.language : undefined;

/** An ast query: its language, then its utilities, rule, constraints and transform, as the engine compiles them. */
function unsupportedQuery(ast: Record<string, Json>, path: string): Found {
  const planned = plannedLanguage(ast, path);
  const language = astLanguage(ast);
  if (planned !== undefined || language === undefined) return planned;
  return unsupportedNamed(ast.utils, `${path}/utils`, language)
    ?? unsupportedPredicate(ast.rule, `${path}/rule`, language)
    ?? unsupportedNamed(ast.constraints, `${path}/constraints`, language)
    ?? unsupportedTransform(ast.transform, `${path}/transform`, language);
}

/** The keys holding queries that the engine compiles in this order (`require_each`'s, `select_when`'s); other keys follow, in key order. */
const QUERY_ORDER: readonly string[] = ["subject", "evidence", "select", "exists"];

const queryRank = (key: string): number => (QUERY_ORDER.includes(key) ? QUERY_ORDER.indexOf(key) : QUERY_ORDER.length);

/** The first ast query under a clause's `value` that is not supported yet, in the order the engine compiles them. */
function unsupportedInClause(value: Json, path: string): Found {
  const entries = sortedEntries(value).sort(([a], [b]) => queryRank(a) - queryRank(b));
  return first(entries, ([key, child]) =>
    key === "ast" && isObject(child) ? unsupportedQuery(child, `${path}/ast`) : unsupportedInClause(child, `${path}/${key}`));
}

interface Define {
  path: string;
  define: Record<string, Json>;
}

const definesOf = (clauses: readonly Json[]): Define[] =>
  clauses.flatMap((clause, index) => (isObject(clause) && isObject(clause.define) ? [{ path: `/clauses/${index}/define`, define: clause.define }] : []));

/** The defines' shared utilities, checked once every define is read: by language, then id, the first define of an id winning. */
function unsupportedShared(defines: readonly Define[]): Found {
  const utils = new Map<string, { language: CodeRuleAstLanguage; at: string; util: Json }>();
  for (const { path, define } of defines) {
    const language = astLanguage(define);
    for (const [id, util] of language === undefined ? [] : entriesOf(define.utils)) {
      const key = `${language}\u0000${id}`;
      if (language !== undefined && !utils.has(key)) utils.set(key, { language, at: `${path}/utils/${id}`, util });
    }
  }
  return first([...utils].sort(byKey), ([, { language, at, util }]) => unsupportedPredicate(util, at, language));
}

/**
 * Why a code-rule document is not supported yet, or undefined when the engine reads every language and capability it
 * names. It reads the registry, and walks the document in the order the engine's compile does: each define's language,
 * the shared utilities, then each clause's queries. So the reason is the one compiling gives.
 */
export function codeRuleLanguageNotSupported(document: Json): Found {
  const clauses: readonly Json[] = isObject(document) && Array.isArray(document.clauses) ? document.clauses : [];
  const defines = definesOf(clauses);
  return first(defines, ({ path, define }) => plannedLanguage(define, path))
    ?? unsupportedShared(defines)
    ?? first(clauses.entries(), ([index, clause]) => (isObject(clause) && clause.define === undefined ? unsupportedInClause(clause, `/clauses/${index}`) : undefined));
}
