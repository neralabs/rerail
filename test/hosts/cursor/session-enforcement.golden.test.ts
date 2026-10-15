import { describe, expect, it } from "vitest";
import { sessionStartNote } from "../../../src/engine/index.js";
import { cursorHookOutput } from "../../../src/hosts/cursor/cursor-hook-adapter.js";
import { goldenJson, NO_EXPLICIT_ANY, NO_FLOATING_PROMISES, SCENE_DASHBOARD, SCENE_SESSION } from "../../support/session-enforcement-scenes.js";
import { fixturesPath } from "../../support/oracles.js";

const GOLDEN = fixturesPath("host", "session-enforcement", "cursor");

/**
 * Cursor's output in each scene of the session enforcement design. Cursor shows the person nothing from a hook, so
 * the steer asks the agent to relay its lines; review a change to these files with Minh.
 */
describe("Cursor session enforcement golden output", () => {
  it("N1: the note only the coding agent reads", async () => {
    await expect(sessionStartNote(SCENE_DASHBOARD, "cursor", SCENE_SESSION, { setUp: true, ruleCount: 26 }))
      .toMatchFileSnapshot(`${GOLDEN}/n1-session-note.txt`);
  });

  it("N4 and N4b: the blocks, then the lines the agent is asked to relay", async () => {
    await expect(goldenJson(cursorHookOutput("postToolUse", [NO_EXPLICIT_ANY]))).toMatchFileSnapshot(`${GOLDEN}/n4-steer.json`);
    await expect(goldenJson(cursorHookOutput("postToolUse", [NO_EXPLICIT_ANY, NO_FLOATING_PROMISES]))).toMatchFileSnapshot(`${GOLDEN}/n4b-two-rules.json`);
  });
});
