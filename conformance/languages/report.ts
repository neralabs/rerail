import { z } from "zod";
import { DIVERGENCE_KINDS } from "./pack.js";

/** Every tier report starts with this header. */
export const REPORT_FORMAT = "rerail.language-conformance/v1";

export const headerSchema = z.object({
  format: z.literal(REPORT_FORMAT),
  pack: z.string(),
  tier: z.string(),
  date: z.string().regex(/^\d{4}-\d{2}-\d{2}$/u),
  kernel: z.object({ path: z.string(), version: z.string() }),
  tools: z.array(z.object({ name: z.string(), version: z.string() })),
});
export type ReportHeader = z.infer<typeof headerSchema>;

const pointSchema = z.object({ path: z.string(), line: z.number().int().positive(), column: z.number().int().positive().optional() });

/**
 * Files a clause unsound under syntax errors skipped (`source-incomplete`, "file has syntax errors") where the grammar
 * reports a syntax error, and files no AST clause read because their error regions cover half of them or more
 * (`source-incomplete`, "file did not parse: …"), each at its first error (the engine's line and column). Attributed to
 * T1, not counted as engine failures; upstream findings in them stay FNs. A real-code sample here (only judged ones are
 * listed) stays judged and fails nothing by itself: it fails a shipped rule only through a label the partial read hides.
 */
const syntaxErrorFilesSchema = z.object({
  count: z.number().int().nonnegative(),
  files: z.array(z.object({ path: z.string(), line: z.number().int().positive(), column: z.number().int().positive() })),
});
export type SyntaxErrorLocation = z.infer<typeof syntaxErrorFilesSchema>["files"][number];

/** Upper bound on the FP, FN and reviewed examples a row lists; the counts are always exact. */
export const EXAMPLE_LIMIT = 50;

export const ruleRowSchema = z.object({
  rule: z.string(),
  /** A port under `candidates/`: scored and reported, never failing the run (`reportFailures`). */
  candidate: z.boolean(),
  /** pack.json lists the port's rule with examples; a shipped port with no fixture TP needs them (`reportFailures`). */
  examples: z.boolean(),
  upstream: z.object({ tool: z.string(), rule: z.string() }),
  capabilities: z.array(z.string()),
  /** `fixtures` or `samples`. */
  source: z.string(),
  /** On samples, labels the rule reports on the line of. */
  tp: z.number().int().nonnegative(),
  /** Unexplained only; explained disagreements are in `divergences`. On samples, reports on clean samples. */
  fp: z.number().int().nonnegative(),
  /** On samples, labels the rule does not report on the line of. */
  fn: z.number().int().nonnegative(),
  precision: z.number().nullable(),
  recall: z.number().nullable(),
  /** Samples only: reports on unlabelled lines of samples labelled for the rule, possibly real violations the labels miss; never failing. */
  reviewed: z.number().int().nonnegative(),
  /** The first 50 `reviewed` reports. */
  reviewedFindings: z.array(pointSchema),
  /**
   * Samples only: the clean samples judged for the rule, and of those, the ones it reports nothing on, reads in full
   * (no syntax error, `source-partial` or parse-induced `source-incomplete`) and finishes without an engine failure, with
   * no failure naming no file (stays-silent evidence; 0 on the fixtures).
   */
  cleanSamples: z.number().int().nonnegative(),
  silentCleanSamples: z.number().int().nonnegative(),
  /**
   * Fixtures only (0 on samples): the rule's own upstream units, the original units whose `source.units` origin names
   * its upstream rule (a unit with no origin is never one, nor a rename variant); and of those, the ones upstream
   * reports the rule nowhere in and Rerail reads in full and reports nothing in (stays-silent evidence).
   */
  ownUnits: z.number().int().nonnegative(),
  silentOwnUnits: z.number().int().nonnegative(),
  /** Fixtures only (0 on samples): upstream's findings of the rule on the original units the rule is scored on. */
  upstreamFindings: z.number().int().nonnegative(),
  /**
   * Fixtures only: the rule scored on its tool's rename variants, apart from the originals (the other counts are the
   * originals' alone); an unexplained FP or FN here fails a shipped rule (holds-under-rename). Null when the tool's
   * test data has no rename variants, and on samples.
   */
  renamed: z
    .object({
      units: z.number().int().nonnegative(),
      tp: z.number().int().nonnegative(),
      fp: z.number().int().nonnegative(),
      fn: z.number().int().nonnegative(),
      falsePositives: z.array(pointSchema),
      falseNegatives: z.array(pointSchema),
    })
    .nullable(),
  /**
   * Failures other than `source-partial` and `syntaxErrorFiles` this rule's code rule reported; each fails a shipped
   * rule. On samples, only those on judged files and those naming no file, and never a parse-induced
   * `source-incomplete` (counted in `partialFiles`).
   */
  engineFailures: z.number().int().nonnegative(),
  /** Samples only: engine failures on a sample's context files, which no label judges; listed, never failing (0 on the fixtures). */
  contextFailures: z.number().int().nonnegative(),
  /**
   * Files with syntax errors the code rule's AST clauses ran around (`source-partial`). On samples, the judged ones,
   * with those a parse-induced `source-incomplete` names, plus one per such failure naming no file; never failing.
   */
  partialFiles: z.number().int().nonnegative(),
  /** The first 50 files are listed; the count is exact. */
  syntaxErrorFiles: syntaxErrorFilesSchema,
  divergences: z.array(
    z.object({
      kind: z.enum(DIVERGENCE_KINDS),
      side: z.enum(["fp", "fn"]),
      path: z.string(),
      line: z.number().int().positive(),
      column: z.number().int().positive().optional(),
      evidence: z.string(),
    }),
  ),
  falsePositives: z.array(pointSchema),
  falseNegatives: z.array(pointSchema),
});
export type RuleRow = z.infer<typeof ruleRowSchema>;

export const sourceSummarySchema = z.object({
  source: z.string(),
  /** Analysis units (fixtures) or judged samples evaluated. */
  units: z.number().int().nonnegative(),
  /** Files evaluated, a sample's context files included. */
  files: z.number().int().nonnegative(),
  bytes: z.number().int().nonnegative(),
  /** Every file of the source any rule skipped for its syntax errors. */
  syntaxErrorFiles: syntaxErrorFilesSchema,
  /** Wall time of the evaluation: cold workspace build plus result paging. */
  evaluationMs: z.number().nonnegative(),
});
export type SourceSummary = z.infer<typeof sourceSummarySchema>;

export const t4ReportSchema = headerSchema.extend({
  tier: z.literal("t4-rules"),
  /** What the run covered; a T4 report of record covers every rule, the fixtures and the samples. */
  scope: z.object({ rules: z.array(z.string()), fixtures: z.boolean(), samples: z.boolean() }),
  coverage: z.object({
    rules: z.number().int().nonnegative(),
    minimumRules: z.number().int().positive(),
    /** Declared capability → the rules exercising it. */
    capabilities: z.record(z.string(), z.array(z.string())),
    /** Declared capabilities with neither a rule nor an exemption. */
    uncovered: z.array(z.string()),
    /** Capabilities covered by exemption (`exemptCapabilities` in pack.json), with their reason and T2/T3 coverage. */
    exempt: z.array(z.object({ capability: z.string(), reason: z.string(), evidence: z.string(), coveredBy: z.array(z.string()) })),
  }),
  sources: z.array(sourceSummarySchema),
  rules: z.array(ruleRowSchema),
  /** Divergences whose every source was scored and that explained nothing. */
  staleDivergences: z.array(z.object({ rule: z.string(), source: z.string().optional(), path: z.string(), line: z.number(), kind: z.string() })),
  /** Configuration errors (missing or mismatched recordings, code rules that do not compile, …); each fails the run. */
  errors: z.array(z.string()),
  /**
   * No unexplained FP, no unexplained FN, no engine failure, no stale divergence, no error. On samples, a file read in
   * part and a context file's engine failure fail nothing by themselves.
   */
  pass: z.boolean(),
});
export type T4Report = z.infer<typeof t4ReportSchema>;

/** Header first, one line per row, so a report diff reads rule by rule. */
export function reportJson(report: T4Report): string {
  const parsed = t4ReportSchema.parse(report);
  const { rules, sources, ...head } = parsed;
  const lines = (rows: readonly unknown[]): string => rows.map(row => `    ${JSON.stringify(row)}`).join(",\n");
  const body = JSON.stringify(head, null, 2).replace(/\n\}$/u, "");
  return `${body},\n  "sources": [\n${lines(sources)}\n  ],\n  "rules": [\n${lines(rules)}\n  ]\n}\n`;
}
