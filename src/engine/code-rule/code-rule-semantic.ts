export type {
  CodeRuleDslArguments,
  CodeRuleDslAstWhen,
  CodeRuleDslBindingFilter,
  CodeRuleDslBinds,
  CodeRuleDslCalls,
  CodeRuleDslFileCondition,
  CodeRuleDslFormatArgs,
  CodeRuleDslLanguageOptions,
  CodeRuleDslLayout,
  CodeRuleDslParse,
  CodeRuleDslResolves,
  CodeRuleDslValueTest,
} from "./code-rule.js";
import type {
  CodeRuleDslArguments,
  CodeRuleDslAstRule,
  CodeRuleDslBinds,
  CodeRuleDslCalls,
  CodeRuleDslFormatArgs,
  CodeRuleDslLayout,
  CodeRuleDslParse,
  CodeRuleDslResolves,
  CodeRuleDslValueTest,
} from "./code-rule.js";

export interface CodeRuleDslSuppress {
  eslint?: { rules: string[] };
  noqa?: { codes: string[]; parent?: "statement" };
  /** Bandit's `# nosec` comments: a bare one, or one naming one of `codes` (or its test name). */
  nosec?: { codes: string[] };
  /** Oxlint's `oxlint-disable` (and `eslint-disable`) comments, as oxlint asks them: `rules` are its own rule names, without a plugin. */
  oxlint?: { rules: string[] };
  /** Biome's `biome-ignore` comments, as Biome 2.5.15 reads them: `rules` are its lint rules, `lint/<group>/<rule>`. */
  biome?: { rules: string[] };
  phpstan?: { identifiers: string[] };
  /** Psalm's `@psalm-suppress` tags; `suppressAll` honours `@psalm-suppress all` (`disableSuppressAll="false"`). */
  psalm?: { issues: string[]; suppressAll?: boolean };
  /** PHP_CodeSniffer's `phpcs:` annotations, as version 4 reads them unless `version: 3` (which adds `@codingStandardsIgnore*`). */
  phpcs?: { sniffs: string[]; version?: 3 | 4 };
}

export type CodeRuleDslSemanticCompareText =
  | "value"
  | "key"
  | "member"
  | "name"
  | "reference"
  | "reference-no-static-keys"
  | "structure"
  | "structure-hash"
  | "structure-parts";

export interface CodeRuleDslSemanticAstRuleFields {
  resolves?: CodeRuleDslResolves;
  value?: CodeRuleDslValueTest;
  arguments?: CodeRuleDslArguments;
  binds?: CodeRuleDslBinds;
  parse?: CodeRuleDslParse;
  capture?: string;
  formatArgs?: CodeRuleDslFormatArgs;
  layout?: CodeRuleDslLayout;
  host?: CodeRuleDslAstRule;
  calls?: CodeRuleDslCalls;
}
