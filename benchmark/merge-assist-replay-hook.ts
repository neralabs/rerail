/**
 * The architecture-conflict replay's Codex hook. See benchmark/merge-assist-replay.ts.
 *
 *   node --import <absolute tsx loader> <absolute path of this file>
 *
 * Codex runs it for SessionStart, PreToolUse and PostToolUse (matcher ""), the hook input as JSON on stdin.
 * `MA_CONTEXT_CONFIG` names the run's `ReplayHookConfig`. Each call works out the workspace files it touches as the
 * context eval's hook does (`touchedPaths`): files a command reads trigger after it runs, files a patch edits before
 * it applies, listed directories and files about to be created never. A triggering file whose area (`areaOf`) no
 * earlier call touched is that area's first touch. Under `align`, the area's ledger units come from the alignment
 * module's `deliverForArea` (ranked, at most {@link AREA_CAP}, contested ones first) and go to the model as
 * `additionalContext`, one terse line each and the line to end with last, as the context eval's `focus-pool` renders
 * them. Under `none` the hook only logs. Every call appends one `ReplayHookLogEntry` line to the log; a failure is
 * logged and never reaches Codex.
 */
import { appendFileSync, existsSync, readFileSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { performance } from "node:perf_hooks";
import { fileURLToPath, pathToFileURL } from "node:url";
import { areaOf, cleanDecision, endLine, type ToolKind, touchedPaths } from "./merge-assist-context-hook.js";

export const REPLAY_ARMS = ["none", "align"] as const;
export type ReplayArm = (typeof REPLAY_ARMS)[number];

/** The most units one area's first touch delivers, contested ones included. */
export const AREA_CAP = 3;

/** A unit of coordination the alignment ledger holds: what one side established, anchored to files and symbols. */
export interface LedgerUnit {
  readonly id: string;
  readonly kind: string;
  readonly text: string;
  readonly author: string;
  readonly pr?: number;
  readonly branch?: string;
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
 * The alignment mechanism under test, loaded from `--align-module` (benchmark/merge-assist-align.ts by default): the
 * ledger as visible at a time, and the per-area choice.
 */
export interface AlignModule {
  ledgerAt(repoPath: string, pair: unknown, time: string, options?: { cacheDir?: string }): Promise<LedgerUnit[]>;
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
}

export interface ReplayHookState {
  calls: number;
  delivered: string[];
  seenAreas: string[];
}

const INITIAL: ReplayHookState = { calls: 0, delivered: [], seenAreas: [] };

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
  readonly delivered: ReadonlyArray<{ readonly unitId: string; readonly area: string; readonly path: string; readonly conflict: boolean }>;
  readonly chars: number;
  readonly text?: string;
  /** How long `deliverForArea` took over this call's opened areas, in milliseconds. */
  readonly deliverMs?: number;
  readonly error?: string;
}

/** A unit's source as a terse line names it. */
function unitSource(unit: LedgerUnit): string {
  if (unit.pr !== undefined) return `${unit.author} #${unit.pr}`;
  return unit.branch === undefined ? unit.author : `${unit.branch}, ${unit.author}`;
}

/** The text injected for `units`: one terse line each in their order, then the line to end with for each contested one. */
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

async function main(): Promise<void> {
  const configPath = process.env.MA_CONTEXT_CONFIG;
  if (configPath === undefined) return;
  const config = JSON.parse(readFileSync(configPath, "utf8")) as ReplayHookConfig;
  let event = "unknown";
  let tool = "unknown";
  try {
    const input = JSON.parse(await stdinText()) as Record<string, unknown>;
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
    const chosen: Array<{ unit: LedgerUnit; area: string; path: string }> = [];
    let deliverMs: number | undefined;
    if (config.arm === "align" && opened.length > 0) {
      const align = await loadAlignModule(config.alignModule!);
      const started = performance.now();
      for (const { area, path, files } of opened) {
        const picks = await align.deliverForArea(config.units, { area, path, prompt: config.prompt, delivered: [...delivered], files }, { ...config.alignOptions });
        for (const unit of areaPicks(picks, delivered)) {
          delivered.add(unit.id);
          chosen.push({ unit, area, path });
        }
      }
      deliverMs = Math.round(performance.now() - started);
    }
    // Contested units first across the call's areas too, so each line to end with comes after every unit line.
    const ordered = [...chosen.filter(item => item.unit.conflict === true), ...chosen.filter(item => item.unit.conflict !== true)];
    const text = ordered.length === 0 ? "" : renderUnits(ordered.map(item => item.unit));
    if (text !== "") process.stdout.write(`${JSON.stringify({ hookSpecificOutput: { hookEventName: event, additionalContext: text } })}\n`);
    const next: ReplayHookState = { calls: state.calls + 1, delivered: [...delivered], seenAreas: [...seen] };
    writeFileSync(config.state, JSON.stringify(next));
    const entry: ReplayHookLogEntry = {
      at: Date.now(),
      event,
      tool,
      kind,
      paths: touched.map(item => (item.directory ? `${item.path}/` : item.path)),
      ...(trigger === "" ? {} : { trigger }),
      ...(opened.length === 0 ? {} : { opened }),
      delivered: ordered.map(({ unit, area, path }) => ({ unitId: unit.id, area, path, conflict: unit.conflict === true })),
      chars: text.length,
      ...(text === "" ? {} : { text }),
      ...(deliverMs === undefined ? {} : { deliverMs }),
    };
    appendFileSync(config.log, `${JSON.stringify(entry)}\n`);
  } catch (error) {
    const entry: ReplayHookLogEntry = { at: Date.now(), event, tool, kind: "other", paths: [], delivered: [], chars: 0, error: error instanceof Error ? error.message : String(error) };
    appendFileSync(config.log, `${JSON.stringify(entry)}\n`);
  }
}

if (process.argv[1] !== undefined && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) await main();
