/**
 * Turns a session's stored events (`GET /api/sessions/{id}/events`) into the typed timeline
 * the design shows: turns of prompt + reply, each with its tool calls paired to results and
 * classified into a verb and a side-effect level. Pure functions over event records.
 */
import { HOST_NAME, truncate } from "./format.js";
import { data } from "./state.js";

export const STALE_MIN = 30;

const VERBS = {
  read: { label: "Read", level: "observe" }, find: { label: "Find", level: "observe" }, search: { label: "Search", level: "observe" }, web: { label: "Web", level: "observe" },
  edit: { label: "Edit", level: "change" }, create: { label: "Create", level: "change" }, delete: { label: "Delete", level: "change" },
  run: { label: "Run", level: "execute" }, mcp: { label: "MCP", level: "outside" }, ask: { label: "Ask", level: "ask" },
  agent: { label: "Subagent", level: "delegate" }, todo: { label: "Plan", level: "coordinate" }, other: { label: "Other", level: "other" },
};

/** Harness tool name → verb. Names are matched case-insensitively; unknown names fall to `other`. */
const TOOL_VERB = {
  read: "read", read_file: "read", cat: "read", view: "read",
  glob: "find", file_search: "find", list_dir: "find", ls: "find", find: "find",
  grep: "search", grep_search: "search", codebase_search: "search", search: "search", ripgrep: "search",
  web_search: "web", webfetch: "web", websearch: "web", web_fetch: "web", fetch: "web", web: "web",
  edit: "edit", search_replace: "edit", apply_patch: "edit", multiedit: "edit", str_replace_editor: "edit", str_replace_based_edit_tool: "edit",
  write: "create", write_file: "create", create_file: "create",
  delete_file: "delete", delete: "delete",
  bash: "run", shell: "run", run_terminal_cmd: "run", exec: "run", eval: "run", terminal: "run",
  ask_user: "ask", askuserquestion: "ask", request_approval: "ask", ask: "ask",
  task: "agent", subagent: "agent", agent: "agent",
  todo: "todo", todowrite: "todo", todo_write: "todo", update_plan: "todo",
};

function verbOf(toolName) {
  const name = String(toolName ?? "").toLowerCase();
  if (name.startsWith("mcp") || name.includes("__")) return "mcp";
  return TOOL_VERB[name] ?? "other";
}

/** First matching pattern names the kind of shell command; order matters. */
const RUN_KINDS = [
  [/\b(test|vitest|jest|pytest)\b/, "tests"],
  [/\b(tsc|typecheck|lint|eslint|build)\b/, "build"],
  [/^git\b/, "git"],
  [/\b(npm (i|install)|pnpm (add|install)|pip install)\b/, "install"],
  [/^rm\b/, "delete"],
];
const RUN_KIND = c => RUN_KINDS.find(([re]) => re.test(c))?.[1] ?? "command";

/** The one string that names what a tool call touched. */
function targetOf(verb, input) {
  if (input === null || typeof input !== "object") return typeof input === "string" ? input : "";
  const pick = keys => { for (const k of keys) { const v = input[k]; if (typeof v === "string" && v.trim()) return v.trim(); } return ""; };
  if (verb === "run") return pick(["command", "cmd", "code", "script"]);
  if (verb === "web") return pick(["url", "query"]);
  if (verb === "search" || verb === "find") return pick(["pattern", "query", "path", "glob"]);
  if (verb === "ask") return pick(["question", "prompt", "message"]);
  if (verb === "agent") return pick(["description", "task", "prompt", "label"]);
  if (verb === "todo") return "plan";
  return pick(["file_path", "path", "filePath", "target_file", "file", "target", "message", "text", "i", "description"]) || firstString(input);
}

function firstString(input) {
  for (const v of Object.values(input)) if (typeof v === "string" && v.trim()) return v.trim();
  return "";
}

/** A result's text body, if the harness recorded one. */
export function outputText(output) {
  if (typeof output === "string") return output;
  if (output && typeof output === "object") {
    if (Array.isArray(output.content)) return output.content.map(c => (typeof c === "string" ? c : c?.text ?? "")).join("\n");
    for (const k of ["text", "stdout", "output", "result"]) if (typeof output[k] === "string") return output[k];
  }
  return "";
}

const lineCount = s => (s ? s.split("\n").length : 0);

/** Group one session's events into turns with paired, classified tool calls. */
export function sessionTurns(events) {
  // Builder state: `turn` is the turn taking events, `open` the tool calls awaiting a result.
  const b = { turns: [], turn: null, open: new Map() };
  for (const e of events) applyEvent(b, e, e.payload && typeof e.payload === "object" ? e.payload : {});
  return b.turns.filter(t => t.u !== null || t.a !== null || t.tools.length).map((t, i) => ({ ...t, ti: i, key: t.key || `t${i}` }));
}

function applyEvent(b, e, pl) {
  switch (e.kind) {
    case "user-prompt": onPrompt(b, e, pl); break;
    case "assistant-message": onReply(b, e, pl); break;
    case "call": onToolCall(b, e, pl); break;
    case "result": onToolResult(b, e, pl); break;
    case "turn-end": onTurnEnd(b, e, pl); break;
    default: break;
  }
}

function openTurn(b) {
  b.turn = { key: "", u: null, a: null, uSeq: null, aSeq: null, uTime: null, aTime: null, tools: [], orphans: [], ended: false };
  b.turns.push(b.turn);
  return b.turn;
}

/** The turn taking assistant output: the current one unless it already ended. */
const liveTurn = b => (b.turn === null || b.turn.ended ? openTurn(b) : b.turn);

function setReply(t, text, e) { t.a = text; t.aSeq = e.sequence; t.aTime = e.observedAt; }

/**
 * A block Claude Code's harness writes into a prompt, which the person never typed: a subagent's or background task's
 * report (`<task-notification>`, `<agent-message>`) or a reminder (`<system-reminder>`). Only whole blocks at the start
 * count, so a prompt that mentions one of these tags, opens one it never closes, or quotes a block after the person's
 * own words keeps it.
 */
const HARNESS_BLOCK = /^\s*<(task-notification|agent-message|system-reminder)(?:\s[^>]*)?>[\s\S]*?<\/\1>/;

/** A prompt's text without the harness blocks it opens with; null when nothing the person wrote is left. */
export function promptText(text) {
  let rest = text;
  for (let block = HARNESS_BLOCK.exec(rest); block !== null; block = HARNESS_BLOCK.exec(rest)) rest = rest.slice(block[0].length);
  if (rest === text) return text;
  return rest.trim() || null;
}

function onPrompt(b, e, pl) {
  const t = b.turn;
  const fresh = t === null || t.u !== null || t.ended || t.tools.length ? openTurn(b) : t;
  fresh.u = promptText(String(pl.text ?? "")); fresh.uSeq = e.sequence; fresh.uTime = e.observedAt; fresh.key = `t${e.sequence}`;
}

function onReply(b, e, pl) {
  const t = liveTurn(b);
  if (t.a === null) setReply(t, String(pl.text ?? ""), e);
  if (!t.key) t.key = `t${e.sequence}`;
}

function onToolCall(b, e, pl) {
  const t = liveTurn(b);
  if (!t.key) t.key = `t${e.sequence}`;
  const ev = toolCall(e, pl);
  t.tools.push(ev);
  if (pl.toolCallId) b.open.set(pl.toolCallId, ev);
}

function toolCall(e, pl) {
  const verb = verbOf(pl.toolName), v = VERBS[verb];
  const target = targetOf(verb, pl.input);
  return {
    key: e.eventId, verb, label: v.label, level: v.level, toolName: String(pl.toolName ?? ""), input: pl.input, target,
    title: (pl.input && typeof pl.input === "object" && typeof pl.input.description === "string" && pl.input.description) || target,
    kind: verb === "run" ? RUN_KIND(target) : null,
    callSeq: e.sequence, callTime: e.observedAt, resSeq: null, resTime: null, output: undefined, failed: false, dur: null, corrections: [], raw: [e],
  };
}

function onToolResult(b, e, pl) {
  const ev = pl.toolCallId ? b.open.get(pl.toolCallId) : undefined;
  if (!ev) return;
  ev.resSeq = e.sequence; ev.resTime = e.observedAt; ev.output = pl.output; ev.failed = pl.failed === true;
  ev.dur = Math.max(0, Math.round((e.observedAt - ev.callTime) / 1000));
  ev.lines = lineCount(outputText(pl.output));
  ev.raw.push(e);
  b.open.delete(pl.toolCallId);
}

function onTurnEnd(b, e, pl) {
  const t = b.turn;
  if (t === null) return;
  if (t.a === null && typeof pl.text === "string" && pl.text) setReply(t, pl.text, e);
  t.ended = true;
}

/**
 * How far back the selected session's loaded transcript and violations both reach: events before this time may
 * have violations not loaded yet, and violations before it belong to events not loaded yet, so the timeline shows
 * neither until Load earlier brings them. `-Infinity` once both are loaded to the start.
 */
export function loadedFrom(sessionId) {
  const detail = data.sessionDetail.get(sessionId), slot = data.sessionFindings;
  const events = detail?.earlier ? detail.events[0]?.observedAt ?? Infinity : -Infinity;
  // Violations of one audit share its start and may straddle a page: the oldest loaded start is not yet complete.
  const violations = slot.owner === sessionId && slot.next !== null ? (slot.rows.at(-1)?.at ?? Infinity) + 1 : -Infinity;
  return Math.max(events, violations);
}

/**
 * The session's violations from the pages loaded for it and inside `loadedFrom`, joined to their rules. A violation
 * whose rule is no longer in the repository keeps its card, with `rule: null`, so the timeline still shows it.
 */
export function sessionCorrections(sessionId) {
  const slot = data.sessionFindings;
  if (slot.owner !== sessionId) return [];
  const from = loadedFrom(sessionId);
  return slot.rows
    .filter(f => f.at >= from)
    .map(f => ({ key: `${f.auditId}:${f.id}`, finding: f, rule: data.rules.get(f.ruleId) ?? null }));
}

/** Every violation logged in the session, once the first page of them is in; `null` before. */
export function sessionViolationTotal(sessionId) {
  const slot = data.sessionFindings;
  return slot.owner === sessionId && slot.pages > 0 ? slot.total : null;
}

/**
 * A finding goes under the latest tool call made before its audit started. The daemon records no event for a finding,
 * but an audit starts between one call's result and the next call, so the time places it: `at` is the audit's start,
 * `callTime` the call's. A finding older than every loaded call (the loaded window opens after it) hangs off the first
 * turn, ahead of its calls.
 */
export function placeCorrections(turns, corrections) {
  // The loaded window can start between a tool call and its result, leaving no turn at all: the corrections still
  // show, in a turn of their own, rather than vanish while the header counts them.
  const placed = turns.length === 0 && corrections.length > 0
    ? [{ key: "t-window", ti: 0, u: null, a: null, uSeq: null, aSeq: null, uTime: null, aTime: null, tools: [], orphans: [], ended: true }]
    : turns;
  const calls = placed.flatMap(t => t.tools);
  for (const c of [...corrections].sort(byAuditThenPlace)) {
    const ev = latestCallBefore(calls, c.finding.at);
    if (ev) ev.corrections.push(c);
    else placed[0].orphans.push(c);
  }
  return placed;
}

/** Oldest audit first; within one, by file then line, as a person reads a report. */
const byAuditThenPlace = (a, b) => a.finding.at - b.finding.at
  || (a.finding.path ?? "").localeCompare(b.finding.path ?? "")
  || (a.finding.span?.startLine ?? 0) - (b.finding.span?.startLine ?? 0);

/** The last of `calls` (in sequence order) made at or before `at`; `null` when `at` precedes them all. */
function latestCallBefore(calls, at) {
  let found = null;
  for (const ev of calls) {
    if (ev.callTime > at) break;
    found = ev;
  }
  return found;
}

/** POSIX single-quoting: safe for any path or id, including spaces and quotes. */
const sq = s => `'${String(s).replace(/'/g, "'\\''")}'`;

export const resumeCommand = s => {
  const name = HOST_NAME[s.host] ?? s.host, cd = `cd ${sq(s.cwd)} && `;
  if (s.host === "claude-code") return { name, cmd: `${cd}claude --resume ${sq(s.sessionId)}` };
  if (s.host === "codex") return { name, cmd: `${cd}codex resume ${sq(s.sessionId)}` };
  return { name, cmd: null, cwd: s.cwd };
};

/**
 * When the session was last heard from: the list row's `lastEventAt` (the daemon's newest event), or the loaded
 * transcript's newest event when that is later (the row can lag a poll); a session with no events falls back to its
 * end, then its start.
 */
export function lastEventAt(s) {
  const loaded = data.sessionDetail.get(s.sessionId)?.events.at(-1)?.observedAt ?? null;
  const row = s.lastEventAt ?? null;
  if (row === null && loaded === null) return s.endedAt ?? s.startedAt;
  return Math.max(row ?? -Infinity, loaded ?? -Infinity);
}

/** A list row's violations: logged in the session, and of those the ones steered (recorded by an audit in Steer mode while their rule was on Steer or Block). */
export function violationCounts(s) {
  const logged = s.violationCount ?? 0;
  return { logged, steered: Math.min(logged, s.steeredViolations ?? 0) };
}

/** What the session's whole transcript says it missed, from the daemon (`GET /api/sessions/{id}`); zeros until it answers. */
const gapsOf = s => data.sessionDetail.get(s.sessionId)?.gaps
  ?? { droppedEvents: 0, uncheckedBoundaries: 0, codeRulesPendingBoundaries: 0, ignoredEdits: 0, ignoreRuleChanges: 0 };

/** Session events its host never delivered, from the `events-dropped` records anywhere in the session. */
export const droppedEvents = s => gapsOf(s).droppedEvents;

/** Boundaries that answered before the session's shell-write check finished. */
export const uncheckedBoundaries = s => gapsOf(s).uncheckedBoundaries;

/** Boundaries that answered before the session's code-rule evaluator had opened. */
export const codeRulesPendingBoundaries = s => gapsOf(s).codeRulesPendingBoundaries;

/** Tool edits of files no rule checked because Git ignores them. */
export const ignoredEdits = s => gapsOf(s).ignoredEdits;

/** Changes to Git's ignore rules during the session. */
export const ignoreRuleChanges = s => gapsOf(s).ignoreRuleChanges;

export const isOpen = s => s.endedAt === null;
/** Open and heard from within STALE_MIN → live; open and silent → "no end recorded". */
export function openState(s, now = Date.now()) {
  if (!isOpen(s)) return null;
  const idleMin = (now - lastEventAt(s)) / 60_000;
  return idleMin <= STALE_MIN ? "live" : "stale";
}

export const describeInput = input => truncate(JSON.stringify(input ?? null, null, 2) ?? "", 4000);
