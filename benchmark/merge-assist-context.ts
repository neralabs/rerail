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
 *
 * The decision index is built once, from the product's own code paths, never by hand: `--build-index` runs
 * `draftDecision` with the person's agent CLI judge on each real decision's capture case in cases.json (its correction,
 * before and after, in a temporary workspace of the project) and keeps the decision text, scope and validated signals
 * it produces; a decision the judge calls one-off, or does not answer, stays out of the index. Distractors and the
 * conflict tasks' teammate decisions, hand-written noise with no example, get their signals from `deriveSignals`. Each
 * entry records its provenance, and its decision text's embedding (the bundled model) is stored beside it. Every run
 * reads that file through `--index`, and an integrity check refuses an index that carries grader knowledge.
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
import { fileURLToPath } from "node:url";
import { z } from "zod";
import {
  type CheckoutContext,
  globPattern,
  openRerailDatabase,
  PACKAGE_ROOT,
  type RerailDatabase,
  recordCheckout,
  setUpRepository,
} from "../src/engine/index.js";
import { type AgentJudge, createAgentJudge, deriveSignals, draftDecision, type DraftOutcome, setRepositoryStarted } from "../src/runtime/index.js";
import { fixturesPath } from "../test/support/oracles.js";
import {
  advance,
  BUDGETS,
  type Channel,
  CHANNELS,
  choose,
  CONFLICT,
  DEDUPE,
  EMBED_MAX_TOKENS,
  fires,
  FORMATS,
  type GuardScore,
  GUARDS,
  type GuardSource,
  type HookConfig,
  type HookLogEntry,
  type IndexEntry,
  INITIAL_STATE,
  loadEmbedder,
  ONLY,
  ORIENTS,
  AREA_RANKS,
  RANKS,
  render,
  scopeMatcher,
  type VariantParameters,
  WHEN,
} from "./merge-assist-context-hook.js";

const DATASET = fixturesPath("host", "benchmark", "merge-assist", "context.json");
const HOOK = fileURLToPath(new URL("./merge-assist-context-hook.ts", import.meta.url));
const REPOSITORY_TSCONFIG = fileURLToPath(new URL("../tsconfig.json", import.meta.url));
const AUTH = join(homedir(), ".codex", "auth.json");
const WORK_BRANCH = "tbelanger/work";
const OWN_AUTHOR = "Tim Belanger";
const RUN_TIMEOUT_MS = 6 * 60_000;
const SPLITS = ["dev", "heldout"] as const;
const KINDS = ["tempt", "neutral", "conflict"] as const;

// ---------------------------------------------------------------------------------------------------------- variants

/** One selection channel: everything but `hybrid`. */
const SINGLE = { channels: "single", orient: null, areaRank: null, guard: null, embedThreshold: null, guardCap: null } as const;
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
};
const CONTROL = "none";

function describeVariant(parameters: VariantParameters | null): string {
  if (parameters === null) return "control: no injection (the hook only logs)";
  const embed = parameters.guard === "embed" || parameters.guard === "both" ? ` embedThreshold=${parameters.embedThreshold ?? "NOT FROZEN"}` : "";
  const channels = parameters.channels === "hybrid" ? ` channels=hybrid orient=${parameters.orient}${parameters.orient === "area" ? ` areaRank=${parameters.areaRank}` : ""} guard=${parameters.guard ?? "none"} guardCap=${parameters.guardCap}${embed}` : "";
  return `when=${parameters.when} budget=${parameters.budget} cap=${parameters.cap ?? "all"} sessionCap=${parameters.sessionCap ?? "all"} dirs=${parameters.dirs} dedupe=${parameters.dedupe} format=${parameters.format} rank=${parameters.rank} threshold=${parameters.threshold ?? "none"} only=${parameters.only} conflict=${parameters.conflict}${channels}`;
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
const DISTRACTOR_AUTHORS = ["Jeremy Barnes", "Priya Natarajan", "Sam Okafor", "Lena Fischer", "Ana Costa", "Wei Zhang", "Tomás Ruiz"];

/**
 * `count` distractors from the dataset's pool, the same ones in the same places for every run: alternately team
 * decisions (with a pull request) and teammates' branch decisions, each by one of the team. Their text and scope are
 * hand-written noise; their signals come from `deriveSignals` when the index is built.
 */
function distractorPlaces(dataset: Dataset, count: number): Array<Omit<BuiltEntry, "signals" | "provenance">> {
  if (count > dataset.distractors.length) throw new Error(`--distractors takes at most ${dataset.distractors.length}, the dataset's pool`);
  const random = seeded(DISTRACTOR_SEED);
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
  /** `draftDecision`: the capture case in cases.json it ran on, and what the candidate became. */
  case: z.string().optional(),
  candidateId: z.string().optional(),
  generality: z.string().optional(),
  level: z.enum(["code", "note"]).optional(),
  ruleDropped: z.string().nullable().optional(),
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
 * favour the real decisions. A decision absent from the index stays absent.
 */
function runIndex(dataset: Dataset, built: BuiltIndex, task: Task, distractors: number): IndexEntry[] {
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
  const random = seeded(DISTRACTOR_SEED + 1);
  for (const place of distractorPlaces(dataset, distractors)) {
    const entry = find(place.id);
    if (entry !== undefined) index.splice(Math.floor(random() * (index.length + 1)), 0, hookEntry(entry));
  }
  return index;
}

// ---------------------------------------------------------------------------------------------------------- options

interface Options {
  variants: string[];
  split: "dev" | "heldout" | "all" | null;
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
}

const USAGE = "Usage: node --import tsx benchmark/merge-assist-context.ts --build-index out.json [--distractors N] [--concurrency N] | --index index.json --variants none,guard-signals,... --split dev|heldout|all [--tasks id,id] [--trials N] [--concurrency N] [--distractors N] [--no-control] [--json out.json] [--keep] [--verbose] [--model m] [-c key=value] | --index index.json --replay run.json [--thresholds 0,0.1,...] | --sweep control.json [--thresholds 0.3,0.35,...] | --list-variants | --validate-oracles";

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
    control: true,
    replay: undefined,
    sweep: undefined,
    thresholds: null,
    index: undefined,
    buildIndex: undefined,
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
        if (split !== "dev" && split !== "heldout" && split !== "all") throw new Error(`--split takes dev, heldout or all\n${USAGE}`);
        options.split = split;
        break;
      }
      case "--tasks": options.tasks = list(flag, value()); break;
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

/** The tasks to run: the split's (dev by default; every split when `--tasks` names them), narrowed by `--tasks`. */
function selectTasks(dataset: Dataset, options: Options): Task[] {
  const split = options.split ?? (options.tasks === null ? "dev" : "all");
  const unknown = (options.tasks ?? []).filter(id => !dataset.tasks.some(task => task.id === id));
  if (unknown.length > 0) throw new Error(`Unknown task id: ${unknown.join(", ")}`);
  return dataset.tasks.filter(task => (split === "all" || task.split === split) && (options.tasks === null || options.tasks.includes(task.id)));
}

// ---------------------------------------------------------------------------------------------------------- git

/** Git for the eval's own commands, isolated from the person's global and system configuration. */
const GIT_ENV: NodeJS.ProcessEnv = {
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

/** A repository at `repo` with the project committed on `main`, and `branch` checked out from it. */
function createRepository(repo: string, project: Readonly<Record<string, string>>, branch: string): void {
  mkdirSync(repo, { recursive: true });
  git(repo, "init", "-q", "-b", "main");
  git(repo, "config", "commit.gpgsign", "false");
  writeTree(repo, project);
  git(repo, "add", "-A");
  git(repo, "commit", "-q", "-m", "Orders service");
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

interface Usage {
  input: number;
  cached: number;
  output: number;
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
  readonly folder: string | null;
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

/** A temporary CODEX_HOME: the person's sign-in through a symlink, no configuration, and the eval hook. */
function createCodexHome(home: string, configPath: string): void {
  mkdirSync(home, { recursive: true });
  symlinkSync(AUTH, join(home, "auth.json"));
  writeFileSync(join(home, "config.toml"), "");
  const loader = fileURLToPath(import.meta.resolve("tsx"));
  const command = [
    "env",
    `MA_CONTEXT_CONFIG=${shellSafe(configPath)}`,
    `TSX_TSCONFIG_PATH=${shellSafe(REPOSITORY_TSCONFIG)}`,
    shellSafe(process.execPath),
    "--import",
    shellSafe(loader),
    shellSafe(HOOK),
  ].join(" ");
  const group = [{ matcher: "", hooks: [{ type: "command", command }] }];
  writeFileSync(join(home, "hooks.json"), `${JSON.stringify({ hooks: { PreToolUse: group, PostToolUse: group } }, null, 2)}\n`);
}

interface CodexOutcome {
  readonly stdout: string;
  readonly stderr: string;
  readonly exitCode: number | null;
  readonly timedOut: boolean;
  readonly wallMs: number;
}

function runCodex(repo: string, codexHome: string, configPath: string, prompt: string, options: Options, rerailHome: string): Promise<CodexOutcome> {
  const args = ["exec", "--json", "--dangerously-bypass-hook-trust", "--sandbox", "workspace-write", "--skip-git-repo-check", "-C", repo];
  if (options.model !== undefined) args.push("-m", options.model);
  for (const setting of options.config) args.push("-c", setting);
  args.push("-");
  const started = performance.now();
  const { promise, resolve, reject } = Promise.withResolvers<CodexOutcome>();
  const child = spawn("codex", args, {
    cwd: repo,
    detached: true,
    env: { ...process.env, CODEX_HOME: codexHome, MA_CONTEXT_CONFIG: configPath, RERAIL_HOME: rerailHome, RERAIL_AUTOSTART: "0" },
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
  }, RUN_TIMEOUT_MS);
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

/** What Codex's `--json` events say: the last agent message, token usage, commands run, and any reported failure. */
function readEvents(stdout: string): { finalMessage: string | null; usage: Usage | null; commands: number; error: string | null } {
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
    if (event.type === "item.completed" && item?.type === "agent_message" && typeof item.text === "string") finalMessage = item.text;
    if (event.type === "item.completed" && item?.type === "command_execution") commands++;
    if (event.type === "turn.completed" && event.usage !== null && typeof event.usage === "object") {
      const fields = event.usage as Record<string, unknown>;
      const count = (value: unknown) => (typeof value === "number" ? value : 0);
      usage ??= { input: 0, cached: 0, output: 0 };
      usage.input += count(fields.input_tokens);
      usage.cached += count(fields.cached_input_tokens);
      usage.output += count(fields.output_tokens);
    }
    if (event.type === "turn.failed" || event.type === "error") {
      const detail = (event.error as Record<string, unknown> | undefined)?.message ?? event.message;
      error = typeof detail === "string" ? detail : JSON.stringify(event);
    }
  }
  return { finalMessage, usage, commands, error };
}

function readLog(path: string): HookLogEntry[] {
  if (!existsSync(path)) return [];
  return readFileSync(path, "utf8").split("\n").filter(line => line.trim() !== "").map(line => JSON.parse(line) as HookLogEntry);
}

/** Codex's own transcripts of the run, under the temporary CODEX_HOME, as raw text; null when it wrote none. */
function transcriptText(codexHome: string): string | null {
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

async function runJob(job: Job, dataset: Dataset, built: BuiltIndex, options: Options): Promise<RunResult> {
  const { task } = job;
  const folder = await realpath(await mkdtemp(join(tmpdir(), "rerail-ma-context-")));
  const repo = join(folder, "repo");
  const codexHome = join(folder, "codex-home");
  const configPath = join(folder, "hook-config.json");
  const logPath = join(folder, "hook.log");
  const index = runIndex(dataset, built, task, options.distractors ?? built.distractors);
  const base = { variant: job.variant, taskId: task.id, split: task.split, kind: task.kind, trial: job.trial, targets: task.targets };
  try {
    createRepository(repo, dataset.project, WORK_BRANCH);
    const config: HookConfig = {
      variant: VARIANTS[job.variant]!,
      index,
      prompt: task.prompt,
      embeddings: Object.fromEntries(index.map(entry => [entry.id, built.embeddings[entry.id]!])),
      modelPath: MODEL_PATH,
      files: Object.keys(dataset.project),
      root: repo,
      log: logPath,
      state: join(folder, "hook-state.json"),
    };
    writeFileSync(configPath, `${JSON.stringify(config, null, 2)}\n`);
    writeFileSync(logPath, "");
    createCodexHome(codexHome, configPath);
    const outcome = await runCodex(repo, codexHome, configPath, task.prompt, options, join(folder, "rerail-home"));
    const events = readEvents(outcome.stdout);
    const log = readLog(logPath);
    const changes = diffAgainstMain(repo);
    const hits = grade(changes, dataset.decisions);
    const graded = task.kind === "neutral" ? Object.keys(hits) : task.targets;
    const files = workspaceFiles(repo);
    const doneMisses = task.done.filter(done => {
      const matcher = globPattern([done.path]);
      const pattern = new RegExp(done.regex, "u");
      return !files.some(path => matcher.test(path) && pattern.test(readFileSync(join(repo, path), "utf8")));
    }).map(done => `${done.path} /${done.regex}/`);
    const natural = globPattern([...task.done.map(done => done.path), ...task.expectedFiles]);
    const injectedCalls = log.filter(call => call.injected.length > 0);
    const injectedDecisions = injectedCalls.flatMap(call => call.injected.map(item => item.decisionId));
    const injectedChars = injectedCalls.reduce((sum, call) => sum + call.chars, 0);
    const relevant = new Set([...task.targets, ...(task.conflict === undefined ? [] : [task.conflict.peer.id])]);
    const transcript = transcriptText(codexHome);
    // What the agent says, not where: link targets and absolute paths name the temporary folders, which say "rerail".
    const message = (events.finalMessage ?? "").replace(/\]\([^)]*\)/gu, "]").replace(/(?<![\w.-])~?\/[\w.@+/-]+/gu, " ");
    const failure = outcome.timedOut
      ? `timed out after ${RUN_TIMEOUT_MS / 60_000} minutes`
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
function wilson(value: Ratio): [number, number] {
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

function median(values: readonly number[]): number {
  if (values.length === 0) return Number.NaN;
  const sorted = [...values].sort((left, right) => left - right);
  const middle = Math.floor(sorted.length / 2);
  return sorted.length % 2 === 1 ? sorted[middle]! : (sorted[middle - 1]! + sorted[middle]!) / 2;
}

function mean(values: readonly number[]): number {
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
function replayRun(dataset: Dataset, built: BuiltIndex, distractors: number, run: RunResult, variant: VariantParameters): ReplayOutcome {
  const task = dataset.tasks.find(entry => entry.id === run.taskId)!;
  const index = runIndex(dataset, built, task, distractors);
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
  const report = JSON.parse(readFileSync(path, "utf8")) as { options: { distractors?: number }; variants: Record<string, VariantParameters | null>; runs: RunResult[] };
  const distractors = report.options.distractors ?? 0;
  const rows: string[][] = [];
  for (const [name, parameters] of Object.entries(report.variants)) {
    if (parameters === null) continue;
    const runs = report.runs.filter(run => run.variant === name && run.error === null && (run.hookLog ?? []).some(entry => entry.candidates !== undefined));
    if (runs.length === 0) {
      console.log(`skip ${name}: its runs logged no scored candidates (a report from before relevance scoring?)`);
      continue;
    }
    // `hybrid` has no threshold to sweep: its replay is the run's own selection, re-chosen.
    for (const threshold of [...new Set([parameters.threshold, ...(parameters.channels === "hybrid" ? [] : thresholds)])]) {
      const outcomes = runs.map(run => replayRun(dataset, built, distractors, run, { ...parameters, threshold }));
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

// ---------------------------------------------------------------------------------------------------------- main

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
  if (options.listVariants) {
    console.log(table(["variant", "parameters"], Object.entries(VARIANTS).map(([name, parameters]) => [name, describeVariant(parameters)])));
    console.log(`\nParameters: when ${WHEN.join("|")}; budget ${BUDGETS.join("|")}; cap N|all; sessionCap N|all; dirs true|false; dedupe ${DEDUPE.join("|")}; format ${FORMATS.join("|")}; rank ${RANKS.join("|")}; threshold 0–1|none; only ${ONLY.join("|")}; conflict ${CONFLICT.join("|")}; channels ${CHANNELS.join("|")}; orient ${ORIENTS.join("|")}; areaRank ${AREA_RANKS.join("|")}; guard ${GUARDS.join("|")}; embedThreshold 0–1; guardCap N`);
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
  const tasks = selectTasks(dataset, options);
  if (tasks.length === 0) throw new Error("No tasks selected.");
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
