// report.md of the ship / no-ship report (report-model.ts), in levels a reader opens only as far as the question needs:
// the verdict on one line; per candidate one row per pack, naming its worst metric and the rule behind it; each pack's
// metrics by corpus behind <details>, open where the pack fails; inside, the rules a slowdown is attributed to, and the
// raw runs with the command that reproduces the cell. GitHub renders the <details>; every pack has an anchor.

import { type CandidateItem, type CellItem, formatMs, type ItemStatus, type MetricItem, metricLabel, type PackItem, type Report, signedMs } from "./report-model.js";

const CORPUS_LABEL = { fixtures: "Fixtures", real: "Real code" } as const;
const TOP_RULES = 5;

function figure(unit: MetricItem["unit"], value: number | null): string {
  if (value === null) return "–";
  return unit === "mb" ? `${Math.round(value)} MB` : formatMs(value);
}

function times(ratio: number | null): string {
  return ratio === null || !Number.isFinite(ratio) ? "–" : `${ratio.toFixed(2)}×`;
}

function statusWord(status: ItemStatus | "no_baseline"): string {
  return { fail: "**FAIL**", noisy: "pass ≈", pass: "pass", not_compared: "n/a", no_baseline: "no baseline" }[status];
}

/** A ratio as a table shows it: bold when it fails, ≈ when it failed only at the median (noise). */
function markedRatio(ratio: number | null, status: ItemStatus): string {
  const text = times(ratio);
  return status === "fail" ? `**${text}**` : status === "noisy" ? `${text} ≈` : text;
}

function anchor(p: PackItem): string {
  return `${p.candidate}-${p.pack}`;
}

function packRow(p: PackItem): string {
  const column = (corpus: "fixtures" | "real"): string => {
    const entry = p.summary.wall_cold[corpus];
    if (entry === undefined) return "–";
    if (!entry.completed) return "**did not finish**";
    return entry.ratio === null ? "n/a" : markedRatio(entry.ratio, entry.status);
  };
  const worst = p.summary.worst === null ? "–" : `${p.summary.worst.corpus} · ${metricLabel(p.summary.worst.metric)} ${markedRatio(p.summary.worst.ratio, p.summary.worst.status)}`;
  return `| [${p.pack}](#${anchor(p)}) | ${statusWord(p.status)} | ${column("fixtures")} | ${column("real")} | ${worst} | ${p.summary.hint ?? ""} |`;
}

const ORDER: Record<PackItem["status"], number> = { fail: 0, noisy: 1, not_compared: 2, pass: 3, no_baseline: 4 };

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

function completionText(cell: CellItem): string {
  if (cell.completion === null) return cell.reason ?? "–";
  if (cell.completion.status === "fail") return `**no**: ${cell.completion.reason}`;
  return cell.completion.kind === "both-failed" ? `neither side: ${cell.completion.reason}` : "both";
}

function cellMetric(cell: CellItem | undefined, metric: string): string {
  if (cell === undefined) return "–";
  if (cell.reason !== null && cell.metrics.length === 0) return cell.status === "fail" ? `**${cell.reason}**` : cell.reason;
  const m = cell.metrics.find((x) => x.metric === metric);
  return m === undefined ? "–" : metricText(m);
}

function drillSection(cell: CellItem): string[] {
  const drill = cell.drill;
  if (drill === null) return [];
  const measure = drill.measure === "evaluate_ms" ? "in-process evaluation time" : "process wall time";
  const row = (r: (typeof drill.rules)[number]): string => {
    const share = r.share === null ? "" : ` (${r.share < 0 ? "−" : ""}${Math.abs(Math.round(r.share * 100))}%)`;
    return `| \`${r.rule}\` | ${figure("ms", r.baseline_ms)} | ${figure("ms", r.candidate_ms)} | ${r.added_ms === null ? "a side failed" : `${signedMs(r.added_ms)}${share}`} |`;
  };
  const header = ["| Rule | Baseline | Candidate | Added |", "|---|---|---|---|"];
  const startup = drill.startup.baseline_ms === null && drill.startup.candidate_ms === null ? "" : ` Empty rule set: ${figure("ms", drill.startup.baseline_ms)} → ${figure("ms", drill.startup.candidate_ms)}.`;
  const lines = [
    `<details${cell.status === "fail" ? " open" : ""}><summary>Rules, ${CORPUS_LABEL[cell.corpus].toLowerCase()}: ${cell.hint ?? `${signedMs(drill.total_added_ms)} in all`}</summary>`,
    "",
    `Each rule alone, ${drill.reps} cold run${drill.reps === 1 ? "" : "s"} per side (median), ${measure}.${startup} Added in all: ${signedMs(drill.total_added_ms)}.`,
    "",
    ...header,
    ...drill.rules.slice(0, TOP_RULES).map(row),
    "",
  ];
  if (drill.rules.length > TOP_RULES) lines.push(`<details><summary>All ${drill.rules.length} rules</summary>`, "", ...header, ...drill.rules.map(row), "", "</details>", "");
  lines.push("</details>", "");
  return lines;
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
    `Load (1-minute) at each run: baseline ${cell.load.baseline.join(", ") || "–"}; candidate ${cell.load.candidate.join(", ") || "–"}. Busiest idle sibling CPU: ${Math.round(cell.sibling_busy_max * 100)}%.`,
  ];
  const samples = [...(cell.findings?.samples ?? []), ...(cell.agreement?.samples ?? [])];
  if (samples.length > 0) lines.push("", "Differing findings, samples:", "", ...samples.map((s) => `- \`${s}\``));
  if (cell.rule_failures.length > 0) lines.push("", "Rules that fail on the candidate only:", "", ...cell.rule_failures.map((f) => `- \`${f.rule}\`: ${f.reason}`));
  lines.push("", "</details>", "");
  return lines;
}

function packSection(p: PackItem): string[] {
  const corpora = (["fixtures", "real"] as const).filter((corpus) => p.cells.some((cell) => cell.corpus === corpus));
  const cell = (corpus: "fixtures" | "real"): CellItem | undefined => p.cells.find((c) => c.corpus === corpus);
  const glance = corpora
    .map((corpus) => {
      const entry = p.summary.wall_cold[corpus];
      return entry === undefined || entry.ratio === null ? null : `${corpus} ${times(entry.ratio)}`;
    })
    .filter((x) => x !== null);
  const findings = { same: "findings same", no_worse: "findings no worse", worse: "**findings worse**", not_compared: "findings not compared" }[p.summary.findings];
  const summary = [`<b>${p.pack}: ${p.status === "fail" ? "FAIL" : p.status === "noisy" ? "pass ≈" : p.status.replace("_", " ")}</b>`, ...glance, findings, p.summary.completed ? "completed" : "**did not finish**"].join(" · ");
  const lines = [`<a id="${anchor(p)}"></a>`, `<details${p.status === "fail" ? " open" : ""}><summary>${summary}</summary>`, ""];
  if (p.status === "no_baseline") return [...lines, "The baseline ships no such pack.", "", "</details>", ""];
  const head = ["Metric", ...corpora.map((corpus) => CORPUS_LABEL[corpus])];
  lines.push(`| ${head.join(" | ")} |`, `|${head.map(() => "---").join("|")}|`);
  for (const metric of ["wall-cold", "wall-warm", "cpu-cold", "cpu-warm", "rss-cold", "rss-warm"]) lines.push(`| ${metricLabel(metric)} | ${corpora.map((corpus) => cellMetric(cell(corpus), metric)).join(" | ")} |`);
  lines.push(`| Findings | ${corpora.map((corpus) => findingsText(cell(corpus)!)).join(" | ")} |`);
  lines.push(`| Completed | ${corpora.map((corpus) => completionText(cell(corpus)!)).join(" | ")} |`);
  lines.push(`| Rules run (baseline / candidate) | ${corpora.map((corpus) => `${cell(corpus)!.rules.baseline} / ${cell(corpus)!.rules.candidate}`).join(" | ")} |`, "");
  if (p.coverage.missing.length > 0) {
    lines.push(`<details open><summary><b>${p.coverage.missing.length} rule${p.coverage.missing.length === 1 ? "" : "s"} the baseline ships cannot run</b></summary>`, "", ...p.coverage.missing.map((m) => `- \`${m.rule}\`: ${m.reason}`), "", "</details>", "");
  }
  if (p.coverage.extra.length > 0) lines.push(`Rules only the candidate ships: ${p.coverage.extra.map((r) => `\`${r}\``).join(", ")}.`, "");
  for (const c of p.cells) lines.push(...drillSection(c));
  for (const c of p.cells) if (c.metrics.length > 0) lines.push(...rawSection(c));
  lines.push("</details>", "");
  return lines;
}

function itemsTable(title: string, items: CandidateItem["failures"], open: boolean): string[] {
  if (items.length === 0) return [];
  return [
    `<details${open ? " open" : ""}><summary>${title}</summary>`,
    "",
    "| Item | Reason | Biggest contributor |",
    "|---|---|---|",
    ...items.map((f) => `| \`${f.id}\` | ${f.reason.replaceAll("|", "\\|")} | ${(f.hint ?? "").replaceAll("|", "\\|")} |`),
    "",
    "</details>",
    "",
  ];
}

function candidateSection(c: CandidateItem): string[] {
  const packs = sortedPacks(c.packs);
  const lines = [
    `## ${c.candidate}: ${c.status}`,
    "",
    `${c.description.charAt(0).toUpperCase()}${c.description.slice(1)}. ${c.counts.failing_packs} of ${c.counts.packs} packs fail${c.counts.noisy_packs > 0 ? `; ${c.counts.noisy_packs} pass only after confirmation (≈)` : ""}.`,
    "",
    "| Pack | Status | Fixtures, wall cold | Real code, wall cold | Worst metric | Biggest contributor |",
    "|---|---|---|---|---|---|",
    ...packs.map(packRow),
    "",
    "Bold: fails the bar (each failing metric shows its confirmation ratio when confirmation runs were made). ≈: failed at the median and passed its confirmation runs; reported, not counted.",
    "",
    ...itemsTable(`All ${c.failures.length} failing items`, c.failures, false),
    ...itemsTable(`${c.noisy.length} noisy items: failed at the median, passed confirmation`, c.noisy, false),
  ];
  for (const p of packs.filter((p) => p.status === "fail" || p.status === "noisy")) lines.push(...packSection(p));
  const rest = packs.filter((p) => p.status !== "fail" && p.status !== "noisy");
  if (rest.length > 0) lines.push(`<details><summary>${rest.length} other packs</summary>`, "", ...rest.flatMap(packSection), "</details>", "");
  return lines;
}

function headline(c: CandidateItem): string {
  const worst = sortedPacks(c.packs)[0];
  const lead =
    c.status === "FAIL" && worst !== undefined && worst.summary.worst !== null
      ? ` · worst: ${worst.pack} ${worst.summary.worst.corpus} ${metricLabel(worst.summary.worst.metric)} ${times(worst.summary.worst.ratio)}${worst.summary.hint === null ? "" : ` (${worst.summary.hint})`}`
      : "";
  return `- **${c.candidate}: ${c.status}** · ${c.counts.failing_packs} of ${c.counts.packs} packs fail${lead}`;
}

function runSection(report: Report): string[] {
  const short = (sha: string): string => sha.slice(0, 10);
  const lines = [
    "<details><summary>Run, inputs and method</summary>",
    "",
    `- Started ${report.run.started_at ?? "–"}, reported ${report.run.finished_at}, by ${report.run.invoked_by ?? "an unknown caller"}.`,
    `- Command: \`${report.run.command ?? "–"}\`; harness at \`${short(report.run.harness_sha)}\`.`,
    `- Baseline \`${report.baseline.ref ?? short(report.baseline.sha)}\` (\`${short(report.baseline.sha)}\`); candidate \`${short(report.candidate.sha)}\`.`,
    `- Bar: each metric's median of ${report.bar.runs_per_side} runs at most ${report.bar.max_ratio.toFixed(2)}× the baseline's; ${report.bar.confirm_runs > 0 ? `a failing cell runs ${report.bar.confirm_runs} confirmation repetitions` : "no confirmation runs were recorded"}; ${report.bar.drill_reps > 0 ? `a confirmed slowdown is attributed to rules with ${report.bar.drill_reps} cold runs per rule and side` : "rule attribution, where shown, states its own runs"}.`,
    `- Pinned CPUs ${report.bar.pinned_cpus.join(" / ") || "–"} (${report.bar.threads.join(", ") || "–"} threads, one per physical core, siblings idle); caps ${report.bar.caps.memory.join(", ") || "–"} memory, ${report.bar.caps.time_s.join(", ") || "–"} s.`,
    "",
  ];
  if (report.machines.length > 0) {
    lines.push("| Host | Instance | CPU | Logical CPUs | OS | Kernel |", "|---|---|---|---|---|---|", ...report.machines.map((m) => `| ${m.host} | ${m.instance_type ?? "–"} | ${m.cpu_model ?? "–"} | ${m.logical_cpus} | ${m.os ?? "–"} | ${m.kernel} |`), "");
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
    "</details>",
    "",
  );
  return lines;
}

export function renderReport(report: Report): string {
  const lines = [
    `# Ship / no-ship: ${report.verdict.status}`,
    "",
    ...report.candidates.map(headline),
    "",
    `Candidate \`${report.candidate.sha.slice(0, 10)}\` against baseline \`${report.baseline.ref ?? report.baseline.sha.slice(0, 10)}\` (\`${report.baseline.sha.slice(0, 10)}\`) · gate: ${report.verdict.gate.join(", ")} · bar: every metric ≤ ${report.bar.max_ratio.toFixed(2)}× the baseline, findings no worse, every run completes.`,
    "",
  ];
  for (const c of report.candidates) lines.push(...candidateSection(c));
  lines.push(...runSection(report));
  return `${lines.join("\n")}\n`;
}
