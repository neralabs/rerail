// report.md of the ship / no-ship report (report-model.ts), in levels a reader opens only as far as the question needs:
// the verdict on one line; per candidate one row per pack, naming its worst metric and the rule behind it; each pack's
// metrics by corpus behind <details>, open where the pack fails; inside, the rules a slowdown is attributed to, and the
// raw runs with the command that reproduces the cell. GitHub renders the <details>; every pack has an anchor.

import { type CandidateItem, type CellItem, formatMs, type ItemStatus, type MetricItem, metricLabel, type PackItem, type Report, signedMs } from "./report-model.js";
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

function runSection(report: Report, baseline: string): string[] {
  const short = (sha: string): string => sha.slice(0, 10);
  const lines = [
    "<details><summary>Run, inputs and method</summary>",
    "",
    `- Started ${report.run.started_at ?? "–"}, reported ${report.run.finished_at}, by ${report.run.invoked_by ?? "an unknown caller"}.`,
    `- Command: \`${report.run.command ?? "–"}\`; harness at \`${short(report.run.harness_sha)}\`.`,
    `- Baseline ${baseline}; candidate \`${short(report.candidate.sha)}\`.`,
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
    `Candidate \`${report.candidate.sha.slice(0, 10)}\` against baseline ${baseline} · gate: ${[...report.verdict.gate, ...(report.waits === null ? [] : ["waits"])].join(", ")} · bar: every metric ≤ ${report.bar.max_ratio.toFixed(2)}× the baseline, findings no worse, every run completes.`,
    "",
  ];
  for (const c of report.candidates) lines.push(...candidateSection(c));
  if (report.waits !== null) lines.push(...waitsSection(report.waits));
  lines.push(...runSection(report, baseline));
  return `${lines.join("\n")}\n`;
}
