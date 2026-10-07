import type { PolicyAstLanguage, PolicyLanguageFamily } from "./policy-languages.js";

export const POLICY_SEMANTIC_CAPABILITIES = ["scope", "value", "arguments", "calls", "formatArgs"] as const;
export type PolicySemanticCapability = (typeof POLICY_SEMANTIC_CAPABILITIES)[number];

export const POLICY_SUPPRESSIONS = ["noqa", "eslint", "phpstan", "psalm", "phpcs"] as const;
export type PolicySuppression = (typeof POLICY_SUPPRESSIONS)[number];

const FAMILY_CAPABILITIES: Readonly<Record<PolicyLanguageFamily, Readonly<Partial<Record<PolicySemanticCapability, true>>>>> = {
  python: { scope: true, value: true, arguments: true, calls: true, formatArgs: true },
  ecma: { scope: true, value: true, arguments: true },
  php: { scope: true, value: true, arguments: true, calls: true, formatArgs: true },
  blade: { scope: true, value: true, arguments: true, calls: true, formatArgs: true },
};

const FAMILY_SUPPRESSIONS: Readonly<Record<PolicyLanguageFamily, readonly PolicySuppression[]>> = {
  python: ["noqa"],
  ecma: ["eslint"],
  php: ["phpstan", "psalm", "phpcs"],
  blade: [],
};

export const policyLanguageImplements = (language: PolicyAstLanguage, family: PolicyLanguageFamily, capability: PolicySemanticCapability): boolean =>
  FAMILY_CAPABILITIES[family]?.[capability] === true;
export const policyLanguageSuppressions = (family: PolicyLanguageFamily): readonly PolicySuppression[] => FAMILY_SUPPRESSIONS[family];
