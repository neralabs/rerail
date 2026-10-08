import type { CodeRuleAstLanguage, CodeRuleLanguageFamily } from "./code-rule-languages.js";

export const CODE_RULE_SEMANTIC_CAPABILITIES = ["scope", "value", "arguments", "calls", "formatArgs"] as const;
export type CodeRuleSemanticCapability = (typeof CODE_RULE_SEMANTIC_CAPABILITIES)[number];

export const CODE_RULE_SUPPRESSIONS = ["noqa", "eslint", "phpstan", "psalm", "phpcs"] as const;
export type CodeRuleSuppression = (typeof CODE_RULE_SUPPRESSIONS)[number];

const FAMILY_CAPABILITIES: Readonly<Record<CodeRuleLanguageFamily, Readonly<Partial<Record<CodeRuleSemanticCapability, true>>>>> = {
  python: { scope: true, value: true, arguments: true, calls: true, formatArgs: true },
  ecma: { scope: true, value: true, arguments: true },
  php: { scope: true, value: true, arguments: true, calls: true, formatArgs: true },
  blade: { scope: true, value: true, arguments: true, calls: true, formatArgs: true },
};

const FAMILY_SUPPRESSIONS: Readonly<Record<CodeRuleLanguageFamily, readonly CodeRuleSuppression[]>> = {
  python: ["noqa"],
  ecma: ["eslint"],
  php: ["phpstan", "psalm", "phpcs"],
  blade: [],
};

export const codeRuleLanguageImplements = (language: CodeRuleAstLanguage, family: CodeRuleLanguageFamily, capability: CodeRuleSemanticCapability): boolean =>
  FAMILY_CAPABILITIES[family]?.[capability] === true;
export const codeRuleLanguageSuppressions = (family: CodeRuleLanguageFamily): readonly CodeRuleSuppression[] => FAMILY_SUPPRESSIONS[family];
