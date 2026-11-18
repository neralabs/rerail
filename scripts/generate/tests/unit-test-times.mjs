// Writes test/support/shards/unit-test-times.generated.ts: how long each unit test file takes in CI, per platform,
// which the time-balanced sequencer (test/support/shards/time-balanced-sequencer.ts) splits the shards by. It reads the
// logs of CI's unit test jobs (`Unit tests (k/n)` on Linux, `Unit tests (Windows, k/n)`), where vitest prints each
// file's time (` ✓ test/… (12 tests) 3456ms`), and keeps each file's median over the runs. The first shard also runs a
// step after the suite (the coverage gate, the hook integration test); its time is kept as `firstShardExtraMs`, scaled
// to test-file time by how many files that shard's suite ran side by side. Files no longer in the checkout are left
// out. Needs the GitHub CLI, signed in; run it in this checkout, not in CI:
//   node scripts/generate/tests/unit-test-times.mjs [<run id>...]
// With no run ids it reads the latest successful CI runs on main.
import { execFileSync } from "node:child_process";
import { existsSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const ROOT = resolve(fileURLToPath(new URL("../../..", import.meta.url)));
const OUTPUT = "test/support/shards/unit-test-times.generated.ts";
/** How many of main's latest successful runs the times come from when no run is named. */
const DEFAULT_RUNS = 5;

/** A unit test job's name: `Unit tests (1/3)` on Linux, `Unit tests (Windows, 1/5)` on Windows. */
const UNIT_JOB = /^Unit tests \((?:(Windows), )?(\d+)\/(\d+)\)$/u;
/** The step that runs the sharded suite, as the log's group header shows its command. */
const SUITE_STEP = "##[group]Run npm test -- --shard=";
const STEP = "##[group]Run ";
/** vitest's line for one test file: its mark, its path, its test count, and (unless every test skipped) its time. */
const FILE_LINE = /^\s*[✓×❯↓]\s+(\S+\.(?:test|spec)\.[cm]?[jt]sx?)\s+\(\d+ tests?[^)]*\)(?:\s+(\d+(?:\.\d+)?)(ms|s))?\s*$/u;
/** vitest's summary line of a run's wall time. */
const DURATION_LINE = /^\s*Duration\s+(\d+(?:\.\d+)?)(ms|s)\b/u;
/** The timestamp the Actions log puts before each line. */
const TIMESTAMP = /^\d{4}-\d\d-\d\dT[\d:.]+Z /u;
/** The colours vitest prints, which the Actions log keeps. */
const COLOUR = /\u001b\[[\d;]*m/gu;

const milliseconds = (value, unit) => Number(value) * (unit === "s" ? 1000 : 1);

/** The platform (as `process.platform` names it) and shard of a unit test job, or undefined for any other job. */
export function unitJob(name) {
  const match = UNIT_JOB.exec(name);
  if (match === null) return undefined;
  return { platform: match[1] === "Windows" ? "win32" : "linux", shard: Number(match[2]) };
}

/**
 * What one unit test job's log says: each file's milliseconds in the sharded suite, the suite's wall time, and the
 * wall time of the vitest runs of the steps after it (zero when there are none).
 */
export function parseUnitLog(log) {
  const files = new Map();
  let section = "before";
  let suiteMs = 0;
  let extraMs = 0;
  for (const raw of log.split(/\r?\n/u)) {
    const line = raw.replace(TIMESTAMP, "").replace(COLOUR, "");
    if (line.startsWith(SUITE_STEP)) section = "suite";
    else if (line.startsWith(STEP) && section !== "before") section = "after";
    if (section === "before") continue;
    const duration = DURATION_LINE.exec(line);
    if (duration !== null) {
      if (section === "suite") suiteMs = milliseconds(duration[1], duration[2]);
      else extraMs += milliseconds(duration[1], duration[2]);
      continue;
    }
    const file = section === "suite" ? FILE_LINE.exec(line) : null;
    if (file !== null) files.set(file[1], file[2] === undefined ? 0 : milliseconds(file[2], file[3]));
  }
  return { files, suiteMs, extraMs };
}

function median(values) {
  const sorted = [...values].sort((a, b) => a - b);
  const middle = Math.floor(sorted.length / 2);
  return sorted.length % 2 === 1 ? sorted[middle] : (sorted[middle - 1] + sorted[middle]) / 2;
}

/**
 * The times file from parsed unit test jobs (`{ platform, shard, log }` with `log` from parseUnitLog), keeping only the
 * files `exists` says the checkout still has.
 */
export function unitTestTimes(jobs, exists) {
  const platforms = {};
  for (const platform of [...new Set(jobs.map((job) => job.platform))].sort()) {
    const ofPlatform = jobs.filter((job) => job.platform === platform);
    const samples = new Map();
    for (const { log } of ofPlatform) {
      for (const [path, ms] of log.files) samples.set(path, [...(samples.get(path) ?? []), ms]);
    }
    const extras = ofPlatform
      .filter(({ shard, log }) => shard === 1 && log.suiteMs > 0)
      .map(({ log }) => log.extraMs * ([...log.files.values()].reduce((sum, ms) => sum + ms, 0) / log.suiteMs));
    const files = {};
    for (const path of [...samples.keys()].sort()) if (exists(path)) files[path] = Math.round(median(samples.get(path)));
    platforms[platform] = { firstShardExtraMs: extras.length === 0 ? 0 : Math.round(median(extras)), files };
  }
  return platforms;
}

const gh = (...args) => execFileSync("gh", ["api", ...args], { cwd: ROOT, encoding: "utf8", maxBuffer: 256 * 1024 * 1024 });

/** A run's jobs, a page of the API's default size at a time: it has answered a page of 100 with a server error. */
function runJobs(run) {
  const lines = gh("--paginate", `repos/{owner}/{repo}/actions/runs/${run}/jobs`, "--jq", ".jobs[] | tojson");
  return lines.split("\n").filter(Boolean).map((line) => JSON.parse(line));
}

function latestRuns(count) {
  const runs = JSON.parse(gh(`repos/{owner}/{repo}/actions/workflows/ci.yml/runs?branch=main&event=push&status=success&per_page=${count}`));
  return runs.workflow_runs.map((run) => String(run.id));
}

function main() {
  const runs = process.argv.length > 2 ? process.argv.slice(2) : latestRuns(DEFAULT_RUNS);
  const jobs = [];
  for (const run of runs) {
    for (const job of runJobs(run)) {
      const unit = unitJob(job.name);
      if (unit === undefined || job.conclusion !== "success") continue;
      jobs.push({ ...unit, log: parseUnitLog(gh(`repos/{owner}/{repo}/actions/jobs/${job.id}/logs`)) });
    }
  }
  if (jobs.length === 0) throw new Error(`no successful unit test job in runs ${runs.join(", ")}`);
  const times = unitTestTimes(jobs, (path) => existsSync(join(ROOT, path)));
  writeFileSync(join(ROOT, OUTPUT), timesModule(times, runs));
  const counts = Object.entries(times).map(([platform, { files }]) => `${platform} ${Object.keys(files).length} files`);
  console.log(`wrote ${OUTPUT} from ${jobs.length} jobs of runs ${runs.join(", ")}: ${counts.join(", ")}`);
}

/** The times file's text: a module of the times, saying which runs they come from. */
function timesModule(times, runs) {
  return [
    "// Generated by scripts/generate/tests/unit-test-times.mjs; do not edit.",
    "// Each unit test file's median milliseconds in CI's unit test jobs, per platform.",
    `// From CI runs ${runs.join(", ")}.`,
    'import type { TestTimes } from "./time-balanced-sequencer.js";',
    "",
    `export const UNIT_TEST_TIMES: TestTimes = ${JSON.stringify(times, null, 2)};`,
    "",
  ].join("\n");
}

if (process.argv[1] !== undefined && import.meta.url === pathToFileURL(process.argv[1]).href) main();
