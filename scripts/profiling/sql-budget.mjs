#!/usr/bin/env node
// One fixed cold acceptance workload. The profiler counts every connection and execution.
import { createHash } from "node:crypto";
import { readFileSync, writeFileSync, mkdirSync, existsSync } from "node:fs";
import { join, resolve, dirname } from "node:path";
import { spawnSync } from "node:child_process";
import { executionBudget, requireExecutionBudget } from "../../benchmark/sql-profile/execution-budget.mjs";

if (process.argv.length !== 3) throw new Error("Usage: node scripts/profiling/sql-budget.mjs NEW_OUTPUT_DIRECTORY");
const directory = resolve(process.argv[2]);
mkdirSync(dirname(directory), { recursive: true });
mkdirSync(directory);
const capture = join(directory, "capture"); // run.py refuses an existing capture directory.
const source = "fixtures/host/data-layer-port/batch-06-recording.json";
const bytes = readFileSync(source), original = JSON.parse(bytes);
const tests = [
  "scripts/tests/test_lint_serverplan_state_tripwires.py::test_rejects_lazy_result_private_step_ref_access",
  "scripts/tests/test_lint_serverplan_state_tripwires.py::test_collector_uses_generated_return_kinds_without_recomputing_project_map",
];
const events = tests.map(test => {
  const matches = original.events.filter(e => e.test === test && e.call === 1);
  if (matches.length !== 1 || !matches[0].upstream_passed || matches[0].retained_reason !== undefined) {
    throw new Error(`Missing original executable observation: ${test}`);
  }
  return matches[0];
});
if (!events[0].findings.length || events[1].findings.length) throw new Error("The acceptance workload requires positive and clean observations");
const input = join(directory, "recording.json");
writeFileSync(input, JSON.stringify({ ...original, checks: ["serverplan_state_tripwires"], events }, null, 2) + "\n");
writeFileSync(join(directory, "provenance.json"), JSON.stringify({ source, sourceSha256: createHash("sha256").update(bytes).digest("hex"),
  commit: original.commit, tests, note: "Unchanged original observations. One cold process includes snapshot and workspace parity for both cases." }, null, 2) + "\n");
const child = spawnSync("python3", ["scripts/profiling/run.py", "--trace", "off", "--sql-profile", "--deadline", "115",
  "--output", capture, "--", process.execPath, "--import", "tsx", "conformance/data-layer/replay.ts",
  "--recording", input, "--output", join(capture, "parity.json")], { stdio: "inherit", timeout: 118_000 });
if (child.error) throw child.error;
const reportPath = join(capture, "sql", "profile.json");
if (!existsSync(reportPath)) throw new Error("Missing complete SQL execution report; acceptance fails closed.");
const report = JSON.parse(readFileSync(reportPath));
const budget = executionBudget(report);
console.log(`SQL execution budget: ${budget.status}; ${budget.observed}/${budget.limit} engine submissions for the complete cold job; ${budget.sqliteInternal} SQLite internal executions; ${budget.totalObserved} total.`);
if (budget.firstOverBudget) console.log(`Execution 101: ${budget.firstOverBudget.sql}`);
requireExecutionBudget(report);
process.exitCode = child.status ?? 1;
