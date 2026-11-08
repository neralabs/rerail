/**
 * Offline test of Merge Assist's conflict resolver on OSS conflict pairs: can the policy pick the design the project
 * adopted, and escalate when it can't? No agent runs; the resolver reads each pair's sides and the repository.
 *
 *   node --import tsx benchmark/merge-assist-resolve.ts build --pairs pairs.json --ids ids.txt|a,b [--repos dir]
 *     [--replay dir] [--bodies dir] --out records.json
 *   node --import tsx benchmark/merge-assist-resolve.ts checks --records records.json --setups dir [--repos dir]
 *     [--ids a,b] [--concurrency N] [--timeout-min N] [--work dir] --out checks.json
 *   node --import tsx benchmark/merge-assist-resolve.ts resolve --records records.json [--checks checks.json]
 *     [--policy file] [--seeds 0,1,2] [--swap] [--ids a,b] [--repos dir] [--cache dir] [--concurrency N]
 *     [--provider amazon-bedrock] [--model m] [--bedrock-region r] [--bedrock-token-cmd cmd] [--no-judge] --out results.json
 *   node --import tsx benchmark/merge-assist-resolve.ts score --results results.json[,more.json] --labels dir
 *   node --import tsx benchmark/merge-assist-resolve.ts log-smoke --results results.json [--repos dir] --scratch dir
 *
 * `build` makes one conflict record per pair from `pairs.json` (the shared pair format, /tmp/ma-ctx/align/oss-spec.md),
 * reading each pair's sides only: the base is the second pull request's `base_sha`; side `a` is the first pull
 * request's head as of the second's start (`first_head_at_second_start`, else the replay spec's `first_visible`, else
 * its last commit visible by then, else its head at that time from GitHub's force-push events, else its head); side
 * `b` is the second's head. Each side's intent is its title and body fetched with `gh` (cached under `--bodies`), with
 * cross-references and supersede wording stripped (`sanitizeIntent`); its anchors are the files it changes. Which
 * record side holds the first pull request is kept beside the record for the scorer; the resolver never reads it.
 *
 * `checks` runs each side's tests and type check, meant for the build boxes: the pair's `setup` (only the `setup` field
 * of `<--setups>/<id>.json` is read) is split at `&&` into install steps (the policy's setup) and checks (type checks:
 * `cargo check`, `tsc`, `mypy`, `go build`/`go vet`, `run check`; the rest tests), adapted per side: a test target or
 * path the side's tree does not have is left out, and a check left with none is dropped for that side. The side's tree
 * is exported with `git archive`, the runner is the product's (`createLocalCheckRunner`); each check is pass, fail or
 * unknown, with the end of its log.
 *
 * `resolve` runs `resolveConflict` on every record under `--policy` (fixtures/host/benchmark/merge-assist/resolve-policy.yaml by
 * default; the checks come from `checks.json`, keyed by commit), once per seed, and with `--swap` again with sides a
 * and b exchanged, which shows each judge the other order: a verdict must follow the content, not the slot. The panel's
 * judges are `codex exec` calls with no tools in an empty folder with an isolated CODEX_HOME, on `--provider`'s model
 * (Bedrock's token from `--bedrock-token-cmd`, handed to Codex only), cached by task key under `--cache`.
 *
 * `score` is the only step that reads labels: `adopted_side` ("first" | "second" | "neither") from `<--labels>/<id>.json`.
 * A verdict for the adopted side is correct, for the other side wrong; an escalation is escalated, and correct when the
 * project adopted neither (where any verdict is wrong). Per seed and orientation it prints the wrong rate over all pairs,
 * accuracy over decided pairs and the escalation rate with Wilson 95% intervals, the step that decided each pair, the
 * split by which side was adopted, and, given swapped runs, whether each verdict picked the same design both ways.
 *
 * `log-smoke` writes a decision-log entry for every resolution of seed 0 (unswapped) into a scratch clone of each
 * pair's repository and reads the log back.
 */
import { execFileSync, spawn, spawnSync } from "node:child_process";
import { existsSync, mkdirSync, readdirSync, readFileSync, renameSync, writeFileSync } from "node:fs";
import { mkdtemp, rm } from "node:fs/promises";
import { homedir, tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { fixturesPath } from "../test/support/oracles.js";
import {
  appendDecision,
  type CheckResult,
  type ConflictRecord,
  conflictRecordSchema,
  type Policy,
  type PolicyCheck,
  parsePolicy,
  questionOf,
  readDecisions,
  sanitizeIntent,
  sideChanges,
  type SideId,
  type Verdict,
} from "../src/engine/index.js";
import {
  type AgentJudge,
  type CheckRunner,
  createLocalCheckRunner,
  decisionEntryFor,
  type JudgeOutcome,
  type JudgeTask,
  type Resolution,
  resolveConflict,
  type SideCheckOutcome,
} from "../src/runtime/index.js";
import { clip, fixed, ratio, type Ratio, table, wilson } from "./merge-assist-context.js";

const DEFAULT_REPOS = "/tmp/ma-ctx/oss/repos";
const DEFAULT_POLICY = fixturesPath("host", "benchmark", "merge-assist", "resolve-policy.yaml");
const DEFAULT_CACHE = join(tmpdir(), "rerail-resolve-cache");
const DEFAULT_BODIES = join(DEFAULT_CACHE, "pr-bodies");
const DEFAULT_MODEL = "openai.gpt-6-luna";
const DEFAULT_REGION = "us-east-1";
const BEDROCK_TOKEN_ENV = "AWS_BEARER_TOKEN_BEDROCK";
const JUDGE_TIMEOUT_MS = 10 * 60_000;
const REPOSITORY_KEY = "eval:merge-assist-resolve";

// ------------------------------------------------------------------------------------------------------- inputs

interface SideCommit {
  sha: string;
  visible_at: string;
}

interface PairSide {
  pr: number;
  title: string;
  created_at: string;
  base_sha: string;
  head_sha: string;
  commits: SideCommit[];
}

/** The fields of a pair `build` reads: its sides, never its conflict description or resolution. */
interface Pair {
  id: string;
  repo: string;
  language: string;
  first: PairSide;
  second: PairSide;
  first_head_at_second_start?: string;
}

/** One pair's conflict record, with what the scorer needs beside it. */
export interface PairRecord {
  id: string;
  repo: string;
  language: string;
  /** Which record side holds the first pull request. Read by `score` only (and by `--swap`, which exchanges it). */
  first: SideId;
  /** Where side a's commit came from. */
  firstHeadSource: string;
  record: ConflictRecord;
}

function readJson<T>(path: string): T {
  return JSON.parse(readFileSync(path, "utf8")) as T;
}

function writeJson(path: string, value: unknown): void {
  writeFileSync(`${path}.tmp`, `${JSON.stringify(value, null, 2)}\n`);
  renameSync(`${path}.tmp`, path);
}

function git(root: string, args: readonly string[]): string | undefined {
  try {
    return execFileSync("git", ["-C", root, ...args], { encoding: "utf8", stdio: ["ignore", "pipe", "ignore"], maxBuffer: 256 * 1024 * 1024 });
  } catch {
    return undefined;
  }
}

function hasCommit(root: string, sha: string): boolean {
  return git(root, ["cat-file", "-e", `${sha}^{commit}`]) !== undefined;
}

/** `<repos>/<owner>__<name>`, matched without case (`Kludex__starlette`). */
export function repoPath(repos: string, repo: string): string {
  const wanted = repo.replace("/", "__").toLowerCase();
  const found = readdirSync(repos).find(entry => entry.toLowerCase() === wanted);
  if (found === undefined) throw new Error(`no clone of ${repo} in ${repos}`);
  return join(repos, found);
}

function idsFrom(text: string | undefined): string[] | null {
  if (text === undefined) return null;
  if (existsSync(text)) return readFileSync(text, "utf8").split(/\s+/u).filter(Boolean);
  return text.split(",").map(id => id.trim()).filter(Boolean);
}

/** `gh api`'s JSON answer, cached in `file`. */
function ghCached<T>(file: string, path: string, paginate = false): T {
  if (existsSync(file)) return readJson<T>(file);
  const output = execFileSync("gh", ["api", ...(paginate ? ["--paginate", "--slurp"] : []), path], { encoding: "utf8", maxBuffer: 64 * 1024 * 1024 });
  const value = JSON.parse(output) as T;
  writeJson(file, value);
  return value;
}

/**
 * The first pull request's head when the second started, and where it came from. GitHub's force-push events name the
 * commit each push left, so the last one by then is the head then.
 */
function firstHeadAt(pair: Pair, root: string, replayDir: string | undefined, bodies: string): { sha: string; source: string } {
  if (pair.first_head_at_second_start !== undefined) return { sha: pair.first_head_at_second_start, source: "first_head_at_second_start" };
  const replay = replayDir === undefined ? undefined : join(replayDir, `${pair.id}.json`);
  if (replay !== undefined && existsSync(replay)) {
    const visible = readJson<{ first_visible?: string }>(replay).first_visible;
    if (visible !== undefined) return { sha: visible, source: "first_visible" };
  }
  const start = pair.second.created_at;
  const visible = pair.first.commits.filter(commit => commit.visible_at <= start).at(-1);
  if (visible !== undefined) return { sha: visible.sha, source: "last commit visible at the second's start" };
  const events = ghCached<Array<Array<{ event?: string; created_at?: string; commit_id?: string }>>>(
    join(bodies, `${pair.repo.replace("/", "__")}-${pair.first.pr}-timeline.json`),
    `repos/${pair.repo}/issues/${pair.first.pr}/timeline?per_page=100`,
    true,
  ).flat();
  const pushed = events.filter(event => event.event === "head_ref_force_pushed" && event.created_at !== undefined && event.created_at <= start && event.commit_id).at(-1);
  if (pushed?.commit_id !== undefined && hasCommit(root, pushed.commit_id)) return { sha: pushed.commit_id, source: "last force push before the second's start" };
  return { sha: pair.first.head_sha, source: "head (no commit known visible at the second's start)" };
}

/** A pull request's title and body as the resolver may read them: fetched once, cross-references and outcomes stripped. */
function intentOf(repo: string, pr: number, bodies: string): string {
  const raw = ghCached<{ title?: string; body?: string | null }>(join(bodies, `${repo.replace("/", "__")}-${pr}.json`), `repos/${repo}/pulls/${pr}`);
  return sanitizeIntent(`${raw.title ?? ""}\n\n${raw.body ?? ""}`);
}

async function build(flags: Flags): Promise<void> {
  const pairsPath = required(flags, "pairs");
  const ids = idsFrom(flags.get("ids"));
  if (ids === null) throw new Error("build needs --ids");
  const repos = flags.get("repos") ?? DEFAULT_REPOS;
  const bodies = flags.get("bodies") ?? DEFAULT_BODIES;
  mkdirSync(bodies, { recursive: true });
  const pairs = readJson<{ pairs: Pair[] }>(pairsPath).pairs;
  const records: PairRecord[] = [];
  for (const id of ids) {
    const pair = pairs.find(candidate => candidate.id === id);
    if (pair === undefined) throw new Error(`no pair ${id} in ${pairsPath}`);
    const root = repoPath(repos, pair.repo);
    const head = firstHeadAt(pair, root, flags.get("replay"), bodies);
    const base = pair.second.base_sha;
    const sides = { a: { pr: pair.first.pr, sha: head.sha }, b: { pr: pair.second.pr, sha: pair.second.head_sha } };
    const anchorsOf = (sha: string) => {
      const changes = sideChanges(root, base, sha);
      if (changes === null) throw new Error(`${id}: ${root} lacks ${sha} or ${base}`);
      return { files: changes.files.map(file => file.path).slice(0, 500), symbols: [] as string[] };
    };
    const anchors: Record<SideId, { files: string[]; symbols: string[] }> = { a: anchorsOf(sides.a.sha), b: anchorsOf(sides.b.sha) };
    const record: ConflictRecord = conflictRecordSchema.parse({
      repository: pair.repo,
      base,
      question: questionOf(anchors.a, anchors.b),
      sides: Object.fromEntries((["a", "b"] as const).map(side => [side, {
        ref: `refs/pull/${sides[side].pr}/head`,
        sha: sides[side].sha,
        intent: intentOf(pair.repo, sides[side].pr, bodies),
        anchors: anchors[side],
      }])),
    });
    records.push({ id, repo: pair.repo, language: pair.language, first: "a", firstHeadSource: head.source, record });
    console.log(`${id}: a ${head.sha.slice(0, 12)} (${head.source}), b ${pair.second.head_sha.slice(0, 12)}, base ${base.slice(0, 12)}, ${anchors.a.files.length}/${anchors.b.files.length} files`);
  }
  writeJson(required(flags, "out"), { records });
}

// ------------------------------------------------------------------------------------------------------- checks

const TYPECHECK_STEP = /\bcargo\s+(?:\+\S+\s+)?check\b|\btsc\b|\bmypy\b|\bpyright\b|\bgo\s+(?:build|vet)\b|\brun\s+(?:check|typecheck|type-check|lint:types)\b/u;
/** An install step: `install`, `ci`, `sync` or `venv` as a word of the command (not inside a path such as `.venv/bin`). */
const INSTALL_STEP = /(?:^|\s)(?:install|ci|sync|venv)(?=\s|$)/u;

export interface SetupPlan {
  /** Steps before the first check: installs, joined with `&&`. */
  setup: string | null;
  /** Checks named by kind and their place in the setup (`typecheck`, `tests`, `tests-2`), so both sides' line up. */
  checks: PolicyCheck[];
}

/** A pair's setup split at `&&` into install steps and checks; a trailing note such as `(Linux)` is dropped. */
export function planSetup(setup: string): SetupPlan {
  const steps = setup.replace(/\s*\([A-Za-z ,/]{1,30}\)\s*$/u, "").split("&&").map(step => step.trim()).filter(Boolean);
  const installs: string[] = [];
  const checks: PolicyCheck[] = [];
  const counts = { tests: 0, typecheck: 0 };
  for (const step of steps) {
    if (checks.length === 0 && INSTALL_STEP.test(step)) {
      installs.push(step);
      continue;
    }
    const kind = TYPECHECK_STEP.test(step) ? "typecheck" : "tests";
    counts[kind] += 1;
    checks.push({ name: counts[kind] === 1 ? kind : `${kind}-${counts[kind]}`, kind, run: step });
  }
  return { setup: installs.length === 0 ? null : installs.join(" && "), checks };
}

/**
 * A check adapted to one side's tree: `cargo --test <name>` targets and path arguments (a test file, a package folder)
 * the tree does not have are left out; null when the check named targets and none is left.
 */
export function adaptCheck(check: PolicyCheck, paths: ReadonlySet<string>): { check: PolicyCheck | null; dropped: string[] } {
  const folders = new Set<string>();
  for (const path of paths) {
    const parts = path.split("/");
    for (let end = 1; end < parts.length; end += 1) folders.add(parts.slice(0, end).join("/"));
  }
  const tokens = check.run.split(/\s+/u);
  // The command itself (`.venv/bin/python`), after any `NAME=value` assignments, is never a target.
  const command = tokens.findIndex(token => !/^[A-Za-z_][A-Za-z0-9_]*=/u.test(token));
  const kept: string[] = [];
  const dropped: string[] = [];
  let targets = 0;
  for (let index = 0; index < tokens.length; index += 1) {
    const token = tokens[index]!;
    if (index <= command) {
      kept.push(token);
      continue;
    }
    if (token === "--test" && index + 1 < tokens.length && check.kind === "tests") {
      const name = tokens[index + 1]!;
      targets += 1;
      index += 1;
      const present = [...paths].some(path => path.endsWith(`tests/${name}.rs`) || path.endsWith(`tests/${name}/main.rs`));
      if (present) kept.push(token, name);
      else dropped.push(`--test ${name}`);
      continue;
    }
    const path = token.replace(/^\.\//u, "").replace(/\/(?:\.\.\.)?$/u, "");
    const pathLike = check.kind === "tests" && /\//u.test(token) && !token.startsWith("-") && !token.includes("=") && /^[\w./-]+$/u.test(token);
    if (pathLike) {
      targets += 1;
      if (paths.has(path) || folders.has(path)) kept.push(token);
      else dropped.push(token);
      continue;
    }
    kept.push(token);
  }
  if (targets > 0 && dropped.length === targets) return { check: null, dropped };
  return { check: { ...check, run: kept.join(" ") }, dropped };
}

/** Where the boxes keep toolchains, ahead of the harness's PATH. */
function checkPath(): string {
  const home = homedir();
  const extra = ["/usr/local/go/bin", join(home, "go", "bin"), join(home, ".cargo", "bin"), join(home, ".local", "bin"), join(home, ".npm-global", "bin")].filter(existsSync);
  return [...extra, ...(process.env.PATH ?? "").split(":")].join(":");
}

export interface SideChecks {
  sha: string;
  setup: string | null;
  dropped: string[];
  outcome: SideCheckOutcome;
  wallMs: number;
}

/** Runs `work` over `items`, at most `concurrency` at once, in order of the items. */
async function pool<T, R>(items: readonly T[], concurrency: number, work: (item: T) => Promise<R>): Promise<R[]> {
  const results: R[] = new Array(items.length);
  let next = 0;
  await Promise.all(Array.from({ length: Math.min(concurrency, items.length) }, async () => {
    while (next < items.length) {
      const index = next++;
      results[index] = await work(items[index]!);
    }
  }));
  return results;
}

async function checks(flags: Flags): Promise<void> {
  const { records } = readJson<{ records: PairRecord[] }>(required(flags, "records"));
  const setups = required(flags, "setups");
  const repos = flags.get("repos") ?? DEFAULT_REPOS;
  const ids = idsFrom(flags.get("ids"));
  const timeoutMinutes = Number(flags.get("timeout-min") ?? 45);
  const work = flags.get("work") ?? join(DEFAULT_CACHE, "targets");
  const out = required(flags, "out");
  const done: Record<string, SideChecks> = existsSync(out) ? readJson<{ sides: Record<string, SideChecks> }>(out).sides : {};
  const jobs = records.filter(entry => ids === null || ids.includes(entry.id)).flatMap(entry => (["a", "b"] as const).map(side => ({ entry, side, sha: entry.record.sides[side].sha })))
    .filter(job => done[job.sha] === undefined);
  // One repository's sides run one after another: they share its build folder and the toolchain it pins, which
  // concurrent first installs corrupt.
  const byRepo = [...Map.groupBy(jobs, job => job.entry.repo).values()];
  const runSide = async ({ entry, side, sha }: (typeof jobs)[number]): Promise<void> => {
    const setupFile = join(setups, `${entry.id}.json`);
    const setup = existsSync(setupFile) ? readJson<{ setup?: string | null }>(setupFile).setup ?? null : null;
    const root = repoPath(repos, entry.repo);
    const started = performance.now();
    let result: SideChecks;
    if (setup === null) {
      result = { sha, setup: null, dropped: [], outcome: { checks: [], benchmarks: {} }, wallMs: 0 };
    } else {
      const plan = planSetup(setup);
      const paths = new Set((git(root, ["ls-tree", "-r", "--name-only", sha]) ?? "").split("\n").filter(Boolean));
      const adapted = plan.checks.map(check => adaptCheck(check, paths));
      const policy = parsePolicyText(`version: 1\nchecks:\n  timeoutMinutes: ${timeoutMinutes}\n`);
      const sidePolicy: Policy = { ...policy, checks: { ...policy.checks, setup: plan.setup, run: adapted.flatMap(item => (item.check === null ? [] : [item.check])) } };
      const env = { ...process.env, PATH: checkPath(), CI: "1", SETUPTOOLS_SCM_PRETEND_VERSION: "99.0.0", CARGO_TARGET_DIR: join(work, entry.repo.replace("/", "__")), CARGO_TERM_COLOR: "never", NO_COLOR: "1", HELIX_DISABLE_AUTO_GRAMMAR_BUILD: "1" };
      const runner = createLocalCheckRunner({ root, env });
      const outcome = await runner.run(side, sha, sidePolicy);
      result = { sha, setup: plan.setup, dropped: adapted.flatMap(item => item.dropped), outcome, wallMs: Math.round(performance.now() - started) };
    }
    done[sha] = result;
    writeJson(out, { sides: done });
    console.log(`${entry.id} ${side} ${sha.slice(0, 12)}: ${result.outcome.checks.map(check => `${check.name} ${check.status}`).join(", ") || "no checks"}${result.dropped.length > 0 ? ` (left out: ${result.dropped.join(", ")})` : ""} in ${Math.round(result.wallMs / 1000)} s`);
  };
  await pool(byRepo, Number(flags.get("concurrency") ?? 4), async group => {
    for (const job of group) await runSide(job);
  });
  writeJson(out, { sides: done });
}

function parsePolicyText(text: string): Policy {
  const parsed = parsePolicy(text);
  if (!parsed.ok) throw new Error(`invalid policy: ${parsed.problems.join("; ")}`);
  return parsed.policy;
}

// ------------------------------------------------------------------------------------------------------- judge

interface JudgeSettings {
  readonly cacheDir: string;
  readonly model: string;
  readonly provider: string | undefined;
  readonly region: string;
  readonly env: Readonly<Record<string, string>>;
  readonly concurrency: number;
}

/** One Codex call's answer, or why there is none. */
type CallOutcome = { ok: true; value: unknown } | { ok: false; reason: "timeout" | "failed" | "invalid"; detail: string };

/**
 * The panel's judge for this benchmark: one `codex exec` per task in an empty private folder, with an isolated
 * CODEX_HOME (the provider, no web, no shell environment credentials), read-only, no shell or other tools, its answer
 * held to the task's schema. Answers are cached by task key; at most `concurrency` calls run at once.
 */
function codexJudge(settings: JudgeSettings): AgentJudge {
  mkdirSync(settings.cacheDir, { recursive: true });
  let running = 0;
  const waiting: Array<() => void> = [];
  const call = async (task: JudgeTask<unknown>): Promise<CallOutcome> => {
    const folder = await mkdtemp(join(tmpdir(), "rerail-resolve-judge-"));
    try {
      const home = join(folder, "codex-home");
      const work = join(folder, "work");
      mkdirSync(home, { mode: 0o700 });
      mkdirSync(work, { mode: 0o700 });
      writeFileSync(join(home, "config.toml"), [
        ...(settings.provider === undefined ? [] : [`model_provider = ${JSON.stringify(settings.provider)}`]),
        `web_search = "disabled"`,
        "",
        ...(settings.provider === undefined ? [] : [`[model_providers.${settings.provider}.aws]`, `region = ${JSON.stringify(settings.region)}`, ""]),
        "[shell_environment_policy]",
        `exclude = ["AWS_*", "*TOKEN*", "*SECRET*", "*KEY*", "*CREDENTIAL*", "*PASSWORD*"]`,
        "",
      ].join("\n"));
      writeFileSync(join(folder, "schema.json"), JSON.stringify(task.jsonSchema));
      const answerPath = join(folder, "answer.json");
      const args = [
        "exec", "--skip-git-repo-check", "--ephemeral", "--sandbox", "read-only",
        "--disable", "shell_tool", "--disable", "unified_exec", "--disable", "code_mode_host", "--disable", "multi_agent", "--disable", "apps",
        "-m", settings.model, "--output-schema", join(folder, "schema.json"), "-o", answerPath, "-",
      ];
      const { promise, resolve: settle } = Promise.withResolvers<{ code: number | null; timedOut: boolean; stderr: string }>();
      const child = spawn("codex", args, { cwd: work, detached: true, env: { ...process.env, ...settings.env, CODEX_HOME: home }, stdio: ["pipe", "ignore", "pipe"] });
      let stderr = "";
      let timedOut = false;
      const timer = setTimeout(() => {
        timedOut = true;
        try {
          process.kill(-child.pid!, "SIGKILL");
        } catch {
          // gone
        }
      }, JUDGE_TIMEOUT_MS);
      child.stderr.on("data", (chunk: Buffer) => {
        stderr = (stderr + chunk.toString("utf8")).slice(-4_000);
      });
      child.on("error", error => settle({ code: null, timedOut: false, stderr: error.message }));
      child.on("close", code => settle({ code, timedOut, stderr }));
      child.stdin.end(task.prompt);
      const result = await promise;
      clearTimeout(timer);
      if (result.timedOut) return { ok: false, reason: "timeout", detail: `no answer in ${JUDGE_TIMEOUT_MS / 60_000} minutes` };
      if (!existsSync(answerPath)) return { ok: false, reason: "failed", detail: `codex exited ${result.code} with no answer: ${clip(result.stderr, 300)}` };
      try {
        return { ok: true, value: JSON.parse(readFileSync(answerPath, "utf8")) as unknown };
      } catch (error) {
        return { ok: false, reason: "invalid", detail: `answer is not JSON: ${error instanceof Error ? error.message : String(error)}` };
      }
    } finally {
      // Codex's background plugin sync may still be writing into CODEX_HOME; a folder left behind fails nothing.
      await rm(folder, { recursive: true, force: true, maxRetries: 5, retryDelay: 500 }).catch(() => undefined);
    }
  };
  return {
    readiness: () => ({ available: true, runner: "codex", problem: null }),
    budget: () => ({ used: 0, limit: Number.POSITIVE_INFINITY, windowMinutes: 60 }),
    async judge<T>(task: JudgeTask<T>): Promise<JudgeOutcome<T>> {
      const cached = join(settings.cacheDir, `${task.cacheKey}.json`);
      if (existsSync(cached)) {
        try {
          return { ok: true, value: task.parse(readJson(cached)), runner: "codex", cached: true };
        } catch {
          // A cached answer the parser now refuses is asked again.
        }
      }
      if (running >= settings.concurrency) {
        const { promise, resolve: release } = Promise.withResolvers<void>();
        waiting.push(release);
        await promise;
      } else running += 1;
      let outcome: CallOutcome;
      try {
        outcome = await call(task as JudgeTask<unknown>);
      } finally {
        const next = waiting.shift();
        if (next === undefined) running -= 1;
        else next();
      }
      if (!outcome.ok) return outcome;
      try {
        const value = task.parse(outcome.value);
        writeJson(cached, outcome.value);
        return { ok: true, value, runner: "codex", cached: false };
      } catch (error) {
        return { ok: false, reason: "invalid", detail: `the answer did not match the schema: ${error instanceof Error ? error.message : String(error)}` };
      }
    },
  };
}

// ------------------------------------------------------------------------------------------------------- resolve

export interface ResolveRun {
  id: string;
  seed: number;
  swapped: boolean;
  /** The record side holding the first pull request in this run. */
  first: SideId;
  verdict: Verdict;
  /** The pull request the verdict picked, by order: what `score` compares with the label. */
  chosen: "first" | "second" | null;
  step: Resolution["step"];
  reason: string;
  resolution: Resolution;
  wallMs: number;
}

/** The record with sides a and b exchanged. */
export function swapped(entry: PairRecord): PairRecord {
  const { a, b } = entry.record.sides;
  return { ...entry, first: entry.first === "a" ? "b" : "a", record: { ...entry.record, sides: { a: b, b: a } } };
}

/** Check outcomes run elsewhere (`checks.json`), looked up by the side's commit; a commit with none ran no checks. */
function recordedChecks(sides: Record<string, SideChecks>): CheckRunner {
  return {
    run: async (_side, sha) => sides[sha]?.outcome ?? { checks: [], benchmarks: {} },
  };
}

/** The policy's checks are whatever each side's recorded run had: the resolver compares them by name. */
function withRecordedChecks(policy: Policy, sides: Record<string, SideChecks>): Policy {
  const names = new Map<string, PolicyCheck>();
  for (const side of Object.values(sides)) for (const check of side.outcome.checks) names.set(check.name, { name: check.name, kind: check.kind, run: "(recorded)" });
  return { ...policy, checks: { ...policy.checks, run: [...names.values()] } };
}

async function resolveAll(flags: Flags): Promise<void> {
  const { records } = readJson<{ records: PairRecord[] }>(required(flags, "records"));
  const ids = idsFrom(flags.get("ids"));
  const repos = flags.get("repos") ?? DEFAULT_REPOS;
  const policyPath = flags.get("policy") ?? DEFAULT_POLICY;
  const basePolicy = parsePolicyText(readFileSync(policyPath, "utf8"));
  const recorded = flags.has("checks") ? readJson<{ sides: Record<string, SideChecks> }>(flags.get("checks")!).sides : null;
  const policy = recorded === null ? basePolicy : withRecordedChecks(basePolicy, recorded);
  const seeds = (flags.get("seeds") ?? "0").split(",").map(Number);
  const tokenCmd = flags.get("bedrock-token-cmd");
  const env: Record<string, string> = {};
  if (tokenCmd !== undefined) {
    const token = spawnSync("sh", ["-c", tokenCmd], { encoding: "utf8", timeout: 120_000 }).stdout?.trim() ?? "";
    if (token === "" || /\s/u.test(token)) throw new Error("--bedrock-token-cmd gave no token");
    env[BEDROCK_TOKEN_ENV] = token;
  }
  const judge = flags.has("no-judge") ? null : codexJudge({
    cacheDir: join(flags.get("cache") ?? DEFAULT_CACHE, "judge"),
    model: flags.get("model") ?? DEFAULT_MODEL,
    provider: flags.get("provider"),
    region: flags.get("bedrock-region") ?? DEFAULT_REGION,
    env,
    concurrency: Number(flags.get("concurrency") ?? 9),
  });
  const orientations = flags.has("swap") ? [false, true] : [false];
  const selected = records.filter(entry => ids === null || ids.includes(entry.id));
  const jobs = selected.flatMap(entry => seeds.flatMap(seed => orientations.map(swap => ({ entry: swap ? swapped(entry) : entry, seed, swap }))));
  const runs = await pool(jobs, 12, async ({ entry, seed, swap }): Promise<ResolveRun> => {
    const started = performance.now();
    const resolution = await resolveConflict({
      root: repoPath(repos, entry.repo),
      repositoryKey: REPOSITORY_KEY,
      record: entry.record,
      policy,
      judge,
      checks: recorded === null ? null : recordedChecks(recorded),
      log: [],
      seed,
    });
    const chosen = resolution.verdict === "escalated" ? null : resolution.verdict === entry.first ? "first" : "second";
    const run: ResolveRun = { id: entry.id, seed, swapped: swap, first: entry.first, verdict: resolution.verdict, chosen, step: resolution.step, reason: resolution.reason, resolution, wallMs: Math.round(performance.now() - started) };
    const votes = resolution.evidence.panel?.votes.map(vote => (vote.readings.some(reading => reading.error !== null) ? "!" : vote.vote === null ? "-" : vote.vote === entry.first ? "F" : "S")).join("") ?? "";
    console.log(`${entry.id} seed ${seed}${swap ? " swapped" : ""}: ${chosen ?? "escalated"} by ${resolution.step}${votes === "" ? "" : ` [${votes}]`}: ${clip(resolution.reason, 160)}`);
    return run;
  });
  writeJson(required(flags, "out"), { policy: policyPath, policyText: readFileSync(policyPath, "utf8"), runs });
}

// ------------------------------------------------------------------------------------------------------- score

type Outcome = "correct" | "wrong" | "escalated";
type Adopted = "first" | "second" | "neither";

/** A run's outcome against the adopted side: escalation is correct when neither design was adopted. */
export function outcomeOf(chosen: "first" | "second" | null, adopted: Adopted): Outcome {
  if (chosen === null) return adopted === "neither" ? "correct" : "escalated";
  return chosen === adopted ? "correct" : "wrong";
}

function interval(value: Ratio): string {
  const [low, high] = wilson(value);
  return `${value.hits}/${value.total} = ${fixed(value.total === 0 ? Number.NaN : value.hits / value.total)} [${fixed(low)}, ${fixed(high)}]`;
}

function score(flags: Flags): void {
  const runs = required(flags, "results").split(",").flatMap(path => readJson<{ runs: ResolveRun[] }>(path).runs);
  const labels = required(flags, "labels");
  const adopted = new Map<string, Adopted>();
  for (const id of new Set(runs.map(run => run.id))) {
    const label = readJson<{ adopted_side?: Adopted }>(join(labels, `${id}.json`));
    if (label.adopted_side !== "first" && label.adopted_side !== "second" && label.adopted_side !== "neither") throw new Error(`${id}: no adopted_side in its label`);
    adopted.set(id, label.adopted_side);
  }
  const groups = new Map<string, ResolveRun[]>();
  for (const run of runs) {
    const key = `seed ${run.seed}${run.swapped ? ", swapped" : ""}`;
    groups.set(key, [...(groups.get(key) ?? []), run]);
  }
  const rows: string[][] = [];
  for (const [key, group] of [...groups].sort(([left], [right]) => left.localeCompare(right))) {
    const outcomes = group.map(run => outcomeOf(run.chosen, adopted.get(run.id)!));
    const count = (outcome: Outcome) => outcomes.filter(value => value === outcome).length;
    const decided = group.filter(run => run.chosen !== null);
    const decidedCorrect = decided.filter(run => run.chosen === adopted.get(run.id)).length;
    const byAdopted = (kind: Adopted) => {
      const subset = group.filter(run => adopted.get(run.id) === kind);
      const values = subset.map(run => outcomeOf(run.chosen, kind));
      return `${values.filter(value => value === "correct").length}/${values.filter(value => value === "wrong").length}/${values.filter(value => value === "escalated").length}`;
    };
    rows.push([
      key,
      `${count("correct")}/${count("wrong")}/${count("escalated")}`,
      interval(ratio(count("wrong"), group.length)),
      interval(ratio(decidedCorrect, decided.length)),
      interval(ratio(group.filter(run => run.chosen === null).length, group.length)),
      byAdopted("first"),
      byAdopted("second"),
      byAdopted("neither"),
    ]);
  }
  console.log(table(["runs", "correct/wrong/escalated", "wrong over all", "accuracy over decided", "escalated", "first adopted c/w/e", "second adopted c/w/e", "neither c/w/e"], rows));
  console.log("\nPass bars: wrong ≤ 5% of pairs, accuracy over decided ≥ 60%, escalation < 50%.\n");
  const ids = [...adopted.keys()].sort();
  const keys = [...groups.keys()].sort();
  console.log(table(["pair", "adopted", ...keys], ids.map(id => [
    id,
    adopted.get(id)!,
    ...keys.map(key => {
      const run = groups.get(key)!.find(candidate => candidate.id === id);
      return run === undefined ? "" : `${outcomeOf(run.chosen, adopted.get(id)!)} (${run.chosen ?? "esc"}, ${run.step})`;
    }),
  ])));
  const steps = new Map<string, number>();
  for (const run of runs) steps.set(run.step, (steps.get(run.step) ?? 0) + 1);
  console.log(`\nDeciding step over all runs: ${[...steps].map(([step, count]) => `${step} ${count}`).join(", ")}`);
  const pairsSwapped = runs.filter(run => run.swapped);
  if (pairsSwapped.length > 0) {
    let same = 0;
    const differing: string[] = [];
    for (const run of pairsSwapped) {
      const straight = runs.find(other => !other.swapped && other.id === run.id && other.seed === run.seed);
      if (straight === undefined) continue;
      if (straight.chosen === run.chosen) same += 1;
      else differing.push(`${run.id} seed ${run.seed}: ${straight.chosen ?? "escalated"} vs swapped ${run.chosen ?? "escalated"}`);
    }
    console.log(`\nOrder invariance: the same design picked (or both escalated) in ${same}/${pairsSwapped.length} swapped runs.${differing.length > 0 ? `\n  ${differing.join("\n  ")}` : ""}`);
  }
}

// ------------------------------------------------------------------------------------------------------- log smoke

async function logSmoke(flags: Flags): Promise<void> {
  const runs = required(flags, "results").split(",").flatMap(path => readJson<{ runs: ResolveRun[] }>(path).runs).filter(run => run.seed === 0 && !run.swapped);
  const { records } = readJson<{ records: PairRecord[] }>(required(flags, "records"));
  const repos = flags.get("repos") ?? DEFAULT_REPOS;
  const scratch = resolve(required(flags, "scratch"));
  mkdirSync(scratch, { recursive: true });
  const written = new Map<string, string[]>();
  for (const run of runs) {
    const entry = records.find(candidate => candidate.id === run.id)!;
    const clone = join(scratch, entry.repo.replace("/", "__"));
    if (!existsSync(clone)) execFileSync("git", ["clone", "--quiet", "--bare", "--shared", repoPath(repos, entry.repo), clone]);
    const decision = decisionEntryFor(entry.record, run.resolution);
    const appended = appendDecision(clone, decision);
    if (!appended.ok) throw new Error(`${run.id}: ${appended.reason}`);
    written.set(clone, [...(written.get(clone) ?? []), decision.id]);
    console.log(`${run.id}: ${decision.verdict} by ${decision.step} → ${appended.commit.slice(0, 12)} in ${clone}`);
  }
  let matched = 0;
  for (const [clone, ids] of written) {
    const log = await readDecisions(clone);
    const read = log.entries.map(logged => logged.entry.id);
    const ok = ids.every(id => read.includes(id)) && log.problems.length === 0;
    if (ok) matched += 1;
    console.log(`${clone}: wrote ${ids.length}, read back ${read.length}${log.problems.length > 0 ? `, problems: ${log.problems.join("; ")}` : ""} ${ok ? "OK" : "MISMATCH"}`);
  }
  console.log(`\nDecision log smoke: ${runs.length} entries in ${written.size} scratch clones, ${matched}/${written.size} read back whole.`);
  if (matched !== written.size) process.exitCode = 1;
}

// ------------------------------------------------------------------------------------------------------- main

type Flags = Map<string, string>;

function required(flags: Flags, name: string): string {
  const value = flags.get(name);
  if (value === undefined) throw new Error(`--${name} is required`);
  return value;
}

const BOOLEAN_FLAGS: Record<string, true> = { swap: true, "no-judge": true };

function parseFlags(argv: readonly string[]): Flags {
  const flags: Flags = new Map();
  for (let index = 0; index < argv.length; index += 1) {
    const flag = argv[index]!;
    if (!flag.startsWith("--")) throw new Error(`unexpected argument ${flag}`);
    const name = flag.slice(2);
    if (BOOLEAN_FLAGS[name] === true) flags.set(name, "true");
    else {
      const value = argv[index + 1];
      if (value === undefined) throw new Error(`${flag} needs a value`);
      flags.set(name, value);
      index += 1;
    }
  }
  return flags;
}

async function main(): Promise<void> {
  const [command, ...rest] = process.argv.slice(2);
  const flags = parseFlags(rest);
  if (command === "build") await build(flags);
  else if (command === "checks") await checks(flags);
  else if (command === "resolve") await resolveAll(flags);
  else if (command === "score") score(flags);
  else if (command === "log-smoke") await logSmoke(flags);
  else throw new Error("usage: merge-assist-resolve.ts build|checks|resolve|score|log-smoke [flags] (see the header)");
}

if (process.argv[1] !== undefined && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) await main();
