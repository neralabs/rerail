// Usage: node benchmark/sql-profile.mjs workload.json /absolute/output/directory
// Reuses the SQL investigation workload schema and verifies complete output against native.
import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdirSync, readFileSync, writeFileSync, existsSync } from "node:fs";
import { resolve, join } from "node:path";
import { fileURLToPath } from "node:url";
import { buildReport } from "./sql-profile/report.mjs";
import { summaryMarkdown } from "./sql-profile/summary.mjs";
import { requireExecutionBudget } from "./sql-profile/execution-budget.mjs";

const [configArg, outputArg] = process.argv.slice(2);
if (!configArg || !outputArg || process.argv.length !== 4) {
  console.error("Usage: node benchmark/sql-profile.mjs workload.json /absolute/output/directory");
  process.exit(1);
}
const configPath = resolve(configArg), output = resolve(outputArg);
// A fresh directory prevents stale catalogues and mixed clocks from entering one report.
if (existsSync(output)) throw new Error(`Output directory already exists: ${output}`);
mkdirSync(output, { recursive: true });
const config = JSON.parse(readFileSync(configPath, "utf8"));
const raw = join(output, "raw");
mkdirSync(raw);
const runner = fileURLToPath(new URL("./sql-set-investigation.mjs", import.meta.url));
const sha256 = path => createHash("sha256").update(readFileSync(path)).digest("hex");
const workload = { configPath, config,
  profilerDriverSha256: sha256(fileURLToPath(import.meta.url)),
  reporterSha256: sha256(fileURLToPath(new URL("./sql-profile/report.mjs", import.meta.url))),
  summarySha256: sha256(fileURLToPath(new URL("./sql-profile/summary.mjs", import.meta.url))),
  addonSha256: sha256(config.addon), runnerSha256: sha256(runner),
  capabilitiesSha256: sha256(config.capabilities),
  policies: config.policies.map(path => ({ path, sha256: sha256(path) })),
  files: config.files.map(path => ({ path, sha256: sha256(resolve(config.corpus, path)) })) };
writeFileSync(join(output, "workload.json"), JSON.stringify(workload, null, 2) + "\n");
const childConfig = join(output, "config.json");
writeFileSync(childConfig, JSON.stringify(config, null, 2) + "\n");
const environment = { ...process.env };
for (const key of ["RERAIL_SQL_PROFILE_DIR", "RERAIL_SQL_TRACE_DIR", "RERAIL_SQL_TRACE_FILTER"]) delete environment[key];
// Preserve engine settings and record them so this profile is reproducible.
workload.environment = Object.fromEntries(Object.entries(environment).filter(([key]) => key.startsWith("RERAIL_SQL_") || key === "RERAIL_BENCH_WORKERS" || key === "JEMALLOC_OVERRIDE"));
async function run(engine) {
  console.error(`Running ${engine}${engine === "sql" ? " with profiling" : " output oracle"}…`);
  const started = Date.now();
  return new Promise((accept, reject) => {
    const child = spawn(process.execPath, [runner, childConfig, "--child", engine, join(output, `${engine}-outcomes.json`)], {
      env: { ...environment, ...(engine === "sql" ? { RERAIL_SQL_PROFILE_DIR: raw } : {}) }, stdio: ["ignore", "pipe", "pipe"],
    });
    let stdout = "", stderr = "";
    child.stdout.setEncoding("utf8").on("data", data => { stdout += data; });
    child.stderr.setEncoding("utf8").on("data", data => { stderr += data; });
    const timer = setTimeout(() => child.kill("SIGKILL"), config.timeoutMs ?? 600_000);
    child.on("error", error => { clearTimeout(timer); reject(error); });
    child.on("close", (code, signal) => {
      clearTimeout(timer);
      writeFileSync(join(output, `${engine}.stderr.log`), stderr);
      writeFileSync(join(output, `${engine}.stdout.log`), stdout);
      if (code !== 0) return reject(new Error(`${engine} failed (${signal ?? code}). See ${engine}.stderr.log. Elapsed ${Date.now() - started} ms.`));
      try {
        const result = JSON.parse(stdout);
        writeFileSync(join(output, `${engine}.json`), JSON.stringify(result, null, 2) + "\n");
        accept(result);
      } catch (error) { reject(error); }
    });
  });
}
const native = await run("native");
let sql, failure;
try { sql = await run("sql"); } catch (error) { failure = error.message; }
workload.results = { native, sql, error: failure,
  parity: !failure && native.digest === sql.digest && native.counts.failures === 0 && sql.counts.failures === 0 };
writeFileSync(join(output, "workload.json"), JSON.stringify(workload, null, 2) + "\n");
const report = await buildReport(raw, output, workload);
console.log(summaryMarkdown(report));
console.log(`Reports: ${output}/summary.md and ${output}/profile.html`);
if (failure) console.error(failure);
if (!workload.results.parity) process.exitCode = 1;
// A correct result above 100 actual executions is still a failed compiler acceptance run.
requireExecutionBudget(report);
