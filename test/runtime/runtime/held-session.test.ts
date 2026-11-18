import { mkdtemp, realpath } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { describe, expect, it, onTestFinished, vi } from "vitest";
import type { AdapterEvent } from "../../../src/engine/index.js";
import { RerailRuntime } from "../../../src/runtime/runtime.js";
import { testEmbedder } from "../../support/hook-events.js";
import { removeFolder } from "../../support/platform/remove-folder.js";
import { setUpTestRepository } from "../../support/repository-rules.js";

/** How many times the runtime has read a repository's rule configuration signature. */
const signatureReads = vi.hoisted(() => ({ count: 0 }));

// The read passed through and counted: what the runtime reads is the module's own answer.
vi.mock("../../../src/engine/catalog/rule-configuration.js", async importOriginal => {
  const actual = await importOriginal<{ ruleConfigurationSignature: (...args: unknown[]) => string }>();
  return {
    ...actual,
    ruleConfigurationSignature: (...args: unknown[]) => {
      signatureReads.count += 1;
      return actual.ruleConfigurationSignature(...args);
    },
  };
});

const RULE = "typescript-eslint-consistent-type-definitions";

/**
 * A runtime for a repository set up with one rule, with conversation rules as the daemon's runtimes have them (a model)
 * or with none (`model: false`), and the start of a session there.
 */
async function startedSession(model: boolean) {
  const root = await realpath(await mkdtemp(join(tmpdir(), "rerail-held-session-")));
  onTestFinished(async () => {
    await removeFolder(root);
  });
  const databasePath = join(root, "rerail.sqlite");
  setUpTestRepository(root, { rerailRules: [RULE] }, databasePath);
  const runtime = new RerailRuntime({
    autostart: false,
    workspaceRoot: root,
    storageRoot: join(root, "sessions"),
    databasePath,
    ...(model ? { modelPath: resolve("model") } : { embedder: testEmbedder }),
    createAuditor: () => ({
      openWorkspace: async () => undefined,
      acceptsPath: () => false,
      auditGeneratedEdits: async (_sessionId, batch) => ({ revision: batch.revision, disposition: "continue", violations: [], failures: [] }),
      close: async () => undefined,
    }),
  });
  // Vitest runs clean-ups in reverse order, so the runtime closes before its folder goes.
  onTestFinished(async () => {
    await runtime.dispose();
  });
  const start: AdapterEvent = { kind: "session-start", eventId: "start", session: { host: "claude-code", sessionId: "held", cwd: root } };
  await runtime.ingest(start);
  return { runtime, start };
}

describe("what a runtime holds of a session at its start", () => {
  it("counts the session's rules against the configuration its start has just read, and reads it no more", async () => {
    const { runtime, start } = await startedSession(true);
    const before = signatureReads.count;
    expect(runtime.heldSession(start)?.sessionNote).toEqual({ setUp: true, ruleCount: 1 });
    // Its conversation rules read the signature in the start's own ingest; a read here would wait again on a slow one.
    expect(signatureReads.count).toBe(before);
  });

  it("counts them in a runtime with no conversation rules by reading the configuration once", async () => {
    const { runtime, start } = await startedSession(false);
    const before = signatureReads.count;
    expect(runtime.heldSession(start)?.sessionNote).toEqual({ setUp: true, ruleCount: 1 });
    expect(signatureReads.count).toBe(before + 1);
  });
});
