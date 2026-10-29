import { deniedByPathBoundary } from "./path-boundary.js";

/**
 * What a coding agent gets in place of transcript content Rerail keeps out of a model conversation: a tool's output,
 * which may be a file it read, a tool call that names a path outside the data boundary
 * ({@link deniedByPathBoundary}): `cat .env`, a read of `.ssh/id_rsa`, a write to `credentials.json`, or the payload of
 * an event kind this code does not know. People read the transcript whole; this is the agent's view, decided here, in
 * code, not in the agent's instructions.
 */
export interface Withheld {
  withheld: "tool output" | "tool input" | "event payload";
  reason: string;
}

export const WITHHELD_OUTPUT: Readonly<Withheld> = {
  withheld: "tool output",
  reason: "a tool's output may be a file's contents; Rerail does not hand one session's reads to another coding agent",
};

export const WITHHELD_INPUT: Readonly<Withheld> = {
  withheld: "tool input",
  reason: "the call names a hidden or secret-like path, which Rerail keeps out of a model conversation",
};

export const WITHHELD_PAYLOAD: Readonly<Withheld> = {
  withheld: "event payload",
  reason: "Rerail does not know what this kind of event holds, so it does not hand it to a coding agent",
};

/** How deep, and how many strings, a tool input is read: a tool input is small, and a pathological one is not read whole. */
const INPUT_DEPTH = 8;
const INPUT_STRINGS = 512;

/** Shell, prose and brace-glob delimiters around a token that may be a path. */
const TOKEN_BREAK = /[\s"'`|;&<>(),:={}]+/u;

/** A segment that is no name: `.`, `..` and runs of dots, or a JavaScript spread or prose ellipsis (`...props`). */
const NOT_A_NAME = /^(?:\.+|\.{3}.*)$/u;

function textNamesDeniedPath(text: string): boolean {
  for (const token of text.split(TOKEN_BREAK)) {
    // A bracket class is read as its letters, so `[.]env` is judged as `.env`.
    const path = token.replace(/[[\]]/gu, "");
    if (!/[A-Za-z]/u.test(path)) continue;
    const segments = path.split(/[\\/]+/u).filter(segment => segment !== "" && !NOT_A_NAME.test(segment));
    if (segments.length > 0 && deniedByPathBoundary(segments.join("/"))) return true;
  }
  return false;
}

function namesDeniedPath(value: unknown, depth: number, budget: { strings: number }): boolean {
  if (typeof value !== "string" && (typeof value !== "object" || value === null)) return false;
  // What is past the budget was not read, so it is not known to be clean.
  if (depth > INPUT_DEPTH) return true;
  if (typeof value === "string") {
    budget.strings -= 1;
    return budget.strings < 0 || textNamesDeniedPath(value);
  }
  return (Array.isArray(value) ? value : Object.values(value)).some(item => namesDeniedPath(item, depth + 1, budget));
}

/**
 * Whether a tool call's input names a path outside the data boundary, wherever the path sits: a shell command, a
 * `file_path` argument, a glob. Every string in the input is read as tokens, broken at shell and prose delimiters and
 * at braces, so `{.env,.npmrc}` is two; a token is a path when it has a letter in it. A path is judged by its own
 * segments, less those that are no name (`.`, `..`, `...props`), so `~/.ssh/id_rsa`, `./.env`, `[.]env`,
 * `/mnt/secrets/..data/password` and `C:\Users\me\.aws\credentials` are all outside, and `src/.hidden/a.ts` is too.
 * It reads names, not a shell: a path built at run time (`cat $(echo .e)nv`) is not seen. An input deeper or larger
 * than it reads counts as naming one.
 */
export function callNamesDeniedPath(input: unknown): boolean {
  return namesDeniedPath(input, 0, { strings: INPUT_STRINGS });
}

/**
 * How a coding agent sees each stored event kind: the conversation (the adapter's events: `session-start` through
 * `turn-end`) and Rerail's own bookkeeping (what `SessionStore` records: HUD tracking, steering deliveries, dropped
 * events, pending checks, rule coverage) pass `whole`; a `result` passes without its `output`; a `call` passes without
 * its `input` when that names a path outside the boundary. A kind not listed here is withheld whole, so a kind added
 * later reaches an agent only once it is listed. A map, not a record, so a stored kind such as `constructor` finds no
 * prototype member.
 */
const AGENT_EVENT_VIEW: ReadonlyMap<string, "whole" | "without output" | "without denied input"> = new Map([
  ["session-start", "whole"],
  ["session-end", "whole"],
  ["session-lifecycle", "whole"],
  ["user-prompt", "whole"],
  ["assistant-delta", "whole"],
  ["assistant-message", "whole"],
  ["assistant-abort", "whole"],
  ["turn-end", "whole"],
  ["call", "without denied input"],
  ["result", "without output"],
  ["hud-tracking-start", "whole"],
  ["steering-delivered", "whole"],
  ["events-dropped", "whole"],
  ["source-discovery-pending", "whole"],
  ["code-rules-pending", "whole"],
  ["rule-coverage", "whole"],
]);

/** A stored transcript event's payload as a coding agent may see it ({@link AGENT_EVENT_VIEW}). */
export function agentTranscriptPayload(kind: string, payload: unknown): unknown {
  const view = AGENT_EVENT_VIEW.get(kind);
  if (view === undefined) return WITHHELD_PAYLOAD;
  if (view === "whole" || typeof payload !== "object" || payload === null || Array.isArray(payload)) return payload;
  if (view === "without output") return "output" in payload ? { ...payload, output: WITHHELD_OUTPUT } : payload;
  return "input" in payload && callNamesDeniedPath(payload.input) ? { ...payload, input: WITHHELD_INPUT } : payload;
}
