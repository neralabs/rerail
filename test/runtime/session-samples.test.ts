import { mkdtemp, realpath, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { runInNewContext } from "node:vm";
import { setFlagsFromString } from "node:v8";
import { afterEach, describe, expect, it } from "vitest";
import { RerailRuntime } from "../../src/runtime/runtime.js";
import { testEmbedder } from "../support/hook-events.js";

setFlagsFromString("--expose-gc");
const gc = runInNewContext("gc") as () => void;

/** Heap bytes in use once full collections have run and their sweeping has finished. */
async function heapInUse(): Promise<number> {
  for (let pass = 0; pass < 3; pass += 1) {
    gc();
    await new Promise(resolve => setTimeout(resolve, 10));
  }
  return process.memoryUsage().heapUsed;
}

const directories: string[] = [];
afterEach(async () => {
  await Promise.all(directories.splice(0).map(path => rm(path, { recursive: true, force: true })));
});

describe("a hook host session's samples", () => {
  // A hundred prompts, each through the session's ledger and database, can outlast the default 5 s on a slow disk.
  it("stay in its ledger, not in memory, since nothing in a hook host queries them and a busy session records thousands", { timeout: 60_000 }, async () => {
    const root = await realpath(await mkdtemp(join(tmpdir(), "rerail-samples-")));
    directories.push(root);
    const runtime = new RerailRuntime({
      autostart: false,
      workspaceRoot: root,
      storageRoot: join(root, "sessions"),
      databasePath: join(root, "rerail.sqlite"),
      embedder: testEmbedder,
      createAuditor: () => ({
        openWorkspace: async () => undefined,
        acceptsPath: () => false,
        auditGeneratedEdits: async (_sessionId, batch) => ({ revision: batch.revision, disposition: "continue", violations: [], failures: [] }),
        close: async () => undefined,
      }),
    });
    try {
      const session = { host: "claude-code" as const, sessionId: "busy", cwd: root };
      await runtime.ingest({ kind: "session-start", eventId: "busy:start", session });
      const prompt = (index: number) => runtime.ingest({
        kind: "user-prompt", eventId: `busy:prompt-${index}`, sessionId: "busy", session, text: `${index} `.repeat(10_000),
      });
      // The first prompts pay for the session's one-time state and the compiled code.
      const warmUp = 20;
      for (let index = 0; index < warmUp; index += 1) await prompt(index);
      const before = await heapInUse();
      const count = 100;
      for (let index = warmUp; index < warmUp + count; index += 1) await prompt(index);
      // Each prompt is about 50 KB of text; what a prompt may leave in memory is its event id and some compiled code.
      expect((await heapInUse() - before) / count).toBeLessThan(10_000);
    } finally {
      await runtime.dispose();
    }
  });
});
