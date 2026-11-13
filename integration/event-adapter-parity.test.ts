import { describe, expect, test } from "vitest";
import { createHash } from "node:crypto";
import { readFile } from "node:fs/promises";
import { join } from "node:path";
import { z } from "zod";
import { createClaudeEventHandler } from "../src/hosts/claude/index.js";
import { createCodexEventHandler } from "../src/hosts/codex/index.js";
import { RerailRuntime } from "../src/runtime/index.js";
import { temporaryDirectories, testEmbedder } from "../test/support/hook-events.js";

const temporaryDirectory = temporaryDirectories("rerail-adapter-parity-");

function testRuntime(root: string): RerailRuntime {
  return new RerailRuntime({
    storageRoot: join(root, "sessions"),
    databasePath: join(root, "rerail.sqlite"),
    workspaceRoot: root,
    embedder: testEmbedder,
  });
}

const ledgerLineSchema = z.object({ type: z.string(), item: z.object({ text: z.string() }).passthrough().optional() }).passthrough();

/**
 * The text of each sample the session's ledger holds (`<storage root>/<sha-256 of the session id>/events.jsonl`). A hook
 * host's session keeps its samples there alone, not in memory, so `select` finds none of them.
 */
async function ledgerSamples(root: string, sessionId: string): Promise<string[]> {
  const directory = createHash("sha256").update(sessionId).digest("hex");
  const ledger = await readFile(join(root, "sessions", directory, "events.jsonl"), "utf8");
  return ledger.trim().split("\n").map(line => ledgerLineSchema.parse(JSON.parse(line)))
    .flatMap(entry => (entry.type === "sample" && entry.item !== undefined ? [entry.item.text] : []));
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

      expect(await ledgerSamples(root, "claude")).toContain("claude-47");
      expect(await ledgerSamples(root, "codex")).toContain("codex-47");
    } finally {
      await runtime.dispose();
    }
  });
});
