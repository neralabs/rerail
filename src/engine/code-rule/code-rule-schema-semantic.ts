import { z } from "zod";

export const CODE_RULE_SEMANTIC_COMPARE_TEXTS = ["value", "key", "member", "name", "reference", "reference-no-static-keys", "structure", "structure-hash", "structure-parts"] as const;

const eslintRuleName = z.string().refine(name => name.length > 0 && !name.includes(",") && name.trim() === name, "Must be a rule name (non-empty, without commas or surrounding white space).");
const ruffCode = z.string().regex(/^[A-Z]+[0-9]+$/, "Must be a rule code (uppercase letters, then digits: F401).");
const banditTest = z.string().regex(/^B[0-9]+$/, "Must be a Bandit test id (B, then digits: B101).");
const phpstanIdentifier = z.string().regex(/^[A-Za-z0-9](?:[A-Za-z0-9.]*[A-Za-z0-9])?$/, "Must be a PHPStan identifier (letters or digits, with internal dots).");
const psalmIssue = z.string().regex(/^(?!all$)[A-Za-z0-9_-]+$/, "Must be a Psalm issue name (letters, digits, `_` or `-`; `all` is suppressAll).");
const phpcsCode = z
  .string()
  .regex(/^\w+\.\w+\.\w+(?:\.\w+)?$/, "Must be a PHP_CodeSniffer sniff or message code (Standard.Category.Sniff or Standard.Category.Sniff.Code).");
const clippyLint = z
  .string()
  .regex(/^[a-z0-9_]+(?:::[a-z0-9_]+)?$/, "Must be a lint name as a lint attribute spells it (clippy::name, a group such as clippy::all, or a rustc lint or group).");
const rustVersion = z.string().regex(/^[0-9]+\.[0-9]+(?:\.[0-9]+)?$/, "Must be a Rust version, major.minor or major.minor.patch (1.53).");
const sqlfluffReference = z
  .string()
  .refine(name => name.length > 0 && !name.includes(",") && name.trim() === name, "Must be a rule's code, name, alias or group (non-empty, without commas or surrounding white space).");

/**
 * A violation's `suppress`: the suppression kinds it may ask for, one entry per linter, in the engine's order (its table in
 * `native/src/code_rule_dsl/suppress.rs`): the key is the kind's `suppress` key and registry name, the value its shape.
 * Which languages honour a kind is the registry's (`suppressions`) and the engine's to check. A new kind is one entry here.
 */
export const codeRuleSuppressSchema = z
  .object({
    /** ESLint's `eslint-disable` comments. */
    eslint: z.object({ rules: z.array(eslintRuleName).min(1).max(256) }).strict().optional(),
    /** Ruff's `# noqa` comments; `parent: "statement"` also reads them where the enclosing statement starts. */
    noqa: z.object({ codes: z.array(ruffCode).min(1).max(256), parent: z.literal("statement").optional() }).strict().optional(),
    /** Bandit's `# nosec` comments: a bare one, or one naming one of `codes` (or its test name). */
    nosec: z.object({ codes: z.array(banditTest).min(1).max(256) }).strict().optional(),
    /** PHPStan's `@phpstan-ignore` comments. */
    phpstan: z.object({ identifiers: z.array(phpstanIdentifier).min(1).max(256) }).strict().optional(),
    /** Psalm's `@psalm-suppress` tags; `suppressAll` honours `@psalm-suppress all` (`disableSuppressAll="false"`). */
    psalm: z.object({ issues: z.array(psalmIssue).min(1).max(256), suppressAll: z.boolean().optional() }).strict().optional(),
    /** PHP_CodeSniffer's `phpcs:` annotations, as version 4 reads them unless `version: 3` (which adds `@codingStandardsIgnore*`). */
    phpcs: z.object({ sniffs: z.array(phpcsCode).min(1).max(256), version: z.union([z.literal(3), z.literal(4)]).optional() }).strict().optional(),
    /**
     * rustc's and Clippy's lint-level attributes, `#[allow(..)]` and `#[expect(..)]`, naming one of `lints` (a lint or its group);
     * with `msrv`, the Rust version the lint needs, also a `#[clippy::msrv = ".."]` in force naming an older one.
     */
    clippy: z.object({ lints: z.array(clippyLint).min(1).max(256), msrv: rustVersion.optional() }).strict().optional(),
    /** SQLFluff's `-- noqa` comments: `rules` are the references SQLFluff knows the rule by (its code, name, aliases and groups), which a directive's code, name or glob may match. */
    sqlfluff: z.object({ rules: z.array(sqlfluffReference).min(1).max(256) }).strict().optional(),
  })
  .strict()
  .refine(suppress => Object.values(suppress).some(value => value !== undefined), { error: () => `suppress must hold ${suppressionKinds()}.` });

/** `eslint, noqa, ..., or phpcs`: the kinds a `suppress` may hold, in order. */
function suppressionKinds(): string {
  const keys = Object.keys(codeRuleSuppressSchema.shape);
  return `${keys.slice(0, -1).join(", ")}, or ${keys.at(-1)}`;
}

/** A violation's `suppress`: per kind, the suppressions it honours. */
export type CodeRuleDslSuppress = z.infer<typeof codeRuleSuppressSchema>;

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
