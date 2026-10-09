import { z } from "zod";
import type { CodeRuleDslSuppress } from "./code-rule-semantic.js";

export const CODE_RULE_SEMANTIC_COMPARE_TEXTS = ["value", "key", "member", "name", "reference", "reference-no-static-keys", "structure", "structure-hash", "structure-parts"] as const;

const eslintRuleName = z.string().refine(name => name.length > 0 && !name.includes(",") && name.trim() === name, "Must be a rule name (non-empty, without commas or surrounding white space).");
const ruffCode = z.string().regex(/^[A-Z]+[0-9]+$/, "Must be a rule code (uppercase letters, then digits: F401).");
const phpstanIdentifier = z.string().regex(/^[A-Za-z0-9](?:[A-Za-z0-9.]*[A-Za-z0-9])?$/, "Must be a PHPStan identifier (letters or digits, with internal dots).");
const psalmIssue = z.string().regex(/^(?!all$)[A-Za-z0-9_-]+$/, "Must be a Psalm issue name (letters, digits, `_` or `-`; `all` is suppressAll).");
const phpcsCode = z
  .string()
  .regex(/^\w+\.\w+\.\w+(?:\.\w+)?$/, "Must be a PHP_CodeSniffer sniff or message code (Standard.Category.Sniff or Standard.Category.Sniff.Code).");
export const codeRuleSuppressSchema = z
  .object({
    eslint: z.object({ rules: z.array(eslintRuleName).min(1).max(256) }).strict().optional(),
    noqa: z.object({ codes: z.array(ruffCode).min(1).max(256), parent: z.literal("statement").optional() }).strict().optional(),
    phpstan: z.object({ identifiers: z.array(phpstanIdentifier).min(1).max(256) }).strict().optional(),
    psalm: z.object({ issues: z.array(psalmIssue).min(1).max(256), suppressAll: z.boolean().optional() }).strict().optional(),
    phpcs: z.object({ sniffs: z.array(phpcsCode).min(1).max(256), version: z.union([z.literal(3), z.literal(4)]).optional() }).strict().optional(),
  })
  .strict()
  .refine(
    suppress =>
      suppress.eslint !== undefined ||
      suppress.noqa !== undefined ||
      suppress.phpstan !== undefined ||
      suppress.psalm !== undefined ||
      suppress.phpcs !== undefined,
    "suppress must hold eslint, noqa, phpstan, psalm, or phpcs.",
  ) as unknown as z.ZodType<CodeRuleDslSuppress>;

/** Semantic matcher fields, inserted in the structural shape at their original key position. */
export function semanticRuleShape(fields: {
  resolves: z.ZodType;
  value: z.ZodType;
  arguments: z.ZodType;
  binds: z.ZodType;
  parse: z.ZodType;
  capture: z.ZodType;
  formatArgs: z.ZodType;
  layout: z.ZodType;
  host: z.ZodType;
  calls: z.ZodType;
}) {
  return {
    resolves: fields.resolves.optional(),
    value: fields.value.optional(),
    arguments: fields.arguments.optional(),
    binds: fields.binds.optional(),
    parse: fields.parse.optional(),
    capture: fields.capture.optional(),
    formatArgs: fields.formatArgs.optional(),
    layout: fields.layout.optional(),
    host: fields.host.optional(),
    calls: fields.calls.optional(),
  };
}
export function semanticSchemaComponents(parse: z.ZodType, formatArgs: z.ZodType): Array<[z.ZodType, string]> {
  return [[parse, "PolicyParse"], [formatArgs, "PolicyFormatArgs"]];
}
