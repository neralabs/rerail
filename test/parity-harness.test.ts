import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { readLedger } from "../scripts/harness-lifecycle.js";
import { hasCompletedViolation } from "../scripts/harness-smoke.js";
import {
  PARITY_HOSTS,
  REQUIRED_CAPABILITIES,
  parityMatrix,
  type CapabilityScenario,
  type ParityAdapter,
  type ParityRun,
} from "../integration/parity/harness.js";

function adapters(overrides: Partial<Record<ParityAdapter["host"], () => Promise<ParityRun>>> = {}): ParityAdapter[] {
  return PARITY_HOSTS.map(host => ({
    host,
    async open() {
      return overrides[host]?.() ?? {
        async step() { return []; },
        async close() {},
      };
    },
  }));
}

function scenarios(): CapabilityScenario[] {
  return REQUIRED_CAPABILITIES.map(capability => ({
    capability,
    async execute(run) {
      await run.step({ kind: "probe", capability });
    },
  }));
}

describe("parityMatrix", () => {
  it("builds the exhaustive adapter and required-capability product", () => {
    const cases = parityMatrix({ adapters: adapters(), scenarios: scenarios() });
    expect(cases).toHaveLength(PARITY_HOSTS.length * REQUIRED_CAPABILITIES.length);
    expect(cases.map(item => item.name)).toContain("[codex] automatic-delivery");
    expect(cases.map(item => item.name)).toContain("[omp] transcript-persistence");
  });

  it("rejects a missing adapter or required scenario before registration", () => {
    expect(() => parityMatrix({ adapters: adapters().slice(0, -1), scenarios: scenarios() })).toThrow(
      "Parity adapters must contain exactly one of: omp, pi, claude-code, codex, cursor",
    );
    expect(() => parityMatrix({ adapters: adapters(), scenarios: scenarios().slice(0, -1) })).toThrow(
      "Missing required parity scenario: performance-budget",
    );
  });

  it("turns an unsupported adapter operation into a labelled failure and closes the run", async () => {
    let closed = false;
    const matrix = parityMatrix({
      adapters: adapters({
        codex: async () => ({
          async step() { throw new Error("operation is not exposed"); },
          async close() { closed = true; },
        }),
      }),
      scenarios: scenarios(),
    });
    const capability = matrix.find(item => item.name === "[codex] automatic-delivery");
    expect(capability).toBeDefined();

    await expect(capability!.run()).rejects.toThrow(
      "[codex] automatic-delivery: operation is not exposed",
    );
    expect(closed).toBe(true);
  });
});

describe("live harness evidence", () => {
  it("correlates steering across bookkeeping entries but rejects malformed proof records", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-ledger-proof-"));
    const directory = join(root, ".rerail", "sessions", "s");
    const event = { sequence: 1, sessionId: "s", eventId: "write", kind: "result", observedAt: 1, payload: { toolName: "write", input: { path: "api/probe.py" } } };
    const sample = { type: "sample", item: { metadata: { fields: { eventId: "write", sampleSource: "tool:write", boundary: "tool-call" } } } };
    const delivery = { type: "delivery", delivery: { requested: "steer", mechanism: "native-queue", content: "SQL_EXECUTE_FSTRING" } };
    const ledger = [sample, { type: "hud-tracking-start" }, { type: "future-bookkeeping" }, delivery];
    try {
      await mkdir(directory, { recursive: true });
      const persist = (entries: unknown[]) => writeFile(join(directory, "events.jsonl"), entries.map(entry => JSON.stringify(entry)).join("\n"));
      await persist(ledger);
      expect(await readLedger(root, [event])).toEqual({
        deliveries: 1, steers: 1, content: "SQL_EXECUTE_FSTRING",
        sample: { source: "tool:write", boundary: "tool-call" }, event: { kind: "result", toolName: "write", mentionsProbe: true },
      });
      await persist([delivery, sample]);
      expect((await readLedger(root, [event])).steers).toBe(0);
      for (const malformed of [{ type: "sample", item: {} }, { type: "delivery", delivery: { content: "SQL_EXECUTE_FSTRING" } }, {}]) {
        await persist([...ledger, malformed]);
        await expect(readLedger(root, [event])).rejects.toThrow();
      }
    } finally {
      await rm(root, { recursive: true, force: true });
    }
  });

  it("requires a completed violation attempt with the same session, call identity and tool", () => {
    const call = { kind: "call", eventId: "call:1", sessionId: "s", toolCallId: "1", toolName: "exec_command", input: "cursor.execute" };
    const result = { kind: "result", eventId: "result:2", sessionId: "s", toolCallId: "2", toolName: "exec_command", output: "done" };
    const evidence = (...payloads: (typeof call | typeof result | (typeof result & { failed: boolean }))[]) =>
      hasCompletedViolation(payloads.map(payload => ({ kind: payload.kind, payload })));
    const retry = { ...call, eventId: "call:2", toolCallId: "2" };
    expect(evidence(call, { ...result, toolCallId: "1", failed: true })).toBe(false);
    expect(evidence(call, { ...result, toolCallId: "1", failed: true }, retry, result)).toBe(true);
    expect(evidence(call, retry)).toBe(false);
    expect(evidence(call, retry, result)).toBe(true);
    expect(evidence(call, result)).toBe(false);
    expect(evidence(retry, { ...result, toolName: "read" })).toBe(false);
    expect(evidence(retry, { ...result, sessionId: "other" })).toBe(false);
    expect(evidence(result, retry)).toBe(false);
    expect(evidence({ ...retry, input: "read the file" }, { ...result, output: "cursor.execute" })).toBe(false);
  });
});
