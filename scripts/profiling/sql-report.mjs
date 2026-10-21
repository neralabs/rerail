// Build the imported SQL reports from the bounded data-layer profiler's capture.
import { readFileSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { buildReport } from "../../benchmark/sql-profile/report.mjs";
import { summaryMarkdown } from "../../benchmark/sql-profile/summary.mjs";

if (process.argv.length !== 3) throw new Error("Usage: node scripts/profiling/sql-report.mjs PROFILE_DIRECTORY");
const directory = resolve(process.argv[2]);
const profile = JSON.parse(readFileSync(join(directory, "profile.json"), "utf8"));
const parity = profile.parity;
const applyCpuMs = parity?.phaseCpuMs ? Object.entries(parity.phaseCpuMs)
  .filter(([phase]) => ["cold", "unchanged", "edit"].includes(phase))
  .reduce((total, [, cpu]) => total + Object.values(cpu ?? {}).reduce((sum, value) => sum + (value ?? 0), 0), 0) : undefined;
const passed = profile.exitCode !== 0 || profile.timedOut ? false : parity ?
  ["falsePositives", "falseNegatives", "failures", "incrementalDifferences"].every(key => Array.isArray(parity[key]) && parity[key].length === 0) : undefined;
const workload = {
  source: profile.source ?? { note: "Source identity was not captured by this historical run." },
  executor: "relational", command: profile.command, nativeSha256: parity?.nativeSha256,
  workloadSha256: parity?.workloadSha256, policyDigests: parity?.policyDigests,
  allocator: parity?.allocator, phaseCpuMs: parity?.phaseCpuMs,
  phaseMarkers: profile.native.phaseMarkers, host: profile.host,
  results: { parity: passed, exitCode: profile.exitCode, timedOut: profile.timedOut,
    sql: applyCpuMs === undefined ? undefined : { apply: { cpuMs: applyCpuMs } },
    oracle: parity ? "Pinned data-layer findings and incremental comparisons; no alternate SQL executor is run."
      : "Unavailable: this driver did not provide parity.json." },
};
writeFileSync(join(directory, "sql", "workload.json"), JSON.stringify(workload, null, 2) + "\n");
const report = await buildReport(join(directory, "sql", "raw"), join(directory, "sql"), workload);
console.log(summaryMarkdown(report));
