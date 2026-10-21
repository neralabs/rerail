import { describe, expect, it } from "vitest";
import { sessionStartNote } from "../../../src/engine/index.js";
import { codexHookOutput } from "../../../src/hosts/codex/codex-hook-adapter.js";
import { goldenJson, NO_EXPLICIT_ANY, TWO_RULES, SCENE_DASHBOARD, SCENE_SESSION } from "../../support/session-enforcement-scenes.js";
import { fixturesPath } from "../../support/oracles.js";

const GOLDEN = fixturesPath("host", "session-enforcement", "codex");

/** Codex's output in each scene of the session enforcement design; review a change to these files with Minh. */
describe("Codex session enforcement golden output", () => {
  it("N1: the note only the coding agent reads", async () => {
    await expect(sessionStartNote(SCENE_DASHBOARD, "codex", SCENE_SESSION, { setUp: true, ruleCount: 26 }))
      .toMatchFileSnapshot(`${GOLDEN}/n1-session-note.txt`);
  });

  it("N4 and N4b: one plain line per rule with its link printed, and the blocks for the agent", async () => {
    await expect(goldenJson(codexHookOutput("PostToolUse", [NO_EXPLICIT_ANY]))).toMatchFileSnapshot(`${GOLDEN}/n4-steer.json`);
    await expect(goldenJson(codexHookOutput("PostToolUse", [TWO_RULES]))).toMatchFileSnapshot(`${GOLDEN}/n4b-two-rules.json`);
  });

  it("N4c: a late steer holds the turn open with its block", async () => {
    await expect(goldenJson(codexHookOutput("Stop", [NO_EXPLICIT_ANY]))).toMatchFileSnapshot(`${GOLDEN}/n4c-late-steer-stop.json`);
  });
});
