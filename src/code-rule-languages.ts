import {
  CODE_RULE_SEMANTIC_CAPABILITIES,
  CODE_RULE_SUPPRESSIONS,
  codeRuleLanguageImplements as semanticLanguageImplements,
  codeRuleLanguageSuppressions as semanticLanguageSuppressions,
} from "./code-rule-languages-semantic.js";
import type { CodeRuleSemanticCapability, CodeRuleSuppression } from "./code-rule-languages-semantic.js";

export { CODE_RULE_SEMANTIC_CAPABILITIES, CODE_RULE_SUPPRESSIONS };
export type { CodeRuleSemanticCapability, CodeRuleSuppression };

export const CODE_RULE_AST_LANGUAGES = ["python", "typescript", "javascript", "php", "blade"] as const;
export type CodeRuleAstLanguage = (typeof CODE_RULE_AST_LANGUAGES)[number];
export const CODE_RULE_LANGUAGE_FAMILIES = ["python", "ecma", "php", "blade"] as const;
export type CodeRuleLanguageFamily = (typeof CODE_RULE_LANGUAGE_FAMILIES)[number];

export type CodeRuleDialectLabel = "python" | "typescript" | "tsx" | "javascript" | "php" | "blade";

type CodeRuleDialect = {
  label: CodeRuleDialectLabel;
  extensions: readonly string[];
  family: CodeRuleLanguageFamily;
};

export const CODE_RULE_DIALECTS: Readonly<Record<CodeRuleDialectLabel, CodeRuleDialect>> = {
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

const CODE_RULE_LANGUAGES: Readonly<Record<CodeRuleAstLanguage, { dialects: readonly CodeRuleDialectLabel[]; family: CodeRuleLanguageFamily }>> = {
  python: { dialects: ["python"], family: "python" },
  typescript: { dialects: ["typescript", "tsx", "javascript"], family: "ecma" },
  javascript: { dialects: ["javascript"], family: "ecma" },
  php: { dialects: ["php"], family: "php" },
  blade: { dialects: ["blade"], family: "blade" },
};


export const codeRuleLanguageFamily = (language: CodeRuleAstLanguage): CodeRuleLanguageFamily => CODE_RULE_LANGUAGES[language].family;
export const codeRuleLanguageDialects = (language: CodeRuleAstLanguage): readonly CodeRuleDialectLabel[] => CODE_RULE_LANGUAGES[language].dialects;
export const codeRuleDialectFamily = (dialect: CodeRuleDialectLabel): CodeRuleLanguageFamily => CODE_RULE_DIALECTS[dialect].family;
/** Each dialect extension and the dialect it selects, for `languageOfPath`. */
export const CODE_RULE_DIALECT_SUFFIXES: ReadonlyMap<string, CodeRuleDialectLabel> = new Map(
  (Object.values(CODE_RULE_DIALECTS) as CodeRuleDialect[]).flatMap(dialect => dialect.extensions.map(extension => [extension, dialect.label] as const)),
);
/** The dialect a path routes to, as the host's walk and the native repository index route it (see `languageOfPath`). */
export const codeRuleDialectForPath = (path: string): CodeRuleDialectLabel | undefined => languageOfPath(CODE_RULE_DIALECT_SUFFIXES, path);
export const codeRuleLanguageImplements = (language: CodeRuleAstLanguage, capability: CodeRuleSemanticCapability): boolean =>
  semanticLanguageImplements(language, codeRuleLanguageFamily(language), capability);
export const codeRuleLanguageSuppressions = (language: CodeRuleAstLanguage): readonly CodeRuleSuppression[] =>
  semanticLanguageSuppressions(codeRuleLanguageFamily(language));
export const codeRuleLanguageUsesFamily = (language: CodeRuleAstLanguage, family: CodeRuleLanguageFamily): boolean => codeRuleLanguageFamily(language) === family;
