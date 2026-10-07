export type {
  PolicyDslArguments,
  PolicyDslAstWhen,
  PolicyDslBindingFilter,
  PolicyDslBinds,
  PolicyDslCalls,
  PolicyDslFileCondition,
  PolicyDslFormatArgs,
  PolicyDslLanguageOptions,
  PolicyDslLayout,
  PolicyDslParse,
  PolicyDslResolves,
  PolicyDslValueTest,
} from "./executable-policy.js";
import type {
  PolicyDslArguments,
  PolicyDslAstRule,
  PolicyDslBinds,
  PolicyDslCalls,
  PolicyDslFormatArgs,
  PolicyDslLayout,
  PolicyDslParse,
  PolicyDslResolves,
  PolicyDslValueTest,
} from "./executable-policy.js";

export interface PolicyDslSuppress {
  eslint?: { rules: string[] };
  noqa?: { codes: string[]; parent?: "statement" };
  phpstan?: { identifiers: string[] };
  /** Psalm's `@psalm-suppress` tags; `suppressAll` honours `@psalm-suppress all` (`disableSuppressAll="false"`). */
  psalm?: { issues: string[]; suppressAll?: boolean };
  /** PHP_CodeSniffer's `phpcs:` annotations, as version 4 reads them unless `version: 3` (which adds `@codingStandardsIgnore*`). */
  phpcs?: { sniffs: string[]; version?: 3 | 4 };
}

export type PolicyDslSemanticCompareText =
  | "value"
  | "key"
  | "member"
  | "name"
  | "reference"
  | "reference-no-static-keys"
  | "structure"
  | "structure-hash"
  | "structure-parts";

export interface PolicyDslSemanticAstRuleFields {
  resolves?: PolicyDslResolves;
  value?: PolicyDslValueTest;
  arguments?: PolicyDslArguments;
  binds?: PolicyDslBinds;
  parse?: PolicyDslParse;
  capture?: string;
  formatArgs?: PolicyDslFormatArgs;
  layout?: PolicyDslLayout;
  host?: PolicyDslAstRule;
  calls?: PolicyDslCalls;
}
