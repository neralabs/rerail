/**
 * Measures the three code-rule steer strategies (`message`, `rewind`, `subagent`) on real coding agents.
 *
 *   node --import tsx benchmark/steer-strategy.ts [--hosts pi,omp,codex,cursor,claude-code]
 *     [--strategies message,rewind,subagent] [--trials N] [--json out.json] [--timeout-ms N] [--keep]
 *     [--compaction-probe] [-- extra host args]
 *
 * Each trial copies `fixtures/host/benchmark/steer-strategy/project` into a fresh Seatbelt sandbox with a private
 * RERAIL_HOME and an in-process daemon (the harness reaches it through RERAIL_ENGINE_SOCKET, RERAIL_AUTOSTART=0),
 * starts the repository with one steer-mode code rule (an f-string SQL `.execute`, scoped to `api/**`), stores the
 * strategy in the checkout's audit settings, and runs the harness in print mode on `prompt.md`: a multi-step task whose
 * first step writes the forbidden call, so the steer lands with work remaining. After the harness exits the runner
 * reads the outcome from the project, Rerail's ledger, and the harness's own transcript or JSON output.
 *
 * For Pi and OMP it also replays each host's compaction cut logic offline over the transcript, at every
 * keepRecentTokens threshold where the cut moves, and reports how many split-turn cuts would summarize a
 * `rerail.rule` steer as the turn's original request instead of the user's prompt.
 */
import { spawn } from "node:child_process";
import { cp, mkdir, mkdtemp, readFile, readdir, realpath, rm, writeFile } from "node:fs/promises";
import { join, resolve } from "node:path";
import { createInterface } from "node:readline";
import { fileURLToPath } from "node:url";
import { z } from "zod";
import {
  estimateTokens,
  findCutPoint,
  sessionEntryToContextMessages,
  SessionManager,
  type SessionEntry,
} from "@earendil-works/pi-coding-agent";
import {
  type RerailHost,
  type SteerStrategy,
  defaultRerailDatabasePath,
  loadRuleCatalog,
  openRerailDatabase,
  recordCheckout,
  RULE_MESSAGE_TYPE,
  SESSION_NOTE_MESSAGE_TYPE,
  setUpRepository,
  SqliteAuditControlStore,
  STEER_STRATEGIES,
  upsertLibraryRule,
} from "../src/engine/index.js";
import { RerailDaemon } from "../src/daemon/index.js";
import { setRepositoryStarted } from "../src/runtime/index.js";
import { fixturesPath } from "../test/support/oracles.js";
import { CODE_RULE } from "../scripts/host/host-lifecycle.js";
import {
  DIST,
  ROOT,
  bounded,
  configureHostAuth,
  ensureFresh,
  hostCli,
  hostCommand,
  jsonlFiles,
  readDeliveries,
  run,
  sandboxAvailable,
  sandboxEnvironment,
  sandboxLeaks,
  sandboxProfile,
  version,
} from "../scripts/host/host-harness.js";

const HOSTS: readonly RerailHost[] = ["pi", "omp", "codex", "cursor", "claude-code"];
const FIXTURE = fixturesPath("host", "benchmark", "steer-strategy");
/** The lifecycle proof's f-string SQL rule, scoped to the package so the fixture's tests never trigger it. */
const RULE = { ...CODE_RULE, id: "rerail-steer-benchmark:sql-execute-fstring", scope: { include: ["api/**/*.py"], exclude: [] } };
/** The rule's violation as a line regex: `.execute(f"...WHERE|VALUES|SET... {`. Multi-line f-strings are not seen. */
const VIOLATION = /\.execute\(\s*(?:r?f|fr)(["'])[^\n]*?\b(?:WHERE|VALUES|SET|HAVING|LIKE|IN\s*\()[^\n]*?\{/iu;
/** Each harness's subagent tool, as the `subagent` steer names it; Claude Code's newer builds call Task `Agent`. */
const SUBAGENT_TOOLS: Record<RerailHost, readonly string[]> = {
  pi: ["rerail_fix"],
  omp: ["task"],
  codex: ["spawn_agent"],
  "claude-code": ["Task", "Agent"],
  cursor: ["Task"],
};
/** The built entry each harness loads; a trial refuses to run it when `src/` is newer. */
const BUILT: Record<RerailHost, string> = {
  pi: join(DIST, "hosts", "pi", "pi-extension.js"),
  omp: join(DIST, "hosts", "omp", "default-extension.js"),
  codex: join(DIST, "hosts", "codex", "codex-hook.js"),
  cursor: join(DIST, "hosts", "cursor", "cursor-hook.js"),
  "claude-code": join(DIST, "hosts", "claude", "claude-hook.js"),
};
const SESSION_DIR: Partial<Record<RerailHost, string>> = { pi: ".pi-sessions", omp: ".omp-sessions" };
const PROBE_QUESTION = "Without using any tools, answer in one sentence: what task did the user ask you to do in this session?";
/** A harness's JSON stream for a multi-step task outgrows host-smoke's 2 MB guard; this one only stops a runaway. */
const OUTPUT_LIMIT = 64_000_000;

export type Options = {
  hosts: RerailHost[];
  strategies: SteerStrategy[];
  trials: number;
  json: string | undefined;
  timeoutMs: number;
  keep: boolean;
  compactionProbe: boolean;
  extra: string[];
};
type Usage = { input: number; output: number; cacheRead: number; cacheWrite: number; cost: number | null };
/** What a split turn starts at: a steer message, Rerail's session-start note (also a custom message), the user's prompt, or anything else. */
type TurnStart = "rule" | "note" | "prompt" | "other";
/** One distinct split-turn cut the sweep reached: the smallest keepRecentTokens that produces it, and its turn start. */
type SplitCut = { keepRecentTokens: number; firstKeptEntryIndex: number; turnStartIndex: number; start: TurnStart };
export type Attribution = { thresholds: number; cuts: number; splitCuts: SplitCut[]; ruleShare: number | null };
type Probe =
  | { keepRecentTokens: number; compacted: boolean; goal: string; originalRequest: string; answer: string; names: "task" | "rule" | "both" | "neither" }
  | { skipped: string };
type Trial = {
  host: RerailHost;
  strategy: SteerStrategy;
  trial: number;
  error?: string;
  exitCode?: number;
  wallMs?: number;
  steers?: number;
  rewound?: number;
  /** Steers that rode their edit's tool result instead of a message. */
  inResult?: number;
  deliveredStrategies?: string[];
  testsPassed?: boolean;
  readmeUpdated?: boolean;
  taskCompleted?: boolean;
  violationRemaining?: boolean;
  subagentCalls?: number;
  usage?: Usage | null;
  compaction?: Attribution;
  probe?: Probe;
  kept?: string;
};

function usage(): never {
  throw new Error(
    "Usage: node --import tsx benchmark/steer-strategy.ts [--hosts pi,omp,codex,cursor,claude-code] [--strategies message,rewind,subagent] [--trials N] [--json out.json] [--timeout-ms N] [--keep] [--compaction-probe] [-- extra host args]",
  );
}
function list<T extends string>(value: string | undefined, allowed: readonly T[], flag: string): T[] {
  if (value === undefined) usage();
  const items = value.split(",").map((item) => item.trim()).filter(Boolean);
  const unknown = items.filter((item) => !allowed.includes(item as T));
  if (items.length === 0 || unknown.length > 0) throw new Error(`${flag}: unknown ${unknown.join(", ") || "(empty)"}; expected ${allowed.join(",")}`);
  return [...new Set(items as T[])];
}
function positive(value: string | undefined, flag: string): number {
  const number = Number(value);
  if (!Number.isSafeInteger(number) || number <= 0) throw new Error(`${flag} must be a positive integer, received ${JSON.stringify(value)}`);
  return number;
}
export function parseArgs(argv: readonly string[]): Options {
  const separator = argv.indexOf("--");
  const own = separator === -1 ? argv : argv.slice(0, separator);
  const options: Options = {
    hosts: [...HOSTS],
    strategies: [...STEER_STRATEGIES],
    trials: 1,
    json: undefined,
    timeoutMs: 900_000,
    keep: false,
    compactionProbe: false,
    extra: separator === -1 ? [] : argv.slice(separator + 1),
  };
  for (let index = 0; index < own.length; index += 1) {
    const flag = own[index];
    const value = own[index + 1];
    if (flag === "--keep") options.keep = true;
    else if (flag === "--compaction-probe") options.compactionProbe = true;
    else {
      if (flag === "--hosts") options.hosts = list(value, HOSTS, flag);
      else if (flag === "--strategies") options.strategies = list(value, STEER_STRATEGIES, flag);
      else if (flag === "--trials") options.trials = positive(value, flag);
      else if (flag === "--timeout-ms") options.timeoutMs = positive(value, flag);
      else if (flag === "--json" && value !== undefined) options.json = resolve(value);
      else usage();
      index += 1;
    }
  }
  return options;
}

/** Starts the scratch repository with the one rule, as `rerail start` would leave it, and stores the strategy. */
function seedRepository(databasePath: string, project: string, strategy: SteerStrategy): void {
  const database = openRerailDatabase(databasePath);
  try {
    upsertLibraryRule(database, loadRuleCatalog(), RULE, "code", { approve: true });
    const { repositoryKey } = recordCheckout(database, project);
    setUpRepository(database, repositoryKey, { packs: [], rules: [{ sourceKind: "library_rule", sourceId: RULE.id, folder: "" }] });
    setRepositoryStarted(database, repositoryKey, true);
  } finally {
    database.$client.close();
  }
  const controls = new SqliteAuditControlStore(databasePath);
  try {
    controls.updateWorkspaceSettings(project, { steerStrategy: strategy });
  } finally {
    controls.close();
  }
}

const LedgerDelivery = z.object({
  delivery: z.object({
    requested: z.string(),
    ruleIds: z.array(z.string()).default([]),
    strategy: z.string().optional(),
    toolCallId: z.string().optional(),
    rewound: z.array(z.string()).optional(),
  }),
});
/**
 * The rule's steers in Rerail's ledger: how many, how many rolled their edit back (`rewound`), how many rode their
 * edit's tool result (`toolCallId`, where the harness can put them there), and the strategies they carried.
 */
async function readSteers(project: string): Promise<{ steers: number; rewound: number; inResult: number; strategies: string[] }> {
  const steers = (await readDeliveries(join(project, ".rerail", "sessions"))).flatMap((entry) => {
    const delivery = LedgerDelivery.safeParse(entry).data?.delivery;
    return delivery !== undefined && delivery.requested === "steer" && delivery.ruleIds.includes(RULE.id) ? [delivery] : [];
  });
  return {
    steers: steers.length,
    rewound: steers.filter((steer) => (steer.rewound?.length ?? 0) > 0).length,
    inResult: steers.filter((steer) => steer.toolCallId !== undefined).length,
    strategies: [...new Set(steers.flatMap((steer) => (steer.strategy === undefined ? [] : [steer.strategy])))],
  };
}

async function pythonFiles(directory: string): Promise<string[]> {
  const names = await readdir(directory, { recursive: true }).catch(() => []);
  return names.filter((name) => name.endsWith(".py")).map((name) => join(directory, name));
}
/** How many tests the fixture's own suite runs; a run that reports another count did not run that suite. */
const FIXTURE_TESTS = 11;

/**
 * The task's own acceptance, judged by the runner: the unit tests pass, README usage is written, no violation is left.
 * The suite it runs is the fixture's, copied back over whatever the agent left in `tests/`, so an agent that edits or
 * deletes a test cannot score as done.
 */
async function inspectProject(
  project: string,
  profile: string,
  env: NodeJS.ProcessEnv,
): Promise<{ testsPassed: boolean; readmeUpdated: boolean; violationRemaining: boolean }> {
  await rm(join(project, "tests"), { recursive: true, force: true });
  await cp(join(FIXTURE, "project", "tests"), join(project, "tests"), { recursive: true });
  const tests = await run("sandbox-exec", ["-f", profile, "python3", "-m", "unittest", "discover", "-s", "tests"], { cwd: project, env, timeout: 120_000 });
  const ran = /^Ran (\d+) tests? in /mu.exec(tests.stderr)?.[1];
  const readme = await readFile(join(project, "README.md"), "utf8").catch(() => "");
  const usageSection = readme.split(/^## Usage\s*$/mu)[1]?.split(/^## /mu)[0] ?? "";
  const sources = await Promise.all((await pythonFiles(join(project, "api"))).map((path) => readFile(path, "utf8")));
  return {
    testsPassed: tests.exitCode === 0 && Number(ran) === FIXTURE_TESTS,
    readmeUpdated: !/TODO/u.test(usageSection) && /Inventory\(/u.test(usageSection) && /add_item/u.test(usageSection),
    violationRemaining: sources.some((source) => VIOLATION.test(source)),
  };
}

const NumberField = z.number().catch(0).default(0);
const TranscriptUsage = z.object({
  input: NumberField,
  output: NumberField,
  cacheRead: NumberField,
  cacheWrite: NumberField,
  cost: z.union([z.number(), z.object({ total: NumberField })]).optional(),
});
const TranscriptMessage = z.object({
  type: z.literal("message"),
  message: z.object({
    role: z.string(),
    content: z.union([z.string(), z.array(z.looseObject({ type: z.string(), name: z.string().optional() }))]).optional(),
    usage: TranscriptUsage.optional(),
    stopReason: z.string().optional(),
    errorMessage: z.string().optional(),
    toolName: z.string().optional(),
    details: z.object({ usage: TranscriptUsage.optional() }).loose().optional(),
  }),
});
function addUsage(total: Usage, usage: z.infer<typeof TranscriptUsage>): void {
  total.input += usage.input;
  total.output += usage.output;
  total.cacheRead += usage.cacheRead;
  total.cacheWrite += usage.cacheWrite;
  const cost = typeof usage.cost === "number" ? usage.cost : usage.cost?.total;
  if (cost !== undefined) total.cost = (total.cost ?? 0) + cost;
}

/** The harness's main session file: the one whose first user message is the prompt (subagents may write their own). */
async function mainTranscript(directory: string, prompt: string): Promise<{ file: string; entries: unknown[] } | undefined> {
  const firstLine = prompt.split("\n")[0] ?? prompt;
  for (const file of await jsonlFiles(directory)) {
    const entries = (await readFile(file, "utf8")).split("\n").filter(Boolean).map((line) => JSON.parse(line) as unknown);
    const user = entries.flatMap((entry) => {
      const message = TranscriptMessage.safeParse(entry).data?.message;
      return message?.role === "user" ? [message] : [];
    })[0];
    const text = typeof user?.content === "string" ? user.content : JSON.stringify(user?.content ?? "");
    if (text.includes(firstLine.slice(0, 60))) return { file, entries };
  }
  return undefined;
}
/** Pi/OMP transcript: subagent tool calls, assistant usage plus the usage each subagent result reports, and the last provider error. */
function transcriptMetrics(host: RerailHost, entries: readonly unknown[]): { subagentCalls: number; usage: Usage | null; modelError?: string } {
  const tools = SUBAGENT_TOOLS[host];
  const total: Usage = { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, cost: null };
  let subagentCalls = 0;
  let reported = false;
  let modelError: string | undefined;
  for (const entry of entries) {
    const message = TranscriptMessage.safeParse(entry).data?.message;
    if (message === undefined) continue;
    if (message.role === "assistant") {
      if (Array.isArray(message.content))
        subagentCalls += message.content.filter((block) => block.type === "toolCall" && block.name !== undefined && tools.includes(block.name)).length;
      if (message.usage !== undefined) {
        addUsage(total, message.usage);
        reported = true;
      }
      modelError = message.stopReason === "error" ? message.errorMessage ?? "assistant stopped with an error" : undefined;
    } else if (message.role === "toolResult" && message.toolName !== undefined && tools.includes(message.toolName) && message.details?.usage !== undefined) {
      addUsage(total, message.details.usage);
    }
  }
  return { subagentCalls, usage: reported ? total : null, ...(modelError === undefined ? {} : { modelError }) };
}

/** Every object nested in a JSON value. */
function* objects(value: unknown): Generator<Record<string, unknown>> {
  if (value === null || typeof value !== "object") return;
  if (!Array.isArray(value)) yield value as Record<string, unknown>;
  for (const child of Object.values(value)) yield* objects(child);
}
const count = (value: unknown): number => (typeof value === "number" && Number.isFinite(value) ? value : 0);
/**
 * Codex (`exec --json`), Claude Code and Cursor (`stream-json`) stdout: subagent tool calls, best-effort across event
 * shapes (a `name`/`tool_name` field, or Cursor's `<tool>ToolCall` key on a started call), and token usage from
 * Codex's `turn.completed` events or the final `result` event.
 */
export function streamMetrics(host: RerailHost, stdout: string): { subagentCalls: number; usage: Usage | null } {
  const tools = SUBAGENT_TOOLS[host];
  const cursorKeys = tools.map((tool) => `${tool.charAt(0).toLowerCase()}${tool.slice(1)}ToolCall`);
  const total: Usage = { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, cost: null };
  let subagentCalls = 0;
  let reported = false;
  for (const line of stdout.split("\n")) {
    let event: unknown;
    try {
      event = JSON.parse(line);
    } catch {
      continue;
    }
    if (event === null || typeof event !== "object") continue;
    const record = event as Record<string, unknown>;
    const started = record.subtype === undefined || record.subtype === "started";
    const completedItem = record.type !== "item.started";
    for (const object of objects(event)) {
      const name = object.name ?? object.tool_name ?? object.tool;
      if (completedItem && started && typeof name === "string" && tools.includes(name) && (object.type === "tool_use" || object.type === undefined || /tool|call/u.test(String(object.type)))) subagentCalls += 1;
      if (started && cursorKeys.some((key) => key in object)) subagentCalls += 1;
    }
    const usage = record.usage;
    if ((record.type === "turn.completed" || record.type === "result") && usage !== null && typeof usage === "object") {
      const fields = usage as Record<string, unknown>;
      reported = true;
      const cached = count(fields.cached_input_tokens) + count(fields.cache_read_input_tokens) + count(fields.cacheReadTokens);
      total.input += count(fields.input_tokens) + count(fields.inputTokens) - count(fields.cached_input_tokens);
      total.output += count(fields.output_tokens) + count(fields.outputTokens);
      total.cacheRead += cached;
      total.cacheWrite += count(fields.cache_creation_input_tokens) + count(fields.cacheWriteTokens);
    }
    if (record.type === "result" && typeof record.total_cost_usd === "number") total.cost = (total.cost ?? 0) + record.total_cost_usd;
  }
  return { subagentCalls, usage: reported ? total : null };
}

function classify(entry: { type: string; customType?: unknown; message?: { role?: unknown } } | undefined): TurnStart {
  if (entry?.type === "custom_message" && entry.customType === RULE_MESSAGE_TYPE) return "rule";
  if (entry?.type === "custom_message" && entry.customType === SESSION_NOTE_MESSAGE_TYPE) return "note";
  if (entry?.type === "message" && entry.message?.role === "user") return "prompt";
  return "other";
}
/**
 * Sweeps one host's cut function over every keepRecentTokens threshold at which its cut moves (the token total of
 * each suffix of counted entries) and keeps each distinct split-turn cut that summarizes a non-empty turn prefix.
 */
function sweep(
  entries: readonly { type: string; customType?: unknown; message?: { role?: unknown } }[],
  start: number,
  tokens: (index: number) => number,
  cut: (keepRecentTokens: number) => { firstKeptEntryIndex: number; turnStartIndex: number; isSplitTurn: boolean },
): Attribution {
  const thresholds: number[] = [];
  let suffix = 0;
  for (let index = entries.length - 1; index >= start; index -= 1) {
    const size = tokens(index);
    if (size === 0) continue;
    suffix += size;
    thresholds.push(suffix);
  }
  const cuts = new Map<number, SplitCut | undefined>();
  for (const keepRecentTokens of thresholds) {
    const point = cut(keepRecentTokens);
    if (cuts.has(point.firstKeptEntryIndex)) continue;
    // OMP reports a cut at a steer as split with turnStartIndex at the cut itself: nothing of the turn is summarized.
    const split = point.isSplitTurn && point.turnStartIndex < point.firstKeptEntryIndex;
    cuts.set(point.firstKeptEntryIndex, split ? { keepRecentTokens, firstKeptEntryIndex: point.firstKeptEntryIndex, turnStartIndex: point.turnStartIndex, start: classify(entries[point.turnStartIndex]) } : undefined);
  }
  const splitCuts = [...cuts.values()].filter((value): value is SplitCut => value !== undefined);
  const decided = splitCuts.filter((split) => split.start !== "other");
  return {
    thresholds: thresholds.length,
    cuts: cuts.size,
    splitCuts,
    ruleShare: decided.length === 0 ? null : decided.filter((split) => split.start === "rule").length / decided.length,
  };
}

/**
 * Pi's cut, through its exported `findCutPoint` and `estimateTokens` over the session's branch from the newest
 * compaction on. Pi's `prepareCompaction` runs the same walk over the session projection; the two agree on a session
 * with no compaction or context edit, which a trial is unless the harness auto-compacted.
 */
export function attributePi(entries: SessionEntry[]): Attribution {
  const start = entries.findLastIndex((entry) => entry.type === "compaction") + 1;
  return sweep(
    entries,
    start,
    (index) => sessionEntryToContextMessages(entries[index]!).reduce((sum, message) => sum + estimateTokens(message), 0),
    (keepRecentTokens) => findCutPoint(entries, start, entries.length, keepRecentTokens),
  );
}

type OmpEntry = { type: string; customType?: unknown; message?: Record<string, unknown> & { role?: unknown } };
/** OMP's Tokenizer without a model encoding: (UTF-8 bytes + 3) >> 2 per fragment (pi-agent-core src/tokenizer.ts:52-62). */
const ompBytes = (fragments: readonly string[]): number => fragments.reduce((sum, text) => sum + ((Buffer.byteLength(text, "utf-8") + 3) >> 2), 0);
/**
 * `Tokenizer.#measureMessage` from @oh-my-pi/pi-agent-core src/tokenizer.ts:184-286 (18.0.4) on its byte-estimate
 * path. The package is Bun-only TypeScript (Bun text imports, `bun` module), so it cannot be loaded here; a known
 * model encoding counts natively and gives other absolute numbers, which the threshold sweep makes irrelevant.
 */
function ompMessageTokens(message: Record<string, unknown>): number {
  const fragments: string[] = [];
  let extra = 0;
  const content = message.content;
  const blocks = Array.isArray(content) ? (content as Record<string, unknown>[]) : [];
  switch (message.role) {
    case "bashExecution":
      for (const field of [message.command, message.output]) if (typeof field === "string") fragments.push(field);
      return ompBytes(fragments);
    case "user":
      if (typeof content === "string") fragments.push(content);
      for (const block of blocks) if (block.type === "text" && typeof block.text === "string" && block.text) fragments.push(block.text);
      break;
    case "assistant":
      for (const block of blocks) {
        if (block.type === "text" && typeof block.text === "string") fragments.push(block.text);
        else if (block.type === "thinking") {
          if (typeof block.thinking === "string") fragments.push(block.thinking);
          if (typeof block.thinkingSignature === "string" && block.thinkingSignature) fragments.push(block.thinkingSignature);
        } else if (block.type === "toolCall") {
          if (typeof block.name === "string") fragments.push(block.name);
          fragments.push(JSON.stringify(block.arguments) ?? "null");
        } else if (block.type === "redactedThinking" && typeof block.data === "string") fragments.push(block.data);
        else if (block.type === "anthropicServerTool") fragments.push(JSON.stringify(block.block) ?? "null");
      }
      break;
    case "hookMessage":
    case "toolResult":
      if (typeof content === "string") fragments.push(content);
      for (const block of blocks) {
        if (block.type === "text" && typeof block.text === "string" && block.text) fragments.push(block.text);
        else if (block.type === "image") extra += 1200;
      }
      break;
    case "branchSummary":
    case "compactionSummary":
      if (typeof message.summary === "string") fragments.push(message.summary);
      break;
    default:
      return 0;
  }
  return extra + ompBytes(fragments);
}
/**
 * `findValidCutPoints`, `findTurnStartIndex` and `findCutPoint` from @oh-my-pi/pi-agent-core
 * src/compaction/compaction.ts:417-562 (18.0.4), line for line: only `message` entries carry tokens, and a
 * `custom_message` (a Rerail steer) is both a cut point and a turn start.
 */
function ompCutPoint(entries: readonly OmpEntry[], start: number, end: number, keepRecentTokens: number): { firstKeptEntryIndex: number; turnStartIndex: number; isSplitTurn: boolean } {
  const cutPoints: number[] = [];
  for (let index = start; index < end; index += 1) {
    const entry = entries[index]!;
    const role = entry.message?.role;
    if (entry.type === "message" && typeof role === "string" && ["bashExecution", "hookMessage", "branchSummary", "compactionSummary", "user", "assistant"].includes(role)) cutPoints.push(index);
    if (entry.type === "branch_summary" || entry.type === "custom_message") cutPoints.push(index);
  }
  if (cutPoints.length === 0) return { firstKeptEntryIndex: start, turnStartIndex: -1, isSplitTurn: false };
  let accumulated = 0;
  let cutIndex = cutPoints[0]!;
  for (let index = end - 1; index >= start; index -= 1) {
    const entry = entries[index]!;
    if (entry.type !== "message" || entry.message === undefined) continue;
    accumulated += ompMessageTokens(entry.message);
    if (accumulated >= keepRecentTokens) {
      cutIndex = cutPoints.find((candidate) => candidate >= index) ?? cutIndex;
      break;
    }
  }
  while (cutIndex > start) {
    const previous = entries[cutIndex - 1]!;
    if (previous.type === "compaction" || previous.type === "message") break;
    cutIndex -= 1;
  }
  const cutEntry = entries[cutIndex]!;
  const isUserMessage = cutEntry.type === "message" && cutEntry.message?.role === "user";
  let turnStartIndex = -1;
  if (!isUserMessage) {
    for (let index = cutIndex; index >= start; index -= 1) {
      const entry = entries[index]!;
      const role = entry.message?.role;
      if (entry.type === "branch_summary" || entry.type === "custom_message" || (entry.type === "message" && (role === "user" || role === "bashExecution"))) {
        turnStartIndex = index;
        break;
      }
    }
  }
  return { firstKeptEntryIndex: cutIndex, turnStartIndex, isSplitTurn: !isUserMessage && turnStartIndex !== -1 };
}
/** OMP's cut over the session file's entries (print mode writes one linear branch) after the newest compaction or `/clear`. */
export function attributeOmp(fileEntries: readonly unknown[]): Attribution {
  const entries = fileEntries.filter((entry): entry is OmpEntry =>
    entry !== null && typeof entry === "object" && "type" in entry && typeof entry.type === "string" && entry.type !== "session");
  const start = entries.findLastIndex((entry) => entry.type === "compaction" || entry.type === "reset_boundary") + 1;
  return sweep(
    entries,
    start,
    (index) => {
      const entry = entries[index]!;
      return entry.type === "message" && entry.message !== undefined ? ompMessageTokens(entry.message) : 0;
    },
    (keepRecentTokens) => ompCutPoint(entries, start, entries.length, keepRecentTokens),
  );
}

/** Which of the user's task and the rule's steer an answer names. */
function names(text: string): "task" | "rule" | "both" | "neither" {
  const task = /inventor|SPEC\.md|stock|unittest|README/iu.test(text);
  const rule = /parameteri[sz]|f-string|string formatting|SQL_EXECUTE|placeholder|injection/iu.test(text);
  if (task && rule) return "both";
  if (task) return "task";
  return rule ? "rule" : "neither";
}
/**
 * `--compaction-probe` (Pi only): forks the trial's session, compacts it for real in RPC mode with Rerail loaded (its
 * compaction guard included) at the smallest keepRecentTokens whose cut summarizes a steer-started turn (else any
 * split turn), then asks what the user asked for. The threshold goes into the sandbox's global Pi settings, which need
 * no project trust.
 */
async function compactionProbe(
  session: string,
  attribution: Attribution,
  project: string,
  profile: string,
  env: NodeJS.ProcessEnv,
  extra: readonly string[],
  timeoutMs: number,
): Promise<Probe> {
  const target = attribution.splitCuts.find((split) => split.start === "rule") ?? attribution.splitCuts[0];
  if (target === undefined) return { skipped: "no split-turn cut in the transcript" };
  const agentDir = env.PI_CODING_AGENT_DIR;
  if (agentDir === undefined) return { skipped: "PI_CODING_AGENT_DIR is unset" };
  await writeFile(join(agentDir, "settings.json"), JSON.stringify({ compaction: { keepRecentTokens: target.keepRecentTokens } }));
  const { args } = hostCommand("pi", project, "");
  const flags = args.slice(0, args.indexOf("--session-dir"));
  const child = spawn("sandbox-exec", ["-f", profile, "pi", ...flags.map((flag) => (flag === "json" ? "rpc" : flag)), "--no-extensions", "--extension", BUILT.pi, "--session-dir", join(project, ".pi-probe-sessions"), ...extra, "--fork", session], {
    cwd: project,
    env,
    stdio: ["pipe", "pipe", "pipe"],
  });
  const events: Record<string, unknown>[] = [];
  const waiters: { test: (event: Record<string, unknown>) => boolean; settle: (event: Record<string, unknown> | undefined) => void }[] = [];
  createInterface({ input: child.stdout }).on("line", (line) => {
    let event: unknown;
    try {
      event = JSON.parse(line);
    } catch {
      return;
    }
    if (event === null || typeof event !== "object") return;
    const record = event as Record<string, unknown>;
    events.push(record);
    for (const waiter of waiters.filter((candidate) => candidate.test(record))) {
      waiters.splice(waiters.indexOf(waiter), 1);
      waiter.settle(record);
    }
  });
  child.stderr.resume();
  const exited = new Promise<void>((settle) => child.once("close", () => settle()));
  const request = async (command: Record<string, unknown>, done: (event: Record<string, unknown>) => boolean): Promise<Record<string, unknown> | undefined> => {
    const { promise, resolve: settle } = Promise.withResolvers<Record<string, unknown> | undefined>();
    waiters.push({ test: done, settle });
    const timer = setTimeout(() => settle(undefined), timeoutMs);
    void exited.then(() => settle(undefined));
    child.stdin.write(`${JSON.stringify(command)}\n`);
    return promise.finally(() => clearTimeout(timer));
  };
  try {
    const compacted = await request({ id: "compact", type: "compact" }, (event) => event.type === "response" && event.id === "compact");
    const data = compacted?.data !== null && typeof compacted?.data === "object" ? (compacted.data as Record<string, unknown>) : {};
    const summary = typeof data.summary === "string" ? data.summary : "";
    await request({ id: "ask", type: "prompt", message: PROBE_QUESTION }, (event) => event.type === "agent_settled");
    const replies = events.flatMap((event) => {
      const message = event.type === "message_end" && event.message !== null && typeof event.message === "object" ? (event.message as Record<string, unknown>) : undefined;
      if (message?.role !== "assistant" || !Array.isArray(message.content)) return [];
      return [(message.content as Record<string, unknown>[]).flatMap((block) => (block.type === "text" && typeof block.text === "string" ? [block.text] : [])).join("")];
    });
    const answer = replies.at(-1)?.trim() ?? "";
    return {
      keepRecentTokens: target.keepRecentTokens,
      compacted: compacted?.success === true,
      goal: (summary.split("## Goal")[1]?.split("\n## ")[0] ?? "").trim(),
      originalRequest: (summary.split("## Original Request")[1]?.split("\n## ")[0] ?? "").trim(),
      answer,
      names: names(answer),
    };
  } finally {
    child.kill("SIGTERM");
    await exited;
  }
}

async function runTrial(host: RerailHost, strategy: SteerStrategy, trial: number, prompt: string, options: Options): Promise<Trial> {
  const result: Trial = { host, strategy, trial };
  const previousRerailHome = process.env.RERAIL_HOME;
  const sandbox = await realpath(await mkdtemp(`/tmp/rerail-steer-${host}-`));
  let daemon: RerailDaemon | undefined;
  try {
    const rerailHome = join(sandbox, "rerail");
    await mkdir(rerailHome, { recursive: true, mode: 0o700 });
    process.env.RERAIL_HOME = rerailHome;
    const project = join(sandbox, "workspace");
    await cp(join(FIXTURE, "project"), project, { recursive: true });
    const env = await sandboxEnvironment(sandbox, project, rerailHome);
    // Steer strategies are experimental: the daemon (this process) and Pi's in-process runtime only honour the seeded
    // strategy with the switch on.
    process.env.RERAIL_EXPERIMENTAL_STEER_STRATEGIES = "1";
    env.RERAIL_EXPERIMENTAL_STEER_STRATEGIES = "1";
    const profile = await sandboxProfile(sandbox, host);
    if (await sandboxLeaks(sandbox, profile, project, env)) throw new Error("Sandbox did not enforce write isolation");
    await configureHostAuth(host, sandbox, profile, env, options.timeoutMs);
    const databasePath = defaultRerailDatabasePath();
    seedRepository(databasePath, project, strategy);
    daemon = await RerailDaemon.start({
      version: "steer-strategy-benchmark",
      workspaceRoot: project,
      stateFile: join(rerailHome, "daemon.json"),
      databasePath,
      port: 0,
      kernelSocketPath: join(sandbox, "engine.sock"),
    });
    env.RERAIL_ENGINE_SOCKET = daemon.kernelSocketPath;
    const { command, args } = hostCommand(host, project, prompt);
    const started = Date.now();
    const output = await run("sandbox-exec", ["-f", profile, command, ...args.slice(0, -1), ...options.extra, prompt], {
      cwd: project,
      env,
      timeout: options.timeoutMs,
      jsonEvents: true,
      outputLimit: OUTPUT_LIMIT,
    });
    result.wallMs = Date.now() - started;
    result.exitCode = output.exitCode;
    const ledger = await readSteers(project);
    result.steers = ledger.steers;
    result.rewound = ledger.rewound;
    result.inResult = ledger.inResult;
    result.deliveredStrategies = ledger.strategies;
    const outcome = await inspectProject(project, profile, env);
    Object.assign(result, outcome, { taskCompleted: outcome.testsPassed && outcome.readmeUpdated });
    const sessionDir = SESSION_DIR[host];
    if (sessionDir === undefined) {
      Object.assign(result, streamMetrics(host, output.stdout));
    } else {
      const transcript = await mainTranscript(join(project, sessionDir), prompt);
      if (transcript === undefined) throw new Error(`no ${host} transcript under ${sessionDir}; harness exited ${output.exitCode}: ${bounded(output.stderr).slice(-800)}`);
      const { modelError, ...metrics } = transcriptMetrics(host, transcript.entries);
      Object.assign(result, metrics);
      if (modelError !== undefined) result.error = `model error: ${bounded(modelError)}`;
      result.compaction = host === "pi" ? attributePi(SessionManager.open(transcript.file).getBranch()) : attributeOmp(transcript.entries);
      if (options.compactionProbe && host === "pi")
        result.probe = await compactionProbe(transcript.file, result.compaction, project, profile, env, options.extra, Math.min(options.timeoutMs, 300_000));
    }
    // JSON-mode harnesses (Codex `exec --json`) report their failure as the last stdout events, not on stderr.
    if (output.exitCode !== 0) result.error ??= `harness exited ${output.exitCode}: ${bounded(output.stderr).slice(-800)} ${bounded(output.stdout).slice(-800)}`.trim();
    if (options.keep) result.kept = sandbox;
    return result;
  } catch (error) {
    result.error = bounded(error instanceof Error ? error.message : String(error));
    return result;
  } finally {
    try {
      await daemon?.close();
    } finally {
      if (previousRerailHome === undefined) delete process.env.RERAIL_HOME;
      else process.env.RERAIL_HOME = previousRerailHome;
      if (!options.keep) await rm(sandbox, { recursive: true, force: true });
    }
  }
}

function mean(values: readonly (number | boolean | null | undefined)[]): number | undefined {
  const numbers = values.flatMap((value) => (typeof value === "boolean" ? [Number(value)] : typeof value === "number" ? [value] : []));
  return numbers.length === 0 ? undefined : numbers.reduce((sum, value) => sum + value, 0) / numbers.length;
}
const cell = (value: number | undefined, format: (value: number) => string): string => (value === undefined ? "–" : format(value));
const percent = (value: number): string => `${Math.round(value * 100)}%`;
const fixed = (digits: number) => (value: number): string => value.toFixed(digits);
/** Mean per host × strategy over the trials that ran; `errors` counts trials that failed or whose harness exited non-zero. */
export function table(trials: readonly Trial[]): string {
  const rows = [
    "| host | strategy | trials | errors | task done | tests pass | README | steers | in result | rewound | violation left | subagent calls | wall s | input tok | output tok | cache read tok | cost $ | split-turn starts rule / note / prompt |",
    "|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|",
  ];
  const groups = Map.groupBy(trials, (trial) => `${trial.host} ${trial.strategy}`);
  for (const group of groups.values()) {
    const ran = group.filter((trial) => trial.wallMs !== undefined);
    const of = (pick: (trial: Trial) => number | boolean | null | undefined) => mean(ran.map(pick));
    const splits = ran.flatMap((trial) => trial.compaction?.splitCuts ?? []);
    const starts = (start: TurnStart): number => splits.filter((split) => split.start === start).length;
    rows.push(
      `| ${group[0]!.host} | ${group[0]!.strategy} | ${group.length} | ${group.filter((trial) => trial.error !== undefined).length} | ` +
        [
          cell(of((trial) => trial.taskCompleted), percent),
          cell(of((trial) => trial.testsPassed), percent),
          cell(of((trial) => trial.readmeUpdated), percent),
          cell(of((trial) => trial.steers), fixed(1)),
          cell(of((trial) => trial.inResult), fixed(1)),
          cell(of((trial) => trial.rewound), fixed(1)),
          cell(of((trial) => trial.violationRemaining), percent),
          cell(of((trial) => trial.subagentCalls), fixed(1)),
          cell(of((trial) => (trial.wallMs ?? 0) / 1000), fixed(0)),
          cell(of((trial) => trial.usage?.input), fixed(0)),
          cell(of((trial) => trial.usage?.output), fixed(0)),
          cell(of((trial) => trial.usage?.cacheRead), fixed(0)),
          cell(of((trial) => trial.usage?.cost), fixed(3)),
          splits.length === 0 ? "–" : `${starts("rule")} / ${starts("note")} / ${starts("prompt")}${starts("other") === 0 ? "" : ` (+${starts("other")} other)`}`,
        ].join(" | ") +
        " |",
    );
  }
  return rows.join("\n");
}

async function packageVersion(name: string): Promise<string> {
  const manifest = z.object({ version: z.string() }).safeParse(JSON.parse(await readFile(join(ROOT, "node_modules", name, "package.json"), "utf8").catch(() => "{}")));
  return manifest.data?.version ?? "?";
}

async function main(): Promise<void> {
  const options = parseArgs(process.argv.slice(2));
  const say = (line: string): void => void process.stdout.write(`${line}\n`);
  const hosts: RerailHost[] = [];
  const versions: Record<string, string> = { node: process.version };
  for (const host of options.hosts) {
    const cliVersion = await version(hostCli(host));
    if (cliVersion === undefined) say(`skip ${host}: \`${hostCli(host)}\` is not installed or fails \`--version\``);
    else {
      hosts.push(host);
      versions[host] = cliVersion;
    }
  }
  if (hosts.length === 0) {
    say("no selected harness is installed; nothing to measure");
    return;
  }
  if (process.platform !== "darwin" || !(await sandboxAvailable()))
    throw new Error("sandbox-exec is required; refusing an unsandboxed live provider run");
  for (const host of hosts) await ensureFresh(BUILT[host]);
  versions["compaction:pi"] = `@earendil-works/pi-coding-agent ${await packageVersion("@earendil-works/pi-coding-agent")}`;
  versions["compaction:omp"] = `@oh-my-pi/pi-agent-core ${await packageVersion("@oh-my-pi/pi-agent-core")} (reimplemented)`;
  say(`versions: ${Object.entries(versions).map(([name, value]) => `${name} ${value}`).join("; ")}`);
  const prompt = await readFile(join(FIXTURE, "prompt.md"), "utf8");
  const trials: Trial[] = [];
  for (const host of hosts) {
    hostLoop: for (const strategy of options.strategies) {
      for (let trial = 1; trial <= options.trials; trial += 1) {
        const result = await runTrial(host, strategy, trial, prompt, options);
        trials.push(result);
        if (result.error?.startsWith("FAILED prerequisite") === true) {
          say(`skip ${host}: ${result.error}`);
          break hostLoop;
        }
        const outcome = result.error === undefined ? `done=${result.taskCompleted} steers=${result.steers} inResult=${result.inResult} rewound=${result.rewound}` : `error: ${result.error.split("\n")[0]}`;
        say(`${host} ${strategy} #${trial}: ${outcome}${result.kept === undefined ? "" : ` (kept ${result.kept})`}`);
      }
    }
  }
  say(`\n${table(trials)}`);
  if (options.json !== undefined) {
    await writeFile(options.json, `${JSON.stringify({ versions, options: { ...options, json: undefined }, trials }, null, 2)}\n`);
    say(`raw results: ${options.json}`);
  }
}

if (process.argv[1] !== undefined && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) await main();
