import { z } from "zod";
import type {
  PolicyDslAstCount,
  PolicyDslAstRelation,
  PolicyDslCompare,
  PolicyDslAstRule,
  PolicyDslAstQuery,
  PolicyDslCheck,
  PolicyDslDefine,
  PolicyDslReportingCheck,
  PolicyDslDocument,
  PolicyDslEvidenceQuery,
  PolicyDslRelation,
  PolicyDslFindingSpec,
  PolicyDslNthChild,
  PolicyDslOptionCondition,
  PolicyDslOptionDeclaration,
  PolicyDslTransformation,
  PolicyDslUtility,
  PolicyDslPredicate,
  PolicyDslQuery,
  PolicyDslScope,
  PolicyDslValue,
} from "./executable-policy.js";
import { RULE_MODES } from "./rule-mode.js";
import type {
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
} from "./executable-policy-semantic.js";
import {
  POLICY_SEMANTIC_COMPARE_TEXTS,
  policySuppressSchema,
  semanticRuleShape,
} from "./policy-schema-semantic.js";
import { isPolicyOptionName, PolicyOptionError, policyOptionConditionHolds, policyOptionValues, resolvePolicyOptions } from "./policy-options.js";
import {
  POLICY_AST_LANGUAGES,
  POLICY_LANGUAGE_FAMILIES,
  policyLanguageFamily,
  policyLanguageImplements,
  policyLanguageSuppressions,
} from "./policy-languages.js";
import type { PolicyAstLanguage, PolicyLanguageFamily, PolicySemanticCapability, PolicySuppression } from "./policy-languages.js";

const [PYTHON_FAMILY, ECMA_FAMILY, PHP_FAMILY, BLADE_FAMILY] = POLICY_LANGUAGE_FAMILIES;

/** Whether a family's values, arguments and variables are PHP's: PHP's, and those of the PHP a Blade template embeds (the kernel's `Family::reads_php`). */
const readsPhp = (family: PolicyLanguageFamily): boolean => family === PHP_FAMILY || family === BLADE_FAMILY;

export { POLICY_AST_LANGUAGES };
export type { PolicyAstLanguage };


/**
 * Zod for `rerail.policy/v1`, the executable policy document the native kernel
 * compiles and evaluates. The TypeScript types in `executable-policy.ts` are
 * the contract; every schema here is declared against them so a drift between
 * the two is a compile error. This is the shape the `rules` table stores for
 * `kind = 'file'` rows; every top-level field maps to a column (`schema.ts`).
 *
 * ast-grep patterns and fact relations are checked structurally only; the
 * kernel remains the authority on whether a pattern parses for its grammar, a
 * relation exists in the workspace capabilities, or a pattern fits the
 * capabilities' `maxAstPatternBytes` / `maxRegexPatternBytes` limits. Anything
 * this schema rejects, the kernel rejects too.
 */

export const POLICY_FORMAT = "rerail.policy/v1" as const;

export const POLICY_REGEX_SOURCES = ["file.content", "file.path"] as const;
export type PolicyRegexSource = (typeof POLICY_REGEX_SOURCES)[number];
export const POLICY_CHECK_FORMS = ["forbid", "require_each", "budget", "llm", "define"] as const;
export type PolicyCheckForm = (typeof POLICY_CHECK_FORMS)[number];
export const POLICY_STOP_BY = ["neighbor", "end"] as const;
export const POLICY_COMPARE_OPS = ["lt", "le", "eq", "ne", "gt", "ge"] as const;
export const POLICY_COMPARE_COLLATIONS = ["ordinal", "fold-lower", "fold-upper", "lowercase-first", "natural", "natural-fold-lower"] as const;
export const POLICY_COMPARE_TEXTS = ["source", "unquoted", ...POLICY_SEMANTIC_COMPARE_TEXTS, "tokens", "node", "index"] as const;
/** The compare texts that test equality only, under the ordinal collation. */
const POLICY_EQUALITY_TEXTS: readonly string[] = ["tokens", "structure", "structure-hash", "structure-parts", "node", "reference", "reference-no-static-keys"];
/** The compare texts that read Python syntax (Ruff's structural equalities). */
const POLICY_STRUCTURE_TEXTS: readonly string[] = ["structure", "structure-hash", "structure-parts"];
/** `witness.span` | `witness.start` | `witness.end` | `file.start` (byte 0 of the file) | `capture.<NAME>`; the kernel checks capture names against the query. */
export const POLICY_FINDING_ANCHOR_PATTERN = /^(?:witness\.(?:span|start|end)|file\.start|capture\.\w+)$/;
export { policySuppressSchema };


const nonEmptyString = z.string().refine(value => value.trim().length > 0, "Must be a non-empty string.");

export const policyScopeSchema = z
  .object({
    include: z.array(nonEmptyString).min(1),
    exclude: z.array(nonEmptyString),
  })
  .strict() as unknown as z.ZodType<PolicyDslScope>;

export const policyValueSchema: z.ZodType<PolicyDslValue> = z.lazy(() =>
  z.union([z.boolean(), z.number(), z.string(), z.null(), z.array(policyValueSchema), z.record(z.string(), policyValueSchema)]),
);


const policyScalarSchema = z.union([z.boolean(), z.number(), z.string(), z.null()]);
export const policyOptionConditionSchema: z.ZodType<PolicyDslOptionCondition> = z.lazy(() =>
  z.union([
    policyScalarSchema,
    z.array(policyScalarSchema),
    z.object({ in: z.array(policyValueSchema) }).strict(),
    z.object({ includes: policyScalarSchema }).strict(),
    z.object({ not: policyOptionConditionSchema }).strict(),
  ]) as unknown as z.ZodType<PolicyDslOptionCondition>,
);
const optionName = z.string().refine(isPolicyOptionName, "An option's name starts with a letter or _ and holds letters, digits, _, - and .");
export const policyOptionDeclarationSchema = z
  .object({
    type: z.enum(["boolean", "string", "number", "integer", "array"]),
    items: z.enum(["string", "number", "integer", "boolean"]).optional(),
    enum: z.array(policyValueSchema).min(1).optional(),
    nullable: z.boolean().optional(),
    default: policyValueSchema,
    description: z.string().optional(),
  })
  .strict() as unknown as z.ZodType<PolicyDslOptionDeclaration>;

export const policyFindingSchema = z
  .object({
    code: nonEmptyString,
    anchor: z.string().regex(POLICY_FINDING_ANCHOR_PATTERN, "Anchor must be witness.span, witness.start, witness.end, file.start, or capture.<NAME>."),
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
    suppress: policySuppressSchema.optional(),
    variables: z
      .record(z.string(), policyValueSchema)
      .refine(variables => Object.keys(variables).length === 0, "Violation variables are not supported yet.")
      .optional(),
  })
  .strict() as unknown as z.ZodType<PolicyDslFindingSpec>;

export type PolicyAstRule = PolicyDslAstRule;
export type PolicyAstRelation = PolicyDslAstRelation;

const AST_RULE_KEYS = ["pattern", "kind", "regex", "nthChild", "all", "any", "not", "matches", "inside", "has", "precedes", "follows", "compare", "count", "resolves", "value", "arguments", "binds", "parse", "capture", "formatArgs", "layout", "host", "calls"] as const;
/** The grammars `parse` applies and their kinds (the kernel's `literal::Grammar`). */
export const POLICY_PARSE_GRAMMARS = ["percentFormat", "braceFormat", "annotation", "eslintDirective", "printfFormat", "scanfFormat", "phpDoc"] as const;
export const POLICY_PARSE_KINDS: Record<(typeof POLICY_PARSE_GRAMMARS)[number], readonly string[]> = {
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
  // Python's kinds, which the kernel checks when it compiles the rule.
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

export const policyCompareSchema = z
  .object({
    left: metavariableName,
    op: z.enum(POLICY_COMPARE_OPS),
    right: metavariableName,
    collation: z.enum(POLICY_COMPARE_COLLATIONS).optional(),
    text: z.enum(POLICY_COMPARE_TEXTS).optional(),
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
    if (compare.text === undefined || !POLICY_EQUALITY_TEXTS.includes(compare.text)) return;
    if (compare.op !== "eq" && compare.op !== "ne") {
      context.addIssue({ code: "custom", path: ["op"], message: `${compare.text} compares for equality only; use eq or ne.` });
    }
    if ((compare.collation ?? "ordinal") !== "ordinal") {
      const what = equalitySubject(compare.text);
      context.addIssue({ code: "custom", path: ["collation"], message: `${compare.text} compares ${what} exactly; use ordinal.` });
    }
  }) as unknown as z.ZodType<PolicyDslCompare>;

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
export const policyTransformationSchema = z.union([
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
]) as unknown as z.ZodType<PolicyDslTransformation>;

/** The transformation's body: its source and settings. */
function transformBody(transformation: PolicyDslTransformation): { source: string; value?: boolean } {
  if ("replace" in transformation) return transformation.replace;
  if ("substring" in transformation) return transformation.substring;
  if ("convert" in transformation) return transformation.convert;
  if ("line" in transformation) return transformation.line;
  if ("lines" in transformation) return transformation.lines;
  return transformation.count;
}

/** The transform variable a transformation reads, `$` sigils aside. */
function transformSourceName(transformation: PolicyDslTransformation): string {
  return transformBody(transformation).source.replace(/^\$+/u, "");
}

/** Whether a transformation reads its capture's node (lines, counts) rather than its text. */
const readsNode = (transformation: PolicyDslTransformation): boolean => "line" in transformation || "lines" in transformation || "count" in transformation;

/** Python's type names for `value.type` (the kernel's `PYTHON_TYPES`). */
export const POLICY_PYTHON_VALUE_TYPES = ["int", "float", "complex", "str", "bytes", "bool", "NoneType", "ellipsis", "tuple", "list", "set", "dict"] as const;
/** TypeScript's type names for `value.type` (the kernel's `TYPESCRIPT_TYPES`). */
export const POLICY_TYPESCRIPT_VALUE_TYPES = ["string", "number", "bigint", "boolean", "null", "undefined", "array", "object", "regexp"] as const;
/** PHP's `get_debug_type` names for `value.type` (the kernel's `PHP_TYPES`). */
export const POLICY_PHP_VALUE_TYPES = ["int", "float", "string", "bool", "null", "array"] as const;
const valueStrings = z.union([nonEmptyString, z.array(nonEmptyString).min(1).max(1024)]);
export const policyValueTestSchema = z
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
  .refine(value => Object.keys(value).some(key => key !== "follow"), "value must hold a predicate: eq, type, constant, startsWith, regex, bitsAny, gt, ge, lt, le or truthy.") as unknown as z.ZodType<PolicyDslValueTest>;

/** A signature entry: a slot name (an identifier), `/` or `*`. */
const signatureEntry = z.string().max(256).regex(/^(?:[A-Za-z_$][A-Za-z0-9_$]*|\/|\*)$/, "Must be a slot name (an identifier), / or *.");
const slotNames = (signature: readonly string[]): string[] => signature.filter(entry => entry !== "/" && entry !== "*");
export const policyArgumentsSchema = z
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
  .refine(entry => (entry.absent ?? []).every(slot => !Object.values(entry.bind ?? {}).includes(slot)), { message: "A bound slot is passed, so it cannot be absent.", path: ["absent"] }) as unknown as z.ZodType<PolicyDslArguments>;

/** ESLint's `LATEST_ECMA_VERSION` in the kernel's pinned globals table (ESLint 10.11.0): what `"latest"` selects. */
export const POLICY_LATEST_ECMA_VERSION = 2026 as const;
export const POLICY_SOURCE_TYPES = ["script", "module", "commonjs"] as const;
/** Globals one `languageOptions` may configure (the kernel's bound): room for real environment unions. */
export const POLICY_MAX_LANGUAGE_GLOBALS = 16384 as const;
/** The Python versions of the kernel's pinned builtins table (Ruff 0.16.9's target versions). */
export const POLICY_PYTHON_VERSIONS = ["3.7", "3.8", "3.9", "3.10", "3.11", "3.12", "3.13", "3.14", "3.15"] as const;
const PYTHON_ONLY_OPTIONS = ["pythonVersion", "typingModules", "extendGenerics"] as const;
/** Names `typingModules` or `extendGenerics` may list (the kernel's bound). */
export const POLICY_MAX_TYPING_NAMES = 256 as const;
const TYPESCRIPT_ONLY_OPTIONS = ["ecmaVersion", "sourceType", "globals", "parserOptions", "jsxPragma", "jsxFragmentName", "lib"] as const;
/** ESLint's global settings with its aliases (`normalizeConfigGlobal`). */
const POLICY_GLOBAL_SETTINGS = ["readonly", "writable", "off", "readable", "writeable", "true", "false"] as const;

/** An identifier: ASCII letters, digits, `_` and `$`, not starting with a digit (the kernel's check). */
const resolveIdentifier = z.string().max(256).regex(/^[A-Za-z_$][A-Za-z0-9_$]*$/, "Must be an identifier (A-Z, a-z, 0-9, _, $).");
/**
 * A Python dotted name, or a PHP name: labels (a letter, `_` or any non-ASCII character, then those or digits:
 * the kernel's `php_names::is_label`) joined by single `\`, optionally fully qualified. Each family refuses the
 * other's shape. The kernel reads Unicode scalar values (no lone surrogate) and bounds a name's UTF-8 bytes.
 */
const RESOLVE_DOTTED_NAME = /^[A-Za-z_$][A-Za-z0-9_$]*(\.[A-Za-z_$][A-Za-z0-9_$]*)*$/;
const PHP_LABEL = String.raw`[A-Za-z_\u0080-\ud7ff\ue000-\u{10ffff}][A-Za-z0-9_\u0080-\ud7ff\ue000-\u{10ffff}]*`;
const RESOLVE_PHP_NAME = new RegExp(String.raw`^\\?${PHP_LABEL}(?:\\${PHP_LABEL})*$`, "u");
const UTF8 = new TextEncoder();
const resolveDottedName = z
  .string()
  .max(256)
  .refine(name => (RESOLVE_DOTTED_NAME.test(name) || RESOLVE_PHP_NAME.test(name)) && UTF8.encode(name).length <= 256, {
    message: String.raw`Must be a dotted name (identifiers joined by .) or a PHP name (labels joined by \) of at most 256 bytes.`,
  });
/** A Python dotted name: ASCII identifiers without `$` joined by `.` (the kernel's check). */
const pythonDottedName = z.string().max(256).regex(/^[A-Za-z_][A-Za-z0-9_]*(\.[A-Za-z_][A-Za-z0-9_]*)*$/, "Must be a dotted name (identifiers joined by .).");
/** The binding forms Python and TypeScript bindings are made in (the kernel's `FORMS`). */
const POLICY_SHARED_BINDING_FORMS = [
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
] as const;
/** The forms a PHP variable is bound in (the kernel's `PHP_FORMS`). */
export const POLICY_PHP_BINDING_FORMS = ["assignment", "augmented", "for", "except", "parameter", "global", "static"] as const;
/** Every binding form a binding filter names; each family narrows to its own. */
export const POLICY_BINDING_FORMS = [...POLICY_SHARED_BINDING_FORMS, "static"] as const;
export const POLICY_PYTHON_SCOPE_KINDS = ["module", "class", "function", "lambda", "comprehension", "typeParameters"] as const;
/** TypeScript's scope kinds (the kernel's): `global` is a script's top level, `module` a module's or CommonJS file's. */
export const POLICY_TYPESCRIPT_SCOPE_KINDS = ["global", "module", "function", "class", "block", "namespace", "typeParameters", "functionName"] as const;
/** PHP's variable scopes (the kernel's `PHP_SCOPES`): the file's top level, a function or method, a closure or arrow function. */
export const POLICY_PHP_SCOPE_KINDS = ["global", "function", "closure"] as const;
/** Every scope kind a binding filter names, Python's first (the kernel's order). */
export const POLICY_SCOPE_KINDS = [...new Set([...POLICY_PYTHON_SCOPE_KINDS, ...POLICY_TYPESCRIPT_SCOPE_KINDS, ...POLICY_PHP_SCOPE_KINDS])] as unknown as readonly [
  ...typeof POLICY_PYTHON_SCOPE_KINDS,
  "global",
  "block",
  "namespace",
  "functionName",
  "closure",
];
const bindingFilterShape = {
  form: z.union([z.enum(POLICY_BINDING_FORMS), z.array(z.enum(POLICY_BINDING_FORMS)).min(1)]).optional(),
  scope: z.union([z.enum(POLICY_SCOPE_KINDS), z.array(z.enum(POLICY_SCOPE_KINDS)).min(1)]).optional(),
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
  site: z.lazy(() => policyAstRuleSchema).optional(),
  value: z.lazy(() => policyAstRuleSchema).optional(),
  annotation: z.lazy(() => policyAstRuleSchema).optional(),
};
const occurrenceShape = { occurrence: z.lazy(() => policyAstRuleSchema).optional() };
export const policyBindsSchema = z
  .object({
    ...bindingFilterShape,
    ...bindingRulesShape,
    ...occurrenceShape,
    redefines: z
      .object({ ...bindingFilterShape, ...bindingRulesShape, sameScope: z.boolean().optional(), sameImport: z.boolean().optional(), every: z.boolean().optional() })
      .strict()
      .optional(),
  })
  .strict() as unknown as z.ZodType<PolicyDslBinds>;

export const policyResolvesSchema = z
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
  ) as unknown as z.ZodType<PolicyDslResolves>;

/** `ecmaVersion`: "latest", 3, 5, 6 to 17 (ES2015 to ES2026), or a year the kernel's table has. */
const isEcmaVersion = (version: number): boolean =>
  version === 3 || version === 5 || (version >= 6 && version <= POLICY_LATEST_ECMA_VERSION - 2009) || (version >= 2015 && version <= POLICY_LATEST_ECMA_VERSION);
const ecmaVersionValue = z.union([
  z.literal("latest"),
  z.number().int().refine(isEcmaVersion, `ecmaVersion must be "latest", 3, 5, 6 to ${POLICY_LATEST_ECMA_VERSION - 2009}, or 2015 to ${POLICY_LATEST_ECMA_VERSION}.`),
]);
const ecmaYear = (version: number | "latest"): number => {
  if (version === "latest") return POLICY_LATEST_ECMA_VERSION;
  return version >= 6 && version < 2015 ? version + 2009 : version;
};
const pythonMinor = (version: string): number => Number(version.slice(2));
const versionRange = <T extends z.ZodTypeAny>(value: T, order: (version: z.infer<T>) => number) =>
  z
    .object({ min: value.optional(), max: value.optional() })
    .strict()
    .refine(range => range.min !== undefined || range.max !== undefined, "Set min, max or both.")
    .refine(range => range.min === undefined || range.max === undefined || order(range.min) <= order(range.max), "min is after max, so the query never applies.");
export const policyAstWhenSchema = z
  .object({
    pythonVersion: versionRange(z.enum(POLICY_PYTHON_VERSIONS), pythonMinor).optional(),
    ecmaVersion: versionRange(ecmaVersionValue, ecmaYear).optional(),
    sourceType: z.union([z.enum(POLICY_SOURCE_TYPES), z.array(z.enum(POLICY_SOURCE_TYPES)).min(1)]).optional(),
    ecmaFeatures: z
      .object({ globalReturn: z.boolean().optional(), impliedStrict: z.boolean().optional() })
      .strict()
      .refine(features => features.globalReturn !== undefined || features.impliedStrict !== undefined, "Set globalReturn, impliedStrict or both.")
      .optional(),
    option: z.record(z.string(), policyOptionConditionSchema).optional(),
  })
  .strict() as unknown as z.ZodType<PolicyDslAstWhen>;
const moduleRegex = z
  .object({ regex: nonEmptyString.optional(), notRegex: nonEmptyString.optional() })
  .strict()
  .refine(condition => condition.regex !== undefined || condition.notRegex !== undefined, "Set regex, notRegex or both.");
/** The `file` keys that read a Python file's module path. */
const PYTHON_FILE_KEYS = ["module", "moduleName", "package", "inPackage"] as const;
export const policyFileConditionSchema = z
  .object({
    path: moduleRegex.optional(),
    declaration: z.boolean().optional(),
    module: moduleRegex.optional(),
    moduleName: moduleRegex.optional(),
    package: moduleRegex.optional(),
    inPackage: z.boolean().optional(),
  })
  .strict()
  .refine(file => Object.values(file).some(value => value !== undefined), "Set path, declaration, module, moduleName, package or inPackage.") as unknown as z.ZodType<PolicyDslFileCondition>;
export const policyLanguageOptionsSchema = z
  .object({
    ecmaVersion: z
      .union([z.literal("latest"), z.number().int().refine(isEcmaVersion, `ecmaVersion must be "latest", 3, 5, 6 to ${POLICY_LATEST_ECMA_VERSION - 2009}, or 2015 to ${POLICY_LATEST_ECMA_VERSION}.`)])
      .optional(),
    sourceType: z.enum(POLICY_SOURCE_TYPES).optional(),
    globals: z
      .record(resolveIdentifier, z.union([z.enum(POLICY_GLOBAL_SETTINGS), z.boolean(), z.null()]))
      .refine(globals => Object.keys(globals).length <= POLICY_MAX_LANGUAGE_GLOBALS, `At most ${POLICY_MAX_LANGUAGE_GLOBALS} globals.`)
      .optional(),
    parserOptions: z
      .object({ ecmaFeatures: z.object({ globalReturn: z.boolean().optional(), impliedStrict: z.boolean().optional() }).strict().optional() })
      .strict()
      .optional(),
    jsxPragma: resolveIdentifier.nullable().optional(),
    jsxFragmentName: resolveIdentifier.nullable().optional(),
    /** typescript-eslint's lib names; the kernel refuses a name its pinned table lacks. */
    lib: z.array(z.string().regex(/^[A-Za-z0-9.]+$/, "Must be a TypeScript lib name.")).optional(),
    pythonVersion: z.enum(POLICY_PYTHON_VERSIONS).optional(),
    typingModules: z.array(pythonDottedName).max(POLICY_MAX_TYPING_NAMES).optional(),
    extendGenerics: z.array(pythonDottedName).max(POLICY_MAX_TYPING_NAMES).optional(),
  })
  .strict()
  .refine(
    options => (options.sourceType ?? "module") !== "module" || options.ecmaVersion === undefined || options.ecmaVersion === "latest" || options.ecmaVersion >= 6,
    'sourceType "module" (the default) needs ecmaVersion 2015 or later; ESLint refuses it.',
  ) as unknown as z.ZodType<PolicyDslLanguageOptions>;

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
export const policyLayoutSchema = z
  .object({ span: lineCount.optional(), start: lineCount.optional(), before: layoutGap.optional(), after: layoutGap.optional() })
  .strict()
  .refine(layout => Object.values(layout).some(value => value !== undefined), "layout must hold span, start, before or after.") as unknown as z.ZodType<PolicyDslLayout>;

export const policyNthChildSchema: z.ZodType<PolicyDslNthChild> = z.lazy(() =>
  z.union([
    nthChildPosition,
    z.object({ position: nthChildPosition, ofRule: policyAstRuleSchema.optional(), reverse: z.boolean().optional() }).strict(),
  ]) as unknown as z.ZodType<PolicyDslNthChild>,
);
export const policyAstRuleSchema: z.ZodType<PolicyDslAstRule> = z.lazy(() =>
  z
    .object({
      pattern: z.union([nonEmptyString, z.object({ context: nonEmptyString, selector: nonEmptyString }).strict()]).optional(),
      kind: nonEmptyString.optional(),
      regex: nonEmptyString.optional(),
      nthChild: policyNthChildSchema.optional(),
      all: z.array(policyAstRuleSchema).min(1).optional(),
      any: z.array(policyAstRuleSchema).min(1).optional(),
      not: policyAstRuleSchema.optional(),
      matches: matchesField,
      inside: policyAstRelationSchema.optional(),
      has: policyAstRelationSchema.optional(),
      precedes: policyAstRelationSchema.optional(),
      follows: policyAstRelationSchema.optional(),
      compare: policyCompareSchema.optional(),
      count: policyAstCountSchema.optional(),
      ...semanticRuleShape({
        resolves: policyResolvesSchema,
        value: policyValueTestSchema,
        arguments: policyArgumentsSchema,
        binds: policyBindsSchema,
        parse: policyParseSchema,
        capture: metavariableName,
        formatArgs: policyFormatArgsSchema,
        layout: policyLayoutSchema,
        host: policyAstRuleSchema,
        calls: policyCallsSchema,
      }),
    })
    .strict()
    .refine(rule => AST_RULE_KEYS.some(key => rule[key] !== undefined), "An ast rule must contain at least one matcher.") as unknown as z.ZodType<PolicyDslAstRule>,
);
export const policyAstRelationSchema: z.ZodType<PolicyDslAstRelation> = z.lazy(() =>
  z
    .object({
      stopBy: z.union([z.enum(POLICY_STOP_BY), policyAstRuleSchema]).optional(),
      field: nonEmptyString.optional(),
      pattern: z.union([nonEmptyString, z.object({ context: nonEmptyString, selector: nonEmptyString }).strict()]).optional(),
      kind: nonEmptyString.optional(),
      regex: nonEmptyString.optional(),
      nthChild: policyNthChildSchema.optional(),
      all: z.array(policyAstRuleSchema).min(1).optional(),
      any: z.array(policyAstRuleSchema).min(1).optional(),
      not: policyAstRuleSchema.optional(),
      matches: matchesField,
      inside: policyAstRelationSchema.optional(),
      has: policyAstRelationSchema.optional(),
      precedes: policyAstRelationSchema.optional(),
      follows: policyAstRelationSchema.optional(),
      compare: policyCompareSchema.optional(),
      count: policyAstCountSchema.optional(),
      ...semanticRuleShape({
        resolves: policyResolvesSchema,
        value: policyValueTestSchema,
        arguments: policyArgumentsSchema,
        binds: policyBindsSchema,
        parse: policyParseSchema,
        capture: metavariableName,
        formatArgs: policyFormatArgsSchema,
        layout: policyLayoutSchema,
        host: policyAstRuleSchema,
        calls: policyCallsSchema,
      }),
    })
    .strict()
    .refine(rule => AST_RULE_KEYS.some(key => rule[key] !== undefined), "An ast relation must contain at least one matcher.") as unknown as z.ZodType<PolicyDslAstRelation>,
);
/** `calls`: its inner rule's keys, and how chains pass and what they bind. */
export const policyCallsSchema: z.ZodType<PolicyDslCalls> = z.lazy(
  () =>
    z
      .object({ through: z.enum(["sync", "any"]).optional(), chain: metavariableName.optional() })
      .passthrough()
      .superRefine((calls, context) => {
        const { through: _through, chain: _chain, ...inner } = calls;
        const parsed = policyAstRuleSchema.safeParse(inner);
        if (!parsed.success) {
          for (const issue of parsed.error.issues) context.addIssue({ ...issue, code: "custom", message: issue.message });
        }
      }) as unknown as z.ZodType<PolicyDslCalls>,
);
export const policyFormatArgsSchema: z.ZodType<PolicyDslFormatArgs> = z.lazy(() =>
  z
    .object({
      style: z.enum(["brace", "percent", "printf", "scanf"]),
      string: metavariableName,
      args: metavariableName,
      unused: policyAstRuleSchema.optional(),
      missing: policyAstRuleSchema.optional(),
    })
    .strict()
    .refine(test => test.unused !== undefined || test.missing !== undefined, "formatArgs must hold unused, missing or both."),
) as unknown as z.ZodType<PolicyDslFormatArgs>;
export const policyParseSchema: z.ZodType<PolicyDslParse> = z.lazy(() =>
  z
    .object({
      grammar: z.string(),
      pattern: z.union([nonEmptyString, z.object({ context: nonEmptyString, selector: nonEmptyString }).strict()]).optional(),
      kind: nonEmptyString.optional(),
      regex: nonEmptyString.optional(),
      nthChild: policyNthChildSchema.optional(),
      all: z.array(policyAstRuleSchema).min(1).optional(),
      any: z.array(policyAstRuleSchema).min(1).optional(),
      not: policyAstRuleSchema.optional(),
      matches: matchesField,
      inside: policyAstRelationSchema.optional(),
      has: policyAstRelationSchema.optional(),
      precedes: policyAstRelationSchema.optional(),
      follows: policyAstRelationSchema.optional(),
      compare: policyCompareSchema.optional(),
      count: policyAstCountSchema.optional(),
      ...semanticRuleShape({
        resolves: policyResolvesSchema,
        value: policyValueTestSchema,
        arguments: policyArgumentsSchema,
        binds: policyBindsSchema,
        parse: policyParseSchema,
        capture: metavariableName,
        formatArgs: policyFormatArgsSchema,
        layout: policyLayoutSchema,
        host: policyAstRuleSchema,
        calls: policyCallsSchema,
      }),
    })
    .strict()
    .refine(rule => (POLICY_PARSE_GRAMMARS as readonly string[]).includes(rule.grammar), {
      message: `grammar must be one of ${POLICY_PARSE_GRAMMARS.join(", ")}.`,
      path: ["grammar"],
    })
    .refine(rule => AST_RULE_KEYS.some(key => rule[key] !== undefined), "parse must hold a rule over the parsed tree.") as unknown as z.ZodType<PolicyDslParse>,
);
export const policyAstCountSchema: z.ZodType<PolicyDslAstCount> = z.lazy(() =>
  z
    .object({
      inside: policyAstRelationSchema.optional(),
      has: policyAstRelationSchema.optional(),
      precedes: policyAstRelationSchema.optional(),
      follows: policyAstRelationSchema.optional(),
      min: z.number().int().nonnegative().optional(),
      max: z.number().int().nonnegative().optional(),
    })
    .strict()
    .refine(count => AST_RELATION_KEYS.filter(key => count[key] !== undefined).length === 1, "A count must contain exactly one relation: inside, has, precedes, or follows.")
    .refine(count => (count.min ?? 0) >= 1 || count.max !== undefined, "A count must set min to at least 1, or set max.")
    .refine(count => count.max === undefined || (count.min ?? 0) <= count.max, "A count's min must not exceed its max.") as unknown as z.ZodType<PolicyDslAstCount>,
);

const policyValuePair = z.tuple([policyValueSchema, policyValueSchema]);
export const policyPredicateSchema: z.ZodType<PolicyDslPredicate> = z.lazy(() =>
  z.union([
    z.object({ all: z.array(policyPredicateSchema).min(1) }).strict(),
    z.object({ any: z.array(policyPredicateSchema).min(1) }).strict(),
    z.object({ not: policyPredicateSchema }).strict(),
    z.object({ eq: policyValuePair }).strict(),
    z.object({ ne: policyValuePair }).strict(),
  ]),
);

export type PolicyQuery = PolicyDslQuery;


/** A utility: a rule, or a rule with parameters, `{ params, rule }`, which a call runs in a frame of its own. */
export const policyUtilitySchema: z.ZodType<PolicyDslUtility> = z.lazy(() =>
  z.union([
    z
      .object({
        params: z
          .array(metavariableName)
          .min(1)
          .refine(params => new Set(params).size === params.length, "A parameter is listed twice."),
        rule: policyAstRuleSchema,
      })
      .strict(),
    policyAstRuleSchema,
  ]) as unknown as z.ZodType<PolicyDslUtility>,
);

/** A utility's rule and parameters (none for a plain rule). */
function utilityParts(util: PolicyDslUtility): { rule: PolicyDslAstRule; params: readonly string[]; path: string[] } {
  return "params" in util && Array.isArray(util.params) && "rule" in util
    ? { rule: util.rule as PolicyDslAstRule, params: util.params, path: ["rule"] }
    : { rule: util as PolicyDslAstRule, params: [], path: [] };
}

/** Whether a rule holds a `calls` term anywhere in it. */
function containsCalls(value: unknown): boolean {
  if (Array.isArray(value)) return value.some(containsCalls);
  if (value === null || typeof value !== "object") return false;
  return Object.entries(value).some(([key, item]) => key === "calls" || containsCalls(item));
}

type ParseGrammar = (typeof POLICY_PARSE_GRAMMARS)[number];
type RulePath = (string | number)[];
type RuleIssue = (path: RulePath, message: string) => void;
/** The query's language options, or `"shared"` for a define's utility (it compiles with each caller's). */
type AstLanguageOptions = PolicyDslLanguageOptions | undefined | "shared";

/** A rule's language checks, as the kernel reads them. */
type AstRuleVisit = (rule: PolicyDslAstRule, path: RulePath, parsed?: ParseGrammar, fields?: boolean) => void;

/** The predicates a language implements only with a capability. */
const RULE_CAPABILITIES: readonly [keyof PolicyDslAstRule, PolicySemanticCapability][] = [
  ["resolves", "scope"],
  ["binds", "scope"],
  ["value", "value"],
  ["compare", "value"],
  ["arguments", "arguments"],
  ["calls", "calls"],
  ["formatArgs", "formatArgs"],
];

/** A family as the kernel's refusals name it. */
const FAMILY_NAMES: Record<PolicyLanguageFamily, string> = { python: "Python", ecma: "TypeScript", php: "php", blade: "blade" };

/** The `resolves` keys php and blade answer: names through `import`, `imported` and `prefix`, variables through `global`, `unresolved` and `binding` (`write` is refused apart). */
const PHP_RESOLVES: Record<string, true> = { import: true, imported: true, prefix: true, global: true, unresolved: true, binding: true, write: true };

/** The `resolves` keys php has no counterpart for, and why (the kernel's `PHP_REFUSED_RESOLVES`). */
const PHP_REFUSED_RESOLVES: Record<string, string> = {
  this: "this asks whether JavaScript's this is the global object; PHP's $this never is: outside an object context it is undefined, which unresolved: true reports",
  sloppyFunctions: "sloppy-mode functions are JavaScript's; PHP has no strict and sloppy modes",
  directives: "/* global */ comments are ESLint's; PHP's superglobals are fixed, and a function declares a global with a global statement, which binding reads",
  declared: "declared globals come from ESLint's and Ruff's configuration; PHP's superglobals are always declared, and no languageOptions configure others",
  scriptDeclarations: "scripts are ESLint's; a PHP file has no script mode, and a superglobal is never a file's declaration",
  relative: String.raw`a use statement names what it imports fully qualified (use namespace\A; does not parse); import resolves a namespace\A reference`,
  seenBefore: "Ruff's seen_module asks whether a module was imported before; a PHP use only aliases a name for the rest of its namespace and loads nothing",
  beforeLocal: "a PHP function sees no variable of an enclosing scope, so a read before the function binds the name is undefined, which unresolved: true reports",
};

/** PHP's superglobals, by the names `resolves.global` lists them (the kernel's `PHP_SUPERGLOBALS`). */
export const POLICY_PHP_SUPERGLOBALS = ["GLOBALS", "_SERVER", "_GET", "_POST", "_FILES", "_COOKIE", "_SESSION", "_REQUEST", "_ENV"] as const;

/** The binding filters php and blade refuse, and what each reads. */
const PHP_REFUSED_FILTERS: readonly (readonly [keyof PolicyDslBindingFilter, string])[] = [
  ["exported", "Python's re-exports and TypeScript's exports; a PHP file exports no variables (an including file shares its global scope, which no declaration exports)"],
  ["builtin", "ESLint's configured globals; PHP's superglobals are no file's bindings (an assignment to $_GET writes the superglobal)"],
  ["directives", "ESLint's /* exported */ comments; PHP exports no variables"],
];

/** A key a rule of some language families may not use (or, without languageOptions, cannot read), where it sits, and why. */
interface LanguageRestriction {
  /** The families whose rules the restriction applies to; undefined for every family. */
  family: PolicyLanguageFamily | readonly PolicyLanguageFamily[] | undefined;
  path: readonly string[];
  applies: (rule: PolicyDslAstRule, languageOptions: AstLanguageOptions) => boolean;
  /** Why, as the kernel words it for the rule's family. */
  message: string | ((rule: PolicyDslAstRule, family: PolicyLanguageFamily) => string);
}

/** The families whose variables and calls are PHP's. */
const PHP_READERS = [PHP_FAMILY, BLADE_FAMILY] as const;

/** Binding and import keys, checked before a rule's occurrences. */
const BINDING_RESTRICTIONS: readonly LanguageRestriction[] = [
  { family: PYTHON_FAMILY, path: ["resolves", "directives"], applies: rule => rule.resolves?.directives !== undefined, message: "/* global */ comments are ESLint's; Python has none." },
  { family: ECMA_FAMILY, path: ["resolves", "import"], applies: rule => rule.resolves?.import !== undefined, message: "Import resolution is Python-only for now; TypeScript rules can use global." },
  { family: ECMA_FAMILY, path: ["resolves", "imported"], applies: rule => rule.resolves?.imported !== undefined, message: "Import resolution is Python-only for now; TypeScript rules can use global." },
  { family: ECMA_FAMILY, path: ["resolves", "binding", "latest"], applies: rule => rule.resolves?.binding?.latest !== undefined, message: "latest reads Python's binding order and declarations; TypeScript has none." },
  { family: ECMA_FAMILY, path: ["resolves", "binding", "outer"], applies: rule => rule.resolves?.binding?.outer !== undefined, message: "outer reads Python's binding order and declarations; TypeScript has none." },
  { family: ECMA_FAMILY, path: ["binds", "latest"], applies: rule => rule.binds?.latest !== undefined, message: "latest reads Python's binding order and declarations; TypeScript has none." },
  { family: ECMA_FAMILY, path: ["binds", "outer"], applies: rule => rule.binds?.outer !== undefined, message: "outer reads Python's binding order and declarations; TypeScript has none." },
  { family: ECMA_FAMILY, path: ["binds", "redefines", "sameImport"], applies: rule => rule.binds?.redefines?.sameImport !== undefined, message: "Qualified import names are Python's; TypeScript has none." },
  { family: PYTHON_FAMILY, path: ["binds", "redefines", "every"], applies: rule => rule.binds?.redefines?.every !== undefined, message: "Python's redefines reads the one binding shadowed; every is TypeScript's." },
  {
    family: PHP_READERS,
    path: ["binds", "redefines", "sameImport"],
    applies: rule => rule.binds?.redefines?.sameImport !== undefined,
    message: (_rule, family) => `Qualified import names are Python's; a ${family === PHP_FAMILY ? "PHP" : FAMILY_NAMES[family]} variable is never bound by an import.`,
  },
];

/** Resolution, argument and compare keys, checked after a rule's binding filters. */
const RESOLUTION_RESTRICTIONS: readonly LanguageRestriction[] = [
  { family: ECMA_FAMILY, path: ["resolves", "beforeLocal"], applies: rule => rule.resolves?.beforeLocal !== undefined, message: "beforeLocal reads Python's binding order; TypeScript has none." },
  {
    family: ECMA_FAMILY,
    path: ["resolves", "unresolved"],
    applies: rule => rule.resolves?.unresolved === "star",
    message: 'unresolved: "star" asks whether a Python star import binds the name; TypeScript has none.',
  },
  { family: PYTHON_FAMILY, path: ["resolves", "this"], applies: rule => rule.resolves?.this !== undefined, message: "this is TypeScript's; Python has none." },
  { family: PYTHON_FAMILY, path: ["resolves", "scriptDeclarations"], applies: rule => rule.resolves?.scriptDeclarations !== undefined, message: "Scripts are ESLint's; Python has none." },
  {
    family: ECMA_FAMILY,
    path: ["resolves", "scriptDeclarations"],
    applies: (rule, languageOptions) => rule.resolves?.scriptDeclarations !== undefined && languageOptions === undefined,
    message: "Whether a file is a script comes from languageOptions.sourceType; declare languageOptions.",
  },
  {
    family: undefined,
    path: ["resolves", "declared"],
    applies: (rule, languageOptions) => rule.resolves?.declared !== undefined && languageOptions === undefined,
    message: "The declared globals come from languageOptions; declare languageOptions.",
  },
  {
    family: ECMA_FAMILY,
    path: ["resolves", "this"],
    applies: (rule, languageOptions) => rule.resolves?.this !== undefined && languageOptions === undefined,
    message: "The global object depends on languageOptions.sourceType; declare languageOptions.",
  },
  { family: ECMA_FAMILY, path: ["resolves", "binding", "stopAtGlobal"], applies: rule => rule.resolves?.binding?.stopAtGlobal !== undefined, message: "global statements are Python's; TypeScript has none." },
  {
    family: PHP_READERS,
    path: ["resolves", "binding", "stopAtGlobal"],
    applies: rule => rule.resolves?.binding?.stopAtGlobal === false,
    message: (_rule, family) =>
      `a ${FAMILY_NAMES[family]} read after global $x reads that statement's binding; which top-level binding the global variable then holds depends on when the function runs, so ${FAMILY_NAMES[family]} has no lookup through it.`,
  },
  {
    family: PYTHON_FAMILY,
    path: ["resolves", "binding", "destructuredValue"],
    applies: rule => rule.resolves?.binding?.destructuredValue !== undefined,
    message: "destructured values are TypeScript's; a Python unpacked name keeps its element.",
  },
  {
    family: ECMA_FAMILY,
    path: ["arguments", "signature"],
    applies: rule => rule.arguments?.signature.some(entry => entry === "/" || entry === "*") === true,
    message: "/ and * mark Python's positional-only and keyword-only slots; JavaScript passes arguments by position only.",
  },
  {
    family: PHP_READERS,
    path: ["arguments", "signature"],
    applies: rule => rule.arguments?.signature.some(entry => entry === "/" || entry === "*") === true,
    message: "/ and * mark Python's positional-only and keyword-only slots; PHP has no positional-only or keyword-only parameters.",
  },
  {
    family: ECMA_FAMILY,
    path: ["arguments", "mode"],
    applies: rule => rule.arguments?.mode === "syntactic",
    message: "syntactic finds arguments by position and keyword as Ruff's find_argument does; TypeScript has no keywords.",
  },
  { family: PYTHON_FAMILY, path: ["arguments", "mode"], applies: rule => rule.arguments?.mode === "estree", message: "estree reads ESLint's node.arguments; use language or syntactic for Python." },
  {
    family: PHP_READERS,
    path: ["arguments", "mode"],
    applies: rule => rule.arguments?.mode === "estree",
    message: (_rule, family) => `estree reads ESLint's node.arguments of ESTree, JavaScript's syntax tree; ${FAMILY_NAMES[family]} has none (use language or syntactic).`,
  },
  {
    family: PYTHON_FAMILY,
    path: ["compare", "text"],
    applies: rule => rule.compare?.text === "key" || rule.compare?.text === "member",
    message: rule => `${rule.compare?.text} reads JavaScript and PHP property names; Python has none.`,
  },
  {
    family: PYTHON_FAMILY,
    path: ["compare", "text"],
    applies: rule => rule.compare?.text === "reference" || rule.compare?.text === "reference-no-static-keys",
    message: rule => `${rule.compare?.text} reads JavaScript and PHP references (ESLint's isSameReference); use structure for Python.`,
  },
  {
    family: PYTHON_FAMILY,
    path: ["compare", "text"],
    applies: rule => rule.compare?.text === "name",
    message: "name reads the names JavaScript and PHP identifiers and strings write; use unquoted for Python.",
  },
  {
    family: ECMA_FAMILY,
    path: ["compare", "text"],
    applies: rule => rule.compare?.text !== undefined && POLICY_STRUCTURE_TEXTS.includes(rule.compare.text),
    message: "The structural modes compare Python and PHP syntax; use tokens for TypeScript.",
  },
  {
    family: PHP_FAMILY,
    path: ["compare", "text"],
    applies: rule => rule.compare?.text === "structure-parts",
    message: "structure-parts compares Python's implicitly concatenated string parts; PHP has no implicit string concatenation.",
  },
  {
    family: BLADE_FAMILY,
    path: ["compare", "text"],
    applies: rule => rule.compare?.text === "structure-parts",
    message: "structure-parts compares Python's implicitly concatenated string parts; blade embeds PHP, which has no implicit string concatenation.",
  },
];

/** Each family's restrictions, in table order: a rule's visit runs on every node of every stored policy it reads, so it filters by family once here rather than per node. */
type FamilyRestrictions = Readonly<Record<PolicyLanguageFamily, readonly LanguageRestriction[]>>;

function restrictionsByFamily(restrictions: readonly LanguageRestriction[]): FamilyRestrictions {
  const of = (family: PolicyLanguageFamily): readonly LanguageRestriction[] =>
    restrictions.filter(restriction => restriction.family === undefined || [restriction.family].flat().includes(family));
  return { [PYTHON_FAMILY]: of(PYTHON_FAMILY), [ECMA_FAMILY]: of(ECMA_FAMILY), [PHP_FAMILY]: of(PHP_FAMILY), [BLADE_FAMILY]: of(BLADE_FAMILY) };
}

const BINDING_RESTRICTIONS_BY_FAMILY = restrictionsByFamily(BINDING_RESTRICTIONS);
const RESOLUTION_RESTRICTIONS_BY_FAMILY = restrictionsByFamily(RESOLUTION_RESTRICTIONS);

function checkLanguageRestrictions(
  restrictions: FamilyRestrictions,
  rule: PolicyDslAstRule,
  path: RulePath,
  family: PolicyLanguageFamily,
  languageOptions: AstLanguageOptions,
  issue: RuleIssue,
): void {
  for (const restriction of restrictions[family]) {
    if (!restriction.applies(rule, languageOptions)) continue;
    issue([...path, ...restriction.path], typeof restriction.message === "string" ? restriction.message : restriction.message(rule, family));
  }
}

/** `host` reads the literal a parse holds: only inside parse, not on formatArgs' fields, and not over an annotation. */
function checkHost(rule: PolicyDslAstRule, path: RulePath, parsed: ParseGrammar | undefined, fields: boolean, issue: RuleIssue, visit: AstRuleVisit): void {
  if (rule.host === undefined) return;
  if (parsed === undefined) issue([...path, "host"], "host tests the literal a parse holds; use it inside parse.");
  else if (!fields && parsed !== "annotation") issue([...path, "host"], "host tests the literal a parse holds; formatArgs' fields have none.");
  else if (parsed !== "annotation") visit(rule.host, [...path, "host"]);
}

/** Keys a rule over a tree of `parsed`'s grammar may not use. */
function checkParsedKeys(rule: PolicyDslAstRule, path: RulePath, parsed: ParseGrammar, issue: RuleIssue): void {
  for (const key of AST_RULE_KEYS) {
    if (rule[key] === undefined || PARSE_RULE_KEYS.includes(key)) continue;
    if (parsed === "annotation" && ANNOTATION_RULE_KEYS.includes(key)) continue;
    if (parsed === "eslintDirective" && key === "resolves") continue;
    issue([...path, key], `${key} is not supported inside parse (${parsed} trees).`);
  }
}

/** An annotation's nodes are Python's, but its names' bindings and literals are the host's. */
function checkAnnotationNode(rule: PolicyDslAstRule, path: RulePath, issue: RuleIssue): void {
  if (rule.host !== undefined) issue([...path, "host"], "host is not supported inside parse (annotation trees).");
  if (rule.resolves?.binding !== undefined) issue([...path, "resolves", "binding"], "A parsed annotation's names are read, but their bindings' nodes are the host's.");
  if (rule.parse !== undefined && rule.parse.grammar !== "annotation") issue([...path, "parse", "grammar"], "A string inside an annotation parses as an annotation.");
}

/** What a node of a parsed tree has: its grammar's kinds, no binding, and source-level compare texts. */
function checkParsedNode(rule: PolicyDslAstRule, path: RulePath, parsed: ParseGrammar, issue: RuleIssue): void {
  const kinds: readonly string[] = (POLICY_PARSE_KINDS as Record<string, readonly string[]>)[parsed] ?? [];
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

/** The family whose literals each grammar parses, and what they are (the kernel's `Grammar::host`); a Blade template's literals are those of the PHP it embeds. */
const GRAMMAR_HOSTS: Record<ParseGrammar, PolicyLanguageFamily> = {
  percentFormat: PYTHON_FAMILY,
  braceFormat: PYTHON_FAMILY,
  annotation: PYTHON_FAMILY,
  eslintDirective: ECMA_FAMILY,
  printfFormat: PHP_FAMILY,
  scanfFormat: PHP_FAMILY,
  phpDoc: PHP_FAMILY,
};
const HOST_NAMES: Record<ParseGrammar, string> = {
  percentFormat: "Python literals",
  braceFormat: "Python literals",
  annotation: "Python literals",
  eslintDirective: "TypeScript comments",
  printfFormat: "PHP strings",
  scanfFormat: "PHP strings",
  phpDoc: "PHP doc comments",
};

/** Why a family's literals have no tree of another family's grammar: its counterpart, or what it lacks. */
const UNHOSTED_REASONS: Partial<Record<PolicyLanguageFamily, Partial<Record<ParseGrammar, string>>>> = {
  [PHP_FAMILY]: {
    percentFormat: "; PHP's printf formats are printfFormat",
    braceFormat: "; PHP has no str.format templates (its printf formats are printfFormat)",
    annotation: "; PHP holds no type in a string: its declared types are syntax, and its PHPDoc types are phpDoc",
  },
  [BLADE_FAMILY]: {
    percentFormat: "; blade embeds PHP, whose printf formats are printfFormat",
    braceFormat: "; blade embeds PHP, which has no str.format templates (its printf formats are printfFormat)",
    annotation: "; blade embeds PHP, which holds no type in a string: its declared types are syntax, and its PHPDoc types are phpDoc",
  },
  [PYTHON_FAMILY]: {
    printfFormat: "; Python's printf formats are percentFormat",
    scanfFormat: "; Python has no scanf formats",
  },
};

/** `parse`: its grammar reads the query's language family's literals, and its rule is a rule over that grammar's tree. */
function visitParse(rule: PolicyDslAstRule, path: RulePath, family: PolicyLanguageFamily, issue: RuleIssue, visit: AstRuleVisit): void {
  if (rule.parse === undefined) return;
  const grammar = rule.parse.grammar as ParseGrammar;
  const host = GRAMMAR_HOSTS[grammar];
  if (POLICY_PARSE_GRAMMARS.includes(grammar) && host !== family && !(host === PHP_FAMILY && readsPhp(family))) {
    issue([...path, "parse", "grammar"], `${grammar} parses ${HOST_NAMES[grammar]}${UNHOSTED_REASONS[family]?.[grammar] ?? ""}.`);
  }
  if (POLICY_PARSE_GRAMMARS.includes(grammar)) visit(rule.parse, [...path, "parse"], grammar);
}

const STYLE_GRAMMARS: Record<PolicyDslFormatArgs["style"], ParseGrammar> = { brace: "braceFormat", percent: "percentFormat", printf: "printfFormat", scanf: "scanfFormat" };

/** Why a family does not read a style's format calls: the style is another family's (ecma has no formatArgs). */
function foreignStyle(family: PolicyLanguageFamily, style: PolicyDslFormatArgs["style"]): string | undefined {
  if (family === PYTHON_FAMILY && style === "printf") return "printf reads PHP's printf-family calls; Python's printf formats are style percent.";
  if (family === PYTHON_FAMILY && style === "scanf") return "scanf reads PHP's sscanf and fscanf calls; Python has no scanf formats.";
  if (family === PHP_FAMILY && style === "percent") return "percent reads Python's % formats; PHP's printf-family calls are style printf.";
  if (family === PHP_FAMILY && style === "brace") return "brace reads Python's str.format calls; PHP has no such templates (its printf-family calls are style printf).";
  if (family === BLADE_FAMILY && style === "percent") return "percent reads Python's % formats; blade embeds PHP, whose printf-family calls are style printf.";
  if (family === BLADE_FAMILY && style === "brace") return "brace reads Python's str.format calls; blade embeds PHP, which has no such templates (its printf-family calls are style printf).";
  return undefined;
}

/** `formatArgs`: a style of the query's family; its unused arguments are the host's nodes, its missing fields nodes of the format's tree. */
function visitFormatArgs(rule: PolicyDslAstRule, path: RulePath, family: PolicyLanguageFamily, parsed: ParseGrammar | undefined, issue: RuleIssue, visit: AstRuleVisit): void {
  if (rule.formatArgs === undefined) return;
  const foreign = foreignStyle(family, rule.formatArgs.style);
  if (foreign !== undefined) issue([...path, "formatArgs", "style"], foreign);
  if (rule.formatArgs.unused !== undefined) visit(rule.formatArgs.unused, [...path, "formatArgs", "unused"], parsed);
  const grammar = STYLE_GRAMMARS[rule.formatArgs.style];
  if (rule.formatArgs.missing !== undefined) visit(rule.formatArgs.missing, [...path, "formatArgs", "missing"], grammar, false);
}

function visitCalls(rule: PolicyDslAstRule, path: RulePath, parsed: ParseGrammar | undefined, issue: RuleIssue, visit: AstRuleVisit): void {
  if (rule.calls === undefined) return;
  const { through: _through, chain: _chain, ...inner } = rule.calls;
  if (containsCalls(inner)) issue([...path, "calls"], "A calls rule cannot contain calls.");
  visit(inner, [...path, "calls"], parsed);
}

/** A binding's `occurrence` is TypeScript's; its rule is a rule over the occurrence. */
function visitOccurrences(rule: PolicyDslAstRule, path: RulePath, family: PolicyLanguageFamily, issue: RuleIssue, visit: AstRuleVisit): void {
  for (const [owner, ownerPath] of [
    [rule.binds, [...path, "binds"]],
    [rule.resolves?.binding, [...path, "resolves", "binding"]],
  ] as const) {
    if (owner?.occurrence === undefined) continue;
    if (family === PYTHON_FAMILY) issue([...ownerPath, "occurrence"], `A variable's occurrences are TypeScript's; ${FAMILY_NAMES[family]} has none.`);
    visit(owner.occurrence, [...ownerPath, "occurrence"]);
  }
}

/** Each of a filter's `form` or `scope` names is one of `known`. */
function checkFilterNames(names: string | readonly string[] | undefined, at: RulePath, known: readonly string[], what: string, issue: RuleIssue): void {
  [names ?? []].flat().forEach((name, index) => {
    if (!known.includes(name)) issue(Array.isArray(names) ? [...at, index] : at, `${name} is not a ${what}; expected one of ${known.join(", ")}.`);
  });
}

/** Each family's scope kinds, as its binding filters name them. */
const SCOPE_KINDS: Record<PolicyLanguageFamily, readonly string[]> = {
  [PYTHON_FAMILY]: POLICY_PYTHON_SCOPE_KINDS,
  [ECMA_FAMILY]: POLICY_TYPESCRIPT_SCOPE_KINDS,
  [PHP_FAMILY]: POLICY_PHP_SCOPE_KINDS,
  [BLADE_FAMILY]: POLICY_PHP_SCOPE_KINDS,
};

/** The filter keys a family refuses, each with why: directives and configured globals are ESLint's; php and blade read neither Ruff's binding order nor exports. */
function refusedFilterKeys(family: PolicyLanguageFamily): readonly (readonly [keyof PolicyDslBindingFilter, string])[] {
  if (family === PYTHON_FAMILY) {
    return [
      ["directives", "/* exported */ comments are ESLint's; Python has none."],
      ["builtin", "Configured globals are ESLint's; Python has none."],
    ];
  }
  if (!readsPhp(family)) return [];
  return PHP_REFUSED_FILTERS.map(([key, what]) => [key, `${key} reads ${what}; ${FAMILY_NAMES[family]} has none.`] as const);
}

/** A binding filter's forms and scope kinds are its language's, and it holds no key its family refuses. */
function checkBindingFilters(rule: PolicyDslAstRule, path: RulePath, family: PolicyLanguageFamily, language: PolicyAstLanguage, issue: RuleIssue): void {
  const filters: [PolicyDslBindingFilter | undefined, RulePath][] = [
    [rule.binds, [...path, "binds"]],
    [rule.binds?.redefines, [...path, "binds", "redefines"]],
    [rule.resolves?.binding, [...path, "resolves", "binding"]],
  ];
  const forms: readonly string[] = readsPhp(family) ? POLICY_PHP_BINDING_FORMS : POLICY_SHARED_BINDING_FORMS;
  const refused = refusedFilterKeys(family);
  for (const [filter, filterPath] of filters) {
    if (filter === undefined) continue;
    checkFilterNames(filter.form, [...filterPath, "form"], forms, `${language} binding form`, issue);
    checkFilterNames(filter.scope, [...filterPath, "scope"], SCOPE_KINDS[family], `${language} scope kind`, issue);
    for (const [key, message] of refused) {
      if (filter[key] !== undefined) issue([...filterPath, key], message);
    }
  }
}

/** php's and blade's resolves: names through import and prefix, variables through superglobals, unresolved and binding; one resolves asks names or variables. */
function checkPhpResolves(rule: PolicyDslAstRule, path: RulePath, family: PolicyLanguageFamily, issue: RuleIssue): void {
  const resolves = rule.resolves;
  if (!readsPhp(family) || resolves === undefined) return;
  const name = FAMILY_NAMES[family];
  const at = [...path, "resolves"];
  for (const key of Object.keys(resolves)) {
    const why = PHP_REFUSED_RESOLVES[key];
    if (why !== undefined) issue([...at, key], `${why}; ${name} has none.`);
    else if (!Object.hasOwn(PHP_RESOLVES, key)) issue([...at, key], `resolves.${key} is not available for ${name} policies.`);
  }
  if ((resolves.import !== undefined || resolves.imported !== undefined) && (resolves.global !== undefined || resolves.unresolved !== undefined)) {
    issue(at, "import and imported resolve class, function and constant names; global and unresolved resolve variables; one resolves asks one of them.");
  }
  if (resolves.unresolved !== undefined && resolves.unresolved !== true) issue([...at, "unresolved"], `${name} policies ask unresolved: true.`);
  const superglobals: readonly string[] = POLICY_PHP_SUPERGLOBALS;
  [resolves.global === true ? [] : (resolves.global ?? [])].flat().forEach((global, index) => {
    if (!superglobals.includes(global)) issue([...at, "global", index], `${global} is no PHP superglobal; expected one of ${superglobals.join(", ")}.`);
  });
}

/** The import-name shape a family refuses, as the kernel's `compile_resolves` does: Python imports are dotted, PHP names join labels with `\`. */
const dottedPhpName = (name: string): string | undefined => (name.includes(".") ? String.raw`${name} is a dotted name; PHP names join labels with \.` : undefined);
const FOREIGN_IMPORT_NAMES: Partial<Record<PolicyLanguageFamily, (name: string) => string | undefined>> = {
  [PYTHON_FAMILY]: name => (name.includes("\\") ? `${name} is a PHP name; Python imports are dotted names.` : undefined),
  [PHP_FAMILY]: dottedPhpName,
  [BLADE_FAMILY]: dottedPhpName,
};

/** Each import and imported name is in its family's shape; the name schema admits both. */
function checkImportNames(rule: PolicyDslAstRule, path: RulePath, family: PolicyLanguageFamily, issue: RuleIssue): void {
  const foreign = FOREIGN_IMPORT_NAMES[family];
  const resolves = rule.resolves;
  if (foreign === undefined || resolves === undefined) return;
  for (const key of ["import", "imported"] as const) {
    const listed = resolves[key];
    const names: readonly string[] = listed === true || listed === undefined ? [] : listed;
    names.forEach((name, index) => {
      const why = foreign(name);
      if (why !== undefined) issue([...path, "resolves", key, index], why);
    });
  }
}

/** Why a family has no `resolves.write`: where ESLint reports a variable is TypeScript's. */
const WRITE_REFUSALS: Partial<Record<PolicyLanguageFamily, string>> = {
  [PYTHON_FAMILY]: "Where ESLint reports a variable is TypeScript's; Python has none.",
  [PHP_FAMILY]:
    "ESLint reports an unused variable once, at its last write; php's usage oracle, Psalm, reports each unused assignment, which binds: { used: false } matches.",
  [BLADE_FAMILY]:
    "ESLint reports an unused variable once, at its last write; blade reads its embedded PHP, whose usage oracle, Psalm, reports each unused assignment, which binds: { used: false } matches.",
};

/** Each family's value type names. */
const VALUE_TYPES: Record<PolicyLanguageFamily, readonly string[]> = {
  [PYTHON_FAMILY]: POLICY_PYTHON_VALUE_TYPES,
  [ECMA_FAMILY]: POLICY_TYPESCRIPT_VALUE_TYPES,
  [PHP_FAMILY]: POLICY_PHP_VALUE_TYPES,
  [BLADE_FAMILY]: POLICY_PHP_VALUE_TYPES,
};

function checkValueTypes(rule: PolicyDslAstRule, path: RulePath, family: PolicyLanguageFamily, language: PolicyAstLanguage, issue: RuleIssue): void {
  const types = rule.value?.type === undefined ? [] : [rule.value.type].flat();
  const known = VALUE_TYPES[family];
  types.forEach((name, index) => {
    if (!known.includes(name)) issue([...path, "value", "type", index], `${name} is not a ${language} type name; expected one of ${known.join(", ")}.`);
  });
}

/** A rule's or count's relations: `field` only in has/inside, outside count and parse. */
function visitRelations(
  owner: PolicyDslAstRule | PolicyDslAstCount,
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
function visitBindingRules(rule: PolicyDslAstRule, path: RulePath, visit: AstRuleVisit): void {
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
function visitSubRules(rule: PolicyDslAstRule, path: RulePath, parsed: ParseGrammar | undefined, fields: boolean, issue: RuleIssue, visit: AstRuleVisit): void {
  for (const key of ["all", "any"] as const) rule[key]?.forEach((item, index) => visit(item, [...path, key, index], parsed, fields));
  if (typeof rule.nthChild === "object" && rule.nthChild.ofRule !== undefined) visit(rule.nthChild.ofRule, [...path, "nthChild", "ofRule"], parsed, fields);
  if (rule.not !== undefined) visit(rule.not, [...path, "not"], parsed, fields);
  visitRelations(rule, path, false, parsed, fields, issue, visit);
  if (rule.count !== undefined) visitRelations(rule.count, [...path, "count"], true, parsed, fields, issue, visit);
  visitBindingRules(rule, path, visit);
}

/**
 * The checks of a rule and its sub-rules that depend on the language: keys and values one language
 * lacks, and kinds a parsed tree's grammar lacks. `languageOptions` is the query's, or `"shared"`
 * for a define's utility, which compiles with each caller's options (so none is required of it).
 * Issues are reported in the order the kernel reads the rule.
 */
function astRuleVisitor(context: z.RefinementCtx, language: PolicyAstLanguage, languageOptions: AstLanguageOptions): AstRuleVisit {
  const family = policyLanguageFamily(language);
  const issue: RuleIssue = (path, message) => context.addIssue({ code: "custom", path, message });
  const visit: AstRuleVisit = (rule, path, parsed, fields = true) => {
    for (const [predicate, capability] of RULE_CAPABILITIES) {
      if (rule[predicate] === undefined || policyLanguageImplements(language, capability)) continue;
      const at = [...path, predicate];
      issue(at, `${at.join(".")}: \`${predicate}\` is not available for ${language} policies`);
    }
    if (parsed === undefined && rule.capture !== undefined) issue([...path, "capture"], "capture binds a node of a parsed tree; use it inside parse.");
    checkHost(rule, path, parsed, fields, issue, visit);
    if (parsed !== undefined) {
      checkParsedKeys(rule, path, parsed, issue);
      checkParsedNode(rule, path, parsed, issue);
    }
    visitParse(rule, path, family, issue, visit);
    visitFormatArgs(rule, path, family, parsed, issue, visit);
    visitCalls(rule, path, parsed, issue, visit);
    checkLanguageRestrictions(BINDING_RESTRICTIONS_BY_FAMILY, rule, path, family, languageOptions, issue);
    visitOccurrences(rule, path, family, issue, visit);
    const writeRefusal = WRITE_REFUSALS[family];
    if (writeRefusal !== undefined && rule.resolves?.write !== undefined) issue([...path, "resolves", "write"], writeRefusal);
    checkBindingFilters(rule, path, family, language, issue);
    checkPhpResolves(rule, path, family, issue);
    checkImportNames(rule, path, family, issue);
    checkLanguageRestrictions(RESOLUTION_RESTRICTIONS_BY_FAMILY, rule, path, family, languageOptions, issue);
    checkValueTypes(rule, path, family, language, issue);
    visitSubRules(rule, path, parsed, fields, issue, visit);
  };
  return visit;
}

/** Transforms that read a node but name a transform variable, and transforms whose sources cycle. */
function checkTransforms(transform: Record<string, PolicyDslTransformation>, context: z.RefinementCtx): void {
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
function checkTransformValue(name: string, transform: Record<string, PolicyDslTransformation>, context: z.RefinementCtx): void {
  const body = transformBody(transform[name]!);
  if (body.value !== true) return;
  if (body.source.startsWith("$$$")) {
    context.addIssue({ code: "custom", path: ["transform", name], message: "A value is read from a single capture ($NAME)." });
  } else if (Object.hasOwn(transform, transformSourceName(transform[name]!))) {
    context.addIssue({ code: "custom", path: ["transform", name], message: "A transform variable is text; read the value of a capture." });
  }
}

/** Language options the query's language family does not read. */
function checkLanguageOptions(language: PolicyAstLanguage, languageOptions: PolicyDslLanguageOptions | undefined, context: z.RefinementCtx): void {
  const family = policyLanguageFamily(language);
  if (readsPhp(family) && languageOptions !== undefined) {
    context.addIssue({
      code: "custom",
      path: ["languageOptions"],
      message: `languageOptions: \`languageOptions\` is not available for ${FAMILY_NAMES[family]} policies`,
    });
    return;
  }
  const foreign = family === PYTHON_FAMILY ? TYPESCRIPT_ONLY_OPTIONS : PYTHON_ONLY_OPTIONS;
  for (const key of foreign) {
    if (languageOptions?.[key] !== undefined) {
      context.addIssue({ code: "custom", path: ["languageOptions", key], message: `${key} is not a ${language} language option.` });
    }
  }
}

/** `when` gates on another language family's versions. */
function checkWhenGates(language: PolicyAstLanguage, when: PolicyDslAstWhen | undefined, context: z.RefinementCtx): void {
  const foreignGates = policyLanguageFamily(language) === PYTHON_FAMILY ? (["ecmaVersion", "sourceType", "ecmaFeatures"] as const) : (["pythonVersion"] as const);
  for (const gate of foreignGates) {
    if (when?.[gate] !== undefined) {
      context.addIssue({ code: "custom", path: ["when", gate], message: `${gate} is not a ${language} gate.` });
    }
  }
}

/** File conditions the query's language family has no notion of. */
function checkFileCondition(language: PolicyAstLanguage, file: PolicyDslFileCondition | undefined, context: z.RefinementCtx): void {
  const family = policyLanguageFamily(language);
  for (const key of PYTHON_FILE_KEYS) {
    if (family !== PYTHON_FAMILY && file?.[key] !== undefined) {
      context.addIssue({ code: "custom", path: ["file", key], message: `Module paths are Python's; ${language} has none.` });
    }
  }
  if (family === PYTHON_FAMILY && file?.declaration !== undefined) {
    context.addIssue({ code: "custom", path: ["file", "declaration"], message: "Declaration files are TypeScript's; python has none." });
  }
}

export const policyQuerySchema: z.ZodType<PolicyDslQuery> = z.lazy(() =>
  z
    .object({
      regex: z
        .object({
          source: z.enum(POLICY_REGEX_SOURCES),
          pattern: nonEmptyString,
        })
        .strict()
        .optional(),
      ast: z
        .object({
          language: z.enum(POLICY_AST_LANGUAGES),
          rule: policyAstRuleSchema,
          utils: z.record(utilityId, policyUtilitySchema).optional(),
          constraints: z.record(nonEmptyString, policyAstRuleSchema).optional(),
          languageOptions: policyLanguageOptionsSchema.optional(),
          when: policyAstWhenSchema.optional(),
          file: policyFileConditionSchema.optional(),
          transform: z.record(metavariableName, policyTransformationSchema).optional(),
        })
        .strict()
        .superRefine((ast, context) => {
          // The kernel is the authority; these are the cross-references a schema can see.
          const utils = ast.utils ?? {};
          checkTransforms(ast.transform ?? {}, context);
          checkLanguageOptions(ast.language, ast.languageOptions, context);
          checkFileCondition(ast.language, ast.file, context);
          checkWhenGates(ast.language, ast.when, context);
          const visit = astRuleVisitor(context, ast.language, ast.languageOptions);
          visit(ast.rule, ["rule"]);
          for (const [id, util] of Object.entries(utils)) visit(utilityParts(util).rule, ["utils", id, ...utilityParts(util).path]);
          for (const [name, constraint] of Object.entries(ast.constraints ?? {})) visit(constraint, ["constraints", name]);
        })
        .optional(),
      select_when: z.object({ select: policyQuerySchema, exists: policyQuerySchema }).strict().optional(),
    })
    .strict()
    .refine(
      query => (["regex", "ast", "select_when"] as const).filter(key => query[key] !== undefined).length === 1,
      "A query must contain exactly one source: regex, ast, or select_when.",
    ) as unknown as z.ZodType<PolicyDslQuery>,
);

const PROJECTION_SOURCE = /^(capture\.[A-Z_][A-Z0-9_]*|file\.(path|directory|name|stem|module|package))$/;

export const policyEvidenceQuerySchema = z
  .object({
    evidence: z
      .object({
        as: nonEmptyString,
        paths: z
          .object({ include: z.array(nonEmptyString).optional(), exclude: z.array(nonEmptyString).optional() })
          .strict()
          .optional(),
        query: z.lazy(() => policyQuerySchema).refine(query => "ast" in query, "An evidence query is an ast query."),
        project: z
          .record(nonEmptyString, z.string().regex(PROJECTION_SOURCE, "expected capture.<NAME> or file.path|directory|name|stem|module|package"))
          .refine(project => Object.keys(project).length > 0, "project names at least one field."),
        where: policyPredicateSchema.optional(),
      })
      .strict(),
  })
  .strict() as unknown as z.ZodType<PolicyDslEvidenceQuery>;

const policyRelationSchema = policyEvidenceQuerySchema as z.ZodType<PolicyDslRelation>;

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
 * `$NAME` / `$$$NAME` references in a finding message, as the kernel reads them: `\\$` is a literal
 * dollar, and a name starting with a digit is literal text.
 */
export function policyMessageVariables(message: string): string[] {
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
function emittingQuery(query: PolicyDslQuery): PolicyDslQuery {
  return "select_when" in query && query.select_when !== undefined ? emittingQuery(query.select_when.select) : query;
}

const FIX_NEEDS_SOURCE_TEXT = "Only forbid clauses over an ast or file.content regex query can suggest a fix.";

type FindingIssue = (path: string, message: string) => void;

/** Anchors and fixes the emitting query's matches cannot carry: no query, a fact, or a file.path regex. */
function checkFindingSource(query: PolicyDslQuery | undefined, finding: PolicyDslReportingCheck["violation"], issue: FindingIssue): void {
  if (query === undefined) {
    if (finding.anchor !== "witness.span") issue("anchor", "require_each, budget, and llm violations support only witness.span.");
    if (finding.fix !== undefined) issue("fix", FIX_NEEDS_SOURCE_TEXT);
    return;
  }
  if (finding.fix === undefined) return;
  if ("fact" in query && query.fact !== undefined) issue("fix", FIX_NEEDS_SOURCE_TEXT);
  else if ("regex" in query && query.regex !== undefined && query.regex.source === "file.path") {
    issue("fix", "A file.path regex match is not source text a fix can replace.");
  }
}

/** A non-ast violation's anchor and message may name only the regex's named groups. */
function checkFindingCaptures(query: PolicyDslQuery | undefined, finding: PolicyDslReportingCheck["violation"], issue: FindingIssue): void {
  const regex = query !== undefined && "regex" in query ? query.regex : undefined;
  const groups = regex === undefined ? new Set<string>() : regexGroupNames(regex.pattern);
  const capture = /^capture\.(\w+)$/.exec(finding.anchor)?.[1];
  if (capture !== undefined && !groups.has(capture)) issue("anchor", `capture.${capture} names no capture of the query.`);
  for (const name of policyMessageVariables(finding.message)) {
    if (!groups.has(name)) issue("message", `$${name} names no capture of the query; write \\$${name} for literal text.`);
  }
}

/** Which directive comments a violation's suppress may name: the query's language's, and only for an ast query. */
function checkFindingSuppress(ast: PolicyDslAstQuery["ast"] | undefined, finding: PolicyDslReportingCheck["violation"], issue: FindingIssue): void {
  if (finding.suppress === undefined) return;
  if (ast === undefined) return issue("suppress", "suppress removes an ast query's violations.");
  const suppressions = policyLanguageSuppressions(ast.language);
  const refusals: ReadonlyArray<readonly [PolicySuppression, string]> = [
    ["eslint", "eslint-disable comments are ESLint's"],
    ["noqa", "noqa comments are Ruff's"],
    ["phpstan", "@phpstan-ignore comments are PHPStan's"],
    ["psalm", "@psalm-suppress tags are Psalm's"],
    ["phpcs", "phpcs: annotations are PHP_CodeSniffer's"],
  ];
  const refused = refusals.find(([linter]) => finding.suppress?.[linter] !== undefined && !suppressions.includes(linter));
  if (refused !== undefined) issue("suppress", `${refused[1]}; a ${ast.language} query has none.`);
}

/**
 * The violation checks the kernel makes against the emitting query that need no grammar: fact-based
 * violations carry no source span or captures, and only ast and file.content regex matches are text a
 * fix can replace. Captures of an ast query are left to the kernel.
 */
function checkFindingAgainstQuery(check: PolicyDslReportingCheck, context: z.RefinementCtx): void {
  const { violation: finding } = check;
  const issue: FindingIssue = (path, message) => context.addIssue({ code: "custom", path: ["violation", path], message });
  const query = check.forbid === undefined ? undefined : emittingQuery(check.forbid);
  checkFindingSource(query, finding, issue);
  const ast = query !== undefined && "ast" in query ? query.ast : undefined;
  if (ast === undefined) {
    if (typeof finding.fix === "object" && finding.fix.range !== "witness.span") issue("fix", "A fix range names a capture of an ast query.");
    if (finding.each !== undefined) issue("each", "each enumerates the captures of an ast query.");
  }
  checkFindingSuppress(ast, finding, issue);
  if (ast !== undefined) return;
  checkFindingCaptures(query, finding, issue);
}

export const policyDefineSchema = z
  .object({
    language: z.enum(POLICY_AST_LANGUAGES).optional(),
    utils: z.record(utilityId, policyUtilitySchema).optional(),
    options: z.record(optionName, policyOptionDeclarationSchema).optional(),
    config: z.record(optionName, policyValueSchema).optional(),
  })
  .strict()
  .refine(define => define.utils !== undefined || define.options !== undefined || define.config !== undefined, "define must hold utils, options or config.")
  .refine(define => define.utils === undefined || define.language !== undefined, {
    message: "language is required: utilities compile for one language.",
    path: ["language"],
  })
  .superRefine((define, context) => {
    if (define.language === undefined) return;
    const visit = astRuleVisitor(context, define.language, "shared");
    for (const [id, util] of Object.entries(define.utils ?? {})) visit(utilityParts(util).rule, ["utils", id, ...utilityParts(util).path]);
  }) as unknown as z.ZodType<PolicyDslDefine>;

export const policyCheckSchema = z
  .object({
    id: nonEmptyString,
    violation: policyFindingSchema.optional(),
    forbid: policyQuerySchema.optional(),
    require_each: z.object({ subject: policyRelationSchema, evidence: policyRelationSchema }).strict().optional(),
    budget: z
      .object({
        query: policyRelationSchema,
        group_by: z.array(nonEmptyString).optional(),
        count: z.object({ lte: z.number().int().nonnegative(), distinct: z.array(nonEmptyString).optional() }).strict(),
      })
      .strict()
      .optional(),
    llm: z.unknown().optional(),
    define: policyDefineSchema.optional(),
  })
  .strict()
  .refine(
    check => POLICY_CHECK_FORMS.filter(form => check[form] !== undefined).length === 1,
    `A clause must contain exactly one constraint form: ${POLICY_CHECK_FORMS.join(", ")}.`,
  )
  .superRefine((check, context) => {
    if (check.define !== undefined) {
      if (check.violation !== undefined) {
        context.addIssue({ code: "custom", path: ["violation"], message: "A define clause reports nothing, so it has no violation." });
      }
    } else if (check.violation === undefined) {
      context.addIssue({ code: "custom", path: ["violation"], message: "violation is required." });
    } else {
      checkFindingAgainstQuery(check as unknown as PolicyDslReportingCheck, context);
    }
  }) as unknown as z.ZodType<PolicyDslCheck>;

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
function astQueries(query: PolicyDslQuery, path: (string | number)[]): [PolicyDslAstQuery["ast"], (string | number)[]][] {
  if ("ast" in query && query.ast !== undefined) return [[query.ast, [...path, "ast"]]];
  if ("select_when" in query && query.select_when !== undefined) {
    return [...astQueries(query.select_when.select, [...path, "select_when", "select"]), ...astQueries(query.select_when.exists, [...path, "select_when", "exists"])];
  }
  return [];
}

type SharedUtilities = Map<string, Map<string, PolicyDslUtility>>;

/** Each language's define utilities, reporting an id a second define of the language repeats. */
function sharedUtilities(rule: PolicyDslDocument, context: z.RefinementCtx): SharedUtilities {
  const shared: SharedUtilities = new Map();
  for (const [index, check] of rule.clauses.entries()) {
    const define = check.define;
    if (define?.language === undefined) continue;
    const known = shared.get(define.language) ?? new Map<string, PolicyDslUtility>();
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
function checkUtilityCall({ id, args, path }: UtilityCall, util: PolicyDslUtility | undefined, context: z.RefinementCtx): void {
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
function reachedUtilities(calls: UtilityCall[], known: Map<string, PolicyDslUtility>): Set<string> {
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
function checkAstQueryUtilities(ast: PolicyDslAstQuery["ast"], astPath: (string | number)[], shared: SharedUtilities, context: z.RefinementCtx): void {
  const known = shared.get(ast.language) ?? new Map<string, PolicyDslUtility>();
  const local: Record<string, PolicyDslUtility> = ast.utils ?? {};
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
function checkDocumentUtilities(rule: PolicyDslDocument, context: z.RefinementCtx): void {
  const shared = sharedUtilities(rule, context);
  for (const [index, check] of rule.clauses.entries()) {
    const define = check.define;
    if (define?.language !== undefined) {
      const known = shared.get(define.language)!;
      const calls: UtilityCall[] = [];
      utilityCalls(define.utils, ["clauses", index, "define", "utils"], calls);
      for (const call of calls) checkUtilityCall(call, known.get(call.id), context);
    }
    if (check.forbid === undefined) continue;
    for (const [ast, astPath] of astQueries(check.forbid, ["clauses", index, "forbid"])) checkAstQueryUtilities(ast, astPath, shared, context);
  }
}


/** One `when.option` condition: the option is declared, and its value can meet the condition. */
function checkWhenOption(values: Map<string, PolicyDslValue>, name: string, condition: PolicyDslOptionCondition, path: (string | number)[], context: z.RefinementCtx): void {
  if (!values.has(name)) {
    context.addIssue({ code: "custom", path, message: `\`${name}\` is not an option this document declares.` });
    return;
  }
  try {
    policyOptionConditionHolds(values.get(name)!, condition, path);
  } catch (error) {
    if (!(error instanceof PolicyOptionError)) throw error;
    context.addIssue({ code: "custom", path: error.path, message: `${error.message}.` });
  }
}

/** Each ast query's `when.option` names options the document declares, with conditions their values can meet. */
function checkWhenOptions(rule: PolicyDslDocument, context: z.RefinementCtx): void {
  let values: Map<string, PolicyDslValue>;
  try {
    values = policyOptionValues(rule);
  } catch {
    return;
  }
  for (const [index, check] of rule.clauses.entries()) {
    if (check.forbid === undefined) continue;
    for (const [ast, astPath] of astQueries(check.forbid, ["clauses", index, "forbid"])) {
      for (const [name, condition] of Object.entries(ast.when?.option ?? {})) checkWhenOption(values, name, condition, [...astPath, "when", "option", name], context);
    }
  }
}

/** The document as written, `$option` references and all; `policyRuleSchema` resolves them first. */
export const policyResolvedRuleSchema = z
  .object({
    format: z.literal(POLICY_FORMAT),
    id: nonEmptyString,
    revision: z.number().int().positive(),
    mode: z.enum(RULE_MODES),
    description: z.string().optional(),
    scope: policyScopeSchema.optional(),
    clauses: z.array(policyCheckSchema).min(1),
  })
  .strict()
  .superRefine((rule, context) => {
    const ids = new Set<string>();
    for (const [index, check] of rule.clauses.entries()) {
      if (ids.has(check.id)) {
        context.addIssue({ code: "custom", path: ["clauses", index, "id"], message: `Duplicate clause id: ${check.id}.` });
      }
      ids.add(check.id);
    }
    if (rule.clauses.every(check => check.define !== undefined)) {
      context.addIssue({ code: "custom", path: ["clauses"], message: "clauses must hold a clause besides define: a define reports nothing." });
    }
    checkDocumentUtilities(rule as unknown as PolicyDslDocument, context);
    checkWhenOptions(rule as unknown as PolicyDslDocument, context);
  }) as unknown as z.ZodType<PolicyDslDocument>;

/**
 * A `rerail.policy/v1` document. Its `$option` references are resolved first, as the kernel resolves
 * them (`resolvePolicyOptions`), so a parsed document holds none: the catalog stores it resolved,
 * with the configuration its defines' `config` records.
 */
export const policyRuleSchema = z.preprocess((input, context) => {
  try {
    return resolvePolicyOptions(input);
  } catch (error) {
    if (!(error instanceof PolicyOptionError)) throw error;
    context.addIssue({ code: "custom", path: error.path, message: `${error.message}.`, input });
    return input;
  }
}, policyResolvedRuleSchema) as unknown as z.ZodType<PolicyDslDocument>;

export type PolicyScope = PolicyDslScope;
export type PolicyFinding = PolicyDslFindingSpec;
export type PolicyCheck = PolicyDslCheck;
export type PolicyRule = PolicyDslDocument;

/** The check form a check uses; the schema guarantees exactly one. */
export function policyCheckForm(check: PolicyCheck): PolicyCheckForm {
  return POLICY_CHECK_FORMS.find(form => check[form] !== undefined) ?? "forbid";
}
