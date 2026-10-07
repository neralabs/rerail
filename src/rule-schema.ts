import { z } from "zod";
import { RULE_MODES } from "./rule-mode.js";

export const COSINE_RULE_DOCUMENT_VERSION = 1 as const;

export const MATCH_OPERATORS = ["gte", "lte"] as const;
export type MatchOperator = (typeof MATCH_OPERATORS)[number];
export type SampleSource = "user-input" | "assistant-text" | `tool:${string}`;
export const SAMPLE_BOUNDARIES = ["sentence", "message", "turn", "tool-call"] as const;
export type SampleBoundary = (typeof SAMPLE_BOUNDARIES)[number];

const MAX_ID_LENGTH = 128;
const MAX_REFERENCE_LENGTH = 32_768;
const MAX_ACTION_LENGTH = 16_384;
const MAX_MINIMUM_TOKENS = 8_192;

const nonEmptyString = (maxLength: number) =>
  z.string().refine(value => value.trim().length > 0, "must be a non-empty string").max(maxLength, `must contain at most ${maxLength} characters`);

export const sampleSourceSchema = z.union(
  [z.literal("user-input"), z.literal("assistant-text"), z.templateLiteral(["tool:", z.string().min(1)])],
  { error: "must be user-input, assistant-text, or tool:<name>" },
) satisfies z.ZodType<SampleSource>;

/**
 * What a structural condition reads: the sample text, or, for a tool call only, the tool's name,
 * its arguments as stable JSON (`input`), or one string argument (`input.<key>[.<key>…]`).
 */
export type MatchField = "text" | "tool.name" | "input" | `input.${string}`;

/** A structural match, in the policy DSL's vocabulary: Rust regex syntax, `eq`, and `all`/`any`/`not`. */
export type MatchCondition =
  | { field: MatchField; regex: string }
  | { field: MatchField; eq: string }
  | { field: MatchField; contains: string }
  | { all: MatchCondition[] }
  | { any: MatchCondition[] }
  | { not: MatchCondition };

export type SimilarityMatch = { operator: MatchOperator; threshold: number };

const MATCH_FIELD_PATTERN = /^(?:text|tool\.name|input|input(?:\.[^.]+)+)$/u;
const CONDITION_TESTS = ["regex", "eq", "contains"] as const;
const CONDITION_COMBINATORS = ["all", "any", "not"] as const;

export const matchConditionSchema: z.ZodType<MatchCondition> = z.lazy(() =>
  z
    .object({
      field: z.string().regex(MATCH_FIELD_PATTERN, "must be text, tool.name, input, or input.<path>").optional(),
      // The kernel judges the dialect and size limit: see kernelRegexIssues in rule-regexes.ts.
      regex: z.string().min(1, "must not be empty").optional(),
      eq: z.string().optional(),
      contains: z.string().min(1, "must not be empty").optional(),
      all: z.array(matchConditionSchema).min(1, "must contain at least one condition").optional(),
      any: z.array(matchConditionSchema).min(1, "must contain at least one condition").optional(),
      not: matchConditionSchema.optional(),
    })
    .strict()
    .superRefine((condition, context) => {
      const combinators = CONDITION_COMBINATORS.filter(key => condition[key] !== undefined);
      const tests = CONDITION_TESTS.filter(key => condition[key] !== undefined);
      if (combinators.length > 0) {
        if (combinators.length + tests.length + (condition.field === undefined ? 0 : 1) > 1) {
          context.addIssue({ code: "custom", message: "must contain exactly one of field, all, any, or not" });
        }
        return;
      }
      if (condition.field === undefined) {
        context.addIssue({ code: "custom", message: "must contain field, all, any, or not" });
      } else if (tests.length !== 1) {
        context.addIssue({ code: "custom", message: "must contain exactly one of regex, eq, or contains" });
      }
    }),
) as unknown as z.ZodType<MatchCondition>;

const similarityMatchSchema = z
  .object({
    operator: z.enum(MATCH_OPERATORS),
    threshold: z.number().min(-1).max(1),
  })
  .strict();

/** Whether a rule's match is a similarity threshold rather than a structural condition. */
export function isSimilarityMatch(match: CosineRule["match"]): match is SimilarityMatch {
  return "operator" in match;
}

/** One-line summary for rule listings: `gte 0.8` for a similarity match, `structural` otherwise. */
export function describeMatch(match: CosineRule["match"]): string {
  return isSimilarityMatch(match) ? `${match.operator} ${match.threshold}` : "structural";
}

/** Tolerates malformed conditions: rule-level refinements still run after a condition reports an issue. */
function readsToolFields(condition: MatchCondition | undefined): boolean {
  if (typeof condition !== "object" || condition === null) return false;
  if ("field" in condition) return condition.field !== "text";
  if ("all" in condition) return condition.all.some(readsToolFields);
  if ("any" in condition) return condition.any.some(readsToolFields);
  return "not" in condition && readsToolFields(condition.not);
}

export const cosineRuleSchema = z
  .object({
    id: nonEmptyString(MAX_ID_LENGTH),
    mode: z.enum(RULE_MODES),
    /** Required by a similarity match and rejected by a structural one. */
    reference: nonEmptyString(MAX_REFERENCE_LENGTH).optional(),
    match: z.union([similarityMatchSchema, matchConditionSchema]),
    sample: z
      .object({
        source: sampleSourceSchema,
        boundary: z.enum(SAMPLE_BOUNDARIES),
        minimumTokens: z.number().int().min(1).max(MAX_MINIMUM_TOKENS).optional(),
      })
      .strict(),
    action: z
      .object({
        content: nonEmptyString(MAX_ACTION_LENGTH),
      })
      .strict(),
    repeat: z
      .discriminatedUnion("mode", [
        z.object({ mode: z.literal("once") }).strict(),
        z.object({ mode: z.literal("always") }).strict(),
        z.object({ mode: z.literal("after-gap"), gapTurns: z.number().int().positive() }).strict(),
      ])
      .optional(),
  })
  .strict()
  .superRefine((rule, context) => {
    if (typeof rule.match !== "object" || rule.match === null || typeof rule.sample !== "object" || rule.sample === null) return;
    if ("operator" in rule.match) {
      if (rule.reference === undefined) context.addIssue({ code: "custom", path: ["reference"], message: "is required by a similarity match" });
      if (rule.sample.source === "tool:*") {
        context.addIssue({ code: "custom", path: ["sample", "source"], message: "tool:* is supported only by structural matches" });
      }
      return;
    }
    if (rule.reference !== undefined) {
      context.addIssue({ code: "custom", path: ["reference"], message: "must be omitted: a structural match computes no similarity" });
    }
    if (rule.sample.minimumTokens !== undefined) {
      context.addIssue({ code: "custom", path: ["sample", "minimumTokens"], message: "must be omitted: a structural match does not tokenize" });
    }
    if (!readsToolFields(rule.match)) return;
    if (typeof rule.sample.source !== "string" || !rule.sample.source.startsWith("tool:")) {
      context.addIssue({ code: "custom", path: ["sample", "source"], message: "must be tool:<name> or tool:* when the match reads tool.name or input" });
    } else if (rule.sample.boundary !== "tool-call") {
      context.addIssue({ code: "custom", path: ["sample", "boundary"], message: "must be tool-call when the match reads tool.name or input" });
    }
  });

export type CosineRule = z.infer<typeof cosineRuleSchema>;

export const cosineRuleDocumentSchema = z
  .object({
    version: z.literal(COSINE_RULE_DOCUMENT_VERSION),
    rules: z.array(cosineRuleSchema),
  })
  .strict();

export type CosineRuleDocument = z.infer<typeof cosineRuleDocumentSchema>;

function record(value: unknown, path: string): Record<string, unknown> {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    throw new TypeError(`${path} must be an object`);
  }
  return value as Record<string, unknown>;
}

export function parseSampleSource(value: unknown, path: string): SampleSource {
  const result = sampleSourceSchema.safeParse(value);
  if (!result.success) throw new TypeError(`${path} must be user-input, assistant-text, or tool:<name>`);
  return result.data;
}

/** Parse one rule, reporting the first failing field as `<path>.<field>: <reason>`. */
export function parseCosineRule(value: unknown, path = "rule"): CosineRule {
  const result = cosineRuleSchema.safeParse(value);
  if (result.success) return result.data;
  const issue = result.error.issues[0]!;
  const location = [path, ...issue.path.map(String)].join(".");
  throw new TypeError(`${location} ${issue.message}`);
}

export function parseCosineRuleDocument(value: unknown): CosineRule[] {
  const document = record(value, "rule document");
  if (document.version !== COSINE_RULE_DOCUMENT_VERSION) {
    throw new TypeError(`rule document.version must equal ${COSINE_RULE_DOCUMENT_VERSION}`);
  }
  if (!Array.isArray(document.rules)) {
    throw new TypeError("rule document.rules must be an array");
  }

  const rules = document.rules.map((rule, index) => parseCosineRule(rule, `rule document.rules[${index}]`));
  const ids = new Set<string>();
  for (const rule of rules) {
    if (ids.has(rule.id)) throw new TypeError(`duplicate rule id: ${rule.id}`);
    ids.add(rule.id);
  }
  return rules;
}
