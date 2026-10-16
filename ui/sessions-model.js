/**
 * Turns a session's stored events (`GET /api/sessions/{id}/events`) into the typed timeline
 * the design shows: turns of prompt + reply, each with its tool calls paired to results and
 * classified into a verb and a side-effect level. Pure functions over event records.
 */
import { HOST_NAME, truncate } from "./format.js";
import { data } from "./state.js";

export const STALE_MIN = 30;

export const VERBS = {
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

export function verbOf(toolName) {
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
export function targetOf(verb, input) {
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

function onPrompt(b, e, pl) {
  const t = b.turn;
  const fresh = t === null || t.u !== null || t.ended || t.tools.length ? openTurn(b) : t;
  fresh.u = String(pl.text ?? ""); fresh.uSeq = e.sequence; fresh.uTime = e.observedAt; fresh.key = `t${e.sequence}`;
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
    callSeq: e.sequence, callTime: e.observedAt, resSeq: null, resTime: null, output: undefined, failed: false, dur: null, correction: null, raw: [e],
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

/** The session's violations from the pages loaded for it and inside `loadedFrom`, joined to their rules. */
export function sessionCorrections(sessionId) {
  const slot = data.sessionFindings;
  if (slot.owner !== sessionId) return [];
  const from = loadedFrom(sessionId);
  const out = [];
  for (const f of slot.rows) {
    const stored = data.rules.get(f.ruleId);
    if (stored && f.at >= from) out.push({ key: `${f.auditId}:${f.id}`, finding: f, rule: stored });
  }
  return out;
}

/** Every violation logged in the session, once the first page of them is in; `null` before. */
export function sessionViolationTotal(sessionId) {
  const slot = data.sessionFindings;
  return slot.owner === sessionId && slot.pages > 0 ? slot.total : null;
}

/**
 * Approximate placement: a finding attaches to the first tool call whose input mentions the
 * finding's path; otherwise it hangs off the second turn. The daemon does not record which
 * event a finding interrupted.
 */
export function placeCorrections(turns, corrections) {
  for (const c of corrections) {
    const key = c.finding.path ? c.finding.path.split(":")[0] : null;
    const ev = key ? toolCallTouching(turns, key) : null;
    if (ev) ev.correction = c;
    else if (turns.length) turns[Math.min(1, turns.length - 1)].orphans.push(c);
  }
  return turns;
}

/** First unclaimed tool call on `key`: a change or command targeting it, else any call whose input mentions it. */
function toolCallTouching(turns, key) {
  return findToolCall(turns, ev => (ev.level === "change" || ev.level === "execute") && ev.target.includes(key))
    ?? findToolCall(turns, ev => JSON.stringify(ev.input ?? "").includes(key));
}

function findToolCall(turns, pred) {
  for (const t of turns) for (const ev of t.tools) if (ev.correction === null && pred(ev)) return ev;
  return null;
}

/** POSIX single-quoting: safe for any path or id, including spaces and quotes. */
const sq = s => `'${String(s).replace(/'/g, "'\\''")}'`;

export const resumeCommand = s => {
  const name = HOST_NAME[s.host] ?? s.host, cd = `cd ${sq(s.cwd)} && `;
  if (s.host === "claude-code") return { name, cmd: `${cd}claude --resume ${sq(s.sessionId)}` };
  if (s.host === "codex") return { name, cmd: `${cd}codex resume ${sq(s.sessionId)}` };
  return { name, cmd: null, how: `Open ${name} in ${s.cwd} and find this conversation in its history. Session id: ${s.sessionId}` };
};

/** Last observed event time for a session, when its transcript is loaded. */
export function lastEventAt(s) {
  const detail = data.sessionDetail.get(s.sessionId);
  const last = detail?.events.at(-1);
  return last ? last.observedAt : s.endedAt ?? s.startedAt;
}

/** Session events its host never delivered, from the `events-dropped` gaps it recorded, when its transcript is loaded. */
export function droppedEvents(s) {
  const events = data.sessionDetail.get(s.sessionId)?.events ?? [];
  return events.reduce((total, e) => total + (e.kind === "events-dropped" ? Number(e.payload?.count ?? 0) : 0), 0);
}

/** Boundaries that answered before the session's shell-write check finished, when its transcript is loaded. */
export function uncheckedBoundaries(s) {
  const events = data.sessionDetail.get(s.sessionId)?.events ?? [];
  return events.filter(e => e.kind === "source-discovery-pending").length;
}

/** Boundaries that answered before the session's code-rule evaluator had opened, when its transcript is loaded. */
export function codeRulesPendingBoundaries(s) {
  const events = data.sessionDetail.get(s.sessionId)?.events ?? [];
  return events.filter(e => e.kind === "code-rules-pending").length;
}

export const isOpen = s => s.endedAt === null;
/** Open and heard from within STALE_MIN → live; open and silent → "no end recorded". */
export function openState(s) {
  if (!isOpen(s)) return null;
  const idleMin = (Date.now() - lastEventAt(s)) / 60_000;
  return idleMin <= STALE_MIN ? "live" : "stale";
}

export const describeInput = input => truncate(JSON.stringify(input ?? null, null, 2) ?? "", 4000);
