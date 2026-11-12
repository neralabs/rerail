import { checkoutPathBoundary, deniedByPathBoundary, sensitiveByName, type CheckoutPathBoundary } from "./path-boundary.js";

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
/** How many of a tool input's path-shaped tokens are resolved on disk, in input order; the rest by name alone. */
const INPUT_RESOLUTIONS = 64;

/**
 * Shell, prose and brace-glob delimiters around a token that may be a path. The split keeps them, at odd indexes, so a
 * URL's lone `:` can be told from the rest ({@link startsUrl}).
 */
const TOKEN_BREAK = /([\s"'`|;&<>(),:={}]+)/u;

/**
 * A URL's scheme (`https`, `ssh`, `git+https`). It has two characters or more, so a drive letter (`C:`) is none, and
 * it is not `file`, whose path names a file on this machine.
 */
const URL_SCHEME = /^(?!file$)[a-z][a-z\d+.-]+$/iu;

/** Whether `token`, after `scheme` and a lone `:`, starts a URL's host and path: `https`, then `//host/api`. */
const startsUrl = (scheme: string | undefined, token: string | undefined): boolean =>
  token?.startsWith("//") === true && URL_SCHEME.test(scheme ?? "");

/** A segment that is no name: `.`, `..` and runs of dots, or a JavaScript spread or prose ellipsis (`...props`). */
const NOT_A_NAME = /^(?:\.+|\.{3}.*)$/u;

/**
 * A token shaped like a path: it has a separator, starts with `.` or `~`, or ends in an extension. Only such a token is
 * resolved on disk; a bare word (a commit message's, a flag's value) is judged by its name, which needs no file system.
 */
const PATH_SHAPED = /[\\/]|^[.~]|\.[^.]+$/u;

/** How a token is judged: by `judge` on its own name and, within a checkout when one is known, by where it leads. */
interface TokenJudge {
  readonly judge: (path: string) => boolean;
  readonly boundary: CheckoutPathBoundary | undefined;
}

function textNamesPath(text: string, { judge, boundary }: TokenJudge): boolean {
  const parts = text.split(TOKEN_BREAK);
  // Whether the token is part of a URL's host and path, and whether that URL has no path yet: until then a lone `:`
  // joins the next token to its host, as a password or a port does (`//user`, `pw@host`, `8080/api`). Past it, a `:`
  // may start a list (`PATH=https://host/bin:/usr/bin`), so the next token is a word of its own.
  let url = false;
  let hostOnly = false;
  for (let index = 0; index < parts.length; index += 2) {
    const token = parts[index] ?? "";
    url = parts[index - 1] === ":" && (hostOnly || startsUrl(parts[index - 2], token));
    hostOnly = url && !token.replace(/^\/\//u, "").includes("/");
    // A bracket class is read as its letters, so `[.]env` is judged as `.env`.
    const path = token.replace(/[[\]]/gu, "");
    if (!/[A-Za-z]/u.test(path)) continue;
    const segments = path.split(/[\\/]+/u).filter(segment => segment !== "" && !NOT_A_NAME.test(segment));
    if (segments.length === 0) continue;
    if (judge(segments.join("/"))) return true;
    // Where a link leads is judged secret-like or not, never hidden: `node_modules/pkg` leads into `.pnpm/`. A URL's
    // host and path name no file here, so they are not looked up: on Windows `//host/api` resolves to the network
    // share `\\host\api`, and the lookup waits for that host, about 21 s for one that never answers.
    if (boundary === undefined || url || !PATH_SHAPED.test(path)) continue;
    const led = boundary.leadsTo(path);
    if (led !== undefined && sensitiveByName(led)) return true;
  }
  return false;
}

function namesPath(value: unknown, depth: number, budget: { strings: number }, judge: TokenJudge): boolean {
  if (typeof value !== "string" && (typeof value !== "object" || value === null)) return false;
  // What is past the budget was not read, so it is not known to be clean.
  if (depth > INPUT_DEPTH) return true;
  if (typeof value === "string") {
    budget.strings -= 1;
    return budget.strings < 0 || textNamesPath(value, judge);
  }
  return (Array.isArray(value) ? value : Object.values(value)).some(item => namesPath(item, depth + 1, budget, judge));
}

const callNames = (input: unknown, judge: (path: string) => boolean, root: string | undefined): boolean =>
  namesPath(input, 0, { strings: INPUT_STRINGS }, {
    judge,
    boundary: root === undefined ? undefined : checkoutPathBoundary(root, INPUT_RESOLUTIONS),
  });

/**
 * Whether a tool call's input names a path outside the data boundary, wherever the path sits: a shell command, a
 * `file_path` argument, a glob. Every string in the input is read as tokens, broken at shell and prose delimiters and
 * at braces, so `{.env,.npmrc}` is two; a token is a path when it has a letter in it. A path is judged by its own
 * segments, less those that are no name (`.`, `..`, `...props`), so `~/.ssh/id_rsa`, `./.env`, `[.]env`,
 * `/mnt/secrets/..data/password` and `C:\Users\me\.aws\credentials` are all outside, and `src/.hidden/a.ts` is too.
 * With the checkout the call ran in (`root`), a path is also outside when it links to a secret-like one
 * ({@link sensitiveByName}), so `cat docs/a.md` is outside when `docs/a.md` links to `.env`, while a link into a hidden
 * folder alone (pnpm's `node_modules/pkg` into `.pnpm/`) is not; only a path-shaped token (`docs/a.md`, `.envrc`,
 * `notes.txt`) is resolved, the first `INPUT_RESOLUTIONS` distinct ones, and a bare word by its name alone, as is a
 * URL's host and path (`//host/api` in `https://host/api`), which name no file on this machine; a `file:` URL's path
 * does, and is resolved. It reads names, not a shell: a path built at run time (`cat $(echo .e)nv`) is not seen. An
 * input deeper or larger than it reads counts as naming one.
 */
export function callNamesDeniedPath(input: unknown, root?: string): boolean {
  return callNames(input, deniedByPathBoundary, root);
}

/**
 * Whether a tool call's input names a secret-like path ({@link sensitiveByName}), read as {@link callNamesDeniedPath}
 * reads it: a hidden path that is not secret-like by name (`.github/workflows/ci.yml`) does not count.
 */
export function callNamesSecretPath(input: unknown, root?: string): boolean {
  return callNames(input, sensitiveByName, root);
}

/**
 * How a coding agent sees each stored event kind: the conversation (the adapter's events: `session-start` through
 * `turn-end`) and Rerail's own bookkeeping (what `SessionStore` records: HUD tracking, steering deliveries, dropped
 * events, pending checks, rule coverage) pass `whole`; a `result` passes without its `output`; a `call` passes without
 * its `input` when that names a path outside the boundary. A kind not listed here is withheld whole, so a kind added
 * later reaches an agent only once it is listed; what Git's ignore rules left unchecked, and changes to them, are
 * withheld, since they quote hidden ignore files. A map, not a record, so a stored kind such as `constructor` finds no
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

/**
 * A stored transcript event's payload as a coding agent may see it ({@link AGENT_EVENT_VIEW}); with the checkout the
 * session ran in (`root`), a call's input is judged by where its paths lead too ({@link callNamesDeniedPath}).
 */
export function agentTranscriptPayload(kind: string, payload: unknown, root?: string): unknown {
  const view = AGENT_EVENT_VIEW.get(kind);
  if (view === undefined) return WITHHELD_PAYLOAD;
  if (view === "whole" || typeof payload !== "object" || payload === null || Array.isArray(payload)) return payload;
  if (view === "without output") return "output" in payload ? { ...payload, output: WITHHELD_OUTPUT } : payload;
  return "input" in payload && callNamesDeniedPath(payload.input, root) ? { ...payload, input: WITHHELD_INPUT } : payload;
}
