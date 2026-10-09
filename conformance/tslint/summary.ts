import { z } from "zod";
import { excludedCount, type ConformanceReport } from "./report.js";
import { compareDeterministicStrings } from "../../src/engine/index.js";

/** One scalar row per rule: the committed, diffable record of a run. The full per-fixture detail stays in tmp/. */
const rowSchema = z.object({
  rule: z.string(),
  /** In the TSLint pack; absent in summaries written before the golden set. */
  golden: z.boolean().default(false),
  raw: z.string(),
  effective: z.string(),
  tp: z.number(),
  fp: z.number(),
  fn: z.number(),
  engineFailures: z.number(),
  partialFiles: z.number(),
  /** Fixture files outside a golden default doc's shipped scope; absent in summaries written before the golden set. */
  outOfScope: z.number().default(0),
  primitive: z.string().nullable(),
  defaultBytes: z.number(),
  totalBytes: z.number(),
  docs: z.number(),
  checks: z.number(),
  relations: z.number(),
  inside: z.number(),
  has: z.number(),
  precedes: z.number(),
  follows: z.number(),
  not: z.number(),
  any: z.number(),
  all: z.number(),
  maxDepth: z.number(),
  evalMs: z.number().nullable(),
});
export type SummaryRow = z.infer<typeof rowSchema>;

const SUMMARY_FORMAT = "rerail-tslint-conformance-summary/v1";
const summarySchema = z.object({
  format: z.literal(SUMMARY_FORMAT),
  label: z.string(),
  generatedAt: z.string(),
  tslintSha: z.string(),
  kernel: z.object({ path: z.string(), version: z.string() }),
  runs: z.number(),
  /** Rules outside the equivalence set; absent in summaries written before exclusions existed. */
  excluded: z.array(z.object({ rule: z.string(), reason: z.string() })).default([]),
  rules: z.array(rowSchema),
});
export type ConformanceSummary = z.infer<typeof summarySchema>;

export function buildSummary(report: ConformanceReport): ConformanceSummary {
  const consolidated = new Map(report.consolidated.rules.map(summary => [summary.rule, summary]));
  return {
    format: SUMMARY_FORMAT,
    label: report.label,
    generatedAt: report.generatedAt,
    tslintSha: report.tslintSha,
    kernel: report.kernel,
    runs: report.runs,
    excluded: report.inventory.rulesExcluded,
    rules: report.rules.map(rule => {
      const summary = consolidated.get(rule.rule);
      return {
        rule: rule.rule,
        golden: rule.golden,
        raw: rule.verdict,
        effective: summary?.effective ?? rule.verdict,
        tp: rule.truePositives,
        fp: rule.falsePositives,
        fn: rule.falseNegatives,
        engineFailures: rule.engineFailures,
        partialFiles: rule.partialFiles,
        outOfScope: rule.outOfScope.length,
        primitive: summary?.primitive ?? null,
        ...rule.size,
        evalMs: rule.evalMs === null ? null : Math.round(rule.evalMs * 1000) / 1000,
      };
    }),
  };
}

/** Pretty header, one line per rule, so a milestone diff reads rule by rule. */
export function summaryJson(summary: ConformanceSummary): string {
  const { rules, excluded, ...header } = summary;
  const head = JSON.stringify(header, null, 2).replace(/\n}$/, "");
  const rows = (list: object[]): string => (list.length === 0 ? "[]" : `[\n${list.map(row => `    ${JSON.stringify(row)}`).join(",\n")}\n  ]`);
  return `${head},\n  "excluded": ${rows(excluded)},\n  "rules": ${rows(rules)}\n}\n`;
}

export function parseSummary(text: string): ConformanceSummary {
  return summarySchema.parse(JSON.parse(text));
}

const TOTALLED = ["tp", "fp", "fn", "engineFailures", "partialFiles", "outOfScope", "defaultBytes", "totalBytes", "docs", "checks", "relations", "not", "any", "all"] as const;
/** Fields whose change puts a rule in the changed-rules table; eval time alone is noise. */
const COMPARED = ["golden", "raw", "effective", "primitive", ...TOTALLED, "inside", "has", "precedes", "follows", "maxDepth"] as const;

const signed = (value: number): string => (value > 0 ? `+${value}` : value === 0 ? "0" : String(value));
const arrow = (before: string | number, after: string | number): string => (before === after ? String(after) : `${before} → ${after}`);
const fixed = (value: number | null): string => (value === null ? "-" : value.toFixed(2));

function counts(rows: SummaryRow[], pick: (row: SummaryRow) => string): Map<string, number> {
  const result = new Map<string, number>();
  for (const row of rows) result.set(pick(row), (result.get(pick(row)) ?? 0) + 1);
  return result;
}

export function deltaMarkdown(before: ConformanceSummary, after: ConformanceSummary): string {
  const lines: string[] = [];
  lines.push(`# TSLint conformance delta: ${after.label} against ${before.label}`, "");
  lines.push(`- before: the ${before.label} summary (generated ${before.generatedAt}; kernel ${before.kernel.version}; not retained, this delta records the change)`);
  lines.push(`- after: ${after.label} (generated ${after.generatedAt}; kernel ${after.kernel.version}); timings median of ${after.runs}`);
  lines.push(`- rules scored: ${before.rules.length} → ${after.rules.length}; ${excludedCount(before.excluded)} → ${excludedCount(after.excluded)}`);
  lines.push("");

  const beforeRows = new Map(before.rules.map(row => [row.rule, row]));
  const afterRows = new Map(after.rules.map(row => [row.rule, row]));
  const common = after.rules.filter(row => beforeRows.has(row.rule));

  lines.push("## Totals", "", "Over rules present in both runs.", "", "| measure | before | after | Δ |", "|---|---|---|---|");
  const commonBefore = common.map(row => beforeRows.get(row.rule)!);
  lines.push(`| golden | ${commonBefore.filter(row => row.golden).length} | ${common.filter(row => row.golden).length} | ${signed(common.filter(row => row.golden).length - commonBefore.filter(row => row.golden).length)} |`);
  for (const [name, pick] of [
    ["raw", (row: SummaryRow) => row.raw],
    ["effective", (row: SummaryRow) => row.effective],
  ] as const) {
    const was = counts(commonBefore, pick);
    const now = counts(common, pick);
    for (const verdict of [...new Set([...was.keys(), ...now.keys()])].sort(compareDeterministicStrings)) {
      const a = was.get(verdict) ?? 0;
      const b = now.get(verdict) ?? 0;
      lines.push(`| ${name} ${verdict} | ${a} | ${b} | ${signed(b - a)} |`);
    }
  }
  for (const key of TOTALLED) {
    const a = commonBefore.reduce((sum, row) => sum + row[key], 0);
    const b = common.reduce((sum, row) => sum + row[key], 0);
    lines.push(`| ${key} | ${a} | ${b} | ${signed(b - a)} |`);
  }
  const msA = commonBefore.reduce((sum, row) => sum + (row.evalMs ?? 0), 0);
  const msB = common.reduce((sum, row) => sum + (row.evalMs ?? 0), 0);
  lines.push(`| evalMs (sum) | ${msA.toFixed(1)} | ${msB.toFixed(1)} | ${signed(Math.round((msB - msA) * 10) / 10)} |`, "");

  const flipped = common.filter(row => {
    const was = beforeRows.get(row.rule)!;
    return was.raw !== row.raw || was.effective !== row.effective;
  });
  lines.push("## Verdict changes", "");
  if (flipped.length === 0) lines.push("None.", "");
  else {
    lines.push("| rule | raw | effective |", "|---|---|---|");
    for (const row of flipped) {
      const was = beforeRows.get(row.rule)!;
      lines.push(`| ${row.rule} | ${arrow(was.raw, row.raw)} | ${arrow(was.effective, row.effective)} |`);
    }
    lines.push("");
  }

  const changed = common.filter(row => {
    const was = beforeRows.get(row.rule)!;
    return COMPARED.some(key => was[key] !== row[key]);
  });
  lines.push("## Changed rules", "", "Rules whose verdict, score or port shape changed (eval ms alone is noise and does not qualify).", "");
  if (changed.length === 0) lines.push("None.", "");
  else {
    lines.push(
      "| rule | golden | effective | TP/FP/FN | out of scope | total bytes | Δ bytes | docs | checks | relations (in/has/pre/fol) | not | any/all | depth | eval ms |",
      "|---|---|---|---|---|---|---|---|---|---|---|---|---|---|",
    );
    for (const row of changed) {
      const was = beforeRows.get(row.rule)!;
      const score = (r: SummaryRow): string => `${r.tp}/${r.fp}/${r.fn}`;
      const relations = (r: SummaryRow): string => `${r.relations} (${r.inside}/${r.has}/${r.precedes}/${r.follows})`;
      const junctions = (r: SummaryRow): string => `${r.any}/${r.all}`;
      lines.push(
        `| ${row.rule} | ${arrow(String(was.golden), String(row.golden))} | ${arrow(was.effective, row.effective)} | ${arrow(score(was), score(row))} | ${arrow(was.outOfScope, row.outOfScope)} | ${arrow(was.totalBytes, row.totalBytes)} | ${signed(row.totalBytes - was.totalBytes)} | ${arrow(was.docs, row.docs)} | ${arrow(was.checks, row.checks)} | ${arrow(relations(was), relations(row))} | ${arrow(was.not, row.not)} | ${arrow(junctions(was), junctions(row))} | ${arrow(was.maxDepth, row.maxDepth)} | ${arrow(fixed(was.evalMs), fixed(row.evalMs))} |`,
      );
    }
    lines.push("");
  }

  const nowExcluded = new Map(after.excluded.map(({ rule, reason }) => [rule, reason]));
  const added = after.rules.filter(row => !beforeRows.has(row.rule)).map(row => row.rule);
  const excluded = before.rules.filter(row => !afterRows.has(row.rule) && nowExcluded.has(row.rule));
  const removed = before.rules.filter(row => !afterRows.has(row.rule) && !nowExcluded.has(row.rule)).map(row => row.rule);
  if (excluded.length > 0) {
    lines.push("## Newly excluded", "", "Scored before, excluded now; not part of the totals above.", "", "| rule | before (effective, TP/FP/FN) | reason |", "|---|---|---|");
    for (const row of excluded) lines.push(`| ${row.rule} | ${row.effective}, ${row.tp}/${row.fp}/${row.fn} | ${nowExcluded.get(row.rule)} |`);
    lines.push("");
  }
  if (added.length > 0) lines.push(`Only in after: ${added.join(", ")}.`, "");
  if (removed.length > 0) lines.push(`Only in before: ${removed.join(", ")}.`, "");
  return `${lines.join("\n")}\n`;
}
