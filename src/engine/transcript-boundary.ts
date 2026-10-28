import { deniedByPathBoundary } from "./path-boundary.js";

/**
 * What a coding agent gets in place of transcript content Rerail keeps out of a model conversation: a tool's output,
 * which may be a file it read, or a tool call that names a path outside the data boundary
 * ({@link deniedByPathBoundary}): `cat .env`, a read of `.ssh/id_rsa`, a write to `credentials.json`. People read the
 * transcript whole; this is the agent's view, decided here, in code, not in the agent's instructions.
 */
export interface Withheld {
  withheld: "tool output" | "tool input";
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

/** Strings anywhere in `value`, to a bounded depth and count: a tool input is small, and a pathological one is not read whole. */
function* stringsOf(value: unknown, depth = 0, seen = { count: 0 }): Generator<string> {
  if (depth > 8 || seen.count > 512) return;
  if (typeof value === "string") {
    seen.count += 1;
    yield value;
  } else if (Array.isArray(value)) {
    for (const item of value) yield* stringsOf(item, depth + 1, seen);
  } else if (typeof value === "object" && value !== null) {
    for (const item of Object.values(value)) yield* stringsOf(item, depth + 1, seen);
  }
}

/** Shell and prose delimiters around a token that may be a path. */
const TOKEN_BREAK = /[\s"'`|;&<>(),:=]+/u;

/**
 * Whether a tool call's input names a path outside the data boundary, wherever the path sits: a shell command, a
 * `file_path` argument, a glob. Every string in the input is read as tokens, and a token is a path when it has a
 * letter in it; `.`, `..` and an ellipsis are not. A path is judged by its own segments, so `~/.ssh/id_rsa`, `./.env`
 * and `C:\Users\me\.aws\credentials` are all outside, and `src/.hidden/a.ts` is too.
 */
export function callNamesDeniedPath(input: unknown): boolean {
  for (const text of stringsOf(input)) {
    for (const token of text.split(TOKEN_BREAK)) {
      if (!/[A-Za-z]/u.test(token)) continue;
      const segments = token.split(/[\\/]+/u).filter(segment => segment !== "" && segment !== "." && segment !== "..");
      if (segments.some(segment => segment.startsWith(".."))) continue;
      if (segments.length > 0 && deniedByPathBoundary(segments.join("/"))) return true;
    }
  }
  return false;
}

/**
 * A stored transcript event's payload as a coding agent may see it: a `result` loses its `output`, a `call` naming a
 * path outside the boundary loses its `input`, and everything else (prompts, the assistant's text, lifecycle, steers)
 * is the conversation itself and passes whole. Payloads that are not adapter events pass whole.
 */
export function agentTranscriptPayload(kind: string, payload: unknown): unknown {
  if (typeof payload !== "object" || payload === null || Array.isArray(payload)) return payload;
  if (kind === "result" && "output" in payload) return { ...payload, output: WITHHELD_OUTPUT };
  if (kind === "call" && "input" in payload && callNamesDeniedPath(payload.input)) return { ...payload, input: WITHHELD_INPUT };
  return payload;
}
