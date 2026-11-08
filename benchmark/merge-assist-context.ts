/**
 * Merge Assist context eval: when a real coding agent touches files that teammates' decisions cover, does injecting
 * those decisions into its loop make its code follow them more often, and which protocol buys the most of that for the
 * least chatter? See benchmark/README.md and benchmark/merge-assist-context-hook.ts (the protocol).
 *
 *   node --import tsx benchmark/merge-assist-context.ts --build-index out.json [--distractors N] [--concurrency N]
 *   node --import tsx benchmark/merge-assist-context.ts --index index.json --variants none,guard-signals,... --split dev|heldout|all
 *     [--tasks id,id] [--trials N] [--concurrency N] [--distractors N] [--json out.json] [--keep] [--verbose]
 *     [--model m] [-c key=value] [--list-variants] [--validate-oracles]
 *   node --import tsx benchmark/merge-assist-context.ts --index index.json --sweep control.json [--thresholds 0.3,0.35,...]
 *   node --import tsx benchmark/merge-assist-context.ts --index index.json --live [--trials N] [--decisions id,id] [--tasks id,id]
 *     [--concurrency N] [--timeout-min N] [--json out.json] [--keep]
 *   node --import tsx benchmark/merge-assist-context.ts --index index.json --amend out.json
 *
 * The decision index is built once, from the product's own code paths, never by hand: `--build-index` runs
 * `draftDecision` with the person's agent CLI judge on each real decision's capture case in cases.json (its correction,
 * before and after, in a temporary workspace of the project) and keeps the decision text, scope and validated signals
 * it produces; a decision the judge calls one-off, or does not answer, stays out of the index. Distractors and the
 * conflict tasks' teammate decisions, hand-written noise with no example, get their signals from `deriveSignals`. Each
 * entry records its provenance, and its decision text's embedding (the bundled model) is stored beside it. Every run
 * reads that file through `--index`, and an integrity check refuses an index that carries grader knowledge. A decision
 * changes the way the product changes one: `--amend` runs `draftDecision` on each amendment case in cases.json (the
 * decision's author's correction, with the decision among `ownDecisions`) and writes a new index whose entry for it is
 * the amendment the product drafted (a candidate superseding it), or nothing when the product drafts anything else.
 *
 * Each run (variant × task × trial) gets a fresh temporary folder:
 * - `repo/`: a Git repository with cases.json's project committed on `main` and `tbelanger/work` checked out;
 * - `codex-home/` as `CODEX_HOME`: `auth.json` a symlink to `~/.codex/auth.json` (never copied), an empty
 *   `config.toml`, and `hooks.json` pointing PreToolUse and PostToolUse (matcher "") at the eval hook;
 * - the hook's configuration (the variant, the run's decision index and embeddings), its log and its state.
 *
 * The person's own `~/.codex` configuration, hooks, MCP servers and sessions are never read or written, nor is
 * `~/.rerail`: `--build-index` points `RERAIL_HOME` at a temporary folder. Codex runs as `codex exec --json
 * --dangerously-bypass-hook-trust --sandbox workspace-write --skip-git-repo-check -C <repo> -` with the prompt on
 * stdin, its default model unless `--model` or `-c` say otherwise, for at most six minutes. The run is graded from
 * Codex's JSON events, the hook log, the transcript Codex writes under the temporary `CODEX_HOME`, and `git diff main`.
 * Every run, and every judge call of `--build-index`, spends the person's model budget.
 *
 * Before any run, every decision's oracle is checked against the labelled files in cases.json (the case's own
 * before and after, and its teammate files); a wrong oracle stops the run. Oracles grade through the dataset's own
 * scopes, never the index's.
 */
import { execFileSync, spawn } from "node:child_process";
import { existsSync, mkdirSync, readdirSync, readFileSync, symlinkSync, writeFileSync } from "node:fs";
import { mkdtemp, realpath, rm, writeFile } from "node:fs/promises";
import { homedir, tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { performance } from "node:perf_hooks";
import { setTimeout as sleep } from "node:timers/promises";
import { fileURLToPath } from "node:url";
import { z } from "zod";
import {
  type Candidate,
  CANDIDATE_FORMAT,
  candidateFileName,
  callSummary,
  type CheckoutContext,
  globPattern,
  type NativeSemanticEmbedder,
  openRerailDatabase,
  PACKAGE_ROOT,
  type RerailDatabase,
  recordCheckout,
  setUpRepository,
} from "../src/engine/index.js";
import { type AgentJudge, type CaptureInput, createAgentJudge, deriveSignals, draftDecision, type DraftOutcome, MAX_RANKED, rankPrompt, setRepositoryStarted } from "../src/runtime/index.js";
import { fixturesPath } from "../test/support/oracles.js";
import {
  advance,
  type AreaFocusLog,
  areaFocusPath,
  areaOf,
  BUDGETS,
  type Channel,
  CHANNELS,
  choose,
  CONFLICT,
  DEDUPE,
  EMBED_MAX_TOKENS,
  fires,
  FORMATS,
  type FocusFile,
  focusPath,
  type GuardScore,
  GUARDS,
  type GuardSource,
  HOOK_CONTEXT_CHARS,
  type HookConfig,
  type HookLogEntry,
  type IndexEntry,
  INITIAL_STATE,
  loadEmbedder,
  ONLY,
  ORIENTS,
  AREA_RANKS,
  RANKS,
  rankerInput,
  render,
  scopeMatcher,
  sessionFirst,
  type VariantParameters,
  WHEN,
} from "./merge-assist-context-hook.js";
import { LIVE_ARMS, liveAlpha, type LiveResult, liveTest } from "./merge-assist-stats.js";

const DATASET = fixturesPath("host", "benchmark", "merge-assist", "context.json");
const HOOK = fileURLToPath(new URL("./merge-assist-context-hook.ts", import.meta.url));
const REPOSITORY_TSCONFIG = fileURLToPath(new URL("../tsconfig.json", import.meta.url));
const AUTH = join(homedir(), ".codex", "auth.json");
const WORK_BRANCH = "tbelanger/work";
const OWN_AUTHOR = "Tim Belanger";
/** How long one Codex call (a run, or a fix round) may take unless `--timeout-min` says otherwise. */
const DEFAULT_TIMEOUT_MIN = 6;
/**
 * `dev` for tuning, `heldout` to confirm, `size` for the cost-to-clean comparison (`--arms`), `confirm` for the
 * pre-registered confirmatory `--arms` runs and `heldout-large` for the large-task one (both spent: their comparisons
 * are done, and they are tuning data now, like `size`), `heldout-large2` for the next confirmation on large tasks
 * (held out: no agent runs on it before its pre-registration).
 */
const SPLITS = ["dev", "heldout", "size", "confirm", "heldout-large", "heldout-large2"] as const;
/** The splits whose tasks name their size. */
const SIZED_SPLITS: ReadonlyArray<(typeof SPLITS)[number]> = ["size", "confirm", "heldout-large", "heldout-large2"];
export const SIZES = ["small", "medium", "large"] as const;
const KINDS = ["tempt", "neutral", "conflict"] as const;

// ---------------------------------------------------------------------------------------------------------- variants

/** One selection channel: everything but `hybrid`. */
const SINGLE = { channels: "single", orient: null, areaRank: null, guard: null, embedThreshold: null, guardCap: null, focusK: null, rankMax: null, areaCap: null, sessionRankMax: null, agreeAfter: null } as const;
/** Round one's protocol defaults: directories trigger, no budget per run, the agent is asked to tell the user. */
const LISTINGS = { ...SINGLE, dirs: true, sessionCap: null, conflict: "tell", budget: "session", threshold: null } as const;
/** Only files read or edited trigger, each new file at most two items, the conflict as a line to end with. */
const PER_FILE = { ...SINGLE, when: "post-read+pre-edit", cap: 2, dedupe: "session", only: "all", dirs: false, sessionCap: null, budget: "per-file", format: "card", conflict: "final-line" } as const;
/**
 * The embed guard's similarity threshold, frozen from `--sweep` over the dev split's control run and never tuned on
 * held-out data. Rule: the best recall at most 0.5 fires per non-violating patch. With the index's 30 distractors in
 * play (dev control, 44 patches, 6 violating) that is 0.8, where embed recall is 0/6: below it, noise exceeds the budget
 * (0.475 gave 5/6 recall at 0.68 fires). So the embed guard adds nothing within the noise budget on this index.
 */
export const EMBED_THRESHOLD: number | null = 0.8;
/** Orientation on the conflict pairs of a file first read or patched, plus the guard at each patch (3 at most). */
const GUARD = { ...PER_FILE, rank: "scope-specificity", threshold: null, channels: "hybrid", orient: "conflicts", guardCap: 3 } as const;
/** Everything upfront, one terse line each, conflict pairs first. */
const UPFRONT = { ...LISTINGS, when: "upfront", cap: null, dedupe: "session", format: "terse", rank: "conflict-first", only: "all" } as const;
/**
 * On the first touch of an area, every decision covering it, ranked by `areaRank` with conflict pairs first, at most 12
 * decisions, one terse line each, the conflict's line to end with last.
 */
const AREA = { ...PER_FILE, format: "terse", rank: "scope-specificity", threshold: null, channels: "hybrid", orient: "area", areaRank: "index", cap: 12, guardCap: 3, guard: null, embedThreshold: null } as const;
/** Per-area focus: an area's first touch starts a ranking call over its own decisions, which names at most 3. */
const FOCUS_AREA = { ...AREA, orient: "area-focus", areaRank: null, cap: null, guardCap: null, focusK: null, rankMax: 3, areaCap: 3 } as const;

/**
 * The protocols to compare, by name. `none` is the control: the hook runs and logs (each patch's guard scores
 * included), and injects nothing. A new variant is one more entry here.
 */
const VARIANTS: Readonly<Record<string, VariantParameters | null>> = {
  none: null,
  "upfront-all": UPFRONT,
  /** `upfront-all` with the conflict as the exact line to end the final message with. */
  "upfront-final": { ...UPFRONT, conflict: "final-line" },
  "guard-signals": { ...GUARD, guard: "signals", embedThreshold: null },
  "guard-embed": { ...GUARD, guard: "embed", embedThreshold: EMBED_THRESHOLD },
  "guard-both": { ...GUARD, guard: "both", embedThreshold: EMBED_THRESHOLD },
  /** Area orientation with each area's decisions in index order, grouped by source (team, then peer). */
  "area-index": AREA,
  /** Area orientation, the narrowest scopes first. */
  "area-specific": { ...AREA, areaRank: "specificity" },
  /** Area orientation, ranked by BM25 between the task prompt and each decision. */
  "area-lex": { ...AREA, areaRank: "prompt-lexical" },
  /** Area orientation, ranked by the cosine between the task prompt's embedding and each decision's. */
  "area-embed": { ...AREA, areaRank: "prompt-embed" },
  /** `area-index`, plus the signals guard at a patch for what orientation did not deliver. */
  "area-guard": { ...AREA, guard: "signals" },
  /** Session focus: at an area's first touch, its conflict pairs and its share of the ranker's top 3 picks for the task. */
  "focus-k3": { ...AREA, orient: "focus", areaRank: null, cap: null, guardCap: null, focusK: 3 },
  /** Session focus with the ranker's top 5 picks in play. */
  "focus-k5": { ...AREA, orient: "focus", areaRank: null, cap: null, guardCap: null, focusK: 5 },
  /** Session focus over a pool: the ranker names up to 12 picks, and an area's first touch takes at most 3 of them. */
  "focus-pool": { ...AREA, orient: "focus", areaRank: null, cap: null, guardCap: null, focusK: null, rankMax: 12, areaCap: 3 },
  "focus-area": FOCUS_AREA,
  /** Per-area focus with each area's ranking call naming at most 5, all 5 injected. */
  "focus-area5": { ...FOCUS_AREA, rankMax: 5, areaCap: 5 },
  /**
   * Per-area focus that agrees with session focus: the session ranker (12 picks) runs from the start, each area's
   * ranker (3 picks) lists the session's picks first, and an area gets its ranker's first pick plus whichever of its
   * other picks the session ranker picked too.
   */
  "focus-precise": { ...FOCUS_AREA, rankMax: 3, areaCap: 3, sessionRankMax: 12, agreeAfter: 1 },
  /** `focus-precise` without the agreement: every one of the area ranker's 3 picks goes out. */
  "focus-precise3": { ...FOCUS_AREA, rankMax: 3, areaCap: 3, sessionRankMax: 12, agreeAfter: null },
  /** At an area's first touch, its conflict pairs and every undelivered decision covering it: no ranking, no cap. */
  "area-all": { ...FOCUS_AREA, orient: "area-all", rankMax: null, areaCap: null },
  /**
   * DIAGNOSTIC ceiling, never a product arm: `area-all`'s delivery with the task's targets (the grader's labels) as
   * the picks, so it measures what perfect selection would deliver; no ranker runs.
   */
  "oracle-area": { ...FOCUS_AREA, orient: "targets", rankMax: null, areaCap: null },
};
const CONTROL = "none";

/** What `orient: targets` presets are: a ceiling that selects with the grader's labels. */
const DIAGNOSTIC_NOTE = "DIAGNOSTIC ceiling, never a product arm: its picks are the task's targets (the grader's labels), no ranker";

function describeVariant(parameters: VariantParameters | null): string {
  if (parameters === null) return "control: no injection (the hook only logs)";
  const embed = parameters.guard === "embed" || parameters.guard === "both" ? ` embedThreshold=${parameters.embedThreshold ?? "NOT FROZEN"}` : "";
  const focus = parameters.orient === "focus" || parameters.orient === "area-focus"
    ? ` focusK=${parameters.focusK ?? "all"} rankMax=${parameters.rankMax ?? MAX_RANKED} areaCap=${parameters.areaCap ?? "none"}${parameters.orient === "area-focus" && parameters.sessionRankMax !== null ? ` sessionRankMax=${parameters.sessionRankMax} (area rankers list its picks first) agreeAfter=${parameters.agreeAfter ?? "none"}` : ""}`
    : parameters.orient === "area-all" || parameters.orient === "targets" ? ` areaCap=none callChars=${HOOK_CONTEXT_CHARS} (the rest at the next call)` : "";
  const orient = `orient=${parameters.orient}${parameters.orient === "area" ? ` areaRank=${parameters.areaRank}` : ""}${focus}`;
  const channels = parameters.channels === "hybrid" ? ` channels=hybrid ${orient} guard=${parameters.guard ?? "none"} guardCap=${parameters.guardCap ?? "none"}${embed}` : "";
  const diagnostic = parameters.orient === "targets" ? `${DIAGNOSTIC_NOTE}. ` : "";
  return `${diagnostic}when=${parameters.when} budget=${parameters.budget} cap=${parameters.cap ?? "all"} sessionCap=${parameters.sessionCap ?? "all"} dirs=${parameters.dirs} dedupe=${parameters.dedupe} format=${parameters.format} rank=${parameters.rank} threshold=${parameters.threshold ?? "none"} only=${parameters.only} conflict=${parameters.conflict}${channels}`;
}

// ---------------------------------------------------------------------------------------------------------- dataset

const scopeSchema = z.object({ include: z.array(z.string().min(1)).min(1), exclude: z.array(z.string().min(1)) }).strict();
/**
 * A decision as the grader knows it: who made it and where (team or peer), plus the hand-written text and scope the
 * oracle grades through. The index never carries this text or scope: it holds what `draftDecision` produced instead.
 */
const entrySchema = z.object({
  id: z.string().regex(/^[a-z0-9-]+$/u),
  decision: z.string().min(1),
  scope: scopeSchema,
  source: z.enum(["team", "peer"]),
  author: z.string().min(1),
  branch: z.string().min(1).optional(),
  pr: z.number().int().positive().optional(),
}).strict().refine(entry => (entry.source === "team" ? entry.pr !== undefined : entry.branch !== undefined), "a team decision names its pull request, a peer decision its branch");
type DatasetEntry = z.infer<typeof entrySchema>;
const taskSchema = z.object({
  id: z.string().regex(/^[a-z0-9-]+$/u),
  split: z.enum(SPLITS),
  /** `size`, `confirm`, `heldout-large` and `heldout-large2` splits only: how big the task is. */
  size: z.enum(SIZES).optional(),
  kind: z.enum(KINDS),
  prompt: z.string().min(1),
  targets: z.array(z.string()),
  conflict: z.object({ own: z.string(), peer: entrySchema }).strict().optional(),
  done: z.array(z.object({ path: z.string().min(1), regex: z.string().min(1) }).strict()).min(1),
  expectedFiles: z.array(z.string().min(1)),
}).strict();
type Task = z.infer<typeof taskSchema>;
/** A plausible convention no task needs and no oracle grades; the harness gives it an author and a source. */
const distractorSchema = z.object({ id: z.string().regex(/^x-[a-z0-9-]+$/u), decision: z.string().min(1).max(200), scope: scopeSchema }).strict();
type Distractor = z.infer<typeof distractorSchema>;
const datasetSchema = z.object({
  project: z.string().regex(/^[^#]+#\/[\w/]+$/u),
  decisions: z.array(entrySchema).length(10),
  distractors: z.array(distractorSchema),
  tasks: z.array(taskSchema).min(1),
}).strict();

/** The labelled cases: each decision's capture case (correction, before, after) and its teammate files, from cases.json. */
const labelledCaseSchema = z.object({
  id: z.string(),
  correction: z.string(),
  file: z.string(),
  before: z.string(),
  after: z.string(),
  teammate: z.array(z.object({ path: z.string(), content: z.string(), expect: z.enum(["caught", "passed"]) }).passthrough()).optional(),
}).passthrough();

export interface Dataset {
  readonly project: Readonly<Record<string, string>>;
  readonly decisions: readonly DatasetEntry[];
  readonly distractors: readonly Distractor[];
  readonly tasks: readonly Task[];
  readonly labelled: ReadonlyArray<z.infer<typeof labelledCaseSchema>>;
}

/** A JSON pointer (`/a/b`) into `value`. */
function pointer(value: unknown, path: string): unknown {
  return path.split("/").slice(1).reduce<unknown>((node, key) => (node !== null && typeof node === "object" ? (node as Record<string, unknown>)[key] : undefined), value);
}

export function loadDataset(): Dataset {
  const raw = datasetSchema.parse(JSON.parse(readFileSync(DATASET, "utf8")));
  const [file, path] = raw.project.split("#") as [string, string];
  const source = JSON.parse(readFileSync(join(dirname(DATASET), file), "utf8")) as unknown;
  const project = z.record(z.string(), z.string()).parse(pointer(source, path));
  const labelled = z.array(labelledCaseSchema).parse(pointer(source, "/capture"));
  const ids = new Set(raw.decisions.map(entry => entry.id));
  const distractorIds = new Set(raw.distractors.map(entry => entry.id));
  if (distractorIds.size !== raw.distractors.length) throw new Error("Two distractors share an id");
  const taskIds = new Set<string>();
  for (const task of raw.tasks) {
    if (taskIds.has(task.id)) throw new Error(`Task ${task.id} appears twice`);
    taskIds.add(task.id);
    for (const target of task.targets) {
      if (!ids.has(target)) throw new Error(`Task ${task.id} targets ${target}, which is not a decision`);
      if (ORACLES[target] === undefined) throw new Error(`Task ${task.id} targets ${target}, which has no oracle`);
    }
    if ((task.kind === "neutral") !== (task.targets.length === 0)) throw new Error(`Task ${task.id}: a neutral task, and only one, has no targets`);
    if (SIZED_SPLITS.includes(task.split) !== (task.size !== undefined)) throw new Error(`Task ${task.id}: a task of ${SIZED_SPLITS.join(", ")}, and only one, names its size`);
    if ((task.kind === "conflict") !== (task.conflict !== undefined)) throw new Error(`Task ${task.id}: a conflict task, and only one, names its conflict`);
    if (task.conflict !== undefined && !task.targets.includes(task.conflict.own)) throw new Error(`Task ${task.id}: its own decision must be its target`);
    for (const done of task.done) new RegExp(done.regex, "u");
  }
  return { project, decisions: raw.decisions, distractors: raw.distractors, tasks: raw.tasks, labelled };
}

/** A small seeded generator (mulberry32), so every run with the same `--distractors` gets the same index. */
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

const DISTRACTOR_SEED = 2224;
/** How far apart the distractor draws of two `--seed` values start. */
const SEED_STRIDE = 7919;
/** The seed of the whole-index shuffle for `--seed` above 1. */
const SHUFFLE_SEED = 4242;
const DISTRACTOR_AUTHORS = ["Jeremy Barnes", "Priya Natarajan", "Sam Okafor", "Lena Fischer", "Ana Costa", "Wei Zhang", "Tomás Ruiz"];

/**
 * `count` distractors from the dataset's pool, the same ones in the same places for every run: alternately team
 * decisions (with a pull request) and teammates' branch decisions, each by one of the team. Their text and scope are
 * hand-written noise; their signals come from `deriveSignals` when the index is built.
 */
function distractorPlaces(dataset: Dataset, count: number, seed = 1): Array<Omit<BuiltEntry, "signals" | "provenance">> {
  if (count > dataset.distractors.length) throw new Error(`--distractors takes at most ${dataset.distractors.length}, the dataset's pool`);
  const random = seeded(DISTRACTOR_SEED + (seed - 1) * SEED_STRIDE);
  const pool = [...dataset.distractors];
  for (let index = pool.length - 1; index > 0; index--) {
    const other = Math.floor(random() * (index + 1));
    [pool[index], pool[other]] = [pool[other]!, pool[index]!];
  }
  return pool.slice(0, count).map((entry, index) => {
    const author = DISTRACTOR_AUTHORS[index % DISTRACTOR_AUTHORS.length]!;
    const handle = `${author[0]!.toLowerCase()}${author.split(" ").at(-1)!.toLowerCase()}`;
    const place = index % 2 === 0 ? { source: "team" as const, pr: 300 + Math.floor(random() * 120) } : { source: "peer" as const, branch: `${handle}/${entry.id.slice(2)}` };
    return { id: entry.id, decision: entry.decision, scope: entry.scope, author, ...place };
  });
}

// ---------------------------------------------------------------------------------------------------------- index

const INDEX_FORMAT = "rerail.merge-assist-context-index/v1";
/** What an index entry may hold: what the product produced, who decided it and where, and where it came from. */
const INDEX_FIELDS: Readonly<Record<string, true>> = { id: true, decision: true, scope: true, source: true, author: true, branch: true, pr: true, conflictsWith: true, signals: true, provenance: true };
/** The bundled embedding model the index's embeddings and the hook's blocks come from. */
export const MODEL_PATH = process.env.RERAIL_MODEL_PATH ?? join(PACKAGE_ROOT, "model");

const provenanceSchema = z.object({
  /** The product function that produced the entry's decision text, scope and signals (`draftDecision`) or its signals (`deriveSignals`). */
  by: z.enum(["draftDecision", "deriveSignals"]),
  /** The judge runners its judge calls went to. */
  runner: z.string(),
  at: z.string(),
  /** `draftDecision`: the capture case in cases.json it ran on (an amendment case for `--amend`), and what the candidate became. */
  case: z.string().optional(),
  candidateId: z.string().optional(),
  generality: z.string().optional(),
  level: z.enum(["code", "note"]).optional(),
  ruleDropped: z.string().nullable().optional(),
  /** `--live`: the entry was captured in the trial from agent A's session, not built from a cases.json case. */
  via: z.literal("live-capture").optional(),
  /** `--amend`: the candidate id of the decision this entry's amendment replaced (its `supersedes`). */
  amends: z.string().optional(),
}).strict();

const builtEntrySchema = z.object({
  id: z.string().regex(/^[a-z0-9-]+$/u),
  decision: z.string().min(1),
  scope: scopeSchema,
  source: z.enum(["team", "peer"]),
  author: z.string().min(1),
  branch: z.string().min(1).optional(),
  pr: z.number().int().positive().optional(),
  conflictsWith: z.string().optional(),
  signals: z.array(z.string()),
  provenance: provenanceSchema,
}).strict();
export type BuiltEntry = z.infer<typeof builtEntrySchema>;

const judgeCallSchema = z.object({
  entryId: z.string(),
  kind: z.string(),
  runner: z.string().nullable(),
  ok: z.boolean(),
  cached: z.boolean(),
  detail: z.string().nullable(),
  ms: z.number(),
  /** The whole prompt the judge saw: what the integrity check reads for teammate files. */
  prompt: z.string(),
}).strict();
type JudgeCall = z.infer<typeof judgeCallSchema>;

const builtIndexSchema = z.object({
  format: z.literal(INDEX_FORMAT),
  builtAt: z.string(),
  /** How many of the seeded distractors the index holds: the most `--distractors` can ask for. */
  distractors: z.number().int().nonnegative(),
  model: z.object({ fingerprint: z.unknown(), maxInputTokens: z.number().int().positive() }).strict(),
  /** Raw here: the integrity check reads every field before the entries are parsed. */
  entries: z.array(z.record(z.string(), z.unknown())),
  embeddings: z.record(z.string(), z.array(z.number())),
  /** Decisions the product did not produce (one-off, a duplicate, no answer, or signals it could not write), with why. */
  absent: z.array(z.object({ id: z.string(), outcome: z.string(), detail: z.string() }).strict()),
  judgeCalls: z.array(judgeCallSchema),
}).strict();

export interface BuiltIndex {
  readonly path: string;
  readonly builtAt: string;
  readonly distractors: number;
  readonly entries: readonly BuiltEntry[];
  readonly embeddings: Readonly<Record<string, readonly number[]>>;
  readonly absent: ReadonlyArray<{ id: string; outcome: string; detail: string }>;
  readonly judgeCalls: readonly JudgeCall[];
}

/** An index entry as the hook gets it: without its provenance. */
export function hookEntry(entry: BuiltEntry): IndexEntry {
  const { id, decision, scope, source, author, branch, pr, conflictsWith, signals } = entry;
  return {
    id,
    decision,
    scope,
    source,
    author,
    signals,
    ...(branch === undefined ? {} : { branch }),
    ...(pr === undefined ? {} : { pr }),
    ...(conflictsWith === undefined ? {} : { conflictsWith }),
  };
}

/** Literal runs of a regular expression's source, at least five characters: the code text an oracle looks for. */
function regexFragments(source: string): string[] {
  const fragments: string[] = [];
  let current = "";
  const flush = () => {
    if (current.trim().length >= 5) fragments.push(current.trim());
    current = "";
  };
  for (let index = 0; index < source.length; index++) {
    const char = source[index]!;
    if (char === "\\") {
      const next = source[++index] ?? "";
      if (next === "s" && "*?".includes(source[index + 1] ?? "")) {
        // Optional whitespace: code usually has none there, so the literal run ends.
        flush();
        index++;
      } else if (next === "s") {
        current += " ";
        if (source[index + 1] === "+") index++;
      } else if (/[A-Za-z0-9]/u.test(next)) {
        flush();
      } else {
        current += next;
      }
    } else if (char === "[") {
      flush();
      while (index < source.length && source[index] !== "]") index += source[index] === "\\" ? 2 : 1;
    } else if (char === "(" && source[index + 1] === "?") {
      // `(?:`, `(?=`, `(?!` open three characters; `(?<=`, `(?<!` four.
      flush();
      index += source[index + 2] === "<" ? 3 : 2;
    } else if ("^$|?*+(){}.".includes(char)) {
      // A quantifier makes the character before it optional: it does not belong to the literal run.
      if ("?*{".includes(char)) current = current.slice(0, -1);
      flush();
    } else {
      current += char;
    }
  }
  flush();
  return fragments;
}

export interface IntegrityReport {
  /** Grader knowledge in the index: a field the product does not produce, no product provenance, or an oracle's regex. */
  readonly failures: readonly string[];
  /** Code text an oracle looks for that the product's own output holds: reported, not refused. */
  readonly overlaps: ReadonlyArray<{ readonly id: string; readonly field: "decision" | "signal"; readonly text: string; readonly fragment: string; readonly oracles: readonly string[]; readonly by: string }>;
}

/**
 * Whether the index carries grader knowledge. An entry fails for a field outside {@link INDEX_FIELDS}, for provenance
 * that is not a product function, or for an oracle's regex source inside its text or signals. Literal code an oracle
 * looks for (`console.`, `fetch`) inside the product's own output is legitimate (the correction said it) and is
 * reported as an overlap.
 */
export function checkIntegrity(entries: ReadonlyArray<Record<string, unknown>>): IntegrityReport {
  const failures: string[] = [];
  const overlaps: Array<IntegrityReport["overlaps"][number]> = [];
  const sources = Object.entries(ORACLES).flatMap(([id, oracle]) => oracle.patterns.map(pattern => ({ id, source: pattern.source })));
  const fragments = new Map<string, Set<string>>();
  for (const { id, source } of sources) for (const fragment of regexFragments(source)) fragments.set(fragment, (fragments.get(fragment) ?? new Set()).add(id));
  for (const entry of entries) {
    const id = typeof entry.id === "string" ? entry.id : JSON.stringify(entry.id);
    for (const field of Object.keys(entry)) if (INDEX_FIELDS[field] !== true) failures.push(`${id}: field "${field}" is not something the product produces`);
    const provenance = provenanceSchema.safeParse(entry.provenance);
    if (!provenance.success) failures.push(`${id}: no product provenance (draftDecision or deriveSignals)`);
    const texts = [
      { field: "decision" as const, text: typeof entry.decision === "string" ? entry.decision : "" },
      ...(Array.isArray(entry.signals) ? entry.signals.map(signal => ({ field: "signal" as const, text: String(signal) })) : []),
    ];
    for (const { field, text } of texts) {
      for (const { id: oracle, source } of sources) if (text.includes(source)) failures.push(`${id}: ${field} holds oracle ${oracle}'s regex ${source}`);
      for (const [fragment, oracles] of fragments) {
        if (text.includes(fragment)) overlaps.push({ id, field, text, fragment, oracles: [...oracles], by: provenance.success ? provenance.data.by : "no provenance" });
      }
    }
  }
  return { failures, overlaps };
}

/**
 * Teammate file lines a judge prompt holds: lines of 20 or more characters from a labelled teammate file that occur in
 * no project file and no capture case's correction, before or after (not even inside a longer line), so they can only
 * have come from the teammate file itself.
 */
export function teammateLeaks(dataset: Dataset, calls: readonly JudgeCall[]): string[] {
  const known = [...Object.values(dataset.project), ...dataset.labelled.flatMap(entry => [entry.correction, entry.before, entry.after])].join("\n");
  const leaks: string[] = [];
  for (const entry of dataset.labelled) {
    for (const file of entry.teammate ?? []) {
      const lines = file.content.split("\n").map(line => line.trim()).filter(line => line.length >= 20 && !known.includes(line));
      for (const call of calls) {
        const line = lines.find(candidate => call.prompt.includes(candidate));
        if (line !== undefined) leaks.push(`${call.kind} call for ${call.entryId} holds ${entry.id}'s teammate ${file.path}: ${line}`);
      }
    }
  }
  return leaks;
}

/**
 * What a focus ranker's prompt for `task` would hold that it must not: anything but the task prompt as the person
 * wrote it and the decision texts of the run's index under opaque ids. An index id (`x-…` names a distractor, a real
 * id names its oracle's topic), an oracle's regex, or the task's labels (kind, split, targets) fails it. Every prompt
 * the variants would send is built offline: the session ranker's for `focus` and for `area-focus` with
 * `sessionRankMax`, and one per area the project has for `area-focus` (an area's ranker sees the same decisions
 * whichever of its files was touched); under `sessionRankMax` the area's decisions are listed with a session answer
 * first (`sessionFirst`, every session candidate in reverse index order: the order changes, never the texts).
 */
function rankerLeaks(dataset: Dataset, built: BuiltIndex, task: Task, distractors: number, seed: number, variants: readonly VariantParameters[]): { prompts: number; failures: string[] } {
  const index = runIndex(dataset, built, task, distractors, seed);
  const files = Object.keys(dataset.project);
  const areaFiles = [...new Map(files.map(file => [areaOf(file), file])).values()];
  const texts = new Set(index.map(entry => entry.decision));
  const sessionAnswer = rankerInput({ index, files, prompt: task.prompt }).entries.toReversed();
  const checked = new Set<string>();
  const failures: string[] = [];
  for (const variant of variants) {
    const max = variant.rankMax ?? MAX_RANKED;
    const sessionMax = variant.orient === "area-focus" ? variant.sessionRankMax : null;
    const plans = variant.orient === "focus"
      ? [{ path: null, max }]
      : variant.orient === "area-focus" ? [...(sessionMax === null ? [] : [{ path: null, max: sessionMax }]), ...areaFiles.map(path => ({ path, max }))] : [];
    for (const { path, max: limit } of plans) {
      const ordered = path !== null && sessionMax !== null;
      const where = `${task.id} (${path === null ? "session" : `area ${areaOf(path) || "."}`}, at most ${limit}${ordered ? ", session's picks first" : ""})`;
      if (checked.has(where)) continue;
      checked.add(where);
      const plain = rankerInput({ index, files, prompt: task.prompt }, path);
      const input = ordered ? sessionFirst(plain, sessionAnswer) : plain;
      const prompt = rankPrompt({ task: input.task, decisions: input.decisions }, "0".repeat(32), limit);
      if (input.task !== task.prompt) failures.push(`${where}: the ranker's task is not the task prompt`);
      if (input.decisions.length !== plain.decisions.length) failures.push(`${where}: the session's order changed which decisions the ranker sees`);
      for (const decision of input.decisions) {
        if (!/^d\d+$/u.test(decision.id)) failures.push(`${where}: decision id ${decision.id} is not opaque`);
        if (!texts.has(decision.decision)) failures.push(`${where}: ${decision.id}'s text is not an index decision`);
      }
      for (const entry of index) if (prompt.includes(entry.id)) failures.push(`${where}: the prompt names index id ${entry.id}`);
      for (const [id, oracle] of Object.entries(ORACLES)) for (const pattern of oracle.patterns) if (prompt.includes(pattern.source)) failures.push(`${where}: the prompt holds oracle ${id}'s regex`);
      for (const label of [`kind: ${task.kind}`, `split: ${task.split}`, task.id]) if (prompt.includes(label)) failures.push(`${where}: the prompt holds the task label ${label}`);
    }
  }
  return { prompts: checked.size, failures };
}

function printIntegrity(report: IntegrityReport): void {
  console.log(`\nIndex integrity: ${report.failures.length === 0 ? "PASS" : `FAIL (${report.failures.length})`}`);
  for (const failure of report.failures) console.log(`  FAIL ${failure}`);
  console.log(`Oracle code text in the product's own output (reported, not refused): ${report.overlaps.filter(overlap => overlap.field === "signal").length} signals, ${new Set(report.overlaps.filter(overlap => overlap.field === "decision").map(overlap => overlap.id)).size} decision texts`);
  for (const overlap of report.overlaps.filter(entry => entry.field === "signal")) console.log(`  ${overlap.id} signal ${JSON.stringify(overlap.text)} holds "${overlap.fragment}" (oracle ${overlap.oracles.join(", ")}; ${overlap.by})`);
  const texts = Map.groupBy(report.overlaps.filter(entry => entry.field === "decision"), overlap => overlap.id);
  for (const [id, overlaps] of texts) console.log(`  ${id} decision text holds ${overlaps.map(overlap => `"${overlap.fragment}"`).join(", ")}`);
}

/** The index `--build-index` wrote, refused when it carries grader knowledge or was embedded with another model. */
export function loadIndex(path: string): BuiltIndex {
  const raw = builtIndexSchema.parse(JSON.parse(readFileSync(path, "utf8")));
  const report = checkIntegrity(raw.entries);
  printIntegrity(report);
  if (report.failures.length > 0) throw new Error(`The index ${path} carries grader knowledge; no run reads it.`);
  const fingerprint = loadEmbedder(MODEL_PATH).embedder.fingerprint();
  if (JSON.stringify(fingerprint) !== JSON.stringify(raw.model.fingerprint) || raw.model.maxInputTokens !== EMBED_MAX_TOKENS) {
    throw new Error(`The index ${path} was embedded with another model or token limit; build a new one.`);
  }
  const entries = raw.entries.map(entry => builtEntrySchema.parse(entry));
  const missing = entries.filter(entry => raw.embeddings[entry.id] === undefined).map(entry => entry.id);
  if (missing.length > 0) throw new Error(`The index ${path} has no embedding for ${missing.join(", ")}`);
  return { path, builtAt: raw.builtAt, distractors: raw.distractors, entries, embeddings: raw.embeddings, absent: raw.absent, judgeCalls: raw.judgeCalls };
}

/**
 * A run's decision index, from the built index: the real decisions the product produced, as team or peer decisions. A
 * conflict task makes its own decision this branch's own (not merged yet) and adds the teammate's branch decision that
 * contradicts it. Distractors, when asked for, go in at seeded places among them, so ties in ranking do not always
 * favour the real decisions. A decision absent from the index stays absent. `--seed` 1 is this layout; another seed
 * draws other distractors, places them elsewhere and then shuffles the whole index, so ties and prompt order change.
 */
function runIndex(dataset: Dataset, built: BuiltIndex, task: Task, distractors: number, seed: number): IndexEntry[] {
  if (distractors > built.distractors) throw new Error(`--distractors takes at most ${built.distractors}, what ${built.path} holds`);
  const find = (id: string) => built.entries.find(entry => entry.id === id);
  const ownPresent = task.conflict !== undefined && find(task.conflict.own) !== undefined;
  const index: IndexEntry[] = dataset.decisions.flatMap(decision => {
    const entry = find(decision.id);
    if (entry === undefined) return [];
    if (task.conflict?.own !== entry.id) return [hookEntry(entry)];
    const { pr: _pr, branch: _branch, ...rest } = hookEntry(entry);
    return [{ ...rest, source: "own" as const, author: OWN_AUTHOR, branch: WORK_BRANCH, conflictsWith: task.conflict.peer.id }];
  });
  const peer = task.conflict === undefined ? undefined : find(task.conflict.peer.id);
  if (peer !== undefined) {
    const { conflictsWith: _conflictsWith, ...rest } = hookEntry(peer);
    index.push(ownPresent ? { ...rest, conflictsWith: task.conflict!.own } : rest);
  }
  const random = seeded(DISTRACTOR_SEED + 1 + (seed - 1) * SEED_STRIDE);
  for (const place of distractorPlaces(dataset, distractors, seed)) {
    const entry = find(place.id);
    if (entry !== undefined) index.splice(Math.floor(random() * (index.length + 1)), 0, hookEntry(entry));
  }
  if (seed === 1) return index;
  const shuffle = seeded(SHUFFLE_SEED + seed);
  for (let position = index.length - 1; position > 0; position--) {
    const other = Math.floor(shuffle() * (position + 1));
    [index[position], index[other]] = [index[other]!, index[position]!];
  }
  return index;
}

// ---------------------------------------------------------------------------------------------------------- options

interface Options {
  variants: string[];
  split: (typeof SPLITS)[number] | "all" | null;
  tasks: string[] | null;
  trials: number;
  concurrency: number;
  json: string | undefined;
  keep: boolean;
  verbose: boolean;
  model: string | undefined;
  config: string[];
  listVariants: boolean;
  validateOnly: boolean;
  /** Null until resolved: a run without `--distractors` uses every distractor the index holds, never silently none. */
  distractors: number | null;
  /** Seeds the distractor draw and, above 1, shuffles the run index (`runIndex`); recorded in the report. */
  seed: number;
  /** Whether the control runs too; `--no-control` drops it, for smoke runs of new presets only. */
  control: boolean;
  /** A `--json` report to re-score offline instead of running anything. */
  replay: string | undefined;
  /** A control run's `--json` report whose patches tune the embed threshold (`--sweep`). */
  sweep: string | undefined;
  /** The thresholds `--replay` or `--sweep` tries; null for each one's default. */
  thresholds: number[] | null;
  /** The decision index every run reads (`--index`), or the file `--build-index` writes. */
  index: string | undefined;
  buildIndex: string | undefined;
  /** `--amend`: the index to write from `--index` with cases.json's amendment cases applied by the product. */
  amend: string | undefined;
  /** How long one Codex call may take, in minutes. */
  timeoutMin: number;
  /** `--arms`: the cost-to-clean comparison's arms (`focus-k3`, `posthoc`) instead of `--variants`; null otherwise. */
  arms: Arm[] | null;
  /** `--live`: the two-agent coordination check instead of `--variants` or `--arms`. */
  live: boolean;
  /** `--live`: the decisions to run (every one with an A task and a B task by default). */
  decisions: string[] | null;
}

const USAGE = "Usage: node --import tsx benchmark/merge-assist-context.ts --build-index out.json [--distractors N] [--concurrency N] | --index index.json --variants none,guard-signals,... --split dev|heldout|size|confirm|heldout-large|heldout-large2|all [--tasks id,id] [--trials N] [--concurrency N] [--distractors N] [--seed N] [--timeout-min N] [--no-control] [--json out.json] [--keep] [--verbose] [--model m] [-c key=value] | --index index.json --arms focus-k3,posthoc [--split size|confirm|heldout-large|heldout-large2] [--tasks id,id] [--trials N] [--timeout-min N] ... | --index index.json --live [--trials N] [--decisions id,id] [--tasks id,id] [--concurrency N] [--timeout-min N] [--json out.json] [--keep] | --index index.json --replay run.json [--thresholds 0,0.1,...] | --index index.json --amend out.json | --sweep control.json [--thresholds 0.3,0.35,...] | --list-variants | --validate-oracles";

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
    variants: Object.keys(VARIANTS),
    split: null,
    tasks: null,
    trials: 1,
    concurrency: 3,
    json: undefined,
    keep: false,
    verbose: false,
    model: undefined,
    config: [],
    listVariants: false,
    validateOnly: false,
    distractors: null,
    seed: 1,
    control: true,
    replay: undefined,
    sweep: undefined,
    thresholds: null,
    index: undefined,
    buildIndex: undefined,
    amend: undefined,
    timeoutMin: DEFAULT_TIMEOUT_MIN,
    arms: null,
    live: false,
    decisions: null,
  };
  for (let index = 0; index < argv.length; index++) {
    const flag = argv[index]!;
    const value = (): string | undefined => argv[++index];
    switch (flag) {
      case "--variants": {
        const variants = list(flag, value());
        const unknown = variants.filter(name => !(name in VARIANTS));
        if (unknown.length > 0) throw new Error(`Unknown variant ${unknown.join(", ")}; --list-variants shows them\n${USAGE}`);
        options.variants = variants.filter(name => name !== CONTROL);
        break;
      }
      case "--split": {
        const split = value();
        if (split !== "all" && !SPLITS.some(known => known === split)) throw new Error(`--split takes ${SPLITS.join(", ")} or all\n${USAGE}`);
        options.split = split as Options["split"];
        break;
      }
      case "--timeout-min": options.timeoutMin = positive(flag, value()); break;
      case "--arms": {
        const arms = list(flag, value());
        const unknown = arms.filter(name => !ARMS.some(arm => arm === name));
        if (unknown.length > 0) throw new Error(`Unknown arm ${unknown.join(", ")}: the arms are ${ARMS.join(", ")}\n${USAGE}`);
        options.arms = arms as Arm[];
        break;
      }
      case "--tasks": options.tasks = list(flag, value()); break;
      case "--live": options.live = true; break;
      case "--decisions": options.decisions = list(flag, value()); break;
      case "--trials": options.trials = positive(flag, value()); break;
      case "--concurrency": options.concurrency = positive(flag, value()); break;
      case "--json": options.json = value(); if (options.json === undefined) throw new Error(USAGE); break;
      case "--keep": options.keep = true; break;
      case "--verbose": options.verbose = true; break;
      case "--model": options.model = value(); if (options.model === undefined) throw new Error(USAGE); break;
      case "-c": case "--config": {
        const setting = value();
        if (setting === undefined || !setting.includes("=")) throw new Error(`${flag} takes key=value\n${USAGE}`);
        options.config.push(setting);
        break;
      }
      case "--list-variants": options.listVariants = true; break;
      case "--validate-oracles": options.validateOnly = true; break;
      case "--seed": options.seed = positive(flag, value()); break;
      case "--distractors": {
        options.distractors = Number(value());
        if (!Number.isInteger(options.distractors) || options.distractors < 0) throw new Error(`--distractors takes a whole number\n${USAGE}`);
        break;
      }
      case "--no-control": options.control = false; break;
      case "--replay": options.replay = value(); if (options.replay === undefined) throw new Error(USAGE); break;
      case "--sweep": options.sweep = value(); if (options.sweep === undefined) throw new Error(USAGE); break;
      case "--index": options.index = value(); if (options.index === undefined) throw new Error(USAGE); break;
      case "--build-index": options.buildIndex = value(); if (options.buildIndex === undefined) throw new Error(USAGE); break;
      case "--amend": options.amend = value(); if (options.amend === undefined) throw new Error(USAGE); break;
      case "--thresholds": {
        const thresholds = list(flag, value()).map(Number);
        if (thresholds.some(threshold => !Number.isFinite(threshold) || threshold < 0 || threshold > 1)) throw new Error(`--thresholds takes numbers from 0 to 1\n${USAGE}`);
        options.thresholds = thresholds;
        break;
      }
      case "--help": case "-h": console.log(USAGE); process.exit(0); break;
      default: throw new Error(`Unknown argument ${flag}\n${USAGE}`);
    }
  }
  if (options.variants[0] === CONTROL) options.variants.shift();
  if (options.control) options.variants.unshift(CONTROL);
  if (options.variants.length === 0) throw new Error(`No variants to run\n${USAGE}`);
  return options;
}

/** The tasks to run: the split's (dev by default, size under `--arms`; every split when `--tasks` names them), narrowed by `--tasks`. */
function selectTasks(dataset: Dataset, options: Options): Task[] {
  const split = options.split ?? (options.tasks !== null ? "all" : options.arms !== null ? "size" : "dev");
  const unknown = (options.tasks ?? []).filter(id => !dataset.tasks.some(task => task.id === id));
  if (unknown.length > 0) throw new Error(`Unknown task id: ${unknown.join(", ")}`);
  return dataset.tasks.filter(task => (split === "all" || task.split === split) && (options.tasks === null || options.tasks.includes(task.id)));
}

// ---------------------------------------------------------------------------------------------------------- git

/** Git for the eval's own commands, isolated from the person's global and system configuration. */
export const GIT_ENV: NodeJS.ProcessEnv = {
  ...process.env,
  GIT_CONFIG_NOSYSTEM: "1",
  GIT_CONFIG_GLOBAL: "/dev/null",
  GIT_TERMINAL_PROMPT: "0",
  GIT_AUTHOR_NAME: "Orders Team",
  GIT_AUTHOR_EMAIL: "orders@example.com",
  GIT_COMMITTER_NAME: "Orders Team",
  GIT_COMMITTER_EMAIL: "orders@example.com",
};

function git(cwd: string, ...args: string[]): string {
  return execFileSync("git", args, { cwd, env: GIT_ENV, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"], maxBuffer: 64 * 1024 * 1024 });
}

function writeTree(root: string, files: Readonly<Record<string, string>>): void {
  for (const [path, content] of Object.entries(files)) {
    mkdirSync(dirname(join(root, path)), { recursive: true });
    writeFileSync(join(root, path), content);
  }
}

/** A repository at `repo` with the project committed on `main`, which stays checked out. */
function createMain(repo: string, project: Readonly<Record<string, string>>): void {
  mkdirSync(repo, { recursive: true });
  git(repo, "init", "-q", "-b", "main");
  git(repo, "config", "commit.gpgsign", "false");
  writeTree(repo, project);
  git(repo, "add", "-A");
  git(repo, "commit", "-q", "-m", "Orders service");
}

/** A repository at `repo` with the project committed on `main`, and `branch` checked out from it. */
function createRepository(repo: string, project: Readonly<Record<string, string>>, branch: string): void {
  createMain(repo, project);
  git(repo, "checkout", "-q", "-b", branch);
}

export interface AddedLine {
  readonly line: number;
  readonly text: string;
}

/** One file of `git diff main`: its added lines (every line, for a new file). */
export interface FileChange {
  readonly path: string;
  readonly status: "added" | "modified" | "deleted";
  readonly added: readonly AddedLine[];
}

/** Everything the workspace changed against `main`, committed or not, untracked files included. */
function diffAgainstMain(repo: string): FileChange[] {
  git(repo, "add", "-A");
  const text = git(repo, "-c", "core.quotePath=false", "diff", "--cached", "main", "--no-renames", "--no-ext-diff", "--no-color", "--unified=0");
  const changes: Array<{ path: string; status: FileChange["status"]; added: AddedLine[] }> = [];
  let current: (typeof changes)[number] | undefined;
  let inHunk = false;
  let next = 0;
  for (const line of text.split("\n")) {
    if (line.startsWith("diff --git ")) {
      current = { path: "", status: "modified", added: [] };
      changes.push(current);
      inHunk = false;
    } else if (current === undefined) {
      continue;
    } else if (!inHunk && line.startsWith("new file mode")) {
      current.status = "added";
    } else if (!inHunk && line.startsWith("deleted file mode")) {
      current.status = "deleted";
    } else if (!inHunk && line.startsWith("--- a/")) {
      current.path = line.slice(6);
    } else if (!inHunk && line.startsWith("+++ b/")) {
      current.path = line.slice(6);
    } else if (line.startsWith("@@")) {
      inHunk = true;
      next = Number(/\+(\d+)/u.exec(line)?.[1] ?? 0);
    } else if (inHunk && line.startsWith("+")) {
      current.added.push({ line: next++, text: line.slice(1) });
    }
  }
  return changes.filter(change => change.path !== "");
}

function workspaceFiles(root: string, folder = ""): string[] {
  return readdirSync(join(root, folder), { withFileTypes: true }).flatMap(entry => {
    if (entry.name === ".git") return [];
    const path = folder === "" ? entry.name : `${folder}/${entry.name}`;
    return entry.isDirectory() ? workspaceFiles(root, path) : entry.isFile() ? [path] : [];
  });
}

// ---------------------------------------------------------------------------------------------------------- oracles

/** Added lines that are code: comment lines say nothing about what the code does. */
function codeLines(file: FileChange): AddedLine[] {
  return file.added.filter(({ text }) => !/^\s*(?:\/\/|\/\*|\*)/u.test(text));
}

/** Runs of added code lines with consecutive line numbers, each joined into one text. */
function codeBlocks(file: FileChange): AddedLine[] {
  const blocks: Array<{ line: number; lines: string[] }> = [];
  let previous = Number.NaN;
  for (const { line, text } of codeLines(file)) {
    if (line !== previous + 1) blocks.push({ line, lines: [] });
    blocks.at(-1)!.lines.push(text);
    previous = line;
  }
  return blocks.map(block => ({ line: block.line, text: block.lines.join("\n") }));
}

/** Lower-case words of the identifiers and words in `text`, camelCase and snake_case split apart. */
function words(text: string): string[] {
  return (text.match(/[A-Za-z_$][\w$]*/gu) ?? []).flatMap(token => token.replace(/([a-z0-9])([A-Z])/gu, "$1 $2").split(/[\s_$]+/u)).filter(word => word !== "").map(word => word.toLowerCase());
}

interface Literal {
  readonly quote: "'" | "\"" | "`";
  readonly start: number;
  readonly end: number;
  readonly content: string;
  /** A template with a tag right before it (q`…`), which hands its values over as parameters. */
  readonly tagged: boolean;
}

/** The string and template literals in a block of code, templates' `${…}` expressions kept in their content. */
function literals(text: string): Literal[] {
  const found: Literal[] = [];
  const quoted = (start: number): number => {
    const quote = text[start];
    for (let index = start + 1; index < text.length; index++) {
      if (text[index] === "\\") index++;
      else if (text[index] === quote || text[index] === "\n") return index + 1;
    }
    return text.length;
  };
  const expression = (start: number): number => {
    let depth = 1;
    let index = start;
    while (index < text.length && depth > 0) {
      const char = text[index]!;
      if (char === "`") { index = template(index); continue; }
      if (char === "\"" || char === "'") { index = quoted(index); continue; }
      if (char === "{") depth++;
      else if (char === "}") depth--;
      index++;
    }
    return index;
  };
  const template = (start: number): number => {
    let index = start + 1;
    while (index < text.length) {
      const char = text[index];
      if (char === "\\") index += 2;
      else if (char === "`") return index + 1;
      else if (char === "$" && text[index + 1] === "{") index = expression(index + 2);
      else index++;
    }
    return text.length;
  };
  let index = 0;
  while (index < text.length) {
    const char = text[index]!;
    if (char === "`" || char === "\"" || char === "'") {
      const end = char === "`" ? template(index) : quoted(index);
      found.push({ quote: char, start: index, end, content: text.slice(index + 1, end - 1), tagged: char === "`" && /[\w$)\]]/u.test(text[index - 1] ?? "") });
      index = end;
    } else {
      index++;
    }
  }
  return found;
}

/** Text that reads as a SQL statement or clause. */
const SQL_TEXT = /\b(?:select\s+(?:[\w*"(]|distinct\b)|insert\s+into\b|update\s+["\w.]+\s+set\b|delete\s+from\b|where\s+["\w.()]+\s*(?:=|<>|!=|<=?|>=?|\bin\b|\bis\b|\bi?like\b)|order\s+by\b|group\s+by\b)/iu;
/** A piece of SQL that is being glued onto a statement: a keyword or a numbered placeholder. */
const SQL_PIECE = /\b(?:select|insert|update|delete|from|where|and|or|order\s+by|limit|offset|values|set|i?like|in)\b|\$\d/iu;

/** SQL built as text: an untagged template that holds SQL, or a literal joined with `+`/`+=` into a SQL statement. */
function sqlBuiltAsText(block: string): Literal | undefined {
  const found = literals(block);
  const statement = found.some(literal => !literal.tagged && SQL_TEXT.test(literal.content));
  return found.find(literal => {
    if (literal.tagged) return false;
    if (literal.quote === "`" && SQL_TEXT.test(literal.content)) return true;
    const joined = /^\s*\+(?!\+)/u.test(block.slice(literal.end)) || /(?<!\+)\+=?\s*$/u.test(block.slice(0, literal.start));
    return joined && (SQL_TEXT.test(literal.content) || (statement && SQL_PIECE.test(literal.content)));
  });
}

const LOOP = /\b(?:for|while)\s*\(|\bdo\s*\{|^\s*do\s*$/u;
const API_CALL = /\b(?:request|fetch)\s*(?:<[^()]*>)?\s*\(/u;
const RETRY_WORDS: Readonly<Record<string, true>> = { attempt: true, attempts: true, tries: true, retry: true, retries: true, retrying: true, retried: true, backoff: true, sleep: true, delay: true };
/** The retry words as one pattern, so the integrity check can look for them in an index. */
const RETRY_WORDS_PATTERN = new RegExp(`\\b(?:${Object.keys(RETRY_WORDS).join("|")})\\b|setTimeout`, "u");
const ZOD_INPUT = /\.(?:json|formData|text|arrayBuffer)\s*\(\s*\)|\bsearchParams\b/u;
const ZOD_SCHEMA = /\bz\.object\s*\(|(?<!JSON)\.(?:safeParse|parse)(?:Async)?\s*\(/u;
const DATE_FORMAT = /\.toLocale(?:Date|Time)String\s*\(|\bIntl\.DateTimeFormat\b/u;
const DATE_CONTEXT = /\bDate\b|dateStyle|timeStyle|weekday|month|hour|minute|(?:[Dd]ate|At|[Tt]ime|[Dd]ay)\s*\)?\s*\??\.toLocaleString/u;

/**
 * A decision's deterministic grader: the places in one in-scope file's added lines that break it, and every regular
 * expression it reads them with (the integrity check refuses an index that holds one).
 */
export interface Oracle {
  readonly describe: string;
  readonly patterns: readonly RegExp[];
  hits(file: FileChange): string[];
}

/** An oracle that flags each added code line `test` holds; by default, a line any of `patterns` matches. */
function lineOracle(describe: string, patterns: readonly RegExp[], test = (text: string) => patterns.some(pattern => pattern.test(text))): Oracle {
  return { describe, patterns, hits: file => codeLines(file).filter(({ text }) => test(text)).map(({ line, text }) => `${line}: ${text.trim()}`) };
}

const LOCALE_STRING = /\.toLocaleString\s*\(/u;

/** Oracles by decision id. A decision without one (logic-out-of-components) is guidance no regex grades. */
export const ORACLES: Readonly<Record<string, Oracle>> = {
  "console-in-src": lineOracle("console.* call", [/\bconsole\.\w+/u]),
  "fetch-outside-client": lineOracle("fetch( call", [/\bfetch\s*\(/u]),
  "env-outside-config": lineOracle("process.env read", [/\bprocess\.env\b|\bprocess\[\s*["']env["']\s*\]/u]),
  "default-export-ui": lineOracle("export default", [/\bexport\s+default\b|\bexport\s*\{[^}]*\bas\s+default\b/u]),
  "dates-through-formatdate": lineOracle("locale date formatting", [DATE_FORMAT, LOCALE_STRING, DATE_CONTEXT], text => DATE_FORMAT.test(text) || (LOCALE_STRING.test(text) && DATE_CONTEXT.test(text))),
  "result-not-throw": lineOracle("throw statement", [/\bthrow\s/u]),
  "sql-in-db": {
    describe: "SQL built by + or an untagged template",
    patterns: [SQL_TEXT, SQL_PIECE],
    hits: file => codeBlocks(file).flatMap(({ line, text }) => {
      const literal = sqlBuiltAsText(text);
      if (literal === undefined) return [];
      const offset = text.slice(0, literal.start).split("\n").length - 1;
      return [`${line + offset}: ${text.split("\n")[offset]!.trim()}`];
    }),
  },
  "retry-in-transport": {
    describe: "a loop that calls request()/fetch() again, with retry words",
    patterns: [LOOP, API_CALL, RETRY_WORDS_PATTERN],
    hits: file => {
      const lines = codeLines(file);
      const callsApi = lines.some(({ text }) => API_CALL.test(text));
      return lines.flatMap(({ line, text }, index) => {
        if (!LOOP.test(text)) return [];
        const window = lines.slice(index, index + 21).map(entry => entry.text);
        const retrying = window.some(entry => words(entry).some(word => RETRY_WORDS[word] === true) || /setTimeout/u.test(entry));
        return retrying && (callsApi || window.some(entry => API_CALL.test(entry))) ? [`${line}: ${text.trim()}`] : [];
      });
    },
  },
  "zod-next-to-handler": {
    describe: "a handler reading input with no zod schema",
    patterns: [ZOD_INPUT, ZOD_SCHEMA],
    hits: file => {
      const lines = codeLines(file);
      const input = lines.find(({ text }) => ZOD_INPUT.test(text));
      return input === undefined || lines.some(({ text }) => ZOD_SCHEMA.test(text)) ? [] : [`${input.line}: ${input.text.trim()}`];
    },
  },
};

/** Each graded decision's hits in `changes`, read through the dataset's scope for it (the grader's, not the index's). */
export function grade(changes: readonly FileChange[], decisions: ReadonlyArray<{ id: string; scope: IndexEntry["scope"] }>): Record<string, string[]> {
  const hits: Record<string, string[]> = {};
  for (const entry of decisions) {
    const oracle = ORACLES[entry.id];
    if (oracle === undefined) continue;
    const scope = scopeMatcher(entry.scope);
    hits[entry.id] = changes.filter(change => change.status !== "deleted" && scope.specificity(change.path, false) >= 0).flatMap(change => oracle.hits(change).map(hit => `${change.path}:${hit}`));
  }
  return hits;
}

interface OracleCheck {
  readonly decisionId: string;
  readonly sample: string;
  readonly path: string;
  readonly expect: "caught" | "passed";
  readonly hits: readonly string[];
}

/**
 * Every oracle against the labelled versions of its capture case in cases.json: the case's `before` (caught), its
 * `after` (passed) and each teammate file as labelled, each written alone over the project in a scratch repository and
 * graded through `git diff main`, the path a run takes.
 */
async function validateOracles(dataset: Dataset): Promise<OracleCheck[]> {
  const folder = await realpath(await mkdtemp(join(tmpdir(), "rerail-ma-context-oracles-")));
  try {
    const repo = join(folder, "repo");
    createRepository(repo, dataset.project, WORK_BRANCH);
    const checks: OracleCheck[] = [];
    for (const decision of dataset.decisions) {
      if (ORACLES[decision.id] === undefined) continue;
      const labelled = dataset.labelled.find(entry => entry.id === decision.id);
      if (labelled === undefined) throw new Error(`No labelled case in cases.json for decision ${decision.id}`);
      const samples = [
        { sample: "before", path: labelled.file, content: labelled.before, expect: "caught" as const },
        { sample: "after", path: labelled.file, content: labelled.after, expect: "passed" as const },
        ...(labelled.teammate ?? []).map(file => ({ sample: "teammate", path: file.path, content: file.content, expect: file.expect })),
      ];
      for (const sample of samples) {
        writeTree(repo, { [sample.path]: sample.content });
        const hits = grade(diffAgainstMain(repo), [decision])[decision.id] ?? [];
        checks.push({ decisionId: decision.id, sample: sample.sample, path: sample.path, expect: sample.expect, hits });
        git(repo, "reset", "-q", "--hard", "HEAD");
        git(repo, "clean", "-qfd");
      }
    }
    return checks;
  } finally {
    await rm(folder, { recursive: true, force: true });
  }
}

function printOracleChecks(checks: readonly OracleCheck[]): boolean {
  console.log("\nOracle validation (labelled files in cases.json)");
  console.log(table(
    ["decision", "sample", "path", "expect", "got", "", "hit"],
    checks.map(check => {
      const got = check.hits.length > 0 ? "caught" : "passed";
      return [check.decisionId, check.sample, check.path, check.expect, got, got === check.expect ? "ok" : "WRONG", clip(check.hits[0] ?? "", 70)];
    }),
  ));
  const wrong = checks.filter(check => (check.hits.length > 0 ? "caught" : "passed") !== check.expect);
  const decisions = new Set(checks.map(check => check.decisionId));
  console.log(`${checks.length - wrong.length}/${checks.length} labelled files graded as labelled, ${decisions.size} oracles${wrong.length === 0 ? "" : `; WRONG: ${wrong.map(check => `${check.decisionId} ${check.sample} ${check.path}`).join(", ")}`}`);
  return wrong.length === 0;
}

// ---------------------------------------------------------------------------------------------------------- run

export interface Usage {
  input: number;
  cached: number;
  output: number;
  /** Input tokens written to the prompt cache, when the provider reports them (Bedrock does); part of `input`. */
  cacheWrite?: number;
}

interface TargetDelivery {
  readonly decisionId: string;
  readonly injected: boolean;
  readonly timely: boolean;
  /** `hybrid`: the channel that first delivered it; null otherwise. */
  readonly via: Channel | null;
  /** Area orientation: in the run's first orientation injection. */
  readonly firstArea: boolean;
  /** A candidate at a call whose orientation injected, not yet delivered, and not injected: the cap cut it. */
  readonly cut: boolean;
}

interface RunResult {
  readonly variant: string;
  readonly taskId: string;
  readonly split: Task["split"];
  readonly kind: Task["kind"];
  readonly trial: number;
  readonly targets: readonly string[];
  readonly error: string | null;
  readonly timedOut: boolean;
  readonly exitCode: number | null;
  readonly wallMs: number;
  readonly usage: Usage | null;
  readonly finalMessage: string | null;
  readonly commands: number;
  readonly hits: Readonly<Record<string, readonly string[]>>;
  readonly violated: boolean;
  readonly done: boolean;
  readonly doneMisses: readonly string[];
  readonly changedFiles: readonly string[];
  readonly overreachFiles: readonly string[];
  readonly addedLines: number;
  readonly hookCalls: number;
  readonly hookErrors: readonly string[];
  readonly injections: number;
  readonly injectedDecisions: readonly string[];
  readonly injectedChars: number;
  readonly injectedTokens: number;
  readonly irrelevant: number;
  /** Injections whose text Codex's transcript holds; null when no transcript was found. */
  readonly landed: number | null;
  readonly delivery: readonly TargetDelivery[];
  readonly chatter: boolean;
  readonly mentions: boolean | null;
  /** The model Codex's transcript names; null without a transcript. */
  readonly model: string | null;
  /** The hook's log, without the injected texts: every trigger's scored candidates, for `--replay`. */
  readonly hookLog: readonly HookLogEntry[];
  /** `orient: focus`: what the session-focus ranker did in the run; null for other variants. */
  readonly focus: FocusRun | null;
  readonly folder: string | null;
}

interface FocusRun {
  /** `focus`, and `area-focus` with `sessionRankMax`: the session ranker's answer, read after the run (waiting for it if it landed after Codex finished); null if it never came, and under other `area-focus` variants. */
  readonly file: FocusFile | null;
  /**
   * `area-focus`: each area's ranker, in the order the areas were first touched: its answer (read as `file` is; its
   * `ms` is the area's ranker time) and how the area's picks went out (null: they never did).
   */
  readonly areas: ReadonlyArray<{ readonly area: string; readonly file: FocusFile | null; readonly via: AreaFocusLog["via"] | null }>;
  /** Whether the ranker had answered at the first call that touched an area; null when no area was touched. */
  readonly readyAtFirstArea: boolean | null;
  /** How long each pre-edit waited for a ranker, in milliseconds. */
  readonly waits: readonly number[];
  readonly fallback: "timeout" | "failed" | null;
}

interface Job {
  readonly variant: string;
  readonly task: Task;
  readonly trial: number;
}

function shellSafe(path: string): string {
  if (!/^[\w./@+-]+$/u.test(path)) throw new Error(`The hook command cannot hold the path ${path}: it has characters a shell would read`);
  return path;
}

/**
 * A temporary CODEX_HOME: the person's sign-in through a symlink, no configuration, and `hook` (the eval hook unless
 * another is named) on SessionStart and every tool call; with `hook` null, no hooks at all.
 */
export function createCodexHome(home: string, configPath: string, hook: string | null = HOOK): void {
  mkdirSync(home, { recursive: true });
  symlinkSync(AUTH, join(home, "auth.json"));
  writeFileSync(join(home, "config.toml"), "");
  if (hook === null) return;
  const loader = fileURLToPath(import.meta.resolve("tsx"));
  const command = [
    "env",
    `MA_CONTEXT_CONFIG=${shellSafe(configPath)}`,
    `TSX_TSCONFIG_PATH=${shellSafe(REPOSITORY_TSCONFIG)}`,
    shellSafe(process.execPath),
    "--import",
    shellSafe(loader),
    shellSafe(hook),
  ].join(" ");
  const group = [{ matcher: "", hooks: [{ type: "command", command }] }];
  writeFileSync(join(home, "hooks.json"), `${JSON.stringify({ hooks: { SessionStart: group, PreToolUse: group, PostToolUse: group } }, null, 2)}\n`);
}

export interface CodexOutcome {
  readonly stdout: string;
  readonly stderr: string;
  readonly exitCode: number | null;
  readonly timedOut: boolean;
  readonly wallMs: number;
}

export interface CodexRunOptions {
  readonly model: string | undefined;
  readonly config: readonly string[];
  readonly timeoutMin: number;
  /** More `codex exec` arguments, before the prompt (`--output-schema f.json`). */
  readonly extraArgs?: readonly string[];
  /**
   * A permission profile CODEX_HOME's config.toml defines, run instead of the `workspace-write` sandbox (`--sandbox`
   * would force the legacy sandbox and ignore the profile).
   */
  readonly permissions?: string;
  /** The Codex executable; `codex` from PATH by default. */
  readonly binary?: string;
  /** More environment for the Codex process only (a provider's credentials), over the harness's own. */
  readonly env?: Readonly<Record<string, string>>;
}

/**
 * One `codex exec` call with `prompt` on stdin; with `resume`, `codex exec resume <thread> -` continues that session in
 * the same CODEX_HOME and workspace (resume takes no `--sandbox` or `-C`: the sandbox is set by `-c`, the folder by
 * the working directory). At most `--timeout-min` minutes.
 */
export function runCodex(repo: string, codexHome: string, configPath: string, prompt: string, options: CodexRunOptions, rerailHome: string, resume?: string): Promise<CodexOutcome> {
  const sandbox = options.permissions === undefined ? `sandbox_mode="workspace-write"` : `default_permissions="${options.permissions}"`;
  const args = resume === undefined
    ? ["exec", "--json", "--dangerously-bypass-hook-trust", ...(options.permissions === undefined ? ["--sandbox", "workspace-write"] : ["-c", sandbox]), "--skip-git-repo-check", "-C", repo]
    : ["exec", "resume", "--json", "--dangerously-bypass-hook-trust", "--skip-git-repo-check", "-c", sandbox];
  if (options.model !== undefined) args.push("-m", options.model);
  for (const setting of options.config) args.push("-c", setting);
  if (options.extraArgs !== undefined) args.push(...options.extraArgs);
  if (resume !== undefined) args.push(resume);
  args.push("-");
  const started = performance.now();
  const { promise, resolve, reject } = Promise.withResolvers<CodexOutcome>();
  const child = spawn(options.binary ?? "codex", args, {
    cwd: repo,
    detached: true,
    env: { ...process.env, ...options.env, CODEX_HOME: codexHome, MA_CONTEXT_CONFIG: configPath, RERAIL_HOME: rerailHome, RERAIL_AUTOSTART: "0" },
    stdio: ["pipe", "pipe", "pipe"],
  });
  const stdout: Buffer[] = [];
  const stderr: Buffer[] = [];
  let timedOut = false;
  const kill = (signal: NodeJS.Signals) => {
    try {
      process.kill(-child.pid!, signal);
    } catch {
      // already gone
    }
  };
  const timer = setTimeout(() => {
    timedOut = true;
    kill("SIGTERM");
    setTimeout(() => kill("SIGKILL"), 5_000).unref();
  }, options.timeoutMin * 60_000);
  child.stdout.on("data", (chunk: Buffer) => stdout.push(chunk));
  child.stderr.on("data", (chunk: Buffer) => stderr.push(chunk));
  child.on("error", error => {
    clearTimeout(timer);
    reject(error);
  });
  child.on("close", code => {
    clearTimeout(timer);
    resolve({ stdout: Buffer.concat(stdout).toString("utf8"), stderr: Buffer.concat(stderr).toString("utf8"), exitCode: code, timedOut, wallMs: performance.now() - started });
  });
  child.stdin.end(prompt);
  return promise;
}

/** What Codex's `--json` events say: the thread id, the last agent message, token usage, commands run, and any reported failure. */
export interface CodexEvents {
  readonly threadId: string | null;
  readonly finalMessage: string | null;
  readonly usage: Usage | null;
  readonly commands: number;
  readonly error: string | null;
}

export function readEvents(stdout: string): CodexEvents {
  let threadId: string | null = null;
  let finalMessage: string | null = null;
  let usage: Usage | null = null;
  let commands = 0;
  let error: string | null = null;
  for (const line of stdout.split("\n")) {
    if (!line.startsWith("{")) continue;
    let event: Record<string, unknown>;
    try {
      event = JSON.parse(line) as Record<string, unknown>;
    } catch {
      continue;
    }
    const item = event.item as Record<string, unknown> | undefined;
    if (event.type === "thread.started" && typeof event.thread_id === "string") threadId = event.thread_id;
    if (event.type === "item.completed" && item?.type === "agent_message" && typeof item.text === "string") finalMessage = item.text;
    if (event.type === "item.completed" && item?.type === "command_execution") commands++;
    if (event.type === "turn.completed" && event.usage !== null && typeof event.usage === "object") {
      const fields = event.usage as Record<string, unknown>;
      const count = (value: unknown) => (typeof value === "number" ? value : 0);
      usage ??= { input: 0, cached: 0, output: 0 };
      usage.input += count(fields.input_tokens);
      usage.cached += count(fields.cached_input_tokens);
      usage.output += count(fields.output_tokens);
      if (typeof fields.cache_write_input_tokens === "number") usage.cacheWrite = (usage.cacheWrite ?? 0) + fields.cache_write_input_tokens;
    }
    if (event.type === "turn.failed" || event.type === "error") {
      const detail = (event.error as Record<string, unknown> | undefined)?.message ?? event.message;
      error = typeof detail === "string" ? detail : JSON.stringify(event);
    }
  }
  return { threadId, finalMessage, usage, commands, error };
}

function readLog(path: string): HookLogEntry[] {
  if (!existsSync(path)) return [];
  return readFileSync(path, "utf8").split("\n").filter(line => line.trim() !== "").map(line => JSON.parse(line) as HookLogEntry);
}

/** How long a finished run waits for a ranker still at work: the judge's own timeout and a margin. */
const FOCUS_SETTLE_MS = 150_000;

/**
 * The rankers' part in a run, from the hook log and `focus.json` (`focus`, and `area-focus` with `sessionRankMax`) and
 * each touched area's `focus-<area>.json` (`area-focus`); waits for rankers still at work so none outlives the run.
 */
async function focusRun(folder: string, variant: VariantParameters, log: readonly HookLogEntry[]): Promise<FocusRun> {
  const { orient } = variant;
  const session = orient === "focus" || (orient === "area-focus" && variant.sessionRankMax !== null);
  const calls = log.flatMap(entry => (entry.focus === undefined ? [] : [entry.focus]));
  const touched = orient === "area-focus" ? [...new Set(calls.flatMap(call => call.fresh))] : [];
  const paths = [...(session ? [focusPath(folder)] : []), ...touched.map(area => areaFocusPath(folder, area))];
  const started = performance.now();
  while (paths.some(path => !existsSync(path)) && performance.now() - started < FOCUS_SETTLE_MS) await sleep(1_000);
  const read = (path: string) => (existsSync(path) ? JSON.parse(readFileSync(path, "utf8")) as FocusFile : null);
  const served = calls.flatMap(call => call.areas ?? []);
  const first = calls.find(call => call.fresh.length > 0);
  const fallbacks = calls.map(call => call.fallback);
  return {
    file: session ? read(focusPath(folder)) : null,
    areas: touched.map(area => ({ area, file: read(areaFocusPath(folder, area)), via: served.find(entry => entry.area === area)?.via ?? null })),
    readyAtFirstArea: first === undefined ? null : first.ready,
    waits: calls.flatMap(call => (call.waitedMs === null ? [] : [call.waitedMs])),
    fallback: fallbacks.includes("failed") ? "failed" : fallbacks.includes("timeout") ? "timeout" : null,
  };
}

/** Codex's own transcripts of the run, under the temporary CODEX_HOME, as raw text; null when it wrote none. */
export function transcriptText(codexHome: string): string | null {
  const sessions = join(codexHome, "sessions");
  if (!existsSync(sessions)) return null;
  const files = workspaceFiles(sessions).filter(path => path.endsWith(".jsonl"));
  return files.length === 0 ? null : files.map(path => readFileSync(join(sessions, path), "utf8")).join("\n");
}

/**
 * Whether each target decision reached the agent, and whether in time: injected at a hook call before the first
 * apply_patch that touches its scope (the dataset's). An injection at that apply_patch's own PreToolUse is late: the
 * model wrote the patch before it saw the decision.
 */
function deliveries(log: readonly HookLogEntry[], targets: readonly string[], dataset: Dataset): TargetDelivery[] {
  return targets.map(decisionId => {
    const entry = dataset.decisions.find(candidate => candidate.id === decisionId)!;
    const scope = scopeMatcher(entry.scope);
    const injectedAt = log.findIndex(call => call.injected.some(item => item.decisionId === decisionId));
    const editAt = log.findIndex(call => call.event === "PreToolUse" && call.kind === "edit" && call.paths.some(path => scope.specificity(path, false) >= 0));
    const via = injectedAt === -1 ? null : log[injectedAt]!.injected.find(item => item.decisionId === decisionId)!.channel ?? null;
    const orienting = (call: HookLogEntry) => call.injected.some(item => item.channel === "orientation");
    const firstArea = log.find(orienting)?.injected.some(item => item.decisionId === decisionId) ?? false;
    const cut = log.some((call, position) => orienting(call)
      && (injectedAt === -1 || injectedAt >= position)
      && !call.injected.some(item => item.decisionId === decisionId)
      && (call.candidates ?? []).some(candidate => candidate.channel === "orientation" && candidate.ids.includes(decisionId)));
    return { decisionId, injected: injectedAt !== -1, timely: injectedAt !== -1 && (editAt === -1 || injectedAt < editAt), via, firstArea, cut };
  });
}

function escapeRegex(text: string): string {
  return text.replace(/[.*+?^${}()|[\]\\]/gu, "\\$&");
}

/** The final message talks to the person about Rerail or the decisions: their words, authors, branches or PRs. */
function chatterPattern(index: readonly IndexEntry[]): RegExp {
  const names = index.flatMap(entry => [entry.author, ...(entry.source === "own" || entry.branch === undefined ? [] : [entry.branch])]).filter(name => name !== OWN_AUTHOR);
  return new RegExp(`\\brerail\\b|\\bdecisions?\\b|\\bdecided\\b|\\bnot merged\\b|#\\d{3}\\b|${[...new Set(names)].map(escapeRegex).join("|")}`, "iu");
}

/**
 * The final message tells the person about the disagreement: it names the teammate, or says they disagree. The
 * `final-line` Note ("Note: <author> decided differently on <branch>: …") matches on both counts.
 */
function mentionPattern(peer: DatasetEntry): RegExp {
  const names = [...peer.author.split(/\s+/u), ...(peer.branch === undefined ? [] : [peer.branch])].map(escapeRegex);
  return new RegExp(`decided differently|conflict|disagree|\\b(?:${names.join("|")})\\b`, "iu");
}

/** The task's `done` checks no workspace file passes: what is still missing for the task to count as finished. */
function doneMissesIn(repo: string, task: Task): string[] {
  const files = workspaceFiles(repo);
  return task.done.filter(done => {
    const matcher = globPattern([done.path]);
    const pattern = new RegExp(done.regex, "u");
    return !files.some(path => matcher.test(path) && pattern.test(readFileSync(join(repo, path), "utf8")));
  }).map(done => `${done.path} /${done.regex}/`);
}

async function runJob(job: Job, dataset: Dataset, built: BuiltIndex, options: Options, setUp = (repo: string) => createRepository(repo, dataset.project, WORK_BRANCH)): Promise<RunResult> {
  const { task } = job;
  const folder = await realpath(await mkdtemp(join(tmpdir(), "rerail-ma-context-")));
  const repo = join(folder, "repo");
  const codexHome = join(folder, "codex-home");
  const configPath = join(folder, "hook-config.json");
  const logPath = join(folder, "hook.log");
  const index = runIndex(dataset, built, task, options.distractors ?? built.distractors, options.seed);
  const base = { variant: job.variant, taskId: task.id, split: task.split, kind: task.kind, trial: job.trial, targets: task.targets };
  try {
    setUp(repo);
    const config: HookConfig = {
      variant: VARIANTS[job.variant]!,
      index,
      prompt: task.prompt,
      embeddings: Object.fromEntries(index.map(entry => [entry.id, built.embeddings[entry.id]!])),
      modelPath: MODEL_PATH,
      files: Object.keys(dataset.project),
      // `oracle-area` only: the diagnostic ceiling selects with the task's labels.
      ...(VARIANTS[job.variant]?.orient === "targets" ? { targets: task.targets } : {}),
      root: repo,
      folder,
      log: logPath,
      state: join(folder, "hook-state.json"),
    };
    writeFileSync(configPath, `${JSON.stringify(config, null, 2)}\n`);
    writeFileSync(logPath, "");
    createCodexHome(codexHome, configPath);
    const outcome = await runCodex(repo, codexHome, configPath, task.prompt, options, join(folder, "rerail-home"));
    const events = readEvents(outcome.stdout);
    const log = readLog(logPath);
    const focus = config.variant?.orient === "focus" || config.variant?.orient === "area-focus" ? await focusRun(folder, config.variant, log) : null;
    const changes = diffAgainstMain(repo);
    const hits = grade(changes, dataset.decisions);
    const graded = task.kind === "neutral" ? Object.keys(hits) : task.targets;
    const doneMisses = doneMissesIn(repo, task);
    const natural = globPattern([...task.done.map(done => done.path), ...task.expectedFiles]);
    const injectedCalls = log.filter(call => call.injected.length > 0);
    const injectedDecisions = injectedCalls.flatMap(call => call.injected.map(item => item.decisionId));
    const injectedChars = injectedCalls.reduce((sum, call) => sum + call.chars, 0);
    const relevant = new Set([...task.targets, ...(task.conflict === undefined ? [] : [task.conflict.peer.id])]);
    const transcript = transcriptText(codexHome);
    // What the agent says, not where: link targets and absolute paths name the temporary folders, which say "rerail".
    const message = (events.finalMessage ?? "").replace(/\]\([^)]*\)/gu, "]").replace(/(?<![\w.-])~?\/[\w.@+/-]+/gu, " ");
    const failure = outcome.timedOut
      ? `timed out after ${options.timeoutMin} minutes`
      : events.error ?? (outcome.exitCode === 0 ? null : `codex exited ${outcome.exitCode}: ${clip(outcome.stderr.trim().split("\n").at(-1) ?? "", 200)}`);
    return {
      ...base,
      error: failure,
      timedOut: outcome.timedOut,
      exitCode: outcome.exitCode,
      wallMs: outcome.wallMs,
      usage: events.usage,
      finalMessage: events.finalMessage,
      commands: events.commands,
      hits,
      violated: graded.some(id => (hits[id] ?? []).length > 0),
      done: doneMisses.length === 0,
      doneMisses,
      changedFiles: changes.map(change => change.path),
      overreachFiles: changes.map(change => change.path).filter(path => !natural.test(path)),
      addedLines: changes.reduce((sum, change) => sum + change.added.length, 0),
      hookCalls: log.length,
      hookErrors: log.flatMap(call => (call.error === undefined ? [] : [call.error])),
      injections: injectedCalls.length,
      injectedDecisions,
      injectedChars,
      injectedTokens: Math.ceil(injectedChars / 4),
      irrelevant: injectedDecisions.filter(id => !relevant.has(id)).length,
      landed: transcript === null ? null : injectedCalls.filter(call => call.text !== undefined && transcript.includes(JSON.stringify(call.text).slice(1, -1))).length,
      delivery: deliveries(log, task.targets, dataset),
      chatter: task.kind !== "conflict" && chatterPattern(index).test(message),
      mentions: task.conflict === undefined ? null : mentionPattern(task.conflict.peer).test(message),
      model: transcript === null ? null : /"model":"([^"]+)"/u.exec(transcript)?.[1] ?? null,
      hookLog: log.map(({ text: _text, ...entry }) => entry),
      focus,
      folder: options.keep ? folder : null,
    };
  } catch (error) {
    return {
      ...base,
      error: error instanceof Error ? error.message : String(error),
      timedOut: false,
      exitCode: null,
      wallMs: 0,
      usage: null,
      finalMessage: null,
      commands: 0,
      hits: {},
      violated: false,
      done: false,
      doneMisses: [],
      changedFiles: [],
      overreachFiles: [],
      addedLines: 0,
      hookCalls: 0,
      hookErrors: [],
      injections: 0,
      injectedDecisions: [],
      injectedChars: 0,
      injectedTokens: 0,
      irrelevant: 0,
      landed: null,
      delivery: [],
      chatter: false,
      mentions: null,
      model: null,
      hookLog: [],
      focus: null,
      folder: options.keep ? folder : null,
    };
  } finally {
    if (options.keep) console.error(`kept ${folder}`);
    else await rm(folder, { recursive: true, force: true });
  }
}

// ---------------------------------------------------------------------------------------------------------- metrics

export interface Ratio {
  readonly hits: number;
  readonly total: number;
}

export const ratio = (hits: number, total: number): Ratio => ({ hits, total });
export const share = (value: Ratio): number => (value.total === 0 ? Number.NaN : value.hits / value.total);

/** Wilson score interval at 95% for a share. */
export function wilson(value: Ratio): [number, number] {
  if (value.total === 0) return [Number.NaN, Number.NaN];
  const z = 1.96;
  const p = value.hits / value.total;
  const n = value.total;
  const centre = (p + (z * z) / (2 * n)) / (1 + (z * z) / n);
  const half = (z * Math.sqrt((p * (1 - p)) / n + (z * z) / (4 * n * n))) / (1 + (z * z) / n);
  return [Math.max(0, centre - half), Math.min(1, centre + half)];
}

/** The `fraction` percentile of `values` by the nearest-rank method. */
function percentile(values: readonly number[], fraction: number): number {
  if (values.length === 0) return Number.NaN;
  const sorted = [...values].sort((left, right) => left - right);
  return sorted[Math.min(sorted.length - 1, Math.ceil(fraction * sorted.length) - 1)]!;
}

export function median(values: readonly number[]): number {
  if (values.length === 0) return Number.NaN;
  const sorted = [...values].sort((left, right) => left - right);
  const middle = Math.floor(sorted.length / 2);
  return sorted.length % 2 === 1 ? sorted[middle]! : (sorted[middle - 1]! + sorted[middle]!) / 2;
}

export function mean(values: readonly number[]): number {
  return values.length === 0 ? Number.NaN : values.reduce((sum, value) => sum + value, 0) / values.length;
}

interface VariantSummary {
  readonly variant: string;
  readonly runs: number;
  readonly failed: number;
  readonly temptViolation: Ratio;
  readonly temptInterval: [number, number];
  readonly reduction: number;
  readonly done: Ratio;
  readonly medianTokens: number;
  readonly neutralOverreach: number;
  readonly neutralViolation: Ratio;
  readonly chatter: Ratio;
  readonly followsOwn: Ratio;
  readonly mentions: Ratio;
  readonly conflictScore: number;
  readonly targetInjected: Ratio;
  readonly targetTimely: Ratio;
  readonly targetComplied: Ratio;
  readonly targetViaOrientation: Ratio;
  readonly targetViaGuard: Ratio;
  readonly targetFirstArea: Ratio;
  readonly targetCut: Ratio;
  readonly irrelevantPerRun: number;
  /** Decisions injected per run, repeats included: median and max. */
  readonly decisionsMedian: number;
  readonly decisionsMax: number;
  /** Injected tokens per run: the 90th percentile. */
  readonly tokensP90: number;
  /** `orient: focus` runs: the ranker had answered at the first area touch; pre-edit waits; runs that fell back; ranker time; judge prompt tokens (estimated as characters / 4). */
  readonly focusReady: Ratio;
  readonly focusWaitMedian: number;
  readonly focusWaitMax: number;
  readonly focusFallback: Ratio;
  readonly rankerMsMedian: number;
  readonly rankerTokensMedian: number;
  readonly landed: Ratio;
  readonly meanInput: number;
  readonly meanCached: number;
  readonly meanOutput: number;
  readonly medianWallS: number;
  readonly meanAddedLines: number;
  readonly hookErrors: number;
  readonly guardrails: ReadonlyArray<{ name: string; pass: boolean | null }>;
  readonly score: number;
}

function summarize(variant: string, runs: readonly RunResult[], control: VariantSummary | undefined): VariantSummary {
  const ran = runs.filter(run => run.error === null);
  const ofKind = (kind: Task["kind"]) => ran.filter(run => run.kind === kind);
  const tempt = ofKind("tempt");
  const neutral = ofKind("neutral");
  const conflict = ofKind("conflict");
  const temptViolation = ratio(tempt.filter(run => run.violated).length, tempt.length);
  const temptRate = share(temptViolation);
  // Against the control only: the control's own row, and runs with --no-control, leave these blank.
  const controlRate = control === undefined ? Number.NaN : share(control.temptViolation);
  const reduction = controlRate > 0 ? (controlRate - temptRate) / controlRate : Number.NaN;
  const done = ratio(ran.filter(run => run.done).length, ran.length);
  const medianTokens = median(ran.map(run => run.injectedTokens));
  const neutralOverreach = mean(neutral.map(run => run.overreachFiles.length));
  const neutralViolation = ratio(neutral.filter(run => run.violated).length, neutral.length);
  const talking = [...tempt, ...neutral];
  const chatter = ratio(talking.filter(run => run.chatter).length, talking.length);
  const followsOwn = ratio(conflict.filter(run => !run.violated).length, conflict.length);
  const mentions = ratio(conflict.filter(run => run.mentions === true).length, conflict.length);
  const conflictScore = share(ratio(conflict.filter(run => !run.violated && run.mentions === true).length, conflict.length));
  const targeted = ran.filter(run => run.kind !== "neutral").flatMap(run => run.delivery.map(delivery => ({ run, delivery })));
  const targetInjected = ratio(targeted.filter(({ delivery }) => delivery.injected).length, targeted.length);
  const timely = targeted.filter(({ delivery }) => delivery.timely);
  const targetTimely = ratio(timely.length, targeted.length);
  const targetComplied = ratio(timely.filter(({ run, delivery }) => (run.hits[delivery.decisionId] ?? []).length === 0).length, targeted.length);
  const targetViaOrientation = ratio(targeted.filter(({ delivery }) => delivery.via === "orientation").length, targeted.length);
  const targetViaGuard = ratio(targeted.filter(({ delivery }) => delivery.via === "guard").length, targeted.length);
  const targetFirstArea = ratio(targeted.filter(({ delivery }) => delivery.firstArea).length, targeted.length);
  const targetCut = ratio(targeted.filter(({ delivery }) => delivery.cut).length, targeted.length);
  const focused = ran.flatMap(run => (run.focus === null ? [] : [run.focus]));
  const waits = focused.flatMap(focus => focus.waits);
  const rankers = focused.flatMap(focus => [focus.file, ...focus.areas.map(area => area.file)].flatMap(file => (file === null ? [] : [file])));
  const landedRuns = ran.filter(run => run.landed !== null);
  const landed = ratio(landedRuns.reduce((sum, run) => sum + run.landed!, 0), landedRuns.reduce((sum, run) => sum + run.injections, 0));
  const at = (value: number, bound: number, op: ">=" | "<="): boolean | null => (Number.isNaN(value) || Number.isNaN(bound) ? null : op === ">=" ? value >= bound - 1e-9 : value <= bound + 1e-9);
  const guardrails = [
    { name: "done", pass: at(share(done), (control === undefined ? Number.NaN : share(control.done)) - 0.1, ">=") },
    { name: "tokens", pass: at(medianTokens, 300, "<=") },
    { name: "neutral overreach", pass: at(neutralOverreach, (control === undefined ? Number.NaN : control.neutralOverreach) + 0.25, "<=") },
    { name: "neutral violation", pass: at(share(neutralViolation), control === undefined ? Number.NaN : share(control.neutralViolation), "<=") },
    { name: "chatter", pass: at(share(chatter), 0.1, "<=") },
    { name: "conflict follows-own", pass: at(share(followsOwn), 0.8, ">=") },
    { name: "conflict mentions", pass: at(share(mentions), 0.8, ">=") },
  ];
  const overreachDelta = control === undefined || Number.isNaN(neutralOverreach) || Number.isNaN(control.neutralOverreach) ? 0 : neutralOverreach - control.neutralOverreach;
  const score = (Number.isNaN(reduction) ? 0 : reduction)
    - 0.1 * ((Number.isNaN(medianTokens) ? 0 : medianTokens) / 100)
    - 0.5 * overreachDelta
    - 0.5 * (1 - (Number.isNaN(conflictScore) ? 1 : conflictScore));
  return {
    variant,
    runs: runs.length,
    failed: runs.length - ran.length,
    temptViolation,
    temptInterval: wilson(temptViolation),
    reduction,
    done,
    medianTokens,
    neutralOverreach,
    neutralViolation,
    chatter,
    followsOwn,
    mentions,
    conflictScore,
    targetInjected,
    targetTimely,
    targetComplied,
    targetViaOrientation,
    targetViaGuard,
    targetFirstArea,
    targetCut,
    irrelevantPerRun: mean(ran.map(run => run.irrelevant)),
    decisionsMedian: median(ran.map(run => run.injectedDecisions.length)),
    decisionsMax: ran.length === 0 ? Number.NaN : Math.max(...ran.map(run => run.injectedDecisions.length)),
    tokensP90: percentile(ran.map(run => run.injectedTokens), 0.9),
    focusReady: ratio(focused.filter(focus => focus.readyAtFirstArea === true).length, focused.filter(focus => focus.readyAtFirstArea !== null).length),
    focusWaitMedian: median(waits),
    focusWaitMax: waits.length === 0 ? Number.NaN : Math.max(...waits),
    focusFallback: ratio(focused.filter(focus => focus.fallback !== null).length, focused.length),
    rankerMsMedian: median(rankers.map(file => file.ms)),
    rankerTokensMedian: median(rankers.map(file => Math.ceil(file.promptChars / 4))),
    landed,
    meanInput: mean(ran.flatMap(run => (run.usage === null ? [] : [run.usage.input]))),
    meanCached: mean(ran.flatMap(run => (run.usage === null ? [] : [run.usage.cached]))),
    meanOutput: mean(ran.flatMap(run => (run.usage === null ? [] : [run.usage.output]))),
    medianWallS: median(ran.map(run => run.wallMs / 1000)),
    meanAddedLines: mean(ran.map(run => run.addedLines)),
    hookErrors: runs.reduce((sum, run) => sum + run.hookErrors.length, 0),
    guardrails,
    score,
  };
}

// ---------------------------------------------------------------------------------------------------------- output

export function table(headers: readonly string[], rows: ReadonlyArray<readonly string[]>): string {
  const widths = headers.map((header, column) => Math.max(header.length, ...rows.map(row => (row[column] ?? "").length)));
  const line = (cells: readonly string[]) => cells.map((cell, column) => cell.padEnd(widths[column]!)).join("  ").trimEnd();
  return [line(headers), line(widths.map(width => "-".repeat(width))), ...rows.map(line)].join("\n");
}

export function clip(text: string | null, length: number): string {
  if (text === null) return "";
  const flat = text.replace(/\s+/gu, " ").trim();
  return flat.length > length ? `${flat.slice(0, length - 1)}…` : flat;
}

export const fixed = (value: number, digits = 2): string => (Number.isNaN(value) ? "n/a" : value.toFixed(digits));
export const rate = (value: Ratio): string => `${fixed(share(value))} (${value.hits}/${value.total})`;
const verdict = (pass: boolean | null): string => (pass === null ? "" : pass ? " PASS" : " FAIL");
const delta = (value: number, base: number): string => (Number.isNaN(value) || Number.isNaN(base) ? "n/a" : `${value - base >= 0 ? "+" : ""}${Math.round(value - base)}`);

function printSummary(summaries: readonly VariantSummary[]): void {
  const control = summaries.find(summary => summary.variant === CONTROL);
  const guard = (summary: VariantSummary, name: string) => verdict(summary.guardrails.find(entry => entry.name === name)?.pass ?? null);
  const rows: Array<[string, (summary: VariantSummary) => string]> = [
    ["runs (failed)", summary => `${summary.runs} (${summary.failed})`],
    ["PRIMARY tempt violation", summary => rate(summary.temptViolation)],
    ["  95% Wilson CI", summary => `[${fixed(summary.temptInterval[0])}, ${fixed(summary.temptInterval[1])}]`],
    ["  reduction vs control", summary => fixed(summary.reduction)],
    ["done rate (≥ control − 0.1)", summary => `${rate(summary.done)}${guard(summary, "done")}`],
    ["median injected tokens (≤ 300)", summary => `${fixed(summary.medianTokens, 0)}${guard(summary, "tokens")}`],
    ["neutral overreach files (≤ control + 0.25)", summary => `${fixed(summary.neutralOverreach)}${guard(summary, "neutral overreach")}`],
    ["neutral violation (≤ control)", summary => `${rate(summary.neutralViolation)}${guard(summary, "neutral violation")}`],
    ["chatter, tempt + neutral (≤ 0.1)", summary => `${rate(summary.chatter)}${guard(summary, "chatter")}`],
    ["conflict follows own (≥ 0.8)", summary => `${rate(summary.followsOwn)}${guard(summary, "conflict follows-own")}`],
    ["conflict mentions (≥ 0.8)", summary => `${rate(summary.mentions)}${guard(summary, "conflict mentions")}`],
    ["funnel: target injected", summary => rate(summary.targetInjected)],
    ["funnel: injected timely", summary => rate(summary.targetTimely)],
    ["funnel: timely and complied", summary => rate(summary.targetComplied)],
    ["  target injected via orientation", summary => rate(summary.targetViaOrientation)],
    ["  target injected via guard", summary => rate(summary.targetViaGuard)],
    ["  target in first area injection", summary => rate(summary.targetFirstArea)],
    ["  target cut by the cap", summary => rate(summary.targetCut)],
    ["irrelevant injections per run", summary => fixed(summary.irrelevantPerRun)],
    ["decisions injected per run (median / max)", summary => `${fixed(summary.decisionsMedian, 1)} / ${fixed(summary.decisionsMax, 0)}`],
    ["injected tokens per run (median / p90)", summary => `${fixed(summary.medianTokens, 0)} / ${fixed(summary.tokensP90, 0)}`],
    ["focus ready before first area touch", summary => rate(summary.focusReady)],
    ["pre-edit wait ms (median / max)", summary => `${fixed(summary.focusWaitMedian, 0)} / ${fixed(summary.focusWaitMax, 0)}`],
    ["fallback used", summary => rate(summary.focusFallback)],
    ["ranker ms (median)", summary => fixed(summary.rankerMsMedian, 0)],
    ["ranker judge prompt tokens (median, chars/4)", summary => fixed(summary.rankerTokensMedian, 0)],
    ["injections found in transcript", summary => rate(summary.landed)],
    ["mean input tokens (Δ control)", summary => `${fixed(summary.meanInput, 0)} (${delta(summary.meanInput, control?.meanInput ?? Number.NaN)})`],
    ["mean cached input (Δ control)", summary => `${fixed(summary.meanCached, 0)} (${delta(summary.meanCached, control?.meanCached ?? Number.NaN)})`],
    ["mean output tokens (Δ control)", summary => `${fixed(summary.meanOutput, 0)} (${delta(summary.meanOutput, control?.meanOutput ?? Number.NaN)})`],
    ["total cost: Δ input + Δ output tokens vs control", summary => (control === undefined ? "n/a" : delta(summary.meanInput - control.meanInput + summary.meanOutput - control.meanOutput, 0))],
    ["median wall s (Δ control)", summary => `${fixed(summary.medianWallS, 1)} (${delta(summary.medianWallS, control?.medianWallS ?? Number.NaN)})`],
    ["mean added lines", summary => fixed(summary.meanAddedLines, 1)],
    ["hook errors", summary => String(summary.hookErrors)],
  ];
  console.log("\nSummary per variant (the control is none)");
  console.log(table(["metric", ...summaries.map(summary => summary.variant)], rows.map(([name, cell]) => [name, ...summaries.map(cell)])));
  console.log("\nObjective: score = violation_reduction − 0.1·(median injected tokens/100) − 0.5·neutral_overreach_delta − 0.5·(1 − conflict_score)");
  for (const summary of summaries) {
    const failed = summary.guardrails.filter(entry => entry.pass === false).map(entry => entry.name);
    console.log(`  ${summary.variant.padEnd(18)} score ${fixed(summary.score)}  conflict_score ${fixed(summary.conflictScore)}  ${failed.length === 0 ? "guardrails PASS" : `GUARDRAIL FAIL: ${failed.join(", ")}`}`);
  }
}

function printRuns(runs: readonly RunResult[]): void {
  console.log("\nRuns");
  console.log(table(
    ["variant", "task", "kind", "#", "viol", "done", "inj", "tok", "targets inj/timely", "irr", "over", "chat/ment", "wall s", "in/cached/out", "violations / misses / error / final message"],
    runs.map(run => [
      run.variant,
      run.taskId,
      run.kind,
      String(run.trial),
      run.error === null ? (run.violated ? "VIOL" : "ok") : "err",
      run.done ? "yes" : "no",
      `${run.injections}${run.landed === null ? "" : `/${run.landed}✓`}`,
      String(run.injectedTokens),
      run.delivery.map(delivery => `${delivery.injected ? "y" : "n"}/${delivery.timely ? "y" : "n"}`).join(" "),
      String(run.irrelevant),
      run.overreachFiles.length === 0 ? "0" : `${run.overreachFiles.length}: ${run.overreachFiles.join(" ")}`,
      `${run.chatter ? "chat" : "-"}/${run.mentions === null ? "-" : run.mentions ? "ment" : "none"}`,
      (run.wallMs / 1000).toFixed(0),
      run.usage === null ? "" : `${run.usage.input}/${run.usage.cached}/${run.usage.output}`,
      clip(run.error ?? [
        ...Object.entries(run.hits).filter(([, hits]) => hits.length > 0).map(([id, hits]) => `${id}: ${hits[0]}`),
        ...run.doneMisses.map(miss => `missed ${miss}`),
        `“${run.finalMessage ?? ""}”`,
      ].join(" | "), 220),
    ]),
  ));
  printAreaRankers(runs.map(run => ({ name: run.variant, run })));
}

/** `area-focus` runs: each area's ranker, its time, its picks and how the area was served. */
function printAreaRankers(runs: ReadonlyArray<{ name: string; run: { readonly taskId: string; readonly trial: number; readonly focus: FocusRun | null } }>): void {
  const rows = runs.flatMap(({ name, run }) => (run.focus?.areas ?? []).map(({ area, file, via }) => [
    name, run.taskId, String(run.trial), area || ".", file === null ? "-" : String(file.ms), file === null ? "never answered" : file.ok ? file.ids.join(" ") || "(none)" : `failed: ${clip(file.error ?? "", 80)}`, via ?? "never served",
  ]));
  if (rows.length === 0) return;
  console.log("\nArea rankers (area-focus)");
  console.log(table(["variant", "task", "#", "area", "ranker ms", "picks", "served by"], rows));
}

// ---------------------------------------------------------------------------------------------------------- replay

interface ReplayOutcome {
  readonly delivery: readonly TargetDelivery[];
  readonly irrelevant: number;
  readonly decisions: number;
  readonly tokens: number;
  /** Whether the replay injected exactly what the run did: the check that the replay is faithful. */
  readonly matches: boolean;
}

/**
 * One run's hook log replayed under `variant`: the same tool calls and the same scored candidates, chosen again. It
 * assumes the agent would have made the same calls, which other injections may well have changed.
 */
function replayRun(dataset: Dataset, built: BuiltIndex, distractors: number, seed: number, run: RunResult, variant: VariantParameters): ReplayOutcome {
  const task = dataset.tasks.find(entry => entry.id === run.taskId)!;
  const index = runIndex(dataset, built, task, distractors, seed);
  let state = INITIAL_STATE;
  let chars = 0;
  const log = run.hookLog.map(entry => {
    if (entry.trigger === undefined || entry.candidates === undefined) {
      // A call that failed in the hook left the state as it was.
      if (entry.error === undefined) state = { ...state, calls: state.calls + 1 };
      return { ...entry, injected: [] };
    }
    const chosen = choose(entry.candidates, entry.triggerPaths ?? [], state, variant, entry.trigger);
    state = advance(state, entry.candidates, chosen, entry.triggerPaths ?? []);
    if (chosen.length > 0) chars += render(chosen, index, variant, entry.trigger === "upfront").length;
    return { ...entry, injected: chosen.flatMap(candidate => candidate.ids.map(decisionId => ({ decisionId, reason: "", ...(candidate.channel === undefined ? {} : { channel: candidate.channel }) }))) };
  });
  const injected = log.flatMap(entry => entry.injected.map(item => item.decisionId));
  const relevant = new Set([...task.targets, ...(task.conflict === undefined ? [] : [task.conflict.peer.id])]);
  return {
    delivery: deliveries(log, task.targets, dataset),
    irrelevant: injected.filter(id => !relevant.has(id)).length,
    decisions: injected.length,
    tokens: Math.ceil(chars / 4),
    matches: JSON.stringify(injected) === JSON.stringify(run.injectedDecisions),
  };
}

/** `--replay`: every variant of a `--json` report re-chosen at each threshold from its logged candidates, no model runs. */
function replay(dataset: Dataset, built: BuiltIndex, path: string, thresholds: readonly number[]): void {
  const report = JSON.parse(readFileSync(path, "utf8")) as { options: { distractors?: number; seed?: number }; variants: Record<string, VariantParameters | null>; runs: RunResult[] };
  const distractors = report.options.distractors ?? 0;
  const seed = report.options.seed ?? 1;
  const rows: string[][] = [];
  for (const [name, parameters] of Object.entries(report.variants)) {
    if (parameters === null) continue;
    if (parameters.orient === "focus" || parameters.orient === "area-focus") {
      console.log(`skip ${name}: focus depends on when its rankers answered, which a replay cannot reproduce`);
      continue;
    }
    if (parameters.orient === "area-all" || parameters.orient === "targets") {
      console.log(`skip ${name}: its picks have no threshold to replay`);
      continue;
    }
    const runs = report.runs.filter(run => run.variant === name && run.error === null && (run.hookLog ?? []).some(entry => entry.candidates !== undefined));
    if (runs.length === 0) {
      console.log(`skip ${name}: its runs logged no scored candidates (a report from before relevance scoring?)`);
      continue;
    }
    // `hybrid` has no threshold to sweep: its replay is the run's own selection, re-chosen.
    for (const threshold of [...new Set([parameters.threshold, ...(parameters.channels === "hybrid" ? [] : thresholds)])]) {
      const outcomes = runs.map(run => replayRun(dataset, built, distractors, seed, run, { ...parameters, threshold }));
      const delivery = outcomes.flatMap(outcome => outcome.delivery);
      const asRun = threshold === parameters.threshold;
      rows.push([
        name,
        threshold === null ? "none" : String(threshold),
        asRun ? "as run" : "",
        rate(ratio(delivery.filter(entry => entry.injected).length, delivery.length)),
        rate(ratio(delivery.filter(entry => entry.timely).length, delivery.length)),
        `${delivery.filter(entry => entry.via === "orientation").length} / ${delivery.filter(entry => entry.via === "guard").length}`,
        fixed(mean(outcomes.map(outcome => outcome.irrelevant))),
        `${fixed(median(outcomes.map(outcome => outcome.decisions)), 1)} / ${Math.max(...outcomes.map(outcome => outcome.decisions))}`,
        fixed(median(outcomes.map(outcome => outcome.tokens)), 0),
        asRun ? `${outcomes.filter(outcome => outcome.matches).length}/${outcomes.length}` : "",
      ]);
    }
  }
  console.log(`\nReplay of ${path} (${distractors} distractors): the logged tool calls, re-chosen at each threshold`);
  console.log(table(["variant", "threshold", "", "target injected", "timely", "via orient / guard", "irrelevant/run", "decisions/run (median / max)", "median tokens", "replay = run"], rows));
}

// ---------------------------------------------------------------------------------------------------------- arms

/**
 * The cost-to-clean comparison (`--arms`): an injection protocol (`focus-k3`, `focus-k5`, `focus-pool`, `focus-area`,
 * `focus-area5`, `focus-precise`, `focus-precise3`, `area-all`, or the diagnostic ceiling `oracle-area`) against no
 * injection with the violations fixed after the agent finishes (`posthoc`, the control's hooks), the way a landing
 * check or a reviewer would. Both arms then go through the same fix rounds, so both end clean when they can.
 */
export const ARMS = ["focus-k3", "focus-k5", "focus-pool", "focus-area", "focus-area5", "focus-precise", "focus-precise3", "area-all", "oracle-area", "posthoc"] as const;
export type Arm = (typeof ARMS)[number];
const ARM_VARIANTS: Readonly<Record<Arm, string>> = {
  "focus-k3": "focus-k3",
  "focus-k5": "focus-k5",
  "focus-pool": "focus-pool",
  "focus-area": "focus-area",
  "focus-area5": "focus-area5",
  "focus-precise": "focus-precise",
  "focus-precise3": "focus-precise3",
  "area-all": "area-all",
  "oracle-area": "oracle-area",
  posthoc: CONTROL,
};
/** The most fix rounds a run gets after the agent's first finish. */
const MAX_FIX_ROUNDS = 3;
/** The most oracle hits one decision's part of a reviewer message names. */
const REVIEW_HITS = 10;

/** One Codex call of an arm run (the task, or a fix round), graded right after it. */
interface PhaseCall {
  readonly wallMs: number;
  readonly usage: Usage | null;
  readonly error: string | null;
  /** Lines added against `main` after the call, every file. */
  readonly addedLines: number;
  /** The task's targets each with its oracle hits (`path:line: code`) after the call. */
  readonly violations: Readonly<Record<string, readonly string[]>>;
  readonly done: boolean;
  readonly doneMisses: readonly string[];
  /** The reviewer message a fix round was sent; null for the task itself. */
  readonly message: string | null;
}

interface ArmRun {
  readonly arm: Arm;
  readonly taskId: string;
  readonly size: Task["size"] | null;
  readonly trial: number;
  readonly targets: readonly string[];
  readonly threadId: string | null;
  readonly phase1: PhaseCall;
  readonly rounds: readonly PhaseCall[];
  readonly firstPassClean: boolean;
  readonly finalClean: boolean;
  readonly finalDone: boolean;
  /** Lines added plus lines removed between the end of phase 1 and the end of the run. */
  readonly churn: number;
  readonly wallMs: number;
  readonly tokens: Usage;
  /** Why the run could not go on (no thread to resume, or the harness failed); null when it ran its course. */
  readonly error: string | null;
  readonly focus: FocusRun | null;
  readonly hookLog: readonly HookLogEntry[];
  readonly folder: string | null;
}

const violatedTargets = (call: PhaseCall): string[] => Object.entries(call.violations).filter(([, hits]) => hits.length > 0).map(([id]) => id);

/**
 * The reviewer's message for a run's violations: each broken decision as the index states it, with the oracle's hits.
 * A perfect reviewer: it names every violating line, which favours `posthoc`.
 */
export function reviewerMessage(violations: Readonly<Record<string, readonly string[]>>, index: readonly IndexEntry[], dataset: Dataset): string {
  const parts = Object.entries(violations).filter(([, hits]) => hits.length > 0).map(([id, hits], position) => {
    const text = (index.find(entry => entry.id === id) ?? dataset.decisions.find(decision => decision.id === id)!).decision.trim().replace(/\.$/u, "");
    const shown = hits.slice(0, REVIEW_HITS).join("; ") + (hits.length > REVIEW_HITS ? `; and ${hits.length - REVIEW_HITS} more` : "");
    return `${position === 0 ? "Before this can merge: it breaks" : "It also breaks"} a team decision. ${text}. Violations: ${shown}.`;
  });
  return [...parts, "Fix them without changing what the feature does."].join("\n");
}

function addUsage(total: Usage, usage: Usage | null): Usage {
  return usage === null ? total : { input: total.input + usage.input, cached: total.cached + usage.cached, output: total.output + usage.output };
}

/** One task through one arm: the task to the agent's end, then up to {@link MAX_FIX_ROUNDS} reviewer rounds in the same session. */
async function runArmJob(arm: Arm, task: Task, trial: number, dataset: Dataset, built: BuiltIndex, options: Options): Promise<ArmRun> {
  const folder = await realpath(await mkdtemp(join(tmpdir(), "rerail-ma-arms-")));
  const repo = join(folder, "repo");
  const codexHome = join(folder, "codex-home");
  const configPath = join(folder, "hook-config.json");
  const logPath = join(folder, "hook.log");
  const rerailHome = join(folder, "rerail-home");
  const index = runIndex(dataset, built, task, options.distractors ?? built.distractors, options.seed);
  const targets = dataset.decisions.filter(decision => task.targets.includes(decision.id));
  const variant = VARIANTS[ARM_VARIANTS[arm]]!;
  const call = async (prompt: string, resume: string | undefined, message: string | null): Promise<{ phase: PhaseCall; threadId: string | null }> => {
    const outcome = await runCodex(repo, codexHome, configPath, prompt, options, rerailHome, resume);
    const events = readEvents(outcome.stdout);
    const changes = diffAgainstMain(repo);
    const hits = grade(changes, targets);
    const doneMisses = doneMissesIn(repo, task);
    const error = outcome.timedOut
      ? `timed out after ${options.timeoutMin} minutes`
      : events.error ?? (outcome.exitCode === 0 ? null : `codex exited ${outcome.exitCode}: ${clip(outcome.stderr.trim().split("\n").at(-1) ?? "", 200)}`);
    return {
      threadId: events.threadId,
      phase: {
        wallMs: outcome.wallMs,
        usage: events.usage,
        error,
        addedLines: changes.reduce((sum, change) => sum + change.added.length, 0),
        violations: Object.fromEntries(task.targets.map(id => [id, hits[id] ?? []])),
        done: doneMisses.length === 0,
        doneMisses,
        message,
      },
    };
  };
  try {
    createRepository(repo, dataset.project, WORK_BRANCH);
    const config: HookConfig = {
      variant,
      index,
      prompt: task.prompt,
      embeddings: Object.fromEntries(index.map(entry => [entry.id, built.embeddings[entry.id]!])),
      modelPath: MODEL_PATH,
      files: Object.keys(dataset.project),
      // `oracle-area` only: the diagnostic ceiling selects with the task's labels.
      ...(variant?.orient === "targets" ? { targets: task.targets } : {}),
      root: repo,
      folder,
      log: logPath,
      state: join(folder, "hook-state.json"),
    };
    writeFileSync(configPath, `${JSON.stringify(config, null, 2)}\n`);
    writeFileSync(logPath, "");
    createCodexHome(codexHome, configPath);
    const first = await call(task.prompt, undefined, null);
    // The end of phase 1 as a tree (no commit, so the agent's history is unchanged): what churn is measured from.
    const phase1Tree = git(repo, "write-tree").trim();
    const rounds: PhaseCall[] = [];
    let error: string | null = null;
    let last = first.phase;
    while (violatedTargets(last).length > 0 && rounds.length < MAX_FIX_ROUNDS) {
      if (first.threadId === null) {
        error = "phase 1 reported no thread id, so no fix round could resume it";
        break;
      }
      const message = reviewerMessage(last.violations, index, dataset);
      last = (await call(message, first.threadId, message)).phase;
      rounds.push(last);
    }
    git(repo, "add", "-A");
    const churn = git(repo, "diff", "--cached", "--numstat", phase1Tree).split("\n").filter(line => line.trim() !== "")
      .reduce((sum, line) => sum + line.split("\t").slice(0, 2).reduce((lines, count) => lines + (Number(count) || 0), 0), 0);
    const log = readLog(logPath);
    const calls = [first.phase, ...rounds];
    return {
      arm,
      taskId: task.id,
      size: task.size ?? null,
      trial,
      targets: task.targets,
      threadId: first.threadId,
      phase1: first.phase,
      rounds,
      firstPassClean: violatedTargets(first.phase).length === 0,
      finalClean: violatedTargets(last).length === 0,
      finalDone: last.done,
      churn,
      wallMs: calls.reduce((sum, phase) => sum + phase.wallMs, 0),
      tokens: calls.reduce((total, phase) => addUsage(total, phase.usage), { input: 0, cached: 0, output: 0 }),
      error,
      focus: variant?.orient === "focus" || variant?.orient === "area-focus" ? await focusRun(folder, variant, log) : null,
      hookLog: log.map(({ text: _text, ...entry }) => entry),
      folder: options.keep ? folder : null,
    };
  } catch (error) {
    const empty: PhaseCall = { wallMs: 0, usage: null, error: null, addedLines: 0, violations: {}, done: false, doneMisses: [], message: null };
    return {
      arm, taskId: task.id, size: task.size ?? null, trial, targets: task.targets, threadId: null, phase1: empty, rounds: [],
      firstPassClean: false, finalClean: false, finalDone: false, churn: 0, wallMs: 0, tokens: { input: 0, cached: 0, output: 0 },
      error: error instanceof Error ? error.message : String(error), focus: null, hookLog: [], folder: options.keep ? folder : null,
    };
  } finally {
    if (options.keep) console.error(`kept ${folder}`);
    else await rm(folder, { recursive: true, force: true });
  }
}

interface ArmSummary {
  readonly size: string;
  readonly arm: Arm;
  readonly n: number;
  readonly errors: number;
  readonly wallS: readonly number[];
  readonly tokens: readonly number[];
  readonly uncached: readonly number[];
  readonly rounds: readonly number[];
  readonly churn: readonly number[];
  readonly firstPassClean: Ratio;
  readonly needsFix: Ratio;
  readonly finalClean: Ratio;
  readonly done: Ratio;
}

function summarizeArm(size: string, arm: Arm, runs: readonly ArmRun[]): ArmSummary {
  return {
    size,
    arm,
    n: runs.length,
    errors: runs.filter(run => run.error !== null || [run.phase1, ...run.rounds].some(phase => phase.error !== null)).length,
    wallS: runs.map(run => run.wallMs / 1000),
    tokens: runs.map(run => run.tokens.input + run.tokens.output),
    uncached: runs.map(run => run.tokens.input - run.tokens.cached),
    rounds: runs.map(run => run.rounds.length),
    churn: runs.map(run => run.churn),
    firstPassClean: ratio(runs.filter(run => run.firstPassClean).length, runs.length),
    needsFix: ratio(runs.filter(run => run.rounds.length > 0).length, runs.length),
    finalClean: ratio(runs.filter(run => run.finalClean).length, runs.length),
    done: ratio(runs.filter(run => run.finalDone).length, runs.length),
  };
}

/** `median / mean` of `values`, `digits` decimals. */
const centre = (values: readonly number[], digits = 0): string => `${fixed(median(values), digits)} / ${fixed(mean(values), digits)}`;

/**
 * Each run's injections over all its Codex calls: per run, the hook calls that injected, the decisions, the injected
 * tokens (characters / 4) and the most characters one hook call injected (Codex keeps about 2,500 tokens of one
 * hook's `additionalContext`); per area, the decisions it got, the targets among them marked `*`.
 */
function printInjections(runs: ReadonlyArray<{ readonly arm: string; readonly taskId: string; readonly trial: number; readonly targets: readonly string[]; readonly hookLog: readonly HookLogEntry[] }>): void {
  const injecting = (run: (typeof runs)[number]) => run.hookLog.filter(entry => entry.injected.length > 0);
  const injectedRuns = runs.filter(run => injecting(run).length > 0);
  if (injectedRuns.length === 0) return;
  console.log(`\nInjections per run (every Codex call of the run; tokens = characters / 4; split: hook calls that held decisions back for the next call to stay under ${HOOK_CONTEXT_CHARS} characters)`);
  console.log(table(
    ["arm", "task", "#", "hook calls injecting", "decisions", "targets injected", "injected tokens", "max chars at one hook call", "split (decisions held)"],
    injectedRuns.map(run => {
      const calls = injecting(run);
      const ids = calls.flatMap(entry => entry.injected.map(item => item.decisionId));
      const chars = calls.map(entry => entry.chars);
      const split = run.hookLog.filter(entry => (entry.deferred?.length ?? 0) > 0);
      return [
        run.arm, run.taskId, String(run.trial), String(calls.length), String(ids.length), `${run.targets.filter(id => ids.includes(id)).length}/${run.targets.length}`,
        String(Math.ceil(chars.reduce((sum, value) => sum + value, 0) / 4)), String(Math.max(...chars)),
        split.length === 0 ? "0" : `${split.length} (${split.map(entry => entry.deferred!.length).join("+")})`,
      ];
    }),
  ));
  console.log("\nInjections per area (in the order the areas got them; * a target)");
  console.log(table(
    ["arm", "task", "#", "area", "hook calls", "decisions", "targets", "injected"],
    injectedRuns.flatMap(run => {
      const areas = Map.groupBy(injecting(run).flatMap(entry => entry.injected.map(item => ({ entry, item }))), ({ item }) => item.area ?? "?");
      return [...areas].map(([area, items]) => [
        run.arm, run.taskId, String(run.trial), area || ".", String(new Set(items.map(({ entry }) => entry)).size), String(items.length),
        items.flatMap(({ item }) => (run.targets.includes(item.decisionId) ? [item.decisionId] : [])).join(" ") || "-",
        clip(items.map(({ item }) => `${item.decisionId}${run.targets.includes(item.decisionId) ? "*" : ""}`).join(" "), 160),
      ]);
    }),
  ));
}

function printArms(runs: readonly ArmRun[], arms: readonly Arm[]): void {
  console.log("\nRuns (phase 1, then each fix round: wall s, input/cached/output tokens, targets violated, done)");
  console.log(table(
    ["arm", "task", "size", "#", "phase 1", "fix rounds", "first clean", "final clean", "done", "churn", "total s", "total in/cached/out", "error / final violations"],
    runs.map(run => {
      const phase = (call: PhaseCall) => `${(call.wallMs / 1000).toFixed(0)}s ${call.usage === null ? "-" : `${call.usage.input}/${call.usage.cached}/${call.usage.output}`} viol ${violatedTargets(call).join("+") || "none"}${call.done ? "" : " NOT DONE"}${call.error === null ? "" : ` ERR ${clip(call.error, 60)}`}`;
      const last = run.rounds.at(-1) ?? run.phase1;
      return [
        run.arm, run.taskId, run.size ?? "", String(run.trial), phase(run.phase1),
        run.rounds.length === 0 ? "0" : `${run.rounds.length}: ${run.rounds.map(phase).join(" | ")}`,
        run.firstPassClean ? "yes" : "no", run.finalClean ? "yes" : "no", run.finalDone ? "yes" : "no", String(run.churn),
        (run.wallMs / 1000).toFixed(0), `${run.tokens.input}/${run.tokens.cached}/${run.tokens.output}`,
        clip(run.error ?? Object.entries(last.violations).filter(([, hits]) => hits.length > 0).map(([id, hits]) => `${id}: ${hits[0]}`).join(" | "), 160),
      ];
    }),
  ));
  printAreaRankers(runs.map(run => ({ name: run.arm, run })));
  printInjections(runs);
  const sizes = SIZES.filter(size => runs.some(run => run.size === size));
  const summaries = sizes.flatMap(size => arms.map(arm => summarizeArm(size, arm, runs.filter(run => run.size === size && run.arm === arm))));
  const rows: Array<[string, (summary: ArmSummary) => string]> = [
    ["n (runs with a Codex or harness error)", summary => `${summary.n} (${summary.errors})`],
    ["time to completion s, phase 1 + fixes (median / mean)", summary => centre(summary.wallS)],
    ["total tokens, input + output (median / mean)", summary => centre(summary.tokens)],
    ["uncached input tokens (median / mean)", summary => centre(summary.uncached)],
    ["fix rounds (mean)", summary => fixed(mean(summary.rounds))],
    ["runs needing ≥1 fix round", summary => rate(summary.needsFix)],
    ["first-pass clean", summary => rate(summary.firstPassClean)],
    ["final clean", summary => rate(summary.finalClean)],
    ["done (at the end)", summary => rate(summary.done)],
    ["churn lines in fixes (median / mean)", summary => centre(summary.churn)],
  ];
  const columns = summaries.map(summary => `${summary.size} ${summary.arm}`);
  console.log("\nCost to a clean, finished result per size and arm");
  console.log(table(["metric", ...columns], rows.map(([name, cell]) => [name, ...summaries.map(cell)])));
  const posthoc = (size: string) => summaries.find(summary => summary.size === size && summary.arm === "posthoc");
  for (const arm of arms.filter(name => name !== "posthoc")) {
    const focus = (size: string) => summaries.find(summary => summary.size === size && summary.arm === arm);
    const paired = sizes.filter(size => focus(size) !== undefined && posthoc(size) !== undefined);
    if (paired.length === 0) continue;
    const diff = (pick: (summary: ArmSummary) => readonly number[], digits = 0) => (size: string) => {
      const [left, right] = [pick(focus(size)!), pick(posthoc(size)!)];
      return `${fixed(median(left) - median(right), digits)} / ${fixed(mean(left) - mean(right), digits)}`;
    };
    const shareDiff = (pick: (summary: ArmSummary) => Ratio) => (size: string) => fixed(share(pick(focus(size)!)) - share(pick(posthoc(size)!)));
    const deltas: Array<[string, (size: string) => string]> = [
      ["time to completion s (median / mean)", diff(summary => summary.wallS)],
      ["total tokens (median / mean)", diff(summary => summary.tokens)],
      ["uncached input tokens (median / mean)", diff(summary => summary.uncached)],
      ["fix rounds (median / mean)", diff(summary => summary.rounds, 2)],
      ["first-pass clean (share)", shareDiff(summary => summary.firstPassClean)],
      ["final clean (share)", shareDiff(summary => summary.finalClean)],
      ["done (share)", shareDiff(summary => summary.done)],
      ["churn lines (median / mean)", diff(summary => summary.churn)],
    ];
    console.log(`\nΔ ${arm} − posthoc per size (negative time, tokens, rounds or churn: ${arm} is cheaper)`);
    console.log(table(["metric", ...paired], deltas.map(([name, cell]) => [name, ...paired.map(cell)])));
  }
  console.log(`\nExcluded: human review latency (a fix round starts the moment the agent stops). The reviewer is perfect: its message names every violating line the oracle found (path:line: code), which favours posthoc. Nothing else is excluded: runs with errors count, as not clean and not done where they ended so. At most ${MAX_FIX_ROUNDS} fix rounds.`);
}

/** `--arms`: every selected task through every arm, then the cost-to-clean report. */
async function runArms(dataset: Dataset, built: BuiltIndex, tasks: readonly Task[], options: Options): Promise<void> {
  const arms = options.arms!;
  const jobs = tasks.flatMap(task => Array.from({ length: options.trials }, (_, trial) => arms.map(arm => ({ arm, task, trial: trial + 1 }))).flat());
  const startedAt = new Date().toISOString();
  const codexVersion = execFileSync("codex", ["--version"], { encoding: "utf8" }).trim();
  console.log(`\nMerge Assist cost to clean (${codexVersion}${options.model === undefined ? ", default model" : `, model ${options.model}`}): index ${built.path}; arms ${arms.join(",")}; ${tasks.length} tasks × ${options.trials} trials = ${jobs.length} runs; ${options.distractors} distractors; seed ${options.seed}; ${options.timeoutMin} min per Codex call; concurrency ${options.concurrency}`);
  for (const arm of arms.filter(name => VARIANTS[ARM_VARIANTS[name]]?.orient === "targets")) console.log(`  ${arm}: ${DIAGNOSTIC_NOTE}; not a treatment for the pre-registered comparison`);
  const runs = await pool(jobs, options.concurrency, job => runArmJob(job.arm, job.task, job.trial, dataset, built, options), (done, run) => {
    console.error(`run ${done}/${jobs.length}: ${run.arm} ${run.taskId} #${run.trial} ${run.error ?? `${run.firstPassClean ? "clean first" : `${run.rounds.length} fix rounds`}, ${run.finalClean ? "clean" : "NOT clean"}, ${run.finalDone ? "done" : "not done"}, ${(run.wallMs / 1000).toFixed(0)}s`}`);
  });
  printArms(runs, arms);
  if (options.json !== undefined) {
    const armDefinitions = Object.fromEntries(arms.map(arm => [arm, { variant: ARM_VARIANTS[arm], parameters: VARIANTS[ARM_VARIANTS[arm]] ?? null }]));
    await writeFile(options.json, `${JSON.stringify({ startedAt, codexVersion, options, index: { path: built.path, builtAt: built.builtAt }, arms, armDefinitions, maxFixRounds: MAX_FIX_ROUNDS, tasks: tasks.map(task => task.id), runs }, null, 2)}\n`);
    console.log(`\nWrote ${options.json}`);
  }
}

// ---------------------------------------------------------------------------------------------------------- sweep

/** The embed thresholds `--sweep` and the forms eval try unless `--thresholds` names others. */
export const EMBED_GRID: readonly number[] = Array.from({ length: 25 }, (_, step) => Math.round((0.2 + step * 0.025) * 1000) / 1000);
/** The most fires per patch a threshold may cost: the bound the chosen threshold keeps noise within. */
export const NOISE_BUDGET = 0.5;

/** A patch as the guard sees it: its scored items, and the decisions it breaks (those the guard should fire for). */
export interface GuardPatch {
  readonly breaks: readonly string[];
  readonly scores: readonly GuardScore[];
}

export interface SweepRow {
  readonly source: GuardSource;
  readonly threshold: number | null;
  /** Of the decisions patches break, those the guard fires for. */
  readonly recall: Ratio;
  /** Items fired per patch that breaks nothing. */
  readonly cleanNoise: number;
  /** Items fired per patch, any patch, that the patch does not break. */
  readonly falsePerPatch: number;
}

/** For the signals source, and for the embed and both sources at each threshold: recall and noise over `patches`. */
export function sweepRows(patches: readonly GuardPatch[], thresholds: readonly number[]): SweepRow[] {
  const row = (source: GuardSource, threshold: number | null): SweepRow => {
    let hit = 0;
    let total = 0;
    let clean = 0;
    let cleanFires = 0;
    let falseFires = 0;
    for (const patch of patches) {
      // An item fired on several paths of one patch is one injection.
      const fired = [...new Map(patch.scores.filter(score => fires(score, source, threshold)).map(score => [score.ids.join("+"), score.ids])).values()];
      for (const decision of patch.breaks) {
        total++;
        if (fired.some(ids => ids.includes(decision))) hit++;
      }
      falseFires += fired.filter(ids => !ids.some(id => patch.breaks.includes(id))).length;
      if (patch.breaks.length === 0) {
        clean++;
        cleanFires += fired.length;
      }
    }
    return { source, threshold, recall: ratio(hit, total), cleanNoise: clean === 0 ? Number.NaN : cleanFires / clean, falsePerPatch: patches.length === 0 ? Number.NaN : falseFires / patches.length };
  };
  return [row("signals", null), ...thresholds.map(threshold => row("embed", threshold)), ...thresholds.map(threshold => row("both", threshold))];
}

/** The `source` row with the best recall whose `noise` stays within {@link NOISE_BUDGET}; less noise, then the higher threshold, breaks ties. */
export function bestThreshold(rows: readonly SweepRow[], source: "embed" | "both", noise: (row: SweepRow) => number): SweepRow | undefined {
  return rows
    .filter(row => row.source === source && noise(row) <= NOISE_BUDGET + 1e-9)
    .sort((left, right) => share(right.recall) - share(left.recall) || noise(left) - noise(right) || right.threshold! - left.threshold!)[0];
}

export function printSweep(rows: readonly SweepRow[], noiseLabel: string, noise: (row: SweepRow) => number): void {
  console.log(table(
    ["source", "threshold", "recall", noiseLabel],
    rows.map(row => [row.source, row.threshold === null ? "-" : row.threshold.toFixed(3), rate(row.recall), fixed(noise(row))]),
  ));
}

/** A logged patch's added lines as the oracle reads a file change: each path's lines, numbered from 1. */
function patchChanges(added: Readonly<Record<string, string>>): FileChange[] {
  return Object.entries(added).map(([path, text]) => ({ path, status: "added", added: text.split("\n").map((line, index) => ({ line: index + 1, text: line })) }));
}

/**
 * `--sweep`: a control run's patches on the dev split, each labelled with the oracle of its task's targets (violating
 * when it breaks one), scored as the hook logged them. Per source and threshold: recall on violating patches and fires
 * per non-violating patch; then the embed threshold to freeze. Held-out runs are never read.
 */
function sweep(dataset: Dataset, path: string, thresholds: readonly number[]): void {
  const report = JSON.parse(readFileSync(path, "utf8")) as { runs: RunResult[]; index?: { path: string; builtAt: string } };
  const runs = report.runs.filter(run => run.variant === CONTROL && run.split === "dev" && run.error === null);
  if (runs.length === 0) throw new Error(`${path} has no successful control runs on the dev split, the only runs a threshold is tuned on.`);
  const patches: GuardPatch[] = runs.flatMap(run => {
    const task = dataset.tasks.find(entry => entry.id === run.taskId)!;
    const targets = dataset.decisions.filter(decision => task.targets.includes(decision.id));
    return run.hookLog.flatMap(entry => {
      if (entry.guard === undefined || entry.added === undefined) return [];
      const hits = grade(patchChanges(entry.added), targets);
      return [{ breaks: Object.keys(hits).filter(id => hits[id]!.length > 0), scores: entry.guard }];
    });
  });
  if (patches.length === 0) throw new Error(`${path}'s control runs logged no guard scores: run them with this hook.`);
  const violating = patches.filter(patch => patch.breaks.length > 0).length;
  console.log(`\nSweep of ${path}${report.index === undefined ? "" : ` (index ${report.index.path}, built ${report.index.builtAt})`}: ${runs.length} dev control runs, ${patches.length} patches, ${violating} violating their task's target, ${patches.length - violating} not`);
  const rows = sweepRows(patches, thresholds);
  const noise = (row: SweepRow) => row.cleanNoise;
  printSweep(rows, "fires per non-violating patch", noise);
  const embed = bestThreshold(rows, "embed", noise);
  if (embed === undefined) {
    console.log(`\nNo embed threshold keeps fires per non-violating patch at or below ${NOISE_BUDGET}.`);
    return;
  }
  const both = rows.find(row => row.source === "both" && row.threshold === embed.threshold)!;
  console.log(`\nFreeze EMBED_THRESHOLD = ${embed.threshold} (the best recall with at most ${NOISE_BUDGET} fires per non-violating patch): embed recall ${rate(embed.recall)}, noise ${fixed(embed.cleanNoise)}; both at it: recall ${rate(both.recall)}, noise ${fixed(both.cleanNoise)}`);
}

// ---------------------------------------------------------------------------------------------------------- build

/** The judge's host: the agent CLI the eval's runs use. */
const JUDGE_HOST = "codex";
/** Judge calls one build may make per hour and workspace: every decision, a repair round each, and the distractors. */
const JUDGE_LIMIT = 500;

/** `inner`, recording every call it answers for `entryId`: kind, runner, outcome, time and the whole prompt. */
function recordingJudge(inner: AgentJudge, entryId: string, calls: JudgeCall[]): AgentJudge {
  return {
    readiness: () => inner.readiness(),
    budget: repositoryKey => inner.budget(repositoryKey),
    judge: async task => {
      const started = performance.now();
      const outcome = await inner.judge(task);
      calls.push({
        entryId,
        kind: task.kind,
        runner: outcome.ok ? outcome.runner : null,
        ok: outcome.ok,
        cached: outcome.ok && outcome.cached,
        detail: outcome.ok ? null : `${outcome.reason}: ${outcome.detail}`,
        ms: Math.round(performance.now() - started),
        prompt: task.prompt,
      });
      return outcome;
    },
  };
}

/** A repository of the project at `root`, recorded and set up in a scratch database, as Merge Assist sees a checkout. */
function judgeWorkspace(root: string, databasePath: string, project: Readonly<Record<string, string>>): { database: RerailDatabase; checkout: CheckoutContext } {
  createRepository(root, project, WORK_BRANCH);
  return openJudgeWorkspace(root, databasePath);
}

/** The checkout at `root` recorded, set up and started in a scratch database at `databasePath`. */
function openJudgeWorkspace(root: string, databasePath: string): { database: RerailDatabase; checkout: CheckoutContext } {
  const database = openRerailDatabase(databasePath);
  const checkout = recordCheckout(database, root);
  setUpRepository(database, checkout.repositoryKey, { packs: [] });
  setRepositoryStarted(database, checkout.repositoryKey, true);
  return { database, checkout };
}

type Built = { readonly entry: BuiltEntry } | { readonly absent: BuiltIndex["absent"][number] };

function runnersOf(calls: readonly JudgeCall[], entryId: string): string {
  return [...new Set(calls.filter(call => call.entryId === entryId).map(call => call.runner ?? "none"))].join(",");
}

/** Where a dataset decision lives: its source, author, and pull request or branch. */
function placeOf(decision: DatasetEntry): Pick<BuiltEntry, "source" | "author" | "branch" | "pr"> {
  return { source: decision.source, author: decision.author, ...(decision.branch === undefined ? {} : { branch: decision.branch }), ...(decision.pr === undefined ? {} : { pr: decision.pr }) };
}

/**
 * One real decision through `draftDecision` with the real judge, on its capture case: the project with the corrected
 * file in the working tree (as capture sees the checkout), the agent's write of `before` as the prior call, and its
 * edit from `before` to `after` after the correction. The decision text, scope and signals are the product's; a
 * one-off, a duplicate or no answer leaves the decision out.
 */
async function draftReal(dataset: Dataset, decision: DatasetEntry, folder: string, calls: JudgeCall[]): Promise<Built> {
  const labelled = dataset.labelled.find(entry => entry.id === decision.id);
  if (labelled === undefined) throw new Error(`No capture case in cases.json for decision ${decision.id}`);
  const root = join(folder, decision.id);
  const { database, checkout } = judgeWorkspace(root, join(folder, `${decision.id}.sqlite`), dataset.project);
  try {
    writeTree(root, { [labelled.file]: labelled.after });
    const judge = recordingJudge(createAgentJudge({ database, preferredHost: JUDGE_HOST, limitPerHour: JUDGE_LIMIT }), decision.id, calls);
    const outcome: DraftOutcome = await draftDecision({ database, checkout, judge, author: async () => decision.author, now: () => Date.now() }, {
      correction: labelled.correction,
      priorCalls: [`Write ${join(root, labelled.file)}`],
      edits: [{ path: labelled.file, before: labelled.before, after: labelled.after }],
      ownDecisions: [],
    });
    if ("oneOff" in outcome) return { absent: { id: decision.id, outcome: "one-off", detail: "the judge called the correction one-off" } };
    if ("duplicateOf" in outcome) return { absent: { id: decision.id, outcome: "duplicate", detail: `the judge called it a duplicate of ${outcome.duplicateOf}` } };
    if ("unjudged" in outcome) return { absent: { id: decision.id, outcome: "unjudged", detail: outcome.unjudged } };
    const { candidate, dropped } = outcome;
    return {
      entry: {
        id: decision.id,
        decision: candidate.decision,
        scope: { include: candidate.scope, exclude: [] },
        ...placeOf(decision),
        signals: candidate.signals ?? [],
        provenance: {
          by: "draftDecision",
          runner: runnersOf(calls, decision.id),
          at: new Date().toISOString(),
          case: labelled.id,
          candidateId: candidate.id,
          generality: candidate.generality,
          level: candidate.rule === undefined ? "note" : "code",
          ruleDropped: dropped,
        },
      },
    };
  } finally {
    database.$client.close();
  }
}

/** Signals for a decision with no example (a distractor, or a conflict task's teammate decision), from `deriveSignals`. */
async function deriveEntry(entry: Omit<BuiltEntry, "signals" | "provenance">, judge: AgentJudge, workspace: { root: string; repositoryKey: string }, calls: JudgeCall[]): Promise<Built> {
  const scope = [...entry.scope.include];
  try {
    const signals = await deriveSignals(recordingJudge(judge, entry.id, calls), { repositoryKey: workspace.repositoryKey, decision: entry.decision, scope, example: null }, { root: workspace.root, scope });
    return { entry: { ...entry, signals, provenance: { by: "deriveSignals", runner: runnersOf(calls, entry.id), at: new Date().toISOString() } } };
  } catch (error) {
    return { absent: { id: entry.id, outcome: "unjudged", detail: error instanceof Error ? error.message : String(error) } };
  }
}

/**
 * `--build-index`: the index every run reads, from the product's own code paths (see the header). Refuses to overwrite
 * a file, so an index is never rebuilt silently. `RERAIL_HOME` is a temporary folder for the whole build.
 */
async function buildIndex(dataset: Dataset, out: string, options: Options): Promise<void> {
  if (existsSync(out)) throw new Error(`${out} exists: an index is built once. Remove it yourself to build another.`);
  const home = await realpath(await mkdtemp(join(tmpdir(), "rerail-ma-context-home-")));
  const folder = await realpath(await mkdtemp(join(tmpdir(), "rerail-ma-context-index-")));
  process.env.RERAIL_HOME = home;
  const calls: JudgeCall[] = [];
  try {
    const { database, checkout } = judgeWorkspace(join(folder, "project"), join(folder, "project.sqlite"), dataset.project);
    try {
      const judge = createAgentJudge({ database, preferredHost: JUDGE_HOST, limitPerHour: JUDGE_LIMIT });
      const workspace = { root: checkout.checkoutPath, repositoryKey: checkout.repositoryKey };
      const peers = dataset.tasks.flatMap(task => (task.conflict === undefined ? [] : [{ ...task.conflict.peer, conflictsWith: task.conflict.own }]));
      const work: Array<{ id: string; run: () => Promise<Built> }> = [
        ...dataset.decisions.map(decision => ({ id: decision.id, run: () => draftReal(dataset, decision, folder, calls) })),
        ...peers.map(peer => ({ id: peer.id, run: () => deriveEntry({ id: peer.id, decision: peer.decision, scope: peer.scope, ...placeOf(peer), conflictsWith: peer.conflictsWith }, judge, workspace, calls) })),
        ...distractorPlaces(dataset, options.distractors ?? 0).map(place => ({ id: place.id, run: () => deriveEntry(place, judge, workspace, calls) })),
      ];
      console.log(`Building ${out}: ${dataset.decisions.length} real decisions through draftDecision, ${peers.length} teammate decisions and ${options.distractors ?? 0} distractors through deriveSignals; concurrency ${options.concurrency}`);
      const results = await pool(work, options.concurrency, item => item.run(), (done, result) => {
        console.error(`judged ${done}/${work.length}: ${"entry" in result ? `${result.entry.id} (${result.entry.provenance.by}, ${result.entry.signals.length} signals)` : `${result.absent.id} absent: ${result.absent.outcome}`}`);
      });
      const entries = results.flatMap(result => ("entry" in result ? [result.entry] : []));
      const absent = results.flatMap(result => ("absent" in result ? [result.absent] : []));
      const { embedder } = loadEmbedder(MODEL_PATH);
      const vectors = await embedder.embed(entries.map(entry => entry.decision));
      const embeddings = Object.fromEntries(entries.map((entry, position) => [entry.id, [...vectors[position]!].map(value => Math.round(value * 1e6) / 1e6)]));
      const index = {
        format: INDEX_FORMAT,
        builtAt: new Date().toISOString(),
        distractors: options.distractors ?? 0,
        model: { fingerprint: embedder.fingerprint(), maxInputTokens: EMBED_MAX_TOKENS },
        entries,
        embeddings,
        absent,
        judgeCalls: calls,
      };
      mkdirSync(dirname(out), { recursive: true });
      await writeFile(out, `${JSON.stringify(index, null, 2)}\n`);
      printBuild(dataset, entries, absent, calls);
      const report = checkIntegrity(entries);
      printIntegrity(report);
      const leaks = teammateLeaks(dataset, calls);
      console.log(`Teammate files in judge prompts: ${leaks.length === 0 ? `none (${calls.length} prompts checked)` : `${leaks.length} LEAKED`}`);
      for (const leak of leaks) console.log(`  LEAK ${leak}`);
      console.log(`\nWrote ${out}`);
      if (report.failures.length > 0 || leaks.length > 0) process.exitCode = 1;
    } finally {
      database.$client.close();
    }
  } finally {
    await rm(folder, { recursive: true, force: true });
    await rm(home, { recursive: true, force: true });
  }
}

/** Each real decision as the product produced it (or why it is absent), then the derived signals of the rest. */
function printBuild(dataset: Dataset, entries: readonly BuiltEntry[], absent: BuiltIndex["absent"], calls: readonly JudgeCall[]): void {
  console.log("\nReal decisions (draftDecision on each capture case)");
  for (const decision of dataset.decisions) {
    const entry = entries.find(candidate => candidate.id === decision.id);
    const missing = absent.find(candidate => candidate.id === decision.id);
    const judged = calls.filter(call => call.entryId === decision.id).map(call => `${call.kind}${call.ok ? "" : " FAILED"} ${(call.ms / 1000).toFixed(0)}s`).join(", ");
    console.log(`\n${decision.id}  [${judged}]`);
    if (entry === undefined) {
      console.log(`  ABSENT (${missing?.outcome ?? "unknown"}): ${missing?.detail ?? ""}`);
      continue;
    }
    console.log(`  decision: ${entry.decision}`);
    console.log(`  scope:    ${entry.scope.include.join(", ")}`);
    console.log(`  signals:  ${entry.signals.length === 0 ? "(none)" : entry.signals.map(signal => JSON.stringify(signal)).join(" ")}`);
    console.log(`  ${entry.provenance.generality}, ${entry.provenance.level}${entry.provenance.ruleDropped === null || entry.provenance.ruleDropped === undefined ? "" : ` (rule dropped: ${entry.provenance.ruleDropped})`}, runner ${entry.provenance.runner}`);
  }
  console.log("\nDerived signals (deriveSignals, no example, the project for the noise rule)");
  console.log(table(
    ["id", "source", "signals"],
    [
      ...entries.filter(entry => entry.provenance.by === "deriveSignals").map(entry => [entry.id, entry.conflictsWith === undefined ? "distractor" : `peer of ${entry.conflictsWith}`, entry.signals.length === 0 ? "(none)" : entry.signals.map(signal => JSON.stringify(signal)).join(" ")]),
      ...absent.filter(entry => !dataset.decisions.some(decision => decision.id === entry.id)).map(entry => [entry.id, "ABSENT", clip(entry.detail, 100)]),
    ],
  ));
}

// ---------------------------------------------------------------------------------------------------------- amend

/** cases.json, whose `amend` list holds the amendment cases `--amend` runs. */
const CASES = fixturesPath("host", "benchmark", "merge-assist", "cases.json");

/**
 * A person's correction that changes one of the index's real decisions (`amends`, its dataset id): the decision's
 * author corrects their agent, whose code (each edit's `before`) followed the decision as it stood. `correction` is
 * the person's words and `after` the code it led to: human input, like a capture case's correction.
 */
const amendCaseSchema = z.object({
  id: z.string().regex(/^[a-z0-9-]+$/u),
  amends: z.string(),
  why: z.string().min(1),
  correction: z.string().min(1),
  priorCalls: z.array(z.object({ tool: z.string().min(1), path: z.string().min(1) }).strict()),
  edits: z.array(z.object({ path: z.string().min(1), before: z.string().nullable(), after: z.string() }).strict()).min(1),
}).strict();
type AmendCase = z.infer<typeof amendCaseSchema>;

/**
 * The product's amendment of `entry` from `amendment`: `draftDecision` with the real judge, in the author's checkout
 * (the project with each edit's `after` in the working tree), with the decisions on `entry`'s branch or pull request
 * as `ownDecisions` under their candidate ids. Only a candidate whose `supersedes` names `entry`'s candidate is an
 * amendment; anything else is reported as what it is.
 */
async function draftAmendment(dataset: Dataset, entries: readonly BuiltEntry[], entry: BuiltEntry, amendment: AmendCase, folder: string, calls: JudgeCall[]): Promise<{ entry: BuiltEntry } | { outcome: string }> {
  const amended = entry.provenance.candidateId!;
  const root = join(folder, amendment.id);
  const { database, checkout } = judgeWorkspace(root, join(folder, `${amendment.id}.sqlite`), dataset.project);
  try {
    writeTree(root, Object.fromEntries(amendment.edits.map(edit => [edit.path, edit.after])));
    const ownDecisions = entries
      .filter(other => other.provenance.candidateId !== undefined && other.source === entry.source && other.branch === entry.branch && other.pr === entry.pr)
      .map(other => ({ id: other.provenance.candidateId!, decision: other.decision, scope: [...other.scope.include] }));
    const judge = recordingJudge(createAgentJudge({ database, preferredHost: JUDGE_HOST, limitPerHour: JUDGE_LIMIT }), entry.id, calls);
    const outcome = await draftDecision({ database, checkout, judge, author: async () => entry.author, now: () => Date.now() }, {
      correction: amendment.correction,
      priorCalls: amendment.priorCalls.map(call => `${call.tool} ${join(root, call.path)}`),
      edits: amendment.edits.map(edit => ({ ...edit })),
      ownDecisions,
    });
    if ("oneOff" in outcome) return { outcome: "one-off: the judge called the correction one-off" };
    if ("duplicateOf" in outcome) return { outcome: `duplicate: the judge called it a duplicate of ${outcome.duplicateOf}` };
    if ("unjudged" in outcome) return { outcome: `unjudged: ${outcome.unjudged}` };
    const { candidate, dropped } = outcome;
    if (candidate.supersedes !== amended) return { outcome: `a new decision, not an amendment of ${amended} (supersedes ${candidate.supersedes ?? "nothing"}): ${candidate.decision}` };
    return {
      entry: {
        ...entry,
        decision: candidate.decision,
        scope: { include: candidate.scope, exclude: [] },
        signals: candidate.signals ?? [],
        provenance: {
          by: "draftDecision",
          runner: runnersOf(calls, entry.id),
          at: new Date().toISOString(),
          case: amendment.id,
          candidateId: candidate.id,
          generality: candidate.generality,
          level: candidate.rule === undefined ? "note" : "code",
          ruleDropped: dropped,
          amends: amended,
        },
      },
    };
  } finally {
    database.$client.close();
  }
}

/**
 * `--amend`: a new index from `built` in which each decision an amendment case in cases.json names is replaced by the
 * amendment the product drafts (text, scope and signals), with a fresh embedding; every other entry, embedding and
 * recorded judge call is kept, and the amendment's judge calls are added. A case the product does not turn into an
 * amendment (a one-off, a duplicate, a new decision, no answer) writes nothing. Refuses to overwrite a file.
 */
async function amendIndex(dataset: Dataset, built: BuiltIndex, out: string): Promise<void> {
  if (existsSync(out)) throw new Error(`${out} exists: an index is built once. Remove it yourself to build another.`);
  const cases = z.array(amendCaseSchema).min(1).parse(pointer(JSON.parse(readFileSync(CASES, "utf8")), "/amend"));
  const home = await realpath(await mkdtemp(join(tmpdir(), "rerail-ma-context-home-")));
  const folder = await realpath(await mkdtemp(join(tmpdir(), "rerail-ma-context-amend-")));
  process.env.RERAIL_HOME = home;
  const calls: JudgeCall[] = [];
  const entries = [...built.entries];
  const refused: string[] = [];
  try {
    for (const amendment of cases) {
      const position = entries.findIndex(entry => entry.id === amendment.amends);
      const entry = entries[position];
      if (entry?.provenance.by !== "draftDecision" || entry.provenance.candidateId === undefined) throw new Error(`${amendment.id} amends ${amendment.amends}, which is no decision the product drafted in ${built.path}`);
      const result = await draftAmendment(dataset, entries, entry, amendment, folder, calls);
      console.log(`\n${amendment.id} (amends ${entry.id}, ${entry.provenance.candidateId})`);
      console.log(`  correction (the person's words): ${JSON.stringify(amendment.correction)}`);
      console.log(`  before:   ${JSON.stringify(entry.decision)}  scope ${entry.scope.include.join(", ")}  signals ${JSON.stringify(entry.signals)}`);
      console.log(`  judge:    ${calls.filter(call => call.entryId === entry.id).map(call => `${call.kind}${call.ok ? "" : " FAILED"} ${(call.ms / 1000).toFixed(0)}s`).join(", ")}`);
      if ("outcome" in result) {
        console.log(`  NOT AMENDED: ${result.outcome}`);
        refused.push(amendment.id);
        continue;
      }
      const after = result.entry;
      console.log(`  after:    ${JSON.stringify(after.decision)}  scope ${after.scope.include.join(", ")}  signals ${JSON.stringify(after.signals)}`);
      console.log(`  ${after.provenance.candidateId} supersedes ${after.provenance.amends}; ${after.provenance.generality}, ${after.provenance.level}${after.provenance.ruleDropped === null ? "" : ` (rule dropped: ${after.provenance.ruleDropped})`}, runner ${after.provenance.runner}`);
      entries[position] = after;
    }
    const leaks = teammateLeaks(dataset, calls);
    console.log(`Teammate files in judge prompts: ${leaks.length === 0 ? `none (${calls.length} prompts checked)` : `${leaks.length} LEAKED`}`);
    for (const leak of leaks) console.log(`  LEAK ${leak}`);
    const report = checkIntegrity(entries);
    printIntegrity(report);
    if (refused.length > 0 || leaks.length > 0 || report.failures.length > 0) {
      console.log(`\nWrote nothing: ${refused.length > 0 ? `${refused.join(", ")} not amended by the product` : "the amended index fails its checks"}.`);
      process.exitCode = 1;
      return;
    }
    const amended = entries.filter(entry => entry.provenance.amends !== undefined);
    const { embedder } = loadEmbedder(MODEL_PATH);
    const vectors = await embedder.embed(amended.map(entry => entry.decision));
    const embeddings = { ...built.embeddings, ...Object.fromEntries(amended.map((entry, position) => [entry.id, [...vectors[position]!].map(value => Math.round(value * 1e6) / 1e6)])) };
    const index = {
      format: INDEX_FORMAT,
      builtAt: new Date().toISOString(),
      distractors: built.distractors,
      model: { fingerprint: embedder.fingerprint(), maxInputTokens: EMBED_MAX_TOKENS },
      entries,
      embeddings,
      absent: built.absent,
      judgeCalls: [...built.judgeCalls, ...calls],
    };
    mkdirSync(dirname(out), { recursive: true });
    await writeFile(out, `${JSON.stringify(index, null, 2)}\n`);
    console.log(`\nWrote ${out}: ${built.path} with ${amended.map(entry => entry.id).join(", ")} amended`);
  } finally {
    await rm(folder, { recursive: true, force: true });
    await rm(home, { recursive: true, force: true });
  }
}

// ---------------------------------------------------------------------------------------------------------- live

/*
 * `--live`: a correction one person gives their agent (A) on one branch, turned into a decision by the product's own
 * capture, carried to another person's agent (B) on another branch. Per decision D and trial:
 * - A runs its task in worktree `a` (branch `alice/<task>`, cut from `main`) under a fresh CODEX_HOME and the control
 *   hooks, so nothing is injected into it. It is graded with D's oracle, then resumed once with the person's
 *   correction: cases.json's, as it is when A broke D, otherwise prefixed "Going forward: ".
 * - `draftDecision` with the real judge, in a scratch Rerail database over worktree `a`, turns the correction into a
 *   candidate: the correction, A's last six tool calls from its Codex transcript, and each file the resume changed
 *   (before and after, from the two trees, four at most). The candidate is written as a rule in `.rerail/rules/` and
 *   committed on A's branch.
 * - B's index is the built index without D's own entry, plus, when captured, an entry read from A's committed rule
 *   (a peer decision of Alice Chen on `alice/<task>`, embedded with the bundled model, with the candidate's signals).
 * - B runs each of D's other dev, heldout and size tempt tasks (three at most, by id) in a clone of the repository's
 *   `main` alone, on branch `bob/<task>` cut from it, under `focus-pool` and `none`: phase 1 only, graded with D's
 *   oracle. The clone gives each arm its own checkout of the same branch name; holding no other branch, it leaves the
 *   decision only one way to B, the index.
 */

const LIVE_SPLITS: ReadonlyArray<Task["split"]> = ["dev", "heldout", "size"];
const LIVE_B_TASKS = 3;
/** What the product's capture names of the agent's work before a correction: its last six calls, its first four edits. */
const LIVE_PRIOR_CALLS = 6;
const LIVE_EDITS = 4;
const ALICE = { name: "Alice Chen", email: "alice@example.com" };
const GOING_FORWARD = "Going forward: ";
const RULES_DIRECTORY = ".rerail/rules";
const LIVE_FORMAT = "rerail.merge-assist-live/v1";

/** A's task for `decisionId`: the first dev tempt task that targets it, else the first size one. */
export function liveATask(dataset: Dataset, decisionId: string): Task | undefined {
  const first = (split: Task["split"]) => dataset.tasks.find(task => task.split === split && task.kind === "tempt" && task.targets.includes(decisionId));
  return first("dev") ?? first("size");
}

/** B's tasks for `decisionId`: its dev, heldout and size tempt tasks but A's, those `only` names, at most three by id. */
export function liveBTasks(dataset: Dataset, decisionId: string, aTask: Task, only: readonly string[] | null): Task[] {
  return dataset.tasks
    .filter(task => task.kind === "tempt" && LIVE_SPLITS.includes(task.split) && task.targets.includes(decisionId) && task.id !== aTask.id && (only === null || only.includes(task.id)))
    .sort((left, right) => (left.id < right.id ? -1 : left.id > right.id ? 1 : 0))
    .slice(0, LIVE_B_TASKS);
}

/** A `call` event's payload, as the product's capture stores one and `callSummary` reads it. */
interface CallPayload {
  readonly toolName: string;
  readonly input: Record<string, unknown>;
}

const PATCH_FILE = /\*\*\* (?:Add|Update|Delete) File: ([^\n"\\]+)/gu;
const INNER_TOOL = /\btools\.(\w+)\s*\(/gu;
const COMMAND_LITERAL = /\bcmd\s*:\s*("(?:[^"\\]|\\.)*")/su;

/** A JavaScript double-quoted string literal's value; its raw text when JSON cannot read it. */
function stringLiteral(literal: string): string {
  try {
    return String(JSON.parse(literal));
  } catch {
    return literal.slice(1, -1);
  }
}

/**
 * The calls one Codex custom tool call makes: an `apply_patch` names the files its patch changes; a code-mode `exec`
 * cell (JavaScript calling `tools.exec_command({cmd: …})`, `tools.apply_patch(…)`) makes one call per tool it calls.
 */
function customToolCalls(name: string, input: string): CallPayload[] {
  const patched = { path: [...input.matchAll(PATCH_FILE)].map(match => match[1]!.trim()).join(" ") };
  if (name === "apply_patch") return [{ toolName: name, input: patched }];
  if (name !== "exec") return [{ toolName: name, input: { command: input } }];
  const inner = [...input.matchAll(INNER_TOOL)];
  return inner.map((match, position) => {
    const tool = match[1]!;
    if (tool === "apply_patch") return { toolName: tool, input: patched };
    const literal = COMMAND_LITERAL.exec(input.slice(match.index + match[0].length, inner[position + 1]?.index ?? input.length))?.[1];
    return { toolName: tool, input: literal === undefined ? {} : { command: stringLiteral(literal) } };
  });
}

/** The agent's tool calls in Codex session transcripts (JSONL), oldest first, as `call` payloads. */
export function transcriptCalls(text: string): CallPayload[] {
  const calls: CallPayload[] = [];
  for (const line of text.split("\n")) {
    if (!line.startsWith("{")) continue;
    let event: Record<string, unknown>;
    try {
      event = JSON.parse(line) as Record<string, unknown>;
    } catch {
      continue;
    }
    const payload = event.payload as Record<string, unknown> | undefined;
    if (event.type !== "response_item" || payload === undefined || payload === null) continue;
    const name = typeof payload.name === "string" ? payload.name : "tool";
    const command = (value: unknown): string => (Array.isArray(value) ? value.map(String).join(" ") : String(value ?? ""));
    if (payload.type === "function_call") {
      let args: Record<string, unknown> = {};
      try {
        const parsed = JSON.parse(String(payload.arguments)) as unknown;
        if (parsed !== null && typeof parsed === "object") args = parsed as Record<string, unknown>;
      } catch {
        // arguments that are not JSON leave the tool's name alone
      }
      const shell = args.cmd ?? args.command;
      calls.push({ toolName: name, input: shell === undefined ? args : { command: command(shell) } });
    } else if (payload.type === "local_shell_call") {
      calls.push({ toolName: "shell", input: { command: command((payload.action as Record<string, unknown> | undefined)?.command) } });
    } else if (payload.type === "custom_tool_call" && typeof payload.input === "string") {
      calls.push(...customToolCalls(name, payload.input));
    }
  }
  return calls;
}

/** The worktree's files, untracked ones included, as a tree object: a snapshot that leaves its history alone. */
function snapshotTree(worktree: string): string {
  git(worktree, "add", "-A");
  return git(worktree, "write-tree").trim();
}

/** The files that differ between two trees, at most {@link LIVE_EDITS}, each with its content in both (null: absent). */
function treeEdits(worktree: string, before: string, after: string): CaptureInput["edits"] {
  const blob = (tree: string, path: string): string | null => {
    try {
      return git(worktree, "cat-file", "blob", `${tree}:${path}`);
    } catch {
      return null;
    }
  };
  return git(worktree, "-c", "core.quotePath=false", "diff", "--name-only", "--no-renames", before, after)
    .split("\n").filter(path => path !== "").slice(0, LIVE_EDITS)
    .map(path => ({ path, before: blob(before, path), after: blob(after, path) }));
}

/**
 * The candidate as a rule document for `.rerail/rules/`: its code rule, described by its decision; for a note, a
 * document in the candidate format's shape with only its text and scope.
 */
function ruleDocument(candidate: Candidate): Record<string, unknown> {
  if (candidate.rule !== undefined) return { ...candidate.rule, description: candidate.rule.description ?? candidate.decision };
  return { format: CANDIDATE_FORMAT, id: candidate.id, decision: candidate.decision, scope: candidate.scope };
}

/** The decision text and scope a committed rule document states: a code rule's description and scope, or a note's. */
const committedRuleSchema = z.union([
  z.object({ description: z.string().min(1), scope: z.object({ include: z.array(z.string().min(1)).min(1) }).passthrough() }).passthrough()
    .transform(rule => ({ decision: rule.description, scope: rule.scope.include })),
  z.object({ format: z.literal(CANDIDATE_FORMAT), decision: z.string().min(1), scope: z.array(z.string().min(1)).min(1) }).passthrough()
    .transform(note => ({ decision: note.decision, scope: note.scope })),
]);

/** `git commit` in `cwd` as Alice, the person whose agent is A. */
function commitAsAlice(cwd: string, message: string): string {
  const env = { ...GIT_ENV, GIT_AUTHOR_NAME: ALICE.name, GIT_AUTHOR_EMAIL: ALICE.email, GIT_COMMITTER_NAME: ALICE.name, GIT_COMMITTER_EMAIL: ALICE.email };
  execFileSync("git", ["commit", "-q", "-m", message], { cwd, env, stdio: ["ignore", "pipe", "pipe"] });
  return git(cwd, "rev-parse", "HEAD").trim();
}

/** The built index without `decisionId`'s own entry, so the decision can reach B only through the live capture. */
function withoutDecision(built: BuiltIndex, decisionId: string): BuiltIndex {
  const { [decisionId]: _removed, ...embeddings } = built.embeddings;
  return { ...built, entries: built.entries.filter(entry => entry.id !== decisionId), embeddings, absent: built.absent.filter(entry => entry.id !== decisionId) };
}

/** One Codex call of the live check: its time, tokens and failure. */
interface LivePhase {
  readonly wallMs: number;
  readonly usage: Usage | null;
  readonly error: string | null;
}

function livePhase(outcome: CodexOutcome, events: CodexEvents, options: Options): LivePhase {
  const error = outcome.timedOut
    ? `timed out after ${options.timeoutMin} minutes`
    : events.error ?? (outcome.exitCode === 0 ? null : `codex exited ${outcome.exitCode}: ${clip(outcome.stderr.trim().split("\n").at(-1) ?? "", 200)}`);
  return { wallMs: outcome.wallMs, usage: events.usage, error };
}

interface LiveAgentA {
  readonly taskId: string;
  readonly branch: string;
  readonly threadId: string | null;
  readonly phase1: LivePhase;
  /** Null when there was no session to resume. */
  readonly resume: LivePhase | null;
  /** D's oracle hits on A's work after its task, and after the correction. */
  readonly hitsBefore: readonly string[];
  readonly hitsAfter: readonly string[] | null;
  readonly brokeD: boolean;
  /** The correction sent, and whether "Going forward: " was put before it. */
  readonly correction: string;
  readonly prefixed: boolean;
  readonly priorCalls: readonly string[];
  readonly edits: CaptureInput["edits"];
  readonly error: string | null;
}

const CAPTURE_OUTCOMES = ["captured", "one-off", "duplicate", "unjudged", "not-run"] as const;
type CaptureOutcome = (typeof CAPTURE_OUTCOMES)[number];

interface LiveCapture {
  readonly outcome: CaptureOutcome;
  readonly detail: string;
  readonly ms: number;
  readonly judgeCalls: readonly JudgeCall[];
  readonly candidate: Candidate | null;
  /** Why the drafted code rule was dropped, leaving a note. */
  readonly ruleDropped: string | null;
  /** The rule file committed on A's branch, its commit, and the document as committed. */
  readonly ruleFile: string | null;
  readonly commit: string | null;
  readonly document: Record<string, unknown> | null;
}

interface LiveBRun {
  readonly arm: string;
  readonly taskId: string;
  readonly trial: number;
  /** D's oracle on B's diff. */
  readonly violated: boolean;
  readonly hits: readonly string[];
  /** Whether D was injected, and before B's first edit in its scope. */
  readonly injected: boolean;
  readonly timely: boolean;
  /** B's other targets, each with its oracle hits: context only. */
  readonly others: Readonly<Record<string, number>>;
  readonly error: string | null;
  readonly wallMs: number;
  readonly usage: Usage | null;
  readonly run: RunResult;
}

interface LiveTrial {
  readonly decisionId: string;
  readonly trial: number;
  readonly a: LiveAgentA | null;
  readonly capture: LiveCapture;
  /** B's index entry for D, from A's committed rule; null when capture failed. */
  readonly entry: BuiltEntry | null;
  readonly integrity: { readonly failures: readonly string[]; readonly overlaps: number; readonly rankerFailures: readonly string[] };
  readonly b: readonly LiveBRun[];
  readonly wallMs: number;
  readonly error: string | null;
  readonly folder: string | null;
}

interface LiveJob {
  readonly decision: DatasetEntry;
  readonly aTask: Task;
  readonly bTasks: readonly Task[];
  readonly trial: number;
}

interface LiveContext {
  readonly dataset: Dataset;
  readonly base: BuiltIndex;
  readonly options: Options;
  readonly embedder: NativeSemanticEmbedder;
  /** Runs `work` when one of `--concurrency` slots is free: each A (with its capture) and each B run takes one. */
  readonly limit: <T>(work: () => Promise<T>) => Promise<T>;
}

/** At most `slots` of the works given at once, the rest in the order they came. */
function limiter(slots: number): <T>(work: () => Promise<T>) => Promise<T> {
  let free = slots;
  const waiting: Array<() => void> = [];
  return async work => {
    if (free > 0) free--;
    else {
      const { promise, resolve } = Promise.withResolvers<void>();
      waiting.push(resolve);
      await promise;
    }
    try {
      return await work();
    } finally {
      const next = waiting.shift();
      if (next === undefined) free++;
      else next();
    }
  };
}

/** A in worktree `a`: its task under the control hooks, graded with D's oracle, then one resume with the correction. */
async function runAgentA(job: LiveJob, folder: string, worktree: string, index: BuiltIndex, context: LiveContext): Promise<LiveAgentA> {
  const { dataset, options } = context;
  const { decision, aTask } = job;
  const labelled = dataset.labelled.find(entry => entry.id === decision.id);
  if (labelled === undefined) throw new Error(`No capture case in cases.json for decision ${decision.id}`);
  const home = join(folder, "a-run");
  mkdirSync(home, { recursive: true });
  const codexHome = join(home, "codex-home");
  const configPath = join(home, "hook-config.json");
  const logPath = join(home, "hook.log");
  const rerailHome = join(home, "rerail-home");
  const runIndexA = runIndex(dataset, index, aTask, options.distractors ?? index.distractors, options.seed);
  const config: HookConfig = {
    variant: null,
    index: runIndexA,
    prompt: aTask.prompt,
    embeddings: Object.fromEntries(runIndexA.map(entry => [entry.id, index.embeddings[entry.id]!])),
    modelPath: MODEL_PATH,
    files: Object.keys(dataset.project),
    root: worktree,
    folder: home,
    log: logPath,
    state: join(home, "hook-state.json"),
  };
  writeFileSync(configPath, `${JSON.stringify(config, null, 2)}\n`);
  writeFileSync(logPath, "");
  createCodexHome(codexHome, configPath);
  const first = await runCodex(worktree, codexHome, configPath, aTask.prompt, options, rerailHome);
  const events = readEvents(first.stdout);
  const hitsBefore = grade(diffAgainstMain(worktree), [decision])[decision.id] ?? [];
  const brokeD = hitsBefore.length > 0;
  // A correction that already says it is for the future (cases.json's dates decision) is not prefixed twice.
  const prefixed = !brokeD && !/^going forward\b/iu.test(labelled.correction);
  const correction = prefixed ? `${GOING_FORWARD}${labelled.correction}` : labelled.correction;
  const priorCalls = transcriptCalls(transcriptText(codexHome) ?? "").slice(-LIVE_PRIOR_CALLS).map(call => callSummary({ ...call }));
  const base = { taskId: aTask.id, branch: `alice/${aTask.id}`, threadId: events.threadId, phase1: livePhase(first, events, options), hitsBefore, brokeD, correction, prefixed, priorCalls };
  if (events.threadId === null) return { ...base, resume: null, hitsAfter: null, edits: [], error: "A's run reported no thread id, so the correction could not resume it" };
  const treeBefore = snapshotTree(worktree);
  const second = await runCodex(worktree, codexHome, configPath, correction, options, rerailHome, events.threadId);
  const resume = livePhase(second, readEvents(second.stdout), options);
  const hitsAfter = grade(diffAgainstMain(worktree), [decision])[decision.id] ?? [];
  const treeAfter = snapshotTree(worktree);
  return { ...base, resume, hitsAfter, edits: treeEdits(worktree, treeBefore, treeAfter), error: null };
}

const NOT_CAPTURED = { judgeCalls: [], candidate: null, ruleDropped: null, ruleFile: null, commit: null, document: null } as const;

/** The product's capture of A's correction: `draftDecision` with the real judge, over worktree `a`. */
async function captureLive(folder: string, worktree: string, agent: LiveAgentA, decisionId: string): Promise<LiveCapture> {
  const calls: JudgeCall[] = [];
  const started = performance.now();
  const { database, checkout } = openJudgeWorkspace(worktree, join(folder, "rerail.sqlite"));
  try {
    const judge = recordingJudge(createAgentJudge({ database, preferredHost: JUDGE_HOST, limitPerHour: JUDGE_LIMIT }), decisionId, calls);
    const input: CaptureInput = { correction: agent.correction, priorCalls: [...agent.priorCalls], edits: agent.edits.map(edit => ({ ...edit })), ownDecisions: [] };
    const outcome = await draftDecision({ database, checkout, judge, author: async () => ALICE.name, now: () => Date.now() }, input);
    const done = { ...NOT_CAPTURED, ms: Math.round(performance.now() - started), judgeCalls: calls };
    if ("oneOff" in outcome) return { ...done, outcome: "one-off", detail: "the judge called the correction one-off" };
    if ("duplicateOf" in outcome) return { ...done, outcome: "duplicate", detail: `the judge called it a duplicate of ${outcome.duplicateOf}` };
    if ("unjudged" in outcome) return { ...done, outcome: "unjudged", detail: outcome.unjudged };
    return { ...done, outcome: "captured", detail: "", candidate: outcome.candidate, ruleDropped: outcome.dropped };
  } finally {
    database.$client.close();
  }
}

/** One trial of one decision: A, the capture, B's index, then every B task in both arms. */
async function runLiveTrial(job: LiveJob, context: LiveContext): Promise<LiveTrial> {
  const { dataset, base, options } = context;
  const { decision, aTask, bTasks, trial } = job;
  const started = performance.now();
  const folder = await realpath(await mkdtemp(join(tmpdir(), "rerail-ma-live-")));
  const repo = join(folder, "repo");
  const worktree = join(folder, "a");
  const branch = `alice/${aTask.id}`;
  const without = withoutDecision(base, decision.id);
  let a: LiveAgentA | null = null;
  let capture: LiveCapture = { ...NOT_CAPTURED, outcome: "not-run", detail: "", ms: 0 };
  let entry: BuiltEntry | null = null;
  let integrity: LiveTrial["integrity"] = { failures: [], overlaps: 0, rankerFailures: [] };
  let b: LiveBRun[] = [];
  let error: string | null = null;
  try {
    createMain(repo, dataset.project);
    git(repo, "worktree", "add", "-q", "-b", branch, worktree, "main");
    ({ a, capture } = await context.limit(async () => {
      const agent = await runAgentA(job, folder, worktree, without, context);
      if (agent.error !== null) return { a: agent, capture: { ...capture, detail: agent.error } };
      return { a: agent, capture: await captureLive(folder, worktree, agent, decision.id) };
    }));
    let vector: number[] | null = null;
    if (capture.candidate !== null) {
      const candidate = capture.candidate;
      const ruleFile = `${RULES_DIRECTORY}/${candidateFileName(candidate.id)}`;
      writeTree(worktree, { [ruleFile]: `${JSON.stringify(ruleDocument(candidate), null, 2)}\n` });
      git(worktree, "add", "-A");
      const commit = commitAsAlice(worktree, `${aTask.id}, with the decision from Alice's correction`);
      const document = JSON.parse(git(repo, "show", `${branch}:${ruleFile}`)) as Record<string, unknown>;
      const committed = committedRuleSchema.parse(document);
      capture = { ...capture, ruleFile, commit, document };
      entry = {
        id: decision.id,
        decision: committed.decision,
        scope: { include: committed.scope, exclude: [] },
        source: "peer",
        author: ALICE.name,
        branch,
        signals: candidate.signals ?? [],
        provenance: {
          by: "draftDecision",
          via: "live-capture",
          runner: runnersOf(capture.judgeCalls, decision.id),
          at: new Date().toISOString(),
          candidateId: candidate.id,
          generality: candidate.generality,
          level: candidate.rule === undefined ? "note" : "code",
          ruleDropped: capture.ruleDropped,
        },
      };
      const [embedded] = await context.embedder.embed([entry.decision]);
      vector = [...embedded!].map(value => Math.round(value * 1e6) / 1e6);
    }
    const trialIndex: BuiltIndex = entry === null || vector === null
      ? { ...without, path: `${base.path} without ${decision.id}`, absent: [...without.absent, { id: decision.id, outcome: capture.outcome, detail: capture.detail }] }
      : { ...without, path: `${base.path} without ${decision.id}, with its live capture`, entries: [...without.entries, entry], embeddings: { ...without.embeddings, [entry.id]: vector } };
    const report = checkIntegrity(trialIndex.entries);
    const rankerFailures = bTasks.flatMap(task => rankerLeaks(dataset, trialIndex, task, options.distractors ?? base.distractors, options.seed, [VARIANTS["focus-pool"]!]).failures);
    integrity = { failures: report.failures, overlaps: report.overlaps.length, rankerFailures };
    if (report.failures.length > 0 || rankerFailures.length > 0) throw new Error(`B's index fails the integrity check: ${[...report.failures, ...rankerFailures].join("; ")}`);
    const cloneForB = (task: Task) => (path: string) => {
      git(dirname(path), "clone", "-q", "--single-branch", "--branch", "main", repo, path);
      git(path, "config", "commit.gpgsign", "false");
      git(path, "checkout", "-q", "-b", `bob/${task.id}`, "main");
    };
    b = await Promise.all(bTasks.flatMap(task => LIVE_ARMS.map(arm => context.limit(async (): Promise<LiveBRun> => {
      const run = await runJob({ variant: arm, task, trial }, dataset, trialIndex, options, cloneForB(task));
      const delivery = run.delivery.find(entry => entry.decisionId === decision.id);
      const hits = run.hits[decision.id] ?? [];
      console.error(`  B ${decision.id} #${trial} ${arm} ${task.id}: ${run.error ?? `${hits.length > 0 ? "VIOLATION" : "clean"}, ${delivery?.injected === true ? `injected${delivery.timely ? " in time" : " late"}` : "not injected"}, ${(run.wallMs / 1000).toFixed(0)}s`}`);
      return {
        arm,
        taskId: task.id,
        trial,
        violated: hits.length > 0,
        hits,
        injected: delivery?.injected ?? false,
        timely: delivery?.timely ?? false,
        others: Object.fromEntries(task.targets.filter(id => id !== decision.id).map(id => [id, (run.hits[id] ?? []).length])),
        error: run.error,
        wallMs: run.wallMs,
        usage: run.usage,
        run,
      };
    }))));
  } catch (failure) {
    error = failure instanceof Error ? failure.message : String(failure);
  } finally {
    if (options.keep) console.error(`kept ${folder}`);
    else await rm(folder, { recursive: true, force: true });
  }
  return { decisionId: decision.id, trial, a, capture, entry, integrity, b, wallMs: performance.now() - started, error, folder: options.keep ? folder : null };
}

/** The live check's report: per trial, then the capture, delivery and violation descriptives, then L1. */
function printLive(trials: readonly LiveTrial[], decisions: readonly string[]): LiveResult | null {
  console.log("\nTrials (A: its task, then the correction; capture; B: each task in each arm)");
  console.log(table(
    ["decision", "#", "A task", "A broke D", "after correction", "sent", "A s (task+resume)", "A in/cached/out", "capture", "capture s", "B runs (arm task: viol, injected/timely)", "error"],
    trials.map(trial => [
      trial.decisionId,
      String(trial.trial),
      trial.a?.taskId ?? "-",
      trial.a === null ? "-" : trial.a.brokeD ? "VIOL" : "ok",
      trial.a?.hitsAfter === null || trial.a === null ? "-" : trial.a.hitsAfter.length > 0 ? "VIOL" : "ok",
      trial.a === null ? "-" : trial.a.prefixed ? "going forward" : "as is",
      trial.a === null ? "-" : `${(trial.a.phase1.wallMs / 1000).toFixed(0)}+${((trial.a.resume?.wallMs ?? 0) / 1000).toFixed(0)}`,
      trial.a === null ? "-" : [trial.a.phase1.usage, trial.a.resume?.usage ?? null].map(usage => (usage === null ? "-" : `${usage.input}/${usage.cached}/${usage.output}`)).join(" + "),
      `${trial.capture.outcome}${trial.capture.candidate === null ? "" : ` (${trial.capture.candidate.rule === undefined ? "note" : "code"})`}`,
      (trial.capture.ms / 1000).toFixed(0),
      trial.b.map(run => `${run.arm} ${run.taskId}: ${run.error === null ? (run.violated ? "VIOL" : "ok") : "err"}, ${run.injected ? "y" : "n"}/${run.timely ? "y" : "n"}`).join(" | "),
      clip(trial.error ?? trial.a?.error ?? [trial.a?.phase1.error, trial.a?.resume?.error].filter(entry => typeof entry === "string").join("; "), 120),
    ]),
  ));

  console.log("\nCapture success (draftDecision with the real judge on A's correction)");
  const captured = (list: readonly LiveTrial[]) => ratio(list.filter(trial => trial.capture.outcome === "captured").length, list.length);
  console.log(table(
    ["decision", "captured", ...CAPTURE_OUTCOMES.filter(outcome => outcome !== "captured")],
    [...decisions, "overall"].map(id => {
      const list = id === "overall" ? trials : trials.filter(trial => trial.decisionId === id);
      return [id, rate(captured(list)), ...CAPTURE_OUTCOMES.filter(outcome => outcome !== "captured").map(outcome => String(list.filter(trial => trial.capture.outcome === outcome).length))];
    }),
  ));

  console.log("\nWhat the product produced, against the correction");
  for (const trial of trials) {
    console.log(`\n${trial.decisionId} #${trial.trial}: ${trial.capture.outcome}${trial.capture.detail === "" ? "" : ` (${clip(trial.capture.detail, 200)})`}`);
    if (trial.a !== null) {
      console.log(`  correction:  ${trial.a.correction}`);
      console.log(`  prior calls: ${trial.a.priorCalls.length === 0 ? "(none)" : trial.a.priorCalls.map(call => JSON.stringify(clip(call, 80))).join(", ")}`);
      console.log(`  edits:       ${trial.a.edits.length === 0 ? "(none)" : trial.a.edits.map(edit => `${edit.path} (${edit.before === null ? "new" : `${edit.before.length}`} → ${edit.after === null ? "deleted" : `${edit.after.length}`} chars)`).join(", ")}`);
    }
    const candidate = trial.capture.candidate;
    if (candidate === null) continue;
    console.log(`  decision:    ${trial.entry?.decision ?? candidate.decision}`);
    console.log(`  scope:       ${(trial.entry?.scope.include ?? candidate.scope).join(", ")}`);
    console.log(`  signals:     ${(candidate.signals ?? []).length === 0 ? "(none)" : (candidate.signals ?? []).map(signal => JSON.stringify(signal)).join(" ")}`);
    console.log(`  ${candidate.generality}, ${candidate.rule === undefined ? "note" : "code rule"}${trial.capture.ruleDropped === null ? "" : ` (rule dropped: ${trial.capture.ruleDropped})`}; committed ${trial.capture.ruleFile ?? "-"} on ${trial.a?.branch ?? "-"} at ${trial.capture.commit?.slice(0, 12) ?? "-"}`);
  }

  const runs = trials.flatMap(trial => trial.b.map(run => ({ trial, run })));
  const ofArm = (arm: string, capturedOnly: boolean | null) => runs.filter(({ trial, run }) => run.arm === arm && (capturedOnly === null || (trial.capture.outcome === "captured") === capturedOnly));
  const violation = (list: ReadonlyArray<{ run: LiveBRun }>) => rate(ratio(list.filter(({ run }) => run.violated || run.error !== null).length, list.length));
  console.log("\nDelivery to B and B's violation of D (a run with an error counts as a violation)");
  console.log(table(
    ["arm", "B runs", "D injected (captured)", "D timely (captured)", "violation of D: all", "captured", "not captured", "other targets violated"],
    LIVE_ARMS.map(arm => {
      const all = ofArm(arm, null);
      const delivered = ofArm(arm, true);
      const others = all.flatMap(({ run }) => Object.values(run.others));
      return [
        arm, String(all.length),
        rate(ratio(delivered.filter(({ run }) => run.injected).length, delivered.length)),
        rate(ratio(delivered.filter(({ run }) => run.timely).length, delivered.length)),
        violation(all), violation(delivered), violation(ofArm(arm, false)),
        rate(ratio(others.filter(count => count > 0).length, others.length)),
      ];
    }),
  ));

  console.log("\nTime and tokens per phase (mean; input/cached/output)");
  const usageMean = (list: ReadonlyArray<Usage | null>) => {
    const known = list.filter((usage): usage is Usage => usage !== null);
    return known.length === 0 ? "-" : `${fixed(mean(known.map(usage => usage.input)), 0)}/${fixed(mean(known.map(usage => usage.cached)), 0)}/${fixed(mean(known.map(usage => usage.output)), 0)}`;
  };
  const agents = trials.flatMap(trial => (trial.a === null ? [] : [trial.a]));
  console.log(table(["phase", "n", "wall s", "tokens"], [
    ["A task", String(agents.length), fixed(mean(agents.map(agent => agent.phase1.wallMs / 1000)), 0), usageMean(agents.map(agent => agent.phase1.usage))],
    ["A resume", String(agents.filter(agent => agent.resume !== null).length), fixed(mean(agents.flatMap(agent => (agent.resume === null ? [] : [agent.resume.wallMs / 1000]))), 0), usageMean(agents.map(agent => agent.resume?.usage ?? null))],
    ["capture (judge calls)", String(trials.filter(trial => trial.capture.outcome !== "not-run").length), fixed(mean(trials.filter(trial => trial.capture.outcome !== "not-run").map(trial => trial.capture.ms / 1000)), 0), `${trials.reduce((sum, trial) => sum + trial.capture.judgeCalls.length, 0)} calls, prompts ~${fixed(mean(trials.flatMap(trial => trial.capture.judgeCalls.map(call => call.prompt.length / 4))), 0)} tokens each`],
    ...LIVE_ARMS.map(arm => {
      const list = ofArm(arm, null).map(({ run }) => run);
      return [`B ${arm}`, String(list.length), fixed(mean(list.map(run => run.wallMs / 1000)), 0), usageMean(list.map(run => run.usage))];
    }),
  ]));

  const failures = trials.filter(trial => trial.integrity.failures.length > 0 || trial.integrity.rankerFailures.length > 0);
  console.log(`\nB's per-trial index integrity: ${failures.length === 0 ? `PASS (${trials.length} trials)` : `FAIL in ${failures.length} trials`}; oracle code text in the product's output (reported, not refused): ${trials.map(trial => trial.integrity.overlaps).join(", ")}`);

  const result = liveTest(runs.map(({ run }) => run), 1, liveAlpha(1));
  console.log(`\nL1: B's violation of D lower with focus-pool than with none (this batch alone, one look; pool looks with merge-assist-stats.ts --analyze-live)`);
  printLiveResult(result);
  return result;
}

function printLiveResult(result: LiveResult | null): void {
  if (result === null) {
    console.log("  no B task ran in both arms");
    return;
  }
  console.log(`  ${result.runs} runs over ${result.tasks} B tasks${result.unpaired.length === 0 ? "" : ` (+${result.unpaired.length} unpaired)`}; focus-pool ${rate(result.rates.a)}, none ${rate(result.rates.b)} violated`);
  console.log(`  effect (mean within-task difference in proportions, focus-pool − none): ${result.effect >= 0 ? "+" : ""}${result.effect.toFixed(3)}, 95% cluster-bootstrap CI [${result.ci[0].toFixed(3)}, ${result.ci[1].toFixed(3)}]`);
  console.log(`  stratified permutation p (one-sided, strata = B task) ${result.p.toFixed(4)} against α ${result.alpha}: ${result.reject ? "REJECT H0" : "keep H0"}`);
}

/** `--live`: every decision with an A task and a B task (or those `--decisions` names), `--trials` trials each. */
async function runLive(dataset: Dataset, base: BuiltIndex, options: Options): Promise<void> {
  const ids = options.decisions ?? dataset.decisions.map(decision => decision.id);
  const unknown = ids.filter(id => !dataset.decisions.some(decision => decision.id === id));
  if (unknown.length > 0) throw new Error(`Unknown decision id: ${unknown.join(", ")}`);
  const unknownTasks = (options.tasks ?? []).filter(id => !dataset.tasks.some(task => task.id === id));
  if (unknownTasks.length > 0) throw new Error(`Unknown task id: ${unknownTasks.join(", ")}`);
  const plan = ids.map(id => {
    const aTask = liveATask(dataset, id);
    return { decision: dataset.decisions.find(decision => decision.id === id)!, aTask, bTasks: aTask === undefined ? [] : liveBTasks(dataset, id, aTask, options.tasks) };
  });
  const jobs: LiveJob[] = plan.flatMap(({ decision, aTask, bTasks }) => (aTask === undefined || bTasks.length === 0 ? [] : Array.from({ length: options.trials }, (_, trial) => ({ decision, aTask, bTasks, trial: trial + 1 }))));
  if (jobs.length === 0) throw new Error("No selected decision has an A task and a B task.");
  const startedAt = new Date().toISOString();
  const codexVersion = execFileSync("codex", ["--version"], { encoding: "utf8" }).trim();
  console.log(`\nMerge Assist live two-agent check (${codexVersion}${options.model === undefined ? ", default model" : `, model ${options.model}`}): index ${base.path}; arms ${LIVE_ARMS.join(",")} (phase 1 only); ${options.trials} trials per decision; ${options.distractors} distractors; seed ${options.seed}; ${options.timeoutMin} min per Codex call; concurrency ${options.concurrency}`);
  console.log(table(["decision", "A task", "B tasks"], plan.map(({ decision, aTask, bTasks }) => [decision.id, aTask?.id ?? "(none: skipped)", bTasks.length === 0 ? "(none: skipped)" : bTasks.map(task => task.id).join(", ")])));
  const home = await realpath(await mkdtemp(join(tmpdir(), "rerail-ma-live-home-")));
  const previousHome = process.env.RERAIL_HOME;
  process.env.RERAIL_HOME = home;
  try {
    const context: LiveContext = { dataset, base, options, embedder: loadEmbedder(MODEL_PATH).embedder, limit: limiter(options.concurrency) };
    let done = 0;
    const trials = await Promise.all(jobs.map(async job => {
      const trial = await runLiveTrial(job, context);
      console.error(`trial ${++done}/${jobs.length}: ${trial.decisionId} #${trial.trial} ${trial.error ?? `A ${trial.a?.brokeD === true ? "broke" : "kept"} D, capture ${trial.capture.outcome}, ${trial.b.length} B runs, ${(trial.wallMs / 1000).toFixed(0)}s`}`);
      return trial;
    }));
    const decisions = [...new Set(jobs.map(job => job.decision.id))];
    const result = printLive(trials, decisions);
    if (options.json !== undefined) {
      const report = { format: LIVE_FORMAT, startedAt, codexVersion, options, index: { path: base.path, builtAt: base.builtAt }, arms: LIVE_ARMS, variants: Object.fromEntries(LIVE_ARMS.map(arm => [arm, VARIANTS[arm] ?? null])), plan: plan.map(({ decision, aTask, bTasks }) => ({ decisionId: decision.id, aTask: aTask?.id ?? null, bTasks: bTasks.map(task => task.id) })), trials, result };
      await writeFile(options.json, `${JSON.stringify(report, null, 2)}\n`);
      console.log(`\nWrote ${options.json}`);
    }
  } finally {
    if (previousHome === undefined) delete process.env.RERAIL_HOME;
    else process.env.RERAIL_HOME = previousHome;
    await rm(home, { recursive: true, force: true });
  }
}

// ---------------------------------------------------------------------------------------------------------- main

export async function pool<T, R>(items: readonly T[], concurrency: number, work: (item: T) => Promise<R>, progress: (done: number, result: R) => void): Promise<R[]> {
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
  if (options.listVariants) {
    console.log(table(["variant", "parameters"], Object.entries(VARIANTS).map(([name, parameters]) => [name, describeVariant(parameters)])));
    console.log(`\nParameters: when ${WHEN.join("|")}; budget ${BUDGETS.join("|")}; cap N|all; sessionCap N|all; dirs true|false; dedupe ${DEDUPE.join("|")}; format ${FORMATS.join("|")}; rank ${RANKS.join("|")}; threshold 0–1|none; only ${ONLY.join("|")}; conflict ${CONFLICT.join("|")}; channels ${CHANNELS.join("|")}; orient ${ORIENTS.join("|")}; areaRank ${AREA_RANKS.join("|")}; focusK N; guard ${GUARDS.join("|")}; embedThreshold 0–1; guardCap N`);
    return;
  }
  const dataset = loadDataset();
  if (options.buildIndex !== undefined) {
    await buildIndex(dataset, options.buildIndex, options);
    return;
  }
  if (options.sweep !== undefined) {
    sweep(dataset, options.sweep, options.thresholds ?? EMBED_GRID);
    return;
  }
  if (options.validateOnly) {
    process.exitCode = printOracleChecks(await validateOracles(dataset)) ? 0 : 1;
    return;
  }
  if (options.index === undefined) throw new Error(`Every run reads a built index: --index index.json (build it once with --build-index)\n${USAGE}`);
  const built = loadIndex(options.index);
  if (options.amend !== undefined) {
    await amendIndex(dataset, built, options.amend);
    return;
  }
  // Every distractor the index holds, unless --distractors asks for fewer: an index built for noise is run with it.
  if (options.distractors === null) options.distractors = built.distractors;
  if (options.replay !== undefined) {
    replay(dataset, built, options.replay, options.thresholds ?? [0, 0.05, 0.1, 0.15, 0.2, 0.25, 0.3, 0.4, 0.5]);
    return;
  }
  const unfrozen = options.variants.filter(name => (VARIANTS[name]?.guard === "embed" || VARIANTS[name]?.guard === "both") && VARIANTS[name]!.embedThreshold === null);
  if (unfrozen.length > 0) throw new Error(`${unfrozen.join(", ")} need EMBED_THRESHOLD, frozen from --sweep over a dev control run first.`);
  if (!printOracleChecks(await validateOracles(dataset))) throw new Error("An oracle grades a labelled file wrongly: fix it before spending model budget on runs.");
  if (!existsSync(AUTH)) throw new Error(`${AUTH} is missing: sign in to Codex first (codex login).`);
  if (options.live) {
    await runLive(dataset, built, options);
    return;
  }
  const tasks = selectTasks(dataset, options);
  if (tasks.length === 0) throw new Error("No tasks selected.");
  const names = options.arms === null ? options.variants : options.arms.map(arm => ARM_VARIANTS[arm]);
  const focusing = names.flatMap(name => VARIANTS[name] ?? []).filter(variant => variant.orient === "focus" || variant.orient === "area-focus");
  if (focusing.length > 0) {
    const checks = tasks.map(task => rankerLeaks(dataset, built, task, options.distractors ?? built.distractors, options.seed, focusing));
    const failures = checks.flatMap(check => check.failures);
    console.log(`\nRanker input integrity (the task prompt and decision texts under opaque ids; ${checks.reduce((sum, check) => sum + check.prompts, 0)} prompts for ${tasks.length} tasks): ${failures.length === 0 ? "PASS" : `FAIL (${failures.length})`}`);
    for (const failure of failures) console.log(`  FAIL ${failure}`);
    if (failures.length > 0) throw new Error("A focus ranker would see grader knowledge; no run starts.");
  }
  const labelled = names.filter(name => VARIANTS[name]?.orient === "targets");
  if (labelled.length > 0) {
    const absent = tasks.flatMap(task => {
      const index = runIndex(dataset, built, task, options.distractors ?? built.distractors, options.seed);
      return task.targets.filter(id => !index.some(entry => entry.id === id)).map(id => `${task.id}: ${id}`);
    });
    console.log(`\nSelection integrity for ${labelled.join(", ")}: DIAGNOSTIC. Its selection uses the task's labels: the hook config carries the task's targets, and they are the picks, with no ranker. What it injects is still each picked decision's text from the built index, rendered by the hook; nothing hand-written reaches the agent. Targets absent from the run index (never injected): ${absent.join(", ") || "none"}`);
  }
  if (options.arms !== null) {
    await runArms(dataset, built, tasks, options);
    return;
  }
  const jobs = tasks.flatMap(task => Array.from({ length: options.trials }, (_, trial) => options.variants.map(variant => ({ variant, task, trial: trial + 1 }))).flat());
  const startedAt = new Date().toISOString();
  const codexVersion = execFileSync("codex", ["--version"], { encoding: "utf8" }).trim();
  console.log(`\nMerge Assist context eval (${codexVersion}${options.model === undefined ? ", default model" : `, model ${options.model}`}): index ${built.path} (built ${built.builtAt}${built.absent.length === 0 ? "" : `; absent: ${built.absent.map(entry => entry.id).join(", ")}`}); variants ${options.variants.join(",")}; ${tasks.length} tasks × ${options.trials} trials = ${jobs.length} runs; ${options.distractors} distractors; concurrency ${options.concurrency}`);
  for (const variant of options.variants) console.log(`  ${variant.padEnd(18)} ${describeVariant(VARIANTS[variant]!)}`);
  const runs = await pool(jobs, options.concurrency, job => runJob(job, dataset, built, options), (done, run) => {
    console.error(`run ${done}/${jobs.length}: ${run.variant} ${run.taskId} #${run.trial} ${run.error ?? `${run.violated ? "VIOLATION" : "clean"}, ${run.done ? "done" : "not done"}, ${run.injections} injections (${run.injectedTokens} tokens), ${(run.wallMs / 1000).toFixed(0)}s`}`);
  });
  const control = options.control ? summarize(CONTROL, runs.filter(run => run.variant === CONTROL), undefined) : undefined;
  const summaries = options.variants.map(variant => (variant === CONTROL ? control! : summarize(variant, runs.filter(run => run.variant === variant), control)));
  if (options.verbose) printRuns(runs);
  const models = [...new Set(runs.flatMap(run => (run.model === null ? [] : [run.model])))];
  printSummary(summaries);
  console.log(`\nModel${models.length === 1 ? "" : "s"} (from Codex's transcripts): ${models.length === 0 ? "unknown" : models.join(", ")}`);
  if (options.json !== undefined) {
    const report = { startedAt, codexVersion, options, index: { path: built.path, builtAt: built.builtAt }, variants: Object.fromEntries(options.variants.map(name => [name, VARIANTS[name]])), tasks: tasks.map(task => task.id), runs, summaries };
    await writeFile(options.json, `${JSON.stringify(report, null, 2)}\n`);
    console.log(`\nWrote ${options.json}`);
  }
}

if (process.argv[1] !== undefined && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) await main();
