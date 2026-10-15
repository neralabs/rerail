import type { CodeRuleSourceLanguage } from "./code-rule.js";
import {
  LANGUAGE_REGISTRY,
  CODE_RULE_AST_LANGUAGES,
  CODE_RULE_LANGUAGE_FAMILIES,
  CODE_RULE_LANGUAGE_IDS,
  CODE_RULE_PLANNED_LANGUAGES,
  CODE_RULE_SEMANTIC_CAPABILITIES,
  CODE_RULE_SUPPRESSIONS,
} from "./code-rule-language-registry.generated.js";

/**
 * The languages the host knows, all read from the language registry (`languages/*.json`, docs/language-packs.md)
 * through its generated module. The engine generates its own tables from the same files. Nothing here names a
 * language: a language is added to the registry, never to this file.
 */

export { LANGUAGE_REGISTRY, CODE_RULE_AST_LANGUAGES, CODE_RULE_LANGUAGE_FAMILIES, CODE_RULE_LANGUAGE_IDS, CODE_RULE_PLANNED_LANGUAGES, CODE_RULE_SEMANTIC_CAPABILITIES, CODE_RULE_SUPPRESSIONS };

/** Every language a rule may name. */
export type CodeRuleLanguageId = (typeof CODE_RULE_LANGUAGE_IDS)[number];
/** A language the engine reads. */
export type CodeRuleAstLanguage = (typeof CODE_RULE_AST_LANGUAGES)[number];
/** A language a rule may name that the engine does not read yet. */
export type CodeRulePlannedLanguage = (typeof CODE_RULE_PLANNED_LANGUAGES)[number];
export type CodeRuleLanguageFamily = (typeof CODE_RULE_LANGUAGE_FAMILIES)[number];
export type CodeRuleSemanticCapability = (typeof CODE_RULE_SEMANTIC_CAPABILITIES)[number];
export type CodeRuleSuppression = (typeof CODE_RULE_SUPPRESSIONS)[number];
/** Whether a language reads a capability: `refused` rules are invalid, `not-supported-yet` rules are kept inactive. */
export type CodeRuleCapabilitySupport = "supported" | "refused" | "not-supported-yet";

type RegistryEntry = (typeof LANGUAGE_REGISTRY)[number];
type SupportedEntry = Extract<RegistryEntry, { status: "supported" }>;
export type CodeRuleDialectLabel = SupportedEntry["dialects"][number]["label"];

interface CodeRuleDialect {
  label: CodeRuleDialectLabel;
  suffixes: readonly string[];
  family: CodeRuleLanguageFamily;
}

const SUPPORTED: readonly SupportedEntry[] = LANGUAGE_REGISTRY.filter((entry): entry is SupportedEntry => entry.status === "supported");
const ENTRIES = Object.fromEntries(LANGUAGE_REGISTRY.map(entry => [entry.id, entry])) as Readonly<Record<CodeRuleLanguageId, RegistryEntry>>;
/** Each language's status, typed as every status the registry format allows: a registry may hold no planned language. */
const STATUSES: Readonly<Record<string, "supported" | "planned">> = Object.fromEntries(LANGUAGE_REGISTRY.map(entry => [entry.id, entry.status]));

/** Each supported dialect: its file label, the file suffixes that select it, and the family it is read with. */
export const CODE_RULE_DIALECTS = Object.fromEntries(
  SUPPORTED.flatMap(entry =>
    entry.dialects.map((dialect): [CodeRuleDialectLabel, CodeRuleDialect] => [dialect.label, { label: dialect.label, suffixes: dialect.suffixes, family: entry.family }]),
  ),
) as Readonly<Record<CodeRuleDialectLabel, CodeRuleDialect>>;

/**
 * The language `languages` maps a path's basename to: its longest listed suffix (`.blade.php` before `.php`),
 * keys lowercase with their dot and the basename lowercased in ASCII only. The basename must be longer than the
 * suffix: a file named only by one (`.php`) has none, as `extname` reads it. The native repository index
 * (`WorkspaceIndexTask::source_language`) applies the same rule to the same map.
 */
export function languageOfPath<T>(languages: ReadonlyMap<string, T>, path: string): T | undefined {
  const name = path.slice(path.lastIndexOf("/") + 1).replace(/[A-Z]+/g, upper => upper.toLowerCase());
  let found: { suffix: string; language: T } | undefined;
  for (const [suffix, language] of languages) {
    if (name.length > suffix.length && name.endsWith(suffix) && (found === undefined || suffix.length > found.suffix.length)) {
      found = { suffix, language };
    }
  }
  return found?.language;
}

export const isCodeRuleAstLanguage = (language: string): language is CodeRuleAstLanguage => ENTRIES[language as CodeRuleLanguageId]?.status === "supported";
export const isCodeRulePlannedLanguage = (language: string): language is CodeRulePlannedLanguage => STATUSES[language] === "planned";
export const codeRuleLanguageFamily = (language: CodeRuleAstLanguage): CodeRuleLanguageFamily => ENTRIES[language].family as CodeRuleLanguageFamily;
/** The dialects a language's rules run on, the first being the one its own text is read in. */
export const codeRuleLanguageDialects = (language: CodeRuleAstLanguage): readonly CodeRuleDialectLabel[] => {
  const entry = ENTRIES[language] as SupportedEntry;
  return "runsOn" in entry ? entry.runsOn : entry.dialects.map(dialect => dialect.label);
};
export const codeRuleDialectFamily = (dialect: CodeRuleDialectLabel): CodeRuleLanguageFamily => CODE_RULE_DIALECTS[dialect].family;
/** Each supported dialect suffix and the dialect it selects, for `languageOfPath`. Planned languages route nothing. */
export const CODE_RULE_DIALECT_SUFFIXES: ReadonlyMap<string, CodeRuleDialectLabel> = new Map(
  Object.values(CODE_RULE_DIALECTS).flatMap(dialect => dialect.suffixes.map(suffix => [suffix, dialect.label] as const)),
);
/** The dialect a path routes to, as the host's walk and the native repository index route it (see `languageOfPath`). */
export const codeRuleDialectForPath = (path: string): CodeRuleDialectLabel | undefined => languageOfPath(CODE_RULE_DIALECT_SUFFIXES, path);
/** What a language's registry entry declares for a capability. */
export const codeRuleLanguageCapability = (language: CodeRuleLanguageId, capability: CodeRuleSemanticCapability): CodeRuleCapabilitySupport =>
  ENTRIES[language].capabilities[capability];
export const codeRuleLanguageImplements = (language: CodeRuleAstLanguage, capability: CodeRuleSemanticCapability): boolean =>
  codeRuleLanguageCapability(language, capability) === "supported";
export const codeRuleLanguageSuppressions = (language: CodeRuleAstLanguage): readonly CodeRuleSuppression[] => ENTRIES[language].suppressions as readonly CodeRuleSuppression[];

/** Files audits read as text only (regex rules over their content): formats that are no registry language. */
const TEXT_SOURCE_SUFFIXES: ReadonlyMap<string, CodeRuleSourceLanguage> = new Map<string, CodeRuleSourceLanguage>([
  [".json", "json"],
  [".yaml", "yaml"],
  [".yml", "yaml"],
  [".toml", "toml"],
  [".md", "markdown"],
  [".mdx", "markdown"],
  [".rst", "text"],
  [".txt", "text"],
  [".sh", "text"],
  [".bash", "text"],
  [".zsh", "text"],
  [".html", "text"],
  [".css", "text"],
  [".scss", "text"],
]);

/** Each planned language's suffixes: their files are read as text until the engine reads the language. */
const PLANNED_SUFFIXES: readonly string[] = LANGUAGE_REGISTRY.flatMap(entry => (STATUSES[entry.id] === "planned" ? entry.dialects.flatMap(dialect => dialect.suffixes) : []));

/**
 * The suffixes audits and the bridge read and the file label each selects: every supported dialect's, each planned
 * language's (as text), and the other text files'. A supported dialect's suffix wins over a text one. The native
 * repository index receives the same map.
 */
export const SOURCE_LANGUAGES: ReadonlyMap<string, CodeRuleSourceLanguage> = new Map<string, CodeRuleSourceLanguage>([
  ...CODE_RULE_DIALECT_SUFFIXES,
  ...[...PLANNED_SUFFIXES.map(suffix => [suffix, "text"] as const), ...TEXT_SOURCE_SUFFIXES].filter(([suffix]) => !CODE_RULE_DIALECT_SUFFIXES.has(suffix)),
]);
