// Summarize live-query replay artifacts. CPU comparisons pair the same snapshots and omit warmups.
// Usage: node benchmark/sql-query-shapes.mjs /absolute/replay/directory
import { readFileSync, readdirSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { pathToFileURL } from "node:url";

const median = values => [...values].sort((a, b) => a - b)[Math.floor(values.length / 2)];
const sum = (items, key) => items.reduce((n, item) => n + (item[key] ?? 0), 0);
const fields = ["cpuNs", "prepareCpuNs", "vmSteps", "frameNext", "frameColumns", "patternFilters", "factFilters"];

export function summarizeCases(cases) {
  const groups = new Map(), issues = [];
  for (const c of cases) {
    if (!c.report.stat1Restored) issues.push({ case: c.case, problem: "statistics not restored" });
    for (const s of c.report.samples.filter(s => s.round === null)) {
      if (s.metrics.error || s.metrics.parity !== true) issues.push({ case:c.case,variant:s.variant,problem:"warmup failed",error:s.metrics.error,mismatch:s.metrics.parity===false });
    }
    const samples = c.report.samples.filter(s => s.round !== null && Number.isSafeInteger(s.round));
    const variants = new Map();
    for (const sample of [...samples, ...c.report.analyze.flatMap(a => a.samples)]) {
      const rows = variants.get(sample.variant) ?? [];
      rows.push(sample.metrics); variants.set(sample.variant, rows);
    }
    const baseline = variants.get("baseline");
    if (!baseline?.length || baseline.some(m => m.parity !== true || m.error)) {
      issues.push({ case: c.case, problem: "baseline missing or failed" }); continue;
    }
    const base = Object.fromEntries(fields.map(f => [f, median(baseline.map(m => m[f] ?? 0))]));
    for (const [name, rows] of variants) {
      const key = `${c.family}:${name}`;
      if (!groups.has(key)) groups.set(key, { family: c.family, variant: name, cases: [], failures: [] });
      const group = groups.get(key);
      if (rows.some(m => m.parity !== true || m.error)) {
        const failure = { case: c.case, errors: rows.filter(m => m.error).map(m => m.error), mismatch: rows.some(m => m.parity === false),
          batchingViolation: rows.some(m => m.patternFilters > base.patternFilters || m.factFilters > base.factFilters) };
        group.failures.push(failure); issues.push({ family: c.family, variant: name, ...failure }); continue;
      }
      const current = Object.fromEntries(fields.map(f => [f, median(rows.map(m => m[f] ?? 0))]));
      const phase = c.report.analyze.find(p => p.variant === name);
      let setupCpuNs = phase?.setup.cpuNs ?? 0;
      if (name === "btree-unanalyzed") setupCpuNs = c.report.setup[0].cpuNs;
      if (name === "btree-analyzed") setupCpuNs = c.report.setup[1].cpuNs + c.report.setup[2].cpuNs;
      group.cases.push({ case: c.case, ...current, setupCpuNs, base,
        orderedParity: rows.every(m => m.orderedParity !== false),
        batched: current.patternFilters <= base.patternFilters && current.factFilters <= base.factFilters,
        setupError: phase?.setup.error ?? null });
    }
  }
  const results = [...groups.values()].map(group => {
    const cost = Object.fromEntries(fields.map(f => [f, sum(group.cases, f)]));
    const baseCpuNs = group.cases.reduce((n, c) => n + c.base.cpuNs, 0);
    return { ...group, samples: group.cases.length, ...cost,
      cpuMs: cost.cpuNs / 1e6, setupCpuMs: sum(group.cases, "setupCpuNs") / 1e6,
      baselineCpuMs: baseCpuNs / 1e6, cpuRatio: baseCpuNs ? cost.cpuNs / baseCpuNs : null,
      unbatchedCases: group.cases.filter(c => !c.batched).length + group.failures.filter(c => c.batchingViolation).length,
      reorderedCases: group.cases.filter(c => !c.orderedParity).length,
      setupErrors: group.cases.filter(c => c.setupError).length,
    };
  }).sort((a,b) => a.family.localeCompare(b.family) || (a.cpuRatio ?? Infinity) - (b.cpuRatio ?? Infinity));
  const rejected = new Map();
  for (const issue of issues) {
    const budget = issue.error === "interrupted" || (issue.errors?.length > 0 && issue.errors.every(e => e === "interrupted"));
    const reason = issue.mismatch ? "mismatch" : budget ? "budget" : "unexpected";
    rejected.set(`${issue.case}:${issue.variant}:${reason}`,reason);
  }
  const quality = { budgetLimitedSnapshots:[...rejected.values()].filter(r=>r==="budget").length,
    mismatchedSnapshots:[...rejected.values()].filter(r=>r==="mismatch").length,
    unexpectedFailures:[...rejected.values()].filter(r=>r==="unexpected").length };
  return { version: 1, caseCount: cases.length, issues, quality, results,
    interpretation: "Sum of per-snapshot CPU medians, paired with baseline on the same successful snapshots. Native terminal caches are warm. CPU includes result collection and encoding. Setup is charged once per snapshot, outside query CPU. These are diagnostic query comparisons, not end-to-end speedups. Reordered rows require whole-workspace capture and first-match validation. Terminal variants with more filters violate the batching contract." };
}

export function markdown(summary) {
  const lines = ["# SQL shape replay", "", summary.interpretation, "", `Snapshots: ${summary.caseCount}. Budget-limited alternatives: ${summary.quality.budgetLimitedSnapshots}. Mismatched snapshots: ${summary.quality.mismatchedSnapshots}. Unexpected failures: ${summary.quality.unexpectedFailures}.`, "",
    "| Family | Shape | Cases | Query CPU ms | Baseline ratio | Prepare CPU ms | Setup CPU ms | VM steps | Frame advances | Unbatched | Reordered | Setup errors |",
    "|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|"];
  for (const r of summary.results) lines.push(`| ${r.family} | ${r.variant} | ${r.samples} | ${r.cpuMs.toFixed(2)} | ${r.cpuRatio?.toFixed(3) ?? "n/a"} | ${(r.prepareCpuNs/1e6).toFixed(2)} | ${r.setupCpuMs.toFixed(2)} | ${r.vmSteps} | ${r.frameNext} | ${r.unbatchedCases} | ${r.reorderedCases} | ${r.setupErrors} |`);
  return lines.join("\n") + "\n";
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  if (process.argv.length !== 3) throw new Error("Usage: node benchmark/sql-query-shapes.mjs /absolute/replay/directory");
  const directory = resolve(process.argv[2]);
  const cases = readdirSync(join(directory,"cases")).filter(name => /^case-\d+\.json$/.test(name)).sort()
    .map(name => JSON.parse(readFileSync(join(directory,"cases",name),"utf8")));
  const summary = summarizeCases(cases);
  writeFileSync(join(directory,"summary.json"),JSON.stringify(summary,null,2)+"\n");
  writeFileSync(join(directory,"summary.md"),markdown(summary));
  console.log(markdown(summary));
  if (summary.quality.mismatchedSnapshots || summary.quality.unexpectedFailures) process.exitCode = 1;
}
