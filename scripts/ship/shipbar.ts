#!/usr/bin/env node
// The shippability bar: a candidate engine is no worse than main's on every rule pack main ships. One run gives two
// verdicts, one per candidate, each PASS or FAIL per pack, corpus and metric (docs in scripts/README.md, "ship/"):
//
//   ships  the candidate checkout's engine on its packs as they ship (packs/<pack>/rules/*.json), through the product's
//          own path (conformance/languages/runner.ts evaluateWorkspace, as the candidate has it: product-run.ts);
//   rr     the candidate's engine on the hand-written `.rr` rewrites, docs/rule-syntax/packs/<pack>/<id>.rr, through
//          `rerail-engine run-policies <corpus> <out> <rule.rr>...`;
//   main   the baseline: main's checkout on its shipped packs/<pack>/rules/*.json, through its own product path.
//
// Commands (each prints what it refuses and why; nothing is measured unpinned):
//
//   node --import tsx scripts/ship/shipbar.ts prepare --baseline <git ref | dir> --out <dir> [--packs a,b] [--record]
//       the baseline checkout (a git ref is exported to <out>/baseline with its pack data pulled) and the pinned corpora
//       in <out>/corpora: each pack's fixtures from its upstream recordings, and its real-code corpus from
//       scripts/ship/corpora.json, whose file count and hash the corpus must match (--record writes them there).
//       Needs git and the pack data cache; runs anywhere.
//   node --import tsx scripts/ship/shipbar.ts run --baseline <dir | ref> --out <dir> [--candidate <dir>] [--corpora <dir>]
//       [--packs a,b] [--corpus fixtures|real|all] [--runs 3] [--cpus 8-11] [--mem-cap 24G] [--time-cap 2700]
//       [--candidates ships,rr] [--gate ships,rr] [--build]
//       Linux only (taskset, GNU time, a systemd user manager for the memory cap): prepares what is missing, measures
//       every pack and corpus, then writes the verdict. Resumable: a run already in runs.jsonl is not made again.
//   node --import tsx scripts/ship/shipbar.ts verdict --baseline <dir> --out <dir> [--candidate <dir>] [--corpora <dir>]
//       [--packs a,b] [--candidates ships,rr] [--gate ships,rr]
//       verdict.json and report.md from the runs under <out>/runs (several runs' directories merged, say).
//
// Exit status of run and verdict: 0 only when every candidate in --gate (default: every one in --candidates) passes.
//
// Method. Per pack and corpus, each side runs N times, interleaved (the side order rotates each repetition), all on
// the same --cpus (one hardware thread per physical core, the cores' other threads left idle and their load recorded),
// each process under the same memory cgroup cap and time cap. Cold: a fresh, empty RERAIL_TAPE_CACHE store and engine
// TMPDIR; warm: a second process on that store right after. A side that fails a cold run makes no further run of that
// pack and corpus. Each metric is the median of the side's completed runs: wall, CPU (user + sys) and peak RSS, cold
// and warm, each failing above 1.10 times main's. Findings come from each side's first cold run on the same corpus
// directory and are scored by one function (verdict.ts) against the tool's upstream recording with main's
// divergences.json, on (rule, path, line, code) keys: the candidate keeps every true positive of main's and adds no
// false positive main lacks. Real code has no truth: its agreement is reported and does not gate.

import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { appendFileSync, closeSync, copyFileSync, existsSync, mkdirSync, openSync, readdirSync, readFileSync, renameSync, rmSync, statSync, symlinkSync, writeFileSync } from "node:fs";
import { hostname } from "node:os";
import { dirname, join, matchesGlob, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { parseArgs } from "node:util";
import { renderReport } from "./report.js";
import {
  agreement,
  compareCompletion,
  compareFindings,
  compareMetric,
  type CompletionVerdict,
  type Divergence,
  findingKey,
  type FindingsVerdict,
  type MetricVerdict,
  missingRules,
  newRuleFailures,
  overall,
  type RunOutcome,
  scoreFindings,
  type Status,
  TOLERANCE_PERCENT,
} from "./verdict.js";

const HERE = dirname(fileURLToPath(import.meta.url));
/** The checkout this script is in: the default candidate. */
const SELF = resolve(HERE, "..", "..");
const MANIFEST = join(HERE, "corpora.json");

export const CANDIDATES = { ships: "the candidate's packs as they ship, through the product's path", rr: "the hand-written .rr rewrites, through rerail-engine run-policies" } as const;
type Candidate = keyof typeof CANDIDATES;
type Side = "main" | Candidate;
type Corpus = "fixtures" | "real";
const CORPORA: readonly Corpus[] = ["fixtures", "real"];
export const METRICS = ["wall-cold", "wall-warm", "cpu-cold", "cpu-warm", "rss-cold", "rss-warm"] as const;

const PRODUCT_BUILD = "npm ci --ignore-scripts && npm run build:native";
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
      else if (entry.isFile()) out.push(relative(root, full));
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

/** The baseline: an existing checkout, or a git ref of this repository exported to <out>/baseline (pack data pulled). */
function resolveBaseline(spec: string, out: string, packDataCache: string | undefined): Checkout {
  if (existsSync(join(spec, "packs"))) return { dir: resolve(spec), sha: checkoutSha(resolve(spec)) };
  const parsed = run("git", ["-C", SELF, "rev-parse", "--verify", `${spec}^{commit}`]);
  if (parsed.status !== 0) fail(`--baseline ${spec} is neither a checkout (no packs/) nor a commit of ${SELF}`);
  const sha = parsed.stdout.trim();
  const dir = join(out, "baseline");
  if (existsSync(join(dir, SOURCE_FILE)) && readJson<{ sha: string }>(join(dir, SOURCE_FILE)).sha === sha) return { dir, sha };
  if (existsSync(dir)) fail(`${dir} holds another baseline; remove it or pass it as --baseline`);
  mkdirSync(dir, { recursive: true });
  const tar = join(out, "baseline.tar");
  if (run("git", ["-C", SELF, "archive", "-o", tar, sha]).status !== 0 || run("tar", ["-xf", tar, "-C", dir]).status !== 0) fail(`could not export ${sha} to ${dir}`);
  rmSync(tar);
  mkdirSync(join(dir, ".rerail"), { recursive: true });
  writeFileSync(join(dir, SOURCE_FILE), JSON.stringify({ sha, ref: spec }));
  // The pack data: the archives the locks name, from the cache a pull keeps, verified and extracted by main's own pull.
  const cache = packDataCache ?? join(dirname(run("git", ["-C", SELF, "rev-parse", "--path-format=absolute", "--git-common-dir"]).stdout.trim()), ".rerail", "pack-data", "archives");
  mkdirSync(join(dir, ".rerail", "pack-data"), { recursive: true });
  if (existsSync(cache)) symlinkSync(cache, join(dir, ".rerail", "pack-data", "archives"));
  console.log(`shipbar: baseline ${sha} exported to ${dir}; pulling its pack data`);
  const pull = spawnSync("node", ["scripts/s3/pack-fixtures.mjs", "pull"], { cwd: dir, stdio: "inherit" });
  if (pull.status !== 0) fail(`the baseline's pack data pull failed; run \`node scripts/s3/pack-fixtures.mjs pull\` in ${dir}`);
  return { dir, sha };
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

function cpuList(spec: string): number[] {
  const cpus = new Set<number>();
  for (const part of spec.split(",")) {
    const [from, to] = part.split("-").map(Number);
    if (from === undefined || Number.isNaN(from) || (to !== undefined && Number.isNaN(to))) fail(`--cpus ${spec} is not a CPU list`);
    for (let cpu = from; cpu <= (to ?? from); cpu += 1) cpus.add(cpu);
  }
  return [...cpus].sort((a, b) => a - b);
}

/** The pinned CPUs' hyperthread siblings, which must stay idle; refuses a set holding two threads of one core. */
function idleSiblings(cpus: readonly number[]): number[] {
  const siblings = new Set<number>();
  for (const cpu of cpus) {
    const path = `/sys/devices/system/cpu/cpu${cpu}/topology/thread_siblings_list`;
    if (!existsSync(path)) fail(`no CPU ${cpu} (${path})`);
    for (const sibling of cpuList(readFileSync(path, "utf8").trim())) {
      if (sibling === cpu) continue;
      if (cpus.includes(sibling)) fail(`--cpus holds CPUs ${cpu} and ${sibling}, two threads of one core; pin one thread per physical core`);
      siblings.add(sibling);
    }
  }
  return [...siblings].sort((a, b) => a - b);
}

/** Per CPU, busy and total jiffies from /proc/stat. */
function cpuTimes(): Map<number, { busy: number; total: number }> {
  const times = new Map<number, { busy: number; total: number }>();
  for (const line of readFileSync("/proc/stat", "utf8").split("\n")) {
    const match = /^cpu(\d+)\s+(.*)$/.exec(line);
    if (match === null) continue;
    const fields = match[2]!.split(/\s+/).map(Number);
    const total = fields.slice(0, 8).reduce((a, b) => a + b, 0);
    times.set(Number(match[1]), { busy: total - (fields[3] ?? 0) - (fields[4] ?? 0), total });
  }
  return times;
}

function busiest(before: Map<number, { busy: number; total: number }>, after: Map<number, { busy: number; total: number }>, cpus: readonly number[]): number {
  let most = 0;
  for (const cpu of cpus) {
    const a = before.get(cpu);
    const b = after.get(cpu);
    if (a === undefined || b === undefined || b.total === a.total) continue;
    most = Math.max(most, (b.busy - a.busy) / (b.total - a.total));
  }
  return Math.round(most * 1000) / 1000;
}

/** Linux, taskset, GNU time, and a systemd user manager to cap memory: refuses rather than measure unpinned. */
function requirePlatform(): void {
  if (process.platform !== "linux") fail(`run measures on Linux only (taskset, GNU time, cgroups); this is ${process.platform}. Run \`prepare\` here and \`run\` on a Linux machine.`);
  if (run("taskset", ["-c", "0", "true"]).status !== 0) fail("taskset (util-linux) is missing");
  const time = run("/usr/bin/time", ["--version"]);
  if (!/GNU/.test(time.stdout + time.stderr)) fail("/usr/bin/time is not GNU time");
  if (run("systemd-run", ["--user", "--scope", "--quiet", "-p", "MemoryMax=1G", "true"]).status !== 0) fail("systemd-run --user --scope does not work here (no user manager?); it caps each run's memory");
}

// ---------------------------------------------------------------- builds

function requireBuilt(checkout: string, needs: { product: boolean; engine: boolean }, build: boolean): { engine: string | undefined } {
  const addon = (): boolean => existsSync(join(checkout, "native")) && readdirSync(join(checkout, "native")).some((name) => /^rerail_engine\..+\.node$/.test(name));
  const tsx = (): boolean => existsSync(join(checkout, "node_modules", "tsx"));
  const engine = (): string | undefined => [join(checkout, "target", "release", "rerail-engine"), join(checkout, "native", "target", "release", "rerail-engine")].find((path) => existsSync(path));
  const steps: string[] = [];
  if (needs.product && (!addon() || !tsx())) steps.push(PRODUCT_BUILD);
  if (needs.engine && engine() === undefined) steps.push(ENGINE_BUILD);
  if (steps.length > 0 && !build) fail(`${checkout} is not built; run there (both release profile): ${steps.join(" && ")}\n  or pass --build`);
  for (const step of steps) {
    console.log(`shipbar: building ${checkout}: ${step}`);
    if (spawnSync("bash", ["-c", step], { cwd: checkout, stdio: "inherit" }).status !== 0) fail(`build failed in ${checkout}: ${step}`);
  }
  return { engine: needs.engine ? engine() : undefined };
}

// ---------------------------------------------------------------- the .rr side's rules

interface RrRules {
  /** Per rule main ships: its `.rr` path and policy name, or why it cannot run. */
  rules: Record<string, { path?: string; policy?: string; unavailable: string | null }>;
}

/** Each rule main ships, mapped to its `.rr` rewrite on the candidate, checked by the candidate's `check-rr`. */
function rrRules(candidate: string, engine: string, mainRules: readonly string[], pack: string): RrRules {
  const rules: RrRules["rules"] = {};
  const present: string[] = [];
  for (const rule of mainRules) {
    const path = join(candidate, "docs", "rule-syntax", "packs", pack, `${rule}.rr`);
    if (!existsSync(path)) {
      rules[rule] = { unavailable: "no .rr counterpart" };
      continue;
    }
    const policy = /^\.policy\s+"([^"]+)"/m.exec(readFileSync(path, "utf8"))?.[1];
    if (policy === undefined) {
      rules[rule] = { path, unavailable: "the .rr has no .policy line" };
      continue;
    }
    rules[rule] = { path, policy, unavailable: null };
    present.push(rule);
  }
  if (present.length > 0) {
    const checked = run(engine, ["check-rr", ...present.map((rule) => rules[rule]!.path!)], { cwd: candidate });
    let rows: { path: string; status: string; error?: string }[] = [];
    try {
      rows = JSON.parse(checked.stdout);
    } catch {
      fail(`check-rr gave no report for ${pack}: ${lastLine(checked.stderr)}`);
    }
    const byPath = new Map(rows.map((row) => [resolve(candidate, row.path), row]));
    for (const rule of present) {
      const row = byPath.get(resolve(rules[rule]!.path!));
      if (row === undefined) rules[rule]!.unavailable = "check-rr did not report it";
      else if (row.status !== "reader-passed") rules[rule]!.unavailable = `check-rr ${row.status}: ${(row.error ?? "").slice(0, 200)}`;
    }
  }
  return { rules };
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
  exit: number;
  wallMs: number;
  cpuMs: number;
  rssKB: number;
  /** /proc/loadavg's one-minute figure as the run started. */
  load: number;
  /** The busiest pinned CPU and the busiest idle sibling over the run (fraction busy). */
  pinnedBusy: number;
  siblingBusy: number;
  cpus: string;
  threads: number;
  host: string;
  memCap: string;
  timeCapS: number;
  command: string;
  started: string;
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
  memCap: string;
  timeCapS: number;
}

function readRows(path: string): RunRow[] {
  return existsSync(path) ? readFileSync(path, "utf8").split("\n").filter((line) => line !== "").map((line) => JSON.parse(line) as RunRow) : [];
}

/** <cell>/rr-excluded.json: the `.rr` rules that failed alone on this corpus, each with why, left out of its runs. */
function readExcluded(cell: string): Record<string, string> {
  const path = join(cell, "rr-excluded.json");
  return existsSync(path) ? readJson(path) : {};
}

/** The `.rr` rules a cell runs: every runnable rewrite of a rule main ships, less those that failed alone here. */
function rrIds(rr: RrRules, cell: string): string[] {
  const excluded = readExcluded(cell);
  return Object.entries(rr.rules).filter(([id, rule]) => rule.unavailable === null && excluded[id] === undefined).map(([id]) => id);
}

/** The side's command and its working directory. */
function sideCommand(settings: Settings, side: Side, pack: string, corpus: string, result: string, rundir: string, rrPaths: readonly string[]): { command: string[]; cwd: string } {
  if (side === "rr") return { command: [settings.engine!, "run-policies", corpus, rundir, ...rrPaths], cwd: settings.candidate.dir };
  const checkout = side === "main" ? settings.baseline.dir : settings.candidate.dir;
  return { command: ["node", "--import", "tsx", join(HERE, "product-run.ts"), checkout, corpus, result, join(checkout, "packs", pack, "rules")], cwd: checkout };
}

interface Process {
  side: Side;
  mode: "cold" | "warm";
  rep: number;
  tag: string;
  /** The store directory: emptied for a cold run, reused by the warm run after it. */
  store: string;
  /** The cell file its row goes to: runs.jsonl, or alone.jsonl for a `.rr` rule's run alone. */
  log: string;
  /** run-policies: the rule ids it runs, in order. */
  rules: readonly string[];
}

/** One process of one side, pinned and capped, measured by GNU time; its row is appended to the cell's log. */
function measure(settings: Settings, cell: string, p: Process, pack: string, corpus: string, rr: RrRules | undefined): RunRow {
  const { side, mode, rep, tag, store } = p;
  if (mode === "cold") {
    rmSync(store, { recursive: true, force: true });
    mkdirSync(join(store, "tmp"), { recursive: true });
  }
  const result = join(cell, `${tag}.json`);
  const rundir = join(cell, `${tag}.out`);
  rmSync(rundir, { recursive: true, force: true });
  const { command, cwd } = sideCommand(settings, side, pack, corpus, result, rundir, p.rules.map((id) => rr!.rules[id]!.path!));
  const env: Record<string, string> = {};
  for (const [key, value] of Object.entries(process.env)) if (value !== undefined && !key.startsWith("RERAIL_") && !key.startsWith("SHIPBAR_")) env[key] = value;
  env.RERAIL_TAPE_CACHE = join(store, "store.sqlite");
  if (side === "rr") env.TMPDIR = join(store, "tmp");
  else env.SHIPBAR_ENGINE_TMPDIR = join(store, "tmp");
  const timeFile = join(cell, `${tag}.time`);
  const stderrFile = join(cell, `${tag}.stderr`);
  const full = [
    "-c", settings.cpus.join(","),
    "/usr/bin/time", "-f", "%e %U %S %M", "-o", timeFile,
    // GNU time stays outside the cgroup, so a run the cap kills is still measured (time to the kill, peak RSS).
    "systemd-run", "--user", "--scope", "--quiet", "-p", `MemoryMax=${settings.memCap}`, "-p", "MemorySwapMax=0",
    "timeout", "-s", "KILL", String(settings.timeCapS),
    ...command,
  ];
  const load = Number(readFileSync("/proc/loadavg", "utf8").split(" ")[0]);
  const before = cpuTimes();
  const started = new Date().toISOString();
  const stdout = openSync(side === "rr" ? result : join(cell, `${tag}.stdout`), "w");
  const stderr = openSync(stderrFile, "w");
  const child = spawnSync("taskset", full, { cwd, env, stdio: ["ignore", stdout, stderr] });
  closeSync(stdout);
  closeSync(stderr);
  const after = cpuTimes();
  const timeLine = lastLine(existsSync(timeFile) ? readFileSync(timeFile, "utf8") : "0 0 0 0");
  const [wall = 0, user = 0, sys = 0, rss = 0] = timeLine.split(/\s+/).map(Number);
  const exit = child.status ?? 128 + 9;
  const errorText = readFileSync(stderrFile, "utf8");
  let completed = exit === 0;
  let reason: string | null = null;
  let stoppedOn: string | null = null;
  if (completed) {
    try {
      readJson(result);
    } catch {
      completed = false;
      reason = "exited 0 with no readable result";
    }
  }
  let culprit: string | null = null;
  if (exit !== 0) {
    reason =
      exit === 137 || exit === 128 + 9
        ? `killed at ${wall.toFixed(0)} s, ${(rss / 1048576).toFixed(1)} GiB peak RSS (${wall >= settings.timeCapS - 1 ? `time cap ${settings.timeCapS} s` : `memory cap ${settings.memCap}`})`
        : `exit ${exit}: ${lastLine(errorText)}`;
    if (side === "rr") {
      const started = existsSync(rundir) ? readdirSync(rundir).filter((name) => /^\d+$/.test(name)).length : 0;
      const named = /\/([^/]+)\.rr: /.exec(errorText)?.[1];
      culprit = named !== undefined && p.rules.includes(named) ? named : (p.rules[started - 1] ?? null);
      stoppedOn = `${culprit ?? "before the first policy"} (policy ${started} of ${p.rules.length})`;
    } else {
      stoppedOn = [...errorText.matchAll(/^shipbar: phase (\S+)/gm)].at(-1)?.[1] ?? "start-up";
    }
  }
  const row: RunRow = {
    tag, side, mode, rep, completed, reason, stoppedOn, culprit, rules: side === "rr" ? p.rules.length : null, exit,
    wallMs: Math.round(wall * 1000), cpuMs: Math.round((user + sys) * 1000), rssKB: rss,
    load, pinnedBusy: busiest(before, after, settings.cpus), siblingBusy: busiest(before, after, settings.siblings),
    cpus: settings.cpus.join(","), threads: settings.cpus.length, host: hostname(),
    memCap: settings.memCap, timeCapS: settings.timeCapS, command: command.join(" "), started,
  };
  appendFileSync(join(cell, p.log), `${JSON.stringify(row)}\n`);
  // Kept: the first cold run's findings (and every failure's stderr). The rest is scratch.
  rmSync(rundir, { recursive: true, force: true });
  rmSync(timeFile, { force: true });
  rmSync(join(cell, `${tag}.stdout`), { force: true });
  if (tag === `${side}.cold1` && p.log === "runs.jsonl" && completed) renameSync(result, join(cell, `${side}.findings.json`));
  else rmSync(result, { force: true });
  if (completed) rmSync(stderrFile, { force: true });
  console.log(`shipbar: ${pack} ${relative(settings.corpora, corpus)} ${tag}: ${completed ? "ok" : `FAILED (${reason})`} wall ${wall}s cpu ${(user + sys).toFixed(2)}s rss ${(rss / 1024).toFixed(0)}MB`);
  return row;
}

/**
 * After a `.rr` cold run that exits with an error, the rule it stopped on is run alone on the same corpus under the
 * same caps. Failing alone too, it is left out of this corpus's runs with that reason (rr-excluded.json), and the
 * caller runs the rest again; running alone, the failure belongs to running the pack's rules together and stands. A
 * content store limit is the batch's by nature, and a run the caps killed is not tried again (each try could take the
 * whole time cap): both stand as the pack's failure, with the rule it stopped on.
 */
function excludeIfFailsAlone(settings: Settings, cell: string, pack: string, corpus: string, rr: RrRules, failed: RunRow): boolean {
  if (failed.culprit === null || failed.exit === 137 || /content store hard limit/.test(failed.reason ?? "")) return false;
  const store = join(cell, "rr.alone.store");
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

/** One pack and corpus: N repetitions, the side order rotating, each side cold then warm on its own fresh store. */
function measureCell(settings: Settings, pack: string, corpus: Corpus, sides: readonly Side[], rr: RrRules | undefined): void {
  const cell = join(settings.out, "runs", pack, corpus);
  mkdirSync(cell, { recursive: true });
  const dir = corpusDirectory(settings.corpora, pack, corpus);
  for (let rep = 1; rep <= settings.runs; rep += 1) {
    const order = sides.map((_, i) => sides[(i + rep - 1) % sides.length]!);
    for (const side of order) {
      const store = join(cell, `${side}.r${rep}.store`);
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
}

// ---------------------------------------------------------------- the verdict

interface Failure {
  pack: string;
  corpus: Corpus | null;
  item: string;
  reason: string;
}

interface CellVerdict {
  corpus: Corpus;
  status: Status;
  note: string;
  completion: (CompletionVerdict & { stoppedOn: string | null }) | null;
  ruleFailures: { rule: string; reason: string }[];
  metrics: MetricVerdict[];
  /** fixtures: the set-wise findings bar, with each side's TP/FP/FN; real: agreement only. */
  findings: (Omit<FindingsVerdict, "lostTp" | "newFp"> & { lostTp: number; newFp: number; samples: string[]; main: Counts; candidate: Counts }) | null;
  agreement: { both: number; mainOnly: number; candidateOnly: number; samples: string[] } | null;
  load: { main: number[]; candidate: number[] };
  siblingBusyMax: number;
  runs: { main: number; candidate: number };
  /** How many rules each side ran: main's shipped rules; the candidate's shipped rules, or the `.rr` rules that run. */
  rules: { main: number; candidate: number };
}

interface Counts {
  tp: number;
  fp: number;
  fn: number;
  explainedFp: number;
  explainedFn: number;
}

interface PackVerdict {
  pack: string;
  status: Status | "no-baseline";
  missing: { rule: string; reason: string }[];
  extra: string[];
  cells: CellVerdict[];
}

export interface CandidateVerdict {
  candidate: Candidate;
  description: string;
  status: "PASS" | "FAIL" | "INCOMPLETE";
  failures: Failure[];
  packs: PackVerdict[];
}

export interface Verdict {
  generated: string;
  tolerancePercent: number;
  runs: number;
  baseline: Checkout;
  candidate: Checkout;
  corpora: CorporaLock;
  machine: { hosts: string[]; cpus: string[]; threads: number[]; memCap: string[]; timeCapS: number[] };
  candidates: CandidateVerdict[];
  gate: Candidate[];
}

/** A side's findings from its first cold run, as keys: [rule, path, line, code]. */
function sideFindings(cell: string, side: Side, rr: RrRules | undefined): Set<string> | undefined {
  const path = join(cell, `${side}.findings.json`);
  if (!existsSync(path)) return undefined;
  const keys = new Set<string>();
  if (side === "rr") {
    const policyToRule = new Map(Object.entries(rr?.rules ?? {}).filter(([, rule]) => rule.policy !== undefined).map(([id, rule]) => [rule.policy!, id]));
    for (const d of readJson<{ diagnostics: { policy: string; path: string; line: number; code: string }[] }>(path).diagnostics) {
      keys.add(findingKey(policyToRule.get(d.policy) ?? d.policy, d.path, d.line, d.code));
    }
  } else {
    for (const [rule, file, line, code] of readJson<{ findings: [string, string, number, string][] }>(path).findings) keys.add(findingKey(rule, file, line, code));
  }
  return keys;
}

function ruleFailuresOf(cell: string, side: Side): Map<string, string> {
  const path = join(cell, `${side}.findings.json`);
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
        return meta !== undefined && covered(meta, path!);
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

function cellVerdict(settings: { baseline: Checkout; candidate: Checkout; corpora: string; out: string; runs: number }, lock: CorporaLock, pack: string, corpus: Corpus, candidate: Candidate, rr: RrRules | undefined): CellVerdict {
  const empty: CellVerdict = { corpus, status: "not-compared", note: "", completion: null, ruleFailures: [], metrics: [], findings: null, agreement: null, load: { main: [], candidate: [] }, siblingBusyMax: 0, runs: { main: 0, candidate: 0 }, rules: { main: 0, candidate: 0 } };
  const fixtures = lock.fixtures[pack];
  if (corpus === "fixtures" && fixtures !== undefined && "absent" in fixtures) return { ...empty, note: `no fixtures corpus: ${fixtures.absent}` };
  const cell = join(settings.out, "runs", pack, corpus);
  const all = readRows(join(cell, "runs.jsonl"));
  const main = all.filter((row) => row.side === "main");
  const mine = all.filter((row) => row.side === candidate);
  if (main.length === 0) return { ...empty, note: "not measured" };
  if (candidate === "rr" && rr !== undefined && rrIds(rr, cell).length === 0) {
    return { ...empty, status: "fail", note: "no rule of the pack runs on the .rr side on this corpus" };
  }
  if (mine.length === 0) return { ...empty, note: "candidate not measured" };
  const outcome = (rows: RunRow[]): RunOutcome[] => rows.map((row) => (row.completed ? { completed: true } : { completed: false, reason: `${row.tag}: ${row.reason ?? ""}${row.stoppedOn === null ? "" : `; stopped on ${row.stoppedOn}`}` }));
  const completion = { ...compareCompletion(outcome(main), outcome(mine), 2 * settings.runs), stoppedOn: mine.find((row) => !row.completed)?.stoppedOn ?? null };
  const ruleFailures = candidate === "rr" ? [] : newRuleFailures(ruleFailuresOf(cell, "main"), ruleFailuresOf(cell, candidate));
  const metrics = METRICS.map((metric) => compareMetric(metric, metricValues(main, metric), metricValues(mine, metric)));
  const mainKeys = sideFindings(cell, "main", undefined);
  const candidateKeys = sideFindings(cell, candidate, rr);
  let findings: CellVerdict["findings"] = null;
  let agree: CellVerdict["agreement"] = null;
  if (mainKeys !== undefined && candidateKeys !== undefined) {
    if (corpus === "fixtures") {
      const ruleIds = new Set([...shippedRules(settings.baseline.dir, pack), ...(candidate === "ships" ? shippedRules(settings.candidate.dir, pack) : [])]);
      const { expected, restrict, divergences } = fixtureKeys(settings.baseline.dir, settings.candidate.dir, settings.corpora, pack, ruleIds);
      const m = scoreFindings(expected, restrict(mainKeys), divergences);
      const c = scoreFindings(expected, restrict(candidateKeys), divergences);
      const compared = compareFindings(m, c);
      const counts = (s: typeof m): Counts => ({ tp: s.tp.size, fp: s.fp.size, fn: s.fn.size, explainedFp: s.explainedFp, explainedFn: s.explainedFn });
      findings = { status: compared.status, reason: compared.reason, lostTp: compared.lostTp.length, newFp: compared.newFp.length, samples: [...compared.lostTp.slice(0, 5).map((k) => `lost TP ${k.replaceAll("\u0000", " ")}`), ...compared.newFp.slice(0, 5).map((k) => `new FP ${k.replaceAll("\u0000", " ")}`)], main: counts(m), candidate: counts(c) };
    } else {
      const a = agreement(mainKeys, candidateKeys);
      agree = { both: a.both, mainOnly: a.mainOnly.length, candidateOnly: a.candidateOnly.length, samples: [...a.mainOnly.slice(0, 3).map((k) => `main only ${k.replaceAll("\u0000", " ")}`), ...a.candidateOnly.slice(0, 3).map((k) => `candidate only ${k.replaceAll("\u0000", " ")}`)] };
    }
  }
  const statuses: Status[] = [completion.status, ...metrics.map((m) => m.status), ...(findings === null ? [] : [findings.status]), ...(ruleFailures.length > 0 ? (["fail"] as Status[]) : [])];
  return {
    corpus,
    status: overall(statuses),
    note: "",
    completion,
    ruleFailures,
    metrics,
    findings,
    agreement: agree,
    load: { main: main.map((row) => row.load), candidate: mine.map((row) => row.load) },
    siblingBusyMax: Math.max(0, ...all.map((row) => row.siblingBusy)),
    runs: { main: main.length, candidate: mine.length },
    rules: { main: shippedRules(settings.baseline.dir, pack).length, candidate: candidate === "rr" && rr !== undefined ? rrIds(rr, cell).length : shippedRules(settings.candidate.dir, pack).length },
  };
}

function computeVerdict(settings: { baseline: Checkout; candidate: Checkout; corpora: string; out: string; runs: number }, packs: readonly string[], candidateOnly: readonly string[], corpusKinds: readonly Corpus[], candidates: readonly Candidate[], gate: readonly Candidate[]): Verdict {
  const lock = readJson<CorporaLock>(join(settings.corpora, "corpora.lock.json"));
  const rows = packs.flatMap((pack) => corpusKinds.flatMap((corpus) => readRows(join(settings.out, "runs", pack, corpus, "runs.jsonl"))));
  const distinct = <T>(values: T[]): T[] => [...new Set(values)];
  const verdicts: CandidateVerdict[] = candidates.map((candidate) => {
    const failures: Failure[] = [];
    const packVerdicts: PackVerdict[] = packs.map((pack) => {
      const mainRules = shippedRules(settings.baseline.dir, pack);
      const rrPath = join(settings.out, "runs", pack, "rr-rules.json");
      const rr = candidate === "rr" && existsSync(rrPath) ? readJson<RrRules>(rrPath) : undefined;
      const available = new Map<string, string | null>(
        candidate === "ships" ? shippedRules(settings.candidate.dir, pack).map((rule) => [rule, null]) : Object.entries(rr?.rules ?? {}).map(([rule, r]) => [rule, r.unavailable]),
      );
      // A `.rr` rule that failed alone on a corpus is left out of that corpus's runs: it does not run there.
      for (const corpus of candidate === "rr" ? corpusKinds : []) {
        for (const [rule, reason] of Object.entries(readExcluded(join(settings.out, "runs", pack, corpus)))) {
          const before = available.get(rule);
          available.set(rule, `${before === null || before === undefined ? "" : `${before}; `}fails to run on ${corpus}: ${reason}`);
        }
      }
      const missing = candidate === "rr" && rr === undefined ? [{ rule: "*", reason: "the .rr side was not measured" }] : missingRules(mainRules, available);
      const extra = candidate === "ships" ? [...available.keys()].filter((rule) => !mainRules.includes(rule)) : [];
      for (const m of missing) failures.push({ pack, corpus: null, item: "coverage", reason: `${m.rule}: ${m.reason}` });
      const cells = corpusKinds.map((corpus) => cellVerdict(settings, lock, pack, corpus, candidate, rr));
      for (const cell of cells) {
        if (cell.status === "fail" && cell.note !== "") failures.push({ pack, corpus: cell.corpus, item: "completion", reason: cell.note });
        if (cell.completion?.status === "fail") failures.push({ pack, corpus: cell.corpus, item: "completion", reason: cell.completion.reason });
        for (const f of cell.ruleFailures) failures.push({ pack, corpus: cell.corpus, item: "rule failure", reason: `${f.rule}: ${f.reason}` });
        for (const m of cell.metrics) if (m.status === "fail") failures.push({ pack, corpus: cell.corpus, item: m.metric, reason: m.reason + (m.withinSpread ? " (within the run-to-run spread)" : "") });
        if (cell.findings?.status === "fail") failures.push({ pack, corpus: cell.corpus, item: "findings", reason: `${cell.findings.reason}; e.g. ${cell.findings.samples.slice(0, 2).join(", ")}` });
      }
      const status = overall([...cells.map((cell) => cell.status), ...(missing.length > 0 ? (["fail"] as Status[]) : [])]);
      return { pack, status, missing, extra, cells };
    });
    for (const pack of candidateOnly) packVerdicts.push({ pack, status: "no-baseline", missing: [], extra: [], cells: [] });
    const incomplete = packVerdicts.some((p) => p.cells.some((cell) => cell.status === "not-compared" && !cell.note.startsWith("no fixtures corpus")));
    const status = failures.length > 0 ? "FAIL" : incomplete ? "INCOMPLETE" : "PASS";
    return { candidate, description: CANDIDATES[candidate], status, failures, packs: packVerdicts };
  });
  return {
    generated: new Date().toISOString(),
    tolerancePercent: TOLERANCE_PERCENT,
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
    },
    candidates: verdicts,
    gate: [...gate],
  };
}

function writeVerdict(verdict: Verdict, out: string): number {
  writeFileSync(join(out, "verdict.json"), `${JSON.stringify(verdict, null, 1)}\n`);
  writeFileSync(join(out, "report.md"), renderReport(verdict));
  for (const c of verdict.candidates) console.log(`shipbar: ${c.candidate}: ${c.status} (${c.failures.length} failing items)`);
  console.log(`shipbar: ${join(out, "verdict.json")}, ${join(out, "report.md")}`);
  return verdict.candidates.filter((c) => verdict.gate.includes(c.candidate)).every((c) => c.status === "PASS") ? 0 : 1;
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
      runs: { type: "string", default: "3" },
      cpus: { type: "string", default: "8-11" },
      "mem-cap": { type: "string", default: "24G" },
      "time-cap": { type: "string", default: "2700" },
      candidates: { type: "string", default: "ships,rr" },
      gate: { type: "string" },
      build: { type: "boolean", default: false },
      record: { type: "boolean", default: false },
      "pack-data-cache": { type: "string" },
    },
  });
  const command = positionals[0];
  if (command !== "prepare" && command !== "run" && command !== "verdict") fail("usage: shipbar.ts prepare|run|verdict --baseline <ref|dir> --out <dir> [options] (see the header of scripts/ship/shipbar.ts)");
  if (values.baseline === undefined || values.out === undefined) fail("--baseline and --out are required");
  const out = resolve(values.out);
  mkdirSync(out, { recursive: true });
  const candidates = values.candidates.split(",").map((c) => c.trim()) as Candidate[];
  for (const c of candidates) if (!(c in CANDIDATES)) fail(`--candidates: no candidate ${c} (ships, rr)`);
  const gate = (values.gate?.split(",").map((c) => c.trim()) ?? candidates) as Candidate[];
  for (const c of gate) if (!candidates.includes(c)) fail(`--gate ${c} is not among --candidates`);
  const corpusKinds: Corpus[] = values.corpus === "all" ? [...CORPORA] : values.corpus === "fixtures" || values.corpus === "real" ? [values.corpus] : fail(`--corpus ${values.corpus}: fixtures, real or all`);
  const runs = Number(values.runs);
  if (!Number.isInteger(runs) || runs < 1) fail(`--runs ${values.runs} is not a positive integer`);
  if (command === "run") requirePlatform();

  const baseline = resolveBaseline(values.baseline, out, values["pack-data-cache"]);
  const candidate: Checkout = { dir: resolve(values.candidate), sha: checkoutSha(resolve(values.candidate)) };
  const discovered = discoverPacks(baseline.dir, candidate.dir);
  const requested = values.packs?.split(",").map((p) => p.trim());
  for (const pack of requested ?? []) if (!discovered.packs.includes(pack) && !discovered.candidateOnly.includes(pack)) fail(`--packs: main ships no pack ${pack}`);
  const packs = discovered.packs.filter((pack) => requested === undefined || requested.includes(pack));
  const candidateOnly = discovered.candidateOnly.filter((pack) => requested === undefined || requested.includes(pack));
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
  if (command === "run") {
    if (!existsSync(join(corpora, "corpora.lock.json"))) prepareCorpora(baseline, packs, corpora, values.record);
    const lock = verifyCorpora(corpora, packs, corpusKinds);
    requireBuilt(baseline.dir, { product: true, engine: false }, values.build);
    const { engine } = requireBuilt(candidate.dir, { product: candidates.includes("ships"), engine: candidates.includes("rr") }, values.build);
    const cpus = cpuList(values.cpus);
    const settings: Settings = { ...settingsBase, engine, cpus, siblings: idleSiblings(cpus), memCap: values["mem-cap"], timeCapS: Number(values["time-cap"]) };
    writeFileSync(join(out, "settings.json"), JSON.stringify({ ...settings, candidates, corpusKinds, packs, productBuild: PRODUCT_BUILD, engineBuild: ENGINE_BUILD }, null, 1));
    const sides: Side[] = ["main", ...candidates];
    for (const pack of packs) {
      let rr: RrRules | undefined;
      if (candidates.includes("rr")) {
        const path = join(out, "runs", pack, "rr-rules.json");
        mkdirSync(dirname(path), { recursive: true });
        if (!existsSync(path)) writeFileSync(path, JSON.stringify(rrRules(candidate.dir, engine!, shippedRules(baseline.dir, pack), pack), null, 1));
        rr = readJson<RrRules>(path);
      }
      for (const corpus of corpusKinds) {
        const fixtures = lock.fixtures[pack];
        if (corpus === "fixtures" && fixtures !== undefined && "absent" in fixtures) continue;
        measureCell(settings, pack, corpus, sides, rr);
      }
    }
  }
  process.exitCode = writeVerdict(computeVerdict(settingsBase, packs, candidateOnly, corpusKinds, candidates, gate), out);
}

main();
