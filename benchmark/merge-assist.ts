/**
 * Merge Assist eval: does the product path hold up on realistic corrections and decisions, judged by the person's own
 * agent CLI? See docs/merge-assist-v1-build.md and benchmark/README.md.
 *
 *   node --import tsx benchmark/merge-assist.ts [--suites capture,transfer,compare,lookup] [--trials N]
 *     [--compare-trials N] [--concurrency N] [--cases id,id] [--pairs id,id] [--json out.json] [--verbose] [--keep]
 *
 * Every trial builds a fresh temporary workspace: a Git repository from the dataset's project (committed on `main` as
 * Jeremy Barnes), linked worktrees `jeremy` (`jbarnes/work`) and `tim` (`tbelanger/work`), and its own Rerail database,
 * both worktrees recorded, the repository set up with no packs and started, and Merge Assist turned on. The service is
 * `createMergeAssistService` with the judge it builds by default (`createAgentJudge`: the agent CLI on PATH, the
 * session's host first) and a `gh` stand-in that reports `not-github`, since the repository has no remote. Trials take
 * turns running Rerail's own code (see `EngineTurn`) and overlap only while the judge runs. `RERAIL_HOME` points at a
 * temporary folder for the whole run.
 *
 * - capture: a session (task, the agent's Write, the correction, its Edit) through `captureSession` in `jeremy`.
 * - transfer: each saved code rule committed on `jbarnes/work`, then each teammate file scanned from `tim`.
 * - scope: each saved decision's scope against the case's `appliesTo` and `notAppliesTo` paths.
 * - compare: tim's decision (untracked) against jeremy's (committed) through `scan`, read from the finding's tier.
 * - lookup: 1,000 generated scopes compiled with the engine's glob matcher, timed over 10,000 paths. No model.
 *
 * Capture, transfer and compare spend the person's model budget. Results are never committed.
 */
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { mkdtemp, realpath, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { performance } from "node:perf_hooks";
import { z } from "zod";
import {
  CANDIDATE_DIRECTORY,
  CANDIDATE_FORMAT,
  type Candidate,
  candidateFileName,
  candidateSchema,
  correctionFamilies,
  type Finding,
  globPattern,
  loadRuleCatalog,
  mergeAssistItems,
  mergeAssistJudgments,
  newCandidateId,
  openRerailDatabase,
  type RerailDatabase,
  recordCheckout,
  serializeCandidate,
  setUpRepository,
} from "../src/engine/index.js";
import {
  type AgentJudge,
  createAgentJudge,
  createMergeAssistService,
  type MergeAssistRuntime,
  type MergeAssistServiceOptions,
  type MergeAssistSession,
  type SessionEventRecord,
  setRepositoryStarted,
} from "../src/runtime/index.js";
import { fixturesPath } from "../test/support/oracles.js";

const DATASET = fixturesPath("host", "benchmark", "merge-assist", "cases.json");
const SUITES = ["capture", "transfer", "compare", "lookup"] as const;
type Suite = (typeof SUITES)[number];
const SESSION_HOST = "codex";
const JEREMY_BRANCH = "jbarnes/work";
const TIM_BRANCH = "tbelanger/work";
const LOOKUP_RULES = 1_000;
const LOOKUP_PATHS = 10_000;
const LOOKUP_AREAS = 40;

// ---------------------------------------------------------------------------------------------------------- dataset

const decisionSchema = z.object({ decision: z.string().min(1), scope: z.array(z.string()).min(1) }).strict();
const captureKinds = ["lasting-checkable", "lasting-guidance", "one-off", "duplicate"] as const;
type CaptureKind = (typeof captureKinds)[number];
const captureCaseSchema = z.object({
  id: z.string().regex(/^[a-z0-9-]+$/u),
  kind: z.enum(captureKinds),
  task: z.string().min(1),
  correction: z.string().min(1),
  file: z.string().min(1),
  existing: decisionSchema.optional(),
  before: z.string(),
  after: z.string(),
  teammate: z.array(z.object({
    path: z.string().min(1),
    content: z.string(),
    expect: z.enum(["caught", "passed"]),
    why: z.string().min(1),
  }).strict()).optional(),
  appliesTo: z.array(z.string()).optional(),
  notAppliesTo: z.array(z.string()).optional(),
}).strict();
type CaptureCase = z.infer<typeof captureCaseSchema>;
const compareLabels = ["contradicts", "compatible", "duplicate"] as const;
type CompareLabel = (typeof compareLabels)[number];
const comparePairSchema = z.object({
  id: z.string().regex(/^[a-z0-9-]+$/u),
  label: z.enum(compareLabels),
  mine: decisionSchema,
  theirs: decisionSchema,
  why: z.string().min(1),
}).strict();
type ComparePair = z.infer<typeof comparePairSchema>;
const datasetSchema = z.object({
  project: z.record(z.string(), z.string()),
  capture: z.array(captureCaseSchema),
  compare: z.array(comparePairSchema),
}).strict();
type Dataset = z.infer<typeof datasetSchema>;

// ---------------------------------------------------------------------------------------------------------- options

interface Options {
  suites: Set<Suite>;
  trials: number;
  compareTrials: number;
  concurrency: number;
  cases: string[] | null;
  pairs: string[] | null;
  json: string | undefined;
  verbose: boolean;
  keep: boolean;
}

const USAGE = "Usage: node --import tsx benchmark/merge-assist.ts [--suites capture,transfer,compare,lookup] [--trials N] [--compare-trials N] [--concurrency N] [--cases id,id] [--pairs id,id] [--json out.json] [--verbose] [--keep]";

function positive(flag: string, value: string | undefined): number {
  const number = Number(value);
  if (!Number.isInteger(number) || number < 1) throw new Error(`${flag} takes a positive integer\n${USAGE}`);
  return number;
}

function list(flag: string, value: string | undefined): string[] {
  const items = (value ?? "").split(",").map(item => item.trim()).filter(item => item !== "");
  if (items.length === 0) throw new Error(`${flag} takes a comma-separated list\n${USAGE}`);
  return items;
}

function parseOptions(argv: readonly string[]): Options {
  const options: Options = {
    suites: new Set(SUITES),
    trials: 3,
    compareTrials: 2,
    concurrency: 4,
    cases: null,
    pairs: null,
    json: undefined,
    verbose: false,
    keep: false,
  };
  for (let index = 0; index < argv.length; index++) {
    const flag = argv[index]!;
    const value = (): string | undefined => argv[++index];
    switch (flag) {
      case "--suites": {
        const suites = list(flag, value());
        const unknown = suites.filter(suite => !(SUITES as readonly string[]).includes(suite));
        if (unknown.length > 0) throw new Error(`Unknown suite ${unknown.join(", ")}\n${USAGE}`);
        options.suites = new Set(suites as Suite[]);
        break;
      }
      case "--trials": options.trials = positive(flag, value()); break;
      case "--compare-trials": options.compareTrials = positive(flag, value()); break;
      case "--concurrency": options.concurrency = positive(flag, value()); break;
      case "--cases": options.cases = list(flag, value()); break;
      case "--pairs": options.pairs = list(flag, value()); break;
      case "--json": options.json = value(); if (options.json === undefined) throw new Error(USAGE); break;
      case "--verbose": options.verbose = true; break;
      case "--keep": options.keep = true; break;
      case "--help": case "-h": console.log(USAGE); process.exit(0); break;
      default: throw new Error(`Unknown argument ${flag}\n${USAGE}`);
    }
  }
  if (options.suites.has("transfer") && !options.suites.has("capture")) {
    throw new Error("The transfer suite checks the code rules capture saves: run it together with capture.");
  }
  return options;
}

/** The capture cases and compare pairs to run: `--cases` names either, `--pairs` only pairs. Unknown ids stop the run. */
function selection(dataset: Dataset, options: Options): { cases: CaptureCase[]; pairs: ComparePair[] } {
  const caseIds = new Set(dataset.capture.map(entry => entry.id));
  const pairIds = new Set(dataset.compare.map(entry => entry.id));
  const unknown = [
    ...(options.cases ?? []).filter(id => !caseIds.has(id) && !pairIds.has(id)),
    ...(options.pairs ?? []).filter(id => !pairIds.has(id)),
  ];
  if (unknown.length > 0) throw new Error(`Unknown case or pair id: ${unknown.join(", ")}`);
  const cases = options.cases === null ? dataset.capture : dataset.capture.filter(entry => options.cases!.includes(entry.id));
  const pairFilter = options.pairs ?? options.cases;
  const pairs = pairFilter === null ? dataset.compare : dataset.compare.filter(entry => pairFilter.includes(entry.id));
  return { cases, pairs };
}

// ---------------------------------------------------------------------------------------------------------- workspace

/** Git for the workspace's own commands, isolated from the person's global and system configuration. */
const GIT_ENV: NodeJS.ProcessEnv = {
  ...process.env,
  GIT_CONFIG_NOSYSTEM: "1",
  GIT_CONFIG_GLOBAL: "/dev/null",
  GIT_TERMINAL_PROMPT: "0",
  GIT_AUTHOR_NAME: "Jeremy Barnes",
  GIT_AUTHOR_EMAIL: "jeremy@example.com",
  GIT_COMMITTER_NAME: "Jeremy Barnes",
  GIT_COMMITTER_EMAIL: "jeremy@example.com",
};

function git(cwd: string, ...args: string[]): string {
  return execFileSync("git", args, { cwd, env: GIT_ENV, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] }).trim();
}

function writeTree(root: string, files: Readonly<Record<string, string>>): void {
  for (const [path, content] of Object.entries(files)) {
    mkdirSync(dirname(join(root, path)), { recursive: true });
    writeFileSync(join(root, path), content);
  }
}

type GithubClient = NonNullable<MergeAssistServiceOptions["github"]>;

/** `gh` for a repository with no remote: nothing to read. A scan never asks it, since the key names no GitHub repository. */
const NOT_GITHUB: GithubClient = {
  readiness: async () => ({ state: "not-github", login: null, repository: null }),
  openPullRequests: async () => [],
  candidatesAt: async () => ({ candidates: [], problems: [], truncated: false }),
  authorPermission: async () => "unknown",
};

/** The end of the queue for the engine turn: resolved when the last taker gives it back. */
let turnQueue: Promise<void> = Promise.resolve();

/**
 * One trial at a time runs Rerail's own code: the native engine evaluates one rule check per process and answers any
 * other with WORKSPACE_BUSY, which capture and scan would record as a rule that does not compile. A trial gives the
 * turn back while it waits on the judge, so judge calls still overlap up to `--concurrency`.
 */
class EngineTurn {
  #release: (() => void) | undefined;

  async take(): Promise<void> {
    const previous = turnQueue;
    const { promise, resolve } = Promise.withResolvers<void>();
    turnQueue = promise;
    await previous;
    this.#release = resolve;
  }

  give(): void {
    this.#release?.();
    this.#release = undefined;
  }
}

/** The person's agent CLI judge, as the service builds it by default, giving up the engine turn while a call runs. */
function turnTakingJudge(inner: AgentJudge, turn: EngineTurn): AgentJudge {
  return {
    readiness: () => inner.readiness(),
    budget: repositoryKey => inner.budget(repositoryKey),
    judge: async task => {
      turn.give();
      try {
        return await inner.judge(task);
      } finally {
        await turn.take();
      }
    },
  };
}

/** A trial's workspace; it holds the engine turn from creation until `dispose`, except while the judge runs. */
interface Workspace {
  readonly jeremyRoot: string;
  readonly timRoot: string;
  readonly database: RerailDatabase;
  readonly service: MergeAssistRuntime;
  dispose(): Promise<void>;
}

async function createWorkspace(project: Readonly<Record<string, string>>, keep: boolean): Promise<Workspace> {
  const turn = new EngineTurn();
  await turn.take();
  const folder = await realpath(await mkdtemp(join(tmpdir(), "rerail-merge-assist-eval-")));
  let database: RerailDatabase | undefined;
  const dispose = async (): Promise<void> => {
    try {
      database?.$client.close();
      if (keep) console.error(`kept ${folder}`);
      else await rm(folder, { recursive: true, force: true });
    } finally {
      turn.give();
    }
  };
  try {
    const repo = join(folder, "repo");
    mkdirSync(repo);
    git(repo, "init", "-q", "-b", "main");
    git(repo, "config", "user.name", "Jeremy Barnes");
    git(repo, "config", "user.email", "jeremy@example.com");
    git(repo, "config", "commit.gpgsign", "false");
    writeTree(repo, project);
    git(repo, "add", "-A");
    git(repo, "commit", "-q", "-m", "Orders service");
    const jeremyRoot = join(folder, "jeremy");
    const timRoot = join(folder, "tim");
    git(repo, "worktree", "add", "-q", "-b", JEREMY_BRANCH, jeremyRoot, "main");
    git(repo, "worktree", "add", "-q", "-b", TIM_BRANCH, timRoot, "main");
    database = openRerailDatabase(join(folder, "rerail.sqlite"));
    const { repositoryKey } = recordCheckout(database, jeremyRoot);
    recordCheckout(database, timRoot);
    setUpRepository(database, repositoryKey, { packs: [] });
    setRepositoryStarted(database, repositoryKey, true);
    const service = createMergeAssistService({
      database,
      ruleCatalog: loadRuleCatalog(),
      github: NOT_GITHUB,
      judge: host => turnTakingJudge(createAgentJudge({ database: database!, ...(host === undefined ? {} : { preferredHost: host }) }), turn),
    });
    await service.setEnabled(jeremyRoot, true);
    return { jeremyRoot, timRoot, database, service, dispose };
  } catch (error) {
    await dispose();
    throw error;
  }
}

/** A decision as a note candidate (no code rule), the way a teammate's branch or an earlier capture carries it. */
function noteCandidate(decision: { decision: string; scope: string[] }, author: string): Candidate {
  return candidateSchema.parse({
    format: CANDIDATE_FORMAT,
    id: newCandidateId(decision.decision),
    decision: decision.decision,
    generality: "repository",
    scope: decision.scope,
    author,
    createdAt: new Date().toISOString().replace(/\.\d{3}Z$/u, "Z"),
    state: "active",
    supersedes: null,
  });
}

function writeCandidate(root: string, candidate: Candidate): string {
  const path = `${CANDIDATE_DIRECTORY}/${candidateFileName(candidate.id)}`;
  writeTree(root, { [path]: serializeCandidate(candidate) });
  return path;
}

interface JudgmentRow {
  readonly task: string;
  readonly runner: string;
  readonly resultJson: string;
  readonly createdAt: number;
}

function judgments(database: RerailDatabase): JudgmentRow[] {
  return database.select().from(mergeAssistJudgments).all()
    .map(({ task, runner, resultJson, createdAt }) => ({ task, runner, resultJson, createdAt }))
    .sort((left, right) => left.createdAt - right.createdAt);
}

function countByTask(rows: readonly JudgmentRow[]): Record<string, number> {
  const counts: Record<string, number> = {};
  for (const row of rows) counts[row.task] = (counts[row.task] ?? 0) + 1;
  return counts;
}

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

// ---------------------------------------------------------------------------------------------------------- capture

/** What a capture trial ended as: saved as a decision, or why not. */
type Classification = "lasting" | "one-off" | "duplicate" | "unjudged" | "filtered" | "not-saved";

interface TransferFile {
  readonly path: string;
  readonly expect: "caught" | "passed";
  readonly caught: boolean;
  readonly tier: Finding["tier"] | null;
  readonly violations: string[];
  readonly reason: string | null;
}

interface CaptureTrial {
  readonly caseId: string;
  readonly kind: CaptureKind;
  readonly trial: number;
  readonly filtered: boolean;
  readonly families: string[];
  readonly classification: Classification;
  readonly saved: number;
  readonly generality: string | null;
  readonly level: "code" | "note" | null;
  readonly decision: string | null;
  readonly scope: string[] | null;
  readonly rule: unknown;
  readonly ruleDropped: string | null;
  readonly unjudged: string[];
  readonly verdict: unknown;
  readonly judgeCalls: Record<string, number>;
  readonly judgeRunners: string[];
  readonly wallMs: number;
  readonly transfer?: { files: TransferFile[]; judgeCalls: number } | { error: string };
  readonly scopeCheck?: { applies: Array<{ path: string; matched: boolean }>; excludes: Array<{ path: string; matched: boolean }> };
  readonly error?: string;
}

/** A stored event as the session store returns it: the whole adapter event as its payload. */
function sessionRecords(sessionId: string, testCase: CaptureCase, jeremyRoot: string): SessionEventRecord[] {
  const filePath = join(jeremyRoot, testCase.file);
  const payloads: Array<Record<string, unknown>> = [
    { kind: "user-prompt", text: testCase.task },
    { kind: "call", toolCallId: "call-write", toolName: "Write", input: { file_path: filePath, content: testCase.before } },
    { kind: "result", toolCallId: "call-write", toolName: "Write", output: `File created successfully at: ${filePath}`, failed: false },
    { kind: "turn-end", text: `Added the change to ${testCase.file}.` },
    { kind: "user-prompt", text: testCase.correction },
    { kind: "call", toolCallId: "call-edit", toolName: "Edit", input: { file_path: filePath, old_string: testCase.before, new_string: testCase.after } },
    { kind: "result", toolCallId: "call-edit", toolName: "Edit", output: `The file ${filePath} has been updated.`, failed: false },
    { kind: "turn-end", text: `Updated ${testCase.file} as asked.` },
  ];
  const start = Date.now();
  return payloads.map((payload, index) => {
    const eventId = `${sessionId}:${index + 1}`;
    return {
      sequence: index + 1,
      sessionId,
      eventId,
      kind: payload.kind as string,
      observedAt: start + index,
      payload: { ...payload, eventId, sessionId },
    };
  });
}

/** The session store's `events` over fixed records: `after`/`before` exclusive, `before` reading the last `limit`. */
function eventSource(records: readonly SessionEventRecord[]): MergeAssistSession["events"] {
  return {
    events(sessionId, { after, before, limit = 500 }) {
      const matching = records.filter(record => record.sessionId === sessionId
        && (after === undefined || record.sequence > after)
        && (before === undefined || record.sequence < before));
      return before === undefined ? matching.slice(0, limit) : matching.slice(-limit);
    },
  };
}

const captureVerdictShape = z.object({ generality: z.string(), duplicateOf: z.string().nullable() }).passthrough();

function classify(filtered: boolean, saved: number, firstVerdict: JudgmentRow | undefined): Classification {
  if (!filtered) return "filtered";
  if (saved > 0) return "lasting";
  if (firstVerdict === undefined) return "unjudged";
  const parsed = captureVerdictShape.safeParse(JSON.parse(firstVerdict.resultJson));
  if (!parsed.success) return "unjudged";
  if (parsed.data.generality === "one-off") return "one-off";
  if (parsed.data.duplicateOf !== null) return "duplicate";
  return "not-saved";
}

async function captureTrial(dataset: Dataset, testCase: CaptureCase, trial: number, options: Options): Promise<CaptureTrial> {
  const families = correctionFamilies(testCase.correction);
  const base = { caseId: testCase.id, kind: testCase.kind, trial, filtered: families.length > 0, families };
  let workspace: Workspace | undefined;
  try {
    workspace = await createWorkspace(dataset.project, options.keep);
    const { jeremyRoot, database, service } = workspace;
    if (testCase.existing !== undefined) writeCandidate(jeremyRoot, noteCandidate(testCase.existing, "Jeremy Barnes"));
    // Validation scans the checkout as it is after the correction.
    writeTree(jeremyRoot, { [testCase.file]: testCase.after });
    const sessionId = `merge-assist-eval-${testCase.id}-${trial}`;
    const started = performance.now();
    const result = await service.captureSession({
      sessionId,
      host: SESSION_HOST,
      checkoutPath: jeremyRoot,
      events: eventSource(sessionRecords(sessionId, testCase, jeremyRoot)),
    });
    const wallMs = performance.now() - started;
    const rows = judgments(database);
    const firstVerdict = rows.find(row => row.task === "capture");
    const ownItems = database.select().from(mergeAssistItems).all().filter(item => item.origin === "own");
    const [candidate] = result.saved;
    const evidence = ownItems.find(item => item.candidateId === candidate?.id)?.evidence as { ruleDropped?: string | null } | undefined;
    const record: CaptureTrial = {
      ...base,
      classification: classify(base.filtered, result.saved.length, firstVerdict),
      saved: result.saved.length,
      generality: candidate?.generality ?? null,
      level: candidate === undefined ? null : candidate.rule !== undefined ? "code" : "note",
      decision: candidate?.decision ?? null,
      scope: candidate?.scope ?? null,
      rule: candidate?.rule ?? null,
      ruleDropped: evidence?.ruleDropped ?? null,
      unjudged: [...result.unjudged],
      verdict: firstVerdict === undefined ? null : JSON.parse(firstVerdict.resultJson),
      judgeCalls: countByTask(rows),
      judgeRunners: [...new Set(rows.map(row => row.runner))],
      wallMs,
    };
    const scopeCheck = candidate !== undefined && (testCase.appliesTo !== undefined || testCase.notAppliesTo !== undefined)
      ? checkScope(candidate.scope, testCase)
      : undefined;
    const transfer = options.suites.has("transfer") && candidate?.rule !== undefined && (testCase.teammate?.length ?? 0) > 0
      ? await transferTrial(workspace, testCase, candidate)
      : undefined;
    return { ...record, ...(scopeCheck === undefined ? {} : { scopeCheck }), ...(transfer === undefined ? {} : { transfer }) };
  } catch (error) {
    return {
      ...base,
      classification: "unjudged",
      saved: 0,
      generality: null,
      level: null,
      decision: null,
      scope: null,
      rule: null,
      ruleDropped: null,
      unjudged: [],
      verdict: null,
      judgeCalls: {},
      judgeRunners: [],
      wallMs: 0,
      error: errorText(error),
    };
  } finally {
    await workspace?.dispose();
  }
}

// ---------------------------------------------------------------------------------------------------------- transfer

/**
 * Commit the saved code rule and the corrected file on Jeremy's branch, then scan Tim's worktree once per teammate
 * file, written there (untracked, or over the project's file) and taken away again. Tim has no decisions of his own,
 * so no scan asks the judge.
 */
async function transferTrial(workspace: Workspace, testCase: CaptureCase, candidate: Candidate): Promise<{ files: TransferFile[]; judgeCalls: number } | { error: string }> {
  const { jeremyRoot, timRoot, database, service } = workspace;
  try {
    git(jeremyRoot, "add", "--", `${CANDIDATE_DIRECTORY}/${candidateFileName(candidate.id)}`, testCase.file);
    git(jeremyRoot, "commit", "-q", "-m", "Decision from a correction");
    recordCheckout(database, timRoot);
    const before = judgments(database).length;
    const files: TransferFile[] = [];
    for (const teammate of testCase.teammate ?? []) {
      const target = join(timRoot, teammate.path);
      const original = existsSync(target) ? readFileSync(target, "utf8") : null;
      writeTree(timRoot, { [teammate.path]: teammate.content });
      try {
        const status = await service.scan(timRoot);
        const finding = status.findings.find(entry => entry.candidate.id === candidate.id);
        const violations = finding?.evidence.violations ?? [];
        files.push({
          path: teammate.path,
          expect: teammate.expect,
          caught: finding?.tier === "review" && violations.some(violation => violation.path === teammate.path),
          tier: finding?.tier ?? null,
          violations: violations.map(violation => `${violation.path}:${violation.line}`),
          reason: finding?.reason ?? null,
        });
      } finally {
        if (original === null) rmSync(target, { force: true });
        else writeFileSync(target, original);
      }
    }
    return { files, judgeCalls: judgments(database).length - before };
  } catch (error) {
    return { error: errorText(error) };
  }
}

function checkScope(scope: readonly string[], testCase: CaptureCase): NonNullable<CaptureTrial["scopeCheck"]> {
  const matcher = globPattern(scope);
  return {
    applies: (testCase.appliesTo ?? []).map(path => ({ path, matched: matcher.test(path) })),
    excludes: (testCase.notAppliesTo ?? []).map(path => ({ path, matched: matcher.test(path) })),
  };
}

// ---------------------------------------------------------------------------------------------------------- compare

type CompareVerdict = CompareLabel | "unsure";

interface CompareTrial {
  readonly pairId: string;
  readonly label: CompareLabel;
  readonly trial: number;
  readonly verdict: CompareVerdict | null;
  readonly tier: Finding["tier"] | null;
  readonly reason: string | null;
  readonly judgment: { verdict: string; explanation: string } | null;
  /** The relation the judge answered, read from its recorded answer (the only trace of a duplicate). */
  readonly judgeRelation: string | null;
  readonly judgeCalls: Record<string, number>;
  readonly wallMs: number;
  readonly error?: string;
}

const compareAnswerShape = z.object({ items: z.array(z.object({ candidateId: z.string(), relation: z.string() }).passthrough()) }).passthrough();

function verdictOf(finding: Finding | undefined): CompareVerdict {
  if (finding === undefined) return "duplicate";
  if (finding.tier === "talk") return "contradicts";
  if (finding.tier === "absorb") return "compatible";
  return "unsure";
}

async function compareTrial(dataset: Dataset, pair: ComparePair, trial: number, options: Options): Promise<CompareTrial> {
  const base = { pairId: pair.id, label: pair.label, trial };
  let workspace: Workspace | undefined;
  try {
    workspace = await createWorkspace(dataset.project, options.keep);
    const { jeremyRoot, timRoot, database, service } = workspace;
    writeCandidate(timRoot, noteCandidate(pair.mine, "tbelanger"));
    const theirs = noteCandidate(pair.theirs, "jbarnes");
    git(jeremyRoot, "add", "--", writeCandidate(jeremyRoot, theirs));
    git(jeremyRoot, "commit", "-q", "-m", "Decision");
    const started = performance.now();
    const status = await service.scan(timRoot);
    const wallMs = performance.now() - started;
    const finding = status.findings.find(entry => entry.candidate.id === theirs.id);
    const rows = judgments(database);
    const answer = rows.find(row => row.task === "compare");
    const parsed = answer === undefined ? undefined : compareAnswerShape.safeParse(JSON.parse(answer.resultJson));
    const relation = parsed?.success === true ? parsed.data.items.find(item => item.candidateId === theirs.id)?.relation ?? null : null;
    return {
      ...base,
      verdict: verdictOf(finding),
      tier: finding?.tier ?? null,
      reason: finding?.reason ?? null,
      judgment: finding?.evidence.judgment ?? null,
      judgeRelation: relation,
      judgeCalls: countByTask(rows),
      wallMs,
    };
  } catch (error) {
    return { ...base, verdict: null, tier: null, reason: null, judgment: null, judgeRelation: null, judgeCalls: {}, wallMs: 0, error: errorText(error) };
  } finally {
    await workspace?.dispose();
  }
}

// ---------------------------------------------------------------------------------------------------------- lookup

interface LookupResult {
  readonly rules: number;
  readonly globs: number;
  readonly paths: number;
  readonly compileMs: number;
  readonly p50Ms: number;
  readonly p99Ms: number;
  readonly meanMatches: number;
}

/** A small seeded generator (mulberry32), so every run times the same rules and paths. */
function seeded(seed: number): () => number {
  let state = seed >>> 0;
  return () => {
    state = (state + 0x6d2b79f5) >>> 0;
    let value = state;
    value = Math.imul(value ^ (value >>> 15), value | 1);
    value ^= value + Math.imul(value ^ (value >>> 7), value | 61);
    return ((value ^ (value >>> 14)) >>> 0) / 4_294_967_296;
  };
}

const AREA_WORDS = [
  "api", "auth", "billing", "cart", "catalog", "checkout", "config", "db", "email", "events",
  "export", "feeds", "files", "flags", "geo", "i18n", "import", "inventory", "invoices", "jobs",
  "lib", "logging", "media", "metrics", "notifications", "orders", "payments", "pricing", "profiles", "queue",
  "refunds", "reports", "routes", "search", "sessions", "shipping", "storage", "tax", "ui", "users",
];
const FILE_WORDS = ["index", "client", "service", "repo", "handler", "model", "schema", "utils", "types", "hooks", "store", "view"];

function lookupSuite(): LookupResult {
  const random = seeded(0x5eed);
  const pick = <T>(items: readonly T[]): T => items[Math.floor(random() * items.length)]!;
  const areas = AREA_WORDS.slice(0, LOOKUP_AREAS);
  const glob = (): string => {
    const area = pick(areas);
    switch (Math.floor(random() * 6)) {
      case 0: return `src/${area}/**`;
      case 1: return "**/*.test.ts";
      case 2: return `src/${area}/${pick(FILE_WORDS)}.ts`;
      case 3: return `src/{${area},${pick(areas)}}/**`;
      case 4: return `src/${area}/**/*.{ts,tsx}`;
      default: return `test/${area}/**`;
    }
  };
  const scopes = Array.from({ length: LOOKUP_RULES }, () => Array.from({ length: 1 + Math.floor(random() * 3) }, glob));
  const compileStart = performance.now();
  const matchers = scopes.map(scope => globPattern(scope));
  const compileMs = performance.now() - compileStart;
  const path = (): string => {
    const area = pick(areas);
    const name = pick(FILE_WORDS);
    switch (Math.floor(random() * 5)) {
      case 0: return `src/${area}/${name}.ts`;
      case 1: return `src/${area}/${pick(FILE_WORDS)}/${name}.tsx`;
      case 2: return `src/${area}/${name}.test.ts`;
      case 3: return `test/${area}/${name}.test.ts`;
      default: return `scripts/${area}-${name}.ts`;
    }
  };
  const paths = Array.from({ length: LOOKUP_PATHS }, path);
  const lookup = (candidate: string): number => {
    let matches = 0;
    for (const matcher of matchers) if (matcher.test(candidate)) matches += 1;
    return matches;
  };
  // Warm the matchers up before timing, as a resident daemon's would be.
  for (const candidate of paths.slice(0, 1_000)) lookup(candidate);
  const times: number[] = [];
  let matched = 0;
  for (const candidate of paths) {
    const start = performance.now();
    matched += lookup(candidate);
    times.push(performance.now() - start);
  }
  times.sort((left, right) => left - right);
  return {
    rules: LOOKUP_RULES,
    globs: scopes.reduce((sum, scope) => sum + scope.length, 0),
    paths: LOOKUP_PATHS,
    compileMs,
    p50Ms: quantile(times, 0.5),
    p99Ms: quantile(times, 0.99),
    meanMatches: matched / LOOKUP_PATHS,
  };
}

// ---------------------------------------------------------------------------------------------------------- metrics

/** The value at `q` of an ascending list (nearest rank); NaN for an empty one. */
function quantile(sorted: readonly number[], q: number): number {
  if (sorted.length === 0) return Number.NaN;
  return sorted[Math.min(sorted.length - 1, Math.max(0, Math.ceil(q * sorted.length) - 1))]!;
}

function ascending(values: readonly number[]): number[] {
  return [...values].sort((left, right) => left - right);
}

interface Ratio {
  readonly hits: number;
  readonly total: number;
}

const ratio = (hits: number, total: number): Ratio => ({ hits, total });
const share = (value: Ratio): number => (value.total === 0 ? Number.NaN : value.hits / value.total);

interface Metric {
  readonly suite: string;
  readonly name: string;
  readonly value: string;
  readonly count: string;
  readonly threshold: string;
  readonly result: "PASS" | "FAIL" | "";
}

function rateMetric(suite: string, name: string, value: Ratio, threshold?: { op: ">=" | "<="; bound: number }): Metric {
  const rate = share(value);
  const result = threshold === undefined || Number.isNaN(rate)
    ? ""
    : (threshold.op === ">=" ? rate >= threshold.bound : rate <= threshold.bound) ? "PASS" : "FAIL";
  return {
    suite,
    name,
    value: Number.isNaN(rate) ? "n/a" : rate.toFixed(2),
    count: `${value.hits}/${value.total}`,
    threshold: threshold === undefined ? "" : `${threshold.op === ">=" ? "≥" : "≤"} ${threshold.bound}`,
    result,
  };
}

function infoMetric(suite: string, name: string, value: string, count = ""): Metric {
  return { suite, name, value, count, threshold: "", result: "" };
}

/** The share of groups (cases or pairs with two or more trials) whose every trial ended the same way. */
function stability<T>(trials: readonly T[], key: (trial: T) => string, outcome: (trial: T) => string): Ratio {
  const groups = new Map<string, string[]>();
  for (const trial of trials) groups.set(key(trial), [...(groups.get(key(trial)) ?? []), outcome(trial)]);
  const repeated = [...groups.values()].filter(outcomes => outcomes.length >= 2);
  return ratio(repeated.filter(outcomes => new Set(outcomes).size === 1).length, repeated.length);
}

/** A drop reason without its specifics (counts, paths), so equal causes count together. */
function dropBucket(reason: string): string {
  return reason
    .replace(/ \(after a repair round\)$/u, "")
    .replace(/; the repair round .*$/u, "")
    .replace(/: .*$/u, "")
    .replace(/matches \d+ (?:place|places) in the corrected repository.*$/u, "matches the corrected repository")
    .replace(/fails its case ".*"$/u, "fails one of its cases");
}

function sum(values: Iterable<number>): number {
  let total = 0;
  for (const value of values) total += value;
  return total;
}

const LASTING: readonly CaptureKind[] = ["lasting-checkable", "lasting-guidance"];

function captureMetrics(trials: readonly CaptureTrial[], cases: readonly CaptureCase[]): Metric[] {
  const metrics: Metric[] = [];
  const lastingCases = cases.filter(entry => LASTING.includes(entry.kind));
  metrics.push(rateMetric("capture", "Filter recall (lasting cases)", ratio(lastingCases.filter(entry => correctionFamilies(entry.correction).length > 0).length, lastingCases.length), { op: ">=", bound: 0.8 }));
  const ran = trials.filter(trial => trial.error === undefined);
  const ofKind = (...kinds: CaptureKind[]) => ran.filter(trial => kinds.includes(trial.kind));
  const saved = (list: readonly CaptureTrial[]) => ratio(list.filter(trial => trial.saved > 0).length, list.length);
  metrics.push(rateMetric("capture", "Lasting saved", saved(ofKind(...LASTING)), { op: ">=", bound: 0.8 }));
  for (const kind of LASTING) metrics.push(rateMetric("capture", `  saved: ${kind}`, saved(ofKind(kind))));
  metrics.push(rateMetric("capture", "One-off false-save", saved(ofKind("one-off")), { op: "<=", bound: 0.1 }));
  const duplicates = ofKind("duplicate");
  metrics.push(rateMetric("capture", "Duplicate suppression", ratio(duplicates.filter(trial => trial.saved === 0).length, duplicates.length), { op: ">=", bound: 0.8 }));
  const checkableSaved = ofKind("lasting-checkable").filter(trial => trial.saved > 0);
  metrics.push(rateMetric("capture", "Rule kept (checkable, of saved)", ratio(checkableSaved.filter(trial => trial.level === "code").length, checkableSaved.length), { op: ">=", bound: 0.6 }));
  const drops = new Map<string, number>();
  for (const trial of checkableSaved) {
    if (trial.ruleDropped !== null) drops.set(dropBucket(trial.ruleDropped), (drops.get(dropBucket(trial.ruleDropped)) ?? 0) + 1);
  }
  for (const [reason, count] of [...drops].sort((left, right) => right[1] - left[1])) metrics.push(infoMetric("capture", `  dropped: ${reason}`, String(count)));
  metrics.push(rateMetric("capture", "Stability (same classification)", stability(ran, trial => trial.caseId, trial => trial.classification), { op: ">=", bound: 0.8 }));
  const judged = ran.filter(trial => trial.filtered);
  const calls = ascending(judged.map(trial => sum(Object.values(trial.judgeCalls))));
  metrics.push(infoMetric("capture", "Judge calls per capture (median / max)", calls.length === 0 ? "n/a" : `${quantile(calls, 0.5)} / ${calls.at(-1)}`, `${calls.length} captures`));
  const byTask = new Map<string, number>();
  for (const trial of judged) for (const [task, count] of Object.entries(trial.judgeCalls)) byTask.set(task, (byTask.get(task) ?? 0) + count);
  if (byTask.size > 0) metrics.push(infoMetric("capture", "  judge calls by task", [...byTask].map(([task, count]) => `${task} ${count}`).join(", ")));
  const walls = ascending(judged.map(trial => trial.wallMs / 1000));
  metrics.push(infoMetric("capture", "Wall time p50 / p90 (s)", walls.length === 0 ? "n/a" : `${quantile(walls, 0.5).toFixed(1)} / ${quantile(walls, 0.9).toFixed(1)}`, `${walls.length} captures`));
  const errors = trials.length - ran.length;
  if (errors > 0) metrics.push(infoMetric("capture", "Trials that failed to run", String(errors)));
  return metrics;
}

function transferFiles(trials: readonly CaptureTrial[]): Array<{ caseId: string; file: TransferFile }> {
  return trials.flatMap(trial => (trial.transfer !== undefined && "files" in trial.transfer ? trial.transfer.files.map(file => ({ caseId: trial.caseId, file })) : []));
}

function transferMetrics(trials: readonly CaptureTrial[]): Metric[] {
  const files = transferFiles(trials);
  const recall = (list: typeof files) => ratio(list.filter(({ file }) => file.expect === "caught" && file.caught).length, list.filter(({ file }) => file.expect === "caught").length);
  const falsePositive = (list: typeof files) => ratio(list.filter(({ file }) => file.expect === "passed" && file.caught).length, list.filter(({ file }) => file.expect === "passed").length);
  const metrics: Metric[] = [
    rateMetric("transfer", "Transfer recall (caught files)", recall(files), { op: ">=", bound: 0.7 }),
    rateMetric("transfer", "Transfer false-positive (passed files)", falsePositive(files), { op: "<=", bound: 0.1 }),
  ];
  for (const caseId of [...new Set(files.map(entry => entry.caseId))]) {
    const own = files.filter(entry => entry.caseId === caseId);
    const caught = recall(own);
    const flagged = falsePositive(own);
    metrics.push(infoMetric("transfer", `  ${caseId}`, `recall ${Number.isNaN(share(caught)) ? "n/a" : share(caught).toFixed(2)}, fp ${Number.isNaN(share(flagged)) ? "n/a" : share(flagged).toFixed(2)}`, `${caught.hits}/${caught.total} caught, ${flagged.hits}/${flagged.total} flagged`));
  }
  const rules = trials.filter(trial => trial.transfer !== undefined);
  const judgeCalls = sum(rules.map(trial => (trial.transfer !== undefined && "files" in trial.transfer ? trial.transfer.judgeCalls : 0)));
  metrics.push({ suite: "transfer", name: "Judge calls during transfer scans", value: String(judgeCalls), count: `${rules.length} rules`, threshold: "= 0", result: rules.length === 0 ? "" : judgeCalls === 0 ? "PASS" : "FAIL" });
  const failed = rules.filter(trial => trial.transfer !== undefined && "error" in trial.transfer).length;
  if (failed > 0) metrics.push(infoMetric("transfer", "Rules whose transfer failed to run", String(failed)));
  return metrics;
}

function scopeMetrics(trials: readonly CaptureTrial[]): Metric[] {
  const checks = trials.flatMap(trial => (trial.scopeCheck === undefined ? [] : [trial.scopeCheck]));
  const applies = checks.flatMap(check => check.applies);
  const excludes = checks.flatMap(check => check.excludes);
  return [
    rateMetric("scope", "Scope coverage (appliesTo matched)", ratio(applies.filter(entry => entry.matched).length, applies.length), { op: ">=", bound: 0.8 }),
    rateMetric("scope", "Scope exclusion (notAppliesTo unmatched)", ratio(excludes.filter(entry => !entry.matched).length, excludes.length), { op: ">=", bound: 0.8 }),
  ];
}

const VERDICTS: readonly CompareVerdict[] = ["contradicts", "compatible", "duplicate", "unsure"];

function compareMetrics(trials: readonly CompareTrial[]): Metric[] {
  const ran = trials.filter(trial => trial.verdict !== null);
  const metrics: Metric[] = [];
  for (const label of compareLabels) {
    const row = ran.filter(trial => trial.label === label);
    if (row.length === 0) continue;
    metrics.push(infoMetric("compare", `  ${label} →`, VERDICTS.map(verdict => `${verdict} ${row.filter(trial => trial.verdict === verdict).length}`).join(", "), `${row.length} trials`));
  }
  const predicted = ran.filter(trial => trial.verdict === "contradicts");
  const actual = ran.filter(trial => trial.label === "contradicts");
  metrics.push(rateMetric("compare", "Contradiction precision", ratio(predicted.filter(trial => trial.label === "contradicts").length, predicted.length)));
  metrics.push(rateMetric("compare", "Contradiction recall", ratio(actual.filter(trial => trial.verdict === "contradicts").length, actual.length), { op: ">=", bound: 0.8 }));
  const compatible = ran.filter(trial => trial.label === "compatible");
  metrics.push(rateMetric("compare", "False contradictions (compatible)", ratio(compatible.filter(trial => trial.verdict === "contradicts").length, compatible.length), { op: "<=", bound: 0.1 }));
  const duplicates = ran.filter(trial => trial.label === "duplicate");
  metrics.push(rateMetric("compare", "Duplicate suppression (compare)", ratio(duplicates.filter(trial => trial.verdict === "duplicate").length, duplicates.length), { op: ">=", bound: 0.7 }));
  metrics.push(rateMetric("compare", "Stability (same verdict)", stability(ran, trial => trial.pairId, trial => trial.verdict!), { op: ">=", bound: 0.8 }));
  const walls = ascending(ran.map(trial => trial.wallMs / 1000));
  metrics.push(infoMetric("compare", "Scan wall time p50 / p90 (s)", walls.length === 0 ? "n/a" : `${quantile(walls, 0.5).toFixed(1)} / ${quantile(walls, 0.9).toFixed(1)}`, `${walls.length} scans`));
  const errors = trials.length - ran.length;
  if (errors > 0) metrics.push(infoMetric("compare", "Trials that failed to run", String(errors)));
  return metrics;
}

function lookupMetrics(lookup: LookupResult): Metric[] {
  return [
    infoMetric("lookup", "Lookup p50 (ms per path)", lookup.p50Ms.toFixed(4), `${lookup.rules} rules, ${lookup.globs} globs`),
    {
      suite: "lookup",
      name: "Lookup p99 (ms per path)",
      value: lookup.p99Ms.toFixed(4),
      count: `${lookup.paths} paths`,
      threshold: "< 1 ms",
      result: lookup.p99Ms < 1 ? "PASS" : "FAIL",
    },
    infoMetric("lookup", "  compile all rules (ms)", lookup.compileMs.toFixed(1), `${lookup.meanMatches.toFixed(1)} matching rules per path`),
  ];
}

// ---------------------------------------------------------------------------------------------------------- output

function table(headers: readonly string[], rows: ReadonlyArray<readonly string[]>): string {
  const widths = headers.map((header, column) => Math.max(header.length, ...rows.map(row => (row[column] ?? "").length)));
  const line = (cells: readonly string[]) => cells.map((cell, column) => cell.padEnd(widths[column]!)).join("  ").trimEnd();
  return [line(headers), line(widths.map(width => "-".repeat(width))), ...rows.map(line)].join("\n");
}

function clip(text: string | null, length: number): string {
  if (text === null) return "";
  const flat = text.replace(/\s+/gu, " ").trim();
  return flat.length > length ? `${flat.slice(0, length - 1)}…` : flat;
}

function printVerbose(captures: readonly CaptureTrial[], compares: readonly CompareTrial[]): void {
  if (captures.length > 0) {
    console.log("\nCapture trials");
    console.log(table(
      ["case", "kind", "#", "filter", "class", "level", "generality", "calls", "wall s", "scope", "decision / dropped / error"],
      captures.map(trial => [
        trial.caseId,
        trial.kind,
        String(trial.trial),
        trial.filtered ? "pass" : "skip",
        trial.classification,
        trial.level ?? "",
        trial.generality ?? "",
        String(sum(Object.values(trial.judgeCalls))),
        (trial.wallMs / 1000).toFixed(1),
        trial.scope?.join(" ") ?? "",
        clip(trial.error ?? [trial.decision, trial.ruleDropped === null ? null : `dropped: ${trial.ruleDropped}`, ...trial.unjudged].filter(text => text !== null).join(" | "), 140),
      ]),
    ));
    const transfers = captures.flatMap(trial => (trial.transfer === undefined
      ? []
      : "error" in trial.transfer
        ? [[trial.caseId, String(trial.trial), "", "", "", "", "", clip(trial.transfer.error, 100)]]
        : trial.transfer.files.map(file => [
          trial.caseId,
          String(trial.trial),
          file.path,
          file.expect,
          file.caught ? "caught" : "passed",
          file.expect === (file.caught ? "caught" : "passed") ? "ok" : "MISS",
          file.tier ?? "none",
          file.violations.join(" "),
        ])));
    if (transfers.length > 0) {
      console.log("\nTransfer files");
      console.log(table(["case", "#", "file", "expect", "scan", "", "tier", "violations / error"], transfers));
    }
    const scopes = captures.flatMap(trial => (trial.scopeCheck === undefined
      ? []
      : [
        ...trial.scopeCheck.applies.map(entry => [trial.caseId, String(trial.trial), entry.path, "applies", entry.matched ? "ok" : "MISS"]),
        ...trial.scopeCheck.excludes.map(entry => [trial.caseId, String(trial.trial), entry.path, "excluded", entry.matched ? "MISS" : "ok"]),
      ]));
    if (scopes.length > 0) {
      console.log("\nScope paths");
      console.log(table(["case", "#", "path", "expect", ""], scopes));
    }
  }
  if (compares.length > 0) {
    console.log("\nCompare trials");
    console.log(table(
      ["pair", "label", "#", "verdict", "", "judge", "wall s", "explanation / reason / error"],
      compares.map(trial => [
        trial.pairId,
        trial.label,
        String(trial.trial),
        trial.verdict ?? "error",
        trial.verdict === trial.label ? "ok" : "MISS",
        trial.judgeRelation ?? "",
        (trial.wallMs / 1000).toFixed(1),
        clip(trial.error ?? (trial.verdict === "unsure" ? trial.reason : trial.judgment?.explanation ?? null), 140),
      ]),
    ));
  }
}

// ---------------------------------------------------------------------------------------------------------- run

async function pool<T, R>(items: readonly T[], concurrency: number, work: (item: T) => Promise<R>, progress: (done: number, result: R) => void): Promise<R[]> {
  const results: R[] = new Array(items.length);
  let next = 0;
  let done = 0;
  const workers = Array.from({ length: Math.min(concurrency, items.length) }, async () => {
    while (next < items.length) {
      const index = next++;
      results[index] = await work(items[index]!);
      progress(++done, results[index]);
    }
  });
  await Promise.all(workers);
  return results;
}

async function main(): Promise<void> {
  const options = parseOptions(process.argv.slice(2));
  const dataset = datasetSchema.parse(JSON.parse(readFileSync(DATASET, "utf8")));
  const { cases, pairs } = selection(dataset, options);
  // A private Rerail home for the whole run: the person's ~/.rerail is never read or written.
  const home = await realpath(await mkdtemp(join(tmpdir(), "rerail-merge-assist-eval-home-")));
  process.env.RERAIL_HOME = home;
  const startedAt = new Date().toISOString();
  try {
    console.log(`Merge Assist eval: suites ${[...options.suites].join(",")}; ${cases.length} capture cases × ${options.trials}, ${pairs.length} compare pairs × ${options.compareTrials}; concurrency ${options.concurrency}`);
    let captures: CaptureTrial[] = [];
    let compares: CompareTrial[] = [];
    let lookup: LookupResult | undefined;
    if (options.suites.has("capture") && cases.length > 0) {
      const jobs = cases.flatMap(testCase => Array.from({ length: options.trials }, (_, trial) => ({ testCase, trial: trial + 1 })));
      captures = await pool(jobs, options.concurrency, job => captureTrial(dataset, job.testCase, job.trial, options), (done, trial) => {
        console.error(`capture ${done}/${jobs.length}: ${trial.caseId} #${trial.trial} ${trial.error === undefined ? `${trial.classification}${trial.level === null ? "" : ` (${trial.level})`}` : `error: ${trial.error}`}`);
      });
    }
    if (options.suites.has("compare") && pairs.length > 0) {
      const jobs = pairs.flatMap(pair => Array.from({ length: options.compareTrials }, (_, trial) => ({ pair, trial: trial + 1 })));
      compares = await pool(jobs, options.concurrency, job => compareTrial(dataset, job.pair, job.trial, options), (done, trial) => {
        console.error(`compare ${done}/${jobs.length}: ${trial.pairId} #${trial.trial} ${trial.verdict ?? `error: ${trial.error}`}`);
      });
    }
    if (options.suites.has("lookup")) lookup = lookupSuite();

    const metrics: Metric[] = [
      ...(options.suites.has("capture") ? captureMetrics(captures, cases) : []),
      ...(options.suites.has("transfer") ? [...transferMetrics(captures), ...scopeMetrics(captures)] : []),
      ...(options.suites.has("compare") ? compareMetrics(compares) : []),
      ...(lookup === undefined ? [] : lookupMetrics(lookup)),
    ];
    const runners = [...new Set([...captures.flatMap(trial => trial.judgeRunners)])];
    if (options.verbose) printVerbose(captures, compares);
    console.log(`\nSummary${runners.length === 0 ? "" : ` (judge: ${runners.join(", ")})`}`);
    console.log(table(["suite", "metric", "value", "n", "threshold", "result"], metrics.map(metric => [metric.suite, metric.name, metric.value, metric.count, metric.threshold, metric.result])));
    if (options.json !== undefined) {
      const report = {
        startedAt,
        options: { ...options, suites: [...options.suites] },
        dataset: { cases: cases.map(entry => entry.id), pairs: pairs.map(entry => entry.id) },
        capture: captures,
        compare: compares,
        lookup: lookup ?? null,
        summary: metrics,
      };
      await writeFile(options.json, `${JSON.stringify(report, null, 2)}\n`);
      console.log(`\nWrote ${options.json}`);
    }
  } finally {
    await rm(home, { recursive: true, force: true });
  }
}

await main();
