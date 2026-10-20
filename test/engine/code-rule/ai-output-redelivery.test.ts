import { realpath, writeFile } from "node:fs/promises";
import type * as TimersPromises from "node:timers/promises";
import { join } from "node:path";
import { describe, expect, it, vi } from "vitest";
import type { AdapterDelivery } from "../../../src/engine/adapter-contract.js";
import { NativeCodeRuleEnforcer } from "../../../src/engine/code-rule/code-rule-enforcer.js";
import { SqliteAuditStore } from "../../../src/engine/audit/audit-store.js";
import { codeRuleSchema } from "../../../src/engine/code-rule/code-rule-schema.js";
import { RerailRuntime } from "../../../src/runtime/index.js";
import { temporaryDirectories, testEmbedder } from "../../support/hook-events.js";
import { testCodeRuleCatalog } from "../../support/repository-rules.js";

/** Holds the enforcer's evaluation pump, which starts on a `setImmediate` yield, while `held` is set. */
const pump = vi.hoisted(() => ({ held: undefined as PromiseWithResolvers<void> | undefined }));
vi.mock("node:timers/promises", async importOriginal => {
  const actual = await importOriginal<typeof TimersPromises>();
  return {
    ...actual,
    setImmediate: async (...args: Parameters<typeof actual.setImmediate>) => {
      if (pump.held !== undefined) await pump.held.promise;
      return actual.setImmediate(...args);
    },
  };
});

const temporaryDirectory = temporaryDirectories("rerail-ai-output-redelivery-");

/**
 * A hook host's second edit of a file reaches Rerail before the first edit's steer reaches the agent: the first edit's
 * evaluation is still queued when the second arrives (superseded), or its steer is queued when the second edit's
 * revision drops it (dropped). In ai-output scope the second edit reports what both generated, so a violation the
 * first one added and the second left in place still reaches the agent once, and one the second removed never does.
 */
async function secondEditBeforeDelivery(timing: "superseded" | "dropped", second: string): Promise<string[]> {
  const root = await realpath(await temporaryDirectory());
  const file = join(root, "edited.py");
  await writeFile(file, "a = 1\nb = 2\n");
  const databasePath = join(root, "rerail.sqlite");
  const runtime = new RerailRuntime({
    storageRoot: join(root, "sessions"), databasePath, workspaceRoot: root, autostart: false, embedder: testEmbedder,
    auditSettings: { scope: "ai-output", mode: "steer" },
    createAuditor: options => new NativeCodeRuleEnforcer(testCodeRuleCatalog([codeRuleSchema.parse({
      format: "rerail.policy/v1", id: "test:needle", revision: 1, mode: "steer",
      scope: { include: ["**/*.py"], exclude: [] },
      clauses: [{
        id: "needle",
        violation: { code: "NEEDLE", anchor: "witness.span", message: "No needle" },
        forbid: { regex: { pattern: "NEEDLE", source: "file.content" } },
      }],
    })]), options),
  });
  const audits = new SqliteAuditStore(databasePath);
  const sessionId = `redelivery-${timing}`;
  const delivered: AdapterDelivery[] = [];
  let event = 0;
  const ingest = async (input: { kind: "call" | "result"; toolCallId: string } | { kind: "turn-end" | "user-prompt" }) => {
    const base = { eventId: `event-${++event}`, sessionId };
    delivered.push(...await runtime.ingest(input.kind === "call"
      ? { ...base, kind: "call", toolCallId: input.toolCallId, toolName: "write", input: { path: "edited.py" } }
      : input.kind === "result"
        ? { ...base, kind: "result", toolCallId: input.toolCallId, toolName: "write", output: "written" }
        : input.kind === "turn-end" ? { ...base, kind: "turn-end", text: "done" } : { ...base, kind: "user-prompt", text: "continue" }));
  };
  const release = () => { pump.held?.resolve(); pump.held = undefined; };
  try {
    await runtime.ingest({ kind: "session-start", eventId: "start", session: { host: "claude-code", sessionId, cwd: root } });
    await ingest({ kind: "turn-end" });
    pump.held = Promise.withResolvers<void>();
    await ingest({ kind: "call", toolCallId: "first" });
    await writeFile(file, "a = NEEDLE\nb = 2\n");
    await ingest({ kind: "result", toolCallId: "first" });
    await ingest({ kind: "call", toolCallId: "second" });
    if (timing === "dropped") {
      // The first edit's steer is queued now, behind the second edit's call: its result's revision drops it.
      release();
      await vi.waitFor(() => expect(audits.list({ sessionId }).audits).toEqual([expect.objectContaining({ status: "completed" })]));
    }
    await writeFile(file, second);
    await ingest({ kind: "result", toolCallId: "second" });
    release();
    await ingest({ kind: "turn-end" });
    await ingest({ kind: "user-prompt" });
    return delivered.filter(delivery => delivery.content.includes("No needle")).map(delivery => delivery.content);
  } finally {
    release();
    await runtime.dispose();
    audits.close();
  }
}

describe("ai-output steers of an edit a second edit of its file overtook", () => {
  for (const timing of ["superseded", "dropped"] as const) {
    it(`reach the agent once when the violation stays (${timing})`, async () => {
      expect(await secondEditBeforeDelivery(timing, "a = NEEDLE\nb = 3\n")).toEqual([expect.stringContaining("edited.py:1:5")]);
    });
    it(`never reach the agent once the second edit removed the violation (${timing})`, async () => {
      expect(await secondEditBeforeDelivery(timing, "a = 1\nb = 3\n")).toEqual([]);
    });
  }
});
