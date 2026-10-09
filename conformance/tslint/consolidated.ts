import { PASS_AUDIT, type PassClass } from "./audit.js";
import type { EngineProbeResult } from "./engine-probes.js";
import type { GrammarProbeResult } from "./grammar-gaps.js";
import { annotationOf, effectiveVerdict, type EffectiveVerdict, type FinalVerdict, type RuleResult } from "./runner.js";

export type { EffectiveVerdict };

export interface RuleSummary {
  rule: string;
  /** In the TSLint pack (`golden.ts`). */
  golden: boolean;
  typeInfo: boolean;
  fixer: boolean;
  raw: FinalVerdict;
  effective: EffectiveVerdict;
  passClass: PassClass | undefined;
  /** Why the PASS is not genuine, or where it is bounded. */
  passNote: string | undefined;
  raw_tp_fp_fn: [number, number, number];
  repaired_tp_fp_fn: [number, number, number] | undefined;
  engineFailures: number;
  /** Files marked `source-partial`: informational, not part of the verdict. */
  partialFiles: number;
  primitive: string | undefined;
  secondary: string[];
  needs: string[];
  reason: string;
  documentKiB: number;
}

export interface Consolidated {
  rawCounts: Record<string, number>;
  effectiveCounts: Record<string, number>;
  passClassCounts: Record<string, number>;
  /** Primary primitive to the rules it blocks (effective verdict not PASS) and the PASS rules that need it to be genuine. */
  histogram: Array<{ primitive: string; blocked: string[]; passNeeds: string[] }>;
  secondary: Array<{ primitive: string; rules: string[] }>;
  typeNeeds: Array<{ need: string; rules: string[] }>;
  typeUnblocked: Array<{ provides: string[]; rules: string[] }>;
  largePorts: Array<{ rule: string; kib: number }>;
  rules: RuleSummary[];
  grammar: GrammarProbeResult[];
  engine: EngineProbeResult[];
}

const LARGE_PORT_BYTES = 50 * 1024;

function firstSentence(text: string): string {
  const match = /^(.*?[.!?])(\s|$)/.exec(text.trim());
  const sentence = (match?.[1] ?? text).trim();
  return sentence.length > 220 ? `${sentence.slice(0, 217)}...` : sentence;
}


const increment = (counts: Record<string, number>, key: string): void => {
  counts[key] = (counts[key] ?? 0) + 1;
};

export function summarize(rules: RuleResult[], grammar: GrammarProbeResult[], engine: EngineProbeResult[]): Consolidated {
  const summaries: RuleSummary[] = rules.map(rule => {
    const effective = effectiveVerdict(rule);
    const audit = effective.startsWith("PASS") ? PASS_AUDIT[rule.rule] : undefined;
    const sidecar = annotationOf(rule);
    const repaired = rule.repaired;
    return {
      rule: rule.rule,
      golden: rule.golden,
      typeInfo: rule.requiresTypeInfo,
      fixer: rule.hasFixer,
      raw: rule.verdict,
      effective,
      passClass: audit?.class,
      passNote: audit === undefined ? undefined : `${audit.depth === undefined ? "" : `[${audit.depth}] `}${audit.note}`,
      raw_tp_fp_fn: [rule.truePositives, rule.falsePositives, rule.falseNegatives],
      repaired_tp_fp_fn: repaired === undefined ? undefined : [repaired.truePositives, repaired.falsePositives, repaired.falseNegatives],
      engineFailures: rule.engineFailures,
      partialFiles: rule.partialFiles,
      // A repaired-fixture PASS already resolved its raw blocker (fixture-artifact/grammar-gap): the sidecar's
      // primitive describes that already-resolved blocker, not what the PASS still needs. Every other verdict
      // (including a raw PASS with an AUTO sidecar, and every non-PASS verdict) keeps the sidecar fallback.
      primitive: audit?.primitive ?? (effective === "PASS (repaired fixture)" ? undefined : sidecar?.primitive),
      secondary: sidecar?.secondary ?? [],
      needs: sidecar?.needs ?? [],
      reason: rule.compileError ?? (audit !== undefined && audit.class !== "genuine" ? audit.note : firstSentence(sidecar?.reason ?? audit?.note ?? "")),
      documentKiB: Math.round(rule.documentBytes / 1024),
    };
  });

  const rawCounts: Record<string, number> = {};
  const effectiveCounts: Record<string, number> = {};
  const passClassCounts: Record<string, number> = {};
  for (const summary of summaries) {
    increment(rawCounts, summary.raw);
    increment(effectiveCounts, summary.effective);
    if (summary.effective.startsWith("PASS")) increment(passClassCounts, summary.passClass ?? "unaudited");
  }

  const histogram = new Map<string, { blocked: string[]; passNeeds: string[] }>();
  const secondary = new Map<string, string[]>();
  const label = (summary: RuleSummary): string => `${summary.rule}${summary.typeInfo ? " (T)" : ""}`;
  for (const summary of summaries) {
    const passing = summary.effective.startsWith("PASS");
    if (summary.primitive !== undefined && (!passing || (summary.passClass !== undefined && summary.passClass !== "genuine"))) {
      const entry = histogram.get(summary.primitive) ?? { blocked: [], passNeeds: [] };
      (passing ? entry.passNeeds : entry.blocked).push(label(summary));
      histogram.set(summary.primitive, entry);
    }
    for (const primitive of summary.secondary) secondary.set(primitive, [...(secondary.get(primitive) ?? []), label(summary)]);
  }

  const typeRules = summaries.filter(summary => summary.needs.length > 0);
  const needs = new Map<string, string[]>();
  for (const summary of typeRules) for (const need of summary.needs) needs.set(need, [...(needs.get(need) ?? []), summary.rule]);
  const layers = [
    ["symbol-resolution"],
    ["symbol-resolution", "cross-file", "compiler-options"],
    ["symbol-resolution", "cross-file", "compiler-options", "overload-resolution"],
    ["symbol-resolution", "cross-file", "compiler-options", "overload-resolution", "expression-type"],
    ["symbol-resolution", "cross-file", "compiler-options", "overload-resolution", "expression-type", "lib-types"],
  ];
  const typeUnblocked = layers.map(provides => ({ provides, rules: typeRules.filter(summary => summary.needs.every(need => provides.includes(need))).map(summary => summary.rule) }));

  const order = (a: { name: string; size: number }, b: { name: string; size: number }): number => b.size - a.size || a.name.localeCompare(b.name);
  return {
    rawCounts,
    effectiveCounts,
    passClassCounts,
    histogram: [...histogram]
      .map(([primitive, entry]) => ({ name: primitive, size: entry.blocked.length + entry.passNeeds.length, primitive, ...entry }))
      .sort(order)
      .map(({ primitive, blocked, passNeeds }) => ({ primitive, blocked, passNeeds })),
    secondary: [...secondary]
      .map(([primitive, list]) => ({ name: primitive, size: list.length, primitive, rules: list }))
      .sort(order)
      .map(({ primitive, rules: list }) => ({ primitive, rules: list })),
    typeNeeds: [...needs]
      .map(([need, list]) => ({ name: need, size: list.length, need, rules: list }))
      .sort(order)
      .map(({ need, rules: list }) => ({ need, rules: list })),
    typeUnblocked,
    largePorts: rules
      .filter(rule => rule.documentBytes > LARGE_PORT_BYTES)
      .map(rule => ({ rule: rule.rule, kib: Math.round(rule.documentBytes / 1024) }))
      .sort((a, b) => b.kib - a.kib),
    rules: summaries,
    grammar,
    engine,
  };
}
