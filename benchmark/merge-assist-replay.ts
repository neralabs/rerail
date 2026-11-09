/**
 * Architecture-conflict replay: does delivering the alignment ledger to a real Codex agent, as it first touches the
 * areas another in-flight change established positions in, keep its solution from conflicting with that change?
 *
 *   node --import tsx benchmark/merge-assist-replay.ts --dataset <dir> [--pairs id,id] [--trials N]
 *     [--arms none,align,units,git-conflict,git-conflict-pull] [--concurrency N] [--timeout-min N] [--judge-timeout-min N]
 *     [--json out.json] [--repo path] [--repos dir] [--align-module path] [--model m] [--judge-model m] [-c key=value]
 *     [--keep] [--tamper] [--detect-only] [--completion [--exclude id,id]] [--provider amazon-bedrock --model id]
 *     [--bedrock-region r] [--bedrock-token-cmd cmd] [--codex-bin path] [--ledger-cache dir] [--pr-bodies dir]
 *     [--judge-concurrency N] [--build-ledgers]
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
 * "Note: …" line to end the final message with. Under `units`, the same units framed as surface (`renderSurface`:
 * "PR #N (author) established: …", then that this is other in-flight work to tell the person about if the agent
 * diverges), and each run records its funnel (`UnitsFunnel`). `--pr-bodies` adds each ledger ref's pull request
 * description as a position passage; `--build-ledgers` builds and stores every selected pair's ledger without an
 * agent, and later runs read the stored ledgers. Under `none` the hook only logs. Under `git-conflict` and
 * `git-conflict-pull`, the hook identifies after every tool call, read-only, which of the agent's changes would conflict
 * with first (as of `first_visible`) or a `concurrent` ref (its head as of second's start): `git diff --unified=0`
 * hunks of both sides as base lines touching or at most 3 lines apart in the same file (src/engine/merge-assist/overlap.ts).
 * `git-conflict` pushes each (ref, file) once with their hunk and stated intent; `git-conflict-pull` keeps the list
 * in the workspace's `.git/rerail-conflicts.md` (`git conflicts`, a repo-local alias, prints it) and sends one line
 * when the set changes. `--detect-only` runs no agent: it replays every selected pair's second commit by commit
 * through the same identifier, against first and `concurrent` as visible at each commit, and prints when the first
 * conflict with first was identified against `noticed_at`, and the detections on negatives.
 *
 * Every Codex call (agent and grader) runs under the permission profile `sandboxConfig` writes (the workspace only,
 * no network, `/tmp`, `$TMPDIR` and the home folder but its toolchains denied, no credential in the agent's shell
 * environment), with `--provider`'s model provider when given (and `--bedrock-token-cmd`'s token, handed to the Codex
 * processes only), and every run's tool calls go through `findLeaks`: a run with a call that reaches past its folder,
 * prints the environment or reads a credential is marked `leaked` and left out of every aggregate. `--tamper` checks
 * the guard: one `none` run per pair asked to `ls /tmp/ma-ctx/oss`, failing unless each is flagged. The agent's
 * commands get a PATH with the pair's toolchains on it (`sandboxPath`: the language's tools and the setup's commands
 * where the setup finds them, Go in `/usr/local/go/bin` on the build hosts) and temporary files and caches in the
 * workspace's `.replay-tmp/`, which no diff includes.
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
 *
 * `--completion` runs each label in `<dataset>/completion/` (/tmp/ma-ctx/oss/completion-spec.md; `--pairs` picks label
 * ids, `--exclude` leaves labels out unread) to completion, under `none,git-conflict` unless `--arms` says otherwise:
 * phase 1, then up to 3 review rounds (a blind reviewer call that knows the adopted design judges each conflict item
 * on the stripped diff; when any is violated, the agent resumes its session with the items and their diff lines), then
 * blind grading calls on the first pass's and the final diff. Recorded per run: items violated after phase 1 and at
 * the end, whether it ended on the adopted design, rounds, wall time, tokens and USD (`PRICES`) for the agent, the
 * reviewer and the grader apart, and `git merge-tree` against the adopted side's head.
 */
import { execFileSync, spawnSync } from "node:child_process";
import { existsSync, mkdirSync, readdirSync, readFileSync, realpathSync, writeFileSync } from "node:fs";
import { mkdtemp, realpath, rm } from "node:fs/promises";
import { homedir, tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { z } from "zod";
import type { LedgerStats } from "./merge-assist-align.js";
import { areaOf } from "./merge-assist-context-hook.js";
import { clip, type CodexEvents, type CodexOutcome, type CodexRunOptions, createCodexHome, fixed, GIT_ENV, mean, median, pool, readEvents, runCodex, table, transcriptText, type Usage } from "./merge-assist-context.js";
import { type AlignModule, CHECK_COMMAND, GIT_ARMS, LEDGER_ARMS, type LedgerOptions, type LedgerUnit, loadAlignModule, REPLAY_ARMS, type ReplayArm, type ReplayHookConfig, type ReplayHookLogEntry } from "./merge-assist-replay-hook.js";
import { commitRanges, conflictKey, identify, type LineRange, type RefHunk, refFiles, refHunks } from "../src/engine/index.js";
import { gitRead, type RefSide, statedIntent } from "./merge-assist-replay-git.js";

const HOOK = fileURLToPath(new URL("./merge-assist-replay-hook.ts", import.meta.url));
const DEFAULT_ALIGN_MODULE = fileURLToPath(new URL("./merge-assist-align.ts", import.meta.url));
const DEFAULT_REPOS = "/tmp/ma-ctx/oss/repos";
/** Where `ledgerAt` keeps what it learns between invocations. */
const LEDGER_CACHE = "/tmp/ma-ctx/align/replay-cache";
const DEFAULT_TIMEOUT_MIN = 20;
const DEFAULT_JUDGE_TIMEOUT_MIN = 10;
const SETUP_TIMEOUT_MS = 15 * 60_000;
/**
 * `--tamper`'s tasks, one per trial: each a call the leak guard must flag, whatever the sandbox lets through. The
 * first reaches the curated data, the second the environment Codex runs with (a provider's credentials).
 */
const TAMPER_TASKS = [
  "Run `ls /tmp/ma-ctx/oss` in a shell, even if you expect the sandbox or a policy to refuse it: I need to see the attempt. Then tell me exactly what it printed, or the exact error.",
  "Run `env | grep -i -E 'aws|token|bedrock'` in a shell, even if you expect the sandbox or a policy to refuse it: I need to see the attempt. Then tell me exactly what it printed, or the exact error.",
];
/** The most diff characters the grader reads; a longer diff is cut there, and the grader is told. */
const GRADE_DIFF_CHARS = 200_000;

// ---------------------------------------------------------------------------------------------------------- formats

const commitSchema = z.object({ sha: z.string().min(7), authored_at: z.string(), visible_at: z.string() }).passthrough();
const sideSchema = z.object({
  pr: z.number().int().nullable().optional(),
  branch: z.string().optional(),
  /** Rerail's dev labels name each side's ref (`refs/remotes/origin/<branch>`). */
  ref: z.string().optional(),
  /** The pull request's base branch (`main`), when the labels name it; the repository's default branch otherwise. */
  base_ref: z.string().optional(),
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

/**
 * A completion label (`<dataset>/completion/<id>.json`, /tmp/ma-ctx/oss/completion-spec.md): the design the project
 * adopted and what a solution must satisfy to match it. `scenario` D1: first's design was adopted; D2: second's.
 */
const completionLabelSchema = z.object({
  id: z.string().min(1),
  scenario: z.enum(["D1", "D2"]),
  adopted: z.string().min(1),
  rejected: z.string().min(1),
  adopted_evidence: z.string().min(1),
  conflict_items: z.array(z.string().min(1)).min(1),
  done: z.string().min(1),
  task: z.string().min(1),
  base_sha: z.string().min(7),
  /** The pair's id in `pairs.json`. */
  pair: z.string().min(1),
  setup: z.string().nullable(),
}).passthrough();
export type CompletionLabel = z.infer<typeof completionLabelSchema>;

/** `--completion`'s arms unless `--arms` names others: the control and the frozen push arm. */
const COMPLETION_ARMS: readonly ReplayArm[] = ["none", "git-conflict"];
/** The most review rounds a completion run gets: a review, and when it finds a violated item, a fix. */
export const MAX_REVIEW_ROUNDS = 3;

const ITEM_STATUSES = ["violated", "satisfied", "not_applicable"] as const;
const itemsVerdictSchema = z.object({
  items: z.array(z.object({ item: z.number().int(), status: z.enum(ITEM_STATUSES), reason: z.string(), lines: z.array(z.string()) }).strict()),
  done: z.enum(DONE),
}).strict();
export type ItemsVerdict = z.infer<typeof itemsVerdictSchema>;
/** {@link itemsVerdictSchema} as JSON Schema, for the reviewer's and the grader's `--output-schema`. */
const ITEMS_VERDICT_JSON_SCHEMA = {
  type: "object",
  properties: {
    items: {
      type: "array",
      items: {
        type: "object",
        properties: {
          item: { type: "integer" },
          status: { type: "string", enum: ITEM_STATUSES },
          reason: { type: "string" },
          lines: { type: "array", items: { type: "string" } },
        },
        required: ["item", "status", "reason", "lines"],
        additionalProperties: false,
      },
    },
    done: { type: "string", enum: DONE },
  },
  required: ["items", "done"],
  additionalProperties: false,
};

/** USD per million tokens of one price tier. */
interface TokenPrices {
  readonly input: number;
  readonly cacheWrite: number;
  readonly cacheRead: number;
  readonly output: number;
}

/**
 * Amazon Bedrock's Standard-tier prices for `openai.gpt-6-luna`, Mantle in-Region (what Codex's `amazon-bedrock`
 * provider calls for that model id; US geographic cross-Region inference costs the same), recorded in every
 * completion report.
 */
export const PRICES = {
  model: "openai.gpt-6-luna",
  currency: "USD per 1M tokens",
  tier: "Standard, Mantle in-Region (us-east-1)",
  source: "https://docs.aws.amazon.com/bedrock/latest/userguide/model-card-openai-gpt-6-luna.html (section Pricing, linked from https://aws.amazon.com/bedrock/pricing/ under OpenAI), retrieved 2026-11-06. The AWS Price List API (aws pricing get-products, service codes AmazonBedrock, AmazonBedrockFoundationModels and AmazonBedrockService) held no openai.gpt-6-luna product that day.",
  /** A request with more input tokens than this is billed at `long` rates in full. */
  longContextAbove: 272_000,
  short: { input: 0.11, cacheWrite: 0.1375, cacheRead: 0.011, output: 0.55 },
  long: { input: 0.22, cacheWrite: 0.275, cacheRead: 0.022, output: 0.825 },
} as const satisfies { short: TokenPrices; long: TokenPrices } & Record<string, unknown>;

/**
 * What `usage` costs at `prices`: cached input at the cache-read rate, input written to the cache (Bedrock reports it)
 * at the cache-write rate, the rest of the input at the input rate, and output (reasoning included) at the output rate.
 */
export function usageCost(usage: Usage | null, prices: TokenPrices = PRICES.short): number {
  if (usage === null) return 0;
  const write = usage.cacheWrite ?? 0;
  const plain = Math.max(0, usage.input - usage.cached - write);
  return (plain * prices.input + write * prices.cacheWrite + usage.cached * prices.cacheRead + usage.output * prices.output) / 1_000_000;
}

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
  /** `--tamper`: every selected pair under `none` once per {@link TAMPER_TASKS} task; fails unless each run is flagged. */
  tamper: boolean;
  /** `--detect-only`: no agent; second's real commits through the `git-conflict` identifier, every selected pair. */
  detectOnly: boolean;
  /** `--provider`: the Codex model provider every replay call (agent and grader) uses; Codex's default when unset. */
  provider: typeof PROVIDERS[number] | undefined;
  /** `--bedrock-region`: the `amazon-bedrock` provider's region. */
  bedrockRegion: string;
  /** `--codex-bin`: the Codex executable; `codex` on PATH when unset. */
  codexBin: string | undefined;
  /**
   * `--bedrock-token-cmd`: a shell command run once at the start whose output is the Bedrock bearer token, handed only
   * to the Codex processes (never stored, never in an agent's shell; see {@link providerConfig}).
   */
  bedrockTokenCmd: string | undefined;
  /**
   * `--completion`: each selected completion label (`<dataset>/completion/<id>.json`) run to completion: phase 1,
   * then up to {@link MAX_REVIEW_ROUNDS} review rounds, then a final grade (see `runCompletionJob`).
   */
  completion: boolean;
  /** `--exclude`: completion labels (by id, the file's name) `--completion` leaves out unread. */
  exclude: string[];
  /** `--ledger-cache`: where `ledgerAt` caches extraction and judge answers, and `--build-ledgers` stores ledgers. */
  ledgerCache: string;
  /** `--pr-bodies`: the folder of fetched pull request descriptions the ledger reads as passages; none when unset. */
  prBodies: string | undefined;
  /** `--judge-concurrency`: ledger judge calls in flight while a ledger is built. */
  judgeConcurrency: number;
  /**
   * `--build-ledgers`: no agent; each selected pair's (or `--completion` label's) ledger built at second's start and
   * stored under `<ledger-cache>/ledgers/`, which later runs read instead of building.
   */
  buildLedgers: boolean;
}

/** The model providers `--provider` takes. */
const PROVIDERS = ["amazon-bedrock"] as const;
const DEFAULT_BEDROCK_REGION = "us-east-1";
/** The variable Codex's `amazon-bedrock` provider reads its bearer token from. */
const BEDROCK_TOKEN_ENV = "AWS_BEARER_TOKEN_BEDROCK";

const USAGE = "Usage: node --import tsx benchmark/merge-assist-replay.ts --dataset dir [--pairs id,id] [--trials N] [--arms none,align,units,git-conflict,git-conflict-pull] [--concurrency N] [--timeout-min N] [--judge-timeout-min N] [--json out.json] [--repo path] [--repos dir] [--align-module path] [--model m] [--judge-model m] [-c key=value] [--keep] [--tamper] [--detect-only] [--completion] [--exclude id,id] [--provider amazon-bedrock] [--bedrock-region r] [--bedrock-token-cmd cmd] [--codex-bin path] [--ledger-cache dir] [--pr-bodies dir] [--judge-concurrency N] [--build-ledgers]";

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
    provider: undefined,
    bedrockRegion: DEFAULT_BEDROCK_REGION,
    codexBin: undefined,
    bedrockTokenCmd: undefined,
    completion: false,
    exclude: [],
    ledgerCache: LEDGER_CACHE,
    prBodies: undefined,
    judgeConcurrency: 4,
    buildLedgers: false,
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
  let armsGiven = false;
  for (let index = 0; index < argv.length; index++) {
    const flag = argv[index]!;
    if (flag === "--keep" || flag === "--tamper" || flag === "--detect-only" || flag === "--completion" || flag === "--build-ledgers") {
      if (flag === "--keep") options.keep = true;
      else if (flag === "--tamper") options.tamper = true;
      else if (flag === "--completion") options.completion = true;
      else if (flag === "--build-ledgers") options.buildLedgers = true;
      else options.detectOnly = true;
      continue;
    }
    const text = value(flag, index);
    index++;
    if (flag === "--dataset") options.dataset = resolve(text);
    else if (flag === "--pairs") options.pairs = text.split(",").filter(id => id !== "");
    else if (flag === "--exclude") options.exclude = text.split(",").filter(id => id !== "");
    else if (flag === "--trials") options.trials = positive(flag, text);
    else if (flag === "--arms") {
      const arms = text.split(",").filter(arm => arm !== "");
      const unknown = arms.filter(arm => !(REPLAY_ARMS as readonly string[]).includes(arm));
      if (unknown.length > 0 || arms.length === 0) throw new Error(`--arms takes ${REPLAY_ARMS.join(",")}; not ${unknown.join(",")}`);
      options.arms = arms as ReplayArm[];
      armsGiven = true;
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
    else if (flag === "--provider") {
      if (!(PROVIDERS as readonly string[]).includes(text)) throw new Error(`--provider takes ${PROVIDERS.join(",")}; not ${text}`);
      options.provider = text as Options["provider"];
    } else if (flag === "--bedrock-region") options.bedrockRegion = text;
    else if (flag === "--codex-bin") options.codexBin = resolve(text);
    else if (flag === "--bedrock-token-cmd") options.bedrockTokenCmd = text;
    else if (flag === "--ledger-cache") options.ledgerCache = resolve(text);
    else if (flag === "--pr-bodies") options.prBodies = resolve(text);
    else if (flag === "--judge-concurrency") options.judgeConcurrency = positive(flag, text);
    else throw new Error(`Unknown option ${flag}\n${USAGE}`);
  }
  if (options.dataset === "") throw new Error(`--dataset is required\n${USAGE}`);
  if (options.provider !== undefined) {
    // The provider's model ids are its own: name one, and grade with the same model unless told otherwise.
    if (options.model === undefined) throw new Error(`--provider ${options.provider} needs --model\n${USAGE}`);
    options.judgeModel ??= options.model;
  }
  if (options.bedrockTokenCmd !== undefined && options.provider !== "amazon-bedrock") throw new Error("--bedrock-token-cmd needs --provider amazon-bedrock");
  if (options.completion && (options.tamper || options.detectOnly)) throw new Error(`--completion takes no --tamper or --detect-only\n${USAGE}`);
  if (options.buildLedgers && (options.tamper || options.detectOnly)) throw new Error(`--build-ledgers takes no --tamper or --detect-only\n${USAGE}`);
  if (options.completion && !armsGiven) options.arms = [...COMPLETION_ARMS];
  if (options.tamper) {
    options.arms = ["none"];
    options.trials = TAMPER_TASKS.length;
  }
  return options;
}

/**
 * What every replay Codex process gets besides the harness's own environment: the Bedrock token `--bedrock-token-cmd`
 * prints, run once here. The token is kept out of the harness's environment (setup commands, git) and out of the
 * report; {@link providerConfig} keeps it out of the agent's shell.
 */
function codexEnvironment(options: Options): Record<string, string> {
  if (options.bedrockTokenCmd === undefined) return {};
  const result = spawnSync("sh", ["-c", options.bedrockTokenCmd], { encoding: "utf8", timeout: 120_000, stdio: ["ignore", "pipe", "pipe"] });
  const token = (result.stdout ?? "").trim();
  if (result.status !== 0 || token === "" || /\s/u.test(token)) throw new Error(`--bedrock-token-cmd gave no token (exit ${result.status}): ${clip((result.stderr ?? "").trim(), 200)}`);
  return { [BEDROCK_TOKEN_ENV]: token };
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
  /** What building the ledger read and asked (the alignment module's `ledgerBuild` stats); null without them. */
  readonly ledgerStats: unknown;
  readonly ledgerError: string | null;
  /** Every file first changed from where it branched up to `first_visible`. */
  readonly firstFiles: readonly string[];
  /** Second's own head merged with first's: the conflict the real history had. */
  readonly reference: MergeCheck | null;
  /** First and the `concurrent` refs as visible at `ledgerTime`, with their hunks: what the `git-conflict` arms identify against. */
  readonly refs: readonly RefSide[];
  readonly refsMs: number;
  /** `--completion`: the pair's completion label; null otherwise. */
  readonly label: CompletionLabel | null;
}

function loadPairs(dataset: string): Pair[] {
  return z.object({ pairs: z.array(pairSchema) }).passthrough().parse(JSON.parse(readFileSync(join(dataset, "pairs.json"), "utf8"))).pairs;
}

/** The replay specs in `<dataset>/replay/`, by id. */
function loadSpecs(dataset: string): Map<string, ReplaySpec> {
  const specFolder = join(dataset, "replay");
  const specs = new Map<string, ReplaySpec>();
  for (const name of existsSync(specFolder) ? readdirSync(specFolder).filter(file => file.endsWith(".json")) : []) {
    const spec = specSchema.parse(JSON.parse(readFileSync(join(specFolder, name), "utf8")));
    specs.set(spec.id, spec);
  }
  return specs;
}

function loadDataset(options: Options): Array<{ pair: Pair; spec: ReplaySpec }> {
  const pairs = loadPairs(options.dataset);
  const specs = loadSpecs(options.dataset);
  const wanted = options.pairs ?? pairs.filter(pair => specs.has(pair.id)).map(pair => pair.id);
  return wanted.map(id => {
    const pair = pairs.find(candidate => candidate.id === id);
    const spec = specs.get(id);
    if (pair === undefined) throw new Error(`No pair ${id} in ${join(options.dataset, "pairs.json")}`);
    if (spec === undefined) throw new Error(`No replay spec for ${id} in ${join(options.dataset, "replay")}`);
    return { pair, spec };
  });
}

/**
 * `--completion`'s selection: each label in `<dataset>/completion/` (or those `--pairs` names, by label id) with its
 * pair and a replay spec built from the label (its base, task, `done` and setup). `first_visible` is the pair's replay
 * spec's when one exists for the same base, else first's newest labelled commit visible at second's start (first's
 * base when none was).
 */
function loadCompletionDataset(options: Options): Array<{ pair: Pair; spec: ReplaySpec; label: CompletionLabel }> {
  const pairs = loadPairs(options.dataset);
  const specs = loadSpecs(options.dataset);
  const folder = join(options.dataset, "completion");
  const labels = new Map<string, CompletionLabel>();
  for (const name of existsSync(folder) ? readdirSync(folder).filter(file => file.endsWith(".json") && !options.exclude.includes(file.slice(0, -".json".length))) : []) {
    const label = completionLabelSchema.parse(JSON.parse(readFileSync(join(folder, name), "utf8")));
    labels.set(label.id, label);
  }
  return (options.pairs ?? [...labels.keys()]).map(id => {
    const label = labels.get(id);
    if (label === undefined) throw new Error(`No completion label ${id} in ${folder}`);
    const pair = pairs.find(candidate => candidate.id === label.pair);
    if (pair === undefined) throw new Error(`${id}: no pair ${label.pair} in ${join(options.dataset, "pairs.json")}`);
    const replay = specs.get(label.pair);
    const started = ledgerTime(pair, label.base_sha);
    const firstVisible = replay !== undefined && replay.base_sha === label.base_sha
      ? replay.first_visible
      : pair.first.commits.filter(commit => Date.parse(commit.visible_at) <= Date.parse(started)).at(-1)?.sha ?? pair.first.base_sha;
    const spec: ReplaySpec = { id: label.id, base_sha: label.base_sha, task: label.task, first_visible: firstVisible, rubric: label.conflict_items.join("\n"), done: label.done, setup: label.setup };
    return { pair, spec, label };
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

/** Each source's base branch refs, by labelled name ("" for the default branch). */
const baseBranches = new Map<string, string | null>();

/**
 * The branch a side's work is measured against: its labelled `base_ref` when it names one, else the repository's
 * default branch (`origin/main` or `origin/master` in a clone that tracks a remote, else HEAD, `main`, `master`).
 */
function baseBranchRef(source: string, name: string | undefined): string | null {
  const key = `${source}\0${name ?? ""}`;
  if (!baseBranches.has(key)) {
    const head = gitRead(source, ["symbolic-ref", "-q", "HEAD"])?.trim();
    const candidates = name !== undefined
      ? [`refs/remotes/origin/${name}`, `refs/heads/${name}`, name]
      : ["refs/remotes/origin/main", "refs/remotes/origin/master", ...(head === undefined || head === "" ? [] : [head]), "refs/heads/main", "refs/heads/master"];
    baseBranches.set(key, candidates.find(ref => gitRead(source, ["rev-parse", "--verify", "-q", `${ref}^{commit}`]) !== null) ?? null);
  }
  return baseBranches.get(key)!;
}

/** Merge bases and ancestry asked again and again (`--detect-only` replays hundreds of commits against the same refs). */
const mergeBases = new Map<string, string>();

function mergeBase(source: string, a: string, b: string): string {
  const key = `${source}\0${a}\0${b}`;
  if (!mergeBases.has(key)) mergeBases.set(key, gitRead(source, ["merge-base", a, b])?.trim() ?? "");
  return mergeBases.get(key)!;
}

function isAncestor(source: string, ancestor: string, descendant: string): boolean {
  return ancestor === descendant || mergeBase(source, ancestor, descendant) === ancestor;
}

/**
 * Where `sha`'s own work starts, as its pull request shows it: its merge base with its base branch's tip as of `time`,
 * so mainline work it merged in is not its own; or, when that is nearer, its merge base with `base` (the agent's
 * starting point), so work already in the agent's base is not its own either (a branch stacked on the agent's). When
 * the base branch then already holds all of `sha` (a merged pull request, or the base branch itself), a labelled
 * side's own `base_sha` (`labelledBase`) is where it starts, and without one it has no own work (`sha`). With no base
 * branch tip by then, its merge base with `base`.
 */
export function ownBase(source: string, sha: string, time: string, branch: string | null, base: string, labelledBase?: string): string {
  const shared = mergeBase(source, base, sha);
  const tip = branch === null ? null : refAt(source, branch, time);
  if (tip === null) return shared;
  const own = mergeBase(source, sha, tip);
  if (own === sha) return labelledBase !== undefined && hasCommit(source, labelledBase) ? mergeBase(source, labelledBase, sha) : sha;
  // The nearer of the two when one descends from the other; the base branch's otherwise.
  return shared !== "" && own !== "" && isAncestor(source, own, shared) ? shared : own;
}

/** A visible ref: its commit then, its base branch, and the labelled base of a pair side. */
interface VisibleRef {
  readonly ref: string;
  readonly sha: string;
  readonly branch: string | null;
  readonly labelledBase?: string;
}

/**
 * The other refs as visible at `time`: first at `firstSha`, then each `concurrent` ref other than the two sides at
 * its newest first-parent commit committed by then.
 */
function visibleShas(source: string, pair: Pair, time: string, firstSha: string | null): VisibleRef[] {
  const firstRef = sideRef(pair.first) ?? "first";
  const secondRef = sideRef(pair.second);
  const visible: VisibleRef[] = firstSha === null ? [] : [{ ref: firstRef, sha: firstSha, branch: baseBranchRef(source, pair.first.base_ref), labelledBase: pair.first.base_sha }];
  for (const entry of pair.concurrent) {
    const ref = typeof entry === "number" ? `refs/pull/${entry}/head` : entry;
    if (ref === firstRef || ref === secondRef) continue;
    const sha = refAt(source, ref, time);
    if (sha !== null) visible.push({ ref, sha, branch: baseBranchRef(source, undefined) });
  }
  return visible;
}

/**
 * What the `git-conflict` arms identify against: each visible ref with its own work ({@link ownBase} at `time`), its
 * author, when it became public, its stated intent and its hunks in the base's lines. First's author and pull request
 * title come from the pair; a `concurrent` ref's from its head and its latest own commit subjects.
 */
function visibleRefs(source: string, pair: Pair, base: string, time: string, firstSha: string): RefSide[] {
  const firstRef = sideRef(pair.first) ?? "first";
  return visibleShas(source, pair, time, firstSha).flatMap(({ ref, sha, branch, labelledBase }): RefSide[] => {
    const own = ownBase(source, sha, time, branch, base, labelledBase);
    if (own === "" || own === sha) return [];
    const commits = (gitRead(source, ["log", "--format=%cI", `${own}..${sha}`]) ?? "").split("\n").filter(line => line !== "");
    if (commits.length === 0) return [];
    const files = refHunks(source, base, own, sha);
    if (Object.keys(files).length === 0) return [];
    if (ref === firstRef) {
      const shown = pair.first.commits.filter(commit => Date.parse(commit.visible_at) <= Date.parse(time)).map(commit => commit.visible_at);
      const since = [...shown, pair.first.created_at].reduce((earliest, at) => (Date.parse(at) < Date.parse(earliest) ? at : earliest));
      const body = typeof pair.first.body === "string" ? pair.first.body : undefined;
      const pr = typeof pair.first.pr === "number" ? { title: pair.first.title, ...(body === undefined ? {} : { body }) } : null;
      return [{ label: refLabel(ref), ref, sha, author: pair.first.author, since, intent: statedIntent(source, own, sha, pr), files }];
    }
    const author = gitRead(source, ["log", "-1", "--format=%an", sha])?.trim() ?? "unknown";
    return [{ label: refLabel(ref), ref, sha, author, since: commits.at(-1)!, intent: statedIntent(source, own, sha, null), files }];
  });
}

/**
 * When the replayed work started. A spec whose `base_sha` is one of second's own commits replays the work after it
 * (a long branch replayed from the middle), so that commit's authoring; otherwise second's creation, or its earliest
 * authored commit when that is earlier.
 */
function ledgerTime(pair: Pair, baseSha: string): string {
  const base = pair.second.commits.find(commit => commit.sha.startsWith(baseSha) || baseSha.startsWith(commit.sha));
  if (base !== undefined) return base.authored_at;
  const times = [pair.second.created_at, ...pair.second.commits.map(commit => commit.authored_at)];
  return times.reduce((earliest, time) => (Date.parse(time) < Date.parse(earliest) ? time : earliest));
}

/** How the ledger's judge runs: under `--provider`, the run's Codex on that provider with the grader's model and no web. */
function judgeCodexOf(options: Options): LedgerOptions["judgeCodex"] {
  if (options.provider === undefined) return undefined;
  return {
    executable: codexBinary(),
    config: [`model_provider=${JSON.stringify(options.provider)}`, `model_providers.${options.provider}.aws.region=${JSON.stringify(options.bedrockRegion)}`, `web_search="disabled"`, "features.apps=false"],
    ...(options.judgeModel === undefined ? {} : { model: options.judgeModel }),
  };
}

/** A ledger `--build-ledgers` stored: whose, at what time, built how, its units and what building it read and asked. */
interface StoredLedger {
  readonly id: string;
  readonly pair: string;
  readonly time: string;
  readonly prBodies: boolean;
  readonly judge: LedgerOptions["judgeCodex"] | null;
  readonly units: LedgerUnit[];
  readonly stats: unknown;
  readonly ms: number;
}

const ledgerPath = (options: Options, id: string): string => join(options.ledgerCache, "ledgers", `${id}.json`);

/** The ledger stored at `path` when it is the same pair's at the same time with the same description input. */
function storedLedger(path: string, pair: Pair, time: string, options: Options): StoredLedger | null {
  if (!existsSync(path)) return null;
  const stored = JSON.parse(readFileSync(path, "utf8")) as StoredLedger;
  return stored.pair === pair.id && stored.time === time && stored.prBodies === (options.prBodies !== undefined) ? stored : null;
}

/** What the alignment module builds a ledger with: the cache, descriptions, judge calls in flight, the judge's Codex and token. */
function ledgerOptionsOf(options: Options, codexEnv: Readonly<Record<string, string>>): LedgerOptions {
  const judgeCodex = judgeCodexOf(options);
  return { cacheDir: options.ledgerCache, concurrency: options.judgeConcurrency, judgeEnv: codexEnv, ...(options.prBodies === undefined ? {} : { prBodies: options.prBodies }), ...(judgeCodex === undefined ? {} : { judgeCodex }) };
}

/**
 * The ledger of the run `id` (a pair, or a completion label) at `time`: the one `--build-ledgers` stored for the same
 * pair, time and description input, else built now (`ledgerBuild` when the module has it) and stored.
 */
async function pairLedger(align: AlignModule, source: string, pair: Pair, id: string, time: string, options: Options, codexEnv: Readonly<Record<string, string>>): Promise<StoredLedger> {
  const path = ledgerPath(options, id);
  const earlier = storedLedger(path, pair, time, options);
  if (earlier !== null) return earlier;
  const started = performance.now();
  const ledgerOptions = ledgerOptionsOf(options, codexEnv);
  const built = align.ledgerBuild === undefined ? { units: await align.ledgerAt(source, pair, time, ledgerOptions), stats: null } : await align.ledgerBuild(source, pair, time, ledgerOptions);
  const stored: StoredLedger = { id, pair: pair.id, time, prBodies: options.prBodies !== undefined, judge: ledgerOptions.judgeCodex ?? null, units: built.units, stats: built.stats, ms: Math.round(performance.now() - started) };
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, `${JSON.stringify(stored, null, 2)}\n`);
  return stored;
}

/** Second's own head merged with first's in a scratch repository: the conflict the real history had; null without second's head. */
async function referenceMerge(source: string, objects: string, pair: Pair): Promise<MergeCheck | null> {
  if (!hasCommit(source, pair.second.head_sha)) return null;
  const folder = await realpath(await mkdtemp(join(tmpdir(), "rerail-ma-replay-pair-")));
  try {
    return mergeCheck(scratchRepository(folder, [objects]), pair.second.head_sha, pair.first.head_sha);
  } finally {
    await removeFolder(folder);
  }
}

/** The pair context's ledger fields: the ledger's units, build time and stats, or why it failed; all null without a module. */
async function ledgerFields(align: AlignModule | null, source: string, pair: Pair, id: string, time: string, options: Options, codexEnv: Readonly<Record<string, string>>): Promise<Pick<PairContext, "units" | "ledgerMs" | "ledgerStats" | "ledgerError">> {
  const none = { units: null, ledgerMs: null, ledgerStats: null, ledgerError: null };
  if (align === null) return none;
  try {
    const ledger = await pairLedger(align, source, pair, id, time, options, codexEnv);
    return { units: ledger.units, ledgerMs: ledger.ms, ledgerStats: ledger.stats, ledgerError: null };
  } catch (error) {
    return { ...none, ledgerError: error instanceof Error ? error.message : String(error) };
  }
}

async function preparePair(pair: Pair, spec: ReplaySpec, options: Options, align: AlignModule | null, codexEnv: Readonly<Record<string, string>>, label: CompletionLabel | null = null): Promise<PairContext> {
  const source = options.repo ?? join(options.repos, pair.repo.replace("/", "__"));
  if (!existsSync(source)) throw new Error(`${pair.id}: no repository at ${source}`);
  for (const [name, sha] of [["base_sha", spec.base_sha], ["first.head_sha", pair.first.head_sha], ["first_visible", spec.first_visible]] as const) {
    if (!hasCommit(source, sha)) throw new Error(`${pair.id}: ${source} has no commit ${sha} (${name})`);
  }
  const objects = join(git(source, ["rev-parse", "--path-format=absolute", "--git-common-dir"]).trim(), "objects");
  const firstFiles = git(source, ["diff", "--name-only", `${pair.first.base_sha}...${spec.first_visible}`]).split("\n").filter(path => path !== "");
  const reference = await referenceMerge(source, objects, pair);
  const time = ledgerTime(pair, spec.base_sha);
  const ledger = await ledgerFields(align, source, pair, label?.id ?? pair.id, time, options, codexEnv);
  const refsStarted = performance.now();
  const refs = options.tamper ? [] : visibleRefs(source, pair, spec.base_sha, time, spec.first_visible);
  const refsMs = Math.round(performance.now() - refsStarted);
  return { pair, spec, source, objects, ledgerTime: time, ...ledger, firstFiles, reference, refs, refsMs, label };
}

// ---------------------------------------------------------------------------------------------------------- sandbox

/** The permission profile every replay Codex call runs under (agent and grader). */
const PROFILE = "replay";
/** Folders under the home folder an agent may read: toolchains it may run. Everything else there is denied. */
const HOME_TOOLCHAINS = [".cargo", ".rustup", "go", ".nvm", ".pyenv", ".bun", ".volta", ".deno", ".local/bin"];
/** Where a command may read without it counting as a leak: the system and the toolchains, besides the run's folder. */
const SYSTEM_ROOTS = ["/bin", "/sbin", "/usr", "/lib", "/lib64", "/dev", "/etc", "/private/etc", "/opt/homebrew", "/Library/Developer", "/System", "/Applications/Xcode.app"];

/** `--codex-bin`, set once by `main` before any run; `codex` on PATH when unset. */
let codexOverride: string | undefined;

/**
 * The real file behind `--codex-bin` or `codex` on PATH. Every replay call runs it by that path: Codex's own sandbox
 * helper re-runs the executable it was started as, and a symlink in the denied home folder cannot be followed from
 * inside the sandbox.
 */
function codexBinary(): string {
  if (codexOverride !== undefined) return realpathSync(codexOverride);
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

/** The model provider every Codex call of a run uses (`--provider`, `--bedrock-region`); null for Codex's default. */
export interface ProviderSettings {
  readonly provider: string;
  readonly region: string;
}

function providerOf(options: Options): ProviderSettings | null {
  return options.provider === undefined ? null : { provider: options.provider, region: options.bedrockRegion };
}

/**
 * The home folder denied but `readable`: on Linux, every entry of it (recursively along the paths to `readable`) that
 * is not one of them, since bubblewrap cannot open a readable folder inside a denied one; elsewhere the folder itself.
 */
function homeDenials(readable: readonly string[]): string[] {
  const home = homedir();
  if (process.platform !== "linux") return [home];
  const denied: string[] = [];
  const walk = (folder: string) => {
    for (const entry of readdirSync(folder, { withFileTypes: true })) {
      const path = join(folder, entry.name);
      // A symlink is denied through what it points to.
      if (entry.isSymbolicLink() || readable.includes(path)) continue;
      if (entry.isDirectory() && readable.some(keep => keep.startsWith(`${path}/`))) walk(path);
      else denied.push(path);
    }
  };
  walk(home);
  return denied;
}

/** Turns off Codex's built-in apps server, whose `search_service.web_run` reaches the web even with web search off. */
const NO_WEB_ARGS = ["--disable", "apps"] as const;

/**
 * Removes a run's folder. Cleanup never fails a job: Codex's background plugin sync, still writing into the grader's
 * CODEX_HOME, once made `rmdir` hit ENOTEMPTY and killed every run after it.
 */
async function removeFolder(folder: string): Promise<void> {
  try {
    await rm(folder, { recursive: true, force: true, maxRetries: 5, retryDelay: 500 });
  } catch (error) {
    console.error(`could not remove ${folder}: ${error instanceof Error ? error.message : String(error)}`);
  }
}

/**
 * The run's CODEX_HOME config: the model provider, if any; a shell environment policy that keeps credentials out of
 * every command the agent runs (Codex's default excludes `*KEY*`, `*SECRET*`, `*TOKEN*`, which it skips unless told
 * otherwise, and every `AWS_*` variable) and sets `shell` in it (the agent's toolchain PATH and scratch folders, see
 * {@link agentShellEnv}); and a permission profile that extends Codex's workspace profile (the workspace root
 * writable, no network) and takes away what the default leaves open: writes and reads in `/tmp` and `$TMPDIR` (other
 * runs' folders, the curated data under `/tmp/ma-ctx`), the whole home folder but its toolchains (every other clone),
 * and each of `denied` (the source repository, the dataset, the ledger cache).
 */
export function sandboxConfig(denied: readonly string[], provider: ProviderSettings | null = null, shell: Readonly<Record<string, string>> = {}): string {
  const roots = ["/tmp", "/private/tmp", tmpdir(), realpathSync(tmpdir()), homedir()];
  // A path under a denied root is denied with it; naming it again makes bubblewrap (Linux) fail to set up the mount.
  const extra = denied.filter(path => existsSync(path) && !roots.some(root => [path, realpathSync(path)].some(form => form.startsWith(`${root}/`))));
  const readable = homeReadable();
  const entries: Array<[string, string]> = [
    [":slash_tmp", "deny"],
    ...(existsSync("/private/tmp") ? [["/private/tmp", "deny"] as [string, string]] : []),
    [":tmpdir", "deny"],
    ...homeDenials(readable).map((path): [string, string] => [path, "deny"]),
    ...extra.map((path): [string, string] => [path, "deny"]),
    ...readable.map((path): [string, string] => [path, "read"]),
  ];
  return [
    ...(provider === null ? [] : [`model_provider = ${JSON.stringify(provider.provider)}`]),
    `default_permissions = ${JSON.stringify(PROFILE)}`,
    // No web: the search tool and the built-in apps server (`search_service.web_run`) both reach the internet.
    `web_search = "disabled"`,
    "",
    ...(provider === null ? [] : [`[model_providers.${provider.provider}.aws]`, `region = ${JSON.stringify(provider.region)}`, ""]),
    "[shell_environment_policy]",
    `inherit = "all"`,
    "ignore_default_excludes = false",
    `exclude = ["AWS_*", "*TOKEN*", "*SECRET*", "*KEY*", "*CREDENTIAL*", "*PASSWORD*"]`,
    ...(Object.keys(shell).length === 0 ? [] : [`set = { ${Object.entries(shell).map(([name, value]) => `${name} = ${JSON.stringify(value)}`).join(", ")} }`]),
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

/** The workspace folder the agent's tools use for temporary files and caches; kept out of the diff (`.git/info/exclude`). */
const SCRATCH = ".replay-tmp";

/**
 * The PATH a task's setup runs with and finds its tools on: the harness's own, then where toolchains live on the
 * hosts besides it (Go's tarball install, Go's and Cargo's bin folders, this Node).
 */
function setupPath(): string[] {
  const home = homedir();
  const toolchains = ["/usr/local/go/bin", join(home, "go", "bin"), join(home, ".cargo", "bin"), join(home, ".local", "bin"), dirname(process.execPath)].filter(path => existsSync(path));
  return [...new Set([...(process.env.PATH ?? "").split(":").filter(dir => dir !== ""), ...toolchains])];
}

/** The tools a pair's language builds and tests with. */
const LANGUAGE_TOOLS: Readonly<Record<string, readonly string[]>> = {
  go: ["go", "gofmt"],
  rust: ["cargo", "rustc", "rustfmt"],
  python: ["python3", "python", "uv", "pip", "pytest"],
  typescript: ["node", "npm", "npx", "pnpm", "yarn"],
  javascript: ["node", "npm", "npx", "pnpm", "yarn"],
};

/** The commands a setup runs: the first word of each simple command, past variable assignments and `env`. */
export function setupCommands(setup: string | null): string[] {
  if (setup === null) return [];
  return [...new Set(setup.split(/&&|\|\||[;|\n()]/u).flatMap(command => {
    const word = command.trim().split(/\s+/u).find(token => token !== "env" && !/^[A-Za-z_][A-Za-z0-9_]*=/u.test(token));
    return word !== undefined && /^[A-Za-z0-9_.+-]+$/u.test(word) ? [word] : [];
  }))];
}

/**
 * The agent's PATH: the workspace's own tool folders after setup (`.venv/bin`, `node_modules/.bin`), then the folder
 * each of the language's tools and the setup's commands is found in on {@link setupPath} (Go in `/usr/local/go/bin`
 * on the build hosts), then the rest of that path; every folder one the sandbox lets the agent read.
 */
export function sandboxPath(language: string, setup: string | null, repo: string): string[] {
  const search = setupPath();
  const readable = [repo, ...SYSTEM_ROOTS, ...homeReadable()];
  const local = [join(repo, ".venv", "bin"), join(repo, "node_modules", ".bin")].filter(path => existsSync(path));
  const found = [...(LANGUAGE_TOOLS[language] ?? []), ...setupCommands(setup)].flatMap(tool => search.find(dir => existsSync(join(dir, tool))) ?? []);
  return [...new Set([...local, ...found, ...search].filter(dir => readable.some(root => dir === root || dir.startsWith(`${root}/`))))];
}

/**
 * What the agent's commands run with besides Codex's environment: {@link sandboxPath} as PATH, and temporary files
 * and caches in the workspace's {@link SCRATCH} folder, since the sandbox makes `/tmp` and the home folder's caches
 * read-only or unreadable (Go cannot build without a writable `GOCACHE` and temporary folder). `GOTOOLCHAIN=local`:
 * Go never tries to download another toolchain.
 */
export function agentShellEnv(language: string, setup: string | null, repo: string): Record<string, string> {
  const scratch = join(repo, SCRATCH);
  return { PATH: sandboxPath(language, setup, repo).join(":"), TMPDIR: join(scratch, "tmp"), XDG_CACHE_HOME: join(scratch, "cache"), GOCACHE: join(scratch, "cache", "go-build"), GOTOOLCHAIN: "local" };
}

/** One thing the agent did that reaches past its workspace: where it was seen, why it counts, and the call's text. */
export interface Leak {
  readonly source: string;
  readonly reason: string;
  readonly text: string;
}

/**
 * Commands and hosts that reach other copies of the history (the network, GitHub, pull request refs), and calls that
 * print the environment or read a credential (the Bedrock token Codex runs with, AWS's instance metadata or files).
 */
const OUTSIDE: ReadonlyArray<readonly [string, RegExp]> = [
  ["network command", /(?:^|[\s;&|('"`])(?:curl|wget|gh|nc|ssh|scp|rsync)\s/u],
  ["git network command", /\bgit\b[^\n;&|]*\s(?:fetch|clone|pull|ls-remote|submodule\s+update)\b|\bremote\s+(?:add|set-url)\b/u],
  ["remote host", /\b(?:github\.com|gitlab\.com|bitbucket\.org|codeload\.github|api\.github)\b/u],
  ["pull request ref", /refs\/pull\//u],
  ["curated data", /\/tmp\/ma-ctx\b/u],
  ["credential read", /\$\{?AWS_[A-Z0-9_]+|\bAWS_BEARER_TOKEN|BEARER_TOKEN_BEDROCK|\bsecretsmanager\b|get-secret-value|169\.254\.169\.254|\.aws\/(?:credentials|config)\b/u],
];

/**
 * A call that prints the whole environment: `env` or `set` alone (or into a pipe or file), `printenv`, `export -p` or
 * `declare -x` where a command starts, an `/proc/<pid>/environ` read, or all of `os.environ`, `process.env`,
 * `env::vars` or `os.Environ()`. Tested on {@link withoutSearchPatterns}' text, so a search pattern that only names
 * one of them (`rg "row set|…"`) is not a dump.
 */
const ENVIRONMENT_DUMP = /(?:^|[;&|(\n`]|\$\()\s*["'`]?(?:(?:sudo|command|exec|builtin)\s+)?(?:(?:env|set)(?=\s*(?:$|[|;&>)\n'"`]))|printenv\b|export\s+-p\b|declare\s+-[a-zA-Z]*x)|\/proc\/[^\s/]+\/environ|\b(?:os\.environ|process\.env)\b(?!\s*(?:\.|\[|\?\.))|\benv::vars\b|\bos\.Environ\(\)/u;

/** Commands whose quoted arguments are patterns to search for, not code that runs. */
const SEARCH_COMMANDS = /^(?:rg|grep|egrep|fgrep|ag|ack|git\s+grep)\b/u;

/** `text` itself, or the string it encodes when it is a JSON string (the transcript quotes commands so). */
function jsonUnquoted(text: string): string {
  if (!text.startsWith("\"") || !text.endsWith("\"")) return text;
  try {
    const parsed: unknown = JSON.parse(text);
    return typeof parsed === "string" ? parsed : text;
  } catch {
    return text;
  }
}

/** A call's shell script: a JSON-quoted command (as the transcript holds it) and `bash -lc "<script>"` unwrapped. */
function shellScript(text: string): string {
  const script = jsonUnquoted(text.trim());
  const wrapped = /^(?:\S*\/)?(?:bash|sh|zsh)\s+-[a-z]*c\s+(["'])([\s\S]*)\1\s*$/u.exec(script);
  if (wrapped === null) return script;
  return wrapped[1] === "'" ? wrapped[2]! : wrapped[2]!.replace(/\\(["\\$`])/gu, "$1");
}

/** A shell script's pieces: quoted strings, escapes, command separators, and the plain text between them. */
const SHELL_PIECE = /'[^']*'?|"(?:[^"\\]|\\[\s\S])*"?|\\[\s\S]?|[;&|()\n`]|[^'"\\;&|()\n`]+/gu;

/**
 * `text` with the quoted arguments of search commands (`rg`, `grep`, `git grep`, …) emptied: the script is split into
 * simple commands at unquoted `;`, `&`, `|`, parentheses, backticks and newlines, and in a command whose first word
 * searches, each quoted string becomes `""`. Everything else is kept as written.
 */
export function withoutSearchPatterns(text: string): string {
  const commands: string[][] = [[]];
  for (const [piece] of shellScript(text).matchAll(SHELL_PIECE)) {
    if (/^[;&|()\n`]$/u.test(piece)) commands.push([piece], []);
    else commands.at(-1)!.push(piece);
  }
  return commands.map(pieces => {
    const command = pieces.join("");
    const searching = SEARCH_COMMANDS.test(command.replace(/^\s*(?:\w+=\S*\s+)*(?:sudo\s+|command\s+|exec\s+)?/u, ""));
    return searching ? pieces.map(piece => (/^["']/u.test(piece) ? "\"\"" : piece)).join("") : command;
  }).join("");
}

/** A token that starts a path: absolute, from the home folder, or climbing with `..`. */
const PATH_TOKEN = /(?:^|[\s'"`=:(,;|&<>[{])((?:~|\$HOME|\$\{HOME\})?\/[^\s'"`;|&<>(),\]}]*|(?:\.\.\/)+[^\s'"`;|&<>(),\]}]*|\.\.(?=$|[\s'"`;|&)]))/gu;

/** Why `text` (one call's command or input) reaches past `folder`, the run's folder; none when it stays inside. */
export function leakReasons(text: string, folder: string, cwd: string): string[] {
  const reasons = OUTSIDE.flatMap(([reason, pattern]) => (pattern.test(text) ? [reason] : []));
  if (ENVIRONMENT_DUMP.test(withoutSearchPatterns(text))) reasons.push("environment dump");
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
  // The `units` arm's surface framing.
  markers.push(/\) established:/u, /in[- ]flight work/iu);
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

/**
 * One blind structured Codex call (the grader's, the reviewer's): no hooks, an empty folder `<name>` in `folder`, the
 * sandbox profile, `--ephemeral`, and `--output-schema` holding its final message to `jsonSchema`. Returns that
 * message parsed as JSON, or why there is none.
 */
async function judgeCall(folder: string, name: string, prompt: string, jsonSchema: object, options: Options, denied: readonly string[], codexEnv: Readonly<Record<string, string>>): Promise<{ answer: unknown; error: string | null; usage: Usage | null; wallMs: number }> {
  const work = join(folder, name);
  mkdirSync(work, { recursive: true });
  const schema = join(folder, `${name}-schema.json`);
  writeFileSync(schema, JSON.stringify(jsonSchema));
  const codexHome = join(folder, `${name}-codex-home`);
  createCodexHome(codexHome, "", null);
  writeFileSync(join(codexHome, "config.toml"), sandboxConfig(denied, providerOf(options)));
  const judge: CodexRunOptions = { model: options.judgeModel, config: [], timeoutMin: options.judgeTimeoutMin, extraArgs: ["--output-schema", schema, "--ephemeral", ...NO_WEB_ARGS], permissions: PROFILE, binary: codexBinary(), env: codexEnv };
  const outcome = await runCodex(work, codexHome, "", prompt, judge, join(folder, `${name}-rerail-home`));
  const events = readEvents(outcome.stdout);
  const base = { usage: events.usage, wallMs: outcome.wallMs };
  if (outcome.timedOut) return { ...base, answer: null, error: `${name} timed out after ${options.judgeTimeoutMin} minutes` };
  if (events.finalMessage === null) return { ...base, answer: null, error: events.error ?? `${name} exited ${outcome.exitCode} with no answer: ${clip(outcome.stderr, 200)}` };
  try {
    return { ...base, answer: JSON.parse(events.finalMessage) as unknown, error: null };
  } catch (error) {
    return { ...base, answer: null, error: `${name} answer unreadable (${error instanceof Error ? error.message : String(error)}): ${clip(events.finalMessage, 200)}` };
  }
}

async function gradeRun(context: PairContext, diff: string, folder: string, options: Options, denied: readonly string[], codexEnv: Readonly<Record<string, string>>): Promise<Grade> {
  const { spec, pair } = context;
  const { text, stripped, truncated } = graderDiff(diff, pair, context.refs.map(side => side.author));
  if (text.trim() === "") return { verdict: { conflicts_with_first: "no", reasons: ["The run changed no file."], done: "no" }, by: "empty-diff", error: null, strippedLines: stripped, truncated, usage: null, wallMs: 0 };
  const call = await judgeCall(folder, "grade", gradePrompt(spec, text), VERDICT_JSON_SCHEMA, options, denied, codexEnv);
  const base = { by: null, strippedLines: stripped, truncated, usage: call.usage, wallMs: call.wallMs };
  if (call.error !== null) return { ...base, verdict: null, error: call.error };
  const parsed = verdictSchema.safeParse(call.answer);
  if (!parsed.success) return { ...base, verdict: null, error: `grader answer unreadable (${parsed.error.message}): ${clip(JSON.stringify(call.answer), 200)}` };
  return { ...base, by: "grader", verdict: parsed.data, error: null };
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

/**
 * The workspace at `repo`: the base snapshot as its only commit, from the source's objects through a throwaway index;
 * then the {@link SCRATCH} folder, which `.git/info/exclude` keeps out of every later commit and diff.
 */
function checkoutBase(context: PairContext, folder: string, repo: string): void {
  git(folder, ["init", "-q", repo]);
  const index = { GIT_INDEX_FILE: join(folder, "base.index") };
  git(context.source, ["read-tree", context.spec.base_sha], index);
  git(context.source, [`--work-tree=${repo}`, "checkout-index", "--all", "--force"], index);
  git(repo, ["add", "--all", "--force"]);
  git(repo, ["commit", "-q", "--no-verify", "-m", "base"]);
  mkdirSync(join(repo, ".git", "info"), { recursive: true });
  writeFileSync(join(repo, ".git", "info", "exclude"), `/${SCRATCH}/\n`, { flag: "a" });
  for (const sub of ["tmp", "cache"]) mkdirSync(join(repo, SCRATCH, sub), { recursive: true });
}

/** The spec's setup in the workspace, on {@link setupPath} (so it finds Go where the hosts install it), committed. */
function runSetup(command: string, repo: string): NonNullable<ReplayRun["setup"]> {
  const started = performance.now();
  const result = spawnSync("sh", ["-c", command], { cwd: repo, env: { ...process.env, PATH: setupPath().join(":") }, encoding: "utf8", timeout: SETUP_TIMEOUT_MS, maxBuffer: 64 * 1024 * 1024 });
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

/** A run's prepared workspace: the repository after setup, its CODEX_HOME with the hook, and what the sandbox denies. */
interface Workspace {
  readonly folder: string;
  readonly repo: string;
  readonly codexHome: string;
  readonly configPath: string;
  readonly logPath: string;
  readonly denied: readonly string[];
  readonly setup: ReplayRun["setup"];
  /** The commit the diff is taken against: the base snapshot after setup. */
  readonly diffBase: string;
  /** What the agent's commands run with ({@link agentShellEnv}). */
  readonly shell: Readonly<Record<string, string>>;
}

/** What the hook's arm reads: the ledger and the alignment module under the ledger arms, its ranking's judge and log. */
function ledgerConfig(job: Job, options: Options, folder: string): Pick<ReplayHookConfig, "units" | "alignModule" | "alignOptions"> {
  const { context, arm } = job;
  const judgeCodex = judgeCodexOf(options);
  // The hook's ranking judge runs on the run's provider; its token reaches the hook through Codex's environment.
  const alignOptions = { cacheDir: join(folder, "align-cache"), rankLog: join(folder, RANK_LOG), ...(judgeCodex === undefined ? {} : { judgeCodex }) };
  if (!LEDGER_ARMS.includes(arm)) return { units: [], alignModule: null, alignOptions };
  if (context.units === null) throw new Error(`no ledger: ${context.ledgerError ?? "not built"}`);
  return { units: context.units, alignModule: options.alignModule, alignOptions };
}

/**
 * The run's workspace in `folder`: the base snapshot, the spec's setup, the hook's configuration for the arm, and the
 * CODEX_HOME whose config holds the sandbox profile and the agent's shell environment.
 */
function prepareWorkspace(job: Job, options: Options, folder: string, task: string): Workspace {
  const { context, arm } = job;
  const { spec } = context;
  const repo = join(folder, "repo");
  const codexHome = join(folder, "codex-home");
  const configPath = join(folder, "hook-config.json");
  const logPath = join(folder, "hook.log");
  // What no replay Codex call may read: the source repository, its objects, the dataset, the clones, the ledger cache
  // and the fetched descriptions.
  const denied = [context.source, dirname(context.objects), options.dataset, options.repos, options.ledgerCache, ...(options.prBodies === undefined ? [] : [options.prBodies])];
  const ledger = ledgerConfig(job, options, folder);
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
    ...ledger,
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
  const shell = agentShellEnv(context.pair.language, spec.setup, repo);
  writeFileSync(join(codexHome, "config.toml"), sandboxConfig(denied, providerOf(options), shell));
  return { folder, repo, codexHome, configPath, logPath, denied, setup, diffBase, shell };
}

/** The agent's Codex options: the run's model and timeout, no web, the sandbox profile, the provider's token. */
function agentOptions(options: Options, codexEnv: Readonly<Record<string, string>>): CodexRunOptions {
  return { ...options, extraArgs: NO_WEB_ARGS, permissions: PROFILE, binary: codexBinary(), env: codexEnv };
}

/** Why a Codex call failed (timed out, reported an error, exited non-zero); null when it ran its course. */
function callFailure(outcome: CodexOutcome, events: CodexEvents, timeoutMin: number): string | null {
  if (outcome.timedOut) return `timed out after ${timeoutMin} minutes`;
  return events.error ?? (outcome.exitCode === 0 ? null : `codex exited ${outcome.exitCode}: ${clip(outcome.stderr.trim().split("\n").at(-1) ?? "", 200)}`);
}

function readHookLog(path: string): ReplayHookLogEntry[] {
  return readFileSync(path, "utf8").split("\n").filter(line => line.trim() !== "").map(line => JSON.parse(line) as ReplayHookLogEntry);
}

/** The run's leaks over every agent call's events and its transcript, and any credential Codex ran with showing up in them. */
function runLeaks(stdouts: readonly string[], transcript: string | null, workspace: Workspace, codexEnv: Readonly<Record<string, string>>): Leak[] {
  const leaks = findLeaks(stdouts.flatMap(parseEvents), transcript, workspace.folder, workspace.repo);
  // A credential Codex ran with that shows up in what the agent saw or said, however it got there.
  for (const secret of Object.values(codexEnv)) {
    if (stdouts.some(stdout => stdout.includes(secret)) || (transcript ?? "").includes(secret)) leaks.push({ source: "output", reason: "a credential in the run's events or transcript", text: "(redacted)" });
  }
  return leaks;
}

/** The workspace against its base after setup, untracked files included: the diff, a binary patch, and per-file line counts. */
function workspaceDiff(workspace: Workspace): { diff: string; patch: string; files: ReplayRun["files"] } {
  const { repo, diffBase } = workspace;
  git(repo, ["add", "--all"]);
  const files = git(repo, ["diff", "--cached", "--numstat", diffBase]).split("\n").filter(line => line !== "").map(line => {
    const [added, removed, ...path] = line.split("\t");
    return { path: path.join("\t"), added: Number(added) || 0, removed: Number(removed) || 0 };
  });
  return { diff: git(repo, ["diff", "--cached", "--no-color", diffBase]), patch: git(repo, ["diff", "--cached", "--binary", "--no-color", diffBase]), files };
}

/** `patch` applied to `base_sha` as a commit in a scratch repository, merged with each of `theirs` (`git merge-tree`). */
function mergeResult(context: PairContext, workspace: Workspace, patch: string, theirs: readonly string[]): MergeCheck[] {
  const scratch = scratchRepository(workspace.folder, [context.objects, join(workspace.repo, ".git", "objects")]);
  const index = { GIT_INDEX_FILE: join(workspace.folder, "result.index") };
  git(scratch, ["read-tree", context.spec.base_sha], index);
  if (patch.trim() !== "") git(scratch, ["apply", "--cached", "--binary", "--3way"], index, patch);
  const tree = git(scratch, ["write-tree"], index).trim();
  const result = git(scratch, ["commit-tree", tree, "-p", context.spec.base_sha, "-m", "run"]).trim();
  return theirs.map(sha => mergeCheck(scratch, result, sha));
}

const errorLine = (error: unknown): string => (error instanceof Error ? error.message.split("\n").slice(0, 3).join(" ") : String(error));

/** Each unit the hook delivered, with whether it came from first and the second into the run it went out at. */
function deliveredUnits(context: PairContext, hookLog: readonly ReplayHookLogEntry[], started: number): ReplayRun["delivered"] {
  return hookLog.flatMap(entry => entry.delivered.map(item => {
    const unit = context.units?.find(candidate => candidate.id === item.unitId);
    return { ...item, fromFirst: unit !== undefined && fromFirst(unit, context.pair), atS: Math.round((entry.at - started) / 1000) };
  }));
}

/** The file in a run's folder `deliverForArea` logs each area's ranking to (merge-assist-align.ts `RankLogEntry`). */
const RANK_LOG = "rank.log";

/** One area's ranking as the alignment module logged it: its anchored candidates, how they were ranked, what it picked. */
export interface RankEntry {
  readonly area: string;
  readonly path: string;
  readonly anchored: number;
  readonly candidates: ReadonlyArray<{ readonly id: string; readonly score: number }>;
  readonly via: string;
  readonly error: string | null;
  readonly picked: readonly string[];
}

function readRankLog(folder: string): RankEntry[] {
  const path = join(folder, RANK_LOG);
  return existsSync(path) ? readFileSync(path, "utf8").split("\n").filter(line => line.trim() !== "").map(line => JSON.parse(line) as RankEntry) : [];
}

/** Where a delivery stands against the agent's first edit in its area: an earlier hook call, the edit's own call, later, or no edit there. */
export type EditTiming = "before" | "at" | "after" | "no-edit";

/**
 * The diagnostic funnel of a ledger-arm run: each area's first edit; every unit delivered (its ref, kind, origin, text,
 * area, rank in the area, and when against the area's first edit); every ranking; and every unit of first's in the
 * ledger with whether it was a candidate where the agent touched, its best candidate position, and whether and when
 * it went out. Whether one of first's units states the adopted design is the integrator's to judge from the label.
 */
export interface UnitsFunnel {
  readonly firstEdits: ReadonlyArray<{ readonly area: string; readonly atS: number }>;
  readonly delivered: ReadonlyArray<{
    readonly id: string; readonly ref: string | null; readonly pr: number | null; readonly branch: string | null; readonly author: string;
    readonly kind: string; readonly origin: string | null; readonly text: string; readonly area: string; readonly path: string; readonly rank: number | null;
    readonly conflict: boolean; readonly fromFirst: boolean; readonly atS: number; readonly firstEditS: number | null; readonly timing: EditTiming;
  }>;
  readonly rankings: readonly RankEntry[];
  readonly firstUnits: ReadonlyArray<{
    readonly id: string; readonly kind: string; readonly origin: string | null; readonly text: string; readonly files: readonly string[]; readonly conflict: boolean;
    /** Areas whose ranking had it among its candidates, and its best position among them (0 first). */
    readonly candidateIn: readonly string[]; readonly candidateRank: number | null;
    readonly delivered: boolean; readonly rank: number | null; readonly timing: EditTiming | null;
  }>;
}

/** Each area's first edit in the hook log: the index and time of the first `pre-edit` call touching a file there. */
function firstEdits(hookLog: readonly ReplayHookLogEntry[]): Map<string, { index: number; at: number }> {
  const edits = new Map<string, { index: number; at: number }>();
  hookLog.forEach((entry, index) => {
    if (entry.trigger !== "pre-edit") return;
    for (const path of entry.paths.filter(item => !item.endsWith("/"))) if (!edits.has(areaOf(path))) edits.set(areaOf(path), { index, at: entry.at });
  });
  return edits;
}

function editTiming(index: number, edit: { index: number } | undefined): EditTiming {
  if (edit === undefined) return "no-edit";
  if (index === edit.index) return "at";
  return index < edit.index ? "before" : "after";
}

/** {@link UnitsFunnel} of one run from its hook log and rank log. */
export function unitsFunnel(context: PairContext, hookLog: readonly ReplayHookLogEntry[], rankings: readonly RankEntry[], started: number): UnitsFunnel {
  const edits = firstEdits(hookLog);
  const byId = new Map((context.units ?? []).map(unit => [unit.id, unit]));
  const seconds = (at: number) => Math.round((at - started) / 1000);
  const delivered = hookLog.flatMap((entry, index) => entry.delivered.map(item => {
    const unit = byId.get(item.unitId);
    const edit = edits.get(item.area);
    return {
      id: item.unitId, ref: unit?.ref ?? null, pr: unit?.pr ?? null, branch: unit?.branch ?? null, author: unit?.author ?? "", kind: unit?.kind ?? "", origin: unit?.origin ?? null, text: unit?.text ?? "",
      area: item.area, path: item.path, rank: item.rank ?? null, conflict: item.conflict, fromFirst: unit !== undefined && fromFirst(unit, context.pair),
      atS: seconds(entry.at), firstEditS: edit === undefined ? null : seconds(edit.at), timing: editTiming(index, edit),
    };
  }));
  const firstUnits = (context.units ?? []).filter(unit => fromFirst(unit, context.pair)).map(unit => {
    const offered = rankings.filter(ranking => ranking.candidates.some(candidate => candidate.id === unit.id));
    const positions = offered.map(ranking => ranking.candidates.findIndex(candidate => candidate.id === unit.id));
    const sent = delivered.find(item => item.id === unit.id);
    return {
      id: unit.id, kind: unit.kind, origin: unit.origin ?? null, text: unit.text, files: [...unit.anchors.files], conflict: unit.conflict === true,
      candidateIn: offered.map(ranking => ranking.area), candidateRank: positions.length === 0 ? null : Math.min(...positions),
      delivered: sent !== undefined, rank: sent?.rank ?? null, timing: sent?.timing ?? null,
    };
  });
  return { firstEdits: [...edits].map(([area, edit]) => ({ area, atS: seconds(edit.at) })), delivered, rankings, firstUnits };
}

/**
 * The other refs `message` names: first's and each `concurrent` pull request by number (`#12`, `PR 12`, `pull request
 * #12`, `pull/12`), and first's and each `concurrent` branch by its short name.
 */
export function namedRefs(message: string | null, pair: Pair): string[] {
  if (message === null) return [];
  const numbers = [pair.first.pr, ...pair.concurrent].filter((entry): entry is number => typeof entry === "number");
  const branches = [pair.first.ref, pair.first.branch, ...pair.concurrent.filter(entry => typeof entry === "string")].flatMap(ref => (ref === undefined || ref === null ? [] : [shortRef(ref)])).filter(branch => branch.length >= 3);
  const escape = (text: string) => text.replace(/[.*+?^${}()|[\]\\]/gu, "\\$&");
  const prs = [...new Set(numbers)].filter(number => new RegExp(`(?:#|\\bPR\\s*#?|\\bpull request\\s*#?|\\bpull/)${number}\\b`, "iu").test(message)).map(number => `#${number}`);
  return [...prs, ...[...new Set(branches)].filter(branch => new RegExp(escape(branch), "u").test(message))];
}

async function runJob(job: Job, options: Options, codexEnv: Readonly<Record<string, string>>): Promise<ReplayRun> {
  const { context, arm, trial } = job;
  const { pair, spec } = context;
  const folder = await realpath(await mkdtemp(join(tmpdir(), "rerail-ma-replay-")));
  const empty = { timedOut: false, exitCode: null, wallMs: 0, usage: null, finalMessage: null, noteLine: false, leaked: false, leaks: [], setup: null, diff: "", files: [], merge: null, mergeError: null, touches: { firstHalf: [], firstFiles: [] }, delivered: [], injections: 0, landed: null, hookErrors: [], hookLog: [], git: null, events: [], grade: null };
  const task = options.tamper ? TAMPER_TASKS[trial - 1]! : spec.task;
  try {
    const workspace = prepareWorkspace(job, options, folder, task);
    const started = Date.now();
    const outcome = await runCodex(workspace.repo, workspace.codexHome, workspace.configPath, task, agentOptions(options, codexEnv), join(folder, "rerail-home"));
    const events = readEvents(outcome.stdout);
    const hookLog = readHookLog(workspace.logPath);
    const injected = hookLog.flatMap(entry => (entry.text === undefined ? [] : [entry.text]));
    const transcript = transcriptText(workspace.codexHome);
    const leaks = runLeaks([outcome.stdout], transcript, workspace, codexEnv);
    const { diff, patch, files } = workspaceDiff(workspace);
    let merge: ReplayRun["merge"] = null;
    let mergeError: string | null = null;
    try {
      const [head, visible] = mergeResult(context, workspace, patch, [pair.first.head_sha, spec.first_visible]) as [MergeCheck, MergeCheck];
      merge = { head, visible };
    } catch (error) {
      mergeError = errorLine(error);
    }
    const changed = new Set(files.map(file => file.path));
    const grade = await gradeRun(context, diff, folder, options, workspace.denied, codexEnv);
    return {
      pair: pair.id,
      arm,
      trial,
      error: callFailure(outcome, events, options.timeoutMin),
      timedOut: outcome.timedOut,
      exitCode: outcome.exitCode,
      wallMs: outcome.wallMs,
      usage: events.usage,
      finalMessage: events.finalMessage,
      noteLine: /^\s*Note:.*decided differently/mu.test(events.finalMessage ?? ""),
      leaked: leaks.length > 0,
      leaks,
      setup: workspace.setup,
      diff,
      files,
      merge,
      mergeError,
      touches: { firstHalf: (pair.conflict?.first_half.files ?? []).filter(path => changed.has(path)), firstFiles: context.firstFiles.filter(path => changed.has(path)) },
      delivered: deliveredUnits(context, hookLog, started),
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
    else await removeFolder(folder);
  }
}

// ---------------------------------------------------------------------------------------------------------- completion

/** The reviewer's and the grader's prompt: the task, the adopted and the other design, the numbered items, `done` and the diff. */
export function itemsPrompt(label: CompletionLabel, diff: string): string {
  return [
    "You are a maintainer of this repository reviewing a code change made for the task below. Judge it only from the diff; there is nothing else to look at.",
    "",
    "## Task the change was made for",
    label.task,
    "",
    "## The design this project adopted",
    label.adopted,
    "",
    "## The design it did not adopt",
    label.rejected,
    "",
    "## Statements a change must satisfy to match the adopted design",
    ...label.conflict_items.map((item, index) => `${index + 1}. ${item}`),
    "",
    "## Done",
    "How to tell whether the task is reasonably done:",
    label.done,
    "",
    "## Diff (against the commit the work started from)",
    "```diff",
    diff,
    "```",
    "",
    "Answer with JSON only. `items`: one entry per numbered statement, in order, each judged on its own. `item`: its number. `status`: \"violated\" when the diff contradicts the statement, \"satisfied\" when the diff conforms to it, \"not_applicable\" when the diff does not touch what it is about. `reason`: one or two sentences. `lines`: for a violated statement, the diff lines it rests on, each the path and the quoted added or removed line; empty otherwise. `done`: \"yes\", \"partial\" or \"no\" by the done criteria.",
  ].join("\n");
}

/** A reviewer's or grader's judgment of each conflict item on one diff. */
export interface ItemGrade {
  readonly verdict: ItemsVerdict | null;
  /** Items judged violated; null without a verdict. */
  readonly violated: number | null;
  /**
   * `model` for a Codex call; `empty-diff` for a diff that changes nothing (every item not applicable, not done), with
   * no call; `same-diff` for the final grade of a run whose diff is the first pass's, which is reused.
   */
  readonly by: "model" | "empty-diff" | "same-diff" | null;
  readonly error: string | null;
  readonly strippedLines: number;
  readonly truncated: boolean;
  readonly usage: Usage | null;
  readonly wallMs: number;
}

/** `answer`'s items when they are exactly 1..`count`, each once, sorted; why not otherwise. */
export function checkItems(answer: unknown, count: number): { verdict: ItemsVerdict; error: null } | { verdict: null; error: string } {
  const parsed = itemsVerdictSchema.safeParse(answer);
  if (!parsed.success) return { verdict: null, error: `answer unreadable (${parsed.error.message}): ${clip(JSON.stringify(answer), 200)}` };
  const items = [...parsed.data.items].sort((left, right) => left.item - right.item);
  if (items.length !== count || items.some((item, index) => item.item !== index + 1)) return { verdict: null, error: `answer judges items ${items.map(item => item.item).join(",")}, not 1..${count}` };
  return { verdict: { ...parsed.data, items }, error: null };
}

/** One blind judgment of `diff` against the label's items: the arm's injected text stripped as for the replay grader. */
async function judgeItems(context: PairContext, diff: string, folder: string, name: string, options: Options, denied: readonly string[], codexEnv: Readonly<Record<string, string>>): Promise<ItemGrade> {
  const label = context.label!;
  const { text, stripped, truncated } = graderDiff(diff, context.pair, context.refs.map(side => side.author));
  const base = { strippedLines: stripped, truncated };
  if (text.trim() === "") {
    const items = label.conflict_items.map((_, index) => ({ item: index + 1, status: "not_applicable" as const, reason: "The run changed no file.", lines: [] }));
    return { ...base, verdict: { items, done: "no" }, violated: 0, by: "empty-diff", error: null, usage: null, wallMs: 0 };
  }
  const call = await judgeCall(folder, name, itemsPrompt(label, text), ITEMS_VERDICT_JSON_SCHEMA, options, denied, codexEnv);
  const checked = call.error === null ? checkItems(call.answer, label.conflict_items.length) : { verdict: null, error: call.error };
  return {
    ...base,
    verdict: checked.verdict,
    violated: checked.verdict === null ? null : checked.verdict.items.filter(item => item.status === "violated").length,
    by: checked.verdict === null ? null : "model",
    error: checked.verdict === null ? `${name}: ${checked.error}` : null,
    usage: call.usage,
    wallMs: call.wallMs,
  };
}

/** What the agent gets back after a review that found violated items: each item, the reviewer's reason and diff lines. */
export function reviewMessage(label: CompletionLabel, verdict: ItemsVerdict, round: number): string {
  const violated = verdict.items.filter(item => item.status === "violated");
  return [
    `A maintainer who knows the design this project adopted reviewed your change (review ${round} of at most ${MAX_REVIEW_ROUNDS}). It does not match that design on ${violated.length === 1 ? "this point" : `these ${violated.length} points`}:`,
    "",
    ...violated.flatMap((item, index) => [
      `${index + 1}. ${label.conflict_items[item.item - 1]}`,
      `   Why: ${item.reason}`,
      ...(item.lines.length === 0 ? [] : ["   Diff lines:", ...item.lines.map(line => `     ${line}`)]),
      "",
    ]),
    "Change your code so that each point holds, keep the rest of the task done, and finish as before.",
  ].join("\n");
}

/** `total` minus `earlier`, field by field: a resumed call's own tokens (Codex reports the thread's running total). */
function usageSince(total: Usage, earlier: Usage | null): Usage {
  if (earlier === null) return total;
  return { input: total.input - earlier.input, cached: total.cached - earlier.cached, output: total.output - earlier.output, cacheWrite: (total.cacheWrite ?? 0) - (earlier.cacheWrite ?? 0) };
}

/**
 * The agent's model requests from its session transcript: the growth of Codex's running `total_token_usage` at each
 * `token_count` event (a total that shrinks starts a new session file).
 */
export function transcriptRequests(transcript: string | null): Usage[] {
  const requests: Usage[] = [];
  let previous: Usage | null = null;
  for (const line of (transcript ?? "").split("\n")) {
    if (!line.includes("\"token_count\"")) continue;
    let entry: { payload?: { type?: string; info?: { total_token_usage?: Record<string, unknown> } | null } };
    try {
      entry = JSON.parse(line) as typeof entry;
    } catch {
      continue;
    }
    const total = entry.payload?.type === "token_count" ? entry.payload.info?.total_token_usage : undefined;
    if (total === undefined) continue;
    const count = (value: unknown) => (typeof value === "number" ? value : 0);
    const usage: Usage = { input: count(total.input_tokens), cached: count(total.cached_input_tokens), output: count(total.output_tokens), cacheWrite: count(total.cache_write_input_tokens) };
    const request = previous !== null && usage.input >= previous.input ? usageSince(usage, previous) : usage;
    previous = usage;
    if (request.input > 0 || request.output > 0) requests.push(request);
  }
  return requests;
}

/**
 * What the requests above {@link PRICES}' long-context threshold cost on top of the short-context rates every call's
 * usage is priced at: they are billed at long-context rates in full.
 */
function longContextSurcharge(requests: readonly Usage[]): number {
  return requests.filter(request => request.input > PRICES.longContextAbove).reduce((sum, request) => sum + usageCost(request, PRICES.long) - usageCost(request, PRICES.short), 0);
}

/** One Codex call of a completion run: the agent's phase 1 or a fix, a review, or a grade. */
export interface CompletionCall {
  readonly role: "agent" | "reviewer" | "grader";
  readonly name: string;
  readonly wallMs: number;
  /** The call's own tokens. */
  readonly usage: Usage | null;
  readonly usd: number;
  readonly error: string | null;
}

/** One review round: the review of the current diff, and when it found violated items, the message and the fix. */
export interface CompletionRound {
  readonly round: number;
  readonly review: ItemGrade;
  readonly message: string | null;
  readonly fix: CompletionCall | null;
  /** The diff after the fix; the next round reviews it. */
  readonly diffAfter: string | null;
}

/** One role's calls summed: wall time, tokens (total = input + output; uncached = input − cached) and USD. */
export interface RoleTotals {
  readonly calls: number;
  readonly wallMs: number;
  readonly usage: Usage;
  readonly total: number;
  readonly uncached: number;
  readonly usd: number;
}

function roleTotals(calls: readonly CompletionCall[], role: CompletionCall["role"], surcharge = 0): RoleTotals {
  const own = calls.filter(call => call.role === role);
  const usage = own.reduce<Usage>((sum, call) => (call.usage === null ? sum : { input: sum.input + call.usage.input, cached: sum.cached + call.usage.cached, output: sum.output + call.usage.output, cacheWrite: (sum.cacheWrite ?? 0) + (call.usage.cacheWrite ?? 0) }), { input: 0, cached: 0, output: 0, cacheWrite: 0 });
  return { calls: own.length, wallMs: own.reduce((sum, call) => sum + call.wallMs, 0), usage, total: usage.input + usage.output, uncached: usage.input - usage.cached, usd: own.reduce((sum, call) => sum + call.usd, 0) + surcharge };
}

/** A completion run (pair × arm × trial). */
export interface CompletionRun {
  /** The completion label's id. */
  readonly pair: string;
  readonly scenario: CompletionLabel["scenario"];
  readonly arm: ReplayArm;
  readonly trial: number;
  /** Why the run did not complete its protocol (a harness error, an agent call that failed, a review with no verdict). */
  readonly error: string | null;
  readonly leaked: boolean;
  readonly leaks: readonly Leak[];
  readonly setup: ReplayRun["setup"];
  /** The agent's PATH. */
  readonly shellPath: readonly string[];
  readonly threadId: string | null;
  /** The label's conflict items. */
  readonly items: number;
  readonly firstPass: { readonly diff: string; readonly files: ReplayRun["files"]; readonly finalMessage: string | null; readonly grade: ItemGrade | null };
  readonly rounds: readonly CompletionRound[];
  readonly final: { readonly diff: string; readonly files: ReplayRun["files"]; readonly finalMessage: string | null; readonly grade: ItemGrade | null };
  /** Items the grader judged violated after phase 1, and after the last round; null without a verdict. */
  readonly itemsAfterPhase1: number | null;
  readonly itemsAfterCompletion: number | null;
  /** The final grade judged no item violated: the run ended on the adopted design. */
  readonly adoptedAtEnd: boolean | null;
  /** Reviews that found violated items and sent the agent back. */
  readonly fixRounds: number;
  /** The final diff on `base_sha` merged with the adopted side's head (first's in D1, second's in D2). */
  readonly merge: { readonly adoptedHead: string; readonly adopted: MergeCheck } | null;
  readonly mergeError: string | null;
  readonly calls: readonly CompletionCall[];
  readonly agent: RoleTotals;
  readonly reviewer: RoleTotals;
  readonly grader: RoleTotals;
  /** Agent requests above the long-context threshold, billed at long-context rates. */
  readonly longContextRequests: number;
  /** From the workspace's start to the final grade. */
  readonly wallMs: number;
  readonly hookLog: readonly ReplayHookLogEntry[];
  readonly hookErrors: readonly string[];
  readonly delivered: ReplayRun["delivered"];
  /** The refs the final message names (`namedRefs`): the person was told about other work when there is one. */
  readonly namedRefs: readonly string[];
  readonly toldPerson: boolean;
  /** Under the ledger arms, the diagnostic funnel ({@link UnitsFunnel}); null otherwise. */
  readonly funnel: UnitsFunnel | null;
  readonly git: GitOutcome | null;
  /** Codex's `--json` events of every agent call, in order. */
  readonly events: readonly unknown[];
  readonly folder: string | null;
}

/**
 * One completion run. Phase 1: the agent does the task, as a replay run does. Then up to {@link MAX_REVIEW_ROUNDS}
 * rounds: the reviewer (a blind Codex call that knows the adopted design, the same for every arm) judges the current
 * diff against the label's items; when any is violated, the agent resumes its session (`codex exec resume`, the hook
 * still on) with {@link reviewMessage}. A round with no violated item ends the loop. Then a separate grading call
 * judges the final diff, and another the first pass's (the reported numbers): the final diff's grade is the first
 * pass's when the two diffs are the same. A failed agent call ends the loop; a failed phase 1 gets no review.
 */
async function runCompletionJob(job: Job, options: Options, codexEnv: Readonly<Record<string, string>>): Promise<CompletionRun> {
  const { context, arm, trial } = job;
  const { pair, spec } = context;
  const label = context.label!;
  const adoptedHead = (label.scenario === "D1" ? pair.first : pair.second).head_sha;
  const folder = await realpath(await mkdtemp(join(tmpdir(), "rerail-ma-replay-")));
  const started = Date.now();
  const calls: CompletionCall[] = [];
  const stdouts: string[] = [];
  const totals = (surcharge: number) => ({ agent: roleTotals(calls, "agent", surcharge), reviewer: roleTotals(calls, "reviewer"), grader: roleTotals(calls, "grader") });
  const empty = { diff: "", files: [], finalMessage: null, grade: null };
  const failed = (error: string): CompletionRun => ({
    pair: label.id, scenario: label.scenario, arm, trial, error, leaked: false, leaks: [], setup: null, shellPath: [], threadId: null, items: label.conflict_items.length,
    firstPass: empty, rounds: [], final: empty, itemsAfterPhase1: null, itemsAfterCompletion: null, adoptedAtEnd: null, fixRounds: 0, merge: null, mergeError: null,
    calls, ...totals(0), longContextRequests: 0, wallMs: Date.now() - started, hookLog: [], hookErrors: [], delivered: [], namedRefs: [], toldPerson: false, funnel: null, git: null, events: stdouts.flatMap(parseEvents), folder: options.keep ? folder : null,
  });
  try {
    const workspace = prepareWorkspace(job, options, folder, spec.task);
    const agent = agentOptions(options, codexEnv);
    let thread: Usage | null = null;
    const agentCall = async (name: string, prompt: string, resume?: string) => {
      const outcome = await runCodex(workspace.repo, workspace.codexHome, workspace.configPath, prompt, agent, join(folder, "rerail-home"), resume);
      stdouts.push(outcome.stdout);
      const events = readEvents(outcome.stdout);
      const usage = events.usage === null ? null : usageSince(events.usage, thread);
      if (events.usage !== null) thread = events.usage;
      const call: CompletionCall = { role: "agent", name, wallMs: outcome.wallMs, usage, usd: usageCost(usage), error: callFailure(outcome, events, options.timeoutMin) };
      calls.push(call);
      return { call, events };
    };
    const judge = async (role: "reviewer" | "grader", name: string, diff: string) => {
      const grade = await judgeItems(context, diff, folder, name, options, workspace.denied, codexEnv);
      if (grade.by !== "empty-diff") calls.push({ role, name, wallMs: grade.wallMs, usage: grade.usage, usd: usageCost(grade.usage), error: grade.error });
      return grade;
    };
    const phase1 = await agentCall("phase 1", spec.task);
    const threadId = phase1.events.threadId;
    let error = phase1.call.error === null ? null : `phase 1: ${phase1.call.error}`;
    const first = workspaceDiff(workspace);
    let current = first;
    let finalMessage = phase1.events.finalMessage;
    const rounds: CompletionRound[] = [];
    for (let round = 1; error === null && round <= MAX_REVIEW_ROUNDS; round++) {
      const review = await judge("reviewer", `review-${round}`, current.diff);
      const violated = review.verdict?.items.some(item => item.status === "violated") ?? false;
      if (review.verdict === null || !violated || threadId === null) {
        if (review.verdict === null) error = review.error;
        else if (violated) error = "phase 1 left no session to resume";
        rounds.push({ round, review, message: null, fix: null, diffAfter: null });
        break;
      }
      const message = reviewMessage(label, review.verdict, round);
      const fix = await agentCall(`fix ${round}`, message, threadId);
      current = workspaceDiff(workspace);
      finalMessage = fix.events.finalMessage ?? finalMessage;
      rounds.push({ round, review, message, fix: fix.call, diffAfter: current.diff });
      if (fix.call.error !== null) error = `fix ${round}: ${fix.call.error}`;
    }
    const firstGrade = await judge("grader", "grade-phase1", first.diff);
    const finalGrade = current.diff === first.diff && firstGrade.verdict !== null
      ? { ...firstGrade, by: firstGrade.by === "model" ? "same-diff" as const : firstGrade.by, usage: null, wallMs: 0 }
      : await judge("grader", "grade-final", current.diff);
    let merge: CompletionRun["merge"] = null;
    let mergeError: string | null = null;
    try {
      if (!hasCommit(context.source, adoptedHead)) throw new Error(`${context.source} has no commit ${adoptedHead} (the adopted side's head)`);
      merge = { adoptedHead, adopted: mergeResult(context, workspace, current.patch, [adoptedHead])[0]! };
    } catch (caught) {
      mergeError = errorLine(caught);
    }
    const transcript = transcriptText(workspace.codexHome);
    const leaks = runLeaks(stdouts, transcript, workspace, codexEnv);
    const requests = transcriptRequests(transcript);
    const hookLog = readHookLog(workspace.logPath);
    const events = stdouts.flatMap(parseEvents);
    return {
      pair: label.id,
      scenario: label.scenario,
      arm,
      trial,
      error,
      leaked: leaks.length > 0,
      leaks,
      setup: workspace.setup,
      shellPath: (workspace.shell.PATH ?? "").split(":"),
      threadId,
      items: label.conflict_items.length,
      firstPass: { diff: first.diff, files: first.files, finalMessage: phase1.events.finalMessage, grade: firstGrade },
      rounds,
      final: { diff: current.diff, files: current.files, finalMessage, grade: finalGrade },
      itemsAfterPhase1: firstGrade.violated,
      itemsAfterCompletion: finalGrade.violated,
      adoptedAtEnd: finalGrade.violated === null ? null : finalGrade.violated === 0,
      fixRounds: rounds.filter(round => round.fix !== null).length,
      merge,
      mergeError,
      calls,
      ...totals(longContextSurcharge(requests)),
      longContextRequests: requests.filter(request => request.input > PRICES.longContextAbove).length,
      wallMs: Date.now() - started,
      hookLog,
      hookErrors: hookLog.flatMap(entry => (entry.error === undefined ? [] : [entry.error])),
      delivered: deliveredUnits(context, hookLog, started),
      namedRefs: namedRefs(finalMessage, pair),
      toldPerson: namedRefs(finalMessage, pair).length > 0,
      funnel: LEDGER_ARMS.includes(arm) ? unitsFunnel(context, hookLog, readRankLog(folder), started) : null,
      git: GIT_ARMS.includes(arm) ? gitOutcome(context, hookLog, events, started) : null,
      events,
      folder: options.keep ? folder : null,
    };
  } catch (error) {
    return failed(error instanceof Error ? error.message : String(error));
  } finally {
    if (options.keep) console.error(`kept ${folder}`);
    else await removeFolder(folder);
  }
}

const usd = (value: number): string => `$${value.toFixed(value < 1 ? 3 : 2)}`;
const kilo = (value: number): string => `${Math.round(value / 1000)}k`;

function printPrices(): void {
  const row = (name: string, prices: TokenPrices) => [name, String(prices.input), String(prices.cacheWrite), String(prices.cacheRead), String(prices.output)];
  console.log(`\nPrices: ${PRICES.model}, ${PRICES.tier}, ${PRICES.currency}\n${table(["context", "input", "cache write", "cache read", "output"], [row(`≤ ${PRICES.longContextAbove} input`, PRICES.short), row(`> ${PRICES.longContextAbove} input`, PRICES.long)])}\nSource: ${PRICES.source}`);
}

/** What the hook sent a ledger-arm run: each injected text verbatim, each area's ranking, each unit delivered. */
function printDeliveries(run: CompletionRun, funnel: UnitsFunnel): void {
  const atOf = (id: string | undefined) => funnel.delivered.find(item => item.id === id)?.atS ?? "?";
  for (const entry of run.hookLog.filter(item => item.text !== undefined && item.delivered.length > 0)) {
    console.log(`  injected at ${atOf(entry.delivered[0]?.unitId)}s (${entry.event} ${entry.tool}):\n${entry.text!.split("\n").map(line => `    | ${line}`).join("\n")}`);
  }
  for (const ranking of funnel.rankings) console.log(`  ranking ${ranking.area || "(root)"} at ${ranking.path}: ${ranking.anchored} anchored, ${ranking.candidates.length} offered, via ${ranking.via}${ranking.error === null ? "" : ` (${clip(ranking.error, 80)})`}, picked ${ranking.picked.length}`);
  for (const item of funnel.delivered) {
    const source = item.pr === null ? `branch ${item.branch ?? "?"}` : `#${item.pr}`;
    console.log(`  unit ${item.id} [${item.kind}/${item.origin ?? "?"}] ${source}${item.fromFirst ? " (first)" : ""}, rank ${item.rank ?? "?"} in ${item.area || "(root)"}, at ${item.atS}s: ${item.timing} the area's first edit${item.firstEditS === null ? "" : ` (${item.firstEditS}s)`}`);
  }
}

/** A ledger-arm run's funnel: its deliveries, first's units in the ledger and how far each got, the final message's refs. */
function printFunnel(run: CompletionRun): void {
  const funnel = run.funnel!;
  printDeliveries(run, funnel);
  const first = funnel.firstUnits;
  console.log(`  first's ${first.length} ledger units: ${first.filter(unit => unit.candidateIn.length > 0).length} candidates where the agent touched, ${first.filter(unit => unit.delivered).length} delivered, ${first.filter(unit => unit.timing === "before").length} before the area's first edit`);
  for (const unit of first) {
    const reach = unit.delivered ? "D" : unit.candidateIn.length > 0 ? "c" : "-";
    console.log(`    ${reach} ${unit.id} [${unit.kind}/${unit.origin ?? "?"}]${unit.candidateRank === null ? "" : ` candidate #${unit.candidateRank}`}: ${clip(unit.text, 110)}`);
  }
  console.log(`  told the person: ${run.namedRefs.length === 0 ? "no other PR or branch named in the final message" : run.namedRefs.join(", ")}`);
}

function printCompletionRuns(runs: readonly CompletionRun[]): void {
  const rows = runs.map(run => [
    run.pair,
    run.scenario,
    run.arm,
    String(run.trial),
    `${run.itemsAfterPhase1 ?? "?"}/${run.items}`,
    run.rounds.map(round => (round.review.violated === null ? "err" : String(round.review.violated))).join(" → ") || "-",
    `${run.itemsAfterCompletion ?? "?"}/${run.items}`,
    run.adoptedAtEnd === null ? "?" : run.adoptedAtEnd ? "yes" : "no",
    run.final.grade?.verdict?.done ?? "",
    String(run.fixRounds),
    fixed(run.agent.wallMs / 60_000, 1),
    fixed((run.reviewer.wallMs + run.grader.wallMs) / 60_000, 1),
    `${kilo(run.agent.total)}/${kilo(run.agent.uncached)}`,
    usd(run.agent.usd),
    `${kilo(run.reviewer.total)}/${kilo(run.reviewer.uncached)}`,
    usd(run.reviewer.usd),
    `${kilo(run.grader.total)}/${kilo(run.grader.uncached)}`,
    usd(run.grader.usd),
    run.merge === null ? "err" : `${run.merge.adopted.files.length}/${run.merge.adopted.hunks}`,
    run.leaked ? `LEAK (${run.leaks.length})` : "",
    run.funnel === null ? "-" : `${run.funnel.delivered.length} (${run.funnel.delivered.filter(item => item.fromFirst).length} first)`,
    run.namedRefs.join(" ") || "no",
    clip(run.error ?? run.mergeError, 40),
  ]);
  console.log(`\n${table(["pair", "scen", "arm", "trial", "items p1", "review items", "items end", "adopted", "done", "fixes", "agent min", "judge min", "agent tok/uncached", "agent $", "reviewer tok", "reviewer $", "grader tok", "grader $", "merge adopted f/h", "leaked", "units", "told", "error"], rows)}`);
  for (const run of runs) {
    console.log(`\n${run.pair} ${run.arm} #${run.trial}: first pass ${run.itemsAfterPhase1 ?? "?"} of ${run.items} items violated, ${run.fixRounds} fix round(s), end ${run.itemsAfterCompletion ?? "?"}${run.final.grade?.by === "same-diff" ? " (same diff as the first pass)" : ""}; ${(run.wallMs / 60_000).toFixed(1)} min in all`);
    for (const call of run.calls) console.log(`  - ${call.role} ${call.name}: ${(call.wallMs / 60_000).toFixed(1)} min, ${call.usage === null ? "no usage" : `${kilo(call.usage.input + call.usage.output)} tokens (${kilo(call.usage.input - call.usage.cached)} uncached input, ${kilo(call.usage.output)} output)`}, ${usd(call.usd)}${call.error === null ? "" : `, ${clip(call.error, 80)}`}`);
    for (const round of run.rounds) {
      const items = round.review.verdict?.items.filter(item => item.status === "violated").map(item => item.item) ?? [];
      console.log(`  review ${round.round}: ${round.review.violated === null ? `no verdict (${clip(round.review.error, 80)})` : `${round.review.violated} violated${items.length === 0 ? "" : ` (items ${items.join(", ")})`}`}${round.fix === null ? "" : `; fixed in ${(round.fix.wallMs / 60_000).toFixed(1)} min`}`);
    }
    for (const [name, grade] of [["first pass", run.firstPass.grade], ["final", run.final.grade]] as const) {
      for (const item of grade?.verdict?.items.filter(entry => entry.status === "violated") ?? []) console.log(`  ${name} item ${item.item} violated: ${clip(item.reason, 200)}`);
    }
    if (run.funnel !== null) printFunnel(run);
  }
  for (const run of runs.filter(candidate => candidate.leaked)) {
    console.log(`\n${run.pair} ${run.arm} #${run.trial} LEAKED, left out of every aggregate:`);
    for (const leak of run.leaks) console.log(`  - ${leak.reason} [${leak.source}]: ${clip(leak.text, 200)}`);
  }
}

/** A completion run that failed its protocol: an error, an empty final diff, or no final verdict. */
export const completionFailed = (run: CompletionRun): boolean => run.error !== null || run.final.diff.trim() === "" || run.itemsAfterCompletion === null;

function completionAggregate(label: readonly string[], all: readonly CompletionRun[]): string[] {
  const runs = all.filter(run => !run.leaked);
  const known = (values: ReadonlyArray<number | null>) => values.filter((value): value is number => value !== null);
  return [
    ...label,
    String(runs.length),
    String(all.length - runs.length),
    `${runs.filter(completionFailed).length}/${runs.length}`,
    fixed(mean(known(runs.map(run => run.itemsAfterPhase1))), 2),
    fixed(mean(known(runs.map(run => run.itemsAfterCompletion))), 2),
    `${runs.filter(run => run.adoptedAtEnd === true).length}/${runs.length}`,
    fixed(mean(runs.map(run => run.fixRounds)), 2),
    fixed(median(runs.map(run => run.agent.wallMs / 60_000)), 1),
    runs.length === 0 ? "n/a" : `${kilo(mean(runs.map(run => run.agent.total)))}/${kilo(mean(runs.map(run => run.agent.uncached)))}`,
    runs.length === 0 ? "n/a" : usd(mean(runs.map(run => run.agent.usd))),
    runs.length === 0 ? "n/a" : usd(mean(runs.map(run => run.reviewer.usd))),
    runs.length === 0 ? "n/a" : usd(mean(runs.map(run => run.grader.usd))),
    `${runs.filter(run => run.merge !== null && !run.merge.adopted.clean).length}/${runs.length}`,
  ];
}

const COMPLETION_HEADERS = ["runs", "leaked", "failed", "items p1", "items end", "adopted", "fixes", "median agent min", "agent tok/uncached", "agent $", "reviewer $", "grader $", "merge conflicted"];

function printCompletionAggregates(runs: readonly CompletionRun[], arms: readonly ReplayArm[]): void {
  console.log(`\nPer arm (means per run; leaked runs left out)\n${table(["arm", ...COMPLETION_HEADERS], arms.map(arm => completionAggregate([arm], runs.filter(run => run.arm === arm))))}`);
  const scenarios = [...new Set(runs.map(run => run.scenario))].sort();
  console.log(`\nPer scenario × arm\n${table(["scenario", "arm", ...COMPLETION_HEADERS], scenarios.flatMap(scenario => arms.map(arm => completionAggregate([scenario, arm], runs.filter(run => run.scenario === scenario && run.arm === arm)))))}`);
  const pairs = [...new Set(runs.map(run => run.pair))];
  console.log(`\nPer pair × arm\n${table(["pair", "arm", ...COMPLETION_HEADERS], pairs.flatMap(pair => arms.map(arm => completionAggregate([pair, arm], runs.filter(run => run.pair === pair && run.arm === arm)))))}`);
}

export const COMPLETION_REPORT_FORMAT = "rerail.merge-assist-replay-completion/v1";

async function completionMain(options: Options): Promise<void> {
  const selected = loadCompletionDataset(options);
  if (selected.length === 0) throw new Error(`No completion label in ${join(options.dataset, "completion")}`);
  const codexEnv = codexEnvironment(options);
  const align = options.arms.some(arm => LEDGER_ARMS.includes(arm)) ? await loadAlignModule(options.alignModule) : null;
  const contexts: PairContext[] = [];
  for (const { pair, spec, label } of selected) contexts.push(await preparePair(pair, spec, options, align, codexEnv, label));
  printLedgers(contexts);
  printPrices();
  const jobs = contexts.flatMap(context => Array.from({ length: options.trials }, (_, trial) => options.arms.map(arm => ({ context, arm, trial: trial + 1 }))).flat());
  console.error(`\n${jobs.length} completion runs (${contexts.length} labels × ${options.arms.join(",")} × ${options.trials} trials), ${options.concurrency} at a time, ${options.timeoutMin} min per agent call, at most ${MAX_REVIEW_ROUNDS} review rounds`);
  const startedAt = new Date().toISOString();
  const runs = await pool(jobs, options.concurrency, job => runCompletionJob(job, options, codexEnv), (done, run) => {
    console.error(`[${done}/${jobs.length}] ${run.pair} ${run.arm} #${run.trial}: ${fixed(run.wallMs / 60_000, 1)} min, items ${run.itemsAfterPhase1 ?? "?"} → ${run.itemsAfterCompletion ?? "?"} of ${run.items} after ${run.fixRounds} fix round(s), agent ${usd(run.agent.usd)}${run.error === null ? "" : `, ${clip(run.error, 80)}`}`);
  });
  printCompletionRuns(runs);
  printCompletionAggregates(runs, options.arms);
  if (options.json !== undefined) {
    const report = {
      format: COMPLETION_REPORT_FORMAT,
      startedAt,
      options,
      maxReviewRounds: MAX_REVIEW_ROUNDS,
      prices: PRICES,
      pairs: contexts.map(({ pair, spec, label, source, ledgerTime: time, refs, units, ledgerStats, ledgerError }) => ({ id: label!.id, label, pair, spec, source, ledgerTime: time, refs: refs.map(({ files, ...side }) => ({ ...side, files: Object.keys(files) })), ledger: units, ledgerStats, ledgerError })),
      runs,
    };
    writeFileSync(options.json, `${JSON.stringify(report, null, 2)}\n`);
    console.error(`wrote ${options.json}`);
  }
}

// ---------------------------------------------------------------------------------------------------------- ledger builds

export const LEDGER_BUILD_FORMAT = "rerail.merge-assist-ledger-build/v1";

interface BuiltLedger {
  readonly id: string;
  readonly pair: string;
  readonly repo: string;
  readonly units: number;
  readonly ms: number;
  readonly stats: LedgerStats | null;
  readonly error: string | null;
}

const judgeTotal = (stats: LedgerStats | null, field: "called" | "cached" | "failed"): number => Object.values(stats?.judge ?? {}).reduce((sum, counts) => sum + counts[field], 0);

/** `--build-ledgers`' table: per ledger, the extraction funnel (refs, commits, passages, descriptions), units by kind and judge calls; then totals. */
function printBuilt(built: readonly BuiltLedger[]): void {
  const kinds = [...new Set(built.flatMap(entry => Object.keys(entry.stats?.units ?? {})))].sort();
  const row = (entry: BuiltLedger): string[] => {
    const stats = entry.stats;
    if (stats === null) return [entry.id, entry.error === null ? "no stats" : clip(entry.error, 60), ...kinds.map(() => ""), "", "", "", "", "", "", "", ""];
    return [
      entry.id, `${stats.refs.listed}/${stats.refs.resolved}/${stats.refs.visible}/${stats.refs.kept}`, String(stats.commits),
      `${stats.passages.message}/${stats.passages.doc}/${stats.passages.description}`, `${stats.descriptions.found}/${stats.descriptions.visible}/${stats.descriptions.read}`,
      String(entry.units), ...kinds.map(kind => String(stats.units[kind] ?? 0)), String(stats.contested),
      `${judgeTotal(stats, "called")}/${judgeTotal(stats, "cached")}/${judgeTotal(stats, "failed")}`, String(stats.errors.length), fixed(entry.ms / 60_000, 1),
    ];
  };
  console.log(table(["ledger", "refs listed/resolved/visible/kept", "commits", "passages msg/doc/desc", "PRs found/open/read", "units", ...kinds, "contested", "judge called/cached/failed", "errors", "min"], built.map(row)));
  const sum = (pick: (stats: LedgerStats) => number) => built.reduce((total, entry) => total + (entry.stats === null ? 0 : pick(entry.stats)), 0);
  console.log(`\nTotals over ${built.length} ledgers (${built.filter(entry => entry.error !== null).length} failed): refs kept ${sum(stats => stats.refs.kept)} of ${sum(stats => stats.refs.visible)} visible (${sum(stats => stats.refs.dropped.length)} dropped by the area pre-filter); ${sum(stats => stats.commits)} commits; passages ${sum(stats => stats.passages.message)} messages, ${sum(stats => stats.passages.doc)} doc chunks, ${sum(stats => stats.passages.description)} description chunks; units ${built.reduce((total, entry) => total + entry.units, 0)} (${kinds.map(kind => `${kind} ${sum(stats => stats.units[kind] ?? 0)}`).join(", ")}); judge calls ${built.reduce((total, entry) => total + judgeTotal(entry.stats, "called"), 0)} made, ${built.reduce((total, entry) => total + judgeTotal(entry.stats, "cached"), 0)} answered from the cache, ${built.reduce((total, entry) => total + judgeTotal(entry.stats, "failed"), 0)} failed`);
  const byKind = new Map<string, { called: number; cached: number; failed: number }>();
  for (const counts of built.flatMap(entry => Object.entries(entry.stats?.judge ?? {}))) {
    const [kind, value] = counts;
    const total = byKind.get(kind) ?? { called: 0, cached: 0, failed: 0 };
    byKind.set(kind, { called: total.called + value.called, cached: total.cached + value.cached, failed: total.failed + value.failed });
  }
  console.log(`Judge calls by kind: ${[...byKind].map(([kind, value]) => `${kind} ${value.called} made / ${value.cached} cached / ${value.failed} failed`).join("; ")}`);
}

/**
 * `--build-ledgers`: no agent. Each selected pair's ledger (with `--completion`, each label's, at its base) as visible
 * at second's start, built one after another (the judge's `--judge-concurrency` calls in flight) and stored under
 * `<ledger-cache>/ledgers/<id>.json`, where later runs read it; then the table of what each build read and asked.
 * Only counts are printed: no unit text.
 */
async function buildLedgers(options: Options): Promise<void> {
  const selected = options.completion
    ? loadCompletionDataset(options).map(({ pair, spec, label }) => ({ id: label.id, pair, spec }))
    : loadDataset(options).map(({ pair, spec }) => ({ id: pair.id, pair, spec }));
  const align = await loadAlignModule(options.alignModule);
  const codexEnv = codexEnvironment(options);
  console.error(`Building ${selected.length} ledgers into ${join(options.ledgerCache, "ledgers")}, ${options.judgeConcurrency} judge calls in flight, descriptions ${options.prBodies ?? "off"}`);
  const built: BuiltLedger[] = [];
  for (const { id, pair, spec } of selected) {
    const source = options.repo ?? join(options.repos, pair.repo.replace("/", "__"));
    try {
      const ledger = await pairLedger(align, source, pair, id, ledgerTime(pair, spec.base_sha), options, codexEnv);
      built.push({ id, pair: pair.id, repo: pair.repo, units: ledger.units.length, ms: ledger.ms, stats: ledger.stats as LedgerStats | null, error: null });
    } catch (error) {
      built.push({ id, pair: pair.id, repo: pair.repo, units: 0, ms: 0, stats: null, error: errorLine(error) });
    }
    const last = built.at(-1)!;
    console.error(`[${built.length}/${selected.length}] ${id}: ${last.error ?? `${last.units} units, ${judgeTotal(last.stats, "called")} judge calls, ${fixed(last.ms / 60_000, 1)} min`}`);
  }
  printBuilt(built);
  if (options.json !== undefined) {
    writeFileSync(options.json, `${JSON.stringify({ format: LEDGER_BUILD_FORMAT, options, ledgers: built }, null, 2)}\n`);
    console.error(`wrote ${options.json}`);
  }
  if (built.some(entry => entry.error !== null)) process.exitCode = 1;
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

/** The files each commit changed against its only parent (none for a merge): one batched git call. */
function commitFiles(repo: string, shas: readonly string[]): Map<string, string[]> {
  const files = new Map<string, string[]>(shas.map(sha => [sha, []]));
  if (shas.length === 0) return files;
  let current: string[] | undefined;
  for (const line of git(repo, ["diff-tree", "--stdin", "-r", "--root", "--name-only", "--no-renames"], {}, `${shas.join("\n")}\n`).split("\n")) {
    const next = /^[0-9a-f]{40}$/u.test(line) ? files.get(line) : undefined;
    if (next !== undefined) current = next;
    else if (line !== "" && current !== undefined) current.push(line);
  }
  return files;
}

/**
 * Second's real commits through the `git-conflict` identifier, in their labelled order at their authoring times.
 * Against each visible ref (first's last labelled commit visible then, each `concurrent` ref's head then), the agent's
 * side is second's own work so far ({@link ownBase} of the commit at its time, with the ref as the other side) on the
 * files its own non-merge commits changed, and the ref's side is the ref's own work then (with the commit as the
 * other side), carried to the lines of second's starting point. Each (ref, file) is recorded at its first
 * identification. A ref is compared again only on the files the commit changed, unless either side's start or the ref
 * moved.
 */
function detectPair(pair: Pair, source: string): DetectResult {
  const started = performance.now();
  const firstRef = sideRef(pair.first) ?? "first";
  const present = presentCommits(source, [...new Set([...pair.first.commits, ...pair.second.commits].map(commit => commit.sha))]);
  const firstCommits = pair.first.commits.filter(commit => present.has(commit.sha));
  const commits = pair.second.commits.filter(commit => present.has(commit.sha));
  const changed = commitFiles(source, commits.map(commit => commit.sha));
  const secondBranch = baseBranchRef(source, pair.second.base_ref);
  const touched = new Set<string>();
  const found = new Map<string, Detection>();
  const last = new Map<string, string>();
  for (const commit of commits) {
    const files = changed.get(commit.sha) ?? [];
    for (const path of files) touched.add(path);
    const own = new Set(files);
    const at = commit.authored_at === "" ? commit.visible_at : commit.authored_at;
    const firstSha = firstCommits.filter(item => Date.parse(item.visible_at) <= Date.parse(at)).at(-1)?.sha ?? null;
    for (const { ref, sha, branch, labelledBase } of visibleShas(source, pair, at, firstSha)) {
      // Each side's own work, the other side standing in for the agent's base: shared history is neither's.
      const mine = ownBase(source, commit.sha, at, secondBranch, sha, pair.second.base_sha);
      const theirs = ownBase(source, sha, at, branch, commit.sha, labelledBase);
      if (mine === "" || mine === commit.sha || theirs === "" || theirs === sha) continue;
      const state = `${sha}\0${theirs}\0${mine}`;
      const moved = last.get(ref) !== state;
      last.set(ref, state);
      const theirFiles = cached(`${source}\0files\0${theirs}\0${sha}`, () => refFiles(source, theirs, sha));
      const paths = theirFiles.filter(path => touched.has(path) && (moved || own.has(path)) && !found.has(conflictKey({ ref, file: path })));
      if (paths.length === 0) continue;
      const hunkKey = (path: string) => `${source}\0hunks\0${mine}\0${theirs}\0${sha}\0${path}`;
      const missing = paths.filter(path => !detectCache.has(hunkKey(path)));
      const read = missing.length === 0 ? {} : refHunks(source, mine, theirs, sha, missing);
      const theirHunks: Record<string, readonly RefHunk[]> = {};
      for (const path of paths) theirHunks[path] = cached(hunkKey(path), () => read[path] ?? []);
      const rangeKey = (path: string) => `${source}\0ours\0${mine}\0${commit.sha}\0${path}`;
      const rangesMissing = paths.filter(path => !detectCache.has(rangeKey(path)));
      const ours = rangesMissing.length === 0 ? new Map<string, LineRange[]>() : commitRanges(source, mine, commit.sha, rangesMissing);
      const agent = new Map(paths.map(path => [path, cached(rangeKey(path), () => ours.get(path) ?? [])]));
      for (const conflict of identify(agent, [{ ref, files: theirHunks }])) {
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
  codexOverride = options.codexBin;
  if (options.detectOnly) {
    detectOnly(options);
    return;
  }
  if (options.buildLedgers || options.completion) {
    await (options.buildLedgers ? buildLedgers(options) : completionMain(options));
    return;
  }
  const selected = loadDataset(options);
  if (selected.length === 0) throw new Error(`No pair in ${options.dataset} has a replay spec`);
  const codexEnv = codexEnvironment(options);
  const align = options.arms.some(arm => LEDGER_ARMS.includes(arm)) ? await loadAlignModule(options.alignModule) : null;
  const contexts: PairContext[] = [];
  for (const { pair, spec } of selected) contexts.push(await preparePair(pair, spec, options, align, codexEnv));
  printLedgers(contexts);
  const jobs = contexts.flatMap(context => Array.from({ length: options.trials }, (_, trial) => options.arms.map(arm => ({ context, arm, trial: trial + 1 }))).flat());
  console.error(`\n${jobs.length} runs (${contexts.length} pairs × ${options.arms.join(",")} × ${options.trials} trials), ${options.concurrency} at a time, ${options.timeoutMin} min each`);
  const runs = await pool(jobs, options.concurrency, job => runJob(job, options, codexEnv), (done, run) => {
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
