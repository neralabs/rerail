// report.md of the shippability bar (scripts/ship/shipbar.ts): per candidate its overall verdict, the failing items
// first, then every pack, corpus and metric, the run-to-run spread, and the real-code agreement.

import type { CandidateVerdict, Verdict } from "./shipbar.js";
import type { MetricVerdict } from "./verdict.js";

function figure(metric: string, value: number | null): string {
  if (value === null) return "–";
  if (metric.startsWith("rss")) return `${(value / 1024).toFixed(0)} MB`;
  return value >= 10_000 ? `${(value / 1000).toFixed(1)} s` : `${(value / 1000).toFixed(2)} s`;
}

function percent(value: number | null): string {
  return value === null ? "–" : `${(value * 100).toFixed(1)}%`;
}

function cellText(m: MetricVerdict): string {
  if (m.status === "not-compared") return `n/a (${m.reason})`;
  const mark = m.status === "pass" ? "" : "**FAIL** ";
  return `${mark}${figure(m.metric, m.main)} → ${figure(m.metric, m.candidate)} (${m.ratio === null ? "–" : `${m.ratio.toFixed(2)}x`})${m.withinSpread ? " ≈" : ""}`;
}

function candidateSection(verdict: Verdict, c: CandidateVerdict): string[] {
  const lines = [`## ${c.candidate}: ${c.status}`, "", `${c.description}.`, ""];
  if (c.failures.length > 0) {
    lines.push(`### Failing items (${c.failures.length})`, "", "| pack | corpus | item | reason |", "|---|---|---|---|");
    for (const f of c.failures) lines.push(`| ${f.pack} | ${f.corpus ?? "–"} | ${f.item} | ${f.reason.replaceAll("|", "\\|").replaceAll("\n", " ")} |`);
    lines.push("");
  }
  lines.push(
    "### Per pack and corpus",
    "",
    "Main → candidate (ratio); ≈ marks a ratio within the run-to-run spread of the 1.10 bar.",
    "",
    "| pack | corpus | status | completion | wall cold | wall warm | CPU cold | CPU warm | RSS cold | RSS warm | findings (main → candidate TP/FP/FN) |",
    "|---|---|---|---|---|---|---|---|---|---|---|",
  );
  for (const p of c.packs) {
    if (p.status === "no-baseline") {
      lines.push(`| ${p.pack} | – | no baseline | main ships no such pack | | | | | | | |`);
      continue;
    }
    for (const cell of p.cells) {
      const metrics = new Map(cell.metrics.map((m) => [m.metric, m]));
      const metricCell = (name: string): string => {
        const m = metrics.get(name);
        return m === undefined ? "–" : cellText(m);
      };
      const ran = cell.rules.candidate === cell.rules.main ? "" : ` (candidate ran ${cell.rules.candidate} of main's ${cell.rules.main} rules)`;
      const completion = (cell.completion === null ? cell.note : cell.completion.kind === "both-completed" ? "both completed" : `${cell.completion.kind}: ${cell.completion.reason}`) + (cell.completion === null ? "" : ran);
      let findings = "–";
      if (cell.findings !== null) {
        const f = cell.findings;
        findings = `${f.main.tp}/${f.main.fp}/${f.main.fn} → ${f.candidate.tp}/${f.candidate.fp}/${f.candidate.fn}${f.status === "fail" ? ` **FAIL** (${f.reason})` : ""}`;
      } else if (cell.agreement !== null) {
        findings = `agreement: ${cell.agreement.both} both, ${cell.agreement.mainOnly} main only, ${cell.agreement.candidateOnly} candidate only (not gating)`;
      }
      lines.push(
        `| ${p.pack} | ${cell.corpus} | ${cell.status} | ${completion.replaceAll("|", "\\|")} | ${["wall-cold", "wall-warm", "cpu-cold", "cpu-warm", "rss-cold", "rss-warm"].map(metricCell).join(" | ")} | ${findings} |`,
      );
    }
    if (p.missing.length > 0) lines.push(`| ${p.pack} | rules | fail | ${p.missing.length} rule(s) main ships do not run: ${p.missing.slice(0, 3).map((m) => `${m.rule} (${m.reason})`).join("; ").replaceAll("|", "\\|")}${p.missing.length > 3 ? "; …" : ""} | | | | | | | |`);
    if (p.extra.length > 0) lines.push(`| ${p.pack} | rules | info | the candidate ships ${p.extra.length} rule(s) main does not; they run in its timings | | | | | | | |`);
  }
  lines.push("");
  const spreads = c.packs.flatMap((p) => p.cells.flatMap((cell) => cell.metrics.filter((m) => m.status !== "not-compared").map((m) => ({ pack: p.pack, corpus: cell.corpus, m }))));
  if (spreads.length > 0) {
    const worst = [...spreads].sort((a, b) => Math.max(b.m.spreadMain ?? 0, b.m.spreadCandidate ?? 0) - Math.max(a.m.spreadMain ?? 0, a.m.spreadCandidate ?? 0));
    const median = (values: number[]): number => [...values].sort((a, b) => a - b)[values.length >> 1] ?? 0;
    lines.push(
      "### Run-to-run spread",
      "",
      `Spread is (max - min) / median of one side's ${verdict.runs} runs. Median spread over all compared metrics: main ${percent(median(spreads.map((s) => s.m.spreadMain ?? 0)))}, candidate ${percent(median(spreads.map((s) => s.m.spreadCandidate ?? 0)))}.`,
      "",
      "| pack | corpus | metric | main | candidate | spread main | spread candidate | ratio | within spread of the bar |",
      "|---|---|---|---|---|---|---|---|---|",
    );
    const listed = worst.filter((s) => s.m.withinSpread || (s.m.main !== null && !s.m.metric.startsWith("rss") && s.m.main < 1000));
    for (const { pack, corpus, m } of listed) {
      lines.push(`| ${pack} | ${corpus} | ${m.metric} | ${figure(m.metric, m.main)} | ${figure(m.metric, m.candidate)} | ${percent(m.spreadMain)} | ${percent(m.spreadCandidate)} | ${m.ratio?.toFixed(3) ?? "–"} | ${m.withinSpread ? "yes" : "no"} |`);
    }
    if (listed.length === 0) lines.push("| – | – | none | | | | | | |");
    lines.push("", "Listed: every metric whose ratio is within the spread of the bar, and every sub-second time.", "");
  }
  const real = c.packs.flatMap((p) => p.cells.filter((cell) => cell.agreement !== null && cell.agreement.samples.length > 0).map((cell) => ({ pack: p.pack, samples: cell.agreement!.samples })));
  if (real.length > 0) {
    lines.push("### Real-code disagreement samples (not gating)", "");
    for (const r of real) lines.push(`- ${r.pack}: ${r.samples.join("; ")}`);
    lines.push("");
  }
  return lines;
}

export function renderReport(verdict: Verdict): string {
  const lines = [
    "# Shippability bar",
    "",
    `Baseline main ${verdict.baseline.sha}; candidate ${verdict.candidate.sha}. Generated ${verdict.generated} by scripts/ship/shipbar.ts.`,
    "",
    ...verdict.candidates.map((c) => `- **${c.candidate}: ${c.status}** (${c.failures.length} failing items)${verdict.gate.includes(c.candidate) ? "" : " — not gating the exit status"}`),
    "",
    `Bar: each metric's median of ${verdict.runs} runs at most 1.${String(verdict.tolerancePercent).padStart(2, "0")} times main's; findings set-wise against the upstream recording; every run main completes, completed by the candidate; every rule main ships runs.`,
    `Machine: ${verdict.machine.hosts.join(", ") || "–"}; CPUs ${verdict.machine.cpus.join(" / ") || "–"} (${verdict.machine.threads.join("/") || "–"} threads); caps ${verdict.machine.memCap.join("/") || "–"} memory, ${verdict.machine.timeCapS.join("/") || "–"} s.`,
    "",
  ];
  for (const c of verdict.candidates) lines.push(...candidateSection(verdict, c));
  return `${lines.join("\n")}\n`;
}
