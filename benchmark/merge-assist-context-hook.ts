/**
 * The Merge Assist context eval's Codex hook: the injection protocol under test. See benchmark/merge-assist-context.ts.
 *
 *   node --import <absolute tsx loader> <absolute path of this file>
 *
 * Codex runs it for SessionStart, PreToolUse and PostToolUse (matcher ""), the hook input as JSON on stdin.
 * `MA_CONTEXT_CONFIG` names the run's configuration (`HookConfig`): the variant, the decision index built by
 * `--build-index` (with each decision's signals and embedding), the workspace, the log and the state file. The hook
 * works out which workspace paths the call touches, picks the decisions the variant injects for them, and prints
 * `{"hookSpecificOutput":{"hookEventName":…,"additionalContext":…}}`, which Codex adds to the model's context as a
 * developer message. Every call appends one `HookLogEntry` line to the log, injected or not. At every `apply_patch`,
 * whatever the variant (the control included), the entry holds each scope-matching decision's signal hits and
 * embedding similarity, so guard thresholds can be swept offline. `orient: focus` starts the session-focus ranker
 * (merge-assist-context-rank.ts) in the background at SessionStart; `orient: area-focus` starts one per area, at the
 * area's first touch, and with `sessionRankMax` the session's too. A failure is logged and never reaches Codex: the
 * hook then prints nothing.
 */
import { spawn } from "node:child_process";
import { appendFileSync, existsSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { isAbsolute, join, relative, resolve } from "node:path";
import { performance } from "node:perf_hooks";
import { setTimeout as sleep } from "node:timers/promises";
import { fileURLToPath } from "node:url";
import { cosineSimilarity, type GlobMatcher, globPattern, NativeSemanticEmbedder, signalHits } from "../src/engine/index.js";

// ---------------------------------------------------------------------------------------------------------- types

export const WHEN = ["upfront", "post-read", "pre-edit", "post-read+pre-edit"] as const;
export const DEDUPE = ["session", "none"] as const;
export const FORMATS = ["card", "terse"] as const;
export const RANKS = ["conflict-first", "scope-specificity", "relevance"] as const;
export const ONLY = ["all", "own+team+peer", "conflicts"] as const;
export const CONFLICT = ["tell", "final-line"] as const;
export const BUDGETS = ["session", "per-file"] as const;
export const CHANNELS = ["single", "hybrid"] as const;
/** What the guard fires on at a patch: a decision's signals, its embedding's similarity to the added lines, or either. */
export const GUARDS = ["signals", "embed", "both"] as const;
export type GuardSource = (typeof GUARDS)[number];

/**
 * What `hybrid` orientation offers: conflict pairs on a file's first touch; every decision of an area's first touch
 * (`area`); conflict pairs plus the session-focus ranker's picks that cover the area (`focus`); conflict pairs plus the
 * picks of a ranking call over the area's own decisions, started at its first touch (`area-focus`); conflict pairs
 * plus every decision covering the area, unranked and uncapped (`area-all`); or conflict pairs plus the task's target
 * decisions that cover the area (`targets`: a diagnostic ceiling that selects with the grader's labels, never a
 * product protocol).
 */
export const ORIENTS = ["conflicts", "area", "focus", "area-focus", "area-all", "targets"] as const;
/**
 * How area orientation ranks an area's decisions before the cap. `index`: by source, then index order. `specificity`:
 * the narrowest scope first. `prompt-lexical`: BM25 of the task prompt against the decision text. `prompt-embed`: the
 * cosine between the prompt's and the decision's embeddings. The prompt rankers break ties by breadth.
 */
export const AREA_RANKS = ["index", "specificity", "prompt-lexical", "prompt-embed"] as const;
/** One protocol: when it injects, how many decisions at once and per run, how often, how they read, in what order, and which. */
export interface VariantParameters {
  readonly when: (typeof WHEN)[number];
  /** Most decisions per injection, or per newly touched file under `per-file` (a conflict pair counts once); null for no limit. */
  readonly cap: number | null;
  readonly dedupe: (typeof DEDUPE)[number];
  readonly format: (typeof FORMATS)[number];
  readonly rank: (typeof RANKS)[number];
  readonly only: (typeof ONLY)[number];
  /** Whether a directory a command lists or searches (`ls src`, `rg x src`) triggers; when false only files read or edited do. */
  readonly dirs: boolean;
  /** Most decisions injected over the whole run, each member of a conflict pair counted; null for no limit. */
  readonly sessionCap: number | null;
  /** How a disagreement reads: `tell` asks the agent to tell the user; `final-line` hands it the exact line to end with. */
  readonly conflict: (typeof CONFLICT)[number];
  /** `session`: `cap` per injection, `sessionCap` per run. `per-file`: `cap` per newly touched file, each file once. */
  readonly budget: (typeof BUDGETS)[number];
  /** Least relevance score (0–1) a decision needs to be injected; null for none. A conflict pair always passes. */
  readonly threshold: number | null;
  /**
   * `single`: one selection, as the parameters above say. `hybrid`: orientation (see `orient`) plus guard (at a patch,
   * any undelivered decision in scope that `guard` fires on, `guardCap` per patch); `rank`, `threshold` and
   * `sessionCap` do not apply.
   */
  readonly channels: (typeof CHANNELS)[number];
  /**
   * `hybrid` orientation. `conflicts`: the conflict pairs on a file first read or patched, `cap` per file. `area`: on
   * the first touch of an area (`areaOf`), every undelivered decision whose scope covers the touched file or any
   * project file in that area, conflict pairs first, at most `cap` decisions per injection. `area-all` and `targets`:
   * on an area's first touch as `focus` sees it, its conflict pairs, then every undelivered decision covering it
   * (`area-all`, index order) or the task's targets among them (`targets`), no cap.
   */
  readonly orient: (typeof ORIENTS)[number] | null;
  /** `orient: area`: how an area's decisions are ranked before the cap. */
  readonly areaRank: (typeof AREA_RANKS)[number] | null;
  /** `hybrid`: what the guard fires on; null for no guard. */
  readonly guard: GuardSource | null;
  /** `hybrid` with `embed` or `both`: the least similarity (cosine, 0–1) that fires; frozen from `--sweep` on the dev split. */
  readonly embedThreshold: number | null;
  /** `hybrid`: the most decisions the guard injects per patch. */
  readonly guardCap: number | null;
  /** `orient: focus`: how many of the ranker's picks (most relevant first) are in play; null for all it names. */
  readonly focusK: number | null;
  /** `orient: focus` or `area-focus`: the most ids one ranking call names; null for the product's default (`MAX_RANKED`). */
  readonly rankMax: number | null;
  /**
   * `orient: focus` or `area-focus`: the most picks an area's first touch injects, its conflict pairs aside, and the
   * most a fallback ranks for it; null for no per-area limit (a fallback then takes `focusK`).
   */
  readonly areaCap: number | null;
  /**
   * `orient: area-focus`: the session ranker runs too, from the session's start, answered as at most this many ids,
   * and each area's ranker lists the session's picks first, in their order, waiting up to {@link SESSION_WAIT_MS} for
   * them; null for no session ranker.
   */
  readonly sessionRankMax: number | null;
  /**
   * `orient: area-focus` with `sessionRankMax`: an area ranker's picks after its first `agreeAfter` are injected only
   * when the session ranker picked them too, so the two rankers agree on them; null for every pick.
   */
  readonly agreeAfter: number | null;
}

export type DecisionSource = "team" | "peer" | "own";

/** A decision as the index holds it: a team decision carries its pull request, a branch decision its branch. */
export interface IndexEntry {
  readonly id: string;
  readonly decision: string;
  readonly scope: { readonly include: readonly string[]; readonly exclude: readonly string[] };
  readonly source: DecisionSource;
  readonly author: string;
  readonly branch?: string;
  readonly pr?: number;
  /** The decision on the other side of a disagreement: a peer's points at this branch's own, and back. */
  readonly conflictsWith?: string;
  /** Literal code fragments whose presence in added lines puts the decision at stake: the product's validated signals. */
  readonly signals: readonly string[];
}

export interface HookConfig {
  /** null for the control: the hook only logs. */
  readonly variant: VariantParameters | null;
  readonly index: readonly IndexEntry[];
  /** The task as the person asked it: part of every call's context for `relevance`. */
  readonly prompt: string;
  /** Each index entry's embedding of its decision text, by id (`NativeSemanticEmbedder`, `EMBED_MAX_TOKENS`). */
  readonly embeddings: Readonly<Record<string, readonly number[]>>;
  /** The embedding model's folder: the bundled model. */
  readonly modelPath: string;
  /** The project's files when the run starts: what an area holds (`orient: area`). */
  readonly files: readonly string[];
  /** `orient: targets` only: the task's target decision ids, the grader's labels (a diagnostic ceiling). */
  readonly targets?: readonly string[];
  readonly root: string;
  /** The run's folder: where the session-focus ranker writes `focus.json` and keeps its judge database. */
  readonly folder: string;
  readonly log: string;
  readonly state: string;
}

/** One scope-matching item (a decision, or a peer decision and the own one it contradicts) on one patched path. */
export interface GuardScore {
  readonly ids: readonly string[];
  readonly path: string;
  /** The members' signals the path's added lines contain. */
  readonly hits: readonly string[];
  /** The best cosine between a member's embedding and one of the path's added-line blocks; null without added lines. */
  readonly similarity: number | null;
}

export interface HookLogEntry {
  readonly at: number;
  readonly event: string;
  readonly tool: string;
  /** `shell` for a command, `edit` for an apply_patch text, `other` for anything else. */
  readonly kind: ToolKind;
  readonly paths: readonly string[];
  readonly injected: ReadonlyArray<{ readonly decisionId: string; readonly reason: string; readonly channel?: Channel; readonly area?: string }>;
  readonly chars: number;
  /** The injected text, so the harness can find it again in Codex's transcript. */
  readonly text?: string;
  /** At a call the variant triggers on: which trigger, the paths that could trigger, and every scope match, scored. */
  readonly trigger?: string;
  readonly triggerPaths?: readonly string[];
  readonly candidates?: readonly Candidate[];
  /** At every `apply_patch`, the control included: each scope-matching item's signal hits and similarity per path. */
  readonly guard?: readonly GuardScore[];
  /** At every `apply_patch`: the lines it adds, per path, so an oracle can label the patch offline (`--sweep`). */
  readonly added?: Readonly<Record<string, string>>;
  /** At a patch the hook embedded: the model load and the blocks' embedding, in milliseconds, and the block count. */
  readonly embedding?: { readonly loadMs: number; readonly embedMs: number; readonly blocks: number };
  /** At the call that embedded the task prompt (`prompt-embed`): the model load and the prompt's embedding, in milliseconds. */
  readonly promptEmbedding?: { readonly loadMs: number; readonly embedMs: number };
  /** `orient: focus`: the ranker's state at this call and what the call did with it. */
  readonly focus?: FocusLog;
  /** `orient: area-all` or `targets`: decisions chosen at or before this call that wait for the next one, to keep this one under {@link HOOK_CONTEXT_CHARS}. */
  readonly deferred?: readonly string[];
  readonly error?: string;
}

type ToolKind = "shell" | "edit" | "other";
export type Channel = "orientation" | "guard";

export interface HookState {
  calls: number;
  delivered: string[];
  /** Decisions injected so far, repeats included (`sessionCap`). */
  injected: number;
  /** Paths that already had their turn under `per-file`. */
  seenFiles: string[];
  /** Areas (`areaOf`) a triggering file was already in. */
  seenAreas: string[];
  /** `prompt-embed`: the task prompt's embedding, computed once per run. */
  promptVector: number[] | null;
  /** `orient: focus`, or `area-focus` with `sessionRankMax`: whether the session ranker was started. */
  focusStarted: boolean;
  /** `orient: focus` or `area-focus`: files whose area got its conflict pairs before its ranker answered; its picks follow when it does. */
  pendingPaths: string[];
  /** `orient: area-all` or `targets`: chosen items held back to keep a call under {@link HOOK_CONTEXT_CHARS}; the next call sends them first. */
  queued: Candidate[];
}

export const INITIAL_STATE: HookState = { calls: 0, delivered: [], injected: 0, seenFiles: [], seenAreas: [], promptVector: null, focusStarted: false, pendingPaths: [], queued: [] };

/** What a focus ranker writes when it is done: the session's to `focus.json`, an area's to `focus-<area>.json` (`areaFocusPath`). */
export interface FocusFile {
  readonly ok: boolean;
  /** Index entry ids, most relevant first. */
  readonly ids: readonly string[];
  readonly ms: number;
  readonly error: string | null;
  readonly runner: string | null;
  /** The ranking prompt's length: judge tokens are estimated as a quarter of it. */
  readonly promptChars: number;
  readonly candidates: number;
  /** An area ranker under `sessionRankMax`: the session's picks it listed first, in order (empty when the session ranker failed or was late). */
  readonly session?: readonly string[];
}

export interface FocusLog {
  /** `focus`: whether `focus.json` was there at this call (after any wait). `area-focus`: whether no area this call served or still waits for lacked its answer. */
  readonly ready: boolean;
  /** `focus`: the session ranker's answer; null before it and under `area-focus`. */
  readonly ok: boolean | null;
  readonly ids: readonly string[] | null;
  readonly rankerMs: number | null;
  readonly promptChars: number | null;
  /** At a pre-edit whose area the ranker had not answered for: how long the hook waited. */
  readonly waitedMs: number | null;
  /**
   * `timeout`: the wait ran out, so the area was ranked by prompt embedding; `failed`: the ranker failed, the same for
   * the run (`focus`) or for that area (`area-focus`).
   */
  readonly fallback: "timeout" | "failed" | null;
  /** Areas touched for the first time at this call, areas whose picks went out, and files still waiting for picks. */
  readonly fresh: readonly string[];
  readonly served: readonly string[];
  readonly pending: readonly string[];
  /** `area-focus`: each area served at this call, how (its ranker's answer, or a fallback), and its ranker's answer if any. */
  readonly areas?: readonly AreaFocusLog[];
}

export interface AreaFocusLog {
  readonly area: string;
  readonly via: "ranker" | "timeout" | "failed";
  readonly ids: readonly string[] | null;
  readonly rankerMs: number | null;
  readonly promptChars: number | null;
}

/** A file's area: its folder, cut to the first two segments (`src/ui` for `src/ui/forms/X.tsx`, `src` for `src/config.ts`, `` at the root). */
export function areaOf(path: string): string {
  return path.split("/").slice(0, -1).slice(0, 2).join("/");
}

/** How much of a command's output counts as context for `relevance`. */
const RESPONSE_CHARS = 8_192;

const DECISION_CHARS = 280;
/** Extensions that make a token a path even when no such file exists yet: a file the command is about to create. */
const PATH_EXTENSIONS = /\.(?:ts|tsx|js|jsx|mjs|cjs|json|md|sql|css|html|ya?ml|toml)$/u;

// ---------------------------------------------------------------------------------------------------------- scopes

/** A decision's scope, compiled: whether a file, or anything under a directory, falls in it, and how narrowly. */
export interface ScopeMatcher {
  /** The scope's narrowest literal prefix that covers `path`, as its length; -1 when the scope does not cover it. */
  specificity(path: string, directory: boolean): number;
}

/** The part of a glob before its first wildcard: `src/api/` for `src/api/**`. */
function literalPrefix(glob: string): string {
  const wildcard = glob.search(/[*?[{]/u);
  return wildcard === -1 ? glob : glob.slice(0, wildcard);
}

export function scopeMatcher(scope: IndexEntry["scope"]): ScopeMatcher {
  const includes = scope.include.map(glob => ({ glob, prefix: literalPrefix(glob), matcher: globPattern([glob]) }));
  const exclude: GlobMatcher | undefined = scope.exclude.length === 0 ? undefined : globPattern(scope.exclude);
  const covers = (file: string): number => {
    if (exclude?.test(file) === true) return -1;
    return Math.max(-1, ...includes.filter(include => include.matcher.test(file)).map(include => include.prefix.length));
  };
  return {
    specificity(path, directory) {
      if (!directory) return covers(path);
      // A directory touches every decision whose scope could match a file under it: a probe file directly inside it,
      // or a scope rooted deeper inside it (`ls src` touches `src/ui/**`).
      const probe = covers(`${path}/__probe__.ts`);
      const deeper = includes.filter(include => include.prefix.startsWith(`${path}/`)).map(include => include.prefix.length);
      return Math.max(probe, ...deeper);
    },
  };
}

// ---------------------------------------------------------------------------------------------------------- touched paths

function strings(value: unknown, into: string[]): string[] {
  if (typeof value === "string") into.push(value);
  else if (Array.isArray(value)) for (const item of value) strings(item, into);
  else if (value !== null && typeof value === "object") for (const item of Object.values(value)) strings(item, into);
  return into;
}

interface Touched {
  readonly path: string;
  readonly directory: boolean;
  /** A path a command names that does not exist yet: a file it is about to create, not one it reads. */
  readonly created: boolean;
  /** What a patch does to the path; null for a path a command names. */
  readonly edit: "add" | "update" | "delete" | "move" | null;
  /** The lines a patch adds to the path (`+` lines, without the `+`); empty otherwise. */
  readonly added: string;
}

/** A workspace-relative path for `token` read from `cwd`, or undefined outside the workspace (and for the root itself). */
function inWorkspace(root: string, cwd: string, token: string): string | undefined {
  const absolute = isAbsolute(token) ? token : resolve(cwd, token);
  const path = relative(root, absolute);
  if (path === "" || path.startsWith("..") || isAbsolute(path)) return undefined;
  if (path === ".git" || path.startsWith(".git/")) return undefined;
  return path.split("\\").join("/");
}

const PATCH_EDITS = { "Add File": "add", "Update File": "update", "Delete File": "delete", "Move to": "move" } as const;

/** The files an `apply_patch` text changes, each with what it does and the lines it adds. */
function patchSections(patch: string): Array<{ path: string; edit: Touched["edit"]; added: string[] }> {
  const sections: Array<{ path: string; edit: Touched["edit"]; added: string[] }> = [];
  for (const line of patch.split("\n")) {
    const header = /^\*\*\* (Add File|Update File|Delete File|Move to): (.+)$/u.exec(line.trimEnd());
    if (header !== null) sections.push({ path: header[2]!.trim(), edit: PATCH_EDITS[header[1] as keyof typeof PATCH_EDITS], added: [] });
    else if (line.startsWith("+") && sections.length > 0) sections.at(-1)!.added.push(line.slice(1));
  }
  return sections;
}

/** Tokens of a shell command that name a workspace file or directory, or a new file with a known extension. */
function commandPaths(command: string, root: string, cwd: string): Touched[] {
  const touched: Touched[] = [];
  for (const raw of command.split(/[\s'"`|;&<>(){}=,]+/u)) {
    const token = raw.replace(/^\.\//u, "").replace(/:\d+(?::\d+)?$/u, "");
    if (token === "" || token.startsWith("-") || /[*?[\]$\\]/u.test(token)) continue;
    const path = inWorkspace(root, cwd, token);
    if (path === undefined) continue;
    const absolute = resolve(root, path);
    if (existsSync(absolute)) touched.push({ path, directory: statSync(absolute).isDirectory(), created: false, edit: null, added: "" });
    else if (PATH_EXTENSIONS.test(token)) touched.push({ path, directory: false, created: true, edit: null, added: "" });
  }
  return touched;
}

/** What a hook call's tool is, which workspace paths it touches, and what it shows the agent (for `relevance`). */
function touchedPaths(input: Record<string, unknown>, root: string): { kind: ToolKind; touched: Touched[]; context: string } {
  const toolInput = input.tool_input;
  const cwdField = [input.cwd, (toolInput as Record<string, unknown> | null)?.workdir].find(value => typeof value === "string") as string | undefined;
  const cwd = cwdField === undefined ? root : resolve(root, cwdField);
  const texts = strings(toolInput, []);
  const patch = texts.find(text => /^\*\*\* (?:Begin Patch|Add File:|Update File:|Delete File:)/mu.test(text));
  const unique = (list: Touched[]): Touched[] => [...new Map(list.map(entry => [entry.path, entry])).values()];
  if (patch !== undefined) {
    const paths = patchSections(patch).flatMap(({ path, edit, added }) => {
      const inside = inWorkspace(root, cwd, path);
      return inside === undefined ? [] : [{ path: inside, directory: false, created: false, edit, added: added.join("\n") }];
    });
    return { kind: "edit", touched: unique(paths), context: patch };
  }
  const command = (toolInput as Record<string, unknown> | null)?.command;
  if (command === undefined) return { kind: "other", touched: [], context: "" };
  const commands = strings(command, []);
  const response = strings(input.tool_response, []).join("\n").slice(0, RESPONSE_CHARS);
  return { kind: "shell", touched: unique(commands.flatMap(text => commandPaths(text, root, cwd))), context: `${commands.join("\n")}\n${response}` };
}

// ---------------------------------------------------------------------------------------------------------- relevance

const STOPWORDS: Readonly<Record<string, true>> = Object.fromEntries([
  "the", "an", "and", "or", "of", "to", "in", "on", "for", "with", "is", "are", "be", "by", "it", "its", "that", "this",
  "as", "at", "not", "no", "never", "every", "each", "only", "all", "any", "so", "than", "then", "when", "where", "which",
  "who", "what", "do", "does", "don", "their", "they", "them", "one", "own", "like", "also", "but", "if", "else", "use",
  "uses", "used", "using", "through", "into", "out", "over", "from", "here", "there", "has", "have", "can", "may",
  "should", "must", "we", "our", "you", "your", "get", "gets", "new", "const", "let", "var", "return", "function",
  "async", "await", "import", "type", "interface", "true", "false", "null", "undefined", "string", "number", "boolean",
  "void", "promise",
].map(word => [word, true]));

/** Lower-case tokens of `text`: each identifier whole and split at camelCase and `_`; no stopwords. */
export function relevanceTokens(text: string): Set<string> {
  const found = new Set<string>();
  const keep = (token: string) => {
    if (token.length >= 2 && STOPWORDS[token] !== true && !/^\d+$/u.test(token)) found.add(token);
  };
  for (const word of text.match(/[A-Za-z_$][\w$]*/gu) ?? []) {
    keep(word.toLowerCase());
    for (const part of word.replace(/([a-z0-9])([A-Z])/gu, "$1 $2").replace(/([A-Z]+)([A-Z][a-z])/gu, "$1 $2").split(/[\s_$]+/u)) keep(part.toLowerCase());
  }
  return found;
}

/** Each decision's tokens weighed by IDF over the run's index: a token every decision has weighs 0. */
function decisionWeights(index: readonly IndexEntry[]): Map<string, Map<string, number>> {
  const tokens = new Map(index.map(entry => [entry.id, relevanceTokens(entry.decision)]));
  const documents = new Map<string, number>();
  for (const set of tokens.values()) for (const token of set) documents.set(token, (documents.get(token) ?? 0) + 1);
  return new Map([...tokens].map(([id, set]) => [id, new Map([...set].map(token => [token, Math.log((index.length + 1) / (documents.get(token)! + 1))]))]));
}

/** The share of a decision's token weight the context holds, in [0, 1]. */
function relevance(weights: ReadonlyMap<string, number>, context: ReadonlySet<string>): number {
  let total = 0;
  let overlap = 0;
  for (const [token, weight] of weights) {
    total += weight;
    if (context.has(token)) overlap += weight;
  }
  return total === 0 ? 0 : overlap / total;
}

/**
 * BM25 (k1 1.2, b 0.75) of `query` against each document, IDF over all of them. Terms are `relevanceTokens`, counted
 * once per document: decisions are one or two sentences, so a repeated term says little.
 */
export function bm25(query: string, documents: ReadonlyMap<string, string>): Map<string, number> {
  const terms = new Map([...documents].map(([id, text]) => [id, relevanceTokens(text)]));
  const frequency = new Map<string, number>();
  for (const set of terms.values()) for (const term of set) frequency.set(term, (frequency.get(term) ?? 0) + 1);
  const average = [...terms.values()].reduce((sum, set) => sum + set.size, 0) / Math.max(terms.size, 1);
  const asked = relevanceTokens(query);
  return new Map([...terms].map(([id, set]) => {
    let score = 0;
    for (const term of asked) {
      if (!set.has(term)) continue;
      const containing = frequency.get(term)!;
      const idf = Math.log(1 + (terms.size - containing + 0.5) / (containing + 0.5));
      score += (idf * 2.2) / (1 + 1.2 * (0.25 + (0.75 * set.size) / average));
    }
    return [id, score];
  }));
}

// ---------------------------------------------------------------------------------------------------------- guard

/** The most added lines one embedded block holds. */
export const GUARD_BLOCK_LINES = 30;
/** The token limit the embedder is loaded with: a full block of code fits. */
export const EMBED_MAX_TOKENS = 1_024;

/** A path's added lines as embedding inputs: blocks of up to {@link GUARD_BLOCK_LINES} lines, each `path\n<lines>`. Blank blocks are left out. */
export function guardBlocks(path: string, added: string): string[] {
  const lines = added.split("\n");
  const blocks: string[] = [];
  for (let start = 0; start < lines.length; start += GUARD_BLOCK_LINES) {
    const block = lines.slice(start, start + GUARD_BLOCK_LINES);
    if (block.some(line => line.trim() !== "")) blocks.push(`${path}\n${block.join("\n")}`);
  }
  return blocks;
}

/** The bundled embedding model, loaded, and how long loading took in milliseconds. */
export function loadEmbedder(modelPath: string): { embedder: NativeSemanticEmbedder; loadMs: number } {
  const started = performance.now();
  const embedder = new NativeSemanticEmbedder(modelPath, EMBED_MAX_TOKENS);
  return { embedder, loadMs: performance.now() - started };
}

let loaded: { embedder: NativeSemanticEmbedder; loadMs: number } | undefined;

/** The model, loaded at most once per hook call: the first caller's load time, 0 for later ones. */
function sharedEmbedder(modelPath: string): { embedder: NativeSemanticEmbedder; loadMs: number } {
  if (loaded !== undefined) return { embedder: loaded.embedder, loadMs: 0 };
  loaded = loadEmbedder(modelPath);
  return loaded;
}

const round4 = (value: number): number => Math.round(value * 10_000) / 10_000;

/** The best cosine between a decision's embedding and any of a path's blocks; null without either. */
function bestSimilarity(vector: readonly number[] | undefined, blocks: readonly Float32Array[]): number | null {
  if (vector === undefined || blocks.length === 0) return null;
  const decision = Float32Array.from(vector);
  return round4(Math.max(...blocks.map(block => cosineSimilarity(decision, block))));
}

/** Whether the guard source fires on a scored item: a signal hit, a similarity at or above `threshold`, or either. */
export function fires(score: Pick<GuardScore, "hits" | "similarity">, source: GuardSource, threshold: number | null): boolean {
  const bySignals = score.hits.length > 0;
  const byEmbed = threshold !== null && score.similarity !== null && score.similarity >= threshold;
  return source === "signals" ? bySignals : source === "embed" ? byEmbed : bySignals || byEmbed;
}

/** The items (a decision, or a peer decision and the own one it contradicts) whose scope covers `path`, each once. */
export function itemsInScope(index: readonly IndexEntry[], path: string): Array<{ ids: string[]; order: number; specificity: number }> {
  const found = new Map<string, { ids: string[]; order: number; specificity: number }>();
  index.forEach((entry, order) => {
    const specificity = scopeMatcher(entry.scope).specificity(path, false);
    if (specificity < 0) return;
    const ids = itemEntries(entry, index).map(member => member.id);
    const key = ids.join("+");
    if (!found.has(key)) found.set(key, { ids, order, specificity });
  });
  return [...found.values()];
}

/** Each item in scope of `path` scored against its added lines: the members' signal hits and best block similarity. */
export function scorePath(index: readonly IndexEntry[], embeddings: HookConfig["embeddings"], path: string, added: string, blocks: readonly Float32Array[]): GuardScore[] {
  return itemsInScope(index, path).map(({ ids }) => {
    const members = ids.map(id => index.find(entry => entry.id === id)!);
    const hits = [...new Set(members.flatMap(member => signalHits(member.signals, added)))];
    const similarities = ids.map(id => bestSimilarity(embeddings[id], blocks)).filter((value): value is number => value !== null);
    return { ids, path, hits, similarity: similarities.length === 0 ? null : Math.max(...similarities) };
  });
}

/**
 * Every scope-matching item on each patched path, scored. The model is loaded (once, timed) only when some item is in
 * scope; every block of every path is embedded in one batch.
 */
async function guardScores(config: HookConfig, patched: ReadonlyArray<{ path: string; added: string }>): Promise<{ scores: GuardScore[]; embedding?: HookLogEntry["embedding"] }> {
  const relevant = patched.filter(({ path }) => itemsInScope(config.index, path).length > 0);
  if (relevant.length === 0) return { scores: [] };
  const blocks = relevant.map(({ path, added }) => guardBlocks(path, added));
  const { embedder, loadMs } = sharedEmbedder(config.modelPath);
  const started = performance.now();
  const vectors = await embedder.embed(blocks.flat());
  const embedMs = performance.now() - started;
  let next = 0;
  const scores = relevant.flatMap(({ path, added }, position) => {
    const own = vectors.slice(next, next + blocks[position]!.length);
    next += blocks[position]!.length;
    return scorePath(config.index, config.embeddings, path, added, own);
  });
  return { scores, embedding: { loadMs: round4(loadMs), embedMs: round4(embedMs), blocks: vectors.length } };
}

// ---------------------------------------------------------------------------------------------------------- selection

/** A scope match, as the log keeps it: an item (a decision, or a peer decision and the own one it contradicts) on one touched path. */
export interface Candidate {
  /** A peer decision first, then this branch's own it contradicts; one id otherwise. */
  readonly ids: readonly string[];
  readonly path: string | null;
  /** On a file the call names, not a directory it lists. */
  readonly direct: boolean;
  readonly specificity: number;
  readonly order: number;
  readonly conflict: boolean;
  /** The relevance of the item's best member to the call's context, in [0, 1]; for a guard match, its similarity (0 without). */
  readonly score: number;
  /** `hybrid`: which channel offers it; for a guard match, its signal hits and its similarity. */
  readonly channel?: Channel;
  readonly hits?: readonly string[];
  readonly similarity?: number | null;
  /** `hybrid` orientation: the order its group comes in (conflict pairs 0, then team, peer and this branch's decisions). */
  readonly group?: number;
  /** Area orientation: the share of the project's files the item's scope covers. */
  readonly breadth?: number;
}

/** A match on a file the call names beats one on a directory it lists; then the narrower scope wins. */
function outranks(hit: Candidate, other: Candidate): boolean {
  return hit.direct !== other.direct ? hit.direct : hit.specificity > other.specificity;
}

function eligible(entry: IndexEntry, only: VariantParameters["only"]): boolean {
  if (only === "conflicts") return entry.conflictsWith !== undefined;
  if (only === "own+team+peer") return entry.source === "own" || entry.source === "team" || entry.source === "peer";
  return true;
}

/** A peer decision first, then the own decision it contradicts; any other decision on its own. */
function itemEntries(entry: IndexEntry, index: readonly IndexEntry[]): IndexEntry[] {
  const partner = entry.conflictsWith === undefined ? undefined : index.find(other => other.id === entry.conflictsWith);
  if (partner === undefined) return [entry];
  return entry.source === "own" ? [partner, entry] : [entry, partner];
}

function rankCandidates(candidates: readonly Candidate[], rank: VariantParameters["rank"]): Candidate[] {
  return [...candidates].sort((left, right) =>
    (rank === "scope-specificity" ? 0 : Number(right.conflict) - Number(left.conflict))
    || (rank === "relevance" ? right.score - left.score : 0)
    || Number(right.direct) - Number(left.direct)
    || right.specificity - left.specificity
    || left.order - right.order);
}

/** Which trigger, if any, this call is for the variant. */
function triggerOf(variant: VariantParameters, state: HookState, event: string, kind: ToolKind): string {
  if (variant.when === "upfront") return state.calls === 0 ? "upfront" : "";
  if (event === "PostToolUse" && kind === "shell" && (variant.when === "post-read" || variant.when === "post-read+pre-edit")) return "post-read";
  if (event === "PreToolUse" && kind === "edit" && (variant.when === "pre-edit" || variant.when === "post-read+pre-edit")) return "pre-edit";
  return "";
}

/** Area orientation's grouping: conflict pairs, then team, peer and this branch's decisions. */
const AREA_GROUPS: Readonly<Record<DecisionSource, number>> = { team: 1, peer: 2, own: 3 };

/**
 * `hybrid`'s candidates. Orientation, `conflicts`: a match for every eligible conflict pair on each path read or
 * patched. `area`: on each such path, a match for every eligible item whose scope covers the path or any project file
 * in its area, with its breadth and its `areaRank` score (`promptVector` for `prompt-embed`). And at a patch, a guard
 * match for every eligible item in scope the variant's guard source fires on.
 */
function hybridCandidates(config: HookConfig, variant: VariantParameters, triggering: readonly Touched[], trigger: string, scores: readonly GuardScore[], promptVector: readonly number[] | null): Candidate[] {
  const found = new Map<string, Candidate>();
  const paths = triggering.filter(touched => touched.edit !== "delete").map(touched => touched.path);
  const areaFiles = new Map(paths.map(path => [path, [path, ...config.files.filter(file => file !== path && areaOf(file) === areaOf(path))]]));
  const lexical = variant.areaRank === "prompt-lexical" ? bm25(config.prompt, new Map(config.index.map(entry => [entry.id, entry.decision]))) : undefined;
  const prompt = promptVector === null ? undefined : Float32Array.from(promptVector);
  const rankScore = (id: string): number => {
    if (lexical !== undefined) return lexical.get(id) ?? 0;
    const vector = config.embeddings[id];
    return prompt === undefined || vector === undefined ? 0 : cosineSimilarity(prompt, Float32Array.from(vector));
  };
  config.index.forEach((entry, order) => {
    if (!eligible(entry, variant.only)) return;
    const members = itemEntries(entry, config.index);
    const ids = members.map(member => member.id);
    const conflict = ids.length > 1;
    if (variant.orient !== "area" && !conflict) return;
    const matcher = scopeMatcher(entry.scope);
    const breadth = config.files.length === 0 ? 1 : config.files.filter(file => matcher.specificity(file, false) >= 0).length / config.files.length;
    const score = variant.orient === "area" ? round4(Math.max(...ids.map(rankScore))) : 0;
    for (const path of paths) {
      const specificity = matcher.specificity(path, false);
      const covered = specificity >= 0 || (variant.orient === "area" && areaFiles.get(path)!.some(file => matcher.specificity(file, false) >= 0));
      if (!covered) continue;
      const group = conflict ? 0 : AREA_GROUPS[members[0]!.source];
      found.set(`orientation ${ids.join("+")} ${path}`, { ids, path, direct: true, specificity: Math.max(specificity, 0), order, conflict, score, channel: "orientation", group, breadth: round4(breadth) });
    }
  });
  if (trigger !== "pre-edit" || variant.guard === null) return [...found.values()];
  const triggered = new Set(triggering.map(touched => touched.path));
  for (const score of scores) {
    if (!triggered.has(score.path) || !fires(score, variant.guard, variant.embedThreshold)) continue;
    const item = itemsInScope(config.index, score.path).find(candidate => candidate.ids.join("+") === score.ids.join("+"));
    if (item === undefined || !score.ids.some(id => eligible(config.index.find(entry => entry.id === id)!, variant.only))) continue;
    found.set(`guard ${score.ids.join("+")} ${score.path}`, {
      ids: score.ids,
      path: score.path,
      direct: true,
      specificity: item.specificity,
      order: item.order,
      conflict: score.ids.length > 1,
      score: score.similarity ?? 0,
      channel: "guard",
      hits: score.hits,
      similarity: score.similarity,
    });
  }
  return [...found.values()];
}

/** How area orientation orders an area's items before the cap: conflict pairs always first. */
function areaOrder(rank: VariantParameters["areaRank"]): (left: Candidate, right: Candidate) => number {
  const breadth = (candidate: Candidate) => candidate.breadth ?? 1;
  return (left, right) => {
    if (left.conflict !== right.conflict) return Number(right.conflict) - Number(left.conflict);
    if (rank === "index" || rank === null) return (left.group ?? 0) - (right.group ?? 0) || left.order - right.order;
    if (rank === "specificity") return breadth(left) - breadth(right) || left.order - right.order;
    return right.score - left.score || breadth(left) - breadth(right) || left.order - right.order;
  };
}

/**
 * `hybrid`'s choice. Orientation, `conflicts`: each path not seen before gets up to `cap` conflict pairs. `area`: each
 * area not seen before gets its undelivered items ranked by `areaRank`, conflict pairs first, at most `cap` decisions
 * in all, then shown grouped by source. Guard: up to `guardCap` items it fires on, those with more signal hits first,
 * then the more similar. Each decision at most once per run.
 */
function chooseHybrid(candidates: readonly Candidate[], triggerPaths: readonly string[], state: HookState, variant: VariantParameters): Candidate[] {
  const delivered = new Set(state.delivered);
  const open = candidates.filter(candidate => candidate.ids.every(id => !delivered.has(id)));
  const key = (candidate: Candidate) => candidate.ids.join("+");
  const hits = (candidate: Candidate) => candidate.hits?.length ?? 0;
  const similarity = (candidate: Candidate) => candidate.similarity ?? -1;
  const taken = new Set<string>();
  const chosen: Candidate[] = [];
  const area = variant.orient === "area";
  const seen = new Set(area ? state.seenAreas : state.seenFiles);
  let decisions = 0;
  for (const path of triggerPaths) {
    const turn = area ? areaOf(path) : path;
    if (seen.has(turn)) continue;
    seen.add(turn);
    const ranked = open
      .filter(candidate => candidate.channel === "orientation" && candidate.path === path && !taken.has(key(candidate)))
      .sort(area ? areaOrder(variant.areaRank) : (left, right) => left.order - right.order);
    const turnChosen: Candidate[] = [];
    for (const candidate of ranked) {
      if (area ? variant.cap !== null && decisions + candidate.ids.length > variant.cap : variant.cap !== null && turnChosen.length >= variant.cap) continue;
      taken.add(key(candidate));
      turnChosen.push(candidate);
      decisions += candidate.ids.length;
    }
    // Shown grouped: conflict pairs, then team, peer and this branch's decisions, each group in rank order.
    chosen.push(...turnChosen.map((candidate, position) => ({ candidate, position })).sort((left, right) => (left.candidate.group ?? 0) - (right.candidate.group ?? 0) || left.position - right.position).map(({ candidate }) => candidate));
  }
  const guardOrder = (left: Candidate, right: Candidate) => hits(right) - hits(left) || similarity(right) - similarity(left) || left.order - right.order;
  const guard = new Map<string, Candidate>();
  for (const candidate of open) {
    if (candidate.channel !== "guard" || taken.has(key(candidate))) continue;
    const previous = guard.get(key(candidate));
    if (previous === undefined || guardOrder(candidate, previous) < 0) guard.set(key(candidate), candidate);
  }
  const ranked = [...guard.values()].sort(guardOrder);
  return [...chosen, ...(variant.guardCap === null ? ranked : ranked.slice(0, variant.guardCap))];
}

// ---------------------------------------------------------------------------------------------------------- focus

const RANKER = fileURLToPath(new URL("./merge-assist-context-rank.ts", import.meta.url));
/** How long a pre-edit in an area the ranker has not answered for waits before ranking the area by prompt embedding. */
const FOCUS_WAIT_MS = 15_000;
export const FOCUS_POLL_MS = 250;
/** How long an area's ranker under `sessionRankMax` waits for the session ranker's answer before ranking in index order. */
export const SESSION_WAIT_MS = 20_000;

/** What a focus ranker is asked about: the task prompt and decisions under opaque ids; `entries` maps the ids back to index ids. */
export interface RankerInput {
  readonly task: string;
  readonly decisions: Array<{ id: string; decision: string }>;
  readonly entries: string[];
}

/**
 * A focus ranker's input: the task prompt, and every index entry whose scope covers a project file, in index order;
 * for an area's ranker (`path`, the file whose touch started it), only those covering `path` or a project file in its
 * area. Entries go under opaque ids (`d1`, `d2`, …) so no id hints at what an entry is; `entries` maps them back.
 */
export function rankerInput(config: Pick<HookConfig, "index" | "files" | "prompt">, path: string | null = null): RankerInput {
  const files = path === null ? config.files : [path, ...config.files.filter(file => file !== path && areaOf(file) === areaOf(path))];
  const entries = config.index.filter(entry => {
    const matcher = scopeMatcher(entry.scope);
    return files.some(file => matcher.specificity(file, false) >= 0);
  });
  return { task: config.prompt, decisions: entries.map((entry, position) => ({ id: `d${position + 1}`, decision: entry.decision })), entries: entries.map(entry => entry.id) };
}

/** `input` with the entries `first` names (index ids, in that order) moved to the front, the rest in index order, opaque ids renumbered. */
export function sessionFirst(input: RankerInput, first: readonly string[]): RankerInput {
  const text = new Map(input.entries.map((id, position) => [id, input.decisions[position]!.decision]));
  const entries = [...new Set(first.filter(id => text.has(id))), ...input.entries.filter(id => !first.includes(id))];
  return { task: input.task, decisions: entries.map((id, position) => ({ id: `d${position + 1}`, decision: text.get(id)! })), entries };
}

/** An area ranker's picks as they are injected: under `agreeAfter`, those after the first `agreeAfter` only if the session ranker it waited for picked them too. */
export function agreedPicks(answer: Pick<FocusFile, "ids" | "session">, variant: Pick<VariantParameters, "agreeAfter">): readonly string[] {
  const { agreeAfter } = variant;
  const session = answer.session ?? [];
  // Without the session's answer there is nothing to agree with: the area's picks go out as the ranker gave them.
  if (agreeAfter === null || session.length === 0) return answer.ids;
  return answer.ids.filter((id, position) => position < agreeAfter || session.includes(id));
}

export function focusPath(folder: string): string {
  return join(folder, "focus.json");
}

/** Where an area's ranker (`orient: area-focus`) writes its answer: `focus-src-ui.json` for `src/ui`, `focus-root.json` for the root. */
export function areaFocusPath(folder: string, area: string): string {
  return join(folder, `focus-${area === "" ? "root" : area.replaceAll("/", "-")}.json`);
}

export function readFocusFile(path: string): FocusFile | undefined {
  return existsSync(path) ? JSON.parse(readFileSync(path, "utf8")) as FocusFile : undefined;
}

/**
 * A ranker in the background, detached and run as this hook is (same node and loader); this call returns at once.
 * With `path`, it ranks that file's area (`orient: area-focus`); without, the session's decisions.
 */
function startRanker(configPath: string, path?: string): void {
  spawn(process.execPath, [...process.execArgv, RANKER, configPath, ...(path === undefined ? [] : [path])], { detached: true, stdio: "ignore", env: process.env }).unref();
}

/** The state with the task prompt's embedding, computed once per run, and the cost of computing it when this call did. */
async function withPromptVector(config: HookConfig, state: HookState): Promise<{ state: HookState; promptEmbedding?: HookLogEntry["promptEmbedding"] }> {
  if (state.promptVector !== null) return { state };
  const { embedder, loadMs } = sharedEmbedder(config.modelPath);
  const started = performance.now();
  const [vector] = await embedder.embed([config.prompt]);
  return {
    state: { ...state, promptVector: [...vector!].map(value => Math.round(value * 1e6) / 1e6) },
    promptEmbedding: { loadMs: round4(loadMs), embedMs: round4(performance.now() - started) },
  };
}

interface FocusTurn {
  readonly state: HookState;
  readonly chosen: Candidate[];
  readonly candidates: Candidate[];
  readonly log: FocusLog;
  readonly promptEmbedding?: HookLogEntry["promptEmbedding"];
}

/** The triggering files whose area this call touches for the first time: one per area, none for a call that does not trigger. */
function freshPaths(state: HookState, trigger: string, triggering: readonly Touched[]): string[] {
  const seen = new Set(state.seenAreas);
  const fresh: string[] = [];
  for (const touched of trigger === "" ? [] : triggering) {
    if (touched.edit === "delete" || seen.has(areaOf(touched.path))) continue;
    seen.add(areaOf(touched.path));
    fresh.push(touched.path);
  }
  return fresh;
}

/**
 * What a focus call injects: each fresh file's conflict pairs, then for each served file its area's picks in the
 * ranker's order (`picks`) or, with `picks` null, the area ranked by prompt embedding (the fallback); undelivered
 * only, at most `areaCap` per area (a fallback without `areaCap`: `focusK`). The state after it, `pendingPaths` aside.
 */
async function focusChoice(config: HookConfig, variant: VariantParameters, initial: HookState, fresh: readonly string[], served: ReadonlyArray<{ path: string; picks: readonly string[] | null }>): Promise<Omit<FocusTurn, "log">> {
  let state = initial;
  let promptEmbedding: HookLogEntry["promptEmbedding"] | undefined;
  const fallback = served.some(({ picks }) => picks === null);
  if (fallback) ({ state, promptEmbedding } = await withPromptVector(config, state));
  const paths = [...new Set([...fresh, ...served.map(({ path }) => path)])];
  const touched = paths.map(path => ({ path, directory: false, created: false, edit: null, added: "" }));
  const candidates = paths.length === 0 ? [] : hybridCandidates(config, { ...variant, orient: "area", areaRank: fallback ? "prompt-embed" : "index" }, touched, "post-read", [], state.promptVector);
  const delivered = new Set(state.delivered);
  const chosen: Candidate[] = [];
  const open = (candidate: Candidate) => candidate.ids.every(id => !delivered.has(id));
  const take = (candidate: Candidate) => {
    for (const id of candidate.ids) delivered.add(id);
    chosen.push(candidate);
  };
  for (const path of fresh) for (const candidate of candidates) if (candidate.path === path && candidate.conflict && open(candidate)) take(candidate);
  for (const { path, picks } of served) {
    const area = candidates.filter(candidate => candidate.path === path && !candidate.conflict);
    const ranked = picks === null ? area.sort(areaOrder("prompt-embed")) : picks.flatMap(id => area.filter(candidate => candidate.ids.includes(id)));
    const cap = variant.areaCap ?? (picks === null ? variant.focusK : null);
    let taken = 0;
    for (const candidate of ranked) {
      if (cap !== null && taken >= cap) break;
      if (!open(candidate)) continue;
      take(candidate);
      taken += 1;
    }
  }
  const ids = chosen.flatMap(candidate => candidate.ids);
  state = {
    ...state,
    calls: state.calls + 1,
    delivered: [...new Set([...state.delivered, ...ids])],
    injected: state.injected + ids.length,
    seenAreas: [...new Set([...state.seenAreas, ...fresh.map(areaOf)])],
  };
  return { state, chosen, candidates, ...(promptEmbedding === undefined ? {} : { promptEmbedding }) };
}

/**
 * One call under `orient: focus`. At an area's first touch its conflict pairs go out at once; its share of the ranker's
 * top `focusK` picks (those whose scope covers the file or the area, not yet delivered, in the ranker's order, at most
 * `areaCap`) goes out as soon as the ranker has answered, at this call or the first one after. A pre-edit in an area
 * the ranker has not answered for waits up to {@link FOCUS_WAIT_MS}, then ranks that area by prompt embedding
 * (`areaCap`, else `focusK`, at most); a failed ranker means the same for the whole run.
 */
async function focusTurn(config: HookConfig, variant: VariantParameters, initial: HookState, trigger: string, triggering: readonly Touched[]): Promise<FocusTurn> {
  const fresh = freshPaths(initial, trigger, triggering);
  let focus = readFocusFile(focusPath(config.folder));
  let waitedMs: number | null = null;
  if (focus === undefined && trigger === "pre-edit" && fresh.length > 0) {
    const started = performance.now();
    while (focus === undefined && performance.now() - started < FOCUS_WAIT_MS) {
      await sleep(FOCUS_POLL_MS);
      focus = readFocusFile(focusPath(config.folder));
    }
    waitedMs = Math.round(performance.now() - started);
  }
  const fallback = focus !== undefined && !focus.ok ? "failed" : focus === undefined && waitedMs !== null ? "timeout" : null;
  const served = focus !== undefined ? [...initial.pendingPaths, ...fresh] : fallback === "timeout" ? fresh : [];
  const pending = focus !== undefined ? [] : fallback === "timeout" ? initial.pendingPaths : [...initial.pendingPaths, ...fresh];
  const picks = focus?.ok === true ? focus.ids.slice(0, variant.focusK ?? focus.ids.length) : [];
  const turn = await focusChoice(config, variant, initial, fresh, served.map(path => ({ path, picks: fallback === null ? picks : null })));
  const log: FocusLog = {
    ready: focus !== undefined,
    ok: focus?.ok ?? null,
    ids: focus?.ids ?? null,
    rankerMs: focus?.ms ?? null,
    promptChars: focus?.promptChars ?? null,
    waitedMs,
    fallback,
    fresh: fresh.map(areaOf),
    served: served.map(areaOf),
    pending: pending.map(areaOf),
  };
  return { ...turn, state: { ...turn.state, pendingPaths: pending }, log };
}

/**
 * One call under `orient: area-focus`. An area's first touch sends its conflict pairs at once and starts that area's
 * ranker in the background, over the decisions covering the touched file or a project file in the area (under
 * `sessionRankMax`, the session ranker's picks listed first); its picks (`agreedPicks`, `areaCap` at most) go out at the
 * first call after it answers. A pre-edit patching an area whose ranking is pending waits up to {@link FOCUS_WAIT_MS}
 * for it, then ranks the area by prompt embedding (`areaCap` at most); a failed ranker means the same for its area.
 * Areas rank concurrently, each into its own `focus-<area>.json`.
 */
async function areaFocusTurn(config: HookConfig, configPath: string, variant: VariantParameters, initial: HookState, trigger: string, triggering: readonly Touched[]): Promise<FocusTurn> {
  const fresh = freshPaths(initial, trigger, triggering);
  for (const path of fresh) startRanker(configPath, path);
  const waiting = [...initial.pendingPaths, ...fresh];
  const answers = new Map<string, FocusFile>();
  const poll = () => {
    for (const path of waiting) {
      const area = areaOf(path);
      const answer = answers.has(area) ? undefined : readFocusFile(areaFocusPath(config.folder, area));
      if (answer !== undefined) answers.set(area, answer);
    }
  };
  poll();
  const patched = new Set(trigger === "pre-edit" ? triggering.filter(touched => touched.edit !== "delete").map(touched => areaOf(touched.path)) : []);
  const unanswered = () => waiting.filter(path => patched.has(areaOf(path)) && !answers.has(areaOf(path)));
  let waitedMs: number | null = null;
  if (unanswered().length > 0) {
    const started = performance.now();
    while (unanswered().length > 0 && performance.now() - started < FOCUS_WAIT_MS) {
      await sleep(FOCUS_POLL_MS);
      poll();
    }
    waitedMs = Math.round(performance.now() - started);
  }
  const timedOut = new Set(unanswered());
  const via = (path: string): AreaFocusLog["via"] | null => {
    const answer = answers.get(areaOf(path));
    if (answer === undefined) return timedOut.has(path) ? "timeout" : null;
    return answer.ok ? "ranker" : "failed";
  };
  const served = waiting.filter(path => via(path) !== null);
  const pending = waiting.filter(path => via(path) === null);
  const turn = await focusChoice(config, variant, initial, fresh, served.map(path => ({ path, picks: via(path) === "ranker" ? agreedPicks(answers.get(areaOf(path))!, variant) : null })));
  const areas = served.map((path): AreaFocusLog => {
    const answer = answers.get(areaOf(path));
    return { area: areaOf(path), via: via(path)!, ids: answer?.ids ?? null, rankerMs: answer?.ms ?? null, promptChars: answer?.promptChars ?? null };
  });
  const vias = areas.map(area => area.via);
  const log: FocusLog = {
    ready: pending.length === 0 && timedOut.size === 0,
    ok: null,
    ids: null,
    rankerMs: null,
    promptChars: null,
    waitedMs,
    fallback: vias.includes("failed") ? "failed" : vias.includes("timeout") ? "timeout" : null,
    fresh: fresh.map(areaOf),
    served: served.map(areaOf),
    pending: pending.map(areaOf),
    areas,
  };
  return { ...turn, state: { ...turn.state, pendingPaths: pending }, log };
}

/**
 * The most characters one call injects under `orient: area-all` or `targets`: Codex keeps about 2,500 tokens of one
 * hook's `additionalContext` and shows the model only a preview of more; at characters / 4 this stays under it.
 */
export const HOOK_CONTEXT_CHARS = 9_000;

/**
 * One call under `orient: area-all` or `targets`, whose picks need no ranker. An area's first touch (as `focus` counts
 * it) sends its conflict pairs, then every undelivered decision covering the touched file or a project file in the
 * area, in index order (`area-all`), or only the task's targets among them (`targets`), with no cap. What would take
 * the call past {@link HOOK_CONTEXT_CHARS} is not cut: it waits, in order, for the next call, which sends it first.
 */
async function fixedPicksTurn(config: HookConfig, variant: VariantParameters, initial: HookState, trigger: string, triggering: readonly Touched[]): Promise<Omit<FocusTurn, "log"> & { deferred: Candidate[] }> {
  const fresh = freshPaths(initial, trigger, triggering);
  const picks = variant.orient === "targets" ? config.targets ?? [] : config.index.map(entry => entry.id);
  const turn = await focusChoice(config, variant, initial, fresh, fresh.map(path => ({ path, picks })));
  const ready = [...initial.queued, ...turn.chosen];
  // Whole items only, at least one a call: a single decision or conflict pair is far below the limit.
  let sent = ready.length === 0 ? 0 : 1;
  while (sent < ready.length && render(ready.slice(0, sent + 1), config.index, variant, false).length <= HOOK_CONTEXT_CHARS) sent += 1;
  const deferred = ready.slice(sent);
  return { ...turn, chosen: ready.slice(0, sent), deferred, state: { ...turn.state, queued: deferred } };
}

/** Every eligible item's scope match on each triggering path (one pathless match each for `upfront`), scored against `context`. */
function candidatesFor(config: HookConfig, variant: VariantParameters, triggering: readonly Touched[] | null, context: string): Candidate[] {
  const weights = decisionWeights(config.index);
  const contextTokens = relevanceTokens(context);
  const scores = new Map(config.index.map(entry => [entry.id, relevance(weights.get(entry.id)!, contextTokens)]));
  const found = new Map<string, Candidate>();
  config.index.forEach((entry, order) => {
    if (!eligible(entry, variant.only)) return;
    const entries = itemEntries(entry, config.index);
    const ids = entries.map(member => member.id);
    const score = Math.max(...ids.map(id => scores.get(id)!));
    const matcher = scopeMatcher(entry.scope);
    const hits = triggering === null
      ? [{ path: null, direct: true, specificity: 0 }]
      : triggering.map(({ path, directory }) => ({ path, direct: !directory, specificity: matcher.specificity(path, directory) })).filter(hit => hit.specificity >= 0);
    for (const hit of hits) {
      const key = `${ids.join("+")} ${hit.path}`;
      const previous = found.get(key);
      if (previous !== undefined && previous.specificity >= hit.specificity) continue;
      found.set(key, { ids, ...hit, order: previous?.order ?? order, conflict: entries.length > 1, score: Math.round(score * 10_000) / 10_000 });
    }
  });
  return [...found.values()];
}

/**
 * What a variant injects from a call's candidates, given the run so far: undelivered (under `dedupe: session`), at or
 * above the threshold (a conflict pair always), ranked, then budgeted. `per-file` gives each path not seen before up
 * to `cap` items; `session` takes each item's best match, `cap` of them, within what `sessionCap` leaves. Pure, so
 * `--replay` can rerun it over logged candidates at other thresholds.
 */
export function choose(candidates: readonly Candidate[], triggerPaths: readonly string[], state: HookState, variant: VariantParameters, trigger: string): Candidate[] {
  if (variant.channels === "hybrid") return chooseHybrid(candidates, triggerPaths, state, variant);
  const delivered = new Set(variant.dedupe === "session" ? state.delivered : []);
  const threshold = variant.threshold;
  const open = candidates.filter(candidate => candidate.ids.every(id => !delivered.has(id)) && (threshold === null || candidate.conflict || candidate.score >= threshold));
  const key = (candidate: Candidate) => candidate.ids.join("+");
  if (variant.budget === "per-file" && trigger !== "upfront") {
    const seen = new Set(state.seenFiles);
    const taken = new Set<string>();
    const chosen: Candidate[] = [];
    for (const path of triggerPaths) {
      if (seen.has(path)) continue;
      seen.add(path);
      const ranked = rankCandidates(open.filter(candidate => candidate.path === path && !taken.has(key(candidate))), variant.rank);
      for (const candidate of variant.cap === null ? ranked : ranked.slice(0, variant.cap)) {
        taken.add(key(candidate));
        chosen.push(candidate);
      }
    }
    return chosen;
  }
  const best = new Map<string, Candidate>();
  for (const candidate of open) {
    const previous = best.get(key(candidate));
    if (previous === undefined || outranks(candidate, previous)) best.set(key(candidate), candidate);
  }
  const ranked = rankCandidates([...best.values()], variant.rank);
  const capped = variant.cap === null ? ranked : ranked.slice(0, variant.cap);
  if (variant.sessionCap === null) return capped;
  // The run's budget, spent in rank order: an item that no longer fits is skipped, a smaller one after it may still fit.
  let left = variant.sessionCap - state.injected;
  return capped.filter(candidate => {
    if (candidate.ids.length > left) return false;
    left -= candidate.ids.length;
    return true;
  });
}

/**
 * The run's state after a call that chose `chosen` from `candidates` and gave `triggerPaths` their turn. An area whose
 * undelivered orientation matches all lost out to the injection's cap keeps its turn for its next touch.
 */
export function advance(state: HookState, candidates: readonly Candidate[], chosen: readonly Candidate[], triggerPaths: readonly string[]): HookState {
  const ids = chosen.flatMap(candidate => candidate.ids);
  const delivered = new Set(state.delivered);
  const orientation = (candidate: Candidate) => candidate.channel === "orientation" && candidate.path !== null;
  const served = new Set(chosen.filter(orientation).map(candidate => areaOf(candidate.path!)));
  const starved = new Set(candidates.filter(candidate => orientation(candidate) && candidate.ids.every(id => !delivered.has(id))).map(candidate => areaOf(candidate.path!)).filter(area => !served.has(area)));
  return {
    ...state,
    calls: state.calls + 1,
    delivered: [...new Set([...state.delivered, ...ids])],
    injected: state.injected + ids.length,
    seenFiles: [...new Set([...state.seenFiles, ...triggerPaths])],
    seenAreas: [...new Set([...state.seenAreas, ...triggerPaths.map(areaOf).filter(area => !starved.has(area))])],
  };
}

// ---------------------------------------------------------------------------------------------------------- formats

/** A decision's text with control characters and runs of whitespace flattened, at most 280 characters. */
function cleanDecision(text: string): string {
  const flat = text.replace(/[\u0000-\u001f\u007f]+/gu, " ").replace(/\s+/gu, " ").trim();
  return flat.length > DECISION_CHARS ? `${flat.slice(0, DECISION_CHARS - 1)}…` : flat;
}

function cardSource(entry: IndexEntry): string {
  if (entry.source === "team") return `Team (${entry.author}${entry.pr === undefined ? "" : `, #${entry.pr}`})`;
  if (entry.source === "peer") return `${entry.branch ?? entry.author} (not merged)`;
  return "This branch";
}

function terseSource(entry: IndexEntry): string {
  if (entry.source === "team") return `team, ${entry.author}${entry.pr === undefined ? "" : ` #${entry.pr}`}`;
  if (entry.source === "peer") return `${entry.branch ?? entry.author}, not merged`;
  return "this branch";
}

const CONFLICT_LINE = "This branch decided the opposite; keep your approach and tell the user, they are settling it.";

/** A decision quoted inside a line: flattened, without its closing period (it would land before the closing quote). */
function quoted(entry: IndexEntry): string {
  return cleanDecision(entry.decision).replace(/\.$/u, "");
}

/** `final-line`: the exact line the agent ends its final message with, naming the teammate's decision. */
function endLine(peer: IndexEntry): string {
  const where = peer.branch ?? (peer.pr === undefined ? "their branch" : `#${peer.pr}`);
  return `End your final message with one line: "Note: ${peer.author} decided differently on ${where}: ${quoted(peer)}".`;
}

/** `final-line`: this branch's decision to keep, and the exact line the agent ends its final message with. */
function finalLine(peer: IndexEntry, own: IndexEntry): string {
  return `This branch decided: "${quoted(own)}". Keep it. ${endLine(peer)}`;
}

function pathsLabel(paths: readonly string[]): string {
  if (paths.length <= 2) return paths.join(" and ");
  return `${paths.slice(0, 2).join(", ")} and ${paths.length - 2} more`;
}

/** The text injected for `chosen`, a decision or a conflict pair each, in their order. */
export function render(chosen: readonly Candidate[], index: readonly IndexEntry[], variant: VariantParameters, upfront: boolean): string {
  const items = chosen.map(candidate => ({ path: candidate.path, entries: candidate.ids.map(id => index.find(entry => entry.id === id)!) }));
  const { format } = variant;
  if (format === "terse") {
    // A conflict pair stays first, but the line to end with comes last, after every other decision, so it is not buried.
    const endings: string[] = [];
    const lines = items.flatMap(item => {
      const [peer, own] = item.entries;
      if (variant.conflict === "final-line" && peer !== undefined && own !== undefined) {
        endings.push(endLine(peer));
        return [`Rerail decision (${terseSource(peer)}): ${cleanDecision(peer.decision)}`, `This branch decided: "${quoted(own)}". Keep it.`];
      }
      return item.entries.map(entry => `Rerail decision (${terseSource(entry)}): ${cleanDecision(entry.decision)}`);
    });
    return [...lines, ...endings].join("\n");
  }
  const paths = [...new Set(items.flatMap(item => (item.path === null ? [] : [item.path])))];
  const lines = [upfront || paths.length === 0 ? "Rerail: decisions for this repository" : `Rerail: decisions that apply to ${pathsLabel(paths)}`];
  for (const item of items) {
    const [first, own] = item.entries;
    if (first === undefined) continue;
    lines.push(`- ${cardSource(first)}: ${cleanDecision(first.decision)}`);
    if (own === undefined) continue;
    if (variant.conflict === "final-line") lines.push(`  ${finalLine(first, own)}`);
    else lines.push(`  ${CONFLICT_LINE}`, `  This branch decided: ${cleanDecision(own.decision)}`);
  }
  lines.push("Follow these where they apply; your task is unchanged.");
  return lines.join("\n");
}

// ---------------------------------------------------------------------------------------------------------- main

function readState(path: string): HookState {
  if (!existsSync(path)) return INITIAL_STATE;
  return JSON.parse(readFileSync(path, "utf8")) as HookState;
}

async function stdinText(): Promise<string> {
  const chunks: Buffer[] = [];
  for await (const chunk of process.stdin) chunks.push(chunk as Buffer);
  return Buffer.concat(chunks).toString("utf8");
}

async function main(): Promise<void> {
  const configPath = process.env.MA_CONTEXT_CONFIG;
  if (configPath === undefined) return;
  const config = JSON.parse(readFileSync(configPath, "utf8")) as HookConfig;
  let event = "unknown";
  let tool = "unknown";
  try {
    const input = JSON.parse(await stdinText()) as Record<string, unknown>;
    event = typeof input.hook_event_name === "string" ? input.hook_event_name : event;
    tool = typeof input.tool_name === "string" ? input.tool_name : tool;
    const { kind, touched, context } = touchedPaths(input, config.root);
    let state = readState(config.state);
    const variant = config.variant;
    const focusing = variant?.orient === "focus";
    // The session ranker starts at SessionStart, or at the first call when no SessionStart came; under `area-focus`
    // only with `sessionRankMax`, for the area rankers to list its picks first.
    if ((focusing || (variant?.orient === "area-focus" && variant.sessionRankMax !== null)) && !state.focusStarted) {
      startRanker(configPath);
      state = { ...state, focusStarted: true };
    }
    if (event === "SessionStart") {
      // A session's start never counts as a call: `upfront` still waits for the first tool call.
      writeFileSync(config.state, JSON.stringify(state));
      const entry: HookLogEntry = { at: Date.now(), event, tool, kind, paths: [], injected: [], chars: 0 };
      appendFileSync(config.log, `${JSON.stringify(entry)}\n`);
      return;
    }
    let trigger = variant === null ? "" : triggerOf(variant, state, event, kind);
    // Without `dirs`, only files a command reads (or a patch edits) trigger: no listed directories, no files about to be created.
    const triggering = variant === null || trigger === "upfront" ? [] : variant.dirs ? touched : touched.filter(item => !item.directory && !item.created);
    if (trigger !== "upfront" && triggering.length === 0) trigger = "";
    // A deleted file gets no turn: there is nothing left there to orient on.
    const triggerPaths = triggering.filter(item => item.edit !== "delete").map(item => item.path);
    // Every patch is scored for the guard, whatever the variant, so control runs can tune its thresholds offline.
    const patched = event === "PreToolUse" && kind === "edit" ? touched.filter(item => item.edit !== "delete" && item.added !== "").map(({ path, added }) => ({ path, added })) : [];
    const { scores, embedding } = patched.length === 0 ? { scores: [], embedding: undefined } : await guardScores(config, patched);
    let promptEmbedding: HookLogEntry["promptEmbedding"] | undefined;
    let focusLog: FocusLog | undefined;
    let deferred: Candidate[] = [];
    let candidates: Candidate[];
    let chosen: Candidate[];
    let next: HookState;
    if (variant?.orient === "focus" || variant?.orient === "area-focus") {
      const turn = focusing ? await focusTurn(config, variant, state, trigger, triggering) : await areaFocusTurn(config, configPath, variant, state, trigger, triggering);
      ({ candidates, chosen, state: next, log: focusLog, promptEmbedding } = turn);
    } else if (variant?.orient === "area-all" || variant?.orient === "targets") {
      ({ candidates, chosen, state: next, deferred } = await fixedPicksTurn(config, variant, state, trigger, triggering));
    } else {
      // `prompt-embed` embeds the task prompt at the first call that can inject, and keeps it for the rest of the run.
      if (variant?.areaRank === "prompt-embed" && trigger !== "") ({ state, promptEmbedding } = await withPromptVector(config, state));
      candidates = variant === null || trigger === ""
        ? []
        : variant.channels === "hybrid"
          ? hybridCandidates(config, variant, triggering, trigger, scores, state.promptVector)
          : candidatesFor(config, variant, trigger === "upfront" ? null : triggering, `${config.prompt}\n${context}`);
      chosen = variant === null || trigger === "" ? [] : choose(candidates, triggerPaths, state, variant, trigger);
      next = trigger === "" ? { ...state, calls: state.calls + 1 } : advance(state, candidates, chosen, triggerPaths);
    }
    const text = chosen.length === 0 ? "" : render(chosen, config.index, variant!, trigger === "upfront");
    const orientation = (candidate: Candidate): string => {
      const area = areaOf(candidate.path!) || ".";
      if (variant!.orient === "area") return `area ${area}`;
      if (variant!.orient === "focus") return `${candidate.conflict ? "conflict" : focusLog?.fallback === null ? "focus" : `fallback ${focusLog?.fallback}`} ${area}`;
      if (variant!.orient === "area-focus") {
        const via = focusLog?.areas?.find(served => served.area === areaOf(candidate.path!))?.via;
        return `${candidate.conflict ? "conflict" : via === "ranker" ? "focus" : `fallback ${via}`} ${area}`;
      }
      if (variant!.orient === "area-all" || variant!.orient === "targets") return `${candidate.conflict ? "conflict" : variant!.orient} ${area}`;
      return "conflict";
    };
    const injected = chosen.flatMap(candidate => candidate.ids.map((decisionId, position) => ({
      decisionId,
      reason: position === 0
        ? `${candidate.channel ?? trigger}${candidate.path === null ? "" : ` ${candidate.path}`} (${candidate.channel === "guard" ? `hits ${candidate.hits!.join(" ") || "none"}, similarity ${candidate.similarity ?? "none"}` : candidate.channel === "orientation" ? orientation(candidate) : `score ${candidate.score}`})`
        : `conflicts with ${candidate.ids[0]!}`,
      ...(candidate.channel === undefined ? {} : { channel: candidate.channel }),
      ...(candidate.path === null ? {} : { area: areaOf(candidate.path) }),
    })));
    if (text !== "") process.stdout.write(`${JSON.stringify({ hookSpecificOutput: { hookEventName: event, additionalContext: text } })}\n`);
    writeFileSync(config.state, JSON.stringify(next));
    const entry: HookLogEntry = {
      at: Date.now(),
      event,
      tool,
      kind,
      paths: touched.map(item => (item.directory ? `${item.path}/` : item.path)),
      injected,
      chars: text.length,
      ...(text === "" ? {} : { text }),
      ...(trigger === "" && candidates.length === 0 ? {} : { trigger, triggerPaths, candidates }),
      ...(patched.length === 0 ? {} : { guard: scores, added: Object.fromEntries(patched.map(({ path, added }) => [path, added])) }),
      ...(embedding === undefined ? {} : { embedding }),
      ...(promptEmbedding === undefined ? {} : { promptEmbedding }),
      ...(focusLog === undefined ? {} : { focus: focusLog }),
      ...(deferred.length === 0 ? {} : { deferred: deferred.flatMap(candidate => candidate.ids) }),
    };
    appendFileSync(config.log, `${JSON.stringify(entry)}\n`);
  } catch (error) {
    const entry: HookLogEntry = { at: Date.now(), event, tool, kind: "other", paths: [], injected: [], chars: 0, error: error instanceof Error ? error.message : String(error) };
    appendFileSync(config.log, `${JSON.stringify(entry)}\n`);
  }
}

if (process.argv[1] !== undefined && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) await main();
