import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { runClaudeHook } from "../../../src/hosts/claude/claude-hook.js";
import { CLAUDE_HOOK_EVENTS } from "../../../src/setup/index.js";
import { hookOptions, serveText, sessionDatabaseQuery, temporaryDirectories } from "../../support/hook-events.js";
import { fixturesPath } from "../../support/oracles.js";

/**
 * Hook payloads a real Claude Code 2.1.311 session sent on stdin, one per event Rerail registers, with ids, paths and
 * text made neutral but every field and its JSON type kept: Claude Code sends `index` as a number, `final` as a
 * boolean. A Claude Code release that changes a payload's shape is caught here, not on a user's machine.
 */
const PAYLOADS = "claude-code-2.1.311.json";

const temporaryDirectory = temporaryDirectories("rerail-claude-payloads-");

type Payload = Record<string, unknown>;

/** The recorded payloads with `<cwd>` replaced by `cwd`, in the order the session sent them. */
function recordedPayloads(cwd: string): Payload[] {
  const text = readFileSync(fixturesPath("host", "hook-payloads", PAYLOADS), "utf8");
  const swap = (value: unknown): unknown => {
    if (typeof value === "string") return value.replaceAll("<cwd>", cwd);
    if (Array.isArray(value)) return value.map(swap);
    if (value !== null && typeof value === "object") return Object.fromEntries(Object.entries(value).map(([key, item]) => [key, swap(item)]));
    return value;
  };
  return (JSON.parse(text) as Payload[]).map(payload => swap(payload) as Payload);
}

describe("Claude Code hook against payloads a real session sent", () => {
  it("has a recorded payload for every hook event setup registers", () => {
    const recorded = new Set(recordedPayloads("/repo").map(payload => payload.hook_event_name));
    expect(CLAUDE_HOOK_EVENTS.filter(event => !recorded.has(event))).toEqual([]);
  });

  // Twelve hooks, each opening the session store, outlast the default 5 s on a slow disk.
  it("accepts every payload without a hook error and records each event in the session", { timeout: 60_000 }, async () => {
    const root = await temporaryDirectory();
    const options = hookOptions(root);
    const errors: string[] = [];
    for (const payload of recordedPayloads(root)) {
      const { stderr } = await serveText(input => runClaudeHook(input, options), JSON.stringify(payload));
      if (stderr !== "") errors.push(`${String(payload.hook_event_name)}: ${stderr.trim()}`);
    }
    expect(errors).toEqual([]);

    const rows = sessionDatabaseQuery<{ eventId: string; kind: string }>(root, "SELECT event_id AS eventId, kind FROM events ORDER BY sequence")
      .map(row => [row.eventId, row.kind]);
    expect(rows).toEqual(expect.arrayContaining([
      ["fixture-session:SessionStart", "session-start"],
      ["fixture-session:prompt:prompt-1", "user-prompt"],
      ["fixture-session:tool-call:toolu_01FixtureBash000000000000", "call"],
      ["fixture-session:tool-result:toolu_01FixtureBash000000000000", "result"],
      ["fixture-session:tool-call:toolu_01FixtureRead000000000000", "call"],
      ["fixture-session:tool-result:toolu_01FixtureRead000000000000", "result"],
      ["fixture-session:message:message-1:0", "assistant-delta"],
      ["fixture-session:turn:prompt-1", "turn-end"],
      ["fixture-session:prompt:prompt-2", "user-prompt"],
      ["fixture-session:message:message-2:0", "assistant-delta"],
      ["fixture-session:turn-failure:prompt-2", "turn-end"],
      ["fixture-session:SessionEnd", "session-end"],
    ]));
  });
});
