// report.md's rows for metric.ts's figures: each pack's start-up, net times, phases and workload under its gross
// metrics, the net ratio beside the gross one in the pack table, and the rule cost multiple in the drill-down. All
// informational: none of it is a status.

import { type CellCost, KNOWN_GAPS, type NetItem } from "./metric.js";
import { type CellItem, formatMs, type RuleItem } from "./report-model.js";

function ms(value: number | null): string {
  return value === null ? "–" : formatMs(value);
}

function ratio(value: number | null): string {
  return value === null || !Number.isFinite(value) ? "–" : `${value.toFixed(2)}×`;
}

function count(value: number | null): string {
  if (value === null) return "–";
  if (value >= 1e6) return `${(value / 1e6).toFixed(2)} M`;
  if (value >= 1e4) return `${(value / 1e3).toFixed(0)} k`;
  return String(Math.round(value));
}

function scaled(value: number | null, digits: number): string {
  return value === null ? "–" : value.toFixed(digits);
}

/** A cell's net item of one metric, as the pack table's column shows it: the ratio, or why there is none. */
export function netColumn(cost: CellCost | null, metric: NetItem["metric"]): string {
  const item = cost?.net.find((n) => n.metric === metric);
  if (item === undefined) return "–";
  return item.ratio === null ? "n/a" : ratio(item.ratio);
}

/** The rows a pack's metric table gets under its gross metrics, one column per corpus. */
export function costRows(cells: readonly (CellItem | undefined)[]): string[] {
  const row = (label: string, text: (cost: CellCost) => string): string => `| ${label} | ${cells.map((c) => (c?.cost == null ? "–" : text(c.cost))).join(" | ")} |`;
  const net = (metric: NetItem["metric"]) => (cost: CellCost): string => {
    const n = cost.net.find((x) => x.metric === metric)!;
    return n.baseline_ms === null && n.candidate_ms === null ? "–" : `${ms(n.baseline_ms)} → ${ms(n.candidate_ms)} ${ratio(n.ratio)}`;
  };
  return [
    row("Start-up T0 (empty rule set), wall cold", (c) => `${ms(c.baseline.fixed?.["wall-cold"] ?? null)} → ${ms(c.candidate.fixed?.["wall-cold"] ?? null)}`),
    row("Net wall cold (T − T0)", net("wall-cold")),
    row("Net wall warm", net("wall-warm")),
    row("Net CPU cold", net("cpu-cold")),
    row("Net CPU warm", net("cpu-warm")),
    row("Phases load / compile / evaluate, cold", (c) => {
      const p = (side: CellCost["baseline"]): string => (side.phases === null ? "–" : `${ms(side.phases.load_ms)} / ${ms(side.phases.compile_ms)} / ${ms(side.phases.evaluate_ms)}`);
      return `${p(c.baseline)} → ${p(c.candidate)}`;
    }),
    row("Compile per rule", (c) => `${scaled(c.baseline.compile_ms_per_rule, 1)} → ${scaled(c.candidate.compile_ms_per_rule, 1)} ms`),
    row("Evaluate per rule·node (less the empty set's)", (c) => `${scaled(c.baseline.evaluate_ns_per_rule_node, 0)} → ${scaled(c.candidate.evaluate_ns_per_rule_node, 0)} ns`),
    row("Workload", (c) => {
      const w = c.workload;
      return w === null ? "not counted" : `${w.files} files, ${(w.bytes / 1e6).toFixed(2)} MB, ${count(w.lines)} lines, ${count(w.tokens)} tokens, ${count(w.nodes)} nodes`;
    }),
  ];
}

/** The drill-down's multiple column: baseline → candidate, with the rule's flags. */
export function multipleColumn(rule: RuleItem): string {
  if (rule.multiple === undefined) return "–";
  const text = `${scaled(rule.multiple.baseline, 1)}× → ${scaled(rule.multiple.candidate, 1)}×`;
  return rule.flags !== undefined && rule.flags.length > 0 ? `${text} ⚑ ${rule.flags.join("; ")}` : text;
}

/** The note under a pack table: what the rows below the gross metrics are, and their known gaps. */
export const COST_NOTE = `Rows from start-up down are reported, not judged: net = median run − median start-up (the side's empty rule set on the same corpus); evaluate per rule·node = cold evaluate less the empty set's, over Σ rules × syntax nodes of the languages they read. Known gaps: ${KNOWN_GAPS.join("; ")}.`;
