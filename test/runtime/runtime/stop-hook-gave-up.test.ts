import { mkdtemp, realpath } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it, onTestFinished } from "vitest";
import type { AdapterDelivery, AdapterEvent } from "../../../src/engine/index.js";
import { RerailRuntime } from "../../../src/runtime/runtime.js";
import { testEmbedder } from "../../support/hook-events.js";
import { removeFolder } from "../../support/platform/remove-folder.js";

/** The steer the turn's edit checks find once the test lets them end. */
const LATE = "late steer";
/** A steer the checks find while the stop waits for the rest. */
const EARLY = "early steer";

/**
 * A Claude Code session whose turn's edit checks, at its stop, steer `early` when given, then end only once the test
 * opens `checks`, and steer `LATE`. The stop's answer is waited for until the test aborts its signal, as the engine
 * service's does when the hook gives up; an event sent with no signal stands for a hook that waits for its answer.
 */
async function heldStopSession(early?: string) {
  const root = await realpath(await mkdtemp(join(tmpdir(), "rerail-stop-gave-up-")));
  onTestFinished(async () => {
    await removeFolder(root);
  });
  const settling = Promise.withResolvers<void>();
  const checks = Promise.withResolvers<void>();
  const steered = Promise.withResolvers<void>();
  let steers = 0;
  const runtime = new RerailRuntime({
    autostart: false,
    workspaceRoot: root,
    storageRoot: join(root, "sessions"),
    databasePath: join(root, "rerail.sqlite"),
    embedder: testEmbedder,
    createAuditor: ({ steerDelivery }) => ({
      openWorkspace: async () => undefined,
      acceptsPath: () => true,
      auditGeneratedEdits: async (_sessionId, batch) => ({ revision: batch.revision, disposition: "continue", violations: [], failures: [] }),
      settleGeneratedEdits: async () => {
        if (steers++ > 0) return;
        if (early !== undefined) await steerDelivery.requestSteer("stop-gave-up", "steer", early, { revision: 0, scope: [] });
        settling.resolve();
        await checks.promise;
        await steerDelivery.requestSteer("stop-gave-up", "steer", LATE, { revision: 0, scope: [] });
        steered.resolve();
      },
      close: async () => undefined,
    }),
  });
  // Vitest runs clean-ups in reverse order, so the runtime closes before its folder goes; the checks end so the stop can.
  onTestFinished(async () => {
    checks.resolve();
    await runtime.dispose();
  });
  const session = { host: "claude-code" as const, sessionId: "stop-gave-up", cwd: root };
  await runtime.ingest({ kind: "session-start", eventId: "start", session });
  await runtime.codeRulesOpened(session.sessionId);
  /**
   * Sends the turn's end with `hook`'s signal, which the test aborts as the hook gives up (one already aborted: the hook
   * gave up before the stop's turn came); `answer` is what the stop answered.
   */
  const stop = (hook = new AbortController()) => {
    const event: AdapterEvent = { kind: "turn-end", eventId: "stop", sessionId: session.sessionId, session, text: "" };
    const answered: { answer?: readonly AdapterDelivery[] } = {};
    const answering = runtime.ingest(event, { signal: hook.signal }).then((deliveries) => {
      answered.answer = deliveries;
    });
    return { giveUp: () => hook.abort(), answered, answering };
  };
  const next = async (eventId: string) => (await runtime.ingest({ kind: "user-prompt", eventId, sessionId: session.sessionId, text: "next" }))
    .map(delivery => delivery.content);
  return { runtime, sessionId: session.sessionId, settling: settling.promise, release: () => checks.resolve(), steered: steered.promise, stop, next };
}

/** Resolves in the next check phase of the event loop: every promise job queued before it has run by then. */
async function nextCheckPhase(): Promise<void> {
  await new Promise<void>(done => setImmediate(done));
}

describe("a Claude Code stop whose hook gave up", () => {
  it("answers at once while the turn's edit checks still run, and the session still owes its agent their steers", async () => {
    const held = await heldStopSession();
    const { giveUp, answered } = held.stop();
    await held.settling;
    giveUp();
    // From the abort to the stop's answer only promise jobs run, so the next check phase finds it answered. Before, the
    // stop waited for every check, holding the session's next prompt and tool calls behind it.
    await nextCheckPhase();
    expect(answered.answer).toEqual([]);
    // The checks run on: an idle close leaves the session open for the steers they find, as for a check that outran
    // a tool result's wait.
    expect(await held.runtime.closeIdleSession(held.sessionId)).toEqual({ kind: "owing" });
  });

  it("waits for none of the checks when its hook gave up before the stop's turn came", async () => {
    const held = await heldStopSession();
    const hook = new AbortController();
    // The hook gave up while the stop waited behind the session's earlier events or its source walk.
    hook.abort();
    const { answered } = held.stop(hook);
    await held.settling;
    await nextCheckPhase();
    expect(answered.answer).toEqual([]);
    expect(await held.runtime.closeIdleSession(held.sessionId)).toEqual({ kind: "owing" });
  });

  it("hands the steers its checks find to the session's next event, once", async () => {
    const held = await heldStopSession();
    const { giveUp, answered, answering } = held.stop();
    await held.settling;
    giveUp();
    held.release();
    await held.steered;
    await answering;
    // Before, the stop took the steer into the answer its hook no longer read, and recorded it as delivered.
    expect(answered.answer).toEqual([]);
    expect(await held.next("next")).toEqual([LATE]);
    expect(await held.next("after")).toEqual([]);
  });

  it("leaves the steers its checks found while it waited for the session's next event, once", async () => {
    const held = await heldStopSession(EARLY);
    const { giveUp, answered } = held.stop();
    await held.settling;
    giveUp();
    await nextCheckPhase();
    // A stop that stopped waiting yet took what the session holds would put it into an answer no one reads.
    expect(answered.answer).toEqual([]);
    expect(await held.next("next")).toEqual([EARLY]);
    expect(await held.next("after")).toEqual([]);
  });

  it("carries the steers its checks find while its hook still waits", async () => {
    const held = await heldStopSession();
    const answering = held.runtime.ingest({ kind: "turn-end", eventId: "stop", sessionId: held.sessionId, text: "" });
    await held.settling;
    held.release();
    expect((await answering).map(delivery => delivery.content)).toEqual([LATE]);
    expect(await held.next("next")).toEqual([]);
  });
});
