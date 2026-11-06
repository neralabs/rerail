/**
 * Architecture-conflict replay: does delivering the alignment ledger to a real Codex agent, as it first touches the
 * areas another in-flight change established positions in, keep its solution from conflicting with that change?
 *
 *   node --import tsx benchmark/merge-assist-replay.ts --dataset <dir> [--pairs id,id] [--trials N]
 *     [--arms none,align,git-conflict,git-conflict-pull] [--concurrency N] [--timeout-min N] [--judge-timeout-min N]
 *     [--json out.json] [--repo path] [--repos dir] [--align-module path] [--model m] [--judge-model m] [-c key=value]
 *     [--keep] [--tamper] [--detect-only]
 *
 * `--dataset` holds `pairs.json` (`{ pairs: Pair[] }`) and `replay/<id>.json` (the agent-replay specs), in the formats
 * of the shared spec (`/tmp/ma-ctx/align/oss-spec.md`); every pair with a spec runs unless `--pairs` names some. A
 * pair's repository is `--repo`, else `<--repos>/<owner>__<name>` (`/tmp/ma-ctx/oss/repos` by default), and must
 * hold the spec's `base_sha`, first's `head_sha` and the spec's `first_visible`.
 *
 * Each run (pair × arm × trial) gets a fresh folder with a repository holding only the base snapshot (one commit, no
 * refs or objects of the source: nothing of first's or second's later work can be found with git), the spec's
 * `setup` run and committed, and an isolated CODEX_HOME (the person's auth.json through a symlink, hooks.json with
 * merge-assist-replay-hook.ts on SessionStart, PreToolUse and PostToolUse). `codex exec` gets the spec's `task` on
 * stdin, at most `--timeout-min` minutes (20 by default). Under `align`, the hook delivers the ledger units the
 * alignment module's `ledgerAt` builds from first (and `concurrent`) as visible at second's start (see `ledgerTime`),
 * at each area's first touch, through `deliverForArea`: at most 3 per area, contested first, each contested unit's
 * "Note: …" line to end the final message with. Under `none` the hook only logs. Under `git-conflict` and
 * `git-conflict-pull`, the hook identifies after every tool call, read-only, which of the agent's changes would conflict
 * with first (as of `first_visible`) or a `concurrent` ref (its head as of second's start): `git diff --unified=0`
 * hunks of both sides as base lines touching or at most 3 lines apart in the same file (merge-assist-replay-git.ts).
 * `git-conflict` pushes each (ref, file) once with their hunk and stated intent; `git-conflict-pull` keeps the list
 * in the workspace's `.git/rerail-conflicts.md` (`git conflicts`, a repo-local alias, prints it) and sends one line
 * when the set changes. `--detect-only` runs no agent: it replays every selected pair's second commit by commit
 * through the same identifier, against first and `concurrent` as visible at each commit, and prints when the first
 * conflict with first was identified against `noticed_at`, and the detections on negatives.
 *
 * Every Codex call (agent and grader) runs under the permission profile `sandboxConfig` writes (the workspace only,
 * no network, `/tmp`, `$TMPDIR` and the home folder but its toolchains denied), and every run's tool calls go through
 * `findLeaks`: a run with a call that reaches past its folder is marked `leaked` and left out of every aggregate.
 * `--tamper` checks the guard: one `none` run per pair asked to `ls /tmp/ma-ctx/oss`, failing unless each is flagged.
 * Recorded per run: the diff against the base (after setup), Codex's `--json` events, the hook log (each unit with the
 * file and time it went out at), tokens, wall time and the final message. Graded per run, blind: a separate `codex
 * exec` (no hooks, empty folder, `--output-schema`) sees the task, the rubric, `done` and the diff, with every added
 * line that could name the arm stripped ("Note:", "Rerail" (only its unit labels in Rerail's own repository), first's
 * and the other refs' authors, branch and PR numbers, the `git-conflict` arms' wording), and answers
 * `conflicts_with_first` (yes/no/unclear), its reasons citing diff lines
 * and `done` (yes/partial/no). Checked per run, deterministically: the diff applied to `base_sha` as a commit,
 * `git merge-tree` against first's head and against `first_visible` (conflicted files and hunks), and which of
 * first's files (its `first_half` files, and every file it changed up to `first_visible`) the diff touches.
 *
 * It prints the ledger per pair, a row per run, leaked runs with their calls, then aggregates per arm and per pair ×
 * arm; `--json` writes all of it, events and hook logs included. Folders are removed unless `--keep`.
 */
import { execFileSync, spawnSync } from "node:child_process";
import { existsSync, mkdirSync, readdirSync, readFileSync, realpathSync, writeFileSync } from "node:fs";
import { mkdtemp, realpath, rm } from "node:fs/promises";
import { homedir, tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { z } from "zod";
import { clip, type CodexRunOptions, createCodexHome, fixed, GIT_ENV, mean, median, pool, readEvents, runCodex, table, transcriptText, type Usage } from "./merge-assist-context.js";
import { type AlignModule, CHECK_COMMAND, GIT_ARMS, type LedgerUnit, loadAlignModule, REPLAY_ARMS, type ReplayArm, type ReplayHookConfig, type ReplayHookLogEntry } from "./merge-assist-replay-hook.js";
import { commitRanges, conflictKey, gitRead, identify, type LineRange, type RefHunk, refFiles, refHunks, type RefSide, statedIntent } from "./merge-assist-replay-git.js";

const HOOK = fileURLToPath(new URL("./merge-assist-replay-hook.ts", import.meta.url));
const DEFAULT_ALIGN_MODULE = fileURLToPath(new URL("./merge-assist-align.ts", import.meta.url));
const DEFAULT_REPOS = "/tmp/ma-ctx/oss/repos";
/** Where `ledgerAt` keeps what it learns between invocations. */
const LEDGER_CACHE = "/tmp/ma-ctx/align/replay-cache";
const DEFAULT_TIMEOUT_MIN = 20;
const DEFAULT_JUDGE_TIMEOUT_MIN = 10;
const SETUP_TIMEOUT_MS = 15 * 60_000;
/** `--tamper`'s task: one the leak guard must flag, whatever the sandbox lets through. */
const TAMPER_TASK = "Run `ls /tmp/ma-ctx/oss` in a shell and tell me exactly what it printed.";
/** The most diff characters the grader reads; a longer diff is cut there, and the grader is told. */
const GRADE_DIFF_CHARS = 200_000;

// ---------------------------------------------------------------------------------------------------------- formats

const commitSchema = z.object({ sha: z.string().min(7), authored_at: z.string(), visible_at: z.string() }).passthrough();
const sideSchema = z.object({
  pr: z.number().int().nullable().optional(),
  branch: z.string().optional(),
  /** Rerail's dev labels name each side's ref (`refs/remotes/origin/<branch>`). */
  ref: z.string().optional(),
  title: z.string(),
  author: z.string(),
  created_at: z.string(),
  closed_at: z.string().nullable(),
  merged_at: z.string().nullable(),
  base_sha: z.string().min(7),
  head_sha: z.string().min(7),
  commits: z.array(commitSchema),
}).passthrough();
const halfSchema = z.object({ files: z.array(z.string()), symbols: z.array(z.string()), evidence: z.string() }).passthrough();
const pairSchema = z.object({
  id: z.string().min(1),
  repo: z.string().regex(/^[^/\s]+\/[^/\s]+$/u),
  language: z.string(),
  kind: z.enum(["conflict", "negative"]),
  first: sideSchema,
  second: sideSchema,
  concurrent: z.array(z.union([z.number().int(), z.string()])),
  conflict: z.object({ type: z.string(), summary: z.string(), first_half: halfSchema, second_half: halfSchema }).passthrough().optional(),
}).passthrough();
type Pair = z.infer<typeof pairSchema>;
const specSchema = z.object({
  id: z.string().min(1),
  base_sha: z.string().min(7),
  task: z.string().min(1),
  first_visible: z.string().min(7),
  rubric: z.string().min(1),
  done: z.string().min(1),
  setup: z.string().nullable(),
}).passthrough();
type ReplaySpec = z.infer<typeof specSchema>;

const VERDICTS = ["yes", "no", "unclear"] as const;
const DONE = ["yes", "partial", "no"] as const;
const verdictSchema = z.object({ conflicts_with_first: z.enum(VERDICTS), reasons: z.array(z.string()), done: z.enum(DONE) }).strict();
type Verdict = z.infer<typeof verdictSchema>;
/** What `--output-schema` holds the grader's final message to: {@link verdictSchema} as JSON Schema. */
const VERDICT_JSON_SCHEMA = {
  type: "object",
  properties: {
    conflicts_with_first: { type: "string", enum: VERDICTS },
    reasons: { type: "array", items: { type: "string" } },
    done: { type: "string", enum: DONE },
  },
  required: ["conflicts_with_first", "reasons", "done"],
  additionalProperties: false,
};

// ---------------------------------------------------------------------------------------------------------- options

interface Options {
  dataset: string;
  pairs: string[] | null;
  trials: number;
  arms: ReplayArm[];
  concurrency: number;
  timeoutMin: number;
  judgeTimeoutMin: number;
  json: string | undefined;
  repo: string | undefined;
  repos: string;
  alignModule: string;
  model: string | undefined;
  judgeModel: string | undefined;
  config: string[];
  keep: boolean;
  /** `--tamper`: every selected pair once under `none` with {@link TAMPER_TASK}; fails unless each run is flagged. */
  tamper: boolean;
  /** `--detect-only`: no agent; second's real commits through the `git-conflict` identifier, every selected pair. */
  detectOnly: boolean;
}

const USAGE = "Usage: node --import tsx benchmark/merge-assist-replay.ts --dataset dir [--pairs id,id] [--trials N] [--arms none,align,git-conflict,git-conflict-pull] [--concurrency N] [--timeout-min N] [--judge-timeout-min N] [--json out.json] [--repo path] [--repos dir] [--align-module path] [--model m] [--judge-model m] [-c key=value] [--keep] [--tamper] [--detect-only]";

function parseOptions(argv: readonly string[]): Options {
  const options: Options = {
    dataset: "",
    pairs: null,
    trials: 1,
    arms: [...REPLAY_ARMS],
    concurrency: 2,
    timeoutMin: DEFAULT_TIMEOUT_MIN,
    judgeTimeoutMin: DEFAULT_JUDGE_TIMEOUT_MIN,
    json: undefined,
    repo: undefined,
    repos: DEFAULT_REPOS,
    alignModule: DEFAULT_ALIGN_MODULE,
    model: undefined,
    judgeModel: undefined,
    config: [],
    keep: false,
    tamper: false,
    detectOnly: false,
  };
  const value = (flag: string, index: number): string => {
    const next = argv[index + 1];
    if (next === undefined || next.startsWith("--")) throw new Error(`${flag} takes a value\n${USAGE}`);
    return next;
  };
  const positive = (flag: string, text: string): number => {
    const number = Number(text);
    if (!Number.isInteger(number) || number < 1) throw new Error(`${flag} takes a positive integer\n${USAGE}`);
    return number;
  };
  for (let index = 0; index < argv.length; index++) {
    const flag = argv[index]!;
    if (flag === "--keep" || flag === "--tamper" || flag === "--detect-only") {
      if (flag === "--keep") options.keep = true;
      else if (flag === "--tamper") options.tamper = true;
      else options.detectOnly = true;
      continue;
    }
    const text = value(flag, index);
    index++;
    if (flag === "--dataset") options.dataset = resolve(text);
    else if (flag === "--pairs") options.pairs = text.split(",").filter(id => id !== "");
    else if (flag === "--trials") options.trials = positive(flag, text);
    else if (flag === "--arms") {
      const arms = text.split(",").filter(arm => arm !== "");
      const unknown = arms.filter(arm => !(REPLAY_ARMS as readonly string[]).includes(arm));
      if (unknown.length > 0 || arms.length === 0) throw new Error(`--arms takes ${REPLAY_ARMS.join(",")}; not ${unknown.join(",")}`);
      options.arms = arms as ReplayArm[];
    } else if (flag === "--concurrency") options.concurrency = positive(flag, text);
    else if (flag === "--timeout-min") options.timeoutMin = positive(flag, text);
    else if (flag === "--judge-timeout-min") options.judgeTimeoutMin = positive(flag, text);
    else if (flag === "--json") options.json = resolve(text);
    else if (flag === "--repo") options.repo = resolve(text);
    else if (flag === "--repos") options.repos = resolve(text);
    else if (flag === "--align-module") options.alignModule = resolve(text);
    else if (flag === "--model") options.model = text;
    else if (flag === "--judge-model") options.judgeModel = text;
    else if (flag === "-c") options.config.push(text);
    else throw new Error(`Unknown option ${flag}\n${USAGE}`);
  }
  if (options.dataset === "") throw new Error(`--dataset is required\n${USAGE}`);
  if (options.tamper) {
    options.arms = ["none"];
    options.trials = 1;
  }
  return options;
}

// ---------------------------------------------------------------------------------------------------------- git

function git(cwd: string, args: readonly string[], env: NodeJS.ProcessEnv = {}, input?: string): string {
  return execFileSync("git", args, {
    cwd,
    env: { ...GIT_ENV, ...env },
    encoding: "utf8",
    maxBuffer: 256 * 1024 * 1024,
    ...(input === undefined ? { stdio: ["ignore", "pipe", "pipe"] } : { input, stdio: ["pipe", "pipe", "pipe"] }),
  });
}

function hasCommit(repo: string, sha: string): boolean {
  return spawnSync("git", ["cat-file", "-e", `${sha}^{commit}`], { cwd: repo, env: GIT_ENV }).status === 0;
}

/**
 * A bare repository in `folder` that reads `alternates`' objects and writes its own: the merge checks' commits and
 * trees never land in the source repository.
 */
function scratchRepository(folder: string, alternates: readonly string[]): string {
  const scratch = join(folder, "scratch.git");
  git(folder, ["init", "-q", "--bare", scratch]);
  writeFileSync(join(scratch, "objects", "info", "alternates"), `${alternates.join("\n")}\n`);
  return scratch;
}

/** `git merge-tree` of two commits: the files it leaves conflicted and their conflict hunks (`<<<<<<<` markers). */
export interface MergeCheck {
  readonly clean: boolean;
  readonly files: readonly string[];
  readonly hunks: number;
}

function mergeCheck(scratch: string, ours: string, theirs: string): MergeCheck {
  const result = spawnSync("git", ["merge-tree", "--write-tree", "--name-only", "--no-messages", ours, theirs], { cwd: scratch, env: GIT_ENV, encoding: "utf8", maxBuffer: 64 * 1024 * 1024 });
  if (result.status !== 0 && result.status !== 1) throw new Error(`git merge-tree ${ours} ${theirs}: ${result.stderr.trim()}`);
  const [tree, ...rest] = result.stdout.split("\n").filter(line => line !== "");
  const files = [...new Set(rest)];
  let hunks = 0;
  for (const path of files) {
    const blob = spawnSync("git", ["cat-file", "-p", `${tree}:${path}`], { cwd: scratch, env: GIT_ENV, encoding: "utf8", maxBuffer: 64 * 1024 * 1024 });
    if (blob.status === 0) hunks += (blob.stdout.match(/^<{7}(?: |$)/gmu) ?? []).length;
  }
  return { clean: result.status === 0, files, hunks };
}

// ---------------------------------------------------------------------------------------------------------- pairs

interface PairContext {
  readonly pair: Pair;
  readonly spec: ReplaySpec;
  /** The source repository the base snapshot comes from. */
  readonly source: string;
  /** The source's objects folder, as a scratch repository's alternate. */
  readonly objects: string;
  /** Second's start (`ledgerTime`): the time the ledger is built at. */
  readonly ledgerTime: string;
  /** null when no arm needs the ledger. */
  readonly units: readonly LedgerUnit[] | null;
  readonly ledgerMs: number | null;
  readonly ledgerError: string | null;
  /** Every file first changed from where it branched up to `first_visible`. */
  readonly firstFiles: readonly string[];
  /** Second's own head merged with first's: the conflict the real history had. */
  readonly reference: MergeCheck | null;
  /** First and the `concurrent` refs as visible at `ledgerTime`, with their hunks: what the `git-conflict` arms identify against. */
  readonly refs: readonly RefSide[];
  readonly refsMs: number;
}

function loadPairs(dataset: string): Pair[] {
  return z.object({ pairs: z.array(pairSchema) }).passthrough().parse(JSON.parse(readFileSync(join(dataset, "pairs.json"), "utf8"))).pairs;
}

function loadDataset(options: Options): Array<{ pair: Pair; spec: ReplaySpec }> {
  const pairs = loadPairs(options.dataset);
  const specFolder = join(options.dataset, "replay");
  const specs = new Map<string, ReplaySpec>();
  for (const name of existsSync(specFolder) ? readdirSync(specFolder).filter(file => file.endsWith(".json")) : []) {
    const spec = specSchema.parse(JSON.parse(readFileSync(join(specFolder, name), "utf8")));
    specs.set(spec.id, spec);
  }
  const wanted = options.pairs ?? pairs.filter(pair => specs.has(pair.id)).map(pair => pair.id);
  return wanted.map(id => {
    const pair = pairs.find(candidate => candidate.id === id);
    const spec = specs.get(id);
    if (pair === undefined) throw new Error(`No pair ${id} in ${join(options.dataset, "pairs.json")}`);
    if (spec === undefined) throw new Error(`No replay spec for ${id} in ${specFolder}`);
    return { pair, spec };
  });
}

/** A side's ref: the one Rerail's labels name, else its pull request's head. */
function sideRef(side: Pair["first"]): string | null {
  return side.ref ?? (typeof side.pr === "number" ? `refs/pull/${side.pr}/head` : null);
}

/** How a person names a ref: `PR #12`, or `branch a/b`. */
function refLabel(ref: string): string {
  const pr = /^refs\/pull\/(\d+)\/head$/u.exec(ref);
  return pr === null ? `branch ${shortRef(ref)}` : `PR #${pr[1]}`;
}

/** Each ref's first-parent history as (commit, commit time), newest first: read once per source and ref. */
const timelines = new Map<string, Array<{ sha: string; time: number }>>();

/** `ref`'s first commit on its first-parent walk committed at or before `time` (`git rev-list -1 --first-parent --before`). */
function refAt(source: string, ref: string, time: string): string | null {
  const key = `${source}\0${ref}`;
  let timeline = timelines.get(key);
  if (timeline === undefined) {
    timeline = (gitRead(source, ["log", "--first-parent", "--format=%H %ct", ref, "--"]) ?? "").split("\n").filter(line => line !== "").map(line => {
      const [sha, seconds] = line.split(" ") as [string, string];
      return { sha, time: Number(seconds) * 1000 };
    });
    timelines.set(key, timeline);
  }
  const limit = Date.parse(time);
  return timeline.find(entry => entry.time <= limit)?.sha ?? null;
}

/**
 * The other refs as visible at `time`: first at `firstSha`, then each `concurrent` ref other than the two sides at
 * its newest first-parent commit committed by then.
 */
function visibleShas(source: string, pair: Pair, time: string, firstSha: string | null): Array<{ ref: string; sha: string }> {
  const firstRef = sideRef(pair.first) ?? "first";
  const secondRef = sideRef(pair.second);
  const visible = firstSha === null ? [] : [{ ref: firstRef, sha: firstSha }];
  for (const entry of pair.concurrent) {
    const ref = typeof entry === "number" ? `refs/pull/${entry}/head` : entry;
    if (ref === firstRef || ref === secondRef) continue;
    const sha = refAt(source, ref, time);
    if (sha !== null) visible.push({ ref, sha });
  }
  return visible;
}

/**
 * What the `git-conflict` arms identify against: each visible ref with its own commits (from where it meets the
 * base), its author, when it became public, its stated intent and its hunks in the base's lines. First's author and
 * pull request title come from the pair; a `concurrent` ref's from its head and its latest commit subjects.
 */
function visibleRefs(source: string, pair: Pair, base: string, time: string, firstSha: string): RefSide[] {
  const firstRef = sideRef(pair.first) ?? "first";
  return visibleShas(source, pair, time, firstSha).flatMap(({ ref, sha }): RefSide[] => {
    const mergeBase = gitRead(source, ["merge-base", base, sha])?.trim() ?? "";
    const own = (gitRead(source, ["log", "--format=%cI", mergeBase === "" ? sha : `${mergeBase}..${sha}`]) ?? "").split("\n").filter(line => line !== "");
    if (own.length === 0) return [];
    const files = refHunks(source, base, sha);
    if (Object.keys(files).length === 0) return [];
    if (ref === firstRef) {
      const shown = pair.first.commits.filter(commit => Date.parse(commit.visible_at) <= Date.parse(time)).map(commit => commit.visible_at);
      const since = [...shown, pair.first.created_at].reduce((earliest, at) => (Date.parse(at) < Date.parse(earliest) ? at : earliest));
      const body = typeof pair.first.body === "string" ? pair.first.body : undefined;
      const pr = typeof pair.first.pr === "number" ? { title: pair.first.title, ...(body === undefined ? {} : { body }) } : null;
      return [{ label: refLabel(ref), ref, sha, author: pair.first.author, since, intent: statedIntent(source, base, sha, pr), files }];
    }
    const author = gitRead(source, ["log", "-1", "--format=%an", sha])?.trim() ?? "unknown";
    return [{ label: refLabel(ref), ref, sha, author, since: own.at(-1)!, intent: statedIntent(source, base, sha, null), files }];
  });
}

/**
 * When the replayed work started. A spec whose `base_sha` is one of second's own commits replays the work after it
 * (a long branch replayed from the middle), so that commit's authoring; otherwise second's creation, or its earliest
 * authored commit when that is earlier.
 */
function ledgerTime(pair: Pair, spec: ReplaySpec): string {
  const base = pair.second.commits.find(commit => commit.sha.startsWith(spec.base_sha) || spec.base_sha.startsWith(commit.sha));
  if (base !== undefined) return base.authored_at;
  const times = [pair.second.created_at, ...pair.second.commits.map(commit => commit.authored_at)];
  return times.reduce((earliest, time) => (Date.parse(time) < Date.parse(earliest) ? time : earliest));
}

async function preparePair(pair: Pair, spec: ReplaySpec, options: Options, align: AlignModule | null): Promise<PairContext> {
  const source = options.repo ?? join(options.repos, pair.repo.replace("/", "__"));
  if (!existsSync(source)) throw new Error(`${pair.id}: no repository at ${source}`);
  for (const [name, sha] of [["base_sha", spec.base_sha], ["first.head_sha", pair.first.head_sha], ["first_visible", spec.first_visible]] as const) {
    if (!hasCommit(source, sha)) throw new Error(`${pair.id}: ${source} has no commit ${sha} (${name})`);
  }
  const objects = join(git(source, ["rev-parse", "--path-format=absolute", "--git-common-dir"]).trim(), "objects");
  const firstFiles = git(source, ["diff", "--name-only", `${pair.first.base_sha}...${spec.first_visible}`]).split("\n").filter(path => path !== "");
  let reference: MergeCheck | null = null;
  if (hasCommit(source, pair.second.head_sha)) {
    const folder = await realpath(await mkdtemp(join(tmpdir(), "rerail-ma-replay-pair-")));
    try {
      reference = mergeCheck(scratchRepository(folder, [objects]), pair.second.head_sha, pair.first.head_sha);
    } finally {
      await rm(folder, { recursive: true, force: true });
    }
  }
  const time = ledgerTime(pair, spec);
  let units: LedgerUnit[] | null = null;
  let ledgerError: string | null = null;
  let ledgerMs: number | null = null;
  if (align !== null) {
    const started = performance.now();
    try {
      units = await align.ledgerAt(source, pair, time, { cacheDir: LEDGER_CACHE });
    } catch (error) {
      ledgerError = error instanceof Error ? error.message : String(error);
    }
    ledgerMs = Math.round(performance.now() - started);
  }
  const refsStarted = performance.now();
  const refs = options.tamper ? [] : visibleRefs(source, pair, spec.base_sha, time, spec.first_visible);
  const refsMs = Math.round(performance.now() - refsStarted);
  return { pair, spec, source, objects, ledgerTime: time, units, ledgerMs, ledgerError, firstFiles, reference, refs, refsMs };
}

// ---------------------------------------------------------------------------------------------------------- sandbox

/** The permission profile every replay Codex call runs under (agent and grader). */
const PROFILE = "replay";
/** Folders under the home folder an agent may read: toolchains it may run. Everything else there is denied. */
const HOME_TOOLCHAINS = [".cargo", ".rustup", "go", ".nvm", ".pyenv", ".bun", ".volta", ".deno", ".local/bin"];
/** Where a command may read without it counting as a leak: the system and the toolchains, besides the run's folder. */
const SYSTEM_ROOTS = ["/bin", "/sbin", "/usr", "/dev", "/etc", "/private/etc", "/opt/homebrew", "/Library/Developer", "/System", "/Applications/Xcode.app"];

/**
 * The real file behind `codex` on PATH. Every replay call runs it by that path: Codex's own sandbox helper re-runs the
 * executable it was started as, and a symlink in the denied home folder cannot be followed from inside the sandbox.
 */
function codexBinary(): string {
  for (const folder of (process.env.PATH ?? "").split(":")) {
    if (existsSync(join(folder, "codex"))) return realpathSync(join(folder, "codex"));
  }
  throw new Error("No codex on PATH");
}

/** What the replay's agents may read under the home folder: toolchains, Codex's release folder, and this Node. */
function homeReadable(): string[] {
  const home = homedir();
  return [...HOME_TOOLCHAINS.map(folder => join(home, folder)), dirname(dirname(codexBinary())), dirname(dirname(process.execPath))].filter(path => path.startsWith(`${home}/`) && existsSync(path));
}

/**
 * The run's CODEX_HOME config: a permission profile that extends Codex's workspace profile (the workspace root
 * writable, no network) and takes away what the default leaves open: writes and reads in `/tmp` and `$TMPDIR` (other
 * runs' folders, the curated data under `/tmp/ma-ctx`), the whole home folder but its toolchains (every other clone),
 * and each of `denied` (the source repository, the dataset, the ledger cache).
 */
export function sandboxConfig(denied: readonly string[]): string {
  const entries: Array<[string, string]> = [
    [":slash_tmp", "deny"],
    ["/private/tmp", "deny"],
    [":tmpdir", "deny"],
    [homedir(), "deny"],
    ...denied.filter(path => existsSync(path)).map((path): [string, string] => [path, "deny"]),
    ...homeReadable().map((path): [string, string] => [path, "read"]),
  ];
  return [
    `default_permissions = ${JSON.stringify(PROFILE)}`,
    "",
    `[permissions.${PROFILE}]`,
    `description = "Merge Assist replay: the workspace only, no network"`,
    `extends = ":workspace"`,
    "",
    `[permissions.${PROFILE}.filesystem]`,
    ...[...new Map(entries)].map(([path, access]) => `${JSON.stringify(path)} = ${JSON.stringify(access)}`),
    "",
    `[permissions.${PROFILE}.network]`,
    "enabled = false",
    "",
  ].join("\n");
}

/** One thing the agent did that reaches past its workspace: where it was seen, why it counts, and the call's text. */
export interface Leak {
  readonly source: string;
  readonly reason: string;
  readonly text: string;
}

/** Commands and hosts that reach other copies of the history: the network, GitHub, pull request refs. */
const OUTSIDE: ReadonlyArray<readonly [string, RegExp]> = [
  ["network command", /(?:^|[\s;&|('"`])(?:curl|wget|gh|nc|ssh|scp|rsync)\s/u],
  ["git network command", /\bgit\b[^\n;&|]*\s(?:fetch|clone|pull|ls-remote|submodule\s+update)\b|\bremote\s+(?:add|set-url)\b/u],
  ["remote host", /\b(?:github\.com|gitlab\.com|bitbucket\.org|codeload\.github|api\.github)\b/u],
  ["pull request ref", /refs\/pull\//u],
  ["curated data", /\/tmp\/ma-ctx\b/u],
];
/** A token that starts a path: absolute, from the home folder, or climbing with `..`. */
const PATH_TOKEN = /(?:^|[\s'"`=:(,;|&<>[{])((?:~|\$HOME|\$\{HOME\})?\/[^\s'"`;|&<>(),\]}]*|(?:\.\.\/)+[^\s'"`;|&<>(),\]}]*|\.\.(?=$|[\s'"`;|&)]))/gu;

/** Why `text` (one call's command or input) reaches past `folder`, the run's folder; none when it stays inside. */
export function leakReasons(text: string, folder: string, cwd: string): string[] {
  const reasons = OUTSIDE.flatMap(([reason, pattern]) => (pattern.test(text) ? [reason] : []));
  const inside = [folder, folder.replace(/^\/private\//u, "/")];
  const allowed = [...inside, ...SYSTEM_ROOTS, ...homeReadable()];
  for (const match of text.matchAll(PATH_TOKEN)) {
    const token = match[1]!;
    if (token.startsWith("//")) continue;
    const home = /^(?:~|\$HOME|\$\{HOME\})\//u.exec(token);
    const path = home !== null ? join(homedir(), token.slice(home[0].length)) : token.startsWith("/") ? resolve(token) : resolve(cwd, token);
    // `/\d+/` and other regular expressions in code cells: only a path under a real top-level folder counts.
    const top = path.split("/")[1] ?? "";
    if (top === "" || !existsSync(`/${top}`)) continue;
    if (allowed.some(root => path === root || path.startsWith(`${root}/`))) continue;
    reasons.push(`path ${path}`);
  }
  return [...new Set(reasons)];
}

/**
 * Every tool call the agent made, as text with the folder it ran in: from Codex's `--json` events (commands, patched
 * files, MCP calls, web searches) and from its session transcript, which also holds calls the sandbox refused and
 * those a code-mode `exec` cell made. A patch counts by the files it names, not by the text it adds.
 */
function callTexts(events: readonly unknown[], transcript: string | null, repo: string): Array<{ source: string; text: string; cwd: string; reason?: string }> {
  const calls: Array<{ source: string; text: string; cwd: string; reason?: string }> = [];
  const patchFiles = (patch: string) => [...patch.matchAll(/^\*\*\* (?:Add File|Update File|Delete File|Move to): (.+)$/gmu)].map(match => match[1]!).join("\n");
  for (const event of events as Array<Record<string, unknown>>) {
    const item = event.item as Record<string, unknown> | undefined;
    if (event.type !== "item.completed" && event.type !== "item.started") continue;
    if (item?.type === "command_execution") calls.push({ source: "event command", text: String(item.command), cwd: repo });
    else if (item?.type === "file_change") calls.push({ source: "event file change", text: ((item.changes ?? []) as Array<{ path?: string }>).map(change => change.path ?? "").join("\n"), cwd: repo });
    else if (item?.type === "mcp_tool_call") calls.push({ source: "event MCP call", text: JSON.stringify(item), cwd: repo });
    else if (item?.type === "web_search") calls.push({ source: "event web search", text: JSON.stringify(item.query ?? item), cwd: repo, reason: "web search" });
  }
  for (const line of (transcript ?? "").split("\n")) {
    if (!line.startsWith("{")) continue;
    let entry: Record<string, unknown>;
    try {
      entry = JSON.parse(line) as Record<string, unknown>;
    } catch {
      continue;
    }
    const payload = entry.payload as Record<string, unknown> | undefined;
    if (entry.type !== "response_item" || payload === undefined || payload === null) continue;
    const name = String(payload.name ?? payload.type);
    if (payload.type === "function_call") {
      let args: Record<string, unknown> = {};
      try {
        args = JSON.parse(String(payload.arguments)) as Record<string, unknown>;
      } catch {
        args = { raw: String(payload.arguments) };
      }
      const cwd = typeof args.workdir === "string" ? resolve(repo, args.workdir) : repo;
      calls.push({ source: `transcript ${name}`, text: name === "apply_patch" ? patchFiles(String(args.input ?? "")) : JSON.stringify(args.cmd ?? args.command ?? args), cwd });
    } else if (payload.type === "custom_tool_call") {
      const input = String(payload.input ?? "");
      calls.push({ source: `transcript ${name}`, text: name === "apply_patch" ? patchFiles(input) : input.replace(/\*\*\* Begin Patch[\s\S]*?\*\*\* End Patch/gu, patchFiles), cwd: repo });
    } else if (payload.type === "local_shell_call") {
      calls.push({ source: "transcript shell", text: JSON.stringify((payload.action as Record<string, unknown> | undefined)?.command ?? ""), cwd: repo });
    } else if (payload.type === "web_search_call") {
      calls.push({ source: "transcript web search", text: JSON.stringify(payload.action ?? payload), cwd: repo, reason: "web search" });
    }
  }
  return calls;
}

/** The run's leaks: each call whose text reaches past the run's folder, once per text. */
export function findLeaks(events: readonly unknown[], transcript: string | null, folder: string, repo: string): Leak[] {
  const leaks = new Map<string, Leak>();
  for (const call of callTexts(events, transcript, repo)) {
    const reasons = [...(call.reason === undefined ? [] : [call.reason]), ...leakReasons(call.text, folder, call.cwd)];
    if (reasons.length > 0 && !leaks.has(call.text)) leaks.set(call.text, { source: call.source, reason: reasons.join("; "), text: clip(call.text, 400) });
  }
  return [...leaks.values()];
}

// ---------------------------------------------------------------------------------------------------------- grading

/** A ref's branch name: `refs/remotes/origin/a/b` and `refs/heads/a/b` are `a/b`. */
function shortRef(ref: string): string {
  return ref.replace(/^refs\/(?:remotes\/[^/]+|heads)\//u, "");
}

/** Whether a ledger unit came from the pair's first side: its pull request, or its branch. */
function fromFirst(unit: LedgerUnit, pair: Pair): boolean {
  if (unit.pr !== undefined && typeof pair.first.pr === "number") return unit.pr === pair.first.pr;
  const branch = pair.first.ref ?? pair.first.branch;
  return branch !== undefined && unit.branch === shortRef(branch);
}

/**
 * The added lines a grader must not see: anything that could tell it which arm the run was in. `authors` are the
 * other refs' authors the `git-conflict` arms name; the same markers apply to every arm.
 */
function armMarkers(pair: Pair, authors: readonly string[]): RegExp[] {
  const escape = (text: string) => text.replace(/[.*+?^${}()|[\]\\]/gu, "\\$&");
  const markers = [/\bNote:/iu, /decided differently/iu];
  // In Rerail's own repository the word is everywhere; only the hook's unit labels are the arm's.
  markers.push(/rerail/iu.test(pair.repo) ? /\bRerail (?:decision|handoff|position|retirement|contract|copy|ledger)\b/iu : /rerail/iu);
  // The `git-conflict` arms' wording and data fences.
  markers.push(/git conflict in flight/iu, /in[- ]flight conflict/iu, /would conflict with/iu, /\bgit conflicts\b/iu, /rerail-conflicts/iu, /RERAIL-DATA|<<<END [0-9a-f]/u, /\bdiverg(?:e|es|ed|ing)\s+from\b/iu);
  for (const author of new Set([pair.first.author, ...authors])) {
    if (author.length >= 3) markers.push(new RegExp(`\\b${escape(author)}\\b`, "iu"));
  }
  const prs = [pair.first.pr, ...pair.concurrent.filter(item => typeof item === "number")].filter(pr => typeof pr === "number");
  if (prs.length > 0) markers.push(new RegExp(`#(?:${prs.join("|")})\\b`, "u"));
  const refs = [pair.first.branch, pair.first.ref, ...pair.concurrent.filter(item => typeof item === "string")].flatMap(ref => (ref === undefined ? [] : [shortRef(ref)]));
  for (const branch of new Set(refs)) {
    if (branch.length >= 3) markers.push(new RegExp(escape(branch), "u"));
  }
  return markers;
}

/** The diff as the grader reads it: added lines that match {@link armMarkers} left out, then cut at {@link GRADE_DIFF_CHARS}. */
export function graderDiff(diff: string, pair: Pair, authors: readonly string[] = []): { text: string; stripped: number; truncated: boolean } {
  const markers = armMarkers(pair, authors);
  let stripped = 0;
  const kept = diff.split("\n").filter(line => {
    if (!line.startsWith("+") || line.startsWith("+++ ")) return true;
    const strip = markers.some(marker => marker.test(line));
    if (strip) stripped++;
    return !strip;
  }).join("\n");
  const truncated = kept.length > GRADE_DIFF_CHARS;
  return { text: truncated ? `${kept.slice(0, GRADE_DIFF_CHARS)}\n[diff cut here: ${kept.length - GRADE_DIFF_CHARS} more characters]` : kept, stripped, truncated };
}

export function gradePrompt(spec: ReplaySpec, diff: string): string {
  return [
    "You are reviewing a code change made for the task below. Judge it only from the diff; there is nothing else to look at.",
    "",
    "## Task the change was made for",
    spec.task,
    "",
    "## Rubric",
    "How to tell whether a solution conflicts with the design of another change in flight in the same repository:",
    spec.rubric,
    "",
    "## Done",
    "How to tell whether the task is reasonably done:",
    spec.done,
    "",
    "## Diff (against the commit the work started from)",
    "```diff",
    diff,
    "```",
    "",
    "Answer with JSON only. `conflicts_with_first`: \"yes\" when the rubric's statements show the change conflicts with that design, \"no\" when they show it does not, \"unclear\" when the diff does not let you tell. `reasons`: short reasons, each citing the diff lines it rests on (the path and the quoted added or removed line). `done`: \"yes\", \"partial\" or \"no\" by the done criteria.",
  ].join("\n");
}

export interface Grade {
  readonly verdict: Verdict | null;
  /** `grader` for the blind Codex call; `empty-diff` for a run that changed nothing (no conflict, not done), with no call. */
  readonly by: "grader" | "empty-diff" | null;
  /** Why there is no verdict: the grader's failure. */
  readonly error: string | null;
  readonly strippedLines: number;
  readonly truncated: boolean;
  readonly usage: Usage | null;
  readonly wallMs: number;
}

async function gradeRun(context: PairContext, diff: string, folder: string, options: Options, denied: readonly string[]): Promise<Grade> {
  const { spec, pair } = context;
  const { text, stripped, truncated } = graderDiff(diff, pair, context.refs.map(side => side.author));
  if (text.trim() === "") return { verdict: { conflicts_with_first: "no", reasons: ["The run changed no file."], done: "no" }, by: "empty-diff", error: null, strippedLines: stripped, truncated, usage: null, wallMs: 0 };
  const work = join(folder, "grade");
  mkdirSync(work, { recursive: true });
  const schema = join(folder, "grade-schema.json");
  writeFileSync(schema, JSON.stringify(VERDICT_JSON_SCHEMA));
  const codexHome = join(folder, "grade-codex-home");
  createCodexHome(codexHome, "", null);
  writeFileSync(join(codexHome, "config.toml"), sandboxConfig(denied));
  const judge: CodexRunOptions = { model: options.judgeModel, config: [], timeoutMin: options.judgeTimeoutMin, extraArgs: ["--output-schema", schema, "--ephemeral"], permissions: PROFILE, binary: codexBinary() };
  const outcome = await runCodex(work, codexHome, "", gradePrompt(spec, text), judge, join(folder, "grade-rerail-home"));
  const events = readEvents(outcome.stdout);
  const base = { by: null, strippedLines: stripped, truncated, usage: events.usage, wallMs: outcome.wallMs };
  if (outcome.timedOut) return { ...base, verdict: null, error: `grader timed out after ${options.judgeTimeoutMin} minutes` };
  if (events.finalMessage === null) return { ...base, verdict: null, error: events.error ?? `grader exited ${outcome.exitCode} with no answer: ${clip(outcome.stderr, 200)}` };
  try {
    return { ...base, by: "grader", verdict: verdictSchema.parse(JSON.parse(events.finalMessage)), error: null };
  } catch (error) {
    return { ...base, verdict: null, error: `grader answer unreadable (${error instanceof Error ? error.message : String(error)}): ${clip(events.finalMessage, 200)}` };
  }
}

// ---------------------------------------------------------------------------------------------------------- run

interface Job {
  readonly context: PairContext;
  readonly arm: ReplayArm;
  readonly trial: number;
}

export interface ReplayRun {
  readonly pair: string;
  readonly arm: ReplayArm;
  readonly trial: number;
  readonly error: string | null;
  readonly timedOut: boolean;
  readonly exitCode: number | null;
  readonly wallMs: number;
  readonly usage: Usage | null;
  readonly finalMessage: string | null;
  /** The final message holds a "Note: … decided differently …" line. */
  readonly noteLine: boolean;
  /** Whether any call reached past the run's folder (see `findLeaks`); a leaked run is left out of every aggregate. */
  readonly leaked: boolean;
  readonly leaks: readonly Leak[];
  readonly setup: { readonly exitCode: number | null; readonly ms: number; readonly tail: string } | null;
  /** `git diff` of the workspace against the base (after setup), untracked files included. */
  readonly diff: string;
  readonly files: ReadonlyArray<{ readonly path: string; readonly added: number; readonly removed: number }>;
  /** The run's result (the diff on `base_sha`) merged with first's head and with `first_visible`; null when it could not be built. */
  readonly merge: { readonly head: MergeCheck; readonly visible: MergeCheck } | null;
  readonly mergeError: string | null;
  /** The diff's files among first's `first_half` files, and among every file first changed up to `first_visible`. */
  readonly touches: { readonly firstHalf: readonly string[]; readonly firstFiles: readonly string[] };
  /** Each unit the hook delivered: where, and how many seconds into the run. */
  readonly delivered: ReadonlyArray<{ readonly unitId: string; readonly area: string; readonly path: string; readonly conflict: boolean; readonly fromFirst: boolean; readonly atS: number }>;
  /** Hook calls that injected, and how many of their texts Codex's own transcript holds (null without a transcript). */
  readonly injections: number;
  readonly landed: number | null;
  readonly hookErrors: readonly string[];
  readonly hookLog: readonly ReplayHookLogEntry[];
  /** Under the `git-conflict` arms: what the hook identified, and when the agent read the list; null under the others. */
  readonly git: GitOutcome | null;
  /** Codex's `--json` events. */
  readonly events: readonly unknown[];
  readonly grade: Grade | null;
  readonly folder: string | null;
}

/** What the `git-conflict` arms identified over one run, from its hook log and Codex's events. */
export interface GitOutcome {
  /** Refs visible at the start with changes of their own. */
  readonly refs: number;
  /**
   * Each (ref, file) identified: whether the ref is first's, when it was first identified, when its overlap last
   * disappeared (null while it holds at the run's last identification) and when `git-conflict` pushed it, in seconds.
   */
  readonly detections: ReadonlyArray<{ readonly ref: string; readonly file: string; readonly first: boolean; readonly firstAtS: number; readonly resolvedAtS: number | null; readonly pushedAtS: number | null }>;
  /** When the agent ran `git conflicts` or read the list (hook calls after the command), in seconds. */
  readonly checkedAtS: readonly number[];
  /** Commands in Codex's events that ran `git conflicts` or read the list. */
  readonly checks: number;
  /** Conflicts the run's last identification still found. */
  readonly openAtEnd: number;
}

function gitOutcome(context: PairContext, hookLog: readonly ReplayHookLogEntry[], events: readonly unknown[], started: number): GitOutcome {
  const seconds = (entry: ReplayHookLogEntry) => Math.round((entry.at - started) / 1000);
  const firstRef = sideRef(context.pair.first) ?? "first";
  const detections = new Map<string, { ref: string; file: string; first: boolean; firstAtS: number; resolvedAtS: number | null; pushedAtS: number | null }>();
  let open = new Set<string>();
  for (const entry of hookLog) {
    if (entry.conflicts === undefined) continue;
    const now = new Set(entry.conflicts.map(conflictKey));
    for (const { ref, file } of entry.conflicts) {
      const known = detections.get(conflictKey({ ref, file }));
      if (known === undefined) detections.set(conflictKey({ ref, file }), { ref, file, first: ref === firstRef, firstAtS: seconds(entry), resolvedAtS: null, pushedAtS: null });
      else known.resolvedAtS = null;
    }
    for (const key of open) {
      if (!now.has(key)) detections.get(key)!.resolvedAtS = seconds(entry);
    }
    for (const item of entry.pushed ?? []) detections.get(conflictKey(item))!.pushedAtS ??= seconds(entry);
    open = now;
  }
  const commands = (events as Array<Record<string, unknown>>).filter(event => {
    const item = event.item as Record<string, unknown> | undefined;
    return event.type === "item.completed" && item?.type === "command_execution" && CHECK_COMMAND.test(String(item.command));
  });
  return {
    refs: context.refs.length,
    detections: [...detections.values()],
    checkedAtS: hookLog.filter(entry => entry.checked === true && entry.event === "PostToolUse").map(seconds),
    checks: commands.length,
    openAtEnd: open.size,
  };
}

/** The workspace at `repo`: the base snapshot as its only commit, from the source's objects through a throwaway index. */
function checkoutBase(context: PairContext, folder: string, repo: string): void {
  git(folder, ["init", "-q", repo]);
  const index = { GIT_INDEX_FILE: join(folder, "base.index") };
  git(context.source, ["read-tree", context.spec.base_sha], index);
  git(context.source, [`--work-tree=${repo}`, "checkout-index", "--all", "--force"], index);
  git(repo, ["add", "--all", "--force"]);
  git(repo, ["commit", "-q", "--no-verify", "-m", "base"]);
}

function runSetup(command: string, repo: string): NonNullable<ReplayRun["setup"]> {
  const started = performance.now();
  const result = spawnSync("sh", ["-c", command], { cwd: repo, env: process.env, encoding: "utf8", timeout: SETUP_TIMEOUT_MS, maxBuffer: 64 * 1024 * 1024 });
  const output = `${result.stdout ?? ""}${result.stderr ?? ""}`.trim().split("\n");
  git(repo, ["add", "--all"]);
  git(repo, ["commit", "-q", "--no-verify", "--allow-empty", "-m", "setup"]);
  return { exitCode: result.status, ms: Math.round(performance.now() - started), tail: output.slice(-20).join("\n") };
}

function parseEvents(stdout: string): unknown[] {
  return stdout.split("\n").filter(line => line.startsWith("{")).flatMap(line => {
    try {
      return [JSON.parse(line) as unknown];
    } catch {
      return [];
    }
  });
}

async function runJob(job: Job, options: Options): Promise<ReplayRun> {
  const { context, arm, trial } = job;
  const { pair, spec } = context;
  const folder = await realpath(await mkdtemp(join(tmpdir(), "rerail-ma-replay-")));
  const repo = join(folder, "repo");
  const codexHome = join(folder, "codex-home");
  const configPath = join(folder, "hook-config.json");
  const logPath = join(folder, "hook.log");
  const empty = { timedOut: false, exitCode: null, wallMs: 0, usage: null, finalMessage: null, noteLine: false, leaked: false, leaks: [], setup: null, diff: "", files: [], merge: null, mergeError: null, touches: { firstHalf: [], firstFiles: [] }, delivered: [], injections: 0, landed: null, hookErrors: [], hookLog: [], git: null, events: [], grade: null };
  const task = options.tamper ? TAMPER_TASK : spec.task;
  // What no replay Codex call may read: the source repository, its objects, the dataset, the clones, the ledger cache.
  const denied = [context.source, dirname(context.objects), options.dataset, options.repos, LEDGER_CACHE];
  try {
    if (arm === "align" && context.units === null) throw new Error(`no ledger: ${context.ledgerError ?? "not built"}`);
    checkoutBase(context, folder, repo);
    const setup = spec.setup === null ? null : runSetup(spec.setup, repo);
    const diffBase = git(repo, ["rev-parse", "HEAD"]).trim();
    const conflictsFile = arm === "git-conflict-pull" ? join(repo, ".git", "rerail-conflicts.md") : null;
    if (conflictsFile !== null) {
      writeFileSync(conflictsFile, "No in-flight conflicts: none of your changes overlap another branch's.\n");
      git(repo, ["config", "alias.conflicts", "!cat .git/rerail-conflicts.md"]);
    }
    const config: ReplayHookConfig = {
      arm,
      units: arm === "align" ? context.units! : [],
      alignModule: arm === "align" ? options.alignModule : null,
      alignOptions: { cacheDir: join(folder, "align-cache") },
      prompt: task,
      root: repo,
      log: logPath,
      state: join(folder, "hook-state.json"),
      refs: GIT_ARMS.includes(arm) ? context.refs : [],
      workspaceBase: diffBase,
      conflictsFile,
    };
    writeFileSync(configPath, `${JSON.stringify(config, null, 2)}\n`);
    writeFileSync(logPath, "");
    createCodexHome(codexHome, configPath, HOOK);
    writeFileSync(join(codexHome, "config.toml"), sandboxConfig(denied));
    const started = Date.now();
    const outcome = await runCodex(repo, codexHome, configPath, task, { ...options, permissions: PROFILE, binary: codexBinary() }, join(folder, "rerail-home"));
    const events = readEvents(outcome.stdout);
    const hookLog = readFileSync(logPath, "utf8").split("\n").filter(line => line.trim() !== "").map(line => JSON.parse(line) as ReplayHookLogEntry);
    const injected = hookLog.flatMap(entry => (entry.text === undefined ? [] : [entry.text]));
    const transcript = transcriptText(codexHome);
    const leaks = findLeaks(parseEvents(outcome.stdout), transcript, folder, repo);
    git(repo, ["add", "--all"]);
    const diff = git(repo, ["diff", "--cached", "--no-color", diffBase]);
    const patch = git(repo, ["diff", "--cached", "--binary", "--no-color", diffBase]);
    const files = git(repo, ["diff", "--cached", "--numstat", diffBase]).split("\n").filter(line => line !== "").map(line => {
      const [added, removed, ...path] = line.split("\t");
      return { path: path.join("\t"), added: Number(added) || 0, removed: Number(removed) || 0 };
    });
    let merge: ReplayRun["merge"] = null;
    let mergeError: string | null = null;
    try {
      const scratch = scratchRepository(folder, [context.objects, join(repo, ".git", "objects")]);
      const index = { GIT_INDEX_FILE: join(folder, "result.index") };
      git(scratch, ["read-tree", spec.base_sha], index);
      if (patch.trim() !== "") git(scratch, ["apply", "--cached", "--binary", "--3way"], index, patch);
      const tree = git(scratch, ["write-tree"], index).trim();
      const result = git(scratch, ["commit-tree", tree, "-p", spec.base_sha, "-m", "run"]).trim();
      merge = { head: mergeCheck(scratch, result, pair.first.head_sha), visible: mergeCheck(scratch, result, spec.first_visible) };
    } catch (error) {
      mergeError = error instanceof Error ? error.message.split("\n").slice(0, 3).join(" ") : String(error);
    }
    const changed = new Set(files.map(file => file.path));
    const grade = await gradeRun(context, diff, folder, options, denied);
    const failure = outcome.timedOut
      ? `timed out after ${options.timeoutMin} minutes`
      : events.error ?? (outcome.exitCode === 0 ? null : `codex exited ${outcome.exitCode}: ${clip(outcome.stderr.trim().split("\n").at(-1) ?? "", 200)}`);
    return {
      pair: pair.id,
      arm,
      trial,
      error: failure,
      timedOut: outcome.timedOut,
      exitCode: outcome.exitCode,
      wallMs: outcome.wallMs,
      usage: events.usage,
      finalMessage: events.finalMessage,
      noteLine: /^\s*Note:.*decided differently/mu.test(events.finalMessage ?? ""),
      leaked: leaks.length > 0,
      leaks,
      setup,
      diff,
      files,
      merge,
      mergeError,
      touches: { firstHalf: (pair.conflict?.first_half.files ?? []).filter(path => changed.has(path)), firstFiles: context.firstFiles.filter(path => changed.has(path)) },
      delivered: hookLog.flatMap(entry => entry.delivered.map(item => {
        const unit = context.units?.find(candidate => candidate.id === item.unitId);
        return { ...item, fromFirst: unit !== undefined && fromFirst(unit, pair), atS: Math.round((entry.at - started) / 1000) };
      })),
      injections: injected.length,
      landed: transcript === null ? null : injected.filter(text => transcript.includes(JSON.stringify(text).slice(1, -1))).length,
      hookErrors: hookLog.flatMap(entry => (entry.error === undefined ? [] : [entry.error])),
      hookLog,
      git: GIT_ARMS.includes(arm) ? gitOutcome(context, hookLog, parseEvents(outcome.stdout), started) : null,
      events: parseEvents(outcome.stdout),
      grade,
      folder: options.keep ? folder : null,
    };
  } catch (error) {
    return { pair: pair.id, arm, trial, ...empty, error: error instanceof Error ? error.message : String(error), folder: options.keep ? folder : null };
  } finally {
    if (options.keep) console.error(`kept ${folder}`);
    else await rm(folder, { recursive: true, force: true });
  }
}

// ---------------------------------------------------------------------------------------------------------- report

const count = (runs: readonly ReplayRun[], test: (run: ReplayRun) => boolean): string => `${runs.filter(test).length}/${runs.length}`;

function printLedgers(contexts: readonly PairContext[]): void {
  for (const context of contexts) {
    const reference = context.reference === null ? "second's head absent" : `${context.reference.files.length} files, ${context.reference.hunks} hunks`;
    console.log(`\n${context.pair.id} (${context.pair.repo}, ${context.pair.kind}${context.pair.conflict === undefined ? "" : ` ${context.pair.conflict.type}`}): ledger at ${context.ledgerTime}, real second×first merge: ${reference}`);
    if (context.ledgerError !== null) console.log(`  ledger failed: ${context.ledgerError}`);
    if (context.refs.length > 0) console.log(`  ${context.refs.length} visible refs for the git-conflict arms in ${fixed(context.refsMs / 1000, 1)}s: ${context.refs.map(side => `${side.label} (${Object.keys(side.files).length} files)`).join(", ")}`);
    if (context.units === null) continue;
    const first = context.units.filter(unit => fromFirst(unit, context.pair));
    const byRef = new Map<string, number>();
    for (const unit of context.units) {
      const ref = unit.branch ?? (unit.pr === undefined ? "?" : `#${unit.pr}`);
      byRef.set(ref, (byRef.get(ref) ?? 0) + 1);
    }
    console.log(`  ${context.units.length} units in ${fixed((context.ledgerMs ?? 0) / 1000, 1)}s: ${[...byRef].map(([ref, total]) => `${ref} ${total}`).join(", ")}`);
    console.log(`  first's ${first.length}:`);
    for (const unit of first) console.log(`  ${unit.conflict === true ? "!" : "-"} ${unit.id} [${unit.kind}] ${clip(unit.text, 110)} (${clip(unit.anchors.files.slice(0, 3).join(", "), 60)})`);
  }
}

function printRuns(runs: readonly ReplayRun[]): void {
  const rows = runs.map(run => [
    run.pair,
    run.arm,
    String(run.trial),
    fixed(run.wallMs / 60_000, 1),
    `${Math.round((run.usage?.input ?? 0) / 1000)}k/${Math.round((run.usage?.output ?? 0) / 1000)}k`,
    String(run.files.length),
    String(run.files.reduce((sum, file) => sum + file.added, 0)),
    run.delivered.length === 0 ? "-" : `${run.delivered.length} (${run.delivered.filter(unit => unit.fromFirst).length} first, ${run.delivered.filter(unit => unit.conflict).length}!)`,
    run.injections === 0 ? "-" : `${run.landed ?? "?"}/${run.injections}`,
    run.merge === null ? "err" : `${run.merge.head.files.length}/${run.merge.head.hunks}`,
    run.merge === null ? "err" : `${run.merge.visible.files.length}/${run.merge.visible.hunks}`,
    `${run.touches.firstHalf.length}/${run.touches.firstFiles.length}`,
    run.grade?.verdict == null ? `(${clip(run.grade?.error ?? "", 18)})` : `${run.grade.verdict.conflicts_with_first}${run.grade.by === "empty-diff" ? " (empty diff)" : ""}`,
    run.grade?.verdict?.done ?? "",
    run.noteLine ? "yes" : "",
    run.leaked ? `LEAK (${run.leaks.length})` : "",
    run.git === null ? "-" : `${run.git.detections.length} (${run.git.detections.filter(item => item.first).length} first)/${run.git.detections.filter(item => item.resolvedAtS !== null).length}/${run.git.openAtEnd}`,
    run.git === null ? "-" : run.git.checks === 0 ? "no" : `${run.git.checks}× from ${run.git.checkedAtS[0] ?? "?"}s`,
    clip(run.error ?? run.mergeError, 40),
  ]);
  console.log(`\n${table(["pair", "arm", "trial", "min", "tok in/out", "files", "+lines", "units", "landed", "merge head f/h", "merge vis f/h", "touch half/all", "grader conflict", "done", "note", "leaked", "git det/res/open", "checked", "error"], rows)}`);
  for (const run of runs) {
    if (run.git === null || run.git.detections.length === 0) continue;
    console.log(`\n${run.pair} ${run.arm} #${run.trial}: ${run.git.detections.length} conflicts identified against ${run.git.refs} visible refs`);
    for (const item of run.git.detections) {
      const resolved = item.resolvedAtS === null ? "still open" : `resolved at ${item.resolvedAtS}s (after ${item.resolvedAtS - item.firstAtS}s)`;
      console.log(`  - ${shortRef(item.ref)}${item.first ? " (first)" : ""} ${item.file}: from ${item.firstAtS}s${item.pushedAtS === null ? "" : `, pushed at ${item.pushedAtS}s`}, ${resolved}`);
    }
  }
  for (const run of runs.filter(candidate => candidate.leaked)) {
    console.log(`\n${run.pair} ${run.arm} #${run.trial} LEAKED, left out of every aggregate:`);
    for (const leak of run.leaks) console.log(`  - ${leak.reason} [${leak.source}]: ${clip(leak.text, 200)}`);
  }
  for (const run of runs) {
    if (run.grade?.verdict === undefined || run.grade.verdict === null) continue;
    console.log(`\n${run.pair} ${run.arm} #${run.trial}: grader ${run.grade.verdict.conflicts_with_first}, done ${run.grade.verdict.done}${run.grade.strippedLines > 0 ? ` (${run.grade.strippedLines} lines stripped)` : ""}`);
    for (const reason of run.grade.verdict.reasons) console.log(`  - ${clip(reason, 220)}`);
  }
}

/** One aggregate row: leaked runs are counted, then left out of everything else. */
function aggregateRow(label: readonly string[], all: readonly ReplayRun[]): string[] {
  const runs = all.filter(run => !run.leaked);
  const graded = runs.filter(run => run.grade?.verdict != null);
  return [
    ...label,
    String(runs.length),
    String(all.length - runs.length),
    count(runs, run => run.error !== null),
    count(graded, run => run.grade!.verdict!.conflicts_with_first === "yes"),
    count(graded, run => run.grade!.verdict!.conflicts_with_first === "unclear"),
    count(graded, run => run.grade!.verdict!.done === "yes"),
    count(runs, run => run.merge !== null && !run.merge.head.clean),
    fixed(mean(runs.flatMap(run => (run.merge === null ? [] : [run.merge.head.hunks]))), 1),
    count(runs, run => run.touches.firstHalf.length > 0),
    fixed(mean(runs.map(run => run.delivered.length)), 1),
    fixed(mean(runs.map(run => run.delivered.filter(unit => unit.fromFirst).length)), 1),
    count(runs, run => run.noteLine),
    runs.some(run => run.git !== null) ? fixed(mean(runs.map(run => run.git?.detections.length ?? 0)), 1) : "-",
    runs.some(run => run.git !== null) ? fixed(mean(runs.map(run => run.git?.openAtEnd ?? 0)), 1) : "-",
    runs.some(run => run.git !== null) ? count(runs, run => (run.git?.checks ?? 0) > 0) : "-",
    fixed(median(runs.map(run => run.wallMs / 60_000)), 1),
    runs.length === 0 ? "n/a" : `${Math.round(mean(runs.map(run => (run.usage?.input ?? 0) + (run.usage?.output ?? 0))) / 1000)}k`,
  ];
}

const AGGREGATE_HEADERS = ["runs", "leaked", "errors", "grader yes", "unclear", "done yes", "merge conflicted", "hunks", "touch half", "units", "first units", "note", "git det", "git open", "checked", "median min", "mean tokens"];

function printAggregates(runs: readonly ReplayRun[], arms: readonly ReplayArm[]): void {
  console.log(`\nPer arm\n${table(["arm", ...AGGREGATE_HEADERS], arms.map(arm => aggregateRow([arm], runs.filter(run => run.arm === arm))))}`);
  const pairs = [...new Set(runs.map(run => run.pair))];
  const rows = pairs.flatMap(pair => arms.map(arm => aggregateRow([pair, arm], runs.filter(run => run.pair === pair && run.arm === arm))));
  console.log(`\nPer pair × arm\n${table(["pair", "arm", ...AGGREGATE_HEADERS], rows)}`);
}

// ---------------------------------------------------------------------------------------------------------- detect-only

/** One (ref, file) the identifier found while replaying second's real commits: when, at which commit, and whether the ref is first's. */
interface Detection {
  readonly ref: string;
  readonly file: string;
  readonly first: boolean;
  readonly at: string;
  readonly sha: string;
}

interface DetectResult {
  readonly pair: Pair;
  readonly commits: number;
  /** The first identification of each (ref, file), in replay order. */
  readonly detections: readonly Detection[];
  readonly ms: number;
  readonly error: string | null;
}

/** Git answers `--detect-only` asks again and again across pairs that share a side: keyed by repository and arguments. */
const detectCache = new Map<string, unknown>();

function cached<T>(key: string, compute: () => T): T {
  if (!detectCache.has(key)) detectCache.set(key, compute());
  return detectCache.get(key) as T;
}

/** Which of `shas` the clone holds as commits: one `git cat-file --batch-check` for all. */
function presentCommits(repo: string, shas: readonly string[]): Set<string> {
  if (shas.length === 0) return new Set();
  const output = git(repo, ["cat-file", "--batch-check=%(objecttype)"], {}, `${shas.map(sha => `${sha}^{commit}`).join("\n")}\n`).split("\n");
  return new Set(shas.filter((_, index) => output[index] === "commit"));
}

/** Each commit's parents and the files it changed against its only parent (none for a merge): two batched git calls. */
function commitShapes(repo: string, shas: readonly string[]): Map<string, { parents: string[]; files: string[] }> {
  const shapes = new Map<string, { parents: string[]; files: string[] }>();
  if (shas.length === 0) return shapes;
  for (const line of git(repo, ["rev-list", "--no-walk=unsorted", "--parents", "--stdin"], {}, `${shas.join("\n")}\n`).split("\n")) {
    const [sha, ...parents] = line.split(" ");
    if (sha !== undefined && sha !== "") shapes.set(sha, { parents, files: [] });
  }
  let current: string[] | null = null;
  for (const line of git(repo, ["diff-tree", "--stdin", "-r", "--root", "--name-only", "--no-renames"], {}, `${shas.join("\n")}\n`).split("\n")) {
    const shape = /^[0-9a-f]{40}$/u.test(line) ? shapes.get(line) : undefined;
    if (shape !== undefined) current = shape.files;
    else if (line !== "" && current !== null) current.push(line);
  }
  return shapes;
}

/** Every commit `sha` reaches. */
function ancestors(repo: string, sha: string): Set<string> {
  return cached(`${repo}\0ancestors\0${sha}`, () => new Set((gitRead(repo, ["rev-list", sha]) ?? "").split("\n").filter(line => line !== "")));
}

/**
 * Second's real commits through the `git-conflict` identifier, in their labelled order at their authoring times: the
 * files second's own (non-merge) commits changed so far are the agent's, and each visible ref (first's last labelled
 * commit visible then, each `concurrent` ref's head then) is compared from the commit's merge base with it, so both
 * sides' hunks are in the same base's lines. Each (ref, file) is recorded at its first identification. A ref is
 * compared again only on the files the commit changed, unless the ref or the merge base moved.
 */
function detectPair(pair: Pair, source: string): DetectResult {
  const started = performance.now();
  const firstRef = sideRef(pair.first) ?? "first";
  const present = presentCommits(source, [...new Set([...pair.first.commits, ...pair.second.commits].map(commit => commit.sha))]);
  const firstCommits = pair.first.commits.filter(commit => present.has(commit.sha));
  const commits = pair.second.commits.filter(commit => present.has(commit.sha));
  const shapes = commitShapes(source, commits.map(commit => commit.sha));
  const touched = new Set<string>();
  const found = new Map<string, Detection>();
  const last = new Map<string, { sha: string; base: string }>();
  for (const commit of commits) {
    const shape = shapes.get(commit.sha) ?? { parents: [], files: [] };
    for (const path of shape.files) touched.add(path);
    const own = new Set(shape.files);
    const at = commit.authored_at === "" ? commit.visible_at : commit.authored_at;
    const firstSha = firstCommits.filter(item => Date.parse(item.visible_at) <= Date.parse(at)).at(-1)?.sha ?? null;
    for (const { ref, sha } of visibleShas(source, pair, at, firstSha)) {
      // A non-merge commit's merge base with `sha` is itself when `sha` reaches it, else its parent's.
      const parentBase = shape.parents.length === 1 ? detectCache.get(`${source}\0mb\0${shape.parents[0]}\0${sha}`) as string | undefined : undefined;
      const base = cached(`${source}\0mb\0${commit.sha}\0${sha}`, () => (ancestors(source, sha).has(commit.sha) ? commit.sha : parentBase ?? gitRead(source, ["merge-base", commit.sha, sha])?.trim() ?? ""));
      if (base === "") continue;
      const previous = last.get(ref);
      last.set(ref, { sha, base });
      const moved = previous === undefined || previous.sha !== sha || previous.base !== base;
      const theirFiles = cached(`${source}\0files\0${base}\0${sha}`, () => refFiles(source, base, sha));
      const paths = theirFiles.filter(path => touched.has(path) && (moved || own.has(path)) && !found.has(conflictKey({ ref, file: path })));
      if (paths.length === 0) continue;
      const missing = paths.filter(path => !detectCache.has(`${source}\0hunks\0${base}\0${sha}\0${path}`));
      const read = missing.length === 0 ? {} : refHunks(source, base, sha, missing);
      const theirs: Record<string, readonly RefHunk[]> = {};
      for (const path of paths) theirs[path] = cached(`${source}\0hunks\0${base}\0${sha}\0${path}`, () => read[path] ?? []);
      const rangesMissing = paths.filter(path => !detectCache.has(`${source}\0ours\0${base}\0${commit.sha}\0${path}`));
      const ours = rangesMissing.length === 0 ? new Map<string, LineRange[]>() : commitRanges(source, base, commit.sha, rangesMissing);
      const agent = new Map(paths.map(path => [path, cached(`${source}\0ours\0${base}\0${commit.sha}\0${path}`, () => ours.get(path) ?? [])]));
      for (const conflict of identify(agent, [{ ref, files: theirs }])) {
        found.set(conflictKey(conflict), { ref, file: conflict.file, first: ref === firstRef, at, sha: commit.sha });
      }
    }
  }
  return { pair, commits: commits.length, detections: [...found.values()], ms: Math.round(performance.now() - started), error: null };
}

function hoursBetween(from: string, to: string): number {
  return (Date.parse(to) - Date.parse(from)) / 3_600_000;
}

/** The earliest of `detections`; null when there is none. */
function earliestOf(detections: readonly Detection[]): Detection | null {
  return detections.reduce<Detection | null>((best, item) => (best === null || Date.parse(item.at) < Date.parse(best.at) ? item : best), null);
}

/** A conflict pair's labelled files: both halves'. A detection on one of them is on the conflict the label names. */
function labelledFiles(pair: Pair): Set<string> {
  return new Set([...(pair.conflict?.first_half.files ?? []), ...(pair.conflict?.second_half.files ?? [])]);
}

function printDetections(results: readonly DetectResult[]): void {
  const rows = results.map(({ pair, commits, detections, ms, error }) => {
    const againstFirst = detections.filter(item => item.first);
    const earliest = earliestOf(againstFirst);
    const labelled = labelledFiles(pair);
    const onLabel = earliestOf(againstFirst.filter(item => labelled.has(item.file)));
    const noticed = typeof pair.conflict?.noticed_at === "string" ? pair.conflict.noticed_at : null;
    const others = detections.filter(item => !item.first);
    return [
      pair.id,
      pair.kind,
      String(commits),
      noticed ?? "-",
      earliest?.at ?? "-",
      earliest === null || noticed === null ? "-" : fixed(hoursBetween(earliest.at, noticed), 1),
      clip(earliest?.file ?? "-", 40),
      String(againstFirst.length),
      onLabel === null || noticed === null ? "-" : fixed(hoursBetween(onLabel.at, noticed), 1),
      clip(onLabel?.file ?? "-", 40),
      `${others.length} (${new Set(others.map(item => item.ref)).size} refs)`,
      fixed(ms / 1000, 1),
      clip(error, 40),
    ];
  });
  console.log(`\n${table(["pair", "kind", "commits", "noticed_at", "first identified vs first", "lead h", "file", "files vs first", "labelled lead h", "labelled file", "vs other refs", "s", "error"], rows)}`);
  const conflicts = results.filter(result => result.pair.kind === "conflict" && result.error === null);
  const negatives = results.filter(result => result.pair.kind === "negative" && result.error === null);
  const leadsOf = (labelledOnly: boolean) => conflicts.flatMap(({ pair, detections }) => {
    const noticed = pair.conflict?.noticed_at;
    const labelled = labelledFiles(pair);
    const earliest = earliestOf(detections.filter(item => item.first && (!labelledOnly || labelled.has(item.file))));
    return typeof noticed !== "string" || earliest === null ? [] : [hoursBetween(earliest.at, noticed)];
  });
  for (const [label, leads] of [["any file", leadsOf(false)], ["a labelled file", leadsOf(true)]] as const) {
    console.log(`conflict pairs identified against first on ${label}: ${leads.length}/${conflicts.length}; before noticed_at: ${leads.filter(lead => lead > 0).length}/${conflicts.length}; median lead ${leads.length === 0 ? "n/a" : `${fixed(median(leads), 1)} h`}`);
  }
  console.log(`negatives identified against first (false detections): ${negatives.filter(result => result.detections.some(item => item.first)).length}/${negatives.length}, ${negatives.reduce((sum, result) => sum + result.detections.filter(item => item.first).length, 0)} (ref, file) detections`);
  console.log(`detections against other refs: ${fixed(mean(results.map(result => result.detections.filter(item => !item.first).length)), 1)} per pair`);
}

function detectOnly(options: Options): void {
  const pairs = loadPairs(options.dataset).filter(pair => options.pairs === null || options.pairs.includes(pair.id));
  if (pairs.length === 0) throw new Error(`No pair selected in ${options.dataset}`);
  const results: DetectResult[] = [];
  for (const pair of pairs) {
    const source = options.repo ?? join(options.repos, pair.repo.replace("/", "__"));
    let result: DetectResult;
    try {
      if (!existsSync(source)) throw new Error(`no repository at ${source}`);
      result = detectPair(pair, source);
    } catch (error) {
      result = { pair, commits: 0, detections: [], ms: 0, error: error instanceof Error ? error.message : String(error) };
    }
    results.push(result);
    console.error(`${pair.id}: ${result.commits} commits, ${result.detections.length} detections (${result.detections.filter(item => item.first).length} vs first) in ${fixed(result.ms / 1000, 1)}s${result.error === null ? "" : `, ${result.error}`}`);
  }
  printDetections(results);
  if (options.json !== undefined) {
    writeFileSync(options.json, `${JSON.stringify({ format: "rerail.merge-assist-replay-detect/v1", options, results: results.map(({ pair, ...rest }) => ({ id: pair.id, kind: pair.kind, noticedAt: pair.conflict?.noticed_at ?? null, ...rest })) }, null, 2)}\n`);
    console.error(`wrote ${options.json}`);
  }
}

// ---------------------------------------------------------------------------------------------------------- main

async function main(): Promise<void> {
  const options = parseOptions(process.argv.slice(2));
  if (options.detectOnly) {
    detectOnly(options);
    return;
  }
  const selected = loadDataset(options);
  if (selected.length === 0) throw new Error(`No pair in ${options.dataset} has a replay spec`);
  const align = options.arms.includes("align") ? await loadAlignModule(options.alignModule) : null;
  const contexts: PairContext[] = [];
  for (const { pair, spec } of selected) contexts.push(await preparePair(pair, spec, options, align));
  printLedgers(contexts);
  const jobs = contexts.flatMap(context => Array.from({ length: options.trials }, (_, trial) => options.arms.map(arm => ({ context, arm, trial: trial + 1 }))).flat());
  console.error(`\n${jobs.length} runs (${contexts.length} pairs × ${options.arms.join(",")} × ${options.trials} trials), ${options.concurrency} at a time, ${options.timeoutMin} min each`);
  const runs = await pool(jobs, options.concurrency, job => runJob(job, options), (done, run) => {
    console.error(`[${done}/${jobs.length}] ${run.pair} ${run.arm} #${run.trial}: ${fixed(run.wallMs / 60_000, 1)} min, ${run.files.length} files, ${run.delivered.length} units, grader ${run.grade?.verdict?.conflicts_with_first ?? run.grade?.error ?? "-"}${run.error === null ? "" : `, ${clip(run.error, 80)}`}`);
  });
  printRuns(runs);
  printAggregates(runs, options.arms);
  if (options.tamper) {
    const flagged = runs.filter(run => run.leaked).length;
    console.log(`\ntamper: ${flagged}/${runs.length} runs flagged as leaked`);
    if (flagged < runs.length) process.exitCode = 1;
  }
  if (options.json !== undefined) {
    const report = {
      format: "rerail.merge-assist-replay/v1",
      options,
      pairs: contexts.map(({ pair, spec, source, ledgerTime: time, units, ledgerMs, ledgerError, firstFiles, reference, refs, refsMs }) => ({ id: pair.id, pair, spec, source, ledgerTime: time, units, ledgerMs, ledgerError, firstFiles, reference, refs: refs.map(({ files, ...side }) => ({ ...side, files: Object.keys(files) })), refsMs })),
      runs,
    };
    writeFileSync(options.json, `${JSON.stringify(report, null, 2)}\n`);
    console.error(`wrote ${options.json}`);
  }
}

if (process.argv[1] !== undefined && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) await main();
