#!/usr/bin/env node
// The shippability bar: a candidate engine is no worse than main's on every rule pack main ships. One run gives two
// verdicts, one per candidate, each PASS, FAIL, INCOMPLETE or PARTIAL (docs in scripts/README.md, "ship/"):
//
//   ships  the candidate checkout's engine on its packs as they ship (packs/<pack>/rules/*.json), through the product's
//          own path (conformance/languages/runner.ts evaluateWorkspace, as the candidate has it: product-run.ts);
//   rr     the candidate's engine on the hand-written `.rerail` rewrites, docs/rule-syntax/packs/<pack>/<id>.rerail, through
//          `rerail-engine run-policies <corpus> <out> <rule.rerail>...`;
//   main   the baseline: main's checkout on its shipped packs/<pack>/rules/*.json, through its own product path.
//
// Commands (each prints what it refuses and why; nothing is measured unpinned):
//
//   node --import tsx scripts/ship/shipbar.ts prepare --baseline <release:version | git ref | dir> --out <dir> [--packs a,b]
//       [--record]
//       the baseline checkout and the pinned corpora in <out>/corpora: each pack's fixtures from its upstream
//       recordings, and its real-code corpus from scripts/ship/corpora.json, whose file count and hash the corpus must
//       match (--record writes them there). A git ref is exported to <out>/baseline with its pack data pulled;
//       release:<version> (e.g. release:0.1.4) is the source at its tag v<version> with the published
//       @neralabs/rerail@<version>'s prebuilt engines and JavaScript put in place, so the baseline is never built:
//       `npm ci` is all it needs. Needs git, npm and the pack data cache (AWS); runs anywhere.
//   node --import tsx scripts/ship/shipbar.ts run --baseline <dir | ref> --out <dir> [--candidate <dir>] [--corpora <dir>]
//       [--packs a,b] [--corpus fixtures|real|all] [--runs 1] [--rule-runs 2] [--file-runs 3] [--confirm 3]
//       [--cpus 8-11] [--mem-cap 24G] [--time-cap 2700] [--pack-cap <pack>=<memory>,<seconds>]... [--rr-chunk 50]
//       [--candidates ships] [--gate ships] [--build]
//       Measures the three slices of every pack it is given, always (Method), for each candidate: the shipped packs
//       only by default, `--candidates ships,rr` for the `.rerail` rewrites too (rr needs the candidate's
//       `rerail-engine` release binary as well).
//       Linux, macOS or Windows (platform.ts: how each pins, caps and measures, recorded in every run row): prepares what
//       is missing, measures every pack and corpus, then writes the report. Resumable: a run already in a cell's logs is
//       not made again. --packs and --corpus narrow the run: its verdict is then PARTIAL, never PASS.
//       --pack-cap raises (or lowers) one pack's caps for both sides, say python=56G,14400 so a slow run that finishes
//       gives a number; every run records the caps it ran under. Mind the box's memory: a 56G cap leaves no room for
//       another heavy stream on a 64 GB box.
//   node --import tsx scripts/ship/shipbar.ts verdict --baseline <dir> --out <dir> [--candidate <dir>] [--corpora <dir>]
//       [--packs a,b] [--corpus fixtures|real|all] [--candidates ships,rr] [--gate ships,rr]
//       report.json and report.md from the runs under <out>/runs (several runs' directories merged, say: one per box,
//       each box given some packs). Narrowed by --packs or --corpus, the verdict is PARTIAL; a pack whose runs lack a
//       slice is INCOMPLETE.
//   node --import tsx scripts/ship/shipbar.ts fanout --hosts build-1,build-2 --baseline <release:version | ref | dir>
//       --out <dir> [--candidate <dir>] [--corpora <dir>] [--packs a,b] [--name <work dir on the hosts>]
//       [--workers 0-3/4-7/8-11/12-15] [--stores /dev/shm/shipbar-<name>] [--runs 3] [--rule-runs 2] [--file-runs 3]
//       [--confirm 3] [--mem-cap 24G] [--time-cap 2700] [--pack-cap <pack>=<memory>,<seconds>]... [--poll 180] [--from <phase>]
//       [--candidates ships] [--ssh "<remote shell command>"]
//       From the Mac, the whole bar over Linux hosts (fanout.ts): prepare (as `prepare`); plan every unit (A by
//       repetition, B by chunks of rules, C by file, each unit timing every candidate beside main, none longer than
//       about fanout.ts UNIT_BUDGET_S) and assign them to the hosts longest first; sync the baseline, the candidate,
//       the corpora and this harness to ~/work/<name>/ on each host (rsync over the remote shell: `ssh`, or --ssh /
//       SHIPBAR_SSH, a wrapper taking `<host> <command>` as ssh does); build both sides on the first host (`npm ci
//       --ignore-scripts && npm run build:native`, and the candidate's `rerail-engine` with rr); copy the engines to
//       the others (sha256 checked) and `npm ci` there; on each host one
//       detached `host` process measures (then confirms) with a worker per CPU set; pull every host's out/ and merge it
//       into <out>/runs; report. Each phase's start and end are in <out>/fanout/state.json, and a finished phase is not
//       made again when the command is run again (--from <phase> makes it and every later one again). Both checkouts
//       must hold their Git LFS files. Linux hosts only; the Mac runs prepare and the report, and polls.
//   node --import tsx scripts/ship/shipbar.ts host --plan <plan.json> --host <name> --workers 0-3/4-7 (and run's options)
//   node --import tsx scripts/ship/shipbar.ts work --plan <plan.json> --host <name> --worker <i> --cpus 0-3 --phase measure|confirm
//       A fan-out's processes on a host: `host` verifies the corpora once, then starts a `work` process per CPU set
//       and waits for them, for measure then confirm. Each worker takes this host's units longest first, each claimed by
//       an atomic mkdir of out/claims/<phase>/<unit>, and runs it through run's own code into the same cell logs, so
//       `verdict` over the merged runs/ gives the report. Slice B and C confirm their rules and files as they go; the
//       confirm phase confirms slice A's cells, whose repetitions all ran on this host.
//   node --import tsx scripts/ship/shipbar.ts waits --prepare --out <dir> [--record]
//       the replayed session (scripts/ship/waits.json) built from this checkout's git history into <out>/sessions/,
//       checked against its pinned hash (--record pins it). Runs where the history is; copy <out>/sessions over.
//   node --import tsx scripts/ship/shipbar.ts waits --baseline <dir> --candidate <dir> --out <dir> --corpora <dir>
//       [--waits W1,W2,W5/scan-warm] [--shapes full,one-file,one-rule] [--wait-runs 5] [--replays 2] [--confirm 3]
//       [--cpus 8-11] [--session <file>] [--work /tmp/shipbar-waits] [--repo <waits.json repo>]
//       A --repo without a replayed session (waits.json) runs the step benchmarks only (--waits W1,W5).
//       W1 and W5 include the step benchmarks' edit sequence (waits-steps.ts) in each of --shapes (default full).
//       Linux only: the user waits W1–W8 (waits.ts), through each checkout's built hooks, CLI and daemon (both need
//       `npm run build:js` beside the native engine), then the report with a waits section.
//
// Exit status of run and verdict: 0 only when every candidate in --gate (default: every one in --candidates) passes.
// rr's verdict on a pack counts toward it only where the candidate's docs/rule-syntax/packs/<pack>/equivalence.json
// (rerail.translation-equivalence/v1, written by the translation suite) says `equivalent: true`: every translated rule
// finds what the original does, but for the rules it lists as different, each with why. On any other pack rr is
// measured and judged alike, and reported only: it never fails the run (verdict.ts translationGate).
//
// Method. The benchmark is three slices per pack, all always measured, candidate against main:
//
//   A  all rules × all files: per corpus (fixtures, real code), runs/<pack>/<corpus>/runs.jsonl;
//   B  one rule × all files: per corpus, the empty rule set and every rule main ships that a candidate has (ships:
//      ships too; rr: has a runnable `.rerail` rewrite), each alone, runs/<pack>/<corpus>/one-rule.jsonl;
//   C  all rules × one file: three files of the pack's real-code corpus by bytes among its source extensions
//      (verdict.ts SOURCE_EXTENSIONS): the largest, the median and a small one (5th percentile),
//      runs/<pack>/one-file/<label>/runs.jsonl, the files named in runs/<pack>/one-file/files.json.
//
// Every process runs on the same --cpus (one hardware thread per physical core, the cores' other threads left idle
// and their load recorded with the box's load average in every row; macOS cannot pin, and its runs say so), under the
// same memory cap and time cap, with RERAIL_THREADS the number of --cpus. Cold: a fresh, empty RERAIL_TAPE_CACHE store
// and engine temporary directory; warm: a second process on that store right after. The side order alternates each
// repetition. A side that fails a cold run makes no further run of that cell. The bar, per slice (verdict.ts):
//
//   A  each side --runs times cold then warm; each metric (wall, CPU (user + sys) and peak RSS, cold and warm) is the
//      median of the side's completed runs and fails above 1.15 times main's. Findings come from each side's first
//      cold run and are scored against the tool's upstream recording with main's divergences.json, on (rule, path,
//      line, code) keys: the candidate keeps every true positive of main's and adds no false positive main lacks. Real
//      code has no truth: its agreement is reported and does not gate.
//   B  each rule alone --rule-runs cold runs per side on a fresh store each; its time is product-run.ts's in-process
//      compile + evaluate (median), failing above 1.15 times main's only when still over after its confirmation runs
//      and moved by at least 20 ms (verdict.ts RULE_FLOOR_MS; a rule moved by less is not confirmed: it cannot count).
//      The empty rule set is the fixed cost: its cold wall and its load, compile and evaluate times, each judged
//      alike. Each rule's findings must be the same set on both sides, and the candidate may not fail a rule main runs.
//      rr runs each rule main ships that has a runnable rewrite (rr-rules.json) through `run-policies <corpus> <out>
//      <rule.rerail>`, its findings mapped back from policy to rule. run-policies reports no phase times (its JSON is
//      the findings and each policy's run), so rr's time is its net wall: the rule's cold wall less the median cold
//      wall of rr's empty rule set, a policy of the pack's language with no rules generated into the cell
//      (one-rule.empty.rerail; run-policies takes at least one policy), judged against main's compile + evaluate
//      (verdict.ts netWalls). rr's fixed cost is judged on its cold wall only.
//   C  each file --file-runs times per side, cold then warm on the same store, judged as A's metrics are; its findings
//      must be the same set on both sides (rr: all of its runnable rewrites, against main's findings of those rules).
//
// Anything over the bar runs --confirm more repetitions per side (A: the cell, confirm.jsonl; B: the rule,
// one-rule-confirm.jsonl; C: the file, confirm.jsonl): a figure passing them is noisy, reported and not counted. Every
// run must complete where main's does, and every rule main ships must run on the candidate. Every failing item names
// the phase that moved (load, compile, evaluate, store, startup: verdict.ts locate), from product-run.ts's phases and
// the empty rule set's runs. A pack missing any slice is INCOMPLETE and never passes. Each product side also runs an
// empty rule set over each A corpus, cold and warm (fixed.jsonl), and each corpus's workload is counted once
// (workload/): the report shows start-up, net times, phases and workload beside the gross figures, never judged
// (metric.ts). The report (report-model.ts, schema ship-no-ship/report/v2) is rendered as Markdown by report.ts.
//
// A `.rerail` pack run that does not complete is recorded with how far it got (policies started, elapsed, peak RSS); then
// its rules run in fixed chunks of --rr-chunk (0: none), each chunk its own process under the same caps, cold then
// warm, N times, and the pack's figures are the chunks' sums (peak RSS: the largest chunk's), labelled chunked in the
// report; its completion still fails. Every completed `.rerail` run records whether run-policies ran its policies combined
// (groups of several, the result's `runs[].together`) or alone, and any fallback the engine printed.

import { spawn, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { appendFileSync, closeSync, copyFileSync, cpSync, existsSync, mkdirSync, openSync, readdirSync, readFileSync, renameSync, rmSync, statSync, symlinkSync, writeFileSync } from "node:fs";
import { hostname } from "node:os";
import { dirname, join, matchesGlob, relative, resolve, sep } from "node:path";
import { setTimeout as sleep } from "node:timers/promises";
import { fileURLToPath } from "node:url";
import { parseArgs } from "node:util";
import { engineJson } from "./engine-json.js";
import { claimName, detached, type Done, mergeTrees, type PackInput, type Plan, planFanout, type PlannedUnit, remoteShell, retry, rsync as rsyncOver, ssh as sshOver } from "./fanout.js";
import { cachedWorkload, type CellCost, cellCost, engineBinary, type Phases, readPhases, readWorkload, ruleSet } from "./metric.js";
import { busiest, cpuList, LINUX_METHOD, type MachineInfo, methodCounts, openPlatform, parseProcStat, type Platform, type RunMethod } from "./platform.js";
import { buildReport, type RunMeta } from "./report-model.js";
import { renderReport } from "./report.js";
import { SCENARIOS, scenarioId } from "./waits-model.js";
import { loadSession, prepareSession, type WaitsManifest } from "./waits-session.js";
import { SHAPES } from "./waits-steps.js";
import { readWaitRuns, runWaits, waitsSection } from "./waits.js";
import {
  agreement,
  chunkSums,
  compareCompletion,
  compareFindings,
  compareMetric,
  type CompletionVerdict,
  type Divergence,
  type FileLabel,
  findingKey,
  grouping,
  type FindingsVerdict,
  judgeTime,
  locate,
  type MetricVerdict,
  median,
  missingRules,
  needsConfirmation,
  netWalls,
  newRuleFailures,
  pickFiles,
  RR_RULE_TIME,
  type RunOutcome,
  scoreFindings,
  SOURCE_EXTENSIONS,
  type TimeVerdict,
  SLICES_TOLERANCE_PERCENT,
  translationGate,
} from "./verdict.js";

const HERE = dirname(fileURLToPath(import.meta.url));
/** The checkout this script is in: the default candidate. */
const SELF = resolve(HERE, "..", "..");
const MANIFEST = join(HERE, "corpora.json");
const WAITS_MANIFEST = join(HERE, "waits.json");

export const CANDIDATES = { ships: "the candidate's packs as they ship, through the product's path", rr: "the hand-written .rerail rewrites, through rerail-engine run-policies" } as const;
export type Candidate = keyof typeof CANDIDATES;
type Side = "main" | Candidate;
export type Corpus = "fixtures" | "real";
const CORPORA: readonly Corpus[] = ["fixtures", "real"];
export const METRICS = ["wall-cold", "wall-warm", "cpu-cold", "cpu-warm", "rss-cold", "rss-warm"] as const;

const NPM_INSTALL = "npm ci --ignore-scripts";
const NATIVE_BUILD = "npm run build:native";
const PRODUCT_BUILD = `${NPM_INSTALL} && ${NATIVE_BUILD}`;
const ENGINE_BUILD = "cargo build --release --manifest-path native/Cargo.toml --bin rerail-engine";

// ---------------------------------------------------------------- small helpers

function fail(message: string): never {
  console.error(`shipbar: ${message}`);
  process.exit(2);
}

function run(command: string, args: readonly string[], options: { cwd?: string; input?: string } = {}): { status: number; stdout: string; stderr: string } {
  const result = spawnSync(command, args, { cwd: options.cwd, input: options.input, encoding: "utf8", maxBuffer: 1 << 30 });
  return { status: result.status ?? (result.error === undefined ? 1 : 127), stdout: result.stdout ?? "", stderr: result.stderr ?? String(result.error ?? "") };
}

function readJson<T>(path: string): T {
  return JSON.parse(readFileSync(path, "utf8")) as T;
}

function lastLine(text: string): string {
  const lines = text.split("\n").map((line) => line.trim()).filter((line) => line !== "");
  return (lines.at(-1) ?? "").slice(0, 300);
}

/** Every file under `root`, as sorted paths relative to it. */
function listFiles(root: string): string[] {
  const out: string[] = [];
  const walk = (directory: string): void => {
    for (const entry of readdirSync(directory, { withFileTypes: true })) {
      const full = join(directory, entry.name);
      if (entry.isDirectory()) walk(full);
      else if (entry.isFile()) out.push(relative(root, full).split(sep).join("/"));
    }
  };
  if (existsSync(root)) walk(root);
  return out.sort();
}

/** A tree's identity: its file count, and the sha256 of each sorted path with its content's sha256. */
function hashTree(root: string): { files: number; sha256: string } {
  const files = listFiles(root);
  const hash = createHash("sha256");
  for (const path of files) hash.update(`${path}\u0000${createHash("sha256").update(readFileSync(join(root, path))).digest("hex")}\n`);
  return { files: files.length, sha256: hash.digest("hex") };
}

function copyInto(source: string, destination: string): void {
  mkdirSync(dirname(destination), { recursive: true });
  copyFileSync(source, destination);
}

// ---------------------------------------------------------------- checkouts and packs

interface Checkout {
  dir: string;
  sha: string;
  /** The ref it was resolved from (a tag, a branch), when one was given. */
  ref?: string;
}

const SOURCE_FILE = ".rerail/shipbar-source.json";

/** The commit a checkout is at: git's, else the one `prepare` recorded (a copy synced without .git), else unknown. */
function checkoutSha(dir: string): string {
  const head = run("git", ["-C", dir, "rev-parse", "HEAD"]);
  if (head.status === 0) {
    const dirty = run("git", ["-C", dir, "status", "--porcelain", "--untracked-files=no"]).stdout.trim() !== "";
    return head.stdout.trim() + (dirty ? "+uncommitted" : "");
  }
  return existsSync(join(dir, SOURCE_FILE)) ? readJson<{ sha: string }>(join(dir, SOURCE_FILE)).sha : "unknown";
}

/**
 * The baseline: an existing checkout; release:<version>, the published release (resolveRelease); or a git ref of this
 * repository exported to <out>/baseline with its pack data pulled.
 */
function resolveBaseline(spec: string, out: string, packDataCache: string | undefined): Checkout {
  if (existsSync(join(spec, "packs"))) {
    const source = join(spec, SOURCE_FILE);
    const ref = existsSync(source) ? readJson<{ ref?: string }>(source).ref : undefined;
    return { dir: resolve(spec), sha: checkoutSha(resolve(spec)), ...(ref === undefined ? {} : { ref }) };
  }
  const release = /^release:(\d+\.\d+\.\d+(?:-[\w.]+)?)$/.exec(spec)?.[1];
  if (release !== undefined) return resolveRelease(release, out, packDataCache);
  const parsed = run("git", ["-C", SELF, "rev-parse", "--verify", `${spec}^{commit}`]);
  if (parsed.status !== 0) fail(`--baseline ${spec} is neither a checkout (no packs/), release:<version>, nor a commit of ${SELF}`);
  return exportCommit(parsed.stdout.trim(), spec, out, packDataCache);
}

/** A commit of this repository exported to <out>/baseline, its pack data pulled; reused when already there. */
function exportCommit(sha: string, ref: string, out: string, packDataCache: string | undefined): Checkout {
  const dir = join(out, "baseline");
  if (existsSync(join(dir, SOURCE_FILE)) && readJson<{ sha: string }>(join(dir, SOURCE_FILE)).sha === sha) return { dir, sha, ref };
  if (existsSync(dir)) fail(`${dir} holds another baseline; remove it or pass it as --baseline`);
  mkdirSync(dir, { recursive: true });
  const tar = join(out, "baseline.tar");
  if (run("git", ["-C", SELF, "archive", "-o", tar, sha]).status !== 0 || run("tar", ["-xf", tar, "-C", dir]).status !== 0) fail(`could not export ${sha} to ${dir}`);
  rmSync(tar);
  mkdirSync(join(dir, ".rerail"), { recursive: true });
  writeFileSync(join(dir, SOURCE_FILE), JSON.stringify({ sha, ref }));
  // The pack data: the archives the locks name, from the cache a pull keeps, verified and extracted by main's own pull.
  const cache = packDataCache ?? join(dirname(run("git", ["-C", SELF, "rev-parse", "--path-format=absolute", "--git-common-dir"]).stdout.trim()), ".rerail", "pack-data", "archives");
  mkdirSync(join(dir, ".rerail", "pack-data"), { recursive: true });
  if (existsSync(cache)) symlinkSync(cache, join(dir, ".rerail", "pack-data", "archives"));
  console.log(`shipbar: baseline ${sha} exported to ${dir}; pulling its pack data`);
  const pull = spawnSync("node", ["scripts/s3/pack-fixtures.mjs", "pull"], { cwd: dir, stdio: "inherit" });
  if (pull.status !== 0) fail(`the baseline's pack data pull failed; run \`node scripts/s3/pack-fixtures.mjs pull\` in ${dir}`);
  return { dir, sha, ref };
}

/** <baseline>/.rerail/shipbar-release.json: the published package whose built parts a release baseline holds. */
const RELEASE_FILE = ".rerail/shipbar-release.json";

/**
 * The baseline as users have it: release:<version>. Its source at the tag v<version> (the conformance runner and the
 * packs the product path reads, exported as a git ref is), with the published @neralabs/rerail@<version>'s prebuilt
 * engines for every platform (native/*.node) and its built JavaScript (dist/) put in place of a build: the baseline
 * then needs `npm ci` and nothing built, on any host.
 */
function resolveRelease(version: string, out: string, packDataCache: string | undefined): Checkout {
  const tag = `v${version}`;
  let parsed = run("git", ["-C", SELF, "rev-parse", "--verify", `${tag}^{commit}`]);
  if (parsed.status !== 0) {
    run("git", ["-C", SELF, "fetch", "--quiet", "origin", "tag", tag, "--no-tags"]);
    parsed = run("git", ["-C", SELF, "rev-parse", "--verify", `${tag}^{commit}`]);
  }
  if (parsed.status !== 0) fail(`release ${version}: no tag ${tag} in ${SELF}, and fetching it from origin failed`);
  const checkout = exportCommit(parsed.stdout.trim(), `release:${version}`, out, packDataCache);
  if (existsSync(join(checkout.dir, RELEASE_FILE))) return checkout;
  const name = `@neralabs/rerail@${version}`;
  const packed = run("npm", ["pack", name, "--silent", "--pack-destination", out]);
  const tarball = join(out, lastLine(packed.stdout));
  if (packed.status !== 0 || !existsSync(tarball)) fail(`npm pack ${name} failed: ${lastLine(packed.stderr)}`);
  const unpacked = join(out, "release-package");
  rmSync(unpacked, { recursive: true, force: true });
  mkdirSync(unpacked, { recursive: true });
  if (run("tar", ["-xzf", tarball, "-C", unpacked]).status !== 0) fail(`could not extract ${tarball}`);
  const addons = readdirSync(join(unpacked, "package", "native")).filter((file) => /^rerail_engine\..+\.node$/.test(file));
  if (addons.length === 0) fail(`${name} ships no prebuilt engine (native/rerail_engine.*.node)`);
  for (const addon of addons) copyFileSync(join(unpacked, "package", "native", addon), join(checkout.dir, "native", addon));
  cpSync(join(unpacked, "package", "dist"), join(checkout.dir, "dist"), { recursive: true });
  writeFileSync(join(checkout.dir, RELEASE_FILE), JSON.stringify({ package: name, tag, addons }, null, 1));
  rmSync(unpacked, { recursive: true, force: true });
  rmSync(tarball, { force: true });
  console.log(`shipbar: baseline ${name}: ${tag}'s source with the package's ${addons.length} prebuilt engines and dist/`);
  return checkout;
}

function shippedRules(checkout: string, pack: string): string[] {
  const directory = join(checkout, "packs", pack, "rules");
  return existsSync(directory) ? readdirSync(directory).filter((name) => name.endsWith(".json")).map((name) => name.slice(0, -".json".length)).sort() : [];
}

/** Main's packs (every packs/<id>/ that ships a rule), and the candidate's packs main has none of. */
function discoverPacks(baseline: string, candidate: string): { packs: string[]; candidateOnly: string[] } {
  const dirs = (checkout: string): string[] =>
    readdirSync(join(checkout, "packs"), { withFileTypes: true })
      .filter((entry) => entry.isDirectory() && existsSync(join(checkout, "packs", entry.name, "pack.json")))
      .map((entry) => entry.name)
      .sort();
  const packs = dirs(baseline).filter((pack) => shippedRules(baseline, pack).length > 0);
  return { packs, candidateOnly: dirs(candidate).filter((pack) => !packs.includes(pack)) };
}

interface ConformanceRule {
  id: string;
  upstream: { tool: string; rule: string };
  fixtures?: string[];
}

function conformance(checkout: string, pack: string): { rules: ConformanceRule[]; oracles?: string } | undefined {
  const path = join(checkout, "packs", pack, "conformance.json");
  return existsSync(path) ? readJson(path) : undefined;
}

// ---------------------------------------------------------------- corpora

interface Manifest {
  format: 1;
  corpora: Record<string, ({ kind: "git"; commit: string; paths: string[] } | { kind: "samples"; pack: string; exclude: string[] }) & { files: number | null; sha256: string | null }>;
  packs: Record<string, string>;
}

/** <corpora>/corpora.lock.json: what each corpus directory holds, checked again before a run reads it. */
interface CorporaLock {
  baseline: string;
  fixtures: Record<string, { files: number; sha256: string } | { absent: string }>;
  real: Record<string, { files: number; sha256: string }>;
}

/** The recording a fixtures corpus was cut from: per tool, its original files and its recorded findings on them. */
type Expected = Record<string, { original: string[]; findings: { path: string; line: number; rule: string }[] }>;

interface ExpectedFile {
  source: { files: (string | string[])[]; units?: Record<string, { renamedFrom?: string | null }> };
  findings: { path: string; line: number; rule: string }[];
}

/**
 * A pack's fixtures corpus: each tool's recorded original units (rename variants left out, as main scores them apart)
 * at their recorded paths under <tool>/, for every tool a shipped rule ports, and the recording beside it.
 */
function buildFixtures(baseline: string, pack: string, destination: string): { files: number; sha256: string } | { absent: string } {
  const conf = conformance(baseline, pack);
  if (conf === undefined) return { absent: "the pack has no upstream recording (no conformance.json)" };
  const shipped = new Set(shippedRules(baseline, pack));
  const tools = [...new Set(conf.rules.filter((rule) => shipped.has(rule.id)).map((rule) => rule.upstream.tool))].sort();
  if (tools.length === 0) return { absent: "no shipped rule has an upstream recording" };
  const oracles = join(baseline, "packs", pack, conf.oracles ?? "upstream");
  rmSync(destination, { recursive: true, force: true });
  const expected: Expected = {};
  for (const tool of tools) {
    const path = join(oracles, tool, "fixtures.expected.json");
    if (!existsSync(path)) fail(`${path} is missing: run \`node scripts/s3/pack-fixtures.mjs pull\` in ${baseline}`);
    const recording = readJson<ExpectedFile>(path);
    const original = new Set<string>();
    for (const unit of recording.source.files) {
      const paths = typeof unit === "string" ? [unit] : unit;
      const origin = recording.source.units?.[paths[0]!];
      if (origin?.renamedFrom !== undefined && origin.renamedFrom !== null) continue;
      for (const file of paths) original.add(file);
    }
    for (const file of original) {
      const source = join(oracles, tool, "fixtures", file);
      if (!existsSync(source)) fail(`${source} is missing: run \`node scripts/s3/pack-fixtures.mjs pull\` in ${baseline}`);
      copyInto(source, join(destination, "files", tool, file));
    }
    expected[tool] = { original: [...original].sort(), findings: recording.findings.filter((f) => original.has(f.path)).map((f) => ({ path: f.path, line: f.line, rule: f.rule })) };
  }
  writeFileSync(join(destination, "expected.json"), JSON.stringify(expected));
  return hashTree(join(destination, "files"));
}

/** A real-code corpus as the manifest pins it: a commit's paths of this repository, or a pack's locked samples. */
function buildReal(baseline: string, name: string, manifest: Manifest, destination: string): { files: number; sha256: string } {
  const spec = manifest.corpora[name];
  if (spec === undefined) fail(`corpora.json names no corpus ${name}`);
  rmSync(destination, { recursive: true, force: true });
  mkdirSync(destination, { recursive: true });
  if (spec.kind === "git") {
    const tar = `${destination}.tar`;
    const archive = run("git", ["-C", SELF, "archive", "-o", tar, spec.commit, "--", ...spec.paths]);
    if (archive.status !== 0) fail(`git archive ${spec.commit} for ${name}: ${lastLine(archive.stderr)}`);
    if (run("tar", ["-xf", tar, "-C", destination]).status !== 0) fail(`could not extract ${tar}`);
    rmSync(tar);
  } else {
    const upstream = join(baseline, "packs", spec.pack, "upstream");
    const tools = readdirSync(upstream).filter((tool) => existsSync(join(upstream, tool, "samples.lock"))).sort();
    if (tools.length === 0) fail(`${spec.pack} has no samples.lock under ${upstream}`);
    const exclude = new Set(spec.exclude);
    for (const tool of tools) {
      const digest = readJson<{ sha256: string }>(join(upstream, tool, "samples.lock")).sha256;
      const tree = join(baseline, ".rerail", "pack-data", "samples", spec.pack, tool, digest);
      if (!existsSync(tree)) fail(`${tree} is missing: run \`node scripts/s3/pack-fixtures.mjs samples --pack ${spec.pack}\` in ${baseline}`);
      for (const file of listFiles(tree)) {
        if (file === "LICENSES.md" || exclude.has(`${tool}/${file}`)) continue;
        copyInto(join(tree, file), join(destination, tool, file));
      }
    }
  }
  return hashTree(destination);
}

function prepareCorpora(baseline: Checkout, packs: readonly string[], corpora: string, record: boolean): CorporaLock {
  const manifest = readJson<Manifest>(MANIFEST);
  const lockPath = join(corpora, "corpora.lock.json");
  const lock: CorporaLock = existsSync(lockPath) ? readJson(lockPath) : { baseline: baseline.sha, fixtures: {}, real: {} };
  if (lock.baseline !== baseline.sha) fail(`${corpora} was prepared from baseline ${lock.baseline}, not ${baseline.sha}; use another --corpora`);
  for (const pack of packs) {
    if (lock.fixtures[pack] === undefined) {
      lock.fixtures[pack] = buildFixtures(baseline.dir, pack, recordedCorpus(corpora, pack));
      console.log(`shipbar: fixtures ${pack}: ${JSON.stringify(lock.fixtures[pack])}`);
    }
    const name = manifest.packs[pack];
    if (name === undefined) fail(`scripts/ship/corpora.json declares no real-code corpus for pack ${pack}`);
    if (lock.real[name] === undefined) {
      const built = buildReal(baseline.dir, name, manifest, join(corpora, "real", name));
      const pinned = manifest.corpora[name]!;
      if (pinned.sha256 === null || pinned.files === null || record) {
        if (!record) fail(`corpus ${name} is not pinned yet (${built.files} files, ${built.sha256}); pin it with --record`);
        pinned.files = built.files;
        pinned.sha256 = built.sha256;
        writeFileSync(MANIFEST, `${JSON.stringify(manifest, null, 2)}\n`);
      } else if (pinned.sha256 !== built.sha256 || pinned.files !== built.files) {
        fail(`corpus ${name} is ${built.files} files, ${built.sha256}; corpora.json pins ${pinned.files} files, ${pinned.sha256}`);
      }
      lock.real[name] = built;
      console.log(`shipbar: real ${name}: ${built.files} files`);
    }
    writeFileSync(lockPath, JSON.stringify(lock, null, 1));
  }
  return lock;
}

/** Each corpus a run reads, checked byte for byte against the lock `prepare` wrote. */
function verifyCorpora(corpora: string, packs: readonly string[], corpusKinds: readonly Corpus[]): CorporaLock {
  const lockPath = join(corpora, "corpora.lock.json");
  if (!existsSync(lockPath)) fail(`${lockPath} is missing: run \`shipbar.ts prepare\` first`);
  const lock = readJson<CorporaLock>(lockPath);
  const manifest = readJson<Manifest>(MANIFEST);
  for (const pack of packs) {
    const fixtures = lock.fixtures[pack];
    if (corpusKinds.includes("fixtures")) {
      if (fixtures === undefined) fail(`${corpora} has no fixtures of ${pack}: run prepare with this pack`);
      if (!("absent" in fixtures)) {
        const now = hashTree(join(recordedCorpus(corpora, pack), "files"));
        if (now.sha256 !== fixtures.sha256) fail(`fixtures of ${pack} changed since prepare`);
      }
    }
    const name = manifest.packs[pack];
    if (corpusKinds.includes("real")) {
      if (name === undefined || lock.real[name] === undefined) fail(`${corpora} has no real-code corpus of ${pack}: run prepare with this pack`);
      const now = hashTree(join(corpora, "real", name));
      if (now.sha256 !== lock.real[name].sha256 || now.sha256 !== manifest.corpora[name]?.sha256) fail(`real-code corpus ${name} does not match corpora.json`);
    }
  }
  return lock;
}

/** A pack's fixtures corpus and its recording, under <corpora>/recorded/<pack>/. */
function recordedCorpus(corpora: string, pack: string): string {
  return join(corpora, "recorded", pack);
}

function corpusDirectory(corpora: string, pack: string, corpus: Corpus): string {
  return corpus === "fixtures" ? join(recordedCorpus(corpora, pack), "files") : join(corpora, "real", readJson<Manifest>(MANIFEST).packs[pack]!);
}

// ---------------------------------------------------------------- the machine

/** This machine's way to pin, cap and measure a run (platform.ts), or why it has none. */
function requirePlatform(out: string): Platform {
  try {
    return openPlatform(join(out, "platform"));
  } catch (error) {
    return fail(error instanceof Error ? error.message : String(error));
  }
}

// ---------------------------------------------------------------- builds

function requireBuilt(checkout: string, needs: { product: boolean; engine: boolean }, build: boolean): { engine: string | undefined } {
  const addon = (): boolean => existsSync(join(checkout, "native")) && readdirSync(join(checkout, "native")).some((name) => /^rerail_engine\..+\.node$/.test(name));
  const tsx = (): boolean => existsSync(join(checkout, "node_modules", "tsx"));
  const steps: string[] = [];
  // A release baseline holds its prebuilt engine: installing its dependencies is all it needs.
  if (needs.product && !tsx()) steps.push(NPM_INSTALL);
  if (needs.product && !addon()) steps.push(NATIVE_BUILD);
  if (needs.engine && engineBinary(checkout) === undefined) steps.push(ENGINE_BUILD);
  if (steps.length > 0 && !build) fail(`${checkout} is not built; run there (both release profile): ${steps.join(" && ")}\n  or pass --build`);
  for (const step of steps) {
    console.log(`shipbar: building ${checkout}: ${step}`);
    // The platform's shell (cmd.exe on Windows, /bin/sh elsewhere): Windows hosts have no bash on PATH.
    if (spawnSync(step, { cwd: checkout, stdio: "inherit", shell: true }).status !== 0) fail(`build failed in ${checkout}: ${step}`);
  }
  return { engine: needs.engine ? engineBinary(checkout) : undefined };
}

// ---------------------------------------------------------------- the .rerail side's rules

interface RrRules {
  /**
   * Per rule main ships: its `.rerail` translation's files and policy name (the rule id), or why it cannot run. A rule
   * whose clauses span languages is several full policies of one name, one a language (`<id>.rerail`, `<id>.<lang>.rerail`).
   */
  rules: Record<string, { paths?: string[]; policy?: string; unavailable: string | null }>;
}

/**
 * Each rule main ships, mapped to its `.rerail` translation on the candidate, before `check-rr` (what a plan can know):
 * every `.rerail` of docs/rule-syntax/packs/<pack>/ whose `.policy` line names the rule id. A file with no `.policy`
 * line is a table others #include, never run alone.
 */
function rrRewrites(candidate: string, mainRules: readonly string[], pack: string): RrRules {
  const directory = join(candidate, "docs", "rule-syntax", "packs", pack);
  const byPolicy = new Map<string, string[]>();
  for (const name of existsSync(directory) ? readdirSync(directory).filter((file) => file.endsWith(".rerail")).sort() : []) {
    const policy = /^\.policy\s+"([^"]+)"/m.exec(readFileSync(join(directory, name), "utf8"))?.[1];
    if (policy !== undefined) byPolicy.set(policy, [...(byPolicy.get(policy) ?? []), join(directory, name)]);
  }
  const rules: RrRules["rules"] = {};
  for (const rule of mainRules) {
    const paths = byPolicy.get(rule);
    if (paths !== undefined) rules[rule] = { paths, policy: rule, unavailable: null };
    else rules[rule] = { unavailable: existsSync(join(directory, `${rule}.rerail`)) ? `${rule}.rerail declares no .policy "${rule}"` : "no .rerail counterpart" };
  }
  return { rules };
}

/** Each rule main ships, mapped to its `.rerail` translation on the candidate, every file checked by the candidate's `check-rr`. */
function rrRules(candidate: string, engine: string, mainRules: readonly string[], pack: string): RrRules {
  const { rules } = rrRewrites(candidate, mainRules, pack);
  const present = Object.keys(rules).filter((rule) => rules[rule]!.unavailable === null);
  if (present.length > 0) {
    const checked = run(engine, ["check-rr", ...present.flatMap((rule) => rules[rule]!.paths!)], { cwd: candidate });
    let rows: { path: string; status: string; error?: string }[] = [];
    try {
      rows = engineJson<typeof rows>(checked.stdout);
    } catch {
      fail(`check-rr gave no report for ${pack}: ${lastLine(checked.stderr)}`);
    }
    const byPath = new Map(rows.map((row) => [resolve(candidate, row.path), row]));
    for (const rule of present) {
      for (const path of rules[rule]!.paths!) {
        const row = byPath.get(resolve(path));
        const file = relative(candidate, path);
        if (row === undefined) rules[rule]!.unavailable = `check-rr did not report ${file}`;
        else if (row.status !== "reader-passed") rules[rule]!.unavailable = `check-rr ${row.status} on ${file}: ${(row.error ?? "").slice(0, 200)}`;
        if (rules[rule]!.unavailable !== null) break;
      }
    }
  }
  return { rules };
}

/**
 * runs/<pack>/rr-rules.json: the pack's `.rerail` rules as the candidate's `check-rr` finds them, made once (by
 * whichever process measures the pack first: a fan-out's workers may race, so it is written whole, then renamed).
 */
function packRrRules(settings: Settings, pack: string): RrRules {
  const path = join(settings.out, "runs", pack, "rr-rules.json");
  if (!existsSync(path)) {
    if (settings.engine === undefined) fail(`${pack}: the .rerail side needs the candidate's rerail-engine (${ENGINE_BUILD})`);
    mkdirSync(dirname(path), { recursive: true });
    const temporary = `${path}.${process.pid}`;
    writeFileSync(temporary, JSON.stringify(rrRules(settings.candidate.dir, settings.engine, shippedRules(settings.baseline.dir, pack), pack), null, 1));
    renameSync(temporary, path);
  }
  return readJson<RrRules>(path);
}

/** The `.rerail` rules that run: every rewrite `check-rr` reads. */
function rrRunnable(rr: RrRules): string[] {
  return Object.entries(rr.rules).filter(([, rule]) => rule.unavailable === null).map(([id]) => id);
}

/**
 * Slice B's empty rule set on the `.rerail` side, generated into the cell: run-policies takes at least one policy and a
 * policy at least one `.violation`, so a policy declaring one and no rule, in the language most of the pack's
 * rewrites name (`.language`) and the scope most of those name (`.scope`), so it is given the files they are.
 */
function rrEmptyPolicy(cell: string, rr: RrRules): string {
  const path = join(cell, "one-rule.empty.rerail");
  if (existsSync(path)) return path;
  const heads = rrRunnable(rr).flatMap((id) =>
    rr.rules[id]!.paths!.map((file) => {
      const text = readFileSync(file, "utf8");
      return { language: /^\.language\s+(\S+)/m.exec(text)?.[1], scope: /^\.scope\s.*$/m.exec(text)?.[0] };
    }),
  );
  const mostCommon = (values: readonly (string | undefined)[]): string | undefined => {
    const counts = new Map<string, number>();
    for (const value of values) if (value !== undefined) counts.set(value, (counts.get(value) ?? 0) + 1);
    return [...counts].sort((a, b) => b[1] - a[1] || (a[0] < b[0] ? -1 : 1))[0]?.[0];
  };
  const language = mostCommon(heads.map((head) => head.language));
  if (language === undefined) fail(`${cell}: no runnable .rerail rewrite names its .language`);
  const scope = mostCommon(heads.filter((head) => head.language === language).map((head) => head.scope)) ?? '.scope include "**/*"';
  mkdirSync(cell, { recursive: true });
  const temporary = `${path}.${process.pid}`;
  writeFileSync(temporary, `.policy "shipbar-empty-rule-set"\n.description "No rules: the fixed cost of a run-policies run."\n.language ${language}\n.mode steer\n${scope}\n.violation shipbar_empty_rule_set(at: loc) code "shipbar-empty-rule-set" "never found"\n`);
  renameSync(temporary, path);
  return path;
}

// ---------------------------------------------------------------- measuring

interface RunRow {
  tag: string;
  side: Side;
  mode: "cold" | "warm";
  rep: number;
  completed: boolean;
  reason: string | null;
  /** Where a failed run stopped: the phase (product path) or the policy it had started (run-policies). */
  stoppedOn: string | null;
  /** run-policies: the rule a failed run stopped on, when the engine names it or it was the last one started. */
  culprit: string | null;
  /** run-policies: how many rules the run was given. */
  rules: number | null;
  /** run-policies, failed: policies started of the run's, from its out/<k> directories. */
  progress: { started: number; of: number } | null;
  /** run-policies, completed: how its policies ran, from the result's `runs[].together` (null: the engine does not say). */
  grouping: { policies: number; combined: number; alone: number; largest: number } | null;
  /** run-policies: what the engine printed about running policies alone instead of together (fallback), if anything. */
  fallback: string[];
  /** A chunked run's chunk index (chunks.jsonl only). */
  chunk?: number;
  exit: number;
  wallMs: number;
  cpuMs: number;
  rssKB: number;
  /** The one-minute load average as the run started; null where the platform keeps none (Windows). */
  load: number | null;
  /** The busiest pinned CPU and the busiest idle sibling over the run (fraction busy); null where runs are not pinned. */
  pinnedBusy: number | null;
  siblingBusy: number | null;
  /** How the run was pinned, capped and measured (rows from before it was recorded: Linux's). */
  method: RunMethod;
  cpus: string;
  threads: number;
  host: string;
  memCap: string;
  timeCapS: number;
  command: string;
  started: string;
  /** The product path's in-process evaluation time (product-run.ts evaluateMs); null on the `.rerail` side or a failure. */
  evaluateMs: number | null;
  /** product-run.ts's load, compile and evaluate times (metric.ts); null on the `.rerail` side or a failure. */
  phases?: Phases | null;
  /** Slice B (one-rule*.jsonl): the rule this run timed alone, "(none)" for the empty rule set; null elsewhere. */
  rule: string | null;
}

interface Settings {
  baseline: Checkout;
  candidate: Checkout;
  engine: string | undefined;
  corpora: string;
  out: string;
  runs: number;
  cpus: number[];
  siblings: number[];
  platform: Platform;
  /** The default caps, and per pack overrides (`--pack-cap python=56G,14400`): both sides of a pack run under the same. */
  memCap: string;
  timeCapS: number;
  packCaps: Record<string, { memCap: string; timeCapS: number }>;
  /** Confirmation repetitions per side of anything over the bar: an A cell, a B rule, a C file (0: none). */
  confirm: number;
  /** Slice B: cold runs per side of each rule alone (at least 2). */
  ruleRuns: number;
  /** Slice C: cold-then-warm repetitions per side of each file. */
  fileRuns: number;
  /** Rules per chunk when a `.rerail` pack run does not complete (0: no chunked runs). */
  rrChunk: number;
  /** Where the runs' stores go (`--stores`, say a tmpfs), mirroring runs/; unset: in each cell. */
  stores: string | undefined;
}

/**
 * A cell's rows. Several workers may append to one cell's logs (a fan-out's repetitions and rule chunks): a last line
 * still being written, with no newline yet, is not read.
 */
function readRows(path: string): RunRow[] {
  if (!existsSync(path)) return [];
  const lines = readFileSync(path, "utf8").split("\n");
  return lines.slice(0, -1).filter((line) => line !== "").map((line) => ({ method: LINUX_METHOD, ...(JSON.parse(line) as Omit<RunRow, "method">) }));
}

/** A run's store directory: in its cell, or at the same place under --stores. */
function storeDir(settings: Settings, cell: string, name: string): string {
  return settings.stores === undefined ? join(cell, name) : join(settings.stores, relative(settings.out, cell), name);
}

/** <cell>/rr-excluded.json: the `.rerail` rules that failed alone on this corpus, each with why, left out of its runs. */
function readExcluded(cell: string): Record<string, string> {
  const path = join(cell, "rr-excluded.json");
  return existsSync(path) ? readJson(path) : {};
}

/** The `.rerail` rules a cell runs: every runnable rewrite of a rule main ships, less those that failed alone here. */
function rrIds(rr: RrRules, cell: string): string[] {
  const excluded = readExcluded(cell);
  return Object.entries(rr.rules).filter(([id, rule]) => rule.unavailable === null && excluded[id] === undefined).map(([id]) => id);
}

/** The side's command and its working directory; `rulesDir` replaces the pack's rules for a run of one rule. */
function sideCommand(settings: Settings, side: Side, pack: string, corpus: string, result: string, rundir: string, rrPaths: readonly string[], rulesDir: string | undefined): { command: string[]; cwd: string } {
  if (side === "rr") return { command: [settings.engine!, "run-policies", corpus, rundir, ...rrPaths], cwd: settings.candidate.dir };
  const checkout = side === "main" ? settings.baseline.dir : settings.candidate.dir;
  return { command: ["node", "--import", "tsx", join(HERE, "product-run.ts"), checkout, corpus, result, rulesDir ?? join(checkout, "packs", pack, "rules")], cwd: checkout };
}

interface Process {
  side: Side;
  mode: "cold" | "warm";
  rep: number;
  tag: string;
  /** The store directory: emptied for a cold run, reused by the warm run after it. */
  store: string;
  /**
   * The cell file its row goes to: runs.jsonl, confirm.jsonl, fixed.jsonl, one-rule.jsonl, one-rule-confirm.jsonl,
   * alone.jsonl for a `.rerail` rule alone, or chunks.jsonl for a chunk.
   */
  log: string;
  /** run-policies: the rule ids it runs, in order. */
  rules: readonly string[];
  /** run-policies: the policy files to run instead of `rules`' rewrites (slice B's empty rule set). */
  policies?: readonly string[];
  /** The product path: a directory holding the rules to run instead of the pack's. */
  rulesDir?: string;
  /** Slice B: the rule this run times alone, "(none)" for the empty rule set. */
  rule?: string;
  /** Where in the cell a completed run keeps its result (its findings), when not the first cold run's default. */
  findingsAs?: string;
  /** A chunked run's chunk index. */
  chunk?: number;
}

/** The caps a pack's runs take, both sides alike: its `--pack-cap`, else the defaults. */
function capsOf(settings: Settings, pack: string): { memCap: string; timeCapS: number } {
  return settings.packCaps[pack] ?? { memCap: settings.memCap, timeCapS: settings.timeCapS };
}

/** One process of one side, pinned, capped and measured as the platform can; its row is appended to the cell's log. */
function measure(settings: Settings, cell: string, p: Process, pack: string, corpus: string, rr: RrRules | undefined): RunRow {
  const { side, mode, rep, tag, store } = p;
  const caps = capsOf(settings, pack);
  if (mode === "cold") {
    rmSync(store, { recursive: true, force: true });
    mkdirSync(join(store, "tmp"), { recursive: true });
  }
  const result = join(cell, `${tag}.json`);
  const rundir = join(cell, `${tag}.out`);
  rmSync(rundir, { recursive: true, force: true });
  // run-policies: each rule's files in order (a rule of several languages is several policies), and the rule of each.
  const policyRules = p.policies === undefined ? p.rules.flatMap((id) => rr!.rules[id]!.paths!.map((path) => ({ path, id }))) : p.policies.map((path) => ({ path, id: null }));
  const { command, cwd } = sideCommand(settings, side, pack, corpus, result, rundir, policyRules.map((policy) => policy.path), p.rulesDir);
  const env: Record<string, string> = {};
  for (const [key, value] of Object.entries(process.env)) if (value !== undefined && !key.startsWith("RERAIL_") && !key.startsWith("SHIPBAR_")) env[key] = value;
  env.RERAIL_TAPE_CACHE = join(store, "store.sqlite");
  // Windows's programs take their temporary directory from TMP and TEMP.
  if (side === "rr") for (const key of process.platform === "win32" ? ["TMPDIR", "TMP", "TEMP"] : ["TMPDIR"]) env[key] = join(store, "tmp");
  else env.SHIPBAR_ENGINE_TMPDIR = join(store, "tmp");
  // The engine's thread count: what taskset already gives it on Linux, and what pins nothing on macOS.
  env.RERAIL_THREADS = String(settings.cpus.length);
  const stderrFile = join(cell, `${tag}.stderr`);
  const started = new Date().toISOString();
  const stdout = openSync(side === "rr" ? result : join(cell, `${tag}.stdout`), "w");
  const stderr = openSync(stderrFile, "w");
  const measured = settings.platform.measure({ command, cwd, env, cpus: settings.cpus, siblings: settings.siblings, memCap: caps.memCap, timeCapS: caps.timeCapS, stdout, stderr, scratch: join(cell, tag) });
  closeSync(stdout);
  closeSync(stderr);
  const { exit, wallS: wall, peakKB: rss } = measured;
  const cpuS = measured.userS + measured.sysS;
  const errorText = readFileSync(stderrFile, "utf8");
  let completed = exit === 0;
  let reason: string | null = null;
  let stoppedOn: string | null = null;
  let evaluateMs: number | null = null;
  let phases: Phases | null = null;
  let grouped: RunRow["grouping"] = null;
  if (completed) {
    try {
      // The `.rerail` side's result is the engine's stdout: its JSON, then its timing lines (engine-json.ts).
      const output = engineJson<{ evaluateMs?: number; loadMs?: number; compileMs?: number; runs?: { together?: number }[] }>(readFileSync(result, "utf8"));
      if (side !== "rr" && typeof output.evaluateMs === "number") evaluateMs = Math.round(output.evaluateMs);
      if (side !== "rr") phases = readPhases(output);
      if (side === "rr") grouped = grouping(output.runs ?? []);
    } catch {
      completed = false;
      reason = "exited 0 with no readable result";
    }
  }
  // What the engine says about running policies alone instead of together (its fallback), kept from either outcome.
  const fallback = side === "rr" ? errorText.split("\n").map((line) => line.trim()).filter((line) => /fall(s|ing)? ?back|\balone\b/i.test(line)).slice(0, 5).map((line) => line.slice(0, 300)) : [];
  let progress: RunRow["progress"] = null;
  let culprit: string | null = null;
  if (exit !== 0) {
    reason =
      exit === 137 || exit === 128 + 9
        ? `killed at ${wall.toFixed(0)} s, ${(rss / 1048576).toFixed(1)} GiB peak memory (${(measured.killedBy ?? (wall >= caps.timeCapS - 1 ? "time" : "memory")) === "time" ? `time cap ${caps.timeCapS} s` : `memory cap ${caps.memCap}`})`
        : `exit ${exit}: ${lastLine(errorText)}`;
    if (side === "rr") {
      const started = existsSync(rundir) ? readdirSync(rundir).filter((name) => /^\d+$/.test(name)).length : 0;
      const named = /([^\s"']+\.rerail): /.exec(errorText)?.[1];
      const namedRule = named === undefined ? undefined : policyRules.find((policy) => resolve(cwd, named) === resolve(policy.path))?.id;
      culprit = namedRule ?? policyRules[started - 1]?.id ?? null;
      stoppedOn = `${culprit ?? "before the first policy"} (policy ${started} of ${policyRules.length})`;
      progress = { started, of: policyRules.length };
    } else {
      stoppedOn = [...errorText.matchAll(/^shipbar: phase (\S+)/gm)].at(-1)?.[1] ?? "start-up";
    }
  }
  const row: RunRow = {
    tag, side, mode, rep, completed, reason, stoppedOn, culprit, rules: side === "rr" ? p.rules.length : null, progress, grouping: grouped, fallback, ...(p.chunk === undefined ? {} : { chunk: p.chunk }), exit,
    wallMs: Math.round(wall * 1000), cpuMs: Math.round(cpuS * 1000), rssKB: rss,
    load: measured.load, pinnedBusy: measured.pinnedBusy, siblingBusy: measured.siblingBusy, method: settings.platform.method,
    cpus: settings.cpus.join(","), threads: settings.cpus.length, host: hostname(),
    memCap: caps.memCap, timeCapS: caps.timeCapS, command: command.join(" "), started, evaluateMs, phases, rule: p.rule ?? null,
  };
  appendFileSync(join(cell, p.log), `${JSON.stringify(row)}\n`);
  // Kept: the first cold run's findings, a slice B run's where asked (and every failure's stderr). The rest is scratch.
  rmSync(rundir, { recursive: true, force: true });
  rmSync(join(cell, `${tag}.stdout`), { force: true });
  if (tag === `${side}.cold1` && p.log === "runs.jsonl" && completed) renameSync(result, join(cell, `${side}.findings.json`));
  else if (p.findingsAs !== undefined && completed) renameSync(result, join(cell, p.findingsAs));
  else if (p.chunk !== undefined && mode === "cold" && rep === 1 && completed) renameSync(result, join(cell, `rr.chunk${p.chunk}.findings.json`));
  else rmSync(result, { force: true });
  if (completed) rmSync(stderrFile, { force: true });
  console.log(`shipbar: ${pack} ${relative(settings.corpora, corpus)} ${tag}: ${completed ? "ok" : `FAILED (${reason})`} wall ${wall}s cpu ${cpuS.toFixed(2)}s rss ${(rss / 1024).toFixed(0)}MB`);
  return row;
}

/**
 * After a `.rerail` cold run that exits with an error, the rule it stopped on is run alone on the same corpus under the
 * same caps. Failing alone too, it is left out of this corpus's runs with that reason (rr-excluded.json), and the
 * caller runs the rest again; running alone, the failure belongs to running the pack's rules together and stands. A
 * content store limit is the batch's by nature, and a run the caps killed is not tried again (each try could take the
 * whole time cap): both stand as the pack's failure, with the rule it stopped on.
 */
function excludeIfFailsAlone(settings: Settings, cell: string, pack: string, corpus: string, rr: RrRules, failed: RunRow): boolean {
  if (failed.culprit === null || failed.exit === 137 || /content store hard limit/.test(failed.reason ?? "")) return false;
  const store = storeDir(settings, cell, "rr.alone.store");
  const alone = measure(settings, cell, { side: "rr", mode: "cold", rep: 0, tag: `rr.alone.${failed.culprit}`, store, log: "alone.jsonl", rules: [failed.culprit] }, pack, corpus, rr);
  rmSync(store, { recursive: true, force: true });
  if (alone.completed) return false;
  const excluded = readExcluded(cell);
  excluded[failed.culprit] = `fails alone: ${alone.reason ?? ""}`;
  writeFileSync(join(cell, "rr-excluded.json"), JSON.stringify(excluded, null, 1));
  const kept = readRows(join(cell, "runs.jsonl")).filter((row) => row.tag !== failed.tag);
  writeFileSync(join(cell, "runs.jsonl"), kept.map((row) => `${JSON.stringify(row)}\n`).join(""));
  return true;
}

/**
 * One cell (an A pack and corpus, or a C file): `runs` repetitions, the side order rotating, each side cold then warm on
 * its own fresh store, into <cell>/runs.jsonl.
 */
function measureCell(settings: Settings, cell: string, dir: string, pack: string, reps: readonly number[], sides: readonly Side[], rr: RrRules | undefined): void {
  mkdirSync(cell, { recursive: true });
  for (const rep of reps) {
    const order = sides.map((_, i) => sides[(i + rep - 1) % sides.length]!);
    for (const side of order) {
      const store = storeDir(settings, cell, `${side}.r${rep}.store`);
      const proc = (mode: "cold" | "warm"): Process => ({ side, mode, rep, tag: `${side}.${mode}${rep}`, store, log: "runs.jsonl", rules: side === "rr" ? rrIds(rr!, cell) : [] });
      const done = (): RunRow[] => readRows(join(cell, "runs.jsonl")).filter((row) => row.side === side);
      for (;;) {
        if (done().some((row) => !row.completed) || done().some((row) => row.tag === `${side}.cold${rep}`)) break;
        if (side === "rr" && rrIds(rr!, cell).length === 0) break;
        const cold = measure(settings, cell, proc("cold"), pack, dir, rr);
        if (cold.completed) break;
        rmSync(store, { recursive: true, force: true });
        if (side !== "rr" || !excludeIfFailsAlone(settings, cell, pack, dir, rr!, cold)) break;
      }
      const rows = done();
      if (rows.some((row) => !row.completed) || !rows.some((row) => row.tag === `${side}.cold${rep}`)) continue;
      if (!rows.some((row) => row.tag === `${side}.warm${rep}`)) measure(settings, cell, proc("warm"), pack, dir, rr);
      rmSync(store, { recursive: true, force: true });
    }
  }
  if (sides.includes("rr") && settings.rrChunk > 0 && readRows(join(cell, "runs.jsonl")).some((row) => row.side === "rr" && !row.completed)) {
    measureChunks(settings, cell, pack, dir, rr!);
  }
}

/** <cell>/chunks.json: the `.rerail` rules of a chunked run, chunk by chunk, fixed when the first chunk runs. */
function readChunks(cell: string): string[][] | undefined {
  const path = join(cell, "chunks.json");
  return existsSync(path) ? readJson<string[][]>(path) : undefined;
}

/**
 * A `.rerail` pack run that did not complete still gets a figure: its rules in fixed chunks of --rr-chunk, in order, each
 * chunk its own run-policies process under the same caps, cold (fresh store) then warm, N repetitions (chunks.jsonl).
 * The verdict sums the chunks per repetition and labels the figure chunked. A chunk that fails a cold run is not run
 * again and is listed with its reason; the sums then cover the other chunks' rules only.
 */
function measureChunks(settings: Settings, cell: string, pack: string, corpus: string, rr: RrRules): void {
  let chunks = readChunks(cell);
  if (chunks === undefined) {
    const ids = rrIds(rr, cell);
    chunks = [];
    for (let i = 0; i < ids.length; i += settings.rrChunk) chunks.push(ids.slice(i, i + settings.rrChunk));
    writeFileSync(join(cell, "chunks.json"), JSON.stringify(chunks));
  }
  console.log(`shipbar: ${pack} ${relative(settings.corpora, corpus)}: the .rerail pack run did not complete; ${chunks.length} chunks of up to ${settings.rrChunk} rules`);
  for (let rep = 1; rep <= settings.runs; rep += 1) {
    chunks.forEach((rules, chunk) => {
      const rows = readRows(join(cell, "chunks.jsonl")).filter((row) => row.chunk === chunk);
      if (rows.some((row) => row.mode === "cold" && !row.completed)) return;
      const store = storeDir(settings, cell, `rr.chunk${chunk}.store`);
      const proc = (mode: "cold" | "warm"): Process => ({ side: "rr", mode, rep, tag: `rr.chunk${chunk}.${mode}${rep}`, store, log: "chunks.jsonl", rules, chunk });
      // Cold then warm on its store, as a pair: a repetition resumed after its cold run is not completed with a warm one
      // on a store that is gone.
      if (rows.some((row) => row.tag === `rr.chunk${chunk}.cold${rep}`)) return;
      if (measure(settings, cell, proc("cold"), pack, corpus, rr).completed) measure(settings, cell, proc("warm"), pack, corpus, rr);
      rmSync(store, { recursive: true, force: true });
    });
  }
}

/**
 * Each product side's start-up on this corpus (metric.ts T0): an empty rule set, cold then warm, --runs repetitions
 * interleaved as the cell's, into fixed.jsonl. Reported beside the gross metrics, never judged.
 */
function measureFixed(settings: Settings, pack: string, corpus: Corpus, sides: readonly Side[], reps: readonly number[]): void {
  const cell = join(settings.out, "runs", pack, corpus);
  const dir = corpusDirectory(settings.corpora, pack, corpus);
  const empty = join(settings.out, "empty-rules");
  mkdirSync(empty, { recursive: true });
  const product = sides.filter((side) => side !== "rr");
  for (const rep of reps) {
    for (const side of product.map((_, i) => product[(i + rep - 1) % product.length]!)) {
      const tag = (mode: "cold" | "warm"): string => `fixed.${side}.${mode}${rep}`;
      if (readRows(join(cell, "fixed.jsonl")).some((row) => row.tag === tag("cold"))) continue;
      const store = storeDir(settings, cell, `fixed.${side}.r${rep}.store`);
      const proc = (mode: "cold" | "warm"): Process => ({ side, mode, rep, tag: tag(mode), store, log: "fixed.jsonl", rules: [], rulesDir: empty });
      if (measure(settings, cell, proc("cold"), pack, dir, undefined).completed) measure(settings, cell, proc("warm"), pack, dir, undefined);
      rmSync(store, { recursive: true, force: true });
    }
  }
}

/**
 * A cell (an A pack and corpus, or a C file) whose median fails a metric runs again before the report counts it:
 * --confirm more repetitions of main and of each candidate failing there, interleaved as the first ones, into
 * confirm.jsonl. A metric failing again is confirmed; passing, it is noisy. Only cells where both sides completed every
 * run are confirmed: a cell that did not complete fails on completion, which more runs do not change.
 */
function confirmCell(settings: Settings, cell: string, dir: string, pack: string, candidates: readonly Candidate[], rr: RrRules | undefined): void {
  if (settings.confirm === 0) return;
  const rows = readRows(join(cell, "runs.jsonl"));
  const main = rows.filter((row) => row.side === "main");
  if (main.length === 0 || main.some((row) => !row.completed)) return;
  const failing = candidates.filter((c) => {
    const mine = rows.filter((row) => row.side === c);
    return mine.length > 0 && mine.every((row) => row.completed) && measuredMetrics(main, mine, [], []).some((m) => m.verdict.status === "fail");
  });
  if (failing.length === 0) return;
  const sides: Side[] = ["main", ...failing];
  for (let rep = 1; rep <= settings.confirm; rep += 1) {
    const order = sides.map((_, i) => sides[(i + rep - 1) % sides.length]!);
    for (const side of order) {
      const tag = (mode: "cold" | "warm"): string => `confirm.${side}.${mode}${rep}`;
      // A repetition already made (or started: its cold run logged) is not made again.
      if (readRows(join(cell, "confirm.jsonl")).some((row) => row.tag === tag("cold"))) continue;
      const store = storeDir(settings, cell, `confirm.${side}.r${rep}.store`);
      const proc = (mode: "cold" | "warm"): Process => ({ side, mode, rep, tag: tag(mode), store, log: "confirm.jsonl", rules: side === "rr" ? rrIds(rr!, cell) : [] });
      if (measure(settings, cell, proc("cold"), pack, dir, rr).completed) measure(settings, cell, proc("warm"), pack, dir, rr);
      rmSync(store, { recursive: true, force: true });
    }
  }
}

// ---------------------------------------------------------------- slice B: one rule × all files

/** Slice B's empty rule set: the fixed cost, timed alone as each rule is. */
const NO_RULE = "(none)";
/** Slice B's files in a pack and corpus's cell: its first runs, its confirmation runs and each run's kept findings. */
const ONE_RULE = { runs: "one-rule.jsonl", confirm: "one-rule-confirm.jsonl", findings: "one-rule.findings" } as const;

/**
 * A candidate's slice B rules of a pack: the empty rule set, then every rule main ships that the candidate has (ships:
 * ships too; rr: a rewrite in `rr`, check-rr's runnable ones, or every rewrite where a plan made before check-rr asks).
 */
function oneRuleSet(settings: { baseline: Checkout; candidate: Checkout }, pack: string, candidate: Candidate, rr: RrRules | undefined): string[] {
  const has = new Set(candidate === "ships" ? shippedRules(settings.candidate.dir, pack) : rr === undefined ? [] : rrRunnable(rr));
  return [NO_RULE, ...shippedRules(settings.baseline.dir, pack).filter((rule) => has.has(rule))];
}

/** Slice B's rules of a pack over every candidate measured: the empty rule set, then each rule any candidate has. */
function oneRuleUnion(settings: { baseline: Checkout; candidate: Checkout }, pack: string, candidates: readonly Candidate[], rr: RrRules | undefined): string[] {
  const wanted = new Set(candidates.flatMap((candidate) => oneRuleSet(settings, pack, candidate, rr)));
  return [NO_RULE, ...shippedRules(settings.baseline.dir, pack).filter((rule) => wanted.has(rule))];
}

/** Slice B's rows grouped by the rule they timed. */
function byRule(rows: readonly RunRow[]): Map<string, RunRow[]> {
  const out = new Map<string, RunRow[]>();
  for (const row of rows) {
    if (row.rule === null) continue;
    const list = out.get(row.rule);
    if (list === undefined) out.set(row.rule, [row]);
    else list.push(row);
  }
  return out;
}

/**
 * Slice B's judged times of one rule from its first and confirmation runs, main against `candidate`. ships: its compile
 * + evaluate time; for the empty rule set (the fixed cost) its cold wall and each of its load, compile and evaluate
 * times. rr (run-policies reports no phases): its net wall, the cold wall less `rrEmptyWalls`' median (rr's empty rule
 * set's cold walls), against main's compile + evaluate; for the empty rule set its cold wall.
 */
function oneRuleTimes(rule: string, candidate: Candidate, rows: readonly RunRow[], confirm: readonly RunRow[], rrEmptyWalls: readonly number[]): TimeVerdict[] {
  const values = (from: readonly RunRow[], side: Side, pick: (row: RunRow) => number): number[] => from.filter((row) => row.side === side && row.completed && (side === "rr" || row.phases != null)).map(pick);
  const time = (name: string, pick: (row: RunRow) => number): TimeVerdict => judgeTime(name, values(rows, "main", pick), values(rows, candidate, pick), values(confirm, "main", pick), values(confirm, candidate, pick));
  const wall = (row: RunRow): number => row.wallMs;
  const compileEvaluate = (row: RunRow): number => row.phases!.compile_ms + row.phases!.evaluate_ms;
  if (candidate === "rr") {
    if (rule === NO_RULE) return [time("wall-cold", wall)];
    return [judgeTime(RR_RULE_TIME, values(rows, "main", compileEvaluate), netWalls(values(rows, "rr", wall), rrEmptyWalls), values(confirm, "main", compileEvaluate), netWalls(values(confirm, "rr", wall), rrEmptyWalls))];
  }
  if (rule !== NO_RULE) return [time("compile+evaluate", compileEvaluate)];
  return [time("wall-cold", wall), time("load", (row) => row.phases!.load_ms), time("compile", (row) => row.phases!.compile_ms), time("evaluate", (row) => row.phases!.evaluate_ms)];
}

/** The `.rerail` side's empty rule set's completed cold walls among a cell's slice B rows. */
function rrEmptyWallsOf(rows: ReadonlyMap<string, readonly RunRow[]>): number[] {
  return (rows.get(NO_RULE) ?? []).filter((row) => row.side === "rr" && row.completed).map((row) => row.wallMs);
}

/**
 * Slice B on one pack and corpus: the empty rule set and each rule a candidate has (or `rules` of them: a fan-out's
 * chunk), alone, --rule-runs cold runs per side (main and each candidate that has the rule), the side order rotating,
 * each on a fresh store: the product path with a rules directory holding only that rule, run-policies with only that
 * rule's rewrite (the empty rule set: one-rule.empty.rerail). Into one-rule.jsonl; the first run's findings kept in
 * one-rule.findings/<side>.<rule>.json. A time over the bar by at least the floor runs --confirm more such runs of main
 * and of each candidate over (one-rule-confirm.jsonl). A side whose run of a rule fails makes no further run of it.
 */
function measureOneRule(settings: Settings, pack: string, corpus: Corpus, candidates: readonly Candidate[], rr: RrRules | undefined, rules: readonly string[] = oneRuleUnion(settings, pack, candidates, rr)): void {
  if (rules.length === 0) return;
  const cell = join(settings.out, "runs", pack, corpus);
  const dir = corpusDirectory(settings.corpora, pack, corpus);
  // Each chunk its own scratch: other workers may time other rules of the same cell meanwhile.
  const scratch = join(cell, "one-rule.scratch", rules[0]!);
  const stores = storeDir(settings, cell, join("one-rule.stores", rules[0]!));
  mkdirSync(join(cell, ONE_RULE.findings), { recursive: true });
  const sets = new Map(candidates.map((c) => [c, new Set(oneRuleSet(settings, pack, c, rr))]));
  const emptyPolicy = sets.has("rr") ? rrEmptyPolicy(cell, rr!) : undefined;
  const logged = { [ONE_RULE.runs]: byRule(readRows(join(cell, ONE_RULE.runs))), [ONE_RULE.confirm]: byRule(readRows(join(cell, ONE_RULE.confirm))) };
  const runRule = (rule: string, log: typeof ONE_RULE.runs | typeof ONE_RULE.confirm, reps: number, sides: readonly Side[]): void => {
    const confirming = log === ONE_RULE.confirm;
    for (let rep = 1; rep <= reps; rep += 1) {
      for (const side of sides.map((_, i) => sides[(i + rep - 1) % sides.length]!)) {
        const tag = `${confirming ? "one-confirm" : "one"}.${side}.${rule}.${rep}`;
        const rows = (logged[log].get(rule) ?? []).filter((row) => row.side === side);
        if (rows.some((row) => row.tag === tag || !row.completed)) continue;
        const store = join(stores, `${side}.store`);
        const findingsAs = rep === 1 && !confirming && rule !== NO_RULE ? { findingsAs: join(ONE_RULE.findings, `${side}.${rule}.json`) } : {};
        let proc: Process;
        if (side === "rr") {
          proc = { side, mode: "cold", rep, tag, store, log, rules: rule === NO_RULE ? [] : [rule], ...(rule === NO_RULE ? { policies: [emptyPolicy!] } : {}), rule, ...findingsAs };
        } else {
          const rulesDir = join(scratch, `${side}.rules`);
          rmSync(rulesDir, { recursive: true, force: true });
          mkdirSync(rulesDir, { recursive: true });
          if (rule !== NO_RULE) copyFileSync(join(side === "main" ? settings.baseline.dir : settings.candidate.dir, "packs", pack, "rules", `${rule}.json`), join(rulesDir, `${rule}.json`));
          proc = { side, mode: "cold", rep, tag, store, log, rules: [], rulesDir, rule, ...findingsAs };
        }
        const row = measure(settings, cell, proc, pack, dir, rr);
        const list = logged[log].get(rule);
        if (list === undefined) logged[log].set(rule, [row]);
        else list.push(row);
        rmSync(store, { recursive: true, force: true });
      }
    }
  };
  // A fan-out's chunk without the empty rule set still needs rr's to judge its rules' net walls: timed here first.
  if (!rules.includes(NO_RULE) && rules.some((rule) => sets.get("rr")?.has(rule)) && !(logged[ONE_RULE.runs].get(NO_RULE) ?? []).some((row) => row.side === "rr")) runRule(NO_RULE, ONE_RULE.runs, settings.ruleRuns, ["rr"]);
  for (const rule of rules) {
    const here = candidates.filter((c) => sets.get(c)!.has(rule));
    if (here.length === 0) continue;
    runRule(rule, ONE_RULE.runs, settings.ruleRuns, ["main", ...here]);
    if (settings.confirm === 0) continue;
    const first = logged[ONE_RULE.runs].get(rule) ?? [];
    const confirmed = logged[ONE_RULE.confirm].get(rule) ?? [];
    const over = here.filter((c) => first.every((row) => row.completed || (row.side !== "main" && row.side !== c)) && oneRuleTimes(rule, c, first, confirmed, rrEmptyWallsOf(logged[ONE_RULE.runs])).some((time) => needsConfirmation(time)));
    if (over.length > 0) runRule(rule, ONE_RULE.confirm, settings.confirm, ["main", ...over]);
  }
  rmSync(scratch, { recursive: true, force: true });
  rmSync(stores, { recursive: true, force: true });
}

// ---------------------------------------------------------------- slice C: all rules × one file

interface OneFile {
  label: FileLabel;
  /** The file's path in the pack's real-code corpus. */
  path: string;
  bytes: number;
}

/** runs/<pack>/one-file/files.json: slice C's files of a pack, picked once from its real-code corpus. */
function oneFilesPath(out: string, pack: string): string {
  return join(out, "runs", pack, "one-file", "files.json");
}

function pickOneFiles(settings: { out: string; corpora: string }, pack: string): OneFile[] {
  const path = oneFilesPath(settings.out, pack);
  if (existsSync(path)) return readJson<OneFile[]>(path);
  const root = corpusDirectory(settings.corpora, pack, "real");
  let picked: OneFile[] = [];
  try {
    picked = pickFiles(pack, listFiles(root).map((file) => ({ path: file, bytes: statSync(join(root, file)).size })));
  } catch (error) {
    fail(error instanceof Error ? error.message : String(error));
  }
  if (picked.length === 0) fail(`slice C: the real-code corpus of ${pack} holds no file with its source extensions (${SOURCE_EXTENSIONS[pack]!.join(", ")})`);
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, JSON.stringify(picked, null, 1));
  return picked;
}

/**
 * Slice C on one file of a pack: alone in a corpus directory of its own (runs/<pack>/one-file/<label>/corpus, at its
 * path in the real-code corpus), all of the pack's rules (rr: all of its runnable rewrites), --file-runs repetitions per
 * side, cold then warm on the same store, the side order rotating; anything over the bar confirmed as an A cell is.
 */
function measureOneFile(settings: Settings, pack: string, file: OneFile, candidates: readonly Candidate[], rr: RrRules | undefined): void {
  const cell = join(settings.out, "runs", pack, "one-file", file.label);
  const dir = join(cell, "corpus");
  if (!existsSync(join(dir, file.path))) copyInto(join(corpusDirectory(settings.corpora, pack, "real"), file.path), join(dir, file.path));
  measureCell(settings, cell, dir, pack, repetitions(settings.fileRuns), ["main", ...candidates], rr);
  confirmCell(settings, cell, dir, pack, candidates, rr);
}

/** Repetitions 1..n. */
function repetitions(n: number): number[] {
  return Array.from({ length: n }, (_, i) => i + 1);
}

// ---------------------------------------------------------------- the verdict

/** One metric of a cell: the first runs' verdict and each side's values, and the confirmation runs' when made. */
export interface MeasuredMetric {
  verdict: MetricVerdict;
  mainRuns: number[];
  candidateRuns: number[];
  confirm: { verdict: MetricVerdict; mainRuns: number[]; candidateRuns: number[] } | null;
  /** The phase that moved (verdict.ts locate), for a metric failing at its first runs; absent or null otherwise. */
  locator?: string | null;
}

/** One rule alone on each side: its median time (ms), null where that side's runs failed. */
export interface RuleTiming {
  rule: string;
  main: number | null;
  candidate: number | null;
}

/** A slice B rule's or a slice C file's findings on both sides: the bar is the same set. */
export interface SameFindings {
  both: number;
  mainOnly: number;
  candidateOnly: number;
  samples: string[];
}

/** Slice B: one rule, or the empty rule set (the fixed cost), alone on one corpus. */
export interface OneRuleTiming {
  rule: string;
  /** A rule: its compile + evaluate time; the empty rule set: its cold wall, load, compile and evaluate times. */
  times: TimeVerdict[];
  /** The phase that moved, where a time is over the bar at its first runs; null otherwise. */
  locator: string | null;
  completion: CompletionVerdict;
  /** null where a side kept no findings (the empty rule set; a failed run). */
  findings: SameFindings | null;
  /** The candidate's compile error or engine failure on this rule, where main has none. */
  ruleFailure: string | null;
  /** Each side's median cold wall (ms) and peak RSS (KB) of its first runs: reported, not judged. */
  wallMs: { main: number | null; candidate: number | null };
  rssKB: { main: number | null; candidate: number | null };
}

/** The box while a slice's runs ran: the 1-minute load average at each run's start and the busiest idle sibling CPU. */
export interface LoadSummary {
  runs: number;
  loadMedian: number | null;
  loadMax: number | null;
  siblingBusyMax: number | null;
}

/** Slice B on one corpus: the empty rule set and every rule main ships that the candidate ships, each alone. */
export interface OneRuleVerdict {
  corpus: Corpus;
  /** Why it is not compared ("" when it is): not measured, or the pack has no fixtures corpus. */
  note: string;
  /** Of the rules it should time, those with no run on a side: the slice is incomplete while any remain. */
  unmeasured: string[];
  /** Cold runs per side of each rule (the most any rule made). */
  runs: number;
  fixed: OneRuleTiming | null;
  rules: OneRuleTiming[];
  load: LoadSummary;
}

/** Slice C: all of the pack's rules on one file of its real-code corpus. */
export interface OneFileVerdict {
  label: FileLabel;
  path: string;
  bytes: number;
  /** Why it is not compared ("" when it is). */
  note: string;
  completion: (CompletionVerdict & { stoppedOn: string | null }) | null;
  metrics: MeasuredMetric[];
  findings: SameFindings | null;
  ruleFailures: { rule: string; reason: string }[];
  load: { main: (number | null)[]; candidate: (number | null)[] };
  siblingBusyMax: number | null;
  runs: { main: number; candidate: number };
}

export interface CellVerdict {
  corpus: Corpus;
  /** Why the cell is not compared; "" when it is. */
  note: string;
  /** Why the cell fails without a measurement (the `.rerail` side runs no rule on this corpus); null otherwise. */
  blocked: string | null;
  completion: (CompletionVerdict & { stoppedOn: string | null }) | null;
  ruleFailures: { rule: string; reason: string }[];
  metrics: MeasuredMetric[];
  /** fixtures: the set-wise findings bar, with each side's TP/FP/FN; real: agreement only. */
  findings: (Omit<FindingsVerdict, "lostTp" | "newFp"> & { lostTp: number; newFp: number; samples: string[]; main: Counts; candidate: Counts }) | null;
  agreement: SameFindings | null;
  load: { main: (number | null)[]; candidate: (number | null)[] };
  /** The busiest idle sibling CPU over the cell's runs; null when none of them was pinned. */
  siblingBusyMax: number | null;
  /** How the cell's runs were pinned, capped and measured, each method once. */
  methods: RunMethod[];
  runs: { main: number; candidate: number };
  /** How many rules each side ran: main's shipped rules; the candidate's shipped rules, or the `.rerail` rules that run. */
  rules: { main: number; candidate: number };
  /** metric.ts: start-up, net times, phases and workload; null where the cell was not measured. */
  cost: CellCost | null;
  /** The caps the cell's runs ran under (both sides alike). */
  caps: { memCap: string[]; timeCapS: number[] };
  /** The candidate's whole run did not complete: its metrics and findings are the sums of a chunked run, labelled so. */
  chunked: { size: number; chunks: number; rulesRun: number; rulesOf: number; failedChunks: { chunk: number; rules: string; reason: string }[] } | null;
  /** `.rerail`: how far the failed whole-pack run got (policies started, elapsed, peak RSS). */
  progress: { started: number; of: number; wallMs: number; rssKB: number } | null;
  /** `.rerail`: how run-policies ran the pack's policies (from the completed runs, the chunks' summed), and any fallback it printed. */
  grouping: { policies: number; combined: number; alone: number; largest: number } | null;
  fallback: string[];
}

interface Counts {
  tp: number;
  fp: number;
  fn: number;
  explainedFp: number;
  explainedFn: number;
}

export interface PackVerdict {
  pack: string;
  /** A pack only the candidate ships: nothing to compare it with. */
  noBaseline: boolean;
  missing: { rule: string; reason: string }[];
  extra: string[];
  /** Slice A: all rules × all files, one cell per corpus in scope. */
  cells: CellVerdict[];
  /** Slice B: one rule × all files, one per corpus in scope. */
  oneRule: OneRuleVerdict[];
  /** Slice C: all rules × one file, the three files of the real-code corpus; empty where not measured. */
  oneFile: OneFileVerdict[];
  /**
   * Whether this pack's verdict counts toward the exit status, and why (the `.rerail` side: its equivalence.json,
   * verdict.ts translationGate); null where it always counts (the shipped packs).
   */
  gate: { counted: boolean; reason: string } | null;
}

export interface CandidateVerdict {
  candidate: Candidate;
  description: string;
  packs: PackVerdict[];
}

export interface Verdict {
  tolerancePercent: number;
  runs: number;
  baseline: Checkout;
  candidate: Checkout;
  corpora: CorporaLock;
  /** Each way runs were pinned, capped and measured, with how many runs it measured. */
  machine: { hosts: string[]; cpus: string[]; threads: number[]; memCap: string[]; timeCapS: number[]; methods: { method: RunMethod; runs: number }[] };
  machines: MachineInfo[];
  candidates: CandidateVerdict[];
  gate: Candidate[];
  /** How the run was narrowed (--packs, --corpus): the verdict is then partial, never PASS; empty when it was not. */
  partial: string[];
}

/** A side's findings from its first cold run, as keys: [rule, path, line, code]. */
function sideFindings(cell: string, side: Side, rr: RrRules | undefined, file = `${side}.findings.json`): Set<string> | undefined {
  const path = join(cell, file);
  if (!existsSync(path)) return undefined;
  const keys = new Set<string>();
  if (side === "rr") {
    const policyToRule = new Map(Object.entries(rr?.rules ?? {}).filter(([, rule]) => rule.policy !== undefined).map(([id, rule]) => [rule.policy!, id]));
    for (const d of engineJson<{ diagnostics: { policy: string; path: string; line: number; code: string }[] }>(readFileSync(path, "utf8")).diagnostics) {
      keys.add(findingKey(policyToRule.get(d.policy) ?? d.policy, d.path, d.line, d.code));
    }
  } else {
    for (const [rule, file, line, code] of readJson<{ findings: [string, string, number, string][] }>(path).findings) keys.add(findingKey(rule, file, line, code));
  }
  return keys;
}

function ruleFailuresOf(cell: string, side: Side, file = `${side}.findings.json`): Map<string, string> {
  const path = join(cell, file);
  const failures = new Map<string, string>();
  if (side === "rr" || !existsSync(path)) return failures;
  const result = readJson<{ compileErrors: { rule: string; error: string }[]; failures: { rule: string; code: string; message: string }[] }>(path);
  for (const error of result.compileErrors) failures.set(error.rule, `compile: ${error.error}`);
  for (const failure of result.failures) if (!failures.has(failure.rule)) failures.set(failure.rule, `${failure.code}: ${failure.message}`);
  return failures;
}

/** The fixtures' expected keys and each side's keys, restricted alike to the files each rule's recording covers. */
function fixtureKeys(baseline: string, candidate: string, corpora: string, pack: string, ruleIds: ReadonlySet<string>): { expected: Set<string>; restrict: (keys: Set<string>) => Set<string>; divergences: Divergence[] } {
  const expected = readJson<Expected>(join(recordedCorpus(corpora, pack), "expected.json"));
  const rules = new Map<string, ConformanceRule>();
  for (const rule of conformance(candidate, pack)?.rules ?? []) rules.set(rule.id, rule);
  for (const rule of conformance(baseline, pack)?.rules ?? []) rules.set(rule.id, rule);
  const originals = new Map(Object.entries(expected).map(([tool, recording]) => [tool, new Set(recording.original)]));
  const covered = (rule: ConformanceRule, path: string): boolean => {
    const tool = rule.upstream.tool;
    if (!path.startsWith(`${tool}/`)) return false;
    const inner = path.slice(tool.length + 1);
    return (originals.get(tool)?.has(inner) ?? false) && (rule.fixtures === undefined || rule.fixtures.some((pattern) => matchesGlob(inner, pattern)));
  };
  const exp = new Set<string>();
  for (const rule of rules.values()) {
    if (!ruleIds.has(rule.id)) continue;
    for (const f of expected[rule.upstream.tool]?.findings ?? []) {
      const path = `${rule.upstream.tool}/${f.path}`;
      if (f.rule === rule.upstream.rule && covered(rule, path)) exp.add(findingKey(rule.id, path, f.line, rule.upstream.rule));
    }
  }
  const restrict = (side: Set<string>): Set<string> =>
    new Set(
      [...side].filter((key) => {
        const [rule, path] = key.split("\u0000");
        const meta = rules.get(rule!);
        return meta !== undefined && ruleIds.has(rule!) && covered(meta, path!);
      }),
    );
  const divergenceFile = join(baseline, "packs", pack, "divergences.json");
  const divergences = (existsSync(divergenceFile) ? readJson<Divergence[]>(divergenceFile) : [])
    .filter((d) => d.source === undefined || d.source === "fixtures")
    .map((d) => ({ ...d, path: `${rules.get(d.rule)?.upstream.tool ?? ""}/${d.path}` }));
  return { expected: exp, restrict, divergences };
}

function metricValues(rows: readonly RunRow[], metric: (typeof METRICS)[number]): number[] {
  const [what, mode] = metric.split("-") as ["wall" | "cpu" | "rss", "cold" | "warm"];
  return rows.filter((row) => row.completed && row.mode === mode).map((row) => (what === "wall" ? row.wallMs : what === "cpu" ? row.cpuMs : row.rssKB));
}

/** Each metric of a cell from both sides' first runs, with the confirmation runs' verdict where both sides made some. */
function measuredMetrics(main: readonly RunRow[], candidate: readonly RunRow[], confirmMain: readonly RunRow[], confirmCandidate: readonly RunRow[]): MeasuredMetric[] {
  return METRICS.map((metric) => {
    const mainRuns = metricValues(main, metric);
    const candidateRuns = metricValues(candidate, metric);
    const confirmedMain = metricValues(confirmMain, metric);
    const confirmedCandidate = metricValues(confirmCandidate, metric);
    return {
      verdict: compareMetric(metric, mainRuns, candidateRuns, SLICES_TOLERANCE_PERCENT),
      mainRuns,
      candidateRuns,
      confirm: confirmedMain.length > 0 && confirmedCandidate.length > 0 ? { verdict: compareMetric(metric, confirmedMain, confirmedCandidate, SLICES_TOLERANCE_PERCENT), mainRuns: confirmedMain, candidateRuns: confirmedCandidate } : null,
    };
  });
}

/** Each run's outcome, as the completion bar reads it. */
function outcomes(rows: readonly RunRow[]): RunOutcome[] {
  return rows.map((row) => (row.completed ? { completed: true } : { completed: false, reason: `${row.tag}: ${row.reason ?? ""}${row.stoppedOn === null ? "" : `; stopped on ${row.stoppedOn}`}` }));
}

/** Both sides' findings keys compared as sets: how many both have, and samples of each side's own. */
function sameFindings(main: Set<string> | undefined, candidate: Set<string> | undefined): SameFindings | null {
  if (main === undefined || candidate === undefined) return null;
  const a = agreement(main, candidate);
  const sample = (label: string, keys: string[]): string[] => keys.slice(0, 3).map((k) => `${label} ${k.replaceAll("\u0000", " ")}`);
  return { both: a.both, mainOnly: a.mainOnly.length, candidateOnly: a.candidateOnly.length, samples: [...sample("main only", a.mainOnly), ...sample("candidate only", a.candidateOnly)] };
}

/** Each metric failing at its first runs, with the phase that moved (verdict.ts locate). */
function withLocators(metrics: readonly MeasuredMetric[], main: readonly RunRow[], candidate: readonly RunRow[], fixed: { main: readonly RunRow[]; candidate: readonly RunRow[] } | undefined): MeasuredMetric[] {
  return metrics.map((m) => {
    if (m.verdict.status !== "fail") return m;
    const coldPasses = metrics.find((x) => x.verdict.metric === m.verdict.metric.replace("-warm", "-cold"))?.verdict.status === "pass";
    return { ...m, locator: locate(m.verdict.metric, main, candidate, { coldPasses, ...(fixed === undefined ? {} : { fixed }) }) };
  });
}

function loadSummary(rows: readonly RunRow[]): LoadSummary {
  const loads = rows.flatMap((row) => (row.load === null ? [] : [row.load]));
  return {
    runs: rows.length,
    loadMedian: loads.length > 0 ? Math.round(median(loads) * 100) / 100 : null,
    loadMax: loads.length > 0 ? Math.max(...loads) : null,
    siblingBusyMax: rows.some((row) => row.siblingBusy !== null) ? Math.max(0, ...rows.map((row) => row.siblingBusy ?? 0)) : null,
  };
}

/** Slice B of one candidate on one pack and corpus, from one-rule.jsonl and one-rule-confirm.jsonl. */
function oneRuleVerdict(settings: { baseline: Checkout; candidate: Checkout; out: string }, lock: CorporaLock, pack: string, corpus: Corpus, candidate: Candidate, rr: RrRules | undefined): OneRuleVerdict {
  const empty: OneRuleVerdict = { corpus, note: "", unmeasured: [], runs: 0, fixed: null, rules: [], load: loadSummary([]) };
  const fixtures = lock.fixtures[pack];
  if (corpus === "fixtures" && fixtures !== undefined && "absent" in fixtures) return { ...empty, note: `no fixtures corpus: ${fixtures.absent}` };
  const cell = join(settings.out, "runs", pack, corpus);
  const all = readRows(join(cell, ONE_RULE.runs)).filter((row) => row.side === "main" || row.side === candidate);
  if (!all.some((row) => row.side === candidate)) return { ...empty, note: "not measured" };
  const rows = byRule(all);
  const confirmRows = readRows(join(cell, ONE_RULE.confirm)).filter((row) => row.side === "main" || row.side === candidate);
  const confirm = byRule(confirmRows);
  const runs = Math.max(...all.map((row) => row.rep));
  const expected = oneRuleSet(settings, pack, candidate, rr);
  const rrEmpty = rrEmptyWallsOf(rows);
  const timing = (rule: string): OneRuleTiming => {
    const mine = rows.get(rule) ?? [];
    const times = oneRuleTimes(rule, candidate, mine, confirm.get(rule) ?? [], rrEmpty);
    const main = mine.filter((row) => row.side === "main");
    const theirs = mine.filter((row) => row.side === candidate);
    const medianOf = (side: readonly RunRow[], pick: (row: RunRow) => number): number | null => (side.some((row) => row.completed) ? median(side.filter((row) => row.completed).map(pick)) : null);
    const file = (side: Side): string => join(ONE_RULE.findings, `${side}.${rule}.json`);
    let locator: string | null = null;
    if (times.some((t) => t.overBefore)) {
      // run-policies reports no phases: the `.rerail` side's time is its wall.
      if (candidate === "rr") locator = rule === NO_RULE ? "startup: the empty rule set's cold wall (run-policies reports no phases)" : "evaluate: the rule's net wall (run-policies reports no phases)";
      else locator = locate("wall-cold", main, theirs, rule === NO_RULE ? {} : { only: ["compile", "evaluate"] });
    }
    return {
      rule,
      times,
      locator,
      completion: compareCompletion(outcomes(main), outcomes(theirs), runs),
      findings: rule === NO_RULE ? null : sameFindings(sideFindings(cell, "main", undefined, file("main")), sideFindings(cell, candidate, rr, file(candidate))),
      ruleFailure: rule === NO_RULE ? null : (newRuleFailures(ruleFailuresOf(cell, "main", file("main")), ruleFailuresOf(cell, candidate, file(candidate)))[0]?.reason ?? null),
      wallMs: { main: medianOf(main, (row) => row.wallMs), candidate: medianOf(theirs, (row) => row.wallMs) },
      rssKB: { main: medianOf(main, (row) => row.rssKB), candidate: medianOf(theirs, (row) => row.rssKB) },
    };
  };
  return {
    corpus,
    note: "",
    unmeasured: expected.filter((rule) => !(["main", candidate] as const).every((side) => rows.get(rule)?.some((row) => row.side === side))),
    runs,
    fixed: rows.has(NO_RULE) ? timing(NO_RULE) : null,
    rules: expected.filter((rule) => rule !== NO_RULE && rows.has(rule)).map(timing),
    load: loadSummary([...all, ...confirmRows]),
  };
}

/**
 * Slice C of one candidate on one pack: each of its three files, from runs/<pack>/one-file/<label>/. The `.rerail`
 * side's findings are compared over the rules it ran there, main's restricted alike (a rule it cannot run fails
 * coverage once, as in slice A).
 */
function oneFileVerdicts(settings: { out: string }, pack: string, candidate: Candidate, rr: RrRules | undefined): OneFileVerdict[] {
  const path = oneFilesPath(settings.out, pack);
  if (!existsSync(path)) return [];
  return readJson<OneFile[]>(path).map((file): OneFileVerdict => {
    const cell = join(settings.out, "runs", pack, "one-file", file.label);
    const all = readRows(join(cell, "runs.jsonl")).filter((row) => row.side === "main" || row.side === candidate);
    const main = all.filter((row) => row.side === "main");
    const mine = all.filter((row) => row.side === candidate);
    const base: OneFileVerdict = { ...file, note: "", completion: null, metrics: [], findings: null, ruleFailures: [], load: { main: [], candidate: [] }, siblingBusyMax: null, runs: { main: main.length, candidate: mine.length } };
    if (main.length === 0 || mine.length === 0) return { ...base, note: "not measured" };
    const confirm = readRows(join(cell, "confirm.jsonl")).filter((row) => row.side === "main" || row.side === candidate);
    const measured = [...all, ...confirm];
    const ran = candidate === "rr" && rr !== undefined ? new Set(rrIds(rr, cell)) : undefined;
    const only = (keys: Set<string> | undefined): Set<string> | undefined => (keys === undefined || ran === undefined ? keys : new Set([...keys].filter((key) => ran.has(key.slice(0, key.indexOf("\u0000"))))));
    return {
      ...base,
      completion: { ...compareCompletion(outcomes(main), outcomes(mine), 2 * Math.max(...all.map((row) => row.rep))), stoppedOn: mine.find((row) => !row.completed)?.stoppedOn ?? null },
      metrics: withLocators(measuredMetrics(main, mine, confirm.filter((row) => row.side === "main"), confirm.filter((row) => row.side === candidate)), main, mine, undefined),
      findings: sameFindings(only(sideFindings(cell, "main", undefined)), only(sideFindings(cell, candidate, rr))),
      ruleFailures: newRuleFailures(ruleFailuresOf(cell, "main"), ruleFailuresOf(cell, candidate)),
      load: { main: main.map((row) => row.load), candidate: mine.map((row) => row.load) },
      siblingBusyMax: measured.some((row) => row.siblingBusy !== null) ? Math.max(0, ...measured.map((row) => row.siblingBusy ?? 0)) : null,
    };
  });
}

function cellVerdict(settings: { baseline: Checkout; candidate: Checkout; corpora: string; out: string; runs: number }, lock: CorporaLock, pack: string, corpus: Corpus, candidate: Candidate, rr: RrRules | undefined): CellVerdict {
  const empty: CellVerdict = { corpus, note: "", blocked: null, completion: null, ruleFailures: [], metrics: [], findings: null, agreement: null, load: { main: [], candidate: [] }, siblingBusyMax: null, methods: [], runs: { main: 0, candidate: 0 }, rules: { main: 0, candidate: 0 }, cost: null, caps: { memCap: [], timeCapS: [] }, chunked: null, progress: null, grouping: null, fallback: [] };
  const fixtures = lock.fixtures[pack];
  if (corpus === "fixtures" && fixtures !== undefined && "absent" in fixtures) return { ...empty, note: `no fixtures corpus: ${fixtures.absent}` };
  const cell = join(settings.out, "runs", pack, corpus);
  const all = readRows(join(cell, "runs.jsonl"));
  const main = all.filter((row) => row.side === "main");
  const mine = all.filter((row) => row.side === candidate);
  if (main.length === 0) return { ...empty, note: "not measured" };
  if (candidate === "rr" && rr !== undefined && rrIds(rr, cell).length === 0) return { ...empty, blocked: "no rule of the pack runs on the .rerail side on this corpus" };
  if (mine.length === 0) return { ...empty, note: "candidate not measured" };
  const completion = { ...compareCompletion(outcomes(main), outcomes(mine), 2 * settings.runs), stoppedOn: mine.find((row) => !row.completed)?.stoppedOn ?? null };
  const ruleFailures = candidate === "rr" ? [] : newRuleFailures(ruleFailuresOf(cell, "main"), ruleFailuresOf(cell, candidate));
  const confirm = readRows(join(cell, "confirm.jsonl"));
  const fixedRows = readRows(join(cell, "fixed.jsonl"));
  const fixed = { main: fixedRows.filter((row) => row.side === "main"), candidate: fixedRows.filter((row) => row.side === candidate) };
  let metrics = withLocators(measuredMetrics(main, mine, confirm.filter((row) => row.side === "main"), confirm.filter((row) => row.side === candidate)), main, mine, fixed);
  // A `.rerail` whole-pack run that did not complete: its figures from the chunked run, if one was made (measureChunks).
  const chunks = candidate === "rr" && !mine.every((row) => row.completed) ? readChunks(cell) : undefined;
  const chunkRows = chunks === undefined ? [] : readRows(join(cell, "chunks.jsonl"));
  let chunked: CellVerdict["chunked"] = null;
  let chunkFiles: string[] | null = null;
  if (chunks !== undefined && chunkRows.length > 0) {
    const failedChunks = chunks.flatMap((rules, chunk) => {
      const bad = chunkRows.find((row) => row.chunk === chunk && row.mode === "cold" && !row.completed);
      return bad === undefined ? [] : [{ chunk, rules: `${rules[0]}…${rules.at(-1)} (${rules.length})`, reason: `${bad.reason ?? ""}${bad.stoppedOn === null ? "" : `; stopped on ${bad.stoppedOn}`}` }];
    });
    const okChunks = chunks.map((_, chunk) => chunk).filter((chunk) => !failedChunks.some((f) => f.chunk === chunk));
    const sums = chunkSums(chunkRows.map((row) => ({ chunk: row.chunk!, rep: row.rep, mode: row.mode, completed: row.completed, wallMs: row.wallMs, cpuMs: row.cpuMs, rssKB: row.rssKB })), chunks.length);
    chunked = { size: Math.max(...chunks.map((c) => c.length)), chunks: chunks.length, rulesRun: okChunks.reduce((n, chunk) => n + chunks[chunk]!.length, 0), rulesOf: chunks.flat().length, failedChunks };
    chunkFiles = okChunks.map((chunk) => `rr.chunk${chunk}.findings.json`);
    // The candidate's figures are the chunks' sums per repetition; a cell that did not complete is never confirmed.
    metrics = metrics.map((m) => {
      const [what, mode] = m.verdict.metric.split("-") as ["wall" | "cpu" | "rss", "cold" | "warm"];
      const candidateRuns = sums.filter((s) => s.mode === mode).map((s) => (what === "wall" ? s.wallMs : what === "cpu" ? s.cpuMs : s.rssKB));
      const verdict = compareMetric(m.verdict.metric, m.mainRuns, candidateRuns, SLICES_TOLERANCE_PERCENT);
      return { ...m, verdict: verdict.reason === "" ? verdict : { ...verdict, reason: `${verdict.reason} (chunked: the sum of ${chunks.length} chunk runs)` }, candidateRuns };
    });
  }
  // Findings are compared over the rules the candidate ran (main's shipped rules it has, and any it adds): a rule it
  // cannot run fails coverage once, not again as every key of that rule. A chunked run ran its completed chunks' rules.
  const mainRules = shippedRules(settings.baseline.dir, pack);
  const ran = chunked !== null ? chunks!.filter((_, chunk) => !chunked!.failedChunks.some((f) => f.chunk === chunk)).flat() : candidate === "rr" && rr !== undefined ? rrIds(rr, cell) : shippedRules(settings.candidate.dir, pack);
  const ruleIds = new Set(ran.filter((rule) => candidate === "ships" || mainRules.includes(rule)));
  const only = (keys: Set<string> | undefined): Set<string> | undefined => (keys === undefined ? undefined : new Set([...keys].filter((key) => ruleIds.has(key.slice(0, key.indexOf("\u0000"))))));
  const mainKeys = only(sideFindings(cell, "main", undefined));
  const chunkKeys = chunkFiles?.map((file) => sideFindings(cell, "rr", rr, file));
  const candidateKeys = only(chunkKeys === undefined ? sideFindings(cell, candidate, rr) : chunkKeys.some((keys) => keys === undefined) ? undefined : new Set(chunkKeys.flatMap((keys) => [...keys!])));
  let findings: CellVerdict["findings"] = null;
  if (mainKeys !== undefined && candidateKeys !== undefined && corpus === "fixtures") {
    const { expected, restrict, divergences } = fixtureKeys(settings.baseline.dir, settings.candidate.dir, settings.corpora, pack, ruleIds);
    const m = scoreFindings(expected, restrict(mainKeys), divergences);
    const c = scoreFindings(expected, restrict(candidateKeys), divergences);
    const compared = compareFindings(m, c);
    const counts = (s: typeof m): Counts => ({ tp: s.tp.size, fp: s.fp.size, fn: s.fn.size, explainedFp: s.explainedFp, explainedFn: s.explainedFn });
    findings = { status: compared.status, reason: compared.reason, lostTp: compared.lostTp.length, newFp: compared.newFp.length, samples: [...compared.lostTp.slice(0, 5).map((k) => `lost TP ${k.replaceAll("\u0000", " ")}`), ...compared.newFp.slice(0, 5).map((k) => `new FP ${k.replaceAll("\u0000", " ")}`)], main: counts(m), candidate: counts(c) };
  }
  // `!= null`: rows of an earlier version of this script have no progress or grouping.
  const failedWhole = mine.find((row) => !row.completed && row.progress != null);
  const rrRows = [...mine, ...chunkRows].filter((row) => row.side === "rr");
  const wholeGrouping = mine.find((row) => row.completed && row.grouping != null)?.grouping ?? null;
  const chunkGroupings = chunkRows.filter((row) => row.mode === "cold" && row.rep === 1 && row.grouping != null).map((row) => row.grouping!);
  const measuredRows = [...all, ...chunkRows];
  return {
    corpus,
    note: "",
    blocked: null,
    completion,
    ruleFailures,
    metrics,
    findings,
    agreement: corpus === "real" ? sameFindings(mainKeys, candidateKeys) : null,
    load: { main: main.map((row) => row.load), candidate: mine.map((row) => row.load) },
    siblingBusyMax: measuredRows.some((row) => row.siblingBusy !== null) ? Math.max(0, ...measuredRows.map((row) => row.siblingBusy ?? 0)) : null,
    methods: methodCounts(measuredRows).map((m) => m.method),
    runs: { main: main.length, candidate: mine.length },
    rules: { main: mainRules.length, candidate: candidate === "rr" && rr !== undefined ? rrIds(rr, cell).length : shippedRules(settings.candidate.dir, pack).length },
    cost: cellCost({
      baseline: { runs: main, fixed: fixed.main, rules: ruleSet(join(settings.baseline.dir, "packs", pack, "rules")) },
      // The .rerail side has no start-up run; its rule set is main's rules it runs.
      candidate: { runs: mine, fixed: fixed.candidate, rules: candidate === "rr" ? ruleSet(join(settings.baseline.dir, "packs", pack, "rules"), new Set(ran)) : ruleSet(join(settings.candidate.dir, "packs", pack, "rules")) },
      workload: readWorkload(settings.out, relative(settings.corpora, corpusDirectory(settings.corpora, pack, corpus))),
    }),
    caps: { memCap: [...new Set(measuredRows.map((row) => row.memCap))], timeCapS: [...new Set(measuredRows.map((row) => row.timeCapS))] },
    chunked,
    progress: failedWhole === undefined ? null : { ...failedWhole.progress!, wallMs: failedWhole.wallMs, rssKB: failedWhole.rssKB },
    grouping:
      wholeGrouping ??
      (chunkGroupings.length === 0
        ? null
        : chunkGroupings.reduce((a, g) => ({ policies: a.policies + g.policies, combined: a.combined + g.combined, alone: a.alone + g.alone, largest: Math.max(a.largest, g.largest) }))),
    fallback: [...new Set(rrRows.flatMap((row) => row.fallback ?? []))].slice(0, 5),
  };
}

/** Every run row of a pack's slices under `out`: A's and B's per corpus, C's per file. */
function sliceRows(out: string, pack: string, corpusKinds: readonly Corpus[]): RunRow[] {
  const files = existsSync(oneFilesPath(out, pack)) ? readJson<OneFile[]>(oneFilesPath(out, pack)) : [];
  return [
    ...corpusKinds.flatMap((corpus) => ["runs.jsonl", "confirm.jsonl", ONE_RULE.runs, ONE_RULE.confirm].flatMap((log) => readRows(join(out, "runs", pack, corpus, log)))),
    ...(corpusKinds.includes("real") ? files.flatMap((file) => ["runs.jsonl", "confirm.jsonl"].flatMap((log) => readRows(join(out, "runs", pack, "one-file", file.label, log)))) : []),
  ];
}

/** The candidate's docs/rule-syntax/packs/<pack>/equivalence.json, parsed (null where it is not JSON); undefined when absent. */
function equivalenceDeclaration(candidate: string, pack: string): unknown {
  const path = join(candidate, "docs", "rule-syntax", "packs", pack, "equivalence.json");
  if (!existsSync(path)) return undefined;
  try {
    return JSON.parse(readFileSync(path, "utf8"));
  } catch {
    return null;
  }
}

function computeVerdict(settings: { baseline: Checkout; candidate: Checkout; corpora: string; out: string; runs: number }, packs: readonly string[], candidateOnly: readonly string[], corpusKinds: readonly Corpus[], candidates: readonly Candidate[], gate: readonly Candidate[], partial: readonly string[]): Verdict {
  const lock = readJson<CorporaLock>(join(settings.corpora, "corpora.lock.json"));
  const rows = packs.flatMap((pack) => sliceRows(settings.out, pack, corpusKinds));
  const distinct = <T>(values: T[]): T[] => [...new Set(values)];
  const verdicts: CandidateVerdict[] = candidates.map((candidate) => {
    const packVerdicts: PackVerdict[] = packs.map((pack) => {
      const mainRules = shippedRules(settings.baseline.dir, pack);
      const rrPath = join(settings.out, "runs", pack, "rr-rules.json");
      const rr = candidate === "rr" && existsSync(rrPath) ? readJson<RrRules>(rrPath) : undefined;
      const available = new Map<string, string | null>(
        candidate === "ships" ? shippedRules(settings.candidate.dir, pack).map((rule) => [rule, null]) : Object.entries(rr?.rules ?? {}).map(([rule, r]) => [rule, r.unavailable]),
      );
      // A `.rerail` rule that failed alone on a corpus is left out of that corpus's runs: it does not run there.
      for (const corpus of candidate === "rr" ? corpusKinds : []) {
        for (const [rule, reason] of Object.entries(readExcluded(join(settings.out, "runs", pack, corpus)))) {
          const before = available.get(rule);
          available.set(rule, `${before === null || before === undefined ? "" : `${before}; `}fails to run on ${corpus}: ${reason}`);
        }
      }
      const missing = candidate === "rr" && rr === undefined ? [{ rule: "*", reason: "the .rerail side was not measured" }] : missingRules(mainRules, available);
      const extra = candidate === "ships" ? [...available.keys()].filter((rule) => !mainRules.includes(rule)) : [];
      return {
        pack,
        noBaseline: false,
        missing,
        extra,
        cells: corpusKinds.map((corpus) => cellVerdict(settings, lock, pack, corpus, candidate, rr)),
        oneRule: corpusKinds.map((corpus) => oneRuleVerdict(settings, lock, pack, corpus, candidate, rr)),
        oneFile: corpusKinds.includes("real") ? oneFileVerdicts(settings, pack, candidate, rr) : [],
        gate: candidate === "rr" ? translationGate(pack, equivalenceDeclaration(settings.candidate.dir, pack)) : null,
      };
    });
    for (const pack of candidateOnly) packVerdicts.push({ pack, noBaseline: true, missing: [], extra: [], cells: [], oneRule: [], oneFile: [], gate: null });
    return { candidate, description: CANDIDATES[candidate], packs: packVerdicts };
  });
  const machinesDir = join(settings.out, "machines");
  return {
    tolerancePercent: SLICES_TOLERANCE_PERCENT,
    runs: settings.runs,
    baseline: settings.baseline,
    candidate: settings.candidate,
    corpora: lock,
    machine: {
      hosts: distinct(rows.map((row) => row.host)),
      cpus: distinct(rows.map((row) => row.cpus)),
      threads: distinct(rows.map((row) => row.threads)),
      memCap: distinct(rows.map((row) => row.memCap)),
      timeCapS: distinct(rows.map((row) => row.timeCapS)),
      methods: methodCounts(rows),
    },
    machines: existsSync(machinesDir) ? readdirSync(machinesDir).filter((name) => name.endsWith(".json")).sort().map((name) => readJson<MachineInfo>(join(machinesDir, name))) : [],
    candidates: verdicts,
    gate: [...gate],
    partial: [...partial],
  };
}

/** report.json and report.md; the exit status: 0 only when every gated candidate passes. */
function writeReport(verdict: Verdict, out: string, meta: RunMeta): number {
  const report = buildReport(verdict, meta, waitsSection(out));
  writeFileSync(join(out, "report.json"), `${JSON.stringify(report, null, 1)}\n`);
  writeFileSync(join(out, "report.md"), renderReport(report));
  for (const c of report.candidates) console.log(`shipbar: ${c.candidate}: ${c.status} (${c.counts.failing_items} failing items, ${c.counts.noisy_items} noisy)`);
  console.log(`shipbar: ${join(out, "report.json")}, ${join(out, "report.md")}`);
  return report.verdict.status === "PASS" ? 0 : 1;
}

/**
 * The run's metadata: when it ran (its logs' first run to now), who started it and how, this harness's commit, and the
 * repetitions `run` recorded (none, for runs made before it recorded them).
 */
function runMeta(out: string, packs: readonly string[], corpusKinds: readonly Corpus[]): RunMeta {
  const settingsPath = join(out, "settings.json");
  const recorded = existsSync(settingsPath) ? readJson<{ invokedBy?: string | null; command?: string; confirm?: number; ruleRuns?: number; fileRuns?: number }>(settingsPath) : {};
  const packStarts = packs.flatMap((pack) => sliceRows(out, pack, corpusKinds).map((row) => row.started));
  const starts = [...packStarts, ...readWaitRuns(out).map((row) => row.started)].sort();
  return {
    startedAt: starts[0] ?? null,
    finishedAt: new Date().toISOString(),
    invokedBy: recorded.invokedBy ?? null,
    command: recorded.command ?? null,
    harnessSha: checkoutSha(SELF),
    confirmRuns: recorded.confirm ?? 0,
    ruleRuns: recorded.ruleRuns ?? 0,
    fileRuns: recorded.fileRuns ?? 0,
  };
}

// ---------------------------------------------------------------- measuring processes

interface SettingsBase {
  baseline: Checkout;
  candidate: Checkout;
  corpora: string;
  out: string;
  runs: number;
}

interface MeasureOptions {
  cpus: string;
  memCap: string;
  timeCapS: number;
  packCaps: readonly string[];
  confirm: number;
  ruleRuns: number;
  fileRuns: number;
  rrChunk: number;
  stores: string | undefined;
}

/** A measuring process's settings (`run`, a fan-out's `work`), the process pinned to its CPUs on Linux. */
function openSettings(base: SettingsBase, platform: Platform, engine: string | undefined, packs: readonly string[], o: MeasureOptions): Settings {
  let cpus: number[] = [];
  let siblings: number[] = [];
  try {
    cpus = cpuList(o.cpus);
    siblings = platform.siblings(cpus);
  } catch (error) {
    fail(`--cpus ${o.cpus}: ${error instanceof Error ? error.message : String(error)}`);
  }
  // platform.ts pins each measured run; on Linux this process's own work between runs (check-rr, workload counts,
  // hashing, writing) stays on the stream's cores too, so concurrent streams on one box do not spill onto each other's
  // idle hyperthread siblings. Its children inherit the mask.
  if (process.platform === "linux" && run("taskset", ["-a", "-cp", cpus.join(","), String(process.pid)]).status !== 0) fail(`could not pin this process to CPUs ${cpus.join(",")}`);
  const packCaps: Settings["packCaps"] = {};
  for (const spec of o.packCaps) {
    const match = /^([a-z0-9-]+)=(\d+[KMGT]?),(\d+)$/.exec(spec);
    if (match === null) fail(`--pack-cap ${spec}: <pack>=<memory cap>,<time cap in s>, e.g. python=56G,14400`);
    if (!packs.includes(match[1]!)) fail(`--pack-cap ${spec}: main ships no pack ${match[1]}`);
    packCaps[match[1]!] = { memCap: match[2]!, timeCapS: Number(match[3]) };
  }
  return { ...base, engine, cpus, siblings, platform, memCap: o.memCap, timeCapS: o.timeCapS, packCaps, confirm: o.confirm, ruleRuns: o.ruleRuns, fileRuns: o.fileRuns, rrChunk: o.rrChunk, stores: o.stores };
}

/** One unit of a fan-out's plan, for every candidate the plan measures, through run's own code into the same cell logs. */
function runUnit(settings: Settings, u: PlannedUnit, candidates: readonly Candidate[]): void {
  const rr = candidates.includes("rr") ? packRrRules(settings, u.pack) : undefined;
  if (u.slice === "A") {
    const dir = corpusDirectory(settings.corpora, u.pack, u.corpus);
    if (u.rep === 1) cachedWorkload(settings.out, relative(settings.corpora, dir), dir, engineBinary(settings.candidate.dir));
    measureCell(settings, join(settings.out, "runs", u.pack, u.corpus), dir, u.pack, [u.rep!], ["main", ...candidates], rr);
    measureFixed(settings, u.pack, u.corpus, ["main", ...candidates], [u.rep!]);
  } else if (u.slice === "B") {
    measureOneRule(settings, u.pack, u.corpus, candidates, rr, u.rules!);
  } else {
    const file = pickOneFiles(settings, u.pack).find((f) => f.label === u.label) ?? fail(`${u.id}: no such file in ${oneFilesPath(settings.out, u.pack)}`);
    measureOneFile(settings, u.pack, file, candidates, rr);
  }
}

/**
 * A fan-out worker: this host's units of `phase`, longest first, each claimed by an atomic mkdir of
 * out/claims/<phase>/<unit> (a unit another worker claimed is skipped), its claim marked done when it ends.
 */
function workUnits(settings: Settings, plan: Plan, host: string, worker: number, phase: "measure" | "confirm"): void {
  const claims = join(settings.out, "claims", phase);
  mkdirSync(claims, { recursive: true });
  const todo: { id: string; est: number | null; run: () => void }[] =
    phase === "measure"
      ? plan.units.filter((u) => u.host === host).map((u) => ({ id: u.id, est: u.est, run: () => runUnit(settings, u, plan.candidates) }))
      : plan.confirm.filter((c) => c.host === host).map((c) => ({ id: c.id, est: null, run: () => confirmCell(settings, join(settings.out, "runs", c.pack, c.corpus), corpusDirectory(settings.corpora, c.pack, c.corpus), c.pack, plan.candidates, plan.candidates.includes("rr") ? packRrRules(settings, c.pack) : undefined) }));
  for (const unit of todo) {
    const claim = join(claims, claimName(unit.id));
    try {
      mkdirSync(claim);
    } catch {
      continue;
    }
    const started = Date.now();
    unit.run();
    writeFileSync(join(claim, "done"), "");
    appendFileSync(join(settings.out, "units.jsonl"), `${JSON.stringify({ unit: unit.id, phase, host, worker, cpus: settings.cpus.join(","), est: unit.est, started: new Date(started).toISOString(), seconds: Math.round((Date.now() - started) / 100) / 10 })}\n`);
  }
}

/**
 * A fan-out's host: the corpora verified and both sides checked built once, then a `work` process per CPU set for the
 * measure phase, then again for the confirm phase; out/host.json holds each phase's start, end and workers' exits.
 * A claim left without its done mark (a worker that died) is cleared first, so its unit runs again, resuming.
 */
async function hostRun(base: SettingsBase, plan: Plan, host: string, workers: readonly string[], forward: readonly string[]): Promise<number> {
  const packs = [...new Set(plan.units.filter((u) => u.host === host).map((u) => u.pack))];
  verifyCorpora(base.corpora, packs, CORPORA);
  requireBuilt(base.baseline.dir, { product: true, engine: false }, false);
  requireBuilt(base.candidate.dir, { product: true, engine: plan.candidates.includes("rr") }, false);
  const platform = requirePlatform(base.out);
  mkdirSync(join(base.out, "machines"), { recursive: true });
  writeFileSync(join(base.out, "machines", `${hostname()}.json`), JSON.stringify(platform.machine(), null, 1));
  mkdirSync(join(base.out, "logs"), { recursive: true });
  const record: Record<string, unknown> = { host, hostname: hostname(), workers };
  let failed = false;
  for (const phase of ["measure", "confirm"] as const) {
    const claims = join(base.out, "claims", phase);
    for (const name of existsSync(claims) ? readdirSync(claims) : []) if (!existsSync(join(claims, name, "done"))) rmSync(join(claims, name), { recursive: true, force: true });
    const start = new Date();
    const exits = await Promise.all(
      workers.map((cpus, worker) => {
        const log = openSync(join(base.out, "logs", `${phase}-${worker}.log`), "a");
        const child = spawn(process.execPath, [...process.execArgv, fileURLToPath(import.meta.url), "work", ...forward, "--worker", String(worker), "--cpus", cpus, "--phase", phase], { stdio: ["ignore", log, log] });
        const { promise, resolve: exited } = Promise.withResolvers<number>();
        child.on("close", (code) => {
          closeSync(log);
          exited(code ?? 1);
        });
        return promise;
      }),
    );
    failed ||= exits.some((code) => code !== 0);
    const seconds = Math.round((Date.now() - start.getTime()) / 1000);
    record[phase] = { start: start.toISOString(), end: new Date().toISOString(), seconds, exits };
    writeFileSync(join(base.out, "host.json"), JSON.stringify(record, null, 1));
    console.log(`shipbar host ${host}: ${phase} done in ${seconds} s, worker exits ${exits.join(" ")}`);
  }
  return failed ? 1 : 0;
}

// ---------------------------------------------------------------- the fan-out from the Mac

interface FanoutOptions {
  hosts: string[];
  /** The work directory on each host: ~/work/<name>. */
  name: string;
  workers: string[];
  baselineSpec: string;
  packDataCache: string | undefined;
  candidate: Checkout;
  out: string;
  corpora: string;
  requested: string[] | undefined;
  runs: number;
  confirm: number;
  ruleRuns: number;
  fileRuns: number;
  memCap: string;
  timeCapS: number;
  packCaps: string[];
  stores: string;
  pollS: number;
  /** Make this phase and every later one again (`--from sync` after a checkout changed, say). */
  from: string | undefined;
  /** The remote shell, as words: `ssh`, or --ssh / SHIPBAR_SSH (fanout.ts remoteShell). */
  shell: string[];
  /** The candidates measured beside main (`--candidates`). */
  candidates: Candidate[];
  command: string;
}

/** A fan-out's phases, in order. */
const FANOUT_PHASES = ["prepare", "plan", "sync", "build", "distribute", "measure", "merge"];

/** What a checkout's sync leaves out: history, dependencies and builds (each host installs and builds its own). */
const CHECKOUT_EXCLUDES = ["--exclude=/.git", "--exclude=/node_modules", "--exclude=/target", "--exclude=/native/target", "--exclude=/native/*.node", "--exclude=/dist", "--exclude=/.rerail/pack-data", "--exclude=/.worktrees"];

/** Where a fan-out builds the candidate's `rerail-engine` (with rr), under its checkout on the hosts: target/ there. */
const FANOUT_RR_ENGINE = "candidate/target/release/rerail-engine";

/** The engines a fan-out builds on its first host and copies to the others, as paths under ~/work/<name>/ there. */
function fanoutEngines(candidates: readonly Candidate[]): string[] {
  return ["baseline/native/rerail_engine.linux-*.node", "candidate/native/rerail_engine.linux-*.node", ...(candidates.includes("rr") ? [FANOUT_RR_ENGINE] : [])];
}

/** The host scripts a fan-out starts detached; each marks its start and writes its exit status for the Mac to poll. */
function hostScripts(o: FanoutOptions): Record<string, string> {
  const quote = (arg: string): string => `'${arg.replaceAll("'", "'\\''")}'`;
  const options = [
    ...["--baseline", "../baseline", "--candidate", ".", "--corpora", "../corpora", "--out", "../out", "--plan", "../out/plan.json"],
    ...["--runs", String(o.runs), "--confirm", String(o.confirm), "--rule-runs", String(o.ruleRuns), "--file-runs", String(o.fileRuns)],
    ...["--mem-cap", o.memCap, "--time-cap", String(o.timeCapS), "--stores", o.stores, ...o.packCaps.flatMap((cap) => ["--pack-cap", cap])],
  ];
  const head = (step: string): string => `#!/bin/bash\ncd "$(dirname "$0")/.." || exit 1\nrm -f fanout/${step}.exit\ndate -u +%FT%TZ > fanout/${step}.started\n`;
  return {
    // Both sides at once, through the host's compiler cache (with rr, the candidate's rerail-engine too, into its own
    // target/); then the engines' sha256, for the other hosts to check.
    "build.sh": `${head("build")}side() { ( cd "$1" && sh -c "$2" ) > "fanout/build-$1.log" 2>&1; }\nside baseline ${quote(PRODUCT_BUILD)} & b=$!\nside candidate ${quote(o.candidates.includes("rr") ? `${PRODUCT_BUILD} && CARGO_TARGET_DIR=target ${ENGINE_BUILD}` : PRODUCT_BUILD)} & c=$!\nwait $b; eb=$?\nwait $c; ec=$?\nsha256sum ${fanoutEngines(o.candidates).join(" ")} > fanout/build.sha 2> fanout/build-sha.log\necho "$eb $ec" > fanout/build.exit\n`,
    // The engines built on the first host are in place: install the dependencies and check the engines.
    "install.sh": `${head("install")}side() { ( cd "$1" && npm ci --ignore-scripts ) > "fanout/install-$1.log" 2>&1; }\nside baseline & b=$!\nside candidate & c=$!\nwait $b; eb=$?\nwait $c; ec=$?\nsha256sum -c fanout/build.sha > fanout/install-sha.log 2>&1; es=$?\necho "$eb $ec $es" > fanout/install.exit\n`,
    "host.sh": `${head("host")}cd candidate\nnode --import tsx ../harness/scripts/ship/shipbar.ts host --host "$1" --workers ${quote(o.workers.join("/"))} ${options.map(quote).join(" ")} > ../fanout/host.log 2>&1\necho $? > ../fanout/host.exit\n`,
  };
}

async function fanoutRun(o: FanoutOptions): Promise<number> {
  const root = join(o.out, "fanout");
  mkdirSync(root, { recursive: true });
  const statePath = join(root, "state.json");
  const state: { phases: Record<string, { start: string; end?: string; seconds?: number }> } = existsSync(statePath) ? readJson(statePath) : { phases: {} };
  // --from <phase>: that phase and every later one made again.
  if (o.from !== undefined) {
    const at = FANOUT_PHASES.indexOf(o.from);
    if (at < 0) fail(`--from ${o.from}: one of ${FANOUT_PHASES.join(", ")}`);
    for (const name of FANOUT_PHASES.slice(at)) delete state.phases[name];
  }
  const save = (): void => writeFileSync(statePath, JSON.stringify(state, null, 1));
  const log = (line: string): void => console.log(`shipbar fanout ${new Date().toISOString().slice(11, 19)}: ${line}`);
  const phase = async (name: string, body: () => Promise<void> | void): Promise<void> => {
    const done = state.phases[name];
    if (done?.end !== undefined) return log(`${name}: done before (${done.seconds} s)`);
    const start = Date.now();
    state.phases[name] = { start: new Date(start).toISOString() };
    save();
    log(`${name}: start`);
    await body();
    state.phases[name] = { start: new Date(start).toISOString(), end: new Date().toISOString(), seconds: Math.round((Date.now() - start) / 1000) };
    save();
    log(`${name}: ${state.phases[name]!.seconds} s`);
  };
  const remote = `work/${o.name}`;
  const at = (host: string, path: string): string => `${host}:${remote}/${path}`;
  const ssh = (host: string, command: string, timeoutS?: number): Promise<Done> => sshOver(o.shell, host, command, timeoutS);
  const rsync = (from: string, to: string, extra: readonly string[], timeoutS: number): Promise<Done> => rsyncOver(o.shell, from, to, extra, timeoutS);
  const onHosts = async (hosts: readonly string[], label: string, each: (host: string) => Promise<boolean>): Promise<void> => {
    const ok = await Promise.all(hosts.map(each));
    const bad = hosts.filter((_, i) => !ok[i]);
    if (bad.length > 0) fail(`fanout ${label} failed on ${bad.join(", ")}; run the same command again to resume`);
  };
  const step = async (label: string, attempt: () => Promise<Done>, tries = 3): Promise<boolean> => (await retry(label, tries, attempt, log)).status === 0;
  /** A host script started detached (setsid nohup); started once: a retried start finds its start mark. */
  const start = async (host: string, script: string, args: readonly string[] = []): Promise<boolean> => {
    const marker = script.replace(/\.sh$/, "");
    await ssh(host, `rm -f ${remote}/fanout/${marker}.exit ${remote}/fanout/${marker}.started`);
    for (let attempt = 1; attempt <= 3; attempt += 1) {
      const started = await ssh(host, detached(remote, script, args), 120);
      // The script marks its start first thing; a slow host gets a minute before the start is tried again.
      for (let check = 0; check < 6; check += 1) {
        await sleep(10_000);
        if ((await ssh(host, `test -e ${remote}/fanout/${marker}.started`)).status === 0) return true;
      }
      log(`${host}: starting ${script} failed (${started.status}): ${started.stderr.trim().split("\n").at(-1) ?? ""}`);
    }
    return false;
  };
  /** Poll until `marker` (fanout/<marker>.exit) is on every host; `progress` names what each host is at. */
  const waitFor = async (hosts: readonly string[], marker: string, progress = ""): Promise<Map<string, string>> => {
    const exits = new Map<string, string>();
    let first = true;
    while (exits.size < hosts.length) {
      await sleep((first ? Math.min(60, o.pollS) : o.pollS) * 1000);
      first = false;
      const lines: string[] = [];
      await Promise.all(
        hosts.filter((h) => !exits.has(h)).map(async (host) => {
          const r = await ssh(host, `cd ${remote} && cat fanout/${marker}.exit 2>/dev/null; echo "|$(cut -d' ' -f1 /proc/loadavg)${progress === "" ? "" : `|$(${progress})`}"`);
          if (r.status !== 0) return lines.push(`${host} unreachable (${r.status})`);
          const [exit, rest] = [r.stdout.split("|")[0]!.trim(), r.stdout.split("|").slice(1).join(" ").trim()];
          if (exit !== "") exits.set(host, exit);
          lines.push(`${host} ${exit === "" ? "running" : `exit ${exit}`}, load ${rest}`);
        }),
      );
      log(`${marker}: ${lines.join("; ")}`);
    }
    return exits;
  };

  let baseline: Checkout | undefined;
  await phase("prepare", () => {
    baseline = resolveBaseline(o.baselineSpec, o.out, o.packDataCache);
    // Both sides build on the hosts: neither may hold Git LFS pointers in place of the files (scripts/build/require-lfs.mjs).
    for (const dir of [baseline.dir, o.candidate.dir]) {
      const check = join(dir, "scripts", "build", "require-lfs.mjs");
      const lfs = existsSync(check) ? run("node", [check], { cwd: dir }) : { status: 0, stderr: "" };
      if (lfs.status !== 0) fail(`${dir}: ${lastLine(lfs.stderr)}`);
    }
    mkdirSync(join(o.candidate.dir, ".rerail"), { recursive: true });
    writeFileSync(join(o.candidate.dir, SOURCE_FILE), JSON.stringify({ sha: o.candidate.sha }));
    prepareCorpora(baseline, discoverPacks(baseline.dir, o.candidate.dir).packs, o.corpora, false);
  });
  baseline ??= resolveBaseline(o.baselineSpec, o.out, o.packDataCache);
  const discovered = discoverPacks(baseline.dir, o.candidate.dir);
  for (const pack of o.requested ?? []) if (!discovered.packs.includes(pack)) fail(`--packs: main ships no pack ${pack}`);
  const packs = discovered.packs.filter((pack) => o.requested === undefined || o.requested.includes(pack));
  const settings = { baseline, candidate: o.candidate, corpora: o.corpora, out: o.out, runs: o.runs };
  const planPath = join(o.out, "plan.json");
  const stage = join(root, "stage");
  await phase("plan", () => {
    const lock = verifyCorpora(o.corpora, packs, CORPORA);
    const inputs: PackInput[] = packs.map((pack) => {
      const fixtures = lock.fixtures[pack];
      // The `.rerail` rules as a plan knows them, before check-rr (which runs on the hosts): every rewrite.
      const rr = o.candidates.includes("rr") ? rrRewrites(o.candidate.dir, shippedRules(baseline!.dir, pack), pack) : undefined;
      return { pack, corpora: CORPORA.filter((c) => c === "real" || (fixtures !== undefined && !("absent" in fixtures))), rules: oneRuleUnion(settings, pack, o.candidates, rr), files: pickOneFiles(settings, pack).map((f) => f.label) };
    });
    const plan = planFanout(inputs, o.hosts, o.workers, { runs: o.runs, ruleRuns: o.ruleRuns, fileRuns: o.fileRuns, candidates: o.candidates });
    writeFileSync(planPath, JSON.stringify(plan, null, 1));
    const count = (slice: string): number => plan.units.filter((u) => u.slice === slice).length;
    log(`plan: ${plan.units.length} units (A ${count("A")}, B ${count("B")}, C ${count("C")}), longest ${Math.round(plan.units[0]!.est)} s, planned per host ${JSON.stringify(plan.load)} s`);
    // What each host gets besides the checkouts and corpora: this harness, the host scripts, the plan and C's files.
    rmSync(stage, { recursive: true, force: true });
    cpSync(HERE, join(stage, "harness", "scripts", "ship"), { recursive: true });
    writeFileSync(join(stage, "harness", "package.json"), JSON.stringify({ private: true, type: "module" }));
    mkdirSync(join(stage, "harness", ".rerail"), { recursive: true });
    writeFileSync(join(stage, "harness", SOURCE_FILE), JSON.stringify({ sha: checkoutSha(SELF) }));
    mkdirSync(join(stage, "fanout"), { recursive: true });
    for (const [name, text] of Object.entries(hostScripts(o))) writeFileSync(join(stage, "fanout", name), text);
    copyInto(planPath, join(stage, "out", "plan.json"));
    for (const pack of packs) copyInto(oneFilesPath(o.out, pack), oneFilesPath(join(stage, "out"), pack));
  });
  const plan = readJson<Plan>(planPath);
  await phase("sync", () =>
    onHosts(o.hosts, "sync", async (host) => {
      // A fresh sync starts the host's measurements afresh.
      if (!(await step(`${host} clear`, () => ssh(host, `mkdir -p ${remote} && rm -rf ${remote}/out ${remote}/fanout/*.exit ${remote}/fanout/*.started ${o.stores}`)))) return false;
      const pushes: [string, string, string[]][] = [
        [`${baseline!.dir}/`, "baseline/", ["--delete", ...CHECKOUT_EXCLUDES]],
        [`${o.candidate.dir}/`, "candidate/", ["--delete", ...CHECKOUT_EXCLUDES]],
        [`${o.corpora}/`, "corpora/", ["--delete"]],
        [`${join(stage, "harness")}/`, "harness/", ["--delete"]],
        [`${join(stage, "fanout")}/`, "fanout/", []],
        [`${join(stage, "out")}/`, "out/", []],
      ];
      for (const [from, to, extra] of pushes) if (!(await step(`${host} ${to}`, () => rsync(from, at(host, to), extra, 1800)))) return false;
      log(`${host}: synced`);
      return true;
    }),
  );
  const [builder, ...others] = o.hosts as [string, ...string[]];
  await phase("build", async () => {
    if (!(await start(builder, "build.sh"))) fail(`fanout build: could not start it on ${builder}`);
    const exit = (await waitFor([builder], "build")).get(builder);
    if (exit !== "0 0") fail(`fanout build on ${builder} exited ${exit}: see ~/${remote}/fanout/build-*.log there`);
  });
  await phase("distribute", async () => {
    const sums = await ssh(builder, `cat ${remote}/fanout/build.sha`);
    const engines = sums.stdout.split("\n").filter((l) => l.trim() !== "").map((line) => ({ sha256: line.slice(0, 64), path: line.slice(64).trim() }));
    const expected = fanoutEngines(o.candidates).length;
    if (sums.status !== 0 || engines.length !== expected) fail(`fanout distribute: ${builder}'s fanout/build.sha names ${engines.length} engines, not ${expected}`);
    const local = join(root, "engines");
    for (const engine of engines) {
      mkdirSync(dirname(join(local, engine.path)), { recursive: true });
      if (!(await step(`pull ${engine.path}`, () => rsync(at(builder, engine.path), join(local, engine.path), [], 900)))) fail(`fanout distribute: could not pull ${engine.path} from ${builder}`);
      const got = createHash("sha256").update(readFileSync(join(local, engine.path))).digest("hex");
      if (got !== engine.sha256) fail(`fanout distribute: ${engine.path} from ${builder} is ${got}, its build said ${engine.sha256}`);
    }
    writeFileSync(join(local, "build.sha"), sums.stdout);
    await onHosts(others, "distribute", async (host) => {
      // The rr engine's directory (target/release) is not synced: made first.
      if (!(await step(`${host} engine directories`, () => ssh(host, `cd ${remote} && mkdir -p ${[...new Set(engines.map((engine) => dirname(engine.path)))].join(" ")}`)))) return false;
      for (const engine of engines) if (!(await step(`${host} ${engine.path}`, () => rsync(join(local, engine.path), at(host, engine.path), [], 900)))) return false;
      if (!(await step(`${host} build.sha`, () => rsync(join(local, "build.sha"), at(host, "fanout/build.sha"), [], 300)))) return false;
      return start(host, "install.sh");
    });
    const exits = await waitFor(others, "install");
    const bad = others.filter((h) => exits.get(h) !== "0 0 0");
    if (bad.length > 0) fail(`fanout distribute: npm ci or the engines' check failed on ${bad.map((h) => `${h} (${exits.get(h)})`).join(", ")}: see ~/${remote}/fanout/install-*.log there`);
  });
  await phase("measure", async () => {
    await onHosts(o.hosts, "measure", (host) => start(host, "host.sh", [host]));
    const total = (host: string): number => plan.units.filter((u) => u.host === host).length;
    log(`measure: units per host ${o.hosts.map((h) => `${h} ${total(h)}`).join(", ")}`);
    const exits = await waitFor(o.hosts, "host", "echo measured $(find out/claims/measure -name done 2>/dev/null | wc -l) confirmed $(find out/claims/confirm -name done 2>/dev/null | wc -l)");
    for (const [host, exit] of exits) if (exit !== "0") log(`${host}: host exited ${exit}; see ~/${remote}/fanout/host.log and out/logs/ there (the report says what is missing)`);
  });
  await phase("merge", async () => {
    await onHosts(o.hosts, "pull", (host) => step(`${host} pull`, () => rsync(at(host, "out/"), `${join(o.out, "hosts", host)}/`, ["--exclude=*.store", "--exclude=one-rule.scratch", "--exclude=one-rule.stores", "--exclude=claims", "--exclude=platform"], 1800)));
    const hostDirs = o.hosts.map((h) => join(o.out, "hosts", h));
    for (const tree of ["runs", "machines", "workload"]) {
      const conflicts = mergeTrees(hostDirs.map((d) => join(d, tree)), join(o.out, tree));
      if (conflicts.length > 0) log(`merge ${tree}: ${conflicts.length} files differ between hosts, kept the first host's: ${conflicts.slice(0, 5).join(", ")}`);
    }
    writeFileSync(join(o.out, "units.jsonl"), hostDirs.map((d) => (existsSync(join(d, "units.jsonl")) ? readFileSync(join(d, "units.jsonl"), "utf8") : "")).join(""));
  });
  // The report: every phase's time and each host's measure and confirm beside it.
  const hostsTimes = Object.fromEntries(o.hosts.map((h) => [h, existsSync(join(o.out, "hosts", h, "host.json")) ? readJson(join(o.out, "hosts", h, "host.json")) : null]));
  writeFileSync(join(root, "timings.json"), JSON.stringify({ phases: state.phases, hosts: hostsTimes }, null, 1));
  const caller = run("aws", ["sts", "get-caller-identity", "--query", "Arn", "--output", "text"]);
  writeFileSync(join(o.out, "settings.json"), JSON.stringify({ hosts: o.hosts, workers: o.workers, runs: o.runs, confirm: o.confirm, ruleRuns: o.ruleRuns, fileRuns: o.fileRuns, memCap: o.memCap, timeCapS: o.timeCapS, packCaps: o.packCaps, stores: o.stores, invokedBy: caller.status === 0 ? caller.stdout.trim() : null, command: o.command }, null, 1));
  const partial = discovered.packs.some((pack) => !packs.includes(pack)) ? [`--packs ${o.requested!.join(",")}: ${packs.length} of main's ${discovered.packs.length} packs`] : [];
  const candidateOnly = discovered.candidateOnly.filter((pack) => o.requested === undefined || o.requested.includes(pack));
  return writeReport(computeVerdict(settings, packs, candidateOnly, CORPORA, o.candidates, o.candidates, partial), o.out, runMeta(o.out, packs, CORPORA));
}

// ---------------------------------------------------------------- the command line

function main(): void {
  const { values, positionals } = parseArgs({
    allowPositionals: true,
    options: {
      baseline: { type: "string" },
      candidate: { type: "string", default: SELF },
      corpora: { type: "string" },
      out: { type: "string" },
      packs: { type: "string" },
      corpus: { type: "string", default: "all" },
      runs: { type: "string", default: "1" },
      cpus: { type: "string", default: "8-11" },
      "mem-cap": { type: "string", default: "24G" },
      "time-cap": { type: "string", default: "2700" },
      "pack-cap": { type: "string", multiple: true, default: [] },
      "rr-chunk": { type: "string", default: "50" },
      candidates: { type: "string", default: "ships" },
      gate: { type: "string" },
      confirm: { type: "string", default: "3" },
      "rule-runs": { type: "string", default: "2" },
      "file-runs": { type: "string", default: "3" },
      build: { type: "boolean", default: false },
      record: { type: "boolean", default: false },
      "pack-data-cache": { type: "string" },
      waits: { type: "string", default: "W1,W2,W3,W4,W5,W6,W7,W8" },
      "wait-runs": { type: "string", default: "5" },
      replays: { type: "string", default: "2" },
      session: { type: "string" },
      work: { type: "string", default: "/tmp/shipbar-waits" },
      prepare: { type: "boolean", default: false },
      shapes: { type: "string", default: "full" },
      repo: { type: "string" },
      hosts: { type: "string" },
      name: { type: "string" },
      workers: { type: "string", default: "0-3/4-7/8-11/12-15" },
      stores: { type: "string" },
      poll: { type: "string", default: "180" },
      from: { type: "string" },
      ssh: { type: "string" },
      plan: { type: "string" },
      host: { type: "string" },
      worker: { type: "string" },
      phase: { type: "string", default: "measure" },
    },
  });
  const command = positionals[0];
  const commands = ["prepare", "run", "verdict", "waits", "fanout", "host", "work"];
  if (command === undefined || !commands.includes(command)) fail(`usage: shipbar.ts ${commands.join("|")} --baseline <ref|dir> --out <dir> [options] (see the header of scripts/ship/shipbar.ts)`);
  const preparingSession = command === "waits" && values.prepare;
  if ((values.baseline === undefined && !preparingSession) || values.out === undefined) fail("--baseline and --out are required");
  const out = resolve(values.out);
  mkdirSync(out, { recursive: true });
  const waitsManifest = readJson<WaitsManifest>(WAITS_MANIFEST);
  const repoName = values.repo ?? Object.values(waitsManifest.sessions)[0]!.repo;
  // The repository's replayed session, if it has one: a repository without one runs the step benchmarks only.
  const [sessionName, sessionSpec] = Object.entries(waitsManifest.sessions).find(([, s]) => s.repo === repoName) ?? [null, null];
  const sessionPath = sessionName === null ? null : resolve(values.session ?? join(out, "sessions", `${sessionName}.json`));
  if (preparingSession) {
    if (sessionName === null) fail(`waits.json declares no session of repo ${repoName}`);
    try {
      const session = prepareSession(WAITS_MANIFEST, sessionName, SELF, sessionPath!, values.record);
      console.log(`shipbar: session ${sessionName}: ${session.turns.length} turns, ${session.turns.reduce((sum, t) => sum + t.edits.length, 0)} edits, ${sessionPath}`);
    } catch (error) {
      fail(error instanceof Error ? error.message : String(error));
    }
    return;
  }
  const candidates = values.candidates.split(",").map((c) => c.trim()) as Candidate[];
  for (const c of candidates) if (!(c in CANDIDATES)) fail(`--candidates: no candidate ${c} (ships, rr)`);
  const gate = (values.gate?.split(",").map((c) => c.trim()) ?? candidates) as Candidate[];
  for (const c of gate) if (!candidates.includes(c)) fail(`--gate ${c} is not among --candidates`);
  const corpusKinds: Corpus[] = values.corpus === "all" ? [...CORPORA] : values.corpus === "fixtures" || values.corpus === "real" ? [values.corpus] : fail(`--corpus ${values.corpus}: fixtures, real or all`);
  const count = (name: string, value: string, least: number): number => {
    const n = Number(value);
    if (!Number.isInteger(n) || n < least) fail(`--${name} ${value} is not an integer of at least ${least}`);
    return n;
  };
  const runs = count("runs", values.runs, 1);
  const confirm = count("confirm", values.confirm, 0);
  const ruleRuns = count("rule-runs", values["rule-runs"], 2);
  const fileRuns = count("file-runs", values["file-runs"], 1);
  const workers = values.workers.split("/").map((set) => set.trim()).filter((set) => set !== "");
  const measureOptions = (cpus: string): MeasureOptions => ({ cpus, memCap: values["mem-cap"], timeCapS: Number(values["time-cap"]), packCaps: values["pack-cap"], confirm, ruleRuns, fileRuns, rrChunk: count("rr-chunk", values["rr-chunk"], 0), stores: values.stores === undefined ? undefined : resolve(values.stores) });
  if (command === "fanout") {
    const hosts = values.hosts?.split(",").map((h) => h.trim()).filter((h) => h !== "") ?? [];
    if (hosts.length === 0) fail("fanout: --hosts h1,h2 is required");
    const name = values.name ?? `shipbar-${new Date().toISOString().slice(0, 10)}`;
    const candidateDir = resolve(values.candidate);
    void fanoutRun({
      hosts, name, workers, baselineSpec: values.baseline!, packDataCache: values["pack-data-cache"], candidate: { dir: candidateDir, sha: checkoutSha(candidateDir) }, out,
      corpora: resolve(values.corpora ?? join(out, "corpora")), requested: values.packs?.split(",").map((p) => p.trim()), runs, confirm, ruleRuns, fileRuns,
      memCap: values["mem-cap"], timeCapS: Number(values["time-cap"]), packCaps: values["pack-cap"], stores: values.stores ?? `/dev/shm/shipbar-${name}`,
      pollS: count("poll", values.poll, 10), from: values.from, shell: remoteShell(values.ssh), candidates, command: ["npm run bench:ship --", ...process.argv.slice(2)].join(" "),
    }).then((code) => {
      process.exitCode = code;
    });
    return;
  }
  // Pack runs measure on every platform (platform.ts); the waits pin with taskset and drop the page cache: Linux only.
  if (command === "waits" && process.platform !== "linux") fail(`waits measure on Linux only (taskset, the page cache drop); this is ${process.platform}`);
  const platform = command === "run" || command === "waits" || command === "work" ? requirePlatform(out) : undefined;

  const baseline = resolveBaseline(values.baseline!, out, values["pack-data-cache"]);
  const candidate: Checkout = { dir: resolve(values.candidate), sha: checkoutSha(resolve(values.candidate)) };
  const discovered = discoverPacks(baseline.dir, candidate.dir);
  const requested = values.packs?.split(",").map((p) => p.trim());
  for (const pack of requested ?? []) if (!discovered.packs.includes(pack) && !discovered.candidateOnly.includes(pack)) fail(`--packs: main ships no pack ${pack}`);
  const packs = discovered.packs.filter((pack) => requested === undefined || requested.includes(pack));
  const candidateOnly = discovered.candidateOnly.filter((pack) => requested === undefined || requested.includes(pack));
  // A run narrowed to some packs or one corpus is partial: its verdict says so and is never PASS.
  const partial = [
    ...(discovered.packs.some((pack) => !packs.includes(pack)) ? [`--packs ${values.packs}: ${packs.length} of main's ${discovered.packs.length} packs`] : []),
    ...(corpusKinds.length < CORPORA.length ? [`--corpus ${values.corpus}: slice A and B on ${corpusKinds.join(", ")} only${corpusKinds.includes("real") ? "" : ", no slice C"}`] : []),
  ];
  const corpora = resolve(values.corpora ?? join(out, "corpora"));
  if (command === "prepare") {
    if (!existsSync(join(candidate.dir, ".git")) || run("git", ["-C", candidate.dir, "rev-parse", "HEAD"]).status !== 0) fail(`${candidate.dir} is not a git checkout`);
    mkdirSync(join(candidate.dir, ".rerail"), { recursive: true });
    writeFileSync(join(candidate.dir, SOURCE_FILE), JSON.stringify({ sha: candidate.sha }));
    prepareCorpora(baseline, packs, corpora, values.record);
    console.log(`shipbar: prepared baseline ${baseline.sha} (${baseline.dir}), candidate ${candidate.sha}, corpora ${corpora}`);
    return;
  }
  const settingsBase = { baseline, candidate, corpora, out, runs };
  if (command === "host" || command === "work") {
    if (values.plan === undefined || values.host === undefined) fail(`${command}: --plan and --host are required`);
    const plan = readJson<Plan>(resolve(values.plan));
    if (command === "work") {
      const worker = count("worker", values.worker ?? "", 0);
      if (values.phase !== "measure" && values.phase !== "confirm") fail(`--phase ${values.phase}: measure or confirm`);
      workUnits(openSettings(settingsBase, platform!, plan.candidates.includes("rr") ? engineBinary(candidate.dir) : undefined, packs, measureOptions(values.cpus)), plan, values.host, worker, values.phase);
      return;
    }
    const forward = [
      ...["--baseline", baseline.dir, "--candidate", candidate.dir, "--corpora", corpora, "--out", out, "--plan", resolve(values.plan), "--host", values.host],
      ...["--runs", String(runs), "--confirm", String(confirm), "--rule-runs", String(ruleRuns), "--file-runs", String(fileRuns), "--mem-cap", values["mem-cap"], "--time-cap", values["time-cap"]],
      ...values["pack-cap"].flatMap((cap) => ["--pack-cap", cap]),
      ...(values.stores === undefined ? [] : ["--stores", resolve(values.stores)]),
    ];
    void hostRun(settingsBase, plan, values.host, workers, forward).then((code) => {
      process.exitCode = code;
    });
    return;
  }
  if (command === "waits") {
    const repo = waitsManifest.repos[repoName] ?? fail(`waits.json declares no repo ${repoName}`);
    const corpusDir = join(corpora, "real", repo.corpus);
    const corpus = hashTree(corpusDir);
    if (corpus.sha256 !== readJson<Manifest>(MANIFEST).corpora[repo.corpus]?.sha256) fail(`real-code corpus ${repo.corpus} at ${corpusDir} does not match corpora.json: run prepare`);
    for (const dir of [baseline.dir, candidate.dir]) {
      for (const entry of ["dist/cli/cli.js", "dist/hosts/claude/claude-hook.js", "dist/ui"]) if (!existsSync(join(dir, entry))) fail(`${dir} has no ${entry}; run there: ${PRODUCT_BUILD} && npm run build:js`);
    }
    const shapes = values.shapes.split(",").map((s) => s.trim());
    for (const shape of shapes) if (!(shape in SHAPES)) fail(`--shapes: no shape ${shape} (${Object.keys(SHAPES).join(", ")})`);
    const scenarios = values.waits.split(",").flatMap((token) => {
      const named = SCENARIOS.filter((s) => `${s.wait}/${s.scenario}` === token.trim());
      // A wait named whole takes its step scenarios in --shapes only.
      const ids = (named.length > 0 ? named : SCENARIOS.filter((s) => s.wait === token.trim() && (s.shape === undefined || shapes.includes(s.shape)))).map(scenarioId);
      return ids.length > 0 ? ids : fail(`--waits: no wait or scenario ${token} (W1…W8, or W5/scan-warm)`);
    });
    let session = null;
    if (sessionSpec !== null) {
      try {
        session = loadSession(sessionSpec, sessionName!, sessionPath!);
      } catch (error) {
        fail(error instanceof Error ? error.message : String(error));
      }
    } else if (scenarios.some((id) => !/\/steps-/.test(id))) {
      console.log(`shipbar: repo ${repoName} has no replayed session in waits.json: it runs the step benchmarks only`);
      scenarios.splice(0, scenarios.length, ...scenarios.filter((id) => /\/steps-/.test(id)));
      if (scenarios.length === 0) fail(`--waits names no step benchmark scenario, the only ones repo ${repoName} runs`);
    }
    let cpus: number[] = [];
    let siblings: number[] = [];
    try {
      cpus = cpuList(values.cpus);
      siblings = platform!.siblings(cpus);
    } catch (error) {
      fail(`--cpus ${values.cpus}: ${error instanceof Error ? error.message : String(error)}`);
    }
    mkdirSync(join(out, "machines"), { recursive: true });
    writeFileSync(join(out, "machines", `${hostname()}.json`), JSON.stringify(platform!.machine(), null, 1));
    runWaits(
      {
        checkouts: { baseline: { dir: baseline.dir, sha: baseline.sha }, candidate: { dir: candidate.dir, sha: candidate.sha } },
        out, repoName, repo, corpusDir, corpus, sessionName, session, sessionSha256: sessionSpec?.sha256 ?? null,
        cpus, siblings, runs: count("wait-runs", values["wait-runs"], 1), replays: count("replays", values.replays, 1), confirm, scenarios,
        work: resolve(values.work), limits: waitsManifest.limits, reportOnly: waitsManifest.reportOnly, command: ["npm run bench:ship --", ...process.argv.slice(2)].join(" "),
      },
      { cpuTimes: () => parseProcStat(readFileSync("/proc/stat", "utf8")), busiest },
    );
  }
  if (command === "run") {
    if (!existsSync(join(corpora, "corpora.lock.json"))) prepareCorpora(baseline, packs, corpora, values.record);
    const lock = verifyCorpora(corpora, packs, corpusKinds);
    requireBuilt(baseline.dir, { product: true, engine: false }, values.build);
    const { engine } = requireBuilt(candidate.dir, { product: candidates.includes("ships"), engine: candidates.includes("rr") }, values.build);
    const settings = openSettings(settingsBase, platform!, engine, packs, measureOptions(values.cpus));
    // Who runs it: the AWS identity this shell is signed in as, the one the pack data pull reads with.
    const caller = run("aws", ["sts", "get-caller-identity", "--query", "Arn", "--output", "text"]);
    const invokedBy = caller.status === 0 && caller.stdout.trim() !== "" ? caller.stdout.trim() : null;
    const command = ["npm run bench:ship --", ...process.argv.slice(2)].join(" ");
    writeFileSync(join(out, "settings.json"), JSON.stringify({ ...settings, candidates, corpusKinds, packs, productBuild: PRODUCT_BUILD, engineBuild: ENGINE_BUILD, invokedBy, command }, null, 1));
    mkdirSync(join(out, "machines"), { recursive: true });
    writeFileSync(join(out, "machines", `${hostname()}.json`), JSON.stringify(platform!.machine(), null, 1));
    const sides: Side[] = ["main", ...candidates];
    for (const pack of packs) {
      const rr = candidates.includes("rr") ? packRrRules(settings, pack) : undefined;
      for (const corpus of corpusKinds) {
        const fixtures = lock.fixtures[pack];
        if (corpus === "fixtures" && fixtures !== undefined && "absent" in fixtures) continue;
        const dir = corpusDirectory(corpora, pack, corpus);
        const cell = join(out, "runs", pack, corpus);
        cachedWorkload(out, relative(corpora, dir), dir, engine ?? engineBinary(candidate.dir));
        measureCell(settings, cell, dir, pack, repetitions(runs), sides, rr);
        measureFixed(settings, pack, corpus, sides, repetitions(runs));
        confirmCell(settings, cell, dir, pack, candidates, rr);
        measureOneRule(settings, pack, corpus, candidates, rr);
      }
      if (corpusKinds.includes("real")) for (const file of pickOneFiles(settings, pack)) measureOneFile(settings, pack, file, candidates, rr);
    }
  }
  // A directory with waits and no pack runs reports the waits alone.
  const packsMeasured = command === "run" || existsSync(join(out, "runs"));
  const reported = packsMeasured ? candidates : [];
  process.exitCode = writeReport(computeVerdict(settingsBase, packsMeasured ? packs : [], packsMeasured ? candidateOnly : [], corpusKinds, reported, packsMeasured ? gate : [], partial), out, runMeta(out, packsMeasured ? packs : [], corpusKinds));
}

main();
