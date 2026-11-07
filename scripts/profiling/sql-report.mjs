// Build the imported SQL reports from the bounded data-layer profiler's capture.
import { readFileSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { buildReport } from "../../benchmark/sql-profile/report.mjs";
import { summaryMarkdown } from "../../benchmark/sql-profile/summary.mjs";
import { requireExecutionBudget } from "../../benchmark/sql-profile/execution-budget.mjs";

if (process.argv.length !== 3) throw new Error("Usage: node scripts/profiling/sql-report.mjs PROFILE_DIRECTORY");
const directory = resolve(process.argv[2]);
const profile = JSON.parse(readFileSync(join(directory, "profile.json"), "utf8"));
const parity = profile.parity;
// A previous report can fail its budget after the engine completed successfully.
const targetExitCode = profile.targetExitCode ?? profile.exitCode;
const targetTimedOut = profile.targetTimedOut ?? profile.timedOut;
const applyCpuMs = parity?.phaseCpuMs ? Object.entries(parity.phaseCpuMs)
  .filter(([phase]) => ["cold", "unchanged", "edit"].includes(phase))
  .reduce((total, [, cpu]) => total + Object.values(cpu ?? {}).reduce((sum, value) => sum + (value ?? 0), 0), 0) : undefined;
const passed = targetExitCode !== 0 || targetTimedOut ? false : parity ?
  parity.format === "rerail.data-layer-parity/v1"
    ? ["falsePositives", "falseNegatives", "engineFailures", "executionPathDifferences"].every(key => parity[key] === 0)
      && Array.isArray(parity.missingCoverage) && parity.missingCoverage.length === 0
    : ["falsePositives", "falseNegatives", "failures", "incrementalDifferences"].every(key => Array.isArray(parity[key]) && parity[key].length === 0) : undefined;
const workload = {
  source: profile.source ?? { note: "Source identity was not captured by this historical run." },
  executor: "relational", command: profile.command, nativeSha256: parity?.nativeSha256,
  workloadSha256: parity?.workloadSha256, policyDigests: parity?.policyDigests,
  allocator: parity?.allocator, phaseCpuMs: parity?.phaseCpuMs,
  phaseMarkers: profile.native.phaseMarkers, host: profile.host,
  results: { parity: passed, exitCode: targetExitCode, timedOut: targetTimedOut,
    conformance: parity,
    sql: applyCpuMs === undefined ? undefined : { apply: { cpuMs: applyCpuMs } },
    oracle: parity ? "Pinned data-layer findings and incremental comparisons; no alternate SQL executor is run."
      : "Unavailable: this driver did not provide parity.json." },
};
writeFileSync(join(directory, "sql", "workload.json"), JSON.stringify(workload, null, 2) + "\n");
const report = await buildReport(join(directory, "sql", "raw"), join(directory, "sql"), workload);
console.log(summaryMarkdown(report));
requireExecutionBudget(report);
