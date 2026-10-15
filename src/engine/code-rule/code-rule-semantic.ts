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
export type { CodeRuleDslSuppress } from "./code-rule-schema-semantic.js";
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
