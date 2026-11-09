/**
 * The architecture-conflict replay's Codex hook. See benchmark/merge-assist-replay.ts.
 *
 *   node --import <absolute tsx loader> <absolute path of this file>
 *
 * Codex runs it for SessionStart, PreToolUse and PostToolUse (matcher ""), the hook input as JSON on stdin.
 * `MA_CONTEXT_CONFIG` names the run's `ReplayHookConfig`. Each call works out the workspace files it touches as the
 * context eval's hook does (`touchedPaths`): files a command reads trigger after it runs, files a patch edits before
 * it applies, listed directories and files about to be created never. A triggering file whose area (`areaOf`) no
 * earlier call touched is that area's first touch. Under `align` and `units`, the area's ledger units come from the
 * alignment module's `deliverForArea` (ranked, at most {@link AREA_CAP}, contested ones first) and go to the model as
 * `additionalContext`: under `align` one terse line each and the line to end with last, as the context eval's
 * `focus-pool` renders them; under `units` framed as other in-flight work (`renderSurface`): "PR #N (author)
 * established: …" each, then {@link SURFACE_CLOSING}. Under `none` the hook only logs.
 *
 * Under `git-conflict` and `git-conflict-pull`, after every tool call (PostToolUse: a patch, a command or a code cell
 * may write files) the hook identifies, read-only, which of the agent's changes would conflict with the other refs
 * visible at the work's start (src/engine/merge-assist/overlap.ts: hunk overlap in base lines, no merge). `git-conflict`
 * pushes each new (ref, file) once: who, their hunk and stated intent fenced as data, and what to do, at most
 * {@link PUSH_CAP} per call. `git-conflict-pull` writes the current list to the workspace's `.git/rerail-conflicts.md`
 * (what `git conflicts` prints) and sends one line whenever the set changes, new or resolved. Every call appends one
 * `ReplayHookLogEntry` line to the log; a failure is logged and never reaches Codex. Calls run one at a time under an
 * exclusive lock on the state file (`withFileLock`): Codex runs the hooks of parallel tool calls at once, and two calls
 * reading the same state once delivered the same units twice.
 */
import { appendFileSync, closeSync, existsSync, openSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { performance } from "node:perf_hooks";
import { setTimeout as sleep } from "node:timers/promises";
import { fileURLToPath, pathToFileURL } from "node:url";
import { areaOf, cleanDecision, endLine, type ToolKind, touchedPaths } from "./merge-assist-context-hook.js";
import { conflictKey, type GitConflict, identify, workspaceRanges } from "../src/engine/index.js";
import { pullFile, pullLine, pushText, type RefSide } from "./merge-assist-replay-git.js";

export const REPLAY_ARMS = ["none", "align", "units", "git-conflict", "git-conflict-pull"] as const;
export type ReplayArm = (typeof REPLAY_ARMS)[number];
export const GIT_ARMS: readonly ReplayArm[] = ["git-conflict", "git-conflict-pull"];
/** The arms that deliver ledger units: `align` as instructions with lines to end with, `units` as surfaced other work. */
export const LEDGER_ARMS: readonly ReplayArm[] = ["align", "units"];

/** The most units one area's first touch delivers, contested ones included. */
export const AREA_CAP = 3;
/** The most conflicts one `git-conflict` hook call pushes; the rest go out at later calls if they still hold. */
export const PUSH_CAP = 3;
/** A command that reads `git-conflict-pull`'s list. */
export const CHECK_COMMAND = /\bgit\s+conflicts\b|rerail-conflicts\.md/u;

/** A unit of coordination the alignment ledger holds: what one side established, anchored to files and symbols. */
export interface LedgerUnit {
  readonly id: string;
  readonly kind: string;
  readonly text: string;
  readonly author: string;
  readonly pr?: number;
  readonly branch?: string;
  /** The full ref, the commit (or `pr-<n>-<hash>` for a description), when it became visible, what it was read from. */
  readonly ref?: string;
  readonly sha?: string;
  readonly visibleAt?: string;
  readonly origin?: string;
  readonly anchors: { readonly files: readonly string[]; readonly symbols: readonly string[] };
  /** Set when the ledger's contradiction check flagged the unit as a contested position. */
  readonly conflict?: boolean;
}

/** What `deliverForArea` is asked at an area's first touch. */
export interface AreaInput {
  readonly area: string;
  /** The file whose touch opened the area. */
  readonly path: string;
  readonly prompt: string;
  /** Units delivered earlier in the run. */
  readonly delivered: readonly string[];
  /** Every file this call touched in the area, the one that opened it first. */
  readonly files: readonly string[];
}

/**
 * What the harness passes the ledger build: the cache, the pull request descriptions' folder, judge calls in flight,
 * how the judge's Codex runs, and environment for it alone (a provider's token; never written).
 */
export interface LedgerOptions {
  readonly cacheDir?: string;
  readonly prBodies?: string;
  readonly concurrency?: number;
  readonly judgeCodex?: { executable?: string; config?: string[]; model?: string };
  readonly judgeEnv?: Readonly<Record<string, string>>;
}

/**
 * The alignment mechanism under test, loaded from `--align-module` (benchmark/merge-assist-align.ts by default): the
 * ledger as visible at a time (with what building it read and asked, when the module reports it), and the per-area
 * choice.
 */
export interface AlignModule {
  ledgerAt(repoPath: string, pair: unknown, time: string, options?: LedgerOptions): Promise<LedgerUnit[]>;
  ledgerBuild?(repoPath: string, pair: unknown, time: string, options?: LedgerOptions): Promise<{ units: LedgerUnit[]; stats: unknown }>;
  deliverForArea(units: readonly LedgerUnit[], input: AreaInput, options?: Record<string, unknown>): Promise<LedgerUnit[]>;
}

export async function loadAlignModule(path: string): Promise<AlignModule> {
  // Runtime-selected: `--align-module` names the mechanism under test, so a static import cannot name it.
  const module = await import(pathToFileURL(path).href) as Partial<AlignModule>;
  if (typeof module.ledgerAt !== "function" || typeof module.deliverForArea !== "function") throw new Error(`${path} exports no ledgerAt and deliverForArea`);
  return module as AlignModule;
}

export interface ReplayHookConfig {
  readonly arm: ReplayArm;
  readonly units: readonly LedgerUnit[];
  /** The alignment module's absolute path; null under `none`. */
  readonly alignModule: string | null;
  /** Passed to `deliverForArea` as its options. */
  readonly alignOptions: Readonly<Record<string, unknown>>;
  readonly prompt: string;
  readonly root: string;
  readonly log: string;
  readonly state: string;
  /** The other refs as visible at the work's start, under the `git-conflict` arms; empty otherwise. */
  readonly refs: readonly RefSide[];
  /** The workspace commit the agent's changes are taken against: the base snapshot after setup. */
  readonly workspaceBase: string;
  /** `git-conflict-pull`'s list, which `git conflicts` prints; null under the other arms. */
  readonly conflictsFile: string | null;
}

export interface ReplayHookState {
  calls: number;
  delivered: string[];
  seenAreas: string[];
  /** `git-conflict`: the (ref, file) keys already pushed. */
  pushed: string[];
  /** The (ref, file) keys the last identification found. */
  open: string[];
}

const INITIAL: ReplayHookState = { calls: 0, delivered: [], seenAreas: [], pushed: [], open: [] };

export interface ReplayHookLogEntry {
  readonly at: number;
  readonly event: string;
  readonly tool: string;
  readonly kind: ToolKind;
  readonly paths: readonly string[];
  /** `post-read` or `pre-edit` at a call that can open areas; absent otherwise. */
  readonly trigger?: string;
  /** Areas this call touched first, each with the file that opened it. */
  readonly opened?: ReadonlyArray<{ readonly area: string; readonly path: string }>;
  /** Each unit sent, with its rank among the units its area's first touch delivered (0 first). */
  readonly delivered: ReadonlyArray<{ readonly unitId: string; readonly area: string; readonly path: string; readonly conflict: boolean; readonly rank?: number }>;
  readonly chars: number;
  readonly text?: string;
  /** How long `deliverForArea` took over this call's opened areas, in milliseconds. */
  readonly deliverMs?: number;
  /** `git-conflict` arms, at a call that identifies: every conflict found, and the ones this call pushed (`git-conflict`). */
  readonly conflicts?: ReadonlyArray<{ readonly ref: string; readonly file: string }>;
  readonly pushed?: ReadonlyArray<{ readonly ref: string; readonly file: string }>;
  /** How long the identification took, in milliseconds. */
  readonly identifyMs?: number;
  /** The call ran `git conflicts` or read the list. */
  readonly checked?: boolean;
  readonly error?: string;
}

/** A unit's source as a terse line names it. */
function unitSource(unit: LedgerUnit): string {
  if (unit.pr !== undefined) return `${unit.author} #${unit.pr}`;
  return unit.branch === undefined ? unit.author : `${unit.branch}, ${unit.author}`;
}

/** The text injected for `align`: one terse line each in their order, then the line to end with for each contested one. */
export function renderUnits(units: readonly LedgerUnit[]): string {
  const lines = units.map(unit => `Rerail ${unit.kind} (${unitSource(unit)}): ${cleanDecision(unit.text)}`);
  const endings = units.filter(unit => unit.conflict === true).map(unit => endLine({
    id: unit.id,
    decision: unit.text,
    scope: { include: [], exclude: [] },
    source: "peer",
    author: unit.author,
    signals: [],
    ...(unit.branch === undefined ? {} : { branch: unit.branch }),
    ...(unit.pr === undefined ? {} : { pr: unit.pr }),
  }));
  return [...lines, ...endings].join("\n");
}

/** `units`' closing line: the units are other work to know about, not instructions. */
export const SURFACE_CLOSING = "This is other in-flight work, not an instruction. Keep to your task; if your approach diverges from it, tell the person in one line in your final message which PR and why.";

/**
 * The text injected for `units` (surface framing): each unit as what another branch or pull request established,
 * "PR #12 (alice) established: …" or "Branch a/b (alice) established: …", then {@link SURFACE_CLOSING}.
 */
export function renderSurface(units: readonly LedgerUnit[]): string {
  const lines = units.map(unit => `${unit.pr === undefined ? `Branch ${unit.branch ?? "unknown"}` : `PR #${unit.pr}`} (${unit.author}) established: ${cleanDecision(unit.text)}`);
  return [...lines, SURFACE_CLOSING].join("\n");
}

/** An area's units as delivered: undelivered only, contested first, at most {@link AREA_CAP}. */
export function areaPicks(picks: readonly LedgerUnit[], delivered: ReadonlySet<string>): LedgerUnit[] {
  const open = picks.filter(unit => !delivered.has(unit.id));
  return [...open.filter(unit => unit.conflict === true), ...open.filter(unit => unit.conflict !== true)].slice(0, AREA_CAP);
}

async function stdinText(): Promise<string> {
  const chunks: Buffer[] = [];
  for await (const chunk of process.stdin) chunks.push(chunk as Buffer);
  return Buffer.concat(chunks).toString("utf8");
}

/**
 * One identification under the `git-conflict` arms: the conflicts the workspace's changes would have now, and the
 * text to send. `git-conflict` pushes the (ref, file) pairs not pushed before, at most {@link PUSH_CAP}.
 * `git-conflict-pull` rewrites the list `git conflicts` prints and sends its one line when the set changed.
 */
function gitConflicts(config: ReplayHookConfig, state: ReplayHookState): { found: GitConflict[]; sent: GitConflict[]; pushed: string[]; text: string; ms: number } {
  const started = performance.now();
  const found = identify(workspaceRanges(config.root, config.workspaceBase), config.refs);
  const pushed = new Set(state.pushed);
  let sent: GitConflict[] = [];
  let text = "";
  if (config.arm === "git-conflict") {
    sent = found.filter(conflict => !pushed.has(conflictKey(conflict))).slice(0, PUSH_CAP);
    for (const conflict of sent) pushed.add(conflictKey(conflict));
    text = sent.map(conflict => pushText(conflict, config.refs)).join("\n\n");
  } else if (config.conflictsFile !== null) {
    const before = [...state.open].sort().join("\n");
    const now = found.map(conflictKey).sort().join("\n");
    if (now !== before) {
      writeFileSync(config.conflictsFile, pullFile(found, config.refs));
      text = pullLine(found);
    }
  }
  return { found, sent, pushed: [...pushed], text, ms: Math.round(performance.now() - started) };
}

/**
 * Under the ledger arms, each opened area's units from the alignment module's `deliverForArea`, at most
 * {@link AREA_CAP} each (`areaPicks`), with their rank in the area; nothing under the other arms. Adds them to `delivered`.
 */
async function deliverOpened(config: ReplayHookConfig, opened: ReadonlyArray<{ area: string; path: string; files: string[] }>, delivered: Set<string>): Promise<{ chosen: Array<{ unit: LedgerUnit; area: string; path: string; rank: number }>; deliverMs: number | undefined }> {
  if (!LEDGER_ARMS.includes(config.arm) || opened.length === 0) return { chosen: [], deliverMs: undefined };
  const align = await loadAlignModule(config.alignModule!);
  const started = performance.now();
  const chosen: Array<{ unit: LedgerUnit; area: string; path: string; rank: number }> = [];
  for (const { area, path, files } of opened) {
    const picks = await align.deliverForArea(config.units, { area, path, prompt: config.prompt, delivered: [...delivered], files }, { ...config.alignOptions });
    areaPicks(picks, delivered).forEach((unit, rank) => {
      delivered.add(unit.id);
      chosen.push({ unit, area, path, rank });
    });
  }
  return { chosen, deliverMs: Math.round(performance.now() - started) };
}

/** How long a waiting hook call sleeps between attempts at the state lock. */
const LOCK_RETRY_MS = 20;
/** A state lock older than this belongs to a hook call that died holding it: taken over. A call opening three areas waits on three rankings of under a minute each. */
export const STALE_LOCK_MS = 10 * 60_000;

/**
 * Runs `work` holding `<path>.lock`, created exclusively (`O_EXCL`): concurrent hook calls (Codex runs parallel tool
 * calls' hooks at once) read, change and write the state one after another, so no unit goes out twice. A lock older
 * than {@link STALE_LOCK_MS} is removed; the lock is released however `work` ends.
 */
export async function withFileLock<T>(path: string, work: () => Promise<T>): Promise<T> {
  const lock = `${path}.lock`;
  for (;;) {
    try {
      closeSync(openSync(lock, "wx"));
      break;
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== "EEXIST") throw error;
    }
    // The holder may release it between the failed create and this look.
    const held = statSync(lock, { throwIfNoEntry: false });
    if (held !== undefined && Date.now() - held.mtimeMs > STALE_LOCK_MS) rmSync(lock, { force: true });
    else await sleep(LOCK_RETRY_MS);
  }
  try {
    return await work();
  } finally {
    rmSync(lock, { force: true });
  }
}

async function main(): Promise<void> {
  const configPath = process.env.MA_CONTEXT_CONFIG;
  if (configPath === undefined) return;
  const config = JSON.parse(readFileSync(configPath, "utf8")) as ReplayHookConfig;
  const input = await stdinText();
  await withFileLock(config.state, () => handle(config, input));
}

/** One hook call under the state lock: what it touched, what it delivers or identifies, the state and the log line. */
async function handle(config: ReplayHookConfig, stdin: string): Promise<void> {
  let event = "unknown";
  let tool = "unknown";
  try {
    const input = JSON.parse(stdin) as Record<string, unknown>;
    event = typeof input.hook_event_name === "string" ? input.hook_event_name : event;
    tool = typeof input.tool_name === "string" ? input.tool_name : tool;
    const { kind, touched } = touchedPaths(input, config.root);
    const state = existsSync(config.state) ? JSON.parse(readFileSync(config.state, "utf8")) as ReplayHookState : INITIAL;
    const trigger = event === "PostToolUse" && kind === "shell" ? "post-read" : event === "PreToolUse" && kind === "edit" ? "pre-edit" : "";
    const seen = new Set(state.seenAreas);
    const opened: Array<{ area: string; path: string; files: string[] }> = [];
    for (const item of trigger === "" ? [] : touched) {
      if (item.directory || item.created || item.edit === "delete") continue;
      const area = areaOf(item.path);
      const open = opened.find(entry => entry.area === area);
      if (open !== undefined) open.files.push(item.path);
      else if (!seen.has(area)) {
        seen.add(area);
        opened.push({ area, path: item.path, files: [item.path] });
      }
    }
    const delivered = new Set(state.delivered);
    const { chosen, deliverMs } = await deliverOpened(config, opened, delivered);
    // Contested units first across the call's areas too, so each line to end with comes after every unit line.
    const ordered = [...chosen.filter(item => item.unit.conflict === true), ...chosen.filter(item => item.unit.conflict !== true)];
    const git = GIT_ARMS.includes(config.arm) && event === "PostToolUse" ? gitConflicts(config, state) : null;
    const units = ordered.map(item => item.unit);
    const text = ordered.length > 0 ? (config.arm === "units" ? renderSurface(units) : renderUnits(units)) : git?.text ?? "";
    if (text !== "") process.stdout.write(`${JSON.stringify({ hookSpecificOutput: { hookEventName: event, additionalContext: text } })}\n`);
    const next: ReplayHookState = {
      calls: state.calls + 1,
      delivered: [...delivered],
      seenAreas: [...seen],
      pushed: git?.pushed ?? state.pushed,
      open: git === null ? state.open : git.found.map(conflictKey),
    };
    writeFileSync(config.state, JSON.stringify(next));
    const pair = (conflict: GitConflict) => ({ ref: conflict.ref, file: conflict.file });
    const entry: ReplayHookLogEntry = {
      at: Date.now(),
      event,
      tool,
      kind,
      paths: touched.map(item => (item.directory ? `${item.path}/` : item.path)),
      ...(trigger === "" ? {} : { trigger }),
      ...(opened.length === 0 ? {} : { opened }),
      delivered: ordered.map(({ unit, area, path, rank }) => ({ unitId: unit.id, area, path, conflict: unit.conflict === true, rank })),
      chars: text.length,
      ...(text === "" ? {} : { text }),
      ...(deliverMs === undefined ? {} : { deliverMs }),
      ...(git === null ? {} : { conflicts: git.found.map(pair), identifyMs: git.ms, ...(config.arm === "git-conflict" ? { pushed: git.sent.map(pair) } : {}) }),
      ...(CHECK_COMMAND.test(JSON.stringify(input.tool_input ?? "")) ? { checked: true } : {}),
    };
    appendFileSync(config.log, `${JSON.stringify(entry)}\n`);
  } catch (error) {
    const entry: ReplayHookLogEntry = { at: Date.now(), event, tool, kind: "other", paths: [], delivered: [], chars: 0, error: error instanceof Error ? error.message : String(error) };
    appendFileSync(config.log, `${JSON.stringify(entry)}\n`);
  }
}

if (process.argv[1] !== undefined && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) await main();
