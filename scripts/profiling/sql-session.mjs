// Execute the retained session and independent oracle, then enforce shared acceptance.
import { readFileSync, writeFileSync, mkdirSync } from "node:fs";
import { resolve, join } from "node:path";
import { spawnSync } from "node:child_process";
import { captureDigests, sessionWorkload, sha256 } from "../../benchmark/sql-profile/session-parity.mjs";
import { buildReport } from "../../benchmark/sql-profile/report.mjs";
import { summaryMarkdown } from "../../benchmark/sql-profile/summary.mjs";
import { requireExecutionBudget } from "../../benchmark/sql-profile/execution-budget.mjs";

if (process.argv.length !== 5) throw new Error("Usage: node scripts/profiling/sql-session.mjs VERIFIER REQUEST.json OUTPUT_DIRECTORY");
const [runner, request, output] = process.argv.slice(2).map(value => resolve(value));
const raw = join(output, "raw"), bundle = join(output, "parity");
// A capture belongs to exactly one run. Refuse reuse, including failed previous runs.
mkdirSync(output, { recursive: true }); mkdirSync(raw); mkdirSync(bundle);
writeFileSync(join(bundle, "request.json"), readFileSync(request));
const untracked = spawnSync("git", ["ls-files", "--others", "--exclude-standard", "-z"], { encoding: "utf8" });
if (untracked.status !== 0) throw new Error("Cannot record source identity");
const execution = { runnerSha256: sha256(readFileSync(runner)), source: {
  commit: spawnSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).stdout?.trim(),
  diffSha256: sha256(spawnSync("git", ["diff", "HEAD", "--binary"], { encoding: null }).stdout ?? Buffer.alloc(0)),
  untracked: Object.fromEntries(untracked.stdout.split("\0").filter(Boolean).sort().map(name => [name, sha256(readFileSync(name))])),
} };
for (const mode of ["retained", "oracle"]) {
  const env = { ...process.env }; delete env.RERAIL_SQL_PROFILE_DIR; delete env.RERAIL_PROFILE;
  if (mode === "retained") env.RERAIL_SQL_PROFILE_DIR = raw;
  const result = spawnSync(runner, [mode, join(output, `${mode}-stores`), join(bundle, "request.json")], {
    encoding: "utf8", env, timeout: 300_000, maxBuffer: 256 * 1024 * 1024,
  });
  writeFileSync(join(bundle, `${mode}.json`), result.stdout ?? "");
  writeFileSync(join(bundle, `${mode}.stderr.txt`), result.stderr ?? "");
  execution[mode] = { status: result.status, signal: result.signal, error: result.error?.message };
  if (result.status !== 0 || result.signal || result.error) {
    writeFileSync(join(bundle, "execution.json"), JSON.stringify(execution, null, 2));
    throw new Error(`${mode} verification failed: ${result.stderr ?? result.error?.message}`);
  }
}
if (sha256(readFileSync(runner)) !== execution.runnerSha256) throw new Error("Verifier binary changed during the run");
writeFileSync(join(bundle, "execution.json"), JSON.stringify(execution, null, 2) + "\n");
const files = Object.fromEntries(["request.json", "retained.json", "oracle.json", "execution.json"].map(name => [name, sha256(readFileSync(join(bundle, name)))]));
writeFileSync(join(bundle, "artifacts.json"), JSON.stringify({ files, capture: captureDigests(raw) }, null, 2) + "\n");
const reportDirectory = join(output, "report"); mkdirSync(reportDirectory);
const workload = sessionWorkload(raw, bundle);
writeFileSync(join(reportDirectory, "workload.json"), JSON.stringify(workload, null, 2) + "\n");
const report = await buildReport(raw, reportDirectory, workload);
console.log(summaryMarkdown(report));
requireExecutionBudget(report);
