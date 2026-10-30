export type {
  CodeRuleDslArguments,
  CodeRuleDslAstWhen,
  CodeRuleDslBindingFilter,
  CodeRuleDslBinds,
  CodeRuleDslCalls,
  CodeRuleDslFileCondition,
  CodeRuleDslFormatArgs,
  CodeRuleDslImportTarget,
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
  CodeRuleDslImportTarget,
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
  /**
   * Inside `parse`: a rule of the host language matched on the literal the parse holds. In a rule over an embedded
   * program (a query with `ast.embedded`, or under `embedded`), outside a `parse`: a rule matched on the innermost
   * node of the host document covering the guest node's host span. The innermost of the two contexts decides. Either
   * way it binds nothing: it runs in the host environment as the `parse` (or the enclosing `embedded`; a fresh one
   * under `ast.embedded`) began, plus the inner rule's captures bound so far (text and file span), so a `compare`
   * inside it may compare a host capture with an inner one.
   */
  host?: CodeRuleDslAstRule;
  /**
   * From a node of a host document whose language embeds another (Blade's PHP, Vue's script program): some node of
   * the file's embedded program whose host span lies within the node's range matches the rule. The rule runs in a
   * fresh environment (the host's captures are not visible in it, as in `parse`); the captures it binds are exported
   * into the host rule's environment, spanning the file bytes their nodes stand for. Only in host context: a query
   * without `ast.embedded`, or under a `host` reached from an embedded program; never inside `parse`.
   */
  embedded?: CodeRuleDslAstRule;
  calls?: CodeRuleDslCalls;
  importTarget?: CodeRuleDslImportTarget;
}
