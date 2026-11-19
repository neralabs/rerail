// report.md of the ship / no-ship report (report-model.ts), in levels a reader opens only as far as the question needs:
// the verdict on one line; per candidate one row per pack with its three slices side by side (A all rules × all files,
// B one rule × all files, C all rules × one file); each pack's slices behind <details>, open where the pack fails:
// A's metrics by corpus, B's fixed cost, rule statistics and worst rules, C's three files, and the raw runs with the
// command that reproduces the cell. Every failing item names the phase that moved. GitHub renders the <details>; every
// pack has an anchor.

import { type CandidateItem, type CellItem, formatMs, type ItemStatus, type MetricItem, metricLabel, type OneFileItem, type OneRuleItem, type PackItem, type Report, type RuleItem, type RuleStatsItem, signedMs, type SliceStatus, type TimeItem } from "./report-model.js";
import { COST_NOTE, costRows, multipleColumn } from "./metric-report.js";
import type { WaitItem, WaitMetricItem, WaitsSection } from "./waits-model.js";

const CORPUS_LABEL = { fixtures: "Fixtures", real: "Real code" } as const;
const TOP_RULES = 5;

function figure(unit: MetricItem["unit"], value: number | null): string {
  if (value === null) return "–";
  return unit === "mb" ? `${Math.round(value)} MB` : formatMs(value);
}

function times(ratio: number | null): string {
  return ratio === null || !Number.isFinite(ratio) ? "–" : `${ratio.toFixed(2)}×`;
}

function statusWord(status: ItemStatus | SliceStatus | "incomplete" | "no_baseline"): string {
  return { fail: "**FAIL**", noisy: "pass ≈", pass: "pass", not_compared: "n/a", missing: "**missing**", incomplete: "**incomplete**", no_baseline: "no baseline" }[status];
}

/** A ratio as a table shows it: bold when it fails, ≈ when it failed only at the median (noise). */
function markedRatio(ratio: number | null, status: ItemStatus): string {
  const text = times(ratio);
  return status === "fail" ? `**${text}**` : status === "noisy" ? `${text} ≈` : text;
}

function anchor(p: PackItem): string {
  return `${p.candidate}-${p.pack}`;
}

function percent(value: number | null): string {
  return value === null ? "–" : `${Math.round(value * 100)}%`;
}

/** The summary table's slice B cells: rules, fixed cost, rule ratio median · p95, counted / before the filter, over 1.5×. */
function sliceBCells(B: PackItem["summary"]["slices"]["B"]): string[] {
  if (B.status === "missing") return [statusWord("missing"), "–", "–", "–", "–"];
  const fixed = (["fixtures", "real"] as const).filter((corpus) => corpus in B.fixed_wall).map((corpus) => times(B.fixed_wall[corpus] ?? null)).join(" · ") || "–";
  const s = B.stats;
  if (s === null) return [String(B.rules), fixed, "–", "–", "–"];
  return [String(B.rules), fixed, `${times(s.median_ratio)} · ${times(s.p95_ratio)}`, `${s.failing.length > 0 ? `**${s.failing.length}**` : "0"} / ${s.over_bar_before.length}`, String(s.over_alert.length)];
}

/** The summary table's findings cell: A's verdict, then B's rules and C's files that differ, else those the same. */
function findingsCell(p: PackItem): string {
  const { B, C } = p.summary.slices;
  const findingsA = { same: "same", no_worse: "no worse", worse: "**worse**", not_compared: "–" }[p.summary.findings];
  const findingsB = B.findings.identical + B.findings.differing === 0 ? "–" : B.findings.differing > 0 ? `**${B.findings.differing} differ**` : `${B.findings.identical} same`;
  const findingsC = C.files.length === 0 ? "–" : C.files.some((f) => f.findings_same === false) ? `**${C.files.filter((f) => f.findings_same === false).length} differ**` : `${C.files.filter((f) => f.findings_same === true).length} same`;
  return `${findingsA} · ${findingsB} · ${findingsC}`;
}

/** The summary table's row: each slice's figures side by side, the findings per slice, the box's load. */
function packRow(p: PackItem): string {
  const a = (corpus: "fixtures" | "real"): string => {
    const entry = p.summary.wall_cold[corpus];
    if (entry === undefined) return "–";
    if (!entry.completed) return "**did not finish**";
    return entry.ratio === null ? "n/a" : markedRatio(entry.ratio, entry.status);
  };
  const C = p.summary.slices.C;
  const c = (["largest", "median", "small"] as const).map((label) => {
    const file = C.files.find((f) => f.label === label);
    return file === undefined ? (C.status === "missing" ? statusWord("missing") : "–") : markedRatio(file.worst_ratio, file.metrics_status);
  });
  const load = `${p.summary.load.load_max === null ? "–" : p.summary.load.load_max.toFixed(1)} · ${percent(p.summary.load.sibling_busy_max)}`;
  return `| [${p.pack}](#${anchor(p)}) | ${statusWord(p.status)}${p.counted ? "" : " (reported only)"} | ${a("fixtures")} | ${a("real")} | ${sliceBCells(p.summary.slices.B).join(" | ")} | ${c.join(" | ")} | ${findingsCell(p)} | ${load} | ${(p.summary.hint ?? "").replaceAll("|", "\\|")} |`;
}

const ORDER: Record<PackItem["status"], number> = { fail: 0, incomplete: 1, noisy: 2, not_compared: 3, pass: 4, no_baseline: 5 };

function sortedPacks(packs: readonly PackItem[]): PackItem[] {
  return [...packs].sort((a, b) => ORDER[a.status] - ORDER[b.status] || (b.summary.worst?.ratio ?? 0) - (a.summary.worst?.ratio ?? 0) || (a.pack < b.pack ? -1 : 1));
}

function metricText(m: MetricItem): string {
  if (m.status === "not_compared") return "n/a";
  const spread = m.within_spread && m.status === "fail" ? " (within the run-to-run spread)" : "";
  const confirm = m.confirm === null || m.status === "pass" ? "" : `; confirmation ${times(m.confirm.ratio)}`;
  return `${figure(m.unit, m.baseline.median)} → ${figure(m.unit, m.candidate.median)} ${markedRatio(m.ratio, m.status)}${spread}${confirm}`;
}

function findingsText(cell: CellItem): string {
  if (cell.findings !== null) {
    const f = cell.findings;
    const counts = (c: typeof f.baseline): string => `${c.tp} / ${c.fp} / ${c.fn}`;
    const worse = f.status === "fail" ? ` — **${f.lost_tp} lost TP, ${f.new_fp} new FP**` : "";
    return `${counts(f.baseline)} → ${counts(f.candidate)} (TP / FP / FN)${worse}`;
  }
  if (cell.agreement !== null) return `agreement: ${cell.agreement.both} both, ${cell.agreement.baseline_only} baseline only, ${cell.agreement.candidate_only} candidate only`;
  return "–";
}

/**
 * How the cell ran, beside its completion: how far a failed `.rerail` whole-pack run got, the chunked run standing in for
 * it, whether run-policies ran the policies combined or alone, what the engine printed about it, and the cell's caps
 * when packs ran under different ones (`--pack-cap`).
 */
function cellNotes(cell: CellItem, capsVary: boolean): string[] {
  const { chunked, grouping, progress } = cell;
  return [
    progress === null ? "" : `whole-pack run got to policy ${progress.started} of ${progress.of} in ${(progress.wall_ms / 1000).toFixed(0)} s, ${(progress.rss_kb / 1048576).toFixed(1)} GiB peak RSS`,
    cell.candidate === "rr" && cell.completion !== null && grouping === null ? "combined or alone: not reported by this engine" : "",
    chunked === null ? "" : `chunked: figures are the sums of ${chunked.chunks} chunks of up to ${chunked.size} rules (${chunked.rules_run} of ${chunked.rules_of} rules ran)`,
    ...(chunked?.failed_chunks.map((f) => `chunk ${f.chunk} (${f.rules}) failed: ${f.reason}`) ?? []),
    grouping === null ? "" : grouping.combined > 0 ? `ran combined: ${grouping.combined} of ${grouping.policies} policies in groups (largest ${grouping.largest}), ${grouping.alone} alone` : `ran alone: all ${grouping.policies} policies one by one`,
    ...cell.fallback.map((line) => `engine: ${line}`),
    capsVary ? `caps ${cell.caps.memory.join("/")}, ${cell.caps.time_s.join("/")} s` : "",
  ].filter((note) => note !== "");
}

function completionText(cell: CellItem, capsVary: boolean): string {
  let text: string;
  if (cell.completion === null) text = cell.reason ?? "–";
  else if (cell.completion.status === "fail") text = `**no**: ${cell.completion.reason}`;
  else text = cell.completion.kind === "both-failed" ? `neither side: ${cell.completion.reason}` : "both";
  return text + cellNotes(cell, capsVary).map((note) => `; ${note.replaceAll("|", "\\|")}`).join("");
}

function cellMetric(cell: CellItem | undefined, metric: string): string {
  if (cell === undefined) return "–";
  if (cell.reason !== null && cell.metrics.length === 0) return cell.status === "fail" ? `**${cell.reason}**` : cell.reason;
  const m = cell.metrics.find((x) => x.metric === metric);
  return m === undefined ? "–" : `${cell.chunked === null || m.status === "not_compared" ? "" : "chunked "}${metricText(m)}`;
}

function timeText(t: TimeItem | undefined): string {
  if (t === undefined || t.status === "not_compared") return "–";
  const confirm = t.confirm === null ? "" : `; confirmation ${times(t.confirm.ratio)}`;
  const floor = t.time_status === "under_floor" ? " (under the floor)" : "";
  return `${figure("ms", t.baseline_ms)} → ${figure("ms", t.candidate_ms)} ${markedRatio(t.ratio, t.status)}${floor}${confirm}`;
}

function ruleRow(r: RuleItem): string {
  const share = r.share === null ? "" : ` (${r.share < 0 ? "−" : ""}${Math.abs(Math.round(r.share * 100))}%)`;
  const findings = r.findings === null ? "–" : r.findings.status === "pass" ? "same" : `**${r.findings.baseline_only} / ${r.findings.candidate_only} differ**`;
  return `| \`${r.rule}\` | ${timeText(r.time)} | ${r.added_ms === null ? "a side failed" : `${signedMs(r.added_ms)}${share}`} | ${statusWord(r.status)} | ${findings} | ${(r.locator ?? (r.completion.status === "fail" ? r.completion.reason : "")).replaceAll("|", "\\|")} | ${multipleColumn(r)} |`;
}

function statsText(s: RuleStatsItem): string {
  return `ratio median ${times(s.median_ratio)}, p95 ${times(s.p95_ratio)}; over the bar before confirmation and the floor: ${s.over_bar_before.length}; counted (still over after confirmation, moved at least the floor): ${s.failing.length > 0 ? `**${s.failing.length}**` : "0"}; over 1.5× (moved at least the floor): ${s.over_alert.length}`;
}

/** Slice B's fixed cost table: the empty rule set's cold wall and phases, the phase that moved, whether it completed. */
function fixedCostRows(fixed: OneRuleItem["fixed"]): string[] {
  const time = (name: string): TimeItem | undefined => fixed?.times.find((t) => t.name === name);
  return [
    "| Fixed cost (empty rule set) | Baseline → candidate |",
    "|---|---|",
    ...["wall-cold", "load", "compile", "evaluate"].map((name) => `| ${name === "wall-cold" ? "wall cold" : name} | ${timeText(time(name))} |`),
    ...(fixed?.locator == null ? [] : [`| phase that moved | ${fixed.locator} |`]),
    ...(fixed !== null && fixed.completion.status === "fail" ? [`| completed | **no**: ${fixed.completion.reason.replaceAll("|", "\\|")} |`] : []),
  ];
}

/** Slice B's figures on one corpus: the rule statistics, the findings and the load, then the worst rules. */
function oneRuleFigures(b: OneRuleItem): string[] {
  const worst = b.stats.worst.map((w) => `\`${w.rule}\` ${formatMs(w.baseline_ms)} → ${formatMs(w.candidate_ms)} ${times(w.ratio)}`).join(", ");
  return [
    `Rules: ${statsText(b.stats)}. Findings: ${b.findings.identical} identical, ${b.findings.differing > 0 ? `**${b.findings.differing} differ**` : "0 differ"}${b.findings.not_compared > 0 ? `, ${b.findings.not_compared} not compared` : ""}. Load (1-minute) median ${b.load.load_median ?? "–"}, max ${b.load.load_max ?? "–"} over ${b.load.runs} runs; busiest idle sibling CPU ${b.load.sibling_busy_max === null ? "– (not pinned)" : percent(b.load.sibling_busy_max)}.`,
    "",
    `${TOP_RULES} worst rules: ${worst || "–"}.`,
    "",
  ];
}

/** Slice B's rule tables, each behind <details>: the rules over the bar before confirmation and the floor, then all rules. */
function ruleTables(b: OneRuleItem): string[] {
  const over = b.rules.filter((r) => r.time.over_before);
  const header = [`| Rule | ${b.rule_time}, baseline → candidate | Added | Status | Findings | Phase that moved | Multiple of the median rule |`, "|---|---|---|---|---|---|---|"];
  const lines: string[] = [];
  if (over.length > 0) lines.push(`<details${b.status === "fail" ? " open" : ""}><summary>${over.length} rule${over.length === 1 ? "" : "s"} over the bar before confirmation and the floor</summary>`, "", ...header, ...over.map(ruleRow), "", "</details>", "");
  if (b.rules.length > 0) lines.push(`<details><summary>All ${b.rules.length} rules, the largest added time first</summary>`, "", ...header, ...b.rules.map(ruleRow), "", "</details>", "");
  return lines;
}

/** Slice B on one corpus: the fixed cost, the rule statistics, the worst rules, every rule over the bar, all rules. */
function oneRuleSection(b: OneRuleItem): string[] {
  const label = `Slice B, ${CORPUS_LABEL[b.corpus].toLowerCase()}`;
  if (b.reason !== null) return [`${label}: ${b.reason}.`, ""];
  const wallCold = b.fixed?.times.find((t) => t.name === "wall-cold");
  return [
    `<details${b.status === "fail" ? " open" : ""}><summary>${label}: ${statusWord(b.status)} · ${b.rules.length} rules · fixed cost ${times(wallCold?.ratio ?? null)} · ${b.stats.failing.length} counted over the bar${b.hint === null ? "" : ` · ${b.hint}`}</summary>`,
    "",
    `Each rule alone and the empty rule set, ${b.runs_per_side} cold run${b.runs_per_side === 1 ? "" : "s"} per side (median), rotating, a fresh store each; a rule's time is ${b.rule_time}.${b.unmeasured.length > 0 ? ` **Not timed on both sides: ${b.unmeasured.map((r) => `\`${r}\``).join(", ")}.**` : ""}`,
    "",
    ...fixedCostRows(b.fixed),
    "",
    ...oneRuleFigures(b),
    ...ruleTables(b),
    "</details>",
    "",
  ];
}

/** Slice C: the three files side by side, one column each. */
function oneFileSection(files: readonly OneFileItem[]): string[] {
  if (files.length === 0) return [];
  const head = ["Metric", ...files.map((f) => `${f.label}: \`${f.path}\` (${(f.bytes / 1024).toFixed(1)} KB)`)];
  const metricCell = (f: OneFileItem, metric: string): string => {
    if (f.reason !== null) return f.reason;
    const m = f.metrics.find((x) => x.metric === metric);
    return m === undefined ? "–" : `${metricText(m)}${m.locator === null || m.status !== "fail" ? "" : ` — ${m.locator}`}`;
  };
  const findings = (f: OneFileItem): string => (f.findings === null ? "–" : f.findings.status === "pass" ? `same (${f.findings.both})` : `**${f.findings.baseline_only} baseline only, ${f.findings.candidate_only} candidate only**`);
  const completed = (f: OneFileItem): string => (f.completion === null ? (f.reason ?? "–") : f.completion.status === "fail" ? `**no**: ${f.completion.reason}` : f.completion.kind === "both-failed" ? `neither side: ${f.completion.reason}` : "both");
  const load = (f: OneFileItem): string => `${[...f.load.baseline, ...f.load.candidate].map((l) => l ?? "–").join(", ") || "–"}; sibling ${f.sibling_busy_max === null ? "–" : percent(f.sibling_busy_max)}`;
  const row = (label: string, cell: (f: OneFileItem) => string): string => `| ${label} | ${files.map((f) => cell(f).replaceAll("|", "\\|")).join(" | ")} |`;
  return [
    `<details${files.some((f) => f.status === "fail") ? " open" : ""}><summary>Slice C: ${files.map((f) => `${f.label} ${statusWord(f.status)}`).join(" · ")}</summary>`,
    "",
    "All of the pack's rules on one file of its real-code corpus, cold then warm on the same store, alternating.",
    "",
    `| ${head.join(" | ")} |`,
    `|${head.map(() => "---").join("|")}|`,
    ...["wall-cold", "wall-warm", "cpu-cold", "cpu-warm", "rss-cold", "rss-warm"].map((metric) => row(metricLabel(metric), (f) => metricCell(f, metric))),
    row("Findings", findings),
    row("Completed", completed),
    row("Rules failing on the candidate only", (f) => (f.rule_failures.length === 0 ? "–" : f.rule_failures.map((r) => `\`${r.rule}\`: ${r.reason}`).join("; "))),
    row("Load at each run; busiest sibling", load),
    "",
    "</details>",
    "",
  ];
}

function rawSection(cell: CellItem): string[] {
  const runs = (values: readonly number[], unit: MetricItem["unit"]): string => (values.length === 0 ? "–" : values.map((v) => figure(unit, v)).join(", "));
  const lines = [
    `<details><summary>Reproduce and raw runs, ${CORPUS_LABEL[cell.corpus].toLowerCase()}</summary>`,
    "",
    "```",
    cell.reproduce,
    "```",
    "",
    "| Metric | Baseline runs | Candidate runs | Spread (baseline / candidate) | Confirmation runs (baseline; candidate) |",
    "|---|---|---|---|---|",
    ...cell.metrics.map((m) => {
      const spread = `${m.baseline.spread_pct ?? "–"}% / ${m.candidate.spread_pct ?? "–"}%`;
      const confirm = m.confirm === null ? "–" : `${runs(m.confirm.baseline.runs, m.unit)}; ${runs(m.confirm.candidate.runs, m.unit)}`;
      return `| ${metricLabel(m.metric)} | ${runs(m.baseline.runs, m.unit)} | ${runs(m.candidate.runs, m.unit)} | ${spread} | ${confirm} |`;
    }),
    "",
    `Load (1-minute) at each run: baseline ${cell.load.baseline.map((l) => l ?? "–").join(", ") || "–"}; candidate ${cell.load.candidate.map((l) => l ?? "–").join(", ") || "–"}. Busiest idle sibling CPU: ${cell.sibling_busy_max === null ? "– (not pinned)" : `${Math.round(cell.sibling_busy_max * 100)}%`}.`,
    ...(cell.method_gaps.length === 0 ? [] : ["", `Method gaps of these runs: ${cell.method_gaps.join("; ")}.`]),
  ];
  const samples = [...(cell.findings?.samples ?? []), ...(cell.agreement?.samples ?? [])];
  if (samples.length > 0) lines.push("", "Differing findings, samples:", "", ...samples.map((s) => `- \`${s}\``));
  if (cell.rule_failures.length > 0) lines.push("", "Rules that fail on the candidate only:", "", ...cell.rule_failures.map((f) => `- \`${f.rule}\`: ${f.reason}`));
  lines.push("", "</details>", "");
  return lines;
}

/** A pack's <summary>: its status, each slice's, the wall-cold ratio per corpus, the findings, whether it finished. */
function packSummaryText(p: PackItem, corpora: readonly ("fixtures" | "real")[]): string {
  const glance = corpora
    .map((corpus) => {
      const entry = p.summary.wall_cold[corpus];
      return entry === undefined || entry.ratio === null ? null : `${corpus} ${times(entry.ratio)}`;
    })
    .filter((x) => x !== null);
  const findings = { same: "findings same", no_worse: "findings no worse", worse: "**findings worse**", not_compared: "findings not compared" }[p.summary.findings];
  const slices = (["A", "B", "C"] as const).map((slice) => `${slice} ${statusWord(p.summary.slices[slice].status)}`);
  return [`<b>${p.pack}: ${statusWord(p.status).replaceAll("*", "")}${p.counted ? "" : " (reported only)"}</b>`, ...slices, ...glance, findings, p.summary.completed ? "completed" : "**did not finish**"].join(" · ");
}

/** Why a pack counts toward the exit status or is reported only, and the slices it lacks. */
function packNotes(p: PackItem): string[] {
  const lines: string[] = [];
  if (p.gate_reason !== null) lines.push(`${p.counted ? "Counts toward the exit status" : "**Reported only**: never fails the run"}; ${p.gate_reason}.`, "");
  if (p.incomplete.length > 0) lines.push("**Incomplete**: it never passes while a slice is missing.", "", ...p.incomplete.map((m) => `- slice ${m.slice}: ${m.reason}`), "");
  return lines;
}

/** Slice A's table: each metric, the findings, the completion and the rules run per corpus, then the cost rows. */
function sliceATable(p: PackItem, corpora: readonly ("fixtures" | "real")[], capsVary: boolean): string[] {
  const cell = (corpus: "fixtures" | "real"): CellItem | undefined => p.cells.find((c) => c.corpus === corpus);
  const head = ["Metric", ...corpora.map((corpus) => CORPUS_LABEL[corpus])];
  const lines = ["Slice A, all rules × all files:", "", `| ${head.join(" | ")} |`, `|${head.map(() => "---").join("|")}|`];
  for (const metric of ["wall-cold", "wall-warm", "cpu-cold", "cpu-warm", "rss-cold", "rss-warm"]) lines.push(`| ${metricLabel(metric)} | ${corpora.map((corpus) => cellMetric(cell(corpus), metric)).join(" | ")} |`);
  lines.push(`| Findings | ${corpora.map((corpus) => findingsText(cell(corpus)!)).join(" | ")} |`);
  lines.push(`| Completed | ${corpora.map((corpus) => completionText(cell(corpus)!, capsVary)).join(" | ")} |`);
  lines.push(`| Rules run (baseline / candidate) | ${corpora.map((corpus) => `${cell(corpus)!.rules.baseline} / ${cell(corpus)!.rules.candidate}`).join(" | ")} |`);
  lines.push(...costRows(corpora.map(cell)), "", COST_NOTE, "");
  return lines;
}

/** A pack's coverage: the rules the baseline ships that cannot run, and the rules only the candidate ships. */
function coverageLines(p: PackItem): string[] {
  const lines: string[] = [];
  if (p.coverage.missing.length > 0) {
    lines.push(`<details open><summary><b>${p.coverage.missing.length} rule${p.coverage.missing.length === 1 ? "" : "s"} the baseline ships cannot run</b></summary>`, "", ...p.coverage.missing.map((m) => `- \`${m.rule}\`: ${m.reason}`), "", "</details>", "");
  }
  if (p.coverage.extra.length > 0) lines.push(`Rules only the candidate ships: ${p.coverage.extra.map((r) => `\`${r}\``).join(", ")}.`, "");
  return lines;
}

function packSection(p: PackItem, capsVary: boolean): string[] {
  const corpora = (["fixtures", "real"] as const).filter((corpus) => p.cells.some((cell) => cell.corpus === corpus));
  const open = p.status === "fail" || p.status === "incomplete";
  const lines = [`<a id="${anchor(p)}"></a>`, `<details${open ? " open" : ""}><summary>${packSummaryText(p, corpora)}</summary>`, ""];
  if (p.status === "no_baseline") return [...lines, "The baseline ships no such pack.", "", "</details>", ""];
  lines.push(...packNotes(p), ...sliceATable(p, corpora, capsVary), ...coverageLines(p));
  for (const b of p.one_rule) lines.push(...oneRuleSection(b));
  lines.push(...oneFileSection(p.one_file), ...p.cells.filter((c) => c.metrics.length > 0).flatMap(rawSection), "</details>", "");
  return lines;
}

function itemsTable(title: string, items: CandidateItem["failures"], open: boolean): string[] {
  if (items.length === 0) return [];
  return [
    `<details${open ? " open" : ""}><summary>${title}</summary>`,
    "",
    "| Item | Slice | Reason | Phase that moved | Biggest contributor |",
    "|---|---|---|---|---|",
    ...items.map((f) => `| \`${f.id}\`${f.counted ? "" : " (reported only)"} | ${f.slice ?? "–"} | ${f.reason.replaceAll("|", "\\|")} | ${(f.locator ?? "–").replaceAll("|", "\\|")} | ${(f.hint ?? "").replaceAll("|", "\\|")} |`),
    "",
    "</details>",
    "",
  ];
}

function candidateSection(c: CandidateItem, capsVary: boolean): string[] {
  const packs = sortedPacks(c.packs);
  const lines = [
    `## ${c.candidate}: ${c.status}${countedText(c)}`,
    "",
    `${c.description.charAt(0).toUpperCase()}${c.description.slice(1)}. ${c.counts.failing_packs} of ${c.counts.packs} packs fail${c.counts.incomplete_packs > 0 ? `; ${c.counts.incomplete_packs} incomplete (a slice missing)` : ""}${c.counts.noisy_packs > 0 ? `; ${c.counts.noisy_packs} pass only after confirmation (≈)` : ""}.${reportedText(c)}`,
    "",
    "| Pack | Status | A fixtures, wall cold | A real code, wall cold | B rules | B fixed cost, wall cold (fixtures · real) | B rule ratio, median · p95 | B over 1.15×, counted / before filter | B over 1.5× | C largest | C median | C small | Findings A · B · C | Load max · busiest sibling | Biggest contributor |",
    "|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|",
    ...packs.map(packRow),
    "",
    `Bold: fails the bar (each failing figure shows its confirmation ratio when confirmation runs were made). ≈: failed at the median and passed its confirmation runs; reported, not counted. B: each rule alone, its ${c.packs.find((p) => p.one_rule.length > 0)?.one_rule[0]!.rule_time ?? "compile + evaluate"} time; counted over the bar only when still over after confirmation and moved by at least 20 ms. C: each file's worst metric ratio. Load: the highest 1-minute load at any run's start, and the busiest idle sibling CPU, over all three slices.`,
    "",
    ...itemsTable(`All ${c.failures.length} failing items`, c.failures, false),
    ...itemsTable(`${c.noisy.length} noisy items: failed at the median, passed confirmation`, c.noisy, false),
  ];
  for (const p of packs.filter((p) => p.status === "fail" || p.status === "incomplete" || p.status === "noisy")) lines.push(...packSection(p, capsVary));
  const rest = packs.filter((p) => p.status !== "fail" && p.status !== "incomplete" && p.status !== "noisy");
  if (rest.length > 0) lines.push(`<details><summary>${rest.length} other packs</summary>`, "", ...rest.flatMap((p) => packSection(p, capsVary)), "</details>", "");
  return lines;
}

/** A candidate whose verdict on some packs is reported only: the status the exit status reads, beside its own. */
function countedText(c: CandidateItem): string {
  return c.counts.reported_packs === 0 ? "" : ` (counted: ${c.counted_status ?? "none"}; ${c.counts.reported_packs} pack${c.counts.reported_packs === 1 ? "" : "s"} reported only)`;
}

/** A candidate's packs whose verdict is reported only, and what the exit status reads without them; "" where every pack counts. */
function reportedText(c: CandidateItem): string {
  if (c.counts.reported_packs === 0) return "";
  return ` ${c.counts.reported_packs} pack${c.counts.reported_packs === 1 ? " is" : "s are"} reported only (each says why): ${c.counted_status === null ? "nothing of this candidate counts toward the exit status" : `the exit status reads ${c.counted_status}, over the other packs`}.`;
}

/** The headline's pointer at a failing candidate's worst pack: its failing metric when one fails, and its hint. */
function worstText(worst: PackItem): string {
  const hint = worst.summary.hint === null ? "" : ` (${worst.summary.hint})`;
  const metric = worst.summary.worst;
  if (metric !== null && metric.status === "fail") return ` · worst: ${worst.pack} ${metric.corpus} ${metricLabel(metric.metric)} ${times(metric.ratio)}${hint}`;
  return ` · worst: ${worst.pack}${hint}`;
}

function headline(c: CandidateItem): string {
  const worst = sortedPacks(c.packs)[0];
  const lead = c.status !== "FAIL" || worst === undefined ? "" : worstText(worst);
  return `- **${c.candidate}: ${c.status}**${countedText(c)} · ${c.counts.failing_packs} of ${c.counts.packs} packs fail${c.counts.incomplete_packs > 0 ? `, ${c.counts.incomplete_packs} incomplete` : ""}${lead}`;
}

function runSection(report: Report, baseline: string): string[] {
  const short = (sha: string): string => sha.slice(0, 10);
  const lines = [
    "<details><summary>Run, inputs and method</summary>",
    "",
    `- Started ${report.run.started_at ?? "–"}, reported ${report.run.finished_at}, by ${report.run.invoked_by ?? "an unknown caller"}.`,
    `- Command: \`${report.run.command ?? "–"}\`; harness at \`${short(report.run.harness_sha)}\`.`,
    `- Baseline ${baseline}; candidate \`${short(report.candidate.sha)}\`.`,
    `- Bar: per slice, every figure at most ${report.bar.max_ratio.toFixed(2)}× the baseline's. A: ${report.bar.runs_per_side === 1 ? "each metric's single run per side" : `each metric's median of ${report.bar.runs_per_side} runs per side`}. B: each rule alone and the empty rule set, ${report.bar.rule_runs > 0 ? `${report.bar.rule_runs} cold runs per side` : "its recorded cold runs"}; a rule counts only when still over after confirmation and moved by at least ${report.bar.rule_floor_ms} ms; over ${report.bar.rule_alert_ratio.toFixed(1)}× is counted apart. C: ${report.bar.file_runs > 0 ? `${report.bar.file_runs} runs per side` : "its recorded runs"} of each file, cold then warm. ${report.bar.confirm_runs > 0 ? `Anything over runs ${report.bar.confirm_runs} confirmation repetition${report.bar.confirm_runs === 1 ? "" : "s"} per side` : "No confirmation runs were recorded"}.`,
    `- CPUs ${report.bar.pinned_cpus.join(" / ") || "–"} (${report.bar.threads.join(", ") || "–"} threads, one per physical core, siblings idle where the platform pins); caps ${report.bar.caps.memory.join(", ") || "–"} memory, ${report.bar.caps.time_s.join(", ") || "–"} s.`,
    "",
  ];
  if (report.machines.length > 0) {
    lines.push("| Host | Instance | CPU | Logical CPUs | OS | Kernel |", "|---|---|---|---|---|---|", ...report.machines.map((m) => `| ${m.host} | ${m.instance_type ?? "–"} | ${m.cpu_model ?? "–"} | ${m.logical_cpus} | ${m.os ?? "–"} | ${m.kernel} |`), "");
  }
  if (report.bar.methods.length > 0) {
    lines.push(
      "| Platform | Runs | Pinning | Measured by | Peak memory | Memory cap | Time cap |",
      "|---|---|---|---|---|---|---|",
      ...report.bar.methods.map(({ method: m, runs }) => `| ${m.platform} | ${runs} | ${m.pinning} | ${m.measured} | ${m.peak} | ${m.memoryCap} | ${m.timeCap} |`),
      "",
      ...report.bar.methods.flatMap(({ method, runs, gaps }) => gaps.map((gap) => `- ${method.platform}, ${runs} runs: ${gap}.`)),
      "",
    );
  }
  lines.push(
    "Inputs, pinned by hash:",
    "",
    ...Object.entries(report.inputs.fixtures).map(([pack, f]) => `- fixtures ${pack}: ${"absent" in f ? f.absent : `${f.files} files, \`${f.sha256.slice(0, 12)}\``}`),
    ...Object.entries(report.inputs.real).map(([name, r]) => `- real code ${name}: ${r.files} files, \`${r.sha256.slice(0, 12)}\``),
    "",
    `Measured: ${report.scope.measured.join("; ")}.`,
    "",
    `Not measured by this harness yet: ${report.scope.not_measured.join("; ")}.`,
    "",
    `Known gaps of the reported (not judged) figures: ${report.scope.known_gaps.join("; ")}.`,
    "",
    "</details>",
    "",
  );
  return lines;
}

// ---------------------------------------------------------------- waits

function waitFigure(m: WaitMetricItem, value: number | null): string {
  if (value === null) return "–";
  return m.unit === "rate" ? `${(value * 100).toFixed(1)}%` : formatMs(value);
}

function ciText(c: WaitMetricItem["first"]): string {
  return c.ci === null ? "" : ` (CI ${c.ci.lo.toFixed(2)}–${c.ci.hi.toFixed(2)})`;
}

/** The scenario's figure for the table: the median, or p50 / p95 of a distribution, and its worst counted ratio. */
function waitRow(item: WaitItem): string {
  const metric = (name: string): WaitMetricItem | undefined => item.metrics.find((m) => m.metric === name);
  const shown = item.kind === "single" ? [metric("median")] : [metric("p50"), metric("p95")];
  const side = (pick: (m: WaitMetricItem) => number | null): string => shown.map((m) => (m === undefined ? "–" : waitFigure(m, pick(m)))).join(" / ");
  const counted = item.metrics.filter((m) => m.gated && m.first.ratio !== null);
  const worst = counted.sort((a, b) => (b.status === "fail" ? 1 : 0) - (a.status === "fail" ? 1 : 0) || (b.first.ratio ?? 0) - (a.first.ratio ?? 0))[0];
  const ratio = worst === undefined ? "–" : `${worst.metric === "median" ? "" : `${worst.metric} `}${markedRatio(worst.first.ratio, worst.status)}${ciText(worst.first)}`;
  const limits = item.metrics.filter((m) => m.limit !== null).map((m) => (m.limit!.value === null ? `${m.limit!.name}: not set` : `${m.limit!.name} ${formatMs(m.limit!.value)}: ${m.limit!.candidate_within === false ? "**over**" : "within"}`));
  const status = `${statusWord(item.status)}${item.gated ? "" : " (report only)"}`;
  return `| [\`${item.id}\`](#${item.id.replaceAll("/", "-")}) | ${item.title} | ${status} | ${side((m) => m.first.baseline)} | ${side((m) => m.first.candidate)} | ${ratio} | ${limits.join("; ") || "–"} | ${(item.hint ?? "").replaceAll("|", "\\|")} |`;
}

function waitDetails(item: WaitItem): string[] {
  const notes = (["baseline", "candidate"] as const).flatMap((side) =>
    Object.entries(item.notes[side]).map(([key, values]) => `- ${side} ${key.replaceAll("_", " ")}: ${values.map(String).join(", ")}`),
  );
  const failed = item.runs.failed.map((f) => `- ${f.side} run ${f.rep} failed: ${f.reason}`);
  return [
    `<a id="${item.id.replaceAll("/", "-")}"></a>`,
    `**\`${item.id}\`: ${item.title}** · ${statusWord(item.status)}${item.gated ? "" : " (report only)"}`,
    "",
    `Entry point: ${item.entry}. Store: ${item.store}. Runs per side: ${item.runs.baseline} baseline, ${item.runs.candidate} candidate${item.runs.confirm > 0 ? `; ${item.runs.confirm} confirmation runs` : ""}. Page cache dropped: ${item.cache_dropped === null ? "not applicable (warm)" : item.cache_dropped ? "yes" : "**no** (no passwordless sudo)"}.`,
    "",
    "| Metric | Baseline | Candidate | Ratio | 95% CI | Events or runs | Status | Confirmation | Limit |",
    "|---|---|---|---|---|---|---|---|---|",
    ...item.metrics.map((m) => {
      const c = m.first;
      const confirm = m.confirm === null ? "–" : `${waitFigure(m, m.confirm.baseline)} → ${waitFigure(m, m.confirm.candidate)} ${times(m.confirm.ratio)}${ciText(m.confirm)}`;
      const limit = m.limit === null ? "–" : m.limit.value === null ? `${m.limit.name}: not set` : `${formatMs(m.limit.value)}: baseline ${m.limit.baseline_within ? "within" : "over"}, candidate ${m.limit.candidate_within ? "within" : "**over**"}`;
      const status = m.gated ? statusWord(m.status) : `${statusWord(m.status)} (reported)`;
      return `| ${m.metric} | ${waitFigure(m, c.baseline)} | ${waitFigure(m, c.candidate)} | ${markedRatio(c.ratio, m.gated ? m.status : "pass")} | ${c.ci === null ? "–" : `${c.ci.lo.toFixed(2)}–${c.ci.hi.toFixed(2)}${c.within_spread ? " (holds the bar)" : ""}`} | ${c.n.baseline} / ${c.n.candidate} | ${status} | ${confirm} | ${limit} |`;
    }),
    "",
    ...failed,
    ...notes,
    `- load (1 min) baseline ${item.load.baseline.join(", ") || "–"}; candidate ${item.load.candidate.join(", ") || "–"}; busiest sibling thread ${(item.sibling_busy_max * 100).toFixed(0)}%`,
    `- reproduce: \`${item.reproduce}\``,
    "",
  ];
}

function waitsHeadline(w: WaitsSection): string {
  const failing = w.items.filter((i) => i.gated && i.status === "fail");
  const lead = failing.length === 0 ? "" : ` · ${failing.map((i) => `${i.id.replace(/^wait\//, "")}${i.hint === null ? "" : ` (${i.hint})`}`).join("; ")}`;
  return `- **waits: ${w.status}** · ${w.counts.failing} of ${w.items.filter((i) => i.gated).length} scenarios fail${lead}`;
}

/** A note's values on one side as one figure: the median of its numbers, else its values joined. */
function noteFigure(item: WaitItem | undefined, side: "baseline" | "candidate", key: string): string {
  const values = item?.notes[side][key] ?? [];
  const numbers = values.filter((v): v is number => typeof v === "number").sort((a, b) => a - b);
  if (numbers.length > 0) return String(Math.round(numbers[(numbers.length - 1) >> 1]!));
  return values.length > 0 ? values.map(String).join(", ") : "–";
}

/**
 * One shape's steps as the step benchmarks print them, one row per step: the scan after it (W5) and the verdict on its
 * edit (W1), each side's median, and the scan's findings and daemon CPU (medians; reported).
 */
function stepsTable(shape: string, items: readonly WaitItem[]): string[] {
  const figure = (item: WaitItem | undefined, side: "baseline" | "candidate"): string => {
    const value = item?.metrics[0]?.first[side];
    return value == null ? "–" : formatMs(value);
  };
  const ratio = (item: WaitItem | undefined): string => (item === undefined ? "–" : markedRatio(item.metrics[0]?.first.ratio ?? null, item.status));
  const steps = [...new Set(items.map((i) => i.step!))];
  return [
    `**Step benchmark, ${shape}**`,
    "",
    "| Step | Scan baseline | Scan candidate | Ratio | Verdict baseline | Verdict candidate | Ratio | Findings | Daemon CPU per scan |",
    "|---|---|---|---|---|---|---|---|---|",
    ...steps.map((step) => {
      const scan = items.find((i) => i.step === step && i.wait === "W5");
      const verdict = items.find((i) => i.step === step && i.wait === "W1");
      const pair = (key: string, unit = ""): string => (["baseline", "candidate"] as const).map((side) => noteFigure(scan, side, key)).map((v) => (v === "–" ? v : `${v}${unit}`)).join(" / ");
      return `| ${step} | ${figure(scan, "baseline")} | ${figure(scan, "candidate")} | ${ratio(scan)} | ${figure(verdict, "baseline")} | ${figure(verdict, "candidate")} | ${ratio(verdict)} | ${pair("findings")} | ${pair("daemon_cpu_ms", " ms")} |`;
    }),
    "",
  ];
}

function waitsSection(w: WaitsSection): string[] {
  const plain = w.items.filter((i) => i.shape === null);
  const shapes = [...new Set(w.items.map((i) => i.shape).filter((s) => s !== null))];
  const lines = [
    `## User waits: ${w.status}`,
    "",
    `${w.counts.failing} of ${w.items.filter((i) => i.gated).length} counted scenarios fail${w.counts.noisy > 0 ? `; ${w.counts.noisy} pass only after confirmation (≈)` : ""}. Single waits compare each side's median; distributions show p50 / p95, and fail only when the bootstrap 95% CI of the ratio lies above the bar.`,
    "",
    "| Scenario | Wait | Status | Baseline | Candidate | Ratio | Limit | Biggest contributor |",
    "|---|---|---|---|---|---|---|---|",
    ...plain.map(waitRow),
    "",
    ...(shapes.length > 0 ? ["The step benchmarks' edit sequence (cold, warm, then each edit over the ones before it), through the product on both sides: a repository scan after each step (W5) and the verdict on each edit made through the hooks (W1).", ""] : []),
    ...shapes.flatMap((shape) => stepsTable(shape, w.items.filter((i) => i.shape === shape))),
  ];
  if (w.not_measured.length > 0) lines.push(`<details><summary>${w.not_measured.length} waits not measured</summary>`, "", ...w.not_measured.map((n) => `- \`${n.id}\`: ${n.reason}`), "", "</details>", "");
  const groups = [
    ...[...new Set(plain.map((i) => i.wait))].map((wait) => ({ label: `${wait}: ${plain.find((i) => i.wait === wait)!.title.replace(/[,:].*$/, "")}`, items: plain.filter((i) => i.wait === wait) })),
    ...shapes.map((shape) => ({ label: `Step benchmark, ${shape}`, items: w.items.filter((i) => i.shape === shape) })),
  ];
  for (const { label, items } of groups) {
    const open = items.some((i) => i.gated && (i.status === "fail" || i.status === "noisy"));
    lines.push(`<details${open ? " open" : ""}><summary>${label} · ${items.map((i) => `${i.scenario} ${statusWord(i.status)}`).join(", ")}</summary>`, "", ...items.flatMap(waitDetails), "</details>", "");
  }
  const inputs = [
    ...Object.entries(w.inputs.repos).map(([name, r]) => `- repository ${name}: corpus ${r.corpus}, ${r.files} files, \`${r.sha256.slice(0, 12)}\`; packs ${r.packs.join(", ")}`),
    ...Object.entries(w.inputs.sessions).map(([name, s]) => `- session ${name}: ${s.edits} edits synthesized from ${s.source}, \`${s.sha256.slice(0, 12)}\``),
    `- limits: ${Object.entries(w.limits).map(([name, value]) => `${name} ${value === null ? "not set" : value}`).join("; ")}`,
    ...w.method.map((m) => `- ${m}`),
  ];
  lines.push("<details><summary>Waits: inputs and method</summary>", "", ...inputs, "", "</details>", "");
  return lines;
}

export function renderReport(report: Report): string {
  const sha = report.baseline.sha.slice(0, 10);
  // A ref that is the commit itself (a sha given as --baseline) is not repeated beside it.
  const ref = report.baseline.ref;
  const baseline = ref === null || report.baseline.sha.startsWith(ref) ? `\`${sha}\`` : `\`${ref}\` (\`${sha}\`)`;
  const lines = [
    `# Ship / no-ship: ${report.verdict.status}`,
    "",
    ...report.candidates.map(headline),
    ...(report.waits === null ? [] : [waitsHeadline(report.waits)]),
    "",
    `Candidate \`${report.candidate.sha.slice(0, 10)}\` against baseline ${baseline} · gate: ${[...report.verdict.gate, ...(report.waits === null ? [] : ["waits"])].join(", ")} · bar, per pack and slice (A all rules × all files, B each rule alone, C all rules × one file): every figure ≤ ${report.bar.max_ratio.toFixed(2)}× the baseline (a B rule counts when still over after confirmation and moved ≥ ${report.bar.rule_floor_ms} ms), findings the same, every run completes; a pack missing a slice is incomplete.`,
    "",
    ...(report.verdict.partial.length === 0 ? [] : [`**Partial run**: ${report.verdict.partial.join("; ")}. A partial verdict is never PASS.`, ""]),
    // Runs measured with less than Linux's pinning and kernel caps say so before any figure is read.
    ...report.bar.methods.filter((m) => m.gaps.length > 0).map((m) => `Measured on ${m.method.platform} (${m.runs} runs): ${m.gaps.map((gap) => gap.split(":")[0]).join("; ")}; details under "Run, inputs and method".`),
    ...(report.bar.methods.some((m) => m.gaps.length > 0) ? [""] : []),
  ];
  // Each cell names its caps only when packs ran under different ones (`--pack-cap`).
  const capsVary = report.bar.caps.memory.length > 1 || report.bar.caps.time_s.length > 1;
  for (const c of report.candidates) lines.push(...candidateSection(c, capsVary));
  if (report.waits !== null) lines.push(...waitsSection(report.waits));
  lines.push(...runSection(report, baseline));
  return `${lines.join("\n")}\n`;
}
