import { describe, expect, it } from "vitest";
import { sessionStartNote } from "../../../src/engine/index.js";
import { claudeHookOutput } from "../../../src/hosts/claude/claude-hook-adapter.js";
import { drawn, goldenJson, NO_EXPLICIT_ANY, TWO_RULES, SCENE_DASHBOARD, SCENE_MARK, SCENE_SESSION } from "../../support/session-enforcement-scenes.js";
import { fixturesPath } from "../../support/oracles.js";

const GOLDEN = fixturesPath("host", "session-enforcement", "claude-code");

/** Claude Code's output in each scene of the session enforcement design; review a change to these files with Minh. */
describe("Claude Code session enforcement golden output", () => {
  it("N1: the note only the coding agent reads", async () => {
    await expect(sessionStartNote(SCENE_DASHBOARD, "claude-code", SCENE_SESSION, { setUp: true, ruleCount: 26 }))
      .toMatchFileSnapshot(`${GOLDEN}/n1-session-note.txt`);
  });

  it("N4: one steer, a linked line for the person and the block for the agent", async () => {
    const output = claudeHookOutput("PostToolUse", [NO_EXPLICIT_ANY], { mark: SCENE_MARK });
    await expect(goldenJson(output)).toMatchFileSnapshot(`${GOLDEN}/n4-steer.json`);
    await expect(drawn(String(output.systemMessage))).toMatchFileSnapshot(`${GOLDEN}/n4-steer.txt`);
    await expect(drawn(String(claudeHookOutput("PostToolUse", [NO_EXPLICIT_ANY], { links: "plain", mark: SCENE_MARK }).systemMessage)))
      .toMatchFileSnapshot(`${GOLDEN}/n4-steer-without-links.txt`);
  });

  it("N4b: two rules in one edit, one line each", async () => {
    const output = claudeHookOutput("PostToolUse", [TWO_RULES], { mark: SCENE_MARK });
    await expect(goldenJson(output)).toMatchFileSnapshot(`${GOLDEN}/n4b-two-rules.json`);
    await expect(drawn(String(output.systemMessage))).toMatchFileSnapshot(`${GOLDEN}/n4b-two-rules.txt`);
  });

  it("N4c: a turn-end steer goes out at Stop, the line for the person and the block for the agent, which carries on", async () => {
    await expect(goldenJson(claudeHookOutput("Stop", [NO_EXPLICIT_ANY], { mark: SCENE_MARK }))).toMatchFileSnapshot(`${GOLDEN}/n4c-turn-end-steer-stop.json`);
  });

  it("N4c: a steer held past a turn's end (a continued Stop, or one 0.1.2 held) goes out late with its line at the next tool call", async () => {
    await expect(goldenJson(claudeHookOutput("PostToolUse", [{ ...NO_EXPLICIT_ANY, late: true }], { mark: SCENE_MARK })))
      .toMatchFileSnapshot(`${GOLDEN}/n4c-late-steer-next-message.json`);
  });
});
