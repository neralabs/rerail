import { existsSync } from "node:fs";
import { mkdir, mkdtemp, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { leave, readLedger } from "../../scripts/host/host-lifecycle.js";
import { hasCompletedViolation } from "../../scripts/host/host-smoke.js";
import {
  PARITY_HOSTS,
  REQUIRED_CAPABILITIES,
  parityMatrix,
  type CapabilityScenario,
  type ParityAdapter,
  type ParityRun,
} from "../../integration/support/parity-harness.js";
import { removeFolder } from "../support/platform/remove-folder.js";

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
    expect(cases.map(item => item.name)).toContain("[omp] session-persistence");
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
  it("correlates steers across bookkeeping entries but rejects malformed proof records", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-ledger-proof-"));
    const directory = join(root, ".rerail", "sessions", "s");
    const event = { sequence: 1, sessionId: "s", eventId: "write", kind: "result", observedAt: 1, payload: { toolName: "write", input: { path: "api/probe.py" } } };
    const sample = { type: "sample", item: { metadata: { fields: { eventId: "write", sampleSource: "tool:write", boundary: "tool-call" } } } };
    const steer = "▲ Rerail · rerail-lifecycle:sql-execute-fstring  Steered.";
    const delivery = { type: "delivery", delivery: { requested: "steer", mechanism: "native-queue", content: steer, ruleIds: ["rerail-lifecycle:sql-execute-fstring"] } };
    const unattributed = { type: "delivery", delivery: { requested: "steer", mechanism: "native-queue", content: steer } };
    const ledger = [sample, { type: "hud-tracking-start" }, { type: "future-bookkeeping" }, delivery];
    try {
      await mkdir(directory, { recursive: true });
      const persist = (entries: unknown[]) => writeFile(join(directory, "events.jsonl"), entries.map(entry => JSON.stringify(entry)).join("\n"));
      await persist(ledger);
      expect(await readLedger(root, [event])).toEqual({
        deliveries: 1, steers: 1, content: steer,
        sample: { source: "tool:write", boundary: "tool-call" }, event: { kind: "result", toolName: "write", mentionsProbe: true },
      });
      await persist([delivery, sample]);
      expect((await readLedger(root, [event])).steers).toBe(0);
      await persist([sample, unattributed]);
      expect((await readLedger(root, [event])).steers).toBe(0);
      for (const malformed of [{ type: "sample", item: {} }, { type: "delivery", delivery: { content: steer } }, {}]) {
        await persist([...ledger, malformed]);
        await expect(readLedger(root, [event])).rejects.toThrow();
      }
    } finally {
      await removeFolder(root);
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

describe("the lifecycle run's end", () => {
  const roots: string[] = [];
  const refused = async (): Promise<never> => { throw new Error("Rerail could not prove that pid 4242 is this home's daemon"); };

  afterEach(async () => {
    vi.unstubAllEnvs();
    vi.restoreAllMocks();
    await Promise.all(roots.splice(0).map(async root => await removeFolder(root)));
  });

  /** A run's scratch project with its Rerail home, LIFECYCLE_KEEP set as `keep` says, and what the end writes. */
  async function scratch(keep: boolean): Promise<{ root: string; rerailHome: string; out: string[]; err: string[] }> {
    vi.stubEnv("LIFECYCLE_KEEP", keep ? "1" : "");
    const root = await mkdtemp(join(tmpdir(), "rerail-lifecycle-end-"));
    roots.push(root);
    const rerailHome = join(root, ".rerail-home");
    await mkdir(rerailHome);
    const out: string[] = [];
    const err: string[] = [];
    vi.spyOn(process.stdout, "write").mockImplementation((chunk: string | Uint8Array) => { out.push(String(chunk)); return true; });
    vi.spyOn(process.stderr, "write").mockImplementation((chunk: string | Uint8Array) => { err.push(String(chunk)); return true; });
    return { root, rerailHome, out, err };
  }

  it("keeps a failed run's own error when the daemon's stop throws, says so on stderr and removes the project", async () => {
    const { root, rerailHome, err } = await scratch(false);
    const failure = new Error("the host exited 1");
    await expect(Promise.reject(failure).finally(async () => await leave("omp", { root, rerailHome }, refused))).rejects.toBe(failure);
    expect(err).toContain(`could not stop the Rerail daemon in ${rerailHome}, which may still run; end the process this names: `
      + "Error: Rerail could not prove that pid 4242 is this home's daemon\n");
    expect(existsSync(root)).toBe(false);
  });

  it("keeps a passed run's evidence for its report when the daemon's stop throws, and removes the project", async () => {
    const { root, rerailHome } = await scratch(false);
    const evidence = { passed: true };
    await expect(Promise.resolve(evidence).finally(async () => await leave("omp", { root, rerailHome }, refused))).resolves.toBe(evidence);
    expect(existsSync(root)).toBe(false);
  });

  it("says only that a kept Pi run's project was kept: Pi's run starts no daemon", async () => {
    const { root, rerailHome, out } = await scratch(true);
    await leave("pi", { root, rerailHome }, refused);
    expect(out).toContain(`kept ${root}\n`);
    expect(existsSync(root)).toBe(true);
  });

  it.each([
    { answer: "stopped", stop: async () => "stopped" as const, said: "(its daemon was stopped)" },
    { answer: "not-running", stop: async () => "not-running" as const, said: "(no daemon was running)" },
    { answer: "not-daemon", stop: async () => "not-daemon" as const, said: "(no daemon was running)" },
    { answer: "record-changed", stop: async () => "record-changed" as const, said: "(its daemon was not stopped: daemon.json changed during the stop)" },
    { answer: "a throw", stop: refused, said: "(its daemon could not be stopped)" },
  ])("says what the stop did in a kept OMP run's line: $answer", async ({ stop, said }) => {
    const { root, rerailHome, out } = await scratch(true);
    await leave("omp", { root, rerailHome }, stop);
    expect(out).toContain(`kept ${root} ${said}\n`);
    expect(existsSync(root)).toBe(true);
  });
});
