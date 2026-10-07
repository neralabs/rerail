import { afterEach, describe, expect, it, vi } from "vitest";
import { placeCorrections, resumeCommand, sessionTurns, type Correction } from "../ui/sessions-model.js";
import { titleFromPrompts } from "../ui/poll.js";
import { sinceFor } from "../ui/state.js";

const T0 = 1_700_000_000_000;

/** Stored-event records in arrival order: `sequence` counts up, `observedAt` advances `at` ms per event. */
function events(list: readonly { kind: string; payload: unknown; at?: number }[]) {
  let time = T0;
  return list.map((e, i) => {
    time += e.at ?? 1_000;
    return { sequence: i + 1, sessionId: "s1", eventId: `e${i + 1}`, kind: e.kind, observedAt: time, payload: e.payload };
  });
}

const prompt = (text: string) => ({ kind: "user-prompt", payload: { text } });
const call = (toolCallId: string, toolName: string, input: unknown) => ({ kind: "call", payload: { toolCallId, toolName, input } });
const result = (toolCallId: string, output: unknown, extra: { failed?: boolean; at?: number } = {}) =>
  ({ kind: "result", payload: { toolCallId, toolName: "x", output, ...(extra.failed === undefined ? {} : { failed: extra.failed }) }, ...(extra.at === undefined ? {} : { at: extra.at }) });
const turnEnd = (text = "") => ({ kind: "turn-end", payload: { text } });

const correction = (key: string, path: string | null): Correction => ({ key, finding: { path }, rule: {} });

describe("sessionTurns", () => {
  it("pairs a tool result with its call and measures the round trip", () => {
    const turns = sessionTurns(events([
      prompt("Run the tests"),
      call("c1", "Bash", { command: "npx vitest run" }),
      result("c1", "a\nb\nc", { at: 3_400 }),
      turnEnd("All green."),
    ]));
    expect(turns).toHaveLength(1);
    const [t] = turns;
    expect(t).toMatchObject({ ti: 0, u: "Run the tests", a: "All green.", ended: true, orphans: [] });
    expect(t!.tools).toHaveLength(1);
    expect(t!.tools[0]).toMatchObject({
      verb: "run", level: "execute", kind: "tests", target: "npx vitest run",
      callSeq: 2, resSeq: 3, output: "a\nb\nc", failed: false, dur: 3, lines: 3, correction: null,
    });
  });

  it("keeps the streamed assistant message over the turn-end text and marks failed results", () => {
    const [t] = sessionTurns(events([
      prompt("Do it"),
      { kind: "assistant-message", payload: { text: "Working on it" } },
      call("c1", "Read", { file_path: "src/a.ts" }),
      result("c1", { content: [{ text: "line 1" }, { text: "line 2" }] }, { failed: true }),
      turnEnd("Finished"),
    ]));
    expect(t!.a).toBe("Working on it");
    expect(t!.tools[0]).toMatchObject({ verb: "read", level: "observe", failed: true, lines: 2 });
  });

  it("drops a result whose call was never seen and leaves the pending call unresolved", () => {
    const [t] = sessionTurns(events([
      prompt("Look around"),
      call("c1", "Grep", { pattern: "foo" }),
      result("never-issued", "noise"),
    ]));
    expect(t!.tools).toHaveLength(1);
    expect(t!.tools[0]).toMatchObject({ resSeq: null, dur: null, output: undefined });
    expect(t!.ended).toBe(false);
  });

  it("closes a turn that has tool calls and a reply but no prompt", () => {
    const turns = sessionTurns(events([
      call("c1", "Edit", { file_path: "src/a.ts" }),
      result("c1", "ok"),
      turnEnd("Edited a.ts"),
    ]));
    expect(turns).toHaveLength(1);
    expect(turns[0]).toMatchObject({ u: null, a: "Edited a.ts", ended: true });
    expect(turns[0]!.tools[0]!.resSeq).toBe(2);
  });

  it("starts a new turn for a prompt after the previous turn ended, and for output after it", () => {
    const turns = sessionTurns(events([
      prompt("First"),
      turnEnd("One"),
      prompt("Second"),
      turnEnd("Two"),
      call("c9", "Bash", { command: "git status" }),
    ]));
    expect(turns.map(t => [t.ti, t.u, t.a, t.ended])).toEqual([
      [0, "First", "One", true],
      [1, "Second", "Two", true],
      [2, null, null, false],
    ]);
    expect(turns[2]!.tools[0]!.kind).toBe("git");
  });
});

describe("placeCorrections", () => {
  /** Turn 0 only observes `src/a.ts`; turn 1 runs a command on it, then edits it. */
  const fixture = () => sessionTurns(events([
    prompt("Look first"),
    call("r1", "Read", { file_path: "src/a.ts" }),
    result("r1", "…"),
    turnEnd("Seen."),
    prompt("Now fix it"),
    call("x1", "Bash", { command: "prettier --write src/a.ts" }),
    result("x1", ""),
    call("e1", "Edit", { file_path: "src/a.ts", old_string: "a", new_string: "b" }),
    result("e1", "ok"),
    turnEnd("Fixed."),
  ]));

  it("prefers the first unclaimed change or command targeting the path, then any call mentioning it", () => {
    const turns = fixture();
    const [first, second, third] = [correction("k1", "src/a.ts:12"), correction("k2", "src/a.ts:30"), correction("k3", "src/a.ts")];
    placeCorrections(turns, [first, second, third]);
    const byId = Object.fromEntries(turns.flatMap(t => t.tools).map(ev => [ev.toolName, ev.correction?.key ?? null]));
    expect(byId).toEqual({ Bash: "k1", Edit: "k2", Read: "k3" });
    expect(turns.flatMap(t => t.orphans)).toEqual([]);
  });

  it("orphans a finding no call touches onto the second turn, or the only turn", () => {
    const turns = fixture();
    const missing = correction("k1", "src/elsewhere.ts:1");
    const pathless = correction("k2", null);
    placeCorrections(turns, [missing, pathless]);
    expect(turns[0]!.orphans).toEqual([]);
    expect(turns[1]!.orphans.map(c => c.key)).toEqual(["k1", "k2"]);

    const single = sessionTurns(events([prompt("Hi"), turnEnd("Hello")]));
    placeCorrections(single, [missing]);
    expect(single[0]!.orphans.map(c => c.key)).toEqual(["k1"]);
  });
});

describe("resumeCommand", () => {
  it("builds the Claude Code and Codex commands with a shell-safe cwd and session id", () => {
    expect(resumeCommand({ host: "claude-code", cwd: "/Users/x/My Projects/app", sessionId: "abc-123" })).toEqual({
      name: "Claude Code",
      cmd: "cd '/Users/x/My Projects/app' && claude --resume 'abc-123'",
    });
    expect(resumeCommand({ host: "codex", cwd: "/tmp/it's here", sessionId: "s'id" })).toEqual({
      name: "Codex",
      cmd: "cd '/tmp/it'\\''s here' && codex resume 's'\\''id'",
    });
  });

  it("has no command for hosts without a resume CLI but still names the session", () => {
    const r = resumeCommand({ host: "cursor", cwd: "/w", sessionId: "cur-1" });
    expect(r.name).toBe("Cursor");
    expect(r.cmd).toBeNull();
    expect(r.how).toContain("cur-1");
  });
});

describe("titleFromPrompts", () => {
  it("skips acknowledgements and short prompts in favour of the first real request", () => {
    expect(titleFromPrompts(["ok", "yes.", "fix bug", "Refactor the login flow please", "then do more"])).toBe("Refactor the login flow please");
  });

  it("falls back to the first prompt when nothing better exists, and to null when there are none", () => {
    expect(titleFromPrompts(["ok", "y"])).toBe("ok");
    expect(titleFromPrompts([])).toBeNull();
  });

  it("uses only the first line, collapsed, and trims to the title limit with an ellipsis", () => {
    expect(titleFromPrompts(["  Refactor   the\tlogin flow\nand then the rest"])).toBe("Refactor the login flow");
    const exact = "x".repeat(96);
    expect(titleFromPrompts([exact])).toBe(exact);
    const long = `${"word ".repeat(19)}tail-that-runs-over-the-limit`;
    const title = titleFromPrompts([long])!;
    expect(title.length).toBeLessThanOrEqual(96);
    expect(title.endsWith("…")).toBe(true);
    expect(long.startsWith(title.slice(0, -1))).toBe(true);
  });
});

describe("sinceFor", () => {
  afterEach(() => vi.useRealTimers());

  it("quantizes sliding ranges so the value holds still within ten minutes and steps after", () => {
    const QUANTUM = 600_000, DAY = 86_400_000;
    const aligned = Math.ceil(T0 / QUANTUM) * QUANTUM;
    vi.useFakeTimers({ now: aligned + 7 * DAY + 1_000 });
    expect(sinceFor(7)).toBe(aligned);
    vi.advanceTimersByTime(QUANTUM - 2_000);
    expect(sinceFor(7)).toBe(aligned);
    vi.advanceTimersByTime(2_000);
    expect(sinceFor(7)).toBe(aligned + QUANTUM);
  });

  it("reads local midnight for today and 0 for since-install", () => {
    vi.useFakeTimers({ now: T0 });
    const midnight = new Date(T0);
    midnight.setHours(0, 0, 0, 0);
    expect(sinceFor(0)).toBe(midnight.getTime());
    expect(sinceFor(9999)).toBe(0);
  });
});
