import {
  POLICY_SEMANTIC_CAPABILITIES,
  POLICY_SUPPRESSIONS,
  policyLanguageImplements as semanticLanguageImplements,
  policyLanguageSuppressions as semanticLanguageSuppressions,
} from "./policy-languages-semantic.js";
import type { PolicySemanticCapability, PolicySuppression } from "./policy-languages-semantic.js";

export { POLICY_SEMANTIC_CAPABILITIES, POLICY_SUPPRESSIONS };
export type { PolicySemanticCapability, PolicySuppression };

export const POLICY_AST_LANGUAGES = ["python", "typescript", "javascript", "php", "blade"] as const;
export type PolicyAstLanguage = (typeof POLICY_AST_LANGUAGES)[number];
export const POLICY_LANGUAGE_FAMILIES = ["python", "ecma", "php", "blade"] as const;
export type PolicyLanguageFamily = (typeof POLICY_LANGUAGE_FAMILIES)[number];

export type PolicyDialectLabel = "python" | "typescript" | "tsx" | "javascript" | "php" | "blade";

type PolicyDialect = {
  label: PolicyDialectLabel;
  extensions: readonly string[];
  family: PolicyLanguageFamily;
};

export const POLICY_DIALECTS: Readonly<Record<PolicyDialectLabel, PolicyDialect>> = {
  python: { label: "python", extensions: [".py", ".pyi"], family: "python" },
  typescript: { label: "typescript", extensions: [".ts", ".mts", ".cts"], family: "ecma" },
  tsx: { label: "tsx", extensions: [".tsx"], family: "ecma" },
  javascript: { label: "javascript", extensions: [".js", ".jsx", ".mjs", ".cjs"], family: "ecma" },
  php: { label: "php", extensions: [".php"], family: "php" },
  blade: { label: "blade", extensions: [".blade.php"], family: "blade" },
};

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

const POLICY_LANGUAGES: Readonly<Record<PolicyAstLanguage, { dialects: readonly PolicyDialectLabel[]; family: PolicyLanguageFamily }>> = {
  python: { dialects: ["python"], family: "python" },
  typescript: { dialects: ["typescript", "tsx", "javascript"], family: "ecma" },
  javascript: { dialects: ["javascript"], family: "ecma" },
  php: { dialects: ["php"], family: "php" },
  blade: { dialects: ["blade"], family: "blade" },
};


export const policyLanguageFamily = (language: PolicyAstLanguage): PolicyLanguageFamily => POLICY_LANGUAGES[language].family;
export const policyLanguageDialects = (language: PolicyAstLanguage): readonly PolicyDialectLabel[] => POLICY_LANGUAGES[language].dialects;
export const policyDialectFamily = (dialect: PolicyDialectLabel): PolicyLanguageFamily => POLICY_DIALECTS[dialect].family;
/** Each dialect extension and the dialect it selects, for `languageOfPath`. */
export const POLICY_DIALECT_SUFFIXES: ReadonlyMap<string, PolicyDialectLabel> = new Map(
  (Object.values(POLICY_DIALECTS) as PolicyDialect[]).flatMap(dialect => dialect.extensions.map(extension => [extension, dialect.label] as const)),
);
/** The AST dialect of `path` by `languageOfPath` over the dialects' extensions. */
export const policyDialectForPath = (path: string): PolicyDialectLabel | undefined => languageOfPath(POLICY_DIALECT_SUFFIXES, path);
export const policyLanguageImplements = (language: PolicyAstLanguage, capability: PolicySemanticCapability): boolean =>
  semanticLanguageImplements(language, policyLanguageFamily(language), capability);
export const policyLanguageSuppressions = (language: PolicyAstLanguage): readonly PolicySuppression[] =>
  semanticLanguageSuppressions(policyLanguageFamily(language));
export const policyLanguageUsesFamily = (language: PolicyAstLanguage, family: PolicyLanguageFamily): boolean => policyLanguageFamily(language) === family;
