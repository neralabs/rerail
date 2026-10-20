import { z } from "zod";
import type {
  CodeRuleDslAstCount,
  CodeRuleDslAstRelation,
  CodeRuleDslCompare,
  CodeRuleDslAstRule,
  CodeRuleDslAstQuery,
  CodeRuleDslClause,
  CodeRuleDslDefine,
  CodeRuleDslReportingClause,
  CodeRuleDslDocument,
  CodeRuleDslEvidenceQuery,
  CodeRuleDslRelation,
  CodeRuleDslViolationSpec,
  CodeRuleDslNthChild,
  CodeRuleDslOptionCondition,
  CodeRuleDslOptionDeclaration,
  CodeRuleDslTransformation,
  CodeRuleDslUtility,
  CodeRuleDslPredicate,
  CodeRuleDslQuery,
  CodeRuleDslScope,
  CodeRuleDslValue,
} from "./code-rule.js";
import { RULE_MODES } from "../catalog/rule-mode.js";
import type {
  CodeRuleDslArguments,
  CodeRuleDslAstWhen,
  CodeRuleDslBinds,
  CodeRuleDslCalls,
  CodeRuleDslFileCondition,
  CodeRuleDslFormatArgs,
  CodeRuleDslLanguageOptions,
  CodeRuleDslLayout,
  CodeRuleDslParse,
  CodeRuleDslResolves,
  CodeRuleDslValueTest,
} from "./code-rule-semantic.js";
import {
  CODE_RULE_SEMANTIC_COMPARE_TEXTS,
  codeRuleSuppressSchema,
  semanticRuleShape,
} from "./code-rule-schema-semantic.js";
import { isCodeRuleOptionName, CodeRuleOptionError, codeRuleOptionConditionHolds, codeRuleOptionValues, resolveCodeRuleOptions } from "./code-rule-options.js";
import { CODE_RULE_AST_LANGUAGES, CODE_RULE_LANGUAGE_IDS, isCodeRuleAstLanguage } from "./code-rule-languages.js";
import type { CodeRuleAstLanguage } from "./code-rule-languages.js";

export { CODE_RULE_AST_LANGUAGES };
export type { CodeRuleAstLanguage };


/**
 * Zod for `rerail.policy/v1`, the code-rule document the native engine
 * compiles and evaluates. The TypeScript types in `code-rule.ts` are
 * the contract; every schema here is declared against them so a drift between
 * the two is a compile error. This is the shape the `rules` table stores for
 * `kind = 'code'` rows; every top-level field maps to a column (`schema.ts`).
 *
 * ast-grep patterns and fact relations are checked structurally only; the
 * engine remains the authority on whether a pattern parses for its grammar, a
 * relation exists in the workspace capabilities, or a pattern fits the
 * capabilities' `maxAstPatternBytes` / `maxRegexPatternBytes` limits, and on
 * what a rule's language lacks: every write compiles in the engine
 * (`validateCodeRule`), and every stored row read does (`readStoredCodeRule`).
 * Anything this schema rejects, the engine rejects too.
 */

export const CODE_RULE_FORMAT = "rerail.policy/v1" as const;

export const CODE_RULE_REGEX_SOURCES = ["file.content", "file.path"] as const;
export type CodeRuleRegexSource = (typeof CODE_RULE_REGEX_SOURCES)[number];
export const CLAUSE_KINDS = ["forbid", "require_each", "budget", "relational", "llm", "define"] as const;
export type ClauseKind = (typeof CLAUSE_KINDS)[number];
export const CODE_RULE_STOP_BY = ["neighbor", "end"] as const;
export const CODE_RULE_COMPARE_OPS = ["lt", "le", "eq", "ne", "gt", "ge"] as const;
export const CODE_RULE_COMPARE_COLLATIONS = ["ordinal", "fold-lower", "fold-upper", "lowercase-first", "natural", "natural-fold-lower"] as const;
export const CODE_RULE_COMPARE_TEXTS = ["source", "unquoted", ...CODE_RULE_SEMANTIC_COMPARE_TEXTS, "tokens", "node", "index"] as const;
/** The compare texts that test equality only, under the ordinal collation. */
const CODE_RULE_EQUALITY_TEXTS: readonly string[] = ["tokens", "structure", "structure-hash", "structure-parts", "node", "reference", "reference-no-static-keys"];
/** `witness.span` | `witness.start` | `witness.end` | `file.start` (byte 0 of the file) | `capture.<NAME>`; the engine checks capture names against the query. */
export const CODE_RULE_VIOLATION_ANCHOR_PATTERN = /^(?:witness\.(?:span|start|end)|file\.start|capture\.\w+)$/;
export { codeRuleSuppressSchema };


const nonEmptyString = z.string().refine(value => value.trim().length > 0, "Must be a non-empty string.");

export const codeRuleScopeSchema = z
  .object({
    include: z.array(nonEmptyString).min(1),
    exclude: z.array(nonEmptyString),
  })
  .strict() as unknown as z.ZodType<CodeRuleDslScope>;

export const codeRuleValueSchema: z.ZodType<CodeRuleDslValue> = z.lazy(() =>
  z.union([z.boolean(), z.number(), z.string(), z.null(), z.array(codeRuleValueSchema), z.record(z.string(), codeRuleValueSchema)]),
);


const codeRuleScalarSchema = z.union([z.boolean(), z.number(), z.string(), z.null()]);
export const codeRuleOptionConditionSchema: z.ZodType<CodeRuleDslOptionCondition> = z.lazy(() =>
  z.union([
    codeRuleScalarSchema,
    z.array(codeRuleScalarSchema),
    z.object({ in: z.array(codeRuleValueSchema) }).strict(),
    z.object({ includes: codeRuleScalarSchema }).strict(),
    z.object({ not: codeRuleOptionConditionSchema }).strict(),
  ]) as unknown as z.ZodType<CodeRuleDslOptionCondition>,
);
const optionName = z.string().refine(isCodeRuleOptionName, "An option's name starts with a letter or _ and holds letters, digits, _, - and .");
export const codeRuleOptionDeclarationSchema = z
  .object({
    type: z.enum(["boolean", "string", "number", "integer", "array"]),
    items: z.enum(["string", "number", "integer", "boolean"]).optional(),
    enum: z.array(codeRuleValueSchema).min(1).optional(),
    nullable: z.boolean().optional(),
    default: codeRuleValueSchema,
    description: z.string().optional(),
  })
  .strict() as unknown as z.ZodType<CodeRuleDslOptionDeclaration>;

export const codeRuleViolationSpecSchema = z
  .object({
    code: nonEmptyString,
    anchor: z.string().regex(CODE_RULE_VIOLATION_ANCHOR_PATTERN, "Anchor must be witness.span, witness.start, witness.end, file.start, or capture.<NAME>."),
    message: nonEmptyString,
    fix: z
      .union([
        z.string(),
        z
          .object({
            template: z.string(),
            range: z.string().regex(/^(?:witness\.span|capture\.\w+)$/, "A fix range is witness.span or capture.<NAME>."),
          })
          .strict(),
      ])
      .optional(),
    each: z.string().regex(/^[A-Z0-9_]+$/, "each is a metavariable name without $ (A-Z, 0-9, _).").optional(),
    suppress: codeRuleSuppressSchema.optional(),
    variables: z
      .record(z.string(), codeRuleValueSchema)
      .refine(variables => Object.keys(variables).length === 0, "Violation variables are not supported yet.")
      .optional(),
  })
  .strict() as unknown as z.ZodType<CodeRuleDslViolationSpec>;

export type CodeRuleAstRule = CodeRuleDslAstRule;
export type CodeRuleAstRelation = CodeRuleDslAstRelation;

const AST_RULE_KEYS = ["pattern", "kind", "regex", "nthChild", "all", "any", "not", "matches", "inside", "has", "precedes", "follows", "compare", "count", "resolves", "value", "arguments", "binds", "parse", "capture", "formatArgs", "layout", "filePath", "host", "calls"] as const;
/** The grammars `parse` applies and their kinds (the engine's `literal::Grammar`). */
export const CODE_RULE_PARSE_GRAMMARS = ["percentFormat", "braceFormat", "annotation", "eslintDirective", "printfFormat", "scanfFormat", "phpDoc", "rustFormat", "identifierWords", "pythonFString"] as const;
export const CODE_RULE_PARSE_KINDS: Record<(typeof CODE_RULE_PARSE_GRAMMARS)[number], readonly string[]> = {
  identifierWords: ["words", "word"],
  pythonFString: ["template", "literal", "interpolation"],
  percentFormat: ["format", "literal", "placeholder", "mapping_key", "flags", "width", "precision", "star", "length", "conversion", "error"],
  braceFormat: [
    "format",
    "literal",
    "field",
    "field_name",
    "auto",
    "index",
    "name",
    "attribute",
    "element_index",
    "conversion",
    "format_spec",
    "fill",
    "align",
    "sign",
    "alternate",
    "zero",
    "width",
    "grouping",
    "precision",
    "type",
    "spec_error",
    "error",
  ],
  // Python's kinds, which the engine checks when it compiles the rule.
  annotation: [],
  eslintDirective: ["directive", "item", "setting", "description"],
  printfFormat: ["format", "literal", "placeholder", "argnum", "flags", "width", "precision", "star", "length", "conversion", "error"],
  scanfFormat: ["format", "literal", "placeholder", "suppress", "argnum", "width", "length", "conversion", "set", "error"],
  phpDoc: [
    "php_doc",
    "php_doc_text",
    "php_doc_tag",
    "param_tag_value",
    "typeless_param_tag_value",
    "param_immediately_invoked_callable_tag_value",
    "param_later_invoked_callable_tag_value",
    "param_closure_this_tag_value",
    "pure_unless_callable_is_impure_tag_value",
    "pure_unless_parameter_is_passed_tag_value",
    "var_tag_value",
    "return_tag_value",
    "throws_tag_value",
    "mixin_tag_value",
    "require_extends_tag_value",
    "require_implements_tag_value",
    "sealed_tag_value",
    "deprecated_tag_value",
    "property_tag_value",
    "method_tag_value",
    "method_tag_value_parameter",
    "template_tag_value",
    "extends_tag_value",
    "implements_tag_value",
    "uses_tag_value",
    "type_alias_tag_value",
    "type_alias_import_tag_value",
    "assert_tag_value",
    "assert_tag_property_value",
    "assert_tag_method_value",
    "self_out_tag_value",
    "param_out_tag_value",
    "generic_tag_value",
    "invalid_tag_value",
    "doctrine_tag_value",
    "doctrine_annotation",
    "doctrine_argument",
    "doctrine_array",
    "doctrine_array_item",
    "identifier_type",
    "this_type",
    "nullable_type",
    "union_type",
    "intersection_type",
    "generic_type",
    "array_type",
    "array_shape",
    "array_shape_item",
    "array_shape_unsealed_type",
    "object_shape",
    "object_shape_item",
    "callable_type",
    "callable_type_parameter",
    "conditional_type",
    "conditional_type_for_parameter",
    "const_type",
    "offset_access_type",
    "invalid_type",
    "const_expr_integer",
    "const_expr_float",
    "const_expr_string",
    "const_expr_true",
    "const_expr_false",
    "const_expr_null",
    "const_expr_array",
    "const_expr_array_item",
    "const_fetch",
    "doctrine_const_expr_string",
    "name",
    "variable",
    "description",
  ],
  rustFormat: ["format", "literal", "field", "auto", "index", "name", "format_spec", "type", "error"],
};
/** The keys a rule over a parsed tree may use; one over an annotation, whose nodes are Python's, also these. */
const PARSE_RULE_KEYS: readonly string[] = ["kind", "regex", "all", "any", "not", "inside", "has", "precedes", "follows", "count", "compare", "value", "capture", "host"];
const ANNOTATION_RULE_KEYS: readonly string[] = ["pattern", "matches", "resolves", "parse"];
/** The compare texts a rule over a parsed tree may use. */
const PARSE_COMPARE_TEXTS: readonly string[] = ["source", "unquoted", "value", "node"];
const AST_RELATION_KEYS = ["inside", "has", "precedes", "follows"] as const;
/** ast-grep reserves these characters in utility ids for parameterized utility calls. */
const utilityId = nonEmptyString.refine(id => !/[(),=]/.test(id), "Utility ids must not contain (, ), comma, or =.");
const captureArgument = z.string().regex(/^\$[A-Z0-9_]+$/, "A parameter names a capture: $NAME (A-Z, 0-9, _).");
const matchesField = z
  .union(
    [
      nonEmptyString,
      z
        .record(utilityId, z.record(z.string(), captureArgument))
        .refine(call => Object.keys(call).length === 1, 'matches must call one utility: { "id": { "PARAM": "$CAPTURE" } }.'),
    ],
    { error: 'matches must be the id of a utility rule or a call { "id": { "PARAM": "$CAPTURE" } }.' },
  )
  .optional();

const metavariableName = z.string().regex(/^[A-Z0-9_]+$/, "Must be a metavariable name without $ (A-Z, 0-9, _).");
/** What an equality comparison of `text` compares, as its collation error names it. */
function equalitySubject(text: string): string {
  if (text === "tokens") return "tokens";
  if (text === "node") return "nodes";
  if (text.startsWith("reference")) return "references";
  return "structures";
}

export const codeRuleCompareSchema = z
  .object({
    left: metavariableName,
    op: z.enum(CODE_RULE_COMPARE_OPS),
    right: metavariableName,
    collation: z.enum(CODE_RULE_COMPARE_COLLATIONS).optional(),
    text: z.enum(CODE_RULE_COMPARE_TEXTS).optional(),
    commute: z.array(nonEmptyString).min(1).optional(),
  })
  .strict()
  .superRefine((compare, context) => {
    if (compare.commute !== undefined && compare.text !== "tokens") {
      context.addIssue({ code: "custom", path: ["commute"], message: "commute applies to text tokens only." });
    }
    if (compare.text === "index" && (compare.collation ?? "ordinal") !== "ordinal") {
      context.addIssue({ code: "custom", path: ["collation"], message: "index compares positions exactly; use ordinal." });
    }
    if (compare.text === undefined || !CODE_RULE_EQUALITY_TEXTS.includes(compare.text)) return;
    if (compare.op !== "eq" && compare.op !== "ne") {
      context.addIssue({ code: "custom", path: ["op"], message: `${compare.text} compares for equality only; use eq or ne.` });
    }
    if ((compare.collation ?? "ordinal") !== "ordinal") {
      const what = equalitySubject(compare.text);
      context.addIssue({ code: "custom", path: ["collation"], message: `${compare.text} compares ${what} exactly; use ordinal.` });
    }
  }) as unknown as z.ZodType<CodeRuleDslCompare>;

type NthChildState = "initial" | "n" | "sign" | "num";
type NthChildToken = "sign" | "digit" | "n";

function nthChildToken(char: string): NthChildToken | undefined {
  if (char === "+" || char === "-") return "sign";
  if (char >= "0" && char <= "9") return "digit";
  if (char === "n" || char === "N") return "n";
  return undefined;
}

const NTH_CHILD_START: Record<NthChildToken, NthChildState> = { sign: "sign", digit: "num", n: "n" };

/** The state after `token`, or undefined when `parse_an_b` refuses it there. */
function nthChildTransition(state: NthChildState, token: NthChildToken, hasN: boolean): NthChildState | undefined {
  switch (state) {
    case "initial":
      return NTH_CHILD_START[token];
    case "sign":
      if (token === "sign" || (token === "n" && hasN)) return undefined;
      return token === "digit" ? "num" : "n";
    case "num":
      if (token === "digit") return "num";
      return token === "n" && !hasN ? "n" : undefined;
    case "n":
      return token === "sign" ? "sign" : undefined;
  }
}

/** Why an `nthChild` position is refused, as ast-grep 0.45.3's `parse_an_b` refuses it; undefined when it parses. */
export function nthChildPositionError(position: string): string | undefined {
  let state: NthChildState = "initial";
  let hasN = false;
  for (const char of position.replace(/\s/gu, "")) {
    const token = nthChildToken(char);
    if (token === undefined) return `illegal character \`${char}\``;
    const next = nthChildTransition(state, token, hasN);
    if (next === undefined) return "invalid syntax";
    state = next;
    if (state === "n") hasN = true;
  }
  return state === "initial" || state === "sign" ? "invalid syntax" : undefined;
}
const nthChildPosition = z.union([
  z.number().int().nonnegative(),
  z.string().superRefine((position, context) => {
    const error = nthChildPositionError(position);
    if (error !== undefined) context.addIssue({ code: "custom", message: `nthChild: ${error}.` });
  }),
]);

const transformSource = z.string().regex(/^\$(\$\$)?[A-Z0-9_]+$/, "A transform source is a $-prefixed metavariable: $NAME or $$$NAME.");
export const codeRuleTransformationSchema = z.union([
  z
    .object({
      replace: z
        .object({ source: transformSource, default: z.string().optional(), value: z.boolean().optional(), replace: nonEmptyString, by: z.string() })
        .strict(),
    })
    .strict(),
  z
    .object({
      substring: z
        .object({
          source: transformSource,
          default: z.string().optional(),
          value: z.boolean().optional(),
          startChar: z.number().int().optional(),
          endChar: z.number().int().optional(),
        })
        .strict(),
    })
    .strict(),
  z
    .object({
      convert: z
        .object({
          source: transformSource,
          default: z.string().optional(),
          value: z.boolean().optional(),
          toCase: z.enum(["lowerCase", "upperCase", "capitalize", "camelCase", "snakeCase", "kebabCase", "pascalCase"]),
          separatedBy: z.array(z.enum(["caseChange", "dash", "dot", "slash", "space", "underscore"])).optional(),
        })
        .strict(),
    })
    .strict(),
  z
    .object({ line: z.object({ source: transformSource, default: z.string().optional(), edge: z.enum(["start", "end"]).optional() }).strict() })
    .strict(),
  z.object({ lines: z.object({ source: transformSource, default: z.string().optional() }).strict() }).strict(),
  z.object({ count: z.object({ source: transformSource, default: z.string().optional() }).strict() }).strict(),
]) as unknown as z.ZodType<CodeRuleDslTransformation>;

/** The transformation's body: its source and settings. */
function transformBody(transformation: CodeRuleDslTransformation): { source: string; value?: boolean } {
  if ("replace" in transformation) return transformation.replace;
  if ("substring" in transformation) return transformation.substring;
  if ("convert" in transformation) return transformation.convert;
  if ("line" in transformation) return transformation.line;
  if ("lines" in transformation) return transformation.lines;
  return transformation.count;
}

/** The transform variable a transformation reads, `$` sigils aside. */
function transformSourceName(transformation: CodeRuleDslTransformation): string {
  return transformBody(transformation).source.replace(/^\$+/u, "");
}

/** Whether a transformation reads its capture's node (lines, counts) rather than its text. */
const readsNode = (transformation: CodeRuleDslTransformation): boolean => "line" in transformation || "lines" in transformation || "count" in transformation;

const valueStrings = z.union([nonEmptyString, z.array(nonEmptyString).min(1).max(1024)]);
export const codeRuleValueTestSchema = z
  .object({
    eq: z.union([z.string(), z.number(), z.boolean(), z.null()]).optional(),
    type: valueStrings.optional(),
    constant: z.boolean().optional(),
    startsWith: valueStrings.optional(),
    regex: nonEmptyString.optional(),
    bitsAny: z.number().int().positive().optional(),
    gt: z.number().optional(),
    ge: z.number().optional(),
    lt: z.number().optional(),
    le: z.number().optional(),
    truthy: z.boolean().optional(),
    follow: z.boolean().optional(),
  })
  .strict()
  .refine(value => Object.keys(value).some(key => key !== "follow"), "value must hold a predicate: eq, type, constant, startsWith, regex, bitsAny, gt, ge, lt, le or truthy.") as unknown as z.ZodType<CodeRuleDslValueTest>;

/** A signature entry: a slot name (an identifier), `/` or `*`. */
const signatureEntry = z.string().max(256).regex(/^(?:[A-Za-z_$][A-Za-z0-9_$]*|\/|\*)$/, "Must be a slot name (an identifier), / or *.");
const slotNames = (signature: readonly string[]): string[] => signature.filter(entry => entry !== "/" && entry !== "*");
export const codeRuleArgumentsSchema = z
  .object({
    signature: z.array(signatureEntry).max(256),
    bind: z.record(z.string().regex(/^[A-Z][A-Z0-9_]*$/, "Must be a metavariable name (A-Z first, then A-Z, 0-9, _, without $)."), z.string()).optional(),
    absent: z.array(z.string()).min(1).optional(),
    mode: z.enum(["language", "syntactic", "estree"]).optional(),
  })
  .strict()
  .refine(entry => slotNames(entry.signature).length > 0, { message: "signature must name a slot.", path: ["signature"] })
  .refine(entry => new Set(slotNames(entry.signature)).size === slotNames(entry.signature).length, { message: "A slot is listed twice.", path: ["signature"] })
  .refine(
    entry => {
      const slash = entry.signature.indexOf("/");
      const star = entry.signature.indexOf("*");
      const once = (marker: string) => entry.signature.filter(item => item === marker).length <= 1;
      return once("/") && once("*") && (slash === -1 || star === -1 || slash < star);
    },
    { message: "/ and * may appear once each, / before *.", path: ["signature"] },
  )
  .refine(entry => entry.bind !== undefined || entry.absent !== undefined, "arguments must bind a slot or list absent ones.")
  .refine(entry => entry.bind === undefined || Object.keys(entry.bind).length > 0, { message: "bind must bind a capture.", path: ["bind"] })
  .refine(entry => [...Object.values(entry.bind ?? {}), ...(entry.absent ?? [])].every(slot => slotNames(entry.signature).includes(slot)), "Every bound and absent slot must be a slot of the signature.")
  .refine(entry => (entry.absent ?? []).every(slot => !Object.values(entry.bind ?? {}).includes(slot)), { message: "A bound slot is passed, so it cannot be absent.", path: ["absent"] }) as unknown as z.ZodType<CodeRuleDslArguments>;

/** ESLint's `LATEST_ECMA_VERSION` in the engine's pinned globals table (ESLint 10.11.0): what `"latest"` selects. */
export const CODE_RULE_LATEST_ECMA_VERSION = 2026 as const;
export const CODE_RULE_SOURCE_TYPES = ["script", "module", "commonjs"] as const;
/** Globals one `languageOptions` may configure (the engine's bound): room for real environment unions. */
export const CODE_RULE_MAX_LANGUAGE_GLOBALS = 16384 as const;
/** The Python versions of the engine's pinned builtins table (Ruff 0.16.9's target versions). */
export const CODE_RULE_PYTHON_VERSIONS = ["3.7", "3.8", "3.9", "3.10", "3.11", "3.12", "3.13", "3.14", "3.15"] as const;
/** Names `typingModules` or `extendGenerics` may list (the engine's bound). */
export const CODE_RULE_MAX_TYPING_NAMES = 256 as const;
/** ESLint's global settings with its aliases (`normalizeConfigGlobal`). */
const CODE_RULE_GLOBAL_SETTINGS = ["readonly", "writable", "off", "readable", "writeable", "true", "false"] as const;

/** An identifier: ASCII letters, digits, `_` and `$`, not starting with a digit (the engine's check). */
const resolveIdentifier = z.string().max(256).regex(/^[A-Za-z_$][A-Za-z0-9_$]*$/, "Must be an identifier (A-Z, a-z, 0-9, _, $).");
/**
 * A Python dotted name, a Rust path (identifiers joined by `::`), or a PHP name: labels (a letter, `_` or any
 * non-ASCII character, then those or digits: the engine's `php_names::is_label`) joined by single `\`, optionally
 * fully qualified. The engine refuses each family the others' shapes. It reads Unicode scalar values (no lone
 * surrogate) and bounds a name's UTF-8 bytes.
 */
const RESOLVE_DOTTED_NAME = /^[A-Za-z_$][A-Za-z0-9_$]*(\.[A-Za-z_$][A-Za-z0-9_$]*)*$/;
const RESOLVE_RUST_PATH = /^[A-Za-z_$][A-Za-z0-9_$]*(::[A-Za-z_$][A-Za-z0-9_$]*)*$/;
const PHP_LABEL = String.raw`[A-Za-z_\u0080-\ud7ff\ue000-\u{10ffff}][A-Za-z0-9_\u0080-\ud7ff\ue000-\u{10ffff}]*`;
const RESOLVE_PHP_NAME = new RegExp(String.raw`^\\?${PHP_LABEL}(?:\\${PHP_LABEL})*$`, "u");
const UTF8 = new TextEncoder();
const resolveDottedName = z
  .string()
  .max(256)
  .refine(name => (RESOLVE_DOTTED_NAME.test(name) || RESOLVE_RUST_PATH.test(name) || RESOLVE_PHP_NAME.test(name)) && UTF8.encode(name).length <= 256, {
    message: String.raw`Must be a dotted name (identifiers joined by .), a Rust path (identifiers joined by ::) or a PHP name (labels joined by \) of at most 256 bytes.`,
  });
/** A Python dotted name: ASCII identifiers without `$` joined by `.` (the engine's check). */
const pythonDottedName = z.string().max(256).regex(/^[A-Za-z_][A-Za-z0-9_]*(\.[A-Za-z_][A-Za-z0-9_]*)*$/, "Must be a dotted name (identifiers joined by .).");
/** Every binding form a binding filter names: the engine's `FORMS`, PHP's `static` and SQL's `relation`, `cte` and `target`. */
export const CODE_RULE_BINDING_FORMS = [
  "assignment",
  "annotated",
  "annotation",
  "augmented",
  "named",
  "for",
  "with",
  "except",
  "import",
  "definition",
  "parameter",
  "global",
  "match",
  "typeParameter",
  "typeAlias",
  "implicit",
  "deletion",
  "field",
  "method",
  "accessor",
  "static",
  "relation",
  "cte",
  "target",
] as const;
/** Every scope kind a binding filter names: Python's, then TypeScript's, PHP's and SQL's others (the engine's order). */
export const CODE_RULE_SCOPE_KINDS = ["module", "class", "function", "lambda", "comprehension", "typeParameters", "global", "block", "namespace", "functionName", "closure", "query"] as const;
const bindingFilterShape = {
  form: z.union([z.enum(CODE_RULE_BINDING_FORMS), z.array(z.enum(CODE_RULE_BINDING_FORMS)).min(1)]).optional(),
  scope: z.union([z.enum(CODE_RULE_SCOPE_KINDS), z.array(z.enum(CODE_RULE_SCOPE_KINDS)).min(1)]).optional(),
  used: z.boolean().optional(),
  referenced: z.boolean().optional(),
  builtin: z.boolean().optional(),
  directives: z.literal("eslint").optional(),
  nestedUse: z.boolean().optional(),
  exported: z.boolean().optional(),
  latest: z.boolean().optional(),
  outer: z.boolean().optional(),
  bindings: z
    .object({
      within: z.string().regex(/^[A-Z0-9_]+$/, "within names a capture without `$` (A-Z, 0-9, _).").optional(),
      min: z.number().int().nonnegative().optional(),
      max: z.number().int().nonnegative().optional(),
    })
    .strict()
    .refine(count => (count.min ?? 0) >= 1 || count.max !== undefined, "A binding count must set min to at least 1, or set max.")
    .refine(count => count.max === undefined || (count.min ?? 0) <= count.max, "A binding count's min must not exceed its max.")
    .optional(),
};
const bindingRulesShape = {
  site: z.lazy(() => codeRuleAstRuleSchema).optional(),
  value: z.lazy(() => codeRuleAstRuleSchema).optional(),
  annotation: z.lazy(() => codeRuleAstRuleSchema).optional(),
};
const occurrenceShape = { occurrence: z.lazy(() => codeRuleAstRuleSchema).optional() };
export const codeRuleBindsSchema = z
  .object({
    ...bindingFilterShape,
    ...bindingRulesShape,
    ...occurrenceShape,
    redefines: z
      .object({ ...bindingFilterShape, ...bindingRulesShape, sameScope: z.boolean().optional(), sameImport: z.boolean().optional(), every: z.boolean().optional() })
      .strict()
      .optional(),
  })
  .strict() as unknown as z.ZodType<CodeRuleDslBinds>;

export const codeRuleResolvesSchema = z
  .object({
    global: z.union([z.array(resolveIdentifier).min(1).max(1024), z.literal(true)]).optional(),
    import: z.union([z.array(resolveDottedName).min(1).max(1024), z.literal(true)]).optional(),
    imported: z.array(resolveDottedName).min(1).max(1024).optional(),
    prefix: z.literal(true).optional(),
    directives: z.literal("eslint").optional(),
    this: z.enum(["global", "globalScope"]).optional(),
    declared: z.boolean().optional(),
    scriptDeclarations: z.literal(true).optional(),
    sloppyFunctions: z.literal(true).optional(),
    relative: z.enum(["never", "text"]).optional(),
    binding: z
      .object({
        ...bindingFilterShape,
        ...bindingRulesShape,
        ...occurrenceShape,
        stopAtGlobal: z.boolean().optional(),
        only: z.boolean().optional(),
        destructuredValue: z.boolean().optional(),
      })
      .strict()
      .optional(),
    unresolved: z.union([z.literal(true), z.literal("star")]).optional(),
    write: z.literal("last").optional(),
    beforeLocal: z.literal(true).optional(),
    seenBefore: z.literal(true).optional(),
  })
  .strict()
  .refine(
    resolves =>
      resolves.write !== undefined ||
      resolves.global !== undefined ||
      resolves.import !== undefined ||
      resolves.imported !== undefined ||
      resolves.this !== undefined ||
      resolves.binding !== undefined ||
      resolves.unresolved !== undefined ||
      resolves.beforeLocal !== undefined,
    "resolves must list global names, import names, imported names, this, unresolved, a binding, write, or beforeLocal.",
  )
  .refine(resolves => resolves.seenBefore === undefined || resolves.imported !== undefined, "seenBefore needs imported modules.")
  .refine(
    resolves => resolves.seenBefore === undefined || (resolves.imported ?? []).every(module => !module.includes(".")),
    "seenBefore asks about modules by their first name, as Ruff's seen_module does; list top-level modules.",
  )
  .refine(
    resolves => resolves.seenBefore === undefined || Object.keys(resolves).every(key => key === "seenBefore" || key === "imported"),
    "seenBefore asks whether an import of a listed module was seen; write other resolves terms in a resolves of their own.",
  )
  .refine(
    resolves => resolves.beforeLocal === undefined || Object.keys(resolves).length === 1,
    "beforeLocal asks about a read before the function binds the name; write other resolves terms in a resolves of their own.",
  )
  .refine(
    resolves => resolves.unresolved === undefined || Object.keys(resolves).every(key => key === "unresolved" || key === "directives"),
    "unresolved asks about names with no binding; write other resolves terms in a resolves of their own.",
  )
  .refine(resolves => resolves.prefix === undefined || resolves.import !== true, "import: true matches every qualified name; it lists none to prefix.")
  .refine(resolves => resolves.declared === undefined || resolves.global !== true, "global: true asks for declared globals and builtins; it has no names to undeclare.")
  .refine(
    resolves => resolves.binding === undefined || Object.keys(resolves).every(key => key === "binding" || key === "write"),
    "binding asks about the name's own file-local binding; write other resolves terms in a resolves of their own.",
  )
  .refine(
    resolves => resolves.write === undefined || Object.keys(resolves).every(key => key === "binding" || key === "write"),
    "write asks where ESLint reports a variable; filter it with binding, and write other resolves terms in a resolves of their own.",
  )
  .refine(resolves => resolves.prefix === undefined || resolves.import !== undefined || resolves.imported !== undefined, "Only import and imported names are dotted; prefix needs some.")
  .refine(
    resolves => resolves.directives === undefined || resolves.global !== undefined || resolves.unresolved !== undefined,
    "/* global */ comments only add global names; list some.",
  )
  .refine(resolves => resolves.declared === undefined || resolves.global !== undefined, "Only global names are declared; declared needs some.")
  .refine(resolves => resolves.relative === undefined || resolves.imported !== undefined, "Only imported names import sites; relative needs some.")
  .refine(resolves => resolves.sloppyFunctions === undefined || resolves.this === "global", 'Only this: "global" reads a function\'s this; sloppyFunctions needs it.')
  .refine(resolves => resolves.scriptDeclarations === undefined || resolves.global !== undefined, "Only global names resolve to declarations; scriptDeclarations needs some.")
  .refine(
    resolves => resolves.declared !== false || resolves.directives === undefined,
    "/* global */ comments decide what is declared, which declared: false ignores.",
  ) as unknown as z.ZodType<CodeRuleDslResolves>;

/** `ecmaVersion`: "latest", 3, 5, 6 to 17 (ES2015 to ES2026), or a year the engine's table has. */
const isEcmaVersion = (version: number): boolean =>
  version === 3 || version === 5 || (version >= 6 && version <= CODE_RULE_LATEST_ECMA_VERSION - 2009) || (version >= 2015 && version <= CODE_RULE_LATEST_ECMA_VERSION);
const ecmaVersionValue = z.union([
  z.literal("latest"),
  z.number().int().refine(isEcmaVersion, `ecmaVersion must be "latest", 3, 5, 6 to ${CODE_RULE_LATEST_ECMA_VERSION - 2009}, or 2015 to ${CODE_RULE_LATEST_ECMA_VERSION}.`),
]);
const ecmaYear = (version: number | "latest"): number => {
  if (version === "latest") return CODE_RULE_LATEST_ECMA_VERSION;
  return version >= 6 && version < 2015 ? version + 2009 : version;
};
const pythonMinor = (version: string): number => Number(version.slice(2));
const versionRange = <T extends z.ZodTypeAny>(value: T, order: (version: z.infer<T>) => number) =>
  z
    .object({ min: value.optional(), max: value.optional() })
    .strict()
    .refine(range => range.min !== undefined || range.max !== undefined, "Set min, max or both.")
    .refine(range => range.min === undefined || range.max === undefined || order(range.min) <= order(range.max), "min is after max, so the query never applies.");
export const codeRuleAstWhenSchema = z
  .object({
    pythonVersion: versionRange(z.enum(CODE_RULE_PYTHON_VERSIONS), pythonMinor).optional(),
    ecmaVersion: versionRange(ecmaVersionValue, ecmaYear).optional(),
    sourceType: z.union([z.enum(CODE_RULE_SOURCE_TYPES), z.array(z.enum(CODE_RULE_SOURCE_TYPES)).min(1)]).optional(),
    ecmaFeatures: z
      .object({ globalReturn: z.boolean().optional(), impliedStrict: z.boolean().optional() })
      .strict()
      .refine(features => features.globalReturn !== undefined || features.impliedStrict !== undefined, "Set globalReturn, impliedStrict or both.")
      .optional(),
    option: z.record(z.string(), codeRuleOptionConditionSchema).optional(),
  })
  .strict() as unknown as z.ZodType<CodeRuleDslAstWhen>;
const moduleRegex = z
  .object({ regex: nonEmptyString.optional(), notRegex: nonEmptyString.optional() })
  .strict()
  .refine(condition => condition.regex !== undefined || condition.notRegex !== undefined, "Set regex, notRegex or both.");
const packageConditionSchema = z
  .object({
    bin: z.boolean().optional(),
    nodeVersion: z.object({ subsetOf: nonEmptyString }).strict().optional(),
    dependsOn: z.array(nonEmptyString).min(1).optional(),
    path: moduleRegex.optional(),
    serverGraph: z.boolean().optional(),
  })
  .strict()
  .refine(condition => Object.values(condition).some(value => value !== undefined), "Set bin, nodeVersion, dependsOn, path or serverGraph.");
export const codeRuleFileConditionSchema = z
  .object({
    path: moduleRegex.optional(),
    declaration: z.boolean().optional(),
    module: moduleRegex.optional(),
    moduleName: moduleRegex.optional(),
    package: z.union([moduleRegex, packageConditionSchema]).optional(),
    inPackage: z.boolean().optional(),
    jsxRuntime: z.array(z.enum(["react", "other", "unknown"])).min(1).optional(),
  })
  .strict()
  .refine(
    file => Object.values(file).some(value => value !== undefined),
    "Set path, declaration, module, moduleName, package, inPackage or jsxRuntime.",
  ) as unknown as z.ZodType<CodeRuleDslFileCondition>;
export const codeRuleLanguageOptionsSchema = z
  .object({
    ecmaVersion: z
      .union([z.literal("latest"), z.number().int().refine(isEcmaVersion, `ecmaVersion must be "latest", 3, 5, 6 to ${CODE_RULE_LATEST_ECMA_VERSION - 2009}, or 2015 to ${CODE_RULE_LATEST_ECMA_VERSION}.`)])
      .optional(),
    sourceType: z.enum(CODE_RULE_SOURCE_TYPES).optional(),
    globals: z
      .record(resolveIdentifier, z.union([z.enum(CODE_RULE_GLOBAL_SETTINGS), z.boolean(), z.null()]))
      .refine(globals => Object.keys(globals).length <= CODE_RULE_MAX_LANGUAGE_GLOBALS, `At most ${CODE_RULE_MAX_LANGUAGE_GLOBALS} globals.`)
      .optional(),
    parserOptions: z
      .object({ ecmaFeatures: z.object({ globalReturn: z.boolean().optional(), impliedStrict: z.boolean().optional() }).strict().optional() })
      .strict()
      .optional(),
    jsxPragma: resolveIdentifier.nullable().optional(),
    jsxFragmentName: resolveIdentifier.nullable().optional(),
    /** typescript-eslint's lib names; the engine refuses a name its pinned table lacks. */
    lib: z.array(z.string().regex(/^[A-Za-z0-9.]+$/, "Must be a TypeScript lib name.")).optional(),
    pythonVersion: z.enum(CODE_RULE_PYTHON_VERSIONS).optional(),
    typingModules: z.array(pythonDottedName).max(CODE_RULE_MAX_TYPING_NAMES).optional(),
    extendGenerics: z.array(pythonDottedName).max(CODE_RULE_MAX_TYPING_NAMES).optional(),
  })
  .strict()
  .refine(
    options => (options.sourceType ?? "module") !== "module" || options.ecmaVersion === undefined || options.ecmaVersion === "latest" || options.ecmaVersion >= 6,
    'sourceType "module" (the default) needs ecmaVersion 2015 or later; ESLint refuses it.',
  ) as unknown as z.ZodType<CodeRuleDslLanguageOptions>;

const lineCountValue = z.number().int().nonnegative();
const lineCount = z.union([
  z.object({ eq: lineCountValue }).strict(),
  z
    .object({ min: lineCountValue.optional(), max: lineCountValue.optional() })
    .strict()
    .refine(count => count.min !== undefined || count.max !== undefined, "Set eq, min or max.")
    .refine(count => count.min === undefined || count.max === undefined || count.min <= count.max, "min must not exceed max."),
]);
const layoutGap = z
  .object({
    to: z.string().regex(/^(?:token|sibling|capture\.[A-Z0-9_]+)$/, 'to must be "token", "sibling" or "capture.<NAME>".'),
    comments: z.boolean().optional(),
    edge: z.boolean().optional(),
    breaks: lineCount.optional(),
    blank: lineCount.optional(),
  })
  .strict()
  .refine(gap => gap.breaks !== undefined || gap.blank !== undefined, "A gap must count breaks, blank or both.")
  .refine(gap => !gap.to.startsWith("capture.") || (gap.comments === undefined && gap.edge === undefined), "comments and edge choose the neighbouring token or sibling; a capture names its node.");
export const codeRuleLayoutSchema = z
  .object({ span: lineCount.optional(), start: lineCount.optional(), before: layoutGap.optional(), after: layoutGap.optional(), line: z.string().optional(), previousLine: z.string().optional(),
    context: z.object({ before: z.number().int().min(0).max(64).optional(), after: z.number().int().min(0).max(64).optional(), through: z.string().optional(), regex: z.string() }).strict().optional() })
  .strict()
  .refine(layout => Object.values(layout).some(value => value !== undefined), "layout must hold span, start, before or after.") as unknown as z.ZodType<CodeRuleDslLayout>;

export const codeRuleNthChildSchema: z.ZodType<CodeRuleDslNthChild> = z.lazy(() =>
  z.union([
    nthChildPosition,
    z.object({ position: nthChildPosition, ofRule: codeRuleAstRuleSchema.optional(), reverse: z.boolean().optional() }).strict(),
  ]) as unknown as z.ZodType<CodeRuleDslNthChild>,
);
export const codeRuleAstRuleSchema: z.ZodType<CodeRuleDslAstRule> = z.lazy(() =>
  z
    .object({
      pattern: z.union([nonEmptyString, z.object({ context: nonEmptyString, selector: nonEmptyString }).strict()]).optional(),
      kind: nonEmptyString.optional(),
      regex: nonEmptyString.optional(),
      nthChild: codeRuleNthChildSchema.optional(),
      all: z.array(codeRuleAstRuleSchema).min(1).optional(),
      any: z.array(codeRuleAstRuleSchema).min(1).optional(),
      not: codeRuleAstRuleSchema.optional(),
      matches: matchesField,
      inside: codeRuleAstRelationSchema.optional(),
      has: codeRuleAstRelationSchema.optional(),
      precedes: codeRuleAstRelationSchema.optional(),
      follows: codeRuleAstRelationSchema.optional(),
      compare: codeRuleCompareSchema.optional(),
      count: codeRuleAstCountSchema.optional(),
      filePath: z.object({ regex: nonEmptyString.optional(), notRegex: nonEmptyString.optional() }).strict().refine(value => value.regex !== undefined || value.notRegex !== undefined).optional(),
      ...semanticRuleShape({
        resolves: codeRuleResolvesSchema,
        value: codeRuleValueTestSchema,
        arguments: codeRuleArgumentsSchema,
        binds: codeRuleBindsSchema,
        parse: codeRuleParseSchema,
        capture: metavariableName,
        formatArgs: codeRuleFormatArgsSchema,
        layout: codeRuleLayoutSchema,
        host: codeRuleAstRuleSchema,
        calls: codeRuleCallsSchema,
      }),
    })
    .strict()
    .refine(rule => AST_RULE_KEYS.some(key => rule[key] !== undefined), "An ast rule must contain at least one matcher.") as unknown as z.ZodType<CodeRuleDslAstRule>,
);
export const codeRuleAstRelationSchema: z.ZodType<CodeRuleDslAstRelation> = z.lazy(() =>
  z
    .object({
      stopBy: z.union([z.enum(CODE_RULE_STOP_BY), codeRuleAstRuleSchema]).optional(),
      field: nonEmptyString.optional(),
      pattern: z.union([nonEmptyString, z.object({ context: nonEmptyString, selector: nonEmptyString }).strict()]).optional(),
      kind: nonEmptyString.optional(),
      regex: nonEmptyString.optional(),
      nthChild: codeRuleNthChildSchema.optional(),
      all: z.array(codeRuleAstRuleSchema).min(1).optional(),
      any: z.array(codeRuleAstRuleSchema).min(1).optional(),
      not: codeRuleAstRuleSchema.optional(),
      matches: matchesField,
      inside: codeRuleAstRelationSchema.optional(),
      has: codeRuleAstRelationSchema.optional(),
      precedes: codeRuleAstRelationSchema.optional(),
      follows: codeRuleAstRelationSchema.optional(),
      compare: codeRuleCompareSchema.optional(),
      count: codeRuleAstCountSchema.optional(),
      filePath: z.object({ regex: nonEmptyString.optional(), notRegex: nonEmptyString.optional() }).strict().refine(value => value.regex !== undefined || value.notRegex !== undefined).optional(),
      ...semanticRuleShape({
        resolves: codeRuleResolvesSchema,
        value: codeRuleValueTestSchema,
        arguments: codeRuleArgumentsSchema,
        binds: codeRuleBindsSchema,
        parse: codeRuleParseSchema,
        capture: metavariableName,
        formatArgs: codeRuleFormatArgsSchema,
        layout: codeRuleLayoutSchema,
        host: codeRuleAstRuleSchema,
        calls: codeRuleCallsSchema,
      }),
    })
    .strict()
    .refine(rule => AST_RULE_KEYS.some(key => rule[key] !== undefined), "An ast relation must contain at least one matcher.") as unknown as z.ZodType<CodeRuleDslAstRelation>,
);
/** `calls`: its inner rule's keys, and how chains pass and what they bind. */
export const codeRuleCallsSchema: z.ZodType<CodeRuleDslCalls> = z.lazy(
  () =>
    z
      .object({ through: z.enum(["sync", "any"]).optional(), chain: metavariableName.optional(),
        graph: z.enum(["calls", "references"]).optional(), at: z.enum(["call", "definition"]).optional(),
        siblingLinks: z.array(z.object({ decorator: nonEmptyString,
          name: z.object({ replace: nonEmptyString, by: z.string() }).strict() }).strict()).min(1).max(32).optional(),
        constructorFields: z.object({ fieldToParameter: z.object({ replace: nonEmptyString, by: z.string() }).strict() }).strict().optional() })
      .passthrough()
      .superRefine((calls, context) => {
        if (calls.graph === "references" && calls.at !== "definition") {
          context.addIssue({ code: "custom", path: ["at"], message: "graph references requires at definition." });
        }
        if ((calls.siblingLinks !== undefined || calls.constructorFields !== undefined) && calls.graph !== "references") {
          const field = calls.siblingLinks !== undefined ? "siblingLinks" : "constructorFields";
          context.addIssue({ code: "custom", path: [field], message: `${field} requires graph references.` });
        }
        const { through: _through, chain: _chain, graph: _graph, at: _at, siblingLinks: _siblingLinks, constructorFields: _constructorFields, ...inner } = calls;
        const parsed = codeRuleAstRuleSchema.safeParse(inner);
        if (!parsed.success) {
          for (const issue of parsed.error.issues) context.addIssue({ ...issue, code: "custom", message: issue.message });
        }
      }) as unknown as z.ZodType<CodeRuleDslCalls>,
);
export const codeRuleFormatArgsSchema: z.ZodType<CodeRuleDslFormatArgs> = z.lazy(() =>
  z
    .object({
      style: z.enum(["brace", "percent", "printf", "scanf", "rust"]),
      string: metavariableName,
      args: metavariableName,
      unused: codeRuleAstRuleSchema.optional(),
      missing: codeRuleAstRuleSchema.optional(),
    })
    .strict()
    .refine(test => test.unused !== undefined || test.missing !== undefined, "formatArgs must hold unused, missing or both."),
) as unknown as z.ZodType<CodeRuleDslFormatArgs>;
export const codeRuleParseSchema: z.ZodType<CodeRuleDslParse> = z.lazy(() =>
  z
    .object({
      grammar: z.string(),
      interpolation: z.string().refine(value => new TextEncoder().encode(value).length <= 1024, "interpolation exceeds 1024 bytes.").optional(),
      pattern: z.union([nonEmptyString, z.object({ context: nonEmptyString, selector: nonEmptyString }).strict()]).optional(),
      kind: nonEmptyString.optional(),
      regex: nonEmptyString.optional(),
      nthChild: codeRuleNthChildSchema.optional(),
      all: z.array(codeRuleAstRuleSchema).min(1).optional(),
      any: z.array(codeRuleAstRuleSchema).min(1).optional(),
      not: codeRuleAstRuleSchema.optional(),
      matches: matchesField,
      inside: codeRuleAstRelationSchema.optional(),
      has: codeRuleAstRelationSchema.optional(),
      precedes: codeRuleAstRelationSchema.optional(),
      follows: codeRuleAstRelationSchema.optional(),
      compare: codeRuleCompareSchema.optional(),
      count: codeRuleAstCountSchema.optional(),
      ...semanticRuleShape({
        resolves: codeRuleResolvesSchema,
        value: codeRuleValueTestSchema,
        arguments: codeRuleArgumentsSchema,
        binds: codeRuleBindsSchema,
        parse: codeRuleParseSchema,
        capture: metavariableName,
        formatArgs: codeRuleFormatArgsSchema,
        layout: codeRuleLayoutSchema,
        host: codeRuleAstRuleSchema,
        calls: codeRuleCallsSchema,
      }),
    })
    .strict()
    .refine(rule => (CODE_RULE_PARSE_GRAMMARS as readonly string[]).includes(rule.grammar), {
      message: `grammar must be one of ${CODE_RULE_PARSE_GRAMMARS.join(", ")}.`,
      path: ["grammar"],
    })
    .refine(rule => rule.interpolation === undefined || rule.grammar === "pythonFString", "interpolation requires pythonFString.")
    .refine(rule => AST_RULE_KEYS.some(key => key !== "filePath" && rule[key] !== undefined), "parse must hold a rule over the parsed tree.") as unknown as z.ZodType<CodeRuleDslParse>,
);
export const codeRuleAstCountSchema: z.ZodType<CodeRuleDslAstCount> = z.lazy(() =>
  z
    .object({
      inside: codeRuleAstRelationSchema.optional(),
      has: codeRuleAstRelationSchema.optional(),
      precedes: codeRuleAstRelationSchema.optional(),
      follows: codeRuleAstRelationSchema.optional(),
      min: z.number().int().nonnegative().optional(),
      max: z.number().int().nonnegative().optional(),
      as: metavariableName.optional(),
      offset: z.number().int().optional(),
    })
    .strict()
    .refine(count => AST_RELATION_KEYS.filter(key => count[key] !== undefined).length === 1, "A count must contain exactly one relation: inside, has, precedes, or follows.")
    .refine(count => (count.min ?? 0) >= 1 || count.max !== undefined, "A count must set min to at least 1, or set max.")
    .refine(count => count.max === undefined || (count.min ?? 0) <= count.max, "A count's min must not exceed its max.")
    .refine(count => count.offset === undefined || count.as !== undefined, { message: "A count's offset needs as: it is added to the value that as binds.", path: ["offset"] }) as unknown as z.ZodType<CodeRuleDslAstCount>,
);

const codeRuleValuePair = z.tuple([codeRuleValueSchema, codeRuleValueSchema]);
export const codeRulePredicateSchema: z.ZodType<CodeRuleDslPredicate> = z.lazy(() =>
  z.union([
    z.object({ all: z.array(codeRulePredicateSchema).min(1) }).strict(),
    z.object({ any: z.array(codeRulePredicateSchema).min(1) }).strict(),
    z.object({ not: codeRulePredicateSchema }).strict(),
    z.object({ eq: codeRuleValuePair }).strict(),
    z.object({ ne: codeRuleValuePair }).strict(),
    z.object({ lt: codeRuleValuePair }).strict(),
    z.object({ le: codeRuleValuePair }).strict(),
    z.object({ gt: codeRuleValuePair }).strict(),
    z.object({ ge: codeRuleValuePair }).strict(),
  ]),
);

export type CodeRuleQuery = CodeRuleDslQuery;


/** A utility: a rule, or a rule with parameters, `{ params, rule }`, which a call runs in a frame of its own. */
export const codeRuleUtilitySchema: z.ZodType<CodeRuleDslUtility> = z.lazy(() =>
  z.union([
    z
      .object({
        params: z
          .array(metavariableName)
          .min(1)
          .refine(params => new Set(params).size === params.length, "A parameter is listed twice."),
        rule: codeRuleAstRuleSchema,
      })
      .strict(),
    codeRuleAstRuleSchema,
  ]) as unknown as z.ZodType<CodeRuleDslUtility>,
);

/** A utility's rule and parameters (none for a plain rule). */
function utilityParts(util: CodeRuleDslUtility): { rule: CodeRuleDslAstRule; params: readonly string[]; path: string[] } {
  return "params" in util && Array.isArray(util.params) && "rule" in util
    ? { rule: util.rule as CodeRuleDslAstRule, params: util.params, path: ["rule"] }
    : { rule: util as CodeRuleDslAstRule, params: [], path: [] };
}

/** Whether a rule holds a `calls` term anywhere in it. */
function containsCalls(value: unknown): boolean {
  if (Array.isArray(value)) return value.some(containsCalls);
  if (value === null || typeof value !== "object") return false;
  return Object.entries(value).some(([key, item]) => key === "calls" || containsCalls(item));
}

type ParseGrammar = (typeof CODE_RULE_PARSE_GRAMMARS)[number];
type RulePath = (string | number)[];
type RuleIssue = (path: RulePath, message: string) => void;

/** A rule's structural checks, as the engine reads them. */
type AstRuleVisit = (rule: CodeRuleDslAstRule, path: RulePath, parsed?: ParseGrammar, fields?: boolean) => void;

/** `host` reads the literal a parse holds: only inside parse, not on formatArgs' fields, and not over an annotation. */
function checkHost(rule: CodeRuleDslAstRule, path: RulePath, parsed: ParseGrammar | undefined, fields: boolean, issue: RuleIssue, visit: AstRuleVisit): void {
  if (rule.host === undefined) return;
  if (parsed === undefined) issue([...path, "host"], "host tests the literal a parse holds; use it inside parse.");
  else if (!fields && parsed !== "annotation") issue([...path, "host"], "host tests the literal a parse holds; formatArgs' fields have none.");
  else if (parsed !== "annotation") visit(rule.host, [...path, "host"]);
}

/** Keys a rule over a tree of `parsed`'s grammar may not use. */
function checkParsedKeys(rule: CodeRuleDslAstRule, path: RulePath, parsed: ParseGrammar, issue: RuleIssue): void {
  for (const key of AST_RULE_KEYS) {
    if (rule[key] === undefined || PARSE_RULE_KEYS.includes(key)) continue;
    if (parsed === "annotation" && ANNOTATION_RULE_KEYS.includes(key)) continue;
    if (parsed === "eslintDirective" && key === "resolves") continue;
    issue([...path, key], `${key} is not supported inside parse (${parsed} trees).`);
  }
}

/** An annotation's nodes are Python's, but its names' bindings and literals are the host's. */
function checkAnnotationNode(rule: CodeRuleDslAstRule, path: RulePath, issue: RuleIssue): void {
  if (rule.host !== undefined) issue([...path, "host"], "host is not supported inside parse (annotation trees).");
  if (rule.resolves?.binding !== undefined) issue([...path, "resolves", "binding"], "A parsed annotation's names are read, but their bindings' nodes are the host's.");
  if (rule.parse !== undefined && rule.parse.grammar !== "annotation") issue([...path, "parse", "grammar"], "A string inside an annotation parses as an annotation.");
}

/** What a node of a parsed tree has: its grammar's kinds, no binding, and source-level compare texts. */
function checkParsedNode(rule: CodeRuleDslAstRule, path: RulePath, parsed: ParseGrammar, issue: RuleIssue): void {
  const kinds: readonly string[] = (CODE_RULE_PARSE_KINDS as Record<string, readonly string[]>)[parsed] ?? [];
  if (parsed !== "annotation" && rule.kind !== undefined && !kinds.includes(rule.kind)) {
    issue([...path, "kind"], `${rule.kind} is not a ${parsed} kind; expected one of ${kinds.join(", ")}.`);
  }
  if (rule.value?.follow !== undefined) issue([...path, "value", "follow"], "A parsed node has no binding to follow.");
  if (parsed === "annotation") checkAnnotationNode(rule, path, issue);
  if (parsed === "eslintDirective" && rule.resolves !== undefined) {
    for (const key of Object.keys(rule.resolves).filter(key => key !== "global")) {
      issue([...path, "resolves", key], "A directive's node resolves by global only: its text as a declared global of the query's languageOptions.");
    }
  }
  if (parsed !== "annotation" && rule.compare?.text !== undefined && !PARSE_COMPARE_TEXTS.includes(rule.compare.text)) {
    issue([...path, "compare", "text"], "A parsed tree compares by source, unquoted, value or node.");
  }
}

/** `parse`: its rule is a rule over its grammar's tree. */
function visitParse(rule: CodeRuleDslAstRule, path: RulePath, visit: AstRuleVisit): void {
  if (rule.parse === undefined) return;
  const grammar = rule.parse.grammar as ParseGrammar;
  if (CODE_RULE_PARSE_GRAMMARS.includes(grammar)) visit(rule.parse, [...path, "parse"], grammar);
}

const STYLE_GRAMMARS: Record<CodeRuleDslFormatArgs["style"], ParseGrammar> = {
  brace: "braceFormat",
  percent: "percentFormat",
  printf: "printfFormat",
  scanf: "scanfFormat",
  rust: "rustFormat",
};

/** `formatArgs`: its unused arguments are the host's nodes, its missing fields nodes of the format's tree. */
function visitFormatArgs(rule: CodeRuleDslAstRule, path: RulePath, parsed: ParseGrammar | undefined, visit: AstRuleVisit): void {
  if (rule.formatArgs === undefined) return;
  if (rule.formatArgs.unused !== undefined) visit(rule.formatArgs.unused, [...path, "formatArgs", "unused"], parsed);
  const grammar = STYLE_GRAMMARS[rule.formatArgs.style];
  if (rule.formatArgs.missing !== undefined) visit(rule.formatArgs.missing, [...path, "formatArgs", "missing"], grammar, false);
}

function visitCalls(rule: CodeRuleDslAstRule, path: RulePath, parsed: ParseGrammar | undefined, issue: RuleIssue, visit: AstRuleVisit): void {
  if (rule.calls === undefined) return;
  const { through: _through, chain: _chain, graph: _graph, at: _at, siblingLinks: _siblingLinks, constructorFields: _constructorFields, ...inner } = rule.calls;
  if (containsCalls(inner)) issue([...path, "calls"], "A calls rule cannot contain calls.");
  visit(inner, [...path, "calls"], parsed);
}

/** A binding's `occurrence` is a rule over the occurrence. */
function visitOccurrences(rule: CodeRuleDslAstRule, path: RulePath, visit: AstRuleVisit): void {
  for (const [owner, ownerPath] of [
    [rule.binds, [...path, "binds"]],
    [rule.resolves?.binding, [...path, "resolves", "binding"]],
  ] as const) {
    if (owner?.occurrence !== undefined) visit(owner.occurrence, [...ownerPath, "occurrence"]);
  }
}

/** A rule's or count's relations: `field` only in has/inside, outside count and parse. */
function visitRelations(
  owner: CodeRuleDslAstRule | CodeRuleDslAstCount,
  ownerPath: RulePath,
  inCount: boolean,
  parsed: ParseGrammar | undefined,
  fields: boolean,
  issue: RuleIssue,
  visit: AstRuleVisit,
): void {
  for (const key of AST_RELATION_KEYS) {
    const nested = owner[key];
    if (nested === undefined) continue;
    if (nested.field !== undefined) {
      if (inCount) issue([...ownerPath, key, "field"], "field is not supported in count.");
      else if (key !== "inside" && key !== "has") issue([...ownerPath, key, "field"], "field is only supported in has/inside.");
      else if (parsed !== undefined) issue([...ownerPath, key, "field"], "field is not supported inside parse.");
    }
    visit(nested, [...ownerPath, key], parsed, fields);
    if (typeof nested.stopBy === "object") visit(nested.stopBy, [...ownerPath, key, "stopBy"], parsed, fields);
  }
}

/** The rules a binding's site, value and annotation are tested against: the query's own tree. */
function visitBindingRules(rule: CodeRuleDslAstRule, path: RulePath, visit: AstRuleVisit): void {
  for (const key of ["site", "value", "annotation"] as const) {
    const nested = rule.resolves?.binding?.[key];
    if (nested !== undefined) visit(nested, [...path, "resolves", "binding", key]);
    const bound = rule.binds?.[key];
    if (bound !== undefined) visit(bound, [...path, "binds", key]);
    const shadowed = rule.binds?.redefines?.[key];
    if (shadowed !== undefined) visit(shadowed, [...path, "binds", "redefines", key]);
  }
}

/** Sub-rules over the same tree as their parent: all/any, nthChild's ofRule, not, relations, and binding rules. */
function visitSubRules(rule: CodeRuleDslAstRule, path: RulePath, parsed: ParseGrammar | undefined, fields: boolean, issue: RuleIssue, visit: AstRuleVisit): void {
  for (const key of ["all", "any"] as const) rule[key]?.forEach((item, index) => visit(item, [...path, key, index], parsed, fields));
  if (typeof rule.nthChild === "object" && rule.nthChild.ofRule !== undefined) visit(rule.nthChild.ofRule, [...path, "nthChild", "ofRule"], parsed, fields);
  if (rule.not !== undefined) visit(rule.not, [...path, "not"], parsed, fields);
  visitRelations(rule, path, false, parsed, fields, issue, visit);
  if (rule.count !== undefined) {
    if (parsed !== undefined && rule.count.as !== undefined) issue([...path, "count", "as"], "A parsed tree's count binds nothing; count in the ast query's own rule to report its value.");
    visitRelations(rule.count, [...path, "count"], true, parsed, fields, issue, visit);
  }
  visitBindingRules(rule, path, visit);
}

/**
 * The structural checks of a rule and its sub-rules: keys a parsed tree's grammar lacks, its kinds, and where `host`,
 * `capture`, `field` and `calls` may appear. What a rule's language lacks is the engine's to refuse. Issues are
 * reported in the order the engine reads the rule.
 */
function astRuleVisitor(context: z.RefinementCtx): AstRuleVisit {
  const issue: RuleIssue = (path, message) => context.addIssue({ code: "custom", path, message });
  const visit: AstRuleVisit = (rule, path, parsed, fields = true) => {
    if (parsed === undefined && rule.capture !== undefined) issue([...path, "capture"], "capture binds a node of a parsed tree; use it inside parse.");
    checkHost(rule, path, parsed, fields, issue, visit);
    if (parsed !== undefined) {
      checkParsedKeys(rule, path, parsed, issue);
      checkParsedNode(rule, path, parsed, issue);
    }
    visitParse(rule, path, visit);
    visitFormatArgs(rule, path, parsed, visit);
    visitCalls(rule, path, parsed, issue, visit);
    visitOccurrences(rule, path, visit);
    visitSubRules(rule, path, parsed, fields, issue, visit);
  };
  return visit;
}

/** Transforms that read a node but name a transform variable, and transforms whose sources cycle. */
function checkTransforms(transform: Record<string, CodeRuleDslTransformation>, context: z.RefinementCtx): void {
  for (const [name, transformation] of Object.entries(transform)) {
    const source = transformSourceName(transformation);
    if (readsNode(transformation) && Object.hasOwn(transform, source)) {
      context.addIssue({ code: "custom", path: ["transform", name], message: `$${source} is a transform variable, text; line, lines and count read a capture.` });
    }
  }
  for (const name of Object.keys(transform)) {
    const seen = new Set<string>();
    for (let next: string | undefined = name; next !== undefined && Object.hasOwn(transform, next); next = transformSourceName(transform[next]!)) {
      if (seen.has(next)) {
        context.addIssue({ code: "custom", path: ["transform", name], message: `${name} has a cyclic dependency.` });
        break;
      }
      seen.add(next);
    }
    checkTransformValue(name, transform, context);
  }
}

/** A transform reading a value reads it from one capture, not a `$$$` list or a transform variable. */
function checkTransformValue(name: string, transform: Record<string, CodeRuleDslTransformation>, context: z.RefinementCtx): void {
  const body = transformBody(transform[name]!);
  if (body.value !== true) return;
  if (body.source.startsWith("$$$")) {
    context.addIssue({ code: "custom", path: ["transform", name], message: "A value is read from a single capture ($NAME)." });
  } else if (Object.hasOwn(transform, transformSourceName(transform[name]!))) {
    context.addIssue({ code: "custom", path: ["transform", name], message: "A transform variable is text; read the value of a capture." });
  }
}


export const codeRuleQuerySchema: z.ZodType<CodeRuleDslQuery> = z.lazy(() =>
  z
    .object({
      regex: z
        .object({
          source: z.enum(CODE_RULE_REGEX_SOURCES),
          pattern: nonEmptyString,
          content: z.object({ include: z.array(nonEmptyString).max(64).optional(), exclude: z.array(nonEmptyString).max(64).optional() }).strict().optional(),
          include: z.array(nonEmptyString).max(64).optional(),
          exclude: z.array(nonEmptyString).max(64).optional(),
          lines: z.object({ include: z.array(nonEmptyString).max(64).optional(), exclude: z.array(nonEmptyString).max(64).optional(), first: z.boolean().optional(),
            terminators: z.enum(["python", "lf"]).optional(),
            state: z.object({ initial: nonEmptyString.max(64), emit: z.array(nonEmptyString.max(64)).min(1).max(16),
              transitions: z.array(z.object({ from: nonEmptyString.max(64), to: nonEmptyString.max(64), pattern: z.string(), skip: z.boolean().optional(),
                count: z.object({ min: z.number().int().nonnegative().optional(), max: z.number().int().nonnegative().optional(), parity: z.enum(["odd", "even"]).optional() }).strict().optional(),
              }).strict()).min(1).max(64) }).strict().optional(),
            indentation: z.object({ excludeUnder: z.string(), reset: z.string(), ignore: z.array(z.string()).max(64) }).strict().optional(),
          }).strict().optional(),
          paths: codeRuleScopeSchema.optional(),
        })
        .strict()
        .refine(query => query.lines === undefined || query.source === "file.content", "lines requires file.content")
        .refine(query => query.content === undefined || query.source === "file.content", "content requires file.content")
        .optional(),
      ast: z
        .object({
          language: z.enum(CODE_RULE_LANGUAGE_IDS),
          syntax: z.enum(["valid", "invalid"]).optional(),
          rule: codeRuleAstRuleSchema,
          utils: z.record(utilityId, codeRuleUtilitySchema).optional(),
          constraints: z.record(nonEmptyString, codeRuleAstRuleSchema).optional(),
          languageOptions: codeRuleLanguageOptionsSchema.optional(),
          when: codeRuleAstWhenSchema.optional(),
          file: codeRuleFileConditionSchema.optional(),
          transform: z.record(metavariableName, codeRuleTransformationSchema).optional(),
        })
        .strict()
        .superRefine((ast, context) => {
          // The engine is the authority; these are the cross-references a schema can see.
          const utils = ast.utils ?? {};
          checkTransforms(ast.transform ?? {}, context);
          // A planned language's rule is not read yet: compiling refuses it as language-not-supported.
          if (!isCodeRuleAstLanguage(ast.language)) return;
          const visit = astRuleVisitor(context);
          visit(ast.rule, ["rule"]);
          for (const [id, util] of Object.entries(utils)) visit(utilityParts(util).rule, ["utils", id, ...utilityParts(util).path]);
          for (const [name, constraint] of Object.entries(ast.constraints ?? {})) visit(constraint, ["constraints", name]);
        })
        .optional(),
      select_when: z.object({ select: codeRuleQuerySchema, exists: codeRuleQuerySchema }).strict().optional(),
    })
    .strict()
    .refine(
      query => (["regex", "ast", "select_when"] as const).filter(key => query[key] !== undefined).length === 1,
      "A query must contain exactly one source: regex, ast, or select_when.",
    ) as unknown as z.ZodType<CodeRuleDslQuery>,
);

const PROJECTION_SOURCE = /^(capture\.[A-Z_][A-Z0-9_]*(?:\.(?:startByte|endByte|startLine|kind|text))?|file\.(path|directory|name|stem|module|package))$/;

export const codeRuleEvidenceQuerySchema = z
  .object({
    evidence: z
      .object({
        as: nonEmptyString,
        paths: z
          .object({ include: z.array(nonEmptyString).optional(), exclude: z.array(nonEmptyString).optional() })
          .strict()
          .optional(),
        query: z.lazy(() => codeRuleQuerySchema).refine(query => "ast" in query, "An evidence query is an ast query."),
        project: z
          .record(nonEmptyString, z.union([
            z.string().regex(PROJECTION_SOURCE, "expected capture.<NAME> or file.path|directory|name|stem|module|package"),
            z.object({ source: z.string().regex(PROJECTION_SOURCE), regex: nonEmptyString,
              group: z.number().int().nonnegative() }).strict(),
          ]))
          .refine(project => Object.keys(project).length > 0, "project names at least one field."),
        where: codeRulePredicateSchema.optional(),
        each: metavariableName.optional(),
        anchor: nonEmptyString.optional(),
        closure: z.array(z.object({
          name: nonEmptyString, seeds: z.array(nonEmptyString).optional(), seed_field: nonEmptyString.optional(),
          edges: z.object({ from: nonEmptyString, to: nonEmptyString }).strict().optional(),
          extends: z.array(nonEmptyString).optional(), partition_by: z.array(nonEmptyString).optional(),
          contains: z.record(nonEmptyString, nonEmptyString),
        }).strict()).min(1).max(32).optional(),
      })
      .strict()
      .superRefine((evidence, context) => {
        const fields = new Set(Object.keys(evidence.project));
        const outputs = new Set<string>();
        const stages = new Map<string, string[]>();
        for (const [index, stage] of (evidence.closure ?? []).entries()) {
          const issue = (key: string, message: string) => context.addIssue({ code: "custom", path: ["closure", index, key], message });
          if (stages.has(stage.name)) issue("name", `Duplicate closure set ${stage.name}.`);
          const partition = stage.partition_by ?? [];
          for (const dependency of stage.extends ?? []) {
            const earlier = stages.get(dependency);
            if (earlier === undefined || JSON.stringify(earlier) !== JSON.stringify(partition)) {
              issue("extends", `${dependency} must be an earlier set with the same partition_by.`);
            }
          }
          const inputs = [...partition, ...Object.values(stage.contains),
            ...(stage.seed_field === undefined ? [] : [stage.seed_field]),
            ...(stage.edges === undefined ? [] : [stage.edges.from, stage.edges.to])];
          for (const field of inputs) {
            if (!fields.has(field)) issue("contains", `${field} names no projected field.`);
          }
          for (const output of Object.keys(stage.contains)) {
            if (fields.has(output) || outputs.has(output)) issue("contains", `Output ${output} must be new.`);
            outputs.add(output);
          }
          stages.set(stage.name, partition);
        }
      }),
  })
  .strict() as unknown as z.ZodType<CodeRuleDslEvidenceQuery>;

const codeRuleRelationSchema = codeRuleEvidenceQuerySchema as z.ZodType<CodeRuleDslRelation>;

/** The `$NAME` reference starting at the `$` at `index`, and how many characters it spans; undefined when it is literal text. */
function messageVariableAt(message: string, index: number): { name: string; width: number } | undefined {
  let sigils = 1;
  while (sigils < 3 && message[index + sigils] === "$") sigils += 1;
  const braced = message[index + sigils] === "{";
  if (!braced && index > 0 && message[index - 1] === "\\") return undefined;
  const source = message.slice(index + sigils + (braced ? 1 : 0));
  const name = (braced ? /^([A-Z0-9_]+)\}/ : /^([A-Z0-9_]+)/).exec(source)?.[1];
  if (name === undefined || /^\d/.test(name)) return undefined;
  return { name, width: sigils + name.length + (braced ? 1 : 0) };
}

/**
 * `$NAME` / `$$$NAME` references in a violation message, as the engine reads them: `\\$` is a literal
 * dollar, and a name starting with a digit is literal text.
 */
export function codeRuleMessageVariables(message: string): string[] {
  const names: string[] = [];
  let index = message.indexOf("$");
  while (index !== -1) {
    const variable = messageVariableAt(message, index);
    if (variable !== undefined) names.push(variable.name);
    index = message.indexOf("$", index + (variable?.width ?? 1));
  }
  return names;
}

/** Named groups of a Rust regex (`(?<NAME>…)` or `(?P<NAME>…)`). */
function regexGroupNames(pattern: string): Set<string> {
  return new Set([...pattern.matchAll(/\(\?P?<([A-Za-z_][A-Za-z0-9_]*)>/g)].map(match => match[1]!));
}

/** The query whose witnesses a forbid clause reports: `select_when` reports its `select` side. */
function emittingQuery(query: CodeRuleDslQuery): CodeRuleDslQuery {
  return "select_when" in query && query.select_when !== undefined ? emittingQuery(query.select_when.select) : query;
}

const FIX_NEEDS_SOURCE_TEXT = "Only forbid clauses over an ast or file.content regex query can suggest a fix.";

type ViolationIssue = (path: string, message: string) => void;

/** Anchors and fixes the emitting query's matches cannot carry: no query, a fact, or a file.path regex. */
function checkViolationSource(query: CodeRuleDslQuery | undefined, violation: CodeRuleDslReportingClause["violation"], issue: ViolationIssue): void {
  if (query === undefined) {
    if (violation.anchor !== "witness.span") issue("anchor", "require_each, budget, and llm violations support only witness.span.");
    if (violation.fix !== undefined) issue("fix", FIX_NEEDS_SOURCE_TEXT);
    return;
  }
  if (violation.fix === undefined) return;
  if ("fact" in query && query.fact !== undefined) issue("fix", FIX_NEEDS_SOURCE_TEXT);
  else if ("regex" in query && query.regex !== undefined && query.regex.source === "file.path") {
    issue("fix", "A file.path regex match is not source text a fix can replace.");
  }
}

/** A non-ast violation's anchor and message may name only the regex's named groups. */
function checkViolationCaptures(query: CodeRuleDslQuery | undefined, violation: CodeRuleDslReportingClause["violation"], issue: ViolationIssue): void {
  const regex = query !== undefined && "regex" in query ? query.regex : undefined;
  const groups = regex === undefined ? new Set<string>() : regexGroupNames(regex.pattern);
  const capture = /^capture\.(\w+)$/.exec(violation.anchor)?.[1];
  if (capture !== undefined && !groups.has(capture)) issue("anchor", `capture.${capture} names no capture of the query.`);
  for (const name of codeRuleMessageVariables(violation.message)) {
    if (!groups.has(name)) issue("message", `$${name} names no capture of the query; write \\$${name} for literal text.`);
  }
}

/** A violation's suppress removes an ast query's violations; which linters' comments its language has is the engine's to check. */
function checkViolationSuppress(ast: CodeRuleDslAstQuery["ast"] | undefined, violation: CodeRuleDslReportingClause["violation"], issue: ViolationIssue): void {
  if (violation.suppress !== undefined && ast === undefined) issue("suppress", "suppress removes an ast query's violations.");
}

/**
 * The violation checks the engine makes against the emitting query that need no grammar: fact-based
 * violations carry no source span or captures, and only ast and file.content regex matches are text a
 * fix can replace. Captures of an ast query are left to the engine.
 */
function checkViolationAgainstQuery(clause: CodeRuleDslReportingClause, context: z.RefinementCtx): void {
  const { violation: violation } = clause;
  const issue: ViolationIssue = (path, message) => context.addIssue({ code: "custom", path: ["violation", path], message });
  const query = clause.forbid === undefined ? undefined : emittingQuery(clause.forbid);
  checkViolationSource(query, violation, issue);
  const ast = query !== undefined && "ast" in query ? query.ast : undefined;
  if (ast === undefined) {
    if (typeof violation.fix === "object" && violation.fix.range !== "witness.span") issue("fix", "A fix range names a capture of an ast query.");
    if (violation.each !== undefined) issue("each", "each enumerates the captures of an ast query.");
  }
  checkViolationSuppress(ast, violation, issue);
  if (ast !== undefined) return;
  checkViolationCaptures(query, violation, issue);
}

export const codeRuleDefineSchema = z
  .object({
    report_when: z.object({ count: z.object({ gt: z.number().int().nonnegative().max(Number.MAX_SAFE_INTEGER) }).strict() }).strict().optional(),

    language: z.enum(CODE_RULE_LANGUAGE_IDS).optional(),
    utils: z.record(utilityId, codeRuleUtilitySchema).optional(),
    options: z.record(optionName, codeRuleOptionDeclarationSchema).optional(),
    config: z.record(optionName, codeRuleValueSchema).optional(),
  })
  .strict()
  .refine(define => define.utils !== undefined || define.options !== undefined || define.config !== undefined || define.report_when !== undefined, "define must hold utils, options, config or report_when.")
  .refine(define => define.utils === undefined || define.language !== undefined, {
    message: "language is required: utilities compile for one language.",
    path: ["language"],
  })
  .superRefine((define, context) => {
    if (define.language === undefined || !isCodeRuleAstLanguage(define.language)) return;
    const visit = astRuleVisitor(context);
    for (const [id, util] of Object.entries(define.utils ?? {})) visit(utilityParts(util).rule, ["utils", id, ...utilityParts(util).path]);
  }) as unknown as z.ZodType<CodeRuleDslDefine>;

export const codeRuleClauseSchema = z
  .object({
    id: nonEmptyString,
    violation: codeRuleViolationSpecSchema.optional(),
    forbid: codeRuleQuerySchema.optional(),
    require_each: z.object({ subject: codeRuleRelationSchema, evidence: codeRuleRelationSchema }).strict().optional(),
    budget: z
      .object({
        query: codeRuleRelationSchema,
        group_by: z.array(nonEmptyString).optional(),
        count: z.object({ lte: z.number().int().nonnegative(), distinct: z.array(nonEmptyString).optional() }).strict(),
      })
      .strict()
      .optional(),
    relational: z.object({
      partitionBy: z.enum(["file", "repository"]).optional(),
      locations: z.array(z.string().min(1).max(4096).refine(path => !/[\\:\u0000]/u.test(path) && path.split("/").every(part => part !== "" && part !== "." && part !== ".."),
        "A declared diagnostic location must be a normalized relative path.")).min(1).max(4096).refine(paths => new Set(paths).size === paths.length, "Declared diagnostic locations must be distinct.").optional(),
      inputs: z.record(z.string().regex(/^[A-Za-z_][A-Za-z0-9_]{0,63}$/), codeRuleRelationSchema),
      stages: z.array(z.union([
        z.object({ name: nonEmptyString, query: nonEmptyString, indexes: z.array(z.array(nonEmptyString).min(1).max(16)).max(8).optional(),
          deferred: z.record(z.string().regex(/^[A-Za-z_][A-Za-z0-9_]{0,63}$/), nonEmptyString).refine(fields => Object.keys(fields).length >= 1 && Object.keys(fields).length <= 32 && new Set(Object.keys(fields).map(s => s.toLowerCase())).size === Object.keys(fields).length, "Deferred columns must contain 1 to 32 distinct fields.").optional(),
        }).strict(),
        z.object({ name: nonEmptyString, reachability: z.object({ roots: nonEmptyString, edges: nonEmptyString,
          batchSize: z.number().int().min(1).max(4096).optional(),
          graph: z.enum(["directed", "forest"]).optional(),
        }).strict() }).strict(),
      ])).max(64).optional(),
      references: z.object({ language: z.literal("python"),
        siblingLinks: z.array(z.object({ decorator: nonEmptyString, name: z.object({ replace: nonEmptyString, by: z.string() }).strict() }).strict()).min(1).max(32).optional(),
        constructorFields: z.object({ fieldToParameter: z.object({ replace: nonEmptyString, by: z.string() }).strict() }).strict().optional(),
      }).strict().optional(),
      query: nonEmptyString, key: z.array(nonEmptyString).min(1).max(16).optional(), code: nonEmptyString.optional(),
    }).strict().refine(plan => plan.partitionBy !== "file" || (plan.references === undefined && plan.locations === undefined),
      "File partitions cannot declare references or diagnostic locations.").optional(),
    llm: z.unknown().optional(),
    define: codeRuleDefineSchema.optional(),
  })
  .strict()
  .refine(
    clause => CLAUSE_KINDS.filter(form => clause[form] !== undefined).length === 1,
    `A clause must contain exactly one constraint form: ${CLAUSE_KINDS.join(", ")}.`,
  )
  .superRefine((clause, context) => {
    if (clause.define !== undefined) {
      if (clause.violation !== undefined) {
        context.addIssue({ code: "custom", path: ["violation"], message: "A define clause reports nothing, so it has no violation." });
      }
    } else if (clause.violation === undefined) {
      context.addIssue({ code: "custom", path: ["violation"], message: "violation is required." });
    } else {
      checkViolationAgainstQuery(clause as unknown as CodeRuleDslReportingClause, context);
    }
  }) as unknown as z.ZodType<CodeRuleDslClause>;

/** A `matches` a value makes: the utility, the call's arguments (a parameterized call), and its path. */
interface UtilityCall {
  id: string;
  args: Record<string, string> | undefined;
  path: (string | number)[];
}

/** The calls a value makes with `matches`, anywhere in it. */
function utilityCalls(value: unknown, path: (string | number)[], calls: UtilityCall[]): void {
  if (Array.isArray(value)) {
    value.forEach((item, index) => utilityCalls(item, [...path, index], calls));
  } else if (typeof value === "object" && value !== null) {
    for (const [key, child] of Object.entries(value)) {
      if (key === "matches" && typeof child === "string") calls.push({ id: child, args: undefined, path: [...path, key] });
      if (key === "matches" && typeof child === "object" && child !== null && Object.keys(child).length === 1) {
        const [id, args] = Object.entries(child as Record<string, Record<string, string>>)[0]!;
        calls.push({ id, args, path: [...path, key, id] });
      }
      utilityCalls(child, [...path, key], calls);
    }
  }
}

/** The ast queries of a query, with their paths: a `select_when`'s two sides too. */
function astQueries(query: CodeRuleDslQuery, path: (string | number)[]): [CodeRuleDslAstQuery["ast"], (string | number)[]][] {
  if ("ast" in query && query.ast !== undefined) return [[query.ast, [...path, "ast"]]];
  if ("select_when" in query && query.select_when !== undefined) {
    return [...astQueries(query.select_when.select, [...path, "select_when", "select"]), ...astQueries(query.select_when.exists, [...path, "select_when", "exists"])];
  }
  return [];
}

type SharedUtilities = Map<string, Map<string, CodeRuleDslUtility>>;

/** Each language's define utilities, reporting an id a second define of the language repeats. */
function sharedUtilities(rule: CodeRuleDslDocument, context: z.RefinementCtx): SharedUtilities {
  const shared: SharedUtilities = new Map();
  for (const [index, clause] of rule.clauses.entries()) {
    const define = clause.define;
    if (define?.language === undefined) continue;
    const known = shared.get(define.language) ?? new Map<string, CodeRuleDslUtility>();
    shared.set(define.language, known);
    for (const [id, util] of Object.entries(define.utils ?? {})) {
      if (known.has(id)) {
        context.addIssue({ code: "custom", path: ["clauses", index, "define", "utils", id], message: `Utility rule ${id} is already defined by another define of ${define.language}.` });
      }
      known.set(id, util);
    }
  }
  return shared;
}

/** Whether `matches` names a utility, and passes exactly the parameters it takes. */
function checkUtilityCall({ id, args, path }: UtilityCall, util: CodeRuleDslUtility | undefined, context: z.RefinementCtx): void {
  const issue = (message: string, at: (string | number)[] = path): void => context.addIssue({ code: "custom", path: at, message });
  if (util === undefined) return issue(`Utility rule ${id} is not defined in utils or a define of the query's language.`);
  const { params } = utilityParts(util);
  if (args === undefined) {
    if (params.length > 0) issue(`Utility rule ${id} takes parameters (${params.join(", ")}); call it as { "${id}": { "${params[0]}": "$NAME" } }.`);
    return;
  }
  if (params.length === 0) return issue(`Utility rule ${id} takes no parameters; name it.`);
  for (const key of Object.keys(args)) {
    if (!params.includes(key)) issue(`${key} is not a parameter of ${id} (${params.join(", ")}).`, [...path, key]);
  }
  for (const param of params) {
    if (!Object.hasOwn(args, param)) issue(`Parameter ${param} of ${id} is not passed; map it to a capture ("${param}": "$NAME").`);
  }
}

/** The define utilities `calls` reach, directly or through the utilities they call. */
function reachedUtilities(calls: UtilityCall[], known: Map<string, CodeRuleDslUtility>): Set<string> {
  const reached = new Set<string>();
  const pending = calls.map(call => call.id);
  for (let id = pending.pop(); id !== undefined; id = pending.pop()) {
    const util = known.get(id);
    if (util === undefined || reached.has(id)) continue;
    reached.add(id);
    const nested: UtilityCall[] = [];
    utilityCalls(util, [], nested);
    pending.push(...nested.map(call => call.id));
  }
  return reached;
}

/** An ast query's calls, against its own utils and its language's define utilities. */
function checkAstQueryUtilities(ast: CodeRuleDslAstQuery["ast"], astPath: (string | number)[], shared: SharedUtilities, context: z.RefinementCtx): void {
  const known = shared.get(ast.language) ?? new Map<string, CodeRuleDslUtility>();
  const local: Record<string, CodeRuleDslUtility> = ast.utils ?? {};
  const calls: UtilityCall[] = [];
  utilityCalls(ast.rule, [...astPath, "rule"], calls);
  utilityCalls(ast.utils, [...astPath, "utils"], calls);
  utilityCalls(ast.constraints, [...astPath, "constraints"], calls);
  for (const id of reachedUtilities(calls, known)) {
    if (Object.hasOwn(local, id)) {
      context.addIssue({ code: "custom", path: [...astPath, "utils", id], message: `Utility rule ${id} is also defined by a define of ${ast.language}; rename one.` });
    }
  }
  for (const call of calls) checkUtilityCall(call, Object.hasOwn(local, call.id) ? local[call.id] : known.get(call.id), context);
}

/**
 * The cross-references between clauses: a define's utilities are visible to every ast query of its
 * language, so whether `matches` names a utility, and passes the parameters it takes, is a question
 * of the document.
 */
function checkDocumentUtilities(rule: CodeRuleDslDocument, context: z.RefinementCtx): void {
  const shared = sharedUtilities(rule, context);
  for (const [index, clause] of rule.clauses.entries()) {
    const define = clause.define;
    if (define?.language !== undefined) {
      const known = shared.get(define.language)!;
      const calls: UtilityCall[] = [];
      utilityCalls(define.utils, ["clauses", index, "define", "utils"], calls);
      for (const call of calls) checkUtilityCall(call, known.get(call.id), context);
    }
    if (clause.forbid === undefined) continue;
    for (const [ast, astPath] of astQueries(clause.forbid, ["clauses", index, "forbid"])) checkAstQueryUtilities(ast, astPath, shared, context);
  }
}


/** One `when.option` condition: the option is declared, and its value can meet the condition. */
function checkWhenOption(values: Map<string, CodeRuleDslValue>, name: string, condition: CodeRuleDslOptionCondition, path: (string | number)[], context: z.RefinementCtx): void {
  if (!values.has(name)) {
    context.addIssue({ code: "custom", path, message: `\`${name}\` is not an option this document declares.` });
    return;
  }
  try {
    codeRuleOptionConditionHolds(values.get(name)!, condition, path);
  } catch (error) {
    if (!(error instanceof CodeRuleOptionError)) throw error;
    context.addIssue({ code: "custom", path: error.path, message: `${error.message}.` });
  }
}

/** Each ast query's `when.option` names options the document declares, with conditions their values can meet. */
function checkWhenOptions(rule: CodeRuleDslDocument, context: z.RefinementCtx): void {
  let values: Map<string, CodeRuleDslValue>;
  try {
    values = codeRuleOptionValues(rule);
  } catch {
    return;
  }
  for (const [index, clause] of rule.clauses.entries()) {
    if (clause.forbid === undefined) continue;
    for (const [ast, astPath] of astQueries(clause.forbid, ["clauses", index, "forbid"])) {
      for (const [name, condition] of Object.entries(ast.when?.option ?? {})) checkWhenOption(values, name, condition, [...astPath, "when", "option", name], context);
    }
  }
}

/** The document as written, `$option` references and all; `codeRuleSchema` resolves them first. */
export const codeRuleResolvedRuleSchema = z
  .object({
    format: z.literal(CODE_RULE_FORMAT),
    id: nonEmptyString,
    revision: z.number().int().positive(),
    mode: z.enum(RULE_MODES),
    description: z.string().optional(),
    scope: codeRuleScopeSchema.optional(),
    clauses: z.array(codeRuleClauseSchema).min(1),
  })
  .strict()
  .superRefine((rule, context) => {
    const ids = new Set<string>();
    if (rule.clauses.filter(clause => clause.define?.report_when !== undefined).length > 1) {
      context.addIssue({ code: "custom", path: ["clauses"], message: "report_when must be declared once per document." });
    }
    for (const [index, clause] of rule.clauses.entries()) {
      if (ids.has(clause.id)) {
        context.addIssue({ code: "custom", path: ["clauses", index, "id"], message: `Duplicate clause id: ${clause.id}.` });
      }
      ids.add(clause.id);
    }
    if (rule.clauses.every(clause => clause.define !== undefined)) {
      context.addIssue({ code: "custom", path: ["clauses"], message: "clauses must hold a clause besides define: a define reports nothing." });
    }
    checkDocumentUtilities(rule as unknown as CodeRuleDslDocument, context);
    checkWhenOptions(rule as unknown as CodeRuleDslDocument, context);
  }) as unknown as z.ZodType<CodeRuleDslDocument>;

/**
 * A `rerail.policy/v1` document. Its `$option` references are resolved first, as the engine resolves
 * them (`resolveCodeRuleOptions`), so a parsed document holds none: the catalog stores it resolved,
 * with the configuration its defines' `config` records.
 */
export const codeRuleSchema = z.preprocess((input, context) => {
  try {
    return resolveCodeRuleOptions(input);
  } catch (error) {
    if (!(error instanceof CodeRuleOptionError)) throw error;
    context.addIssue({ code: "custom", path: error.path, message: `${error.message}.`, input });
    return input;
  }
}, codeRuleResolvedRuleSchema) as unknown as z.ZodType<CodeRuleDslDocument>;

export type CodeRuleScope = CodeRuleDslScope;
export type CodeRuleViolationSpec = CodeRuleDslViolationSpec;
export type CodeRuleClause = CodeRuleDslClause;
export type CodeRule = CodeRuleDslDocument;

/** The clause kind a clause uses; the schema guarantees exactly one. */
export function clauseKind(clause: CodeRuleClause): ClauseKind {
  return CLAUSE_KINDS.find(form => clause[form] !== undefined) ?? "forbid";
}
