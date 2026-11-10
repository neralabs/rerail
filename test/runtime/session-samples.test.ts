import { mkdtemp, realpath } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { runInNewContext } from "node:vm";
import { setFlagsFromString } from "node:v8";
import { afterEach, describe, expect, it } from "vitest";
import { RerailRuntime } from "../../src/runtime/runtime.js";
import { testEmbedder } from "../support/hook-events.js";
import { removeFolder } from "../support/platform/remove-folder.js";

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
  // A session's source walk can still hold its folder open for a moment after its runtime closed, which fails an
  // immediate remove on Windows (EBUSY); removeFolder waits that out.
  await Promise.all(directories.splice(0).map(path => removeFolder(path)));
});

/** A runtime over `root` whose sessions check no source. */
function samplesRuntime(root: string): RerailRuntime {
  return new RerailRuntime({
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
}

/** A Claude Code session in `root` and its prompts of about 50 KB each. */
function busySession(root: string, sessionId = "busy") {
  const session = { host: "claude-code" as const, sessionId, cwd: root };
  return {
    start: (runtime: RerailRuntime) => runtime.ingest({ kind: "session-start", eventId: `${sessionId}:start`, session }),
    prompt: (runtime: RerailRuntime, index: number) => runtime.ingest({
      kind: "user-prompt", eventId: `${sessionId}:prompt-${index}`, sessionId, session, text: `${index} `.repeat(10_000),
    }),
  };
}

describe("a hook host session's samples", () => {
  // A hundred prompts, each through the session's ledger and database, can outlast the default 5 s on a slow disk.
  it("stay in its ledger, not in memory, since nothing in a hook host queries them and a busy session records thousands", { timeout: 60_000 }, async () => {
    const root = await realpath(await mkdtemp(join(tmpdir(), "rerail-samples-")));
    directories.push(root);
    const runtime = samplesRuntime(root);
    try {
      const busy = busySession(root);
      await busy.start(runtime);
      // The first prompts pay for the session's one-time state and the compiled code.
      const warmUp = 20;
      for (let index = 0; index < warmUp; index += 1) await busy.prompt(runtime, index);
      const before = await heapInUse();
      const count = 100;
      for (let index = warmUp; index < warmUp + count; index += 1) await busy.prompt(runtime, index);
      // Each prompt is about 50 KB of text; what a prompt may leave in memory is its event id and some compiled code.
      expect((await heapInUse() - before) / count).toBeLessThan(10_000);
    } finally {
      await runtime.dispose();
    }
  });

  it("stay in its ledger when the session opens again from it, as after an idle close or a daemon restart", { timeout: 60_000 }, async () => {
    const root = await realpath(await mkdtemp(join(tmpdir(), "rerail-samples-")));
    directories.push(root);
    const busy = busySession(root);
    const count = 100;
    const recording = samplesRuntime(root);
    try {
      await busy.start(recording);
      for (let index = 0; index < count; index += 1) await busy.prompt(recording, index);
    } finally {
      await recording.dispose();
    }
    const reopening = samplesRuntime(root);
    try {
      // A session of its own pays for the runtime's one-time state and the compiled code.
      const warm = busySession(root, "warm");
      await warm.start(reopening);
      await warm.prompt(reopening, 0);
      await reopening.codeRulesOpened("warm");
      const before = await heapInUse();
      await busy.prompt(reopening, count);
      await reopening.codeRulesOpened("busy");
      // Its ledger holds a hundred prompts of about 50 KB; what opening it again may leave is the session's own state.
      expect((await heapInUse() - before) / count).toBeLessThan(10_000);
    } finally {
      await reopening.dispose();
    }
  });
});
