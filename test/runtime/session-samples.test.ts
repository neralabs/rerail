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

/**
 * Heap bytes in use after full collections, read straight after the last one. The waits between collections let work
 * still in flight finish so its memory can go. Nothing allocated after the last collection is counted: what timers and
 * streams allocate while a loaded machine is slow to run the next task moved a reading taken after a wait by hundreds
 * of kilobytes, either way.
 */
async function heapInUse(): Promise<number> {
  for (let pass = 0; pass < 3; pass += 1) {
    gc();
    await new Promise(resolve => setTimeout(resolve, 10));
  }
  gc();
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

/** A Claude Code session in `root` and its prompts of 20 to 40 KB each. */
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
      // Each prompt is 30 to 40 KB of text. A prompt leaves its event id and compiled code, about 1.5 KB; a session that
      // keeps its samples in memory holds about 34 KB a prompt.
      expect((await heapInUse() - before) / count).toBeLessThan(10_000);
    } finally {
      await runtime.dispose();
    }
  });

  it("stay in its ledger when the session opens again from it, as after an idle close or a daemon restart", { timeout: 60_000 }, async () => {
    const root = await realpath(await mkdtemp(join(tmpdir(), "rerail-samples-")));
    directories.push(root);
    const busy = busySession(root);
    const warm = busySession(root, "warm");
    const count = 100;
    const recording = samplesRuntime(root);
    try {
      await busy.start(recording);
      for (let index = 0; index < count; index += 1) await busy.prompt(recording, index);
      await warm.start(recording);
      await warm.prompt(recording, 0);
    } finally {
      await recording.dispose();
    }
    const reopening = samplesRuntime(root);
    try {
      // Another session opened again from its own ledger first pays for the runtime's one-time state and the compiled
      // code of that path, which would otherwise count against the session measured.
      await warm.prompt(reopening, 1);
      await reopening.codeRulesOpened("warm");
      const before = await heapInUse();
      await busy.prompt(reopening, count);
      await reopening.codeRulesOpened("busy");
      // Its ledger holds a hundred prompts of 20 to 30 KB. Opening it again leaves the session's own state and compiled
      // code, about 2 KB a prompt of its ledger; a session that reads its samples back into memory holds about 32 KB.
      expect((await heapInUse() - before) / count).toBeLessThan(10_000);
    } finally {
      await reopening.dispose();
    }
  });
});
