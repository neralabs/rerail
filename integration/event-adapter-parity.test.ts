import { describe, expect, test } from "vitest";
import { join } from "node:path";
import { createClaudeEventHandler } from "../src/claude-event-adapter.js";
import { createCodexEventHandler } from "../src/codex-event-adapter.js";
import { RerailRuntime } from "../src/runtime.js";
import { temporaryDirectories, testEmbedder } from "../test/hook-fixtures.js";

const temporaryDirectory = temporaryDirectories("rerail-adapter-parity-");

function testRuntime(root: string): RerailRuntime {
  return new RerailRuntime({
    storageRoot: join(root, "sessions"),
    databasePath: join(root, "rerail.sqlite"),
    workspaceRoot: root,
    embedder: testEmbedder,
  });
}

describe("event adapter parity", () => {
  test("Claude and Codex drive the same runtime interface", async () => {
    const root = await temporaryDirectory();
    const runtime = testRuntime(root);
    const claude = createClaudeEventHandler(runtime);
    const codex = createCodexEventHandler(runtime);
    try {
      await claude({ hook_event_name: "SessionStart", session_id: "claude", cwd: root });
      await codex({ hook_event_name: "SessionStart", session_id: "codex", cwd: root, source: "startup" });
      await Promise.all(Array.from({ length: 48 }, async (_, index) => {
        await claude({
          hook_event_name: "UserPromptSubmit",
          session_id: "claude",
          cwd: root,
          prompt_id: String(index),
          prompt: `claude-${index}`,
        });
        await codex({
          hook_event_name: "UserPromptSubmit",
          session_id: "codex",
          cwd: root,
          turn_id: String(index),
          prompt: `codex-${index}`,
        });
      }));

      const claudeSelection = await runtime.select("claude", {});
      const codexSelection = await runtime.select("codex", {});
      expect(claudeSelection).toEqual(expect.objectContaining({ items: expect.arrayContaining([expect.objectContaining({ text: "claude-47" })]) }));
      expect(codexSelection).toEqual(expect.objectContaining({ items: expect.arrayContaining([expect.objectContaining({ text: "codex-47" })]) }));
    } finally {
      await runtime.dispose();
    }
  });
});
