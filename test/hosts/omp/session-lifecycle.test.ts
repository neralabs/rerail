import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import type { ExtensionAPI as OmpAPI, ExtensionContext as OmpContext } from "@oh-my-pi/pi-coding-agent";
import type { ExtensionAPI as PiAPI, ExtensionContext as PiContext } from "@earendil-works/pi-coding-agent";
import { afterEach, describe, expect, it, vi } from "vitest";
import { z } from "zod";
import { createConversationRulesExtension } from "../../../src/hosts/omp/extension.js";
import { createPiConversationRulesExtension } from "../../../src/hosts/pi/index.js";
import { CodeRuleBridge, type ConversationRule, defaultRerailDatabasePath, linkSteers, steerBlock, steerOrigin } from "../../../src/engine/index.js";
import { EVENTS_DROPPED_ENTRY_TYPE, MATCH_ENTRY_TYPE, RerailRuntime, SqliteSessionReader, TURN_ENTRY_TYPE } from "../../../src/runtime/index.js";
import { steerDashboard } from "../../../src/presentation/index.js";
import { closeNativeTestDaemons, startNativeTestDaemon } from "../../support/native-daemon.js";
import { setUpTestRepository } from "../../support/repository-rules.js";

async function checkoutSteer(cwd: string): Promise<string> {
  return linkSteers(steerBlock("checkout", "Check checkout requirements.", { mode: "steer", origin: steerOrigin("checkout") }), await steerDashboard(cwd));
}
type Context = OmpContext & PiContext;
type Handler = (event: unknown, context: Context) => unknown;
const directories: string[] = [];
const extensions: Array<{ shutdown(): Promise<unknown> }> = [];
afterEach(async () => {
  // A test that overran its timeout keeps running after this hook. Fence its extension and shut it down while the
  // in-process daemon still answers: shutdown waits out any attach in flight, and an attach started once that daemon
  // is closed would find no daemon in whichever RERAIL_HOME is stubbed by then and spawn a real one.
  await Promise.allSettled(extensions.splice(0).map(extension => extension.shutdown()));
  await closeNativeTestDaemons();
  vi.unstubAllEnvs();
  vi.restoreAllMocks();
  for (const path of directories.splice(0)) rmSync(path, { recursive: true, force: true });
});

async function harness(host: "omp" | "pi") {
  const cwd = mkdtempSync(join(tmpdir(), `rerail-${host}-startup-`));
  directories.push(cwd);
  vi.stubEnv("RERAIL_HOME", join(cwd, "home"));
  const daemon = host === "omp" ? await startNativeTestDaemon(cwd) : undefined;
  const handlers = new Map<string, Handler>();
  const commands = new Map<string, { handler(args: string, context: Context): unknown }>();
  const messages: unknown[] = [];
  const entries: Array<{ customType: string; data: unknown }> = [];
  // Where OMP's appendEntry lands: the session that is current, as the last session event delivered it.
  const entriesBySession = new Map<string, Array<{ customType: string; data: unknown }>>();
  let liveSession = "original";
  const branch: unknown[] = [];
  const api = {
    zod: z,
    setLabel() {},
    on(name: string, handler: Handler) { handlers.set(name, handler); },
    registerCommand(name: string, command: { handler(args: string, context: Context): unknown }) { commands.set(name, command); },
    registerTool() {},
    registerMessageRenderer() {},
    sendMessage(message: unknown) { messages.push(message); },
    appendEntry(customType: string, data: unknown) {
      entries.push({ customType, data });
      entriesBySession.set(liveSession, [...entriesBySession.get(liveSession) ?? [], { customType, data }]);
    },
  } as unknown as OmpAPI & PiAPI;
  const context = {
    cwd,
    sessionManager: { getSessionId: () => "original", getBranch: () => branch },
    ui: { notify() {} },
  } as unknown as Context;
  const rule: ConversationRule = {
    id: "checkout", mode: "steer", reference: "Guest checkout remains available.",
    match: { operator: "gte", threshold: 0.99 },
    sample: { source: "assistant-text", boundary: "sentence", minimumTokens: 1 },
    action: { content: "Check checkout requirements." },
    repeat: { mode: "always" },
  };
  // Only rules a repository adds run.
  setUpTestRepository(cwd, { library: [{ kind: "conversation", rule }] });
  const createExtension = host === "omp" ? createConversationRulesExtension : createPiConversationRulesExtension;
  createExtension({ modelPath: resolve("model"), registerTool: false })(api);
  let tornDown = false;
  const live = (run: () => unknown): Promise<unknown> => tornDown
    ? Promise.reject(new Error("session-lifecycle harness used after its test was torn down"))
    : Promise.resolve(run());
  extensions.push({
    shutdown: () => {
      tornDown = true;
      return Promise.resolve(handlers.get("session_shutdown")!({}, context));
    },
  });
  return {
    context, branch, messages, entries, entriesBySession, daemon,
    emit: (name: string, event: unknown, ctx = context) => {
      if (name === "session_start" || name === "session_switch" || name === "session_branch") liveSession = ctx.sessionManager.getSessionId();
      return live(() => handlers.get(name)!(event, ctx));
    },
    reload: () => live(() => commands.get("cosine-rules")!.handler("reload", context)),
  };
}

function delta(text: string, responseId = "response") {
  return {
    type: "message_update",
    message: { role: "assistant", timestamp: 1, responseId },
    assistantMessageEvent: { type: "text_delta", delta: text },
  };
}

describe.each(["omp", "pi"] as const)("%s transactional attachment", host => {
  it("reports startup failure to racing events, steers no later delta of the failed attach, and attaches on retry", async () => {
    const extension = await harness(host);
    const entered = Promise.withResolvers<void>();
    // The session start never succeeds: the test rejects it once events are racing it.
    const release = Promise.withResolvers<never>();
    const failure = new Error("Session store unavailable");
    vi.spyOn(RerailRuntime.prototype, "openSession").mockImplementationOnce(async () => {
      entered.resolve();
      return await release.promise;
    });
    const starting = extension.emit("session_start", {});
    const failedStart = expect(starting).rejects.toThrow(failure.message);
    try {
      await entered.promise;
      const failedDelta = expect(extension.emit("message_update", delta("Guest checkout remains available. Next"))).rejects.toThrow(failure.message);
      release.reject(failure);
      await Promise.all([failedStart, failedDelta]);
      await extension.emit("message_update", delta("Guest checkout remains available. Next"));
      expect(extension.messages).toEqual([]);

      await extension.emit("session_start", {});
      await extension.emit("message_update", delta("Guest checkout remains available. Next"));
      expect(extension.messages).toEqual([
        expect.objectContaining({ content: await checkoutSteer(extension.context.cwd), details: expect.objectContaining({ sessionId: "original" }) }),
      ]);
    } finally {
      await extension.emit("session_shutdown", {});
    }
  });

  it("preserves the prior stream and turn state when a same-session reload fails", async () => {
    const extension = await harness(host);
    await extension.emit("session_start", {});
    try {
      await extension.emit("message_update", delta("Guest checkout "));
      extension.branch.push({ type: "custom", id: "restored-turn", timestamp: new Date(0).toISOString(), customType: TURN_ENTRY_TYPE, data: { turn: 19 } });
      const failure = new Error("Session reload unavailable");
      vi.spyOn(RerailRuntime.prototype, "openSession").mockRejectedValueOnce(failure);
      await expect(extension.reload()).rejects.toThrow(failure.message);

      await extension.emit("message_update", delta("remains available. Next"));
      expect(extension.messages).toEqual([
        expect.objectContaining({ content: await checkoutSteer(extension.context.cwd), details: expect.objectContaining({ sessionId: "original" }) }),
      ]);
      expect(extension.entries.filter(entry => entry.customType === MATCH_ENTRY_TYPE)).toEqual([
        expect.objectContaining({ data: expect.objectContaining({ ruleId: "checkout", turn: 0 }) }),
      ]);

      extension.branch.length = 0;
      await extension.reload();
      await extension.emit("message_update", delta("Guest checkout remains available. Next", "after-retry"));
      expect(extension.messages).toHaveLength(2);
    } finally {
      await extension.emit("session_shutdown", {});
    }
  });

  it("serializes a queued session switch behind a failed startup before releasing waiting deltas", async () => {
    const extension = await harness(host);
    const entered = Promise.withResolvers<void>();
    const release = Promise.withResolvers<never>();
    const failure = new Error("First session store unavailable");
    vi.spyOn(RerailRuntime.prototype, "openSession").mockImplementationOnce(async () => {
      entered.resolve();
      return await release.promise;
    });
    const starting = extension.emit("session_start", {});
    const failedStart = expect(starting).rejects.toThrow(failure.message);
    try {
      await entered.promise;
      const replacement = {
        ...extension.context,
        sessionManager: { getSessionId: () => "replacement", getBranch: () => [] },
      } as unknown as Context;
      const switching = extension.emit("session_start", {}, replacement);
      const streaming = extension.emit("message_update", delta("Guest checkout remains available. Next"), replacement);
      release.reject(failure);
      await Promise.all([failedStart, switching, streaming]);
      expect(extension.messages).toEqual([
        expect.objectContaining({ content: await checkoutSteer(extension.context.cwd), details: expect.objectContaining({ sessionId: "replacement" }) }),
      ]);
    } finally {
      await extension.emit("session_shutdown", {});
    }
  });
});

it("counts events that arrive while attach is failing and records them once attached", async () => {
  const extension = await harness("omp");
  const failure = new Error("Session store unavailable");
  vi.spyOn(RerailRuntime.prototype, "openSession").mockRejectedValueOnce(failure);
  const reader = new SqliteSessionReader(defaultRerailDatabasePath());
  try {
    await expect(extension.emit("session_start", {})).rejects.toThrow(failure.message);
    // The attach retries in the background; until it lands, events resolve without reaching Rerail and are counted.
    await extension.emit("message_update", delta("Guest checkout remains available. Next"));
    await extension.emit("tool_call", { toolCallId: "edit-1", toolName: "edit", input: { path: "a.ts" } });
    await extension.emit("tool_result", { toolCallId: "edit-1", toolName: "edit", content: [], details: {}, isError: false });
    await extension.emit("turn_end", { message: { role: "assistant", timestamp: 2, responseId: "response", content: [] } });
    expect(extension.messages).toEqual([]);
    expect(extension.entries.filter(entry => entry.customType === EVENTS_DROPPED_ENTRY_TYPE)).toEqual([
      { customType: EVENTS_DROPPED_ENTRY_TYPE, data: expect.objectContaining({ count: 1, kinds: { "assistant-delta": 1 }, recovered: false }) },
    ]);

    const gap = expect.objectContaining({
      count: 4,
      kinds: { "assistant-delta": 1, call: 1, result: 1, "turn-end": 1 },
      lastEventId: "turn-end:2:response",
    });
    await vi.waitFor(() => {
      expect(reader.events("original").filter(event => event.kind === "events-dropped").map(event => event.payload)).toEqual([gap]);
    }, { timeout: 15_000, interval: 100 });
    expect(extension.entries.filter(entry => entry.customType === EVENTS_DROPPED_ENTRY_TYPE).map(entry => entry.data)).toEqual([
      expect.objectContaining({ recovered: false }),
      expect.objectContaining({ count: 4, recovered: true }),
    ]);
    // Attached, the session takes events again.
    await extension.emit("message_update", delta("Guest checkout remains available. Next", "after-attach"));
    expect(extension.messages).toEqual([
      expect.objectContaining({ content: await checkoutSteer(extension.context.cwd), details: expect.objectContaining({ sessionId: "original" }) }),
    ]);
  } finally {
    reader.close();
    await extension.emit("session_shutdown", {});
  }
}, 30_000);

it("counts an event that waited on the attach that failed, and records it once a retry attaches", async () => {
  const extension = await harness("omp");
  const entered = Promise.withResolvers<void>();
  // The session start fails only once an event is already waiting on it.
  const release = Promise.withResolvers<never>();
  const failure = new Error("Session store unavailable");
  vi.spyOn(RerailRuntime.prototype, "openSession").mockImplementationOnce(async () => {
    entered.resolve();
    return await release.promise;
  });
  const reader = new SqliteSessionReader(defaultRerailDatabasePath());
  try {
    const starting = expect(extension.emit("session_start", {})).rejects.toThrow(failure.message);
    await entered.promise;
    const racing = expect(extension.emit("message_update", delta("Guest checkout remains available. Next"))).rejects.toThrow(failure.message);
    release.reject(failure);
    await Promise.all([starting, racing]);
    expect(extension.entries.filter(entry => entry.customType === EVENTS_DROPPED_ENTRY_TYPE).map(entry => entry.data)).toEqual([
      expect.objectContaining({ count: 1, kinds: { "assistant-delta": 1 }, recovered: false }),
    ]);
    await vi.waitFor(() => {
      expect(reader.events("original").filter(event => event.kind === "events-dropped").map(event => event.payload))
        .toEqual([expect.objectContaining({ count: 1, kinds: { "assistant-delta": 1 } })]);
    }, { timeout: 15_000, interval: 100 });
  } finally {
    reader.close();
    await extension.emit("session_shutdown", {});
  }
}, 30_000);

it("stores the whole assistant message when only some of its deltas reached the daemon", async () => {
  const extension = await harness("omp");
  const stderr = vi.spyOn(process.stderr, "write").mockReturnValue(true);
  const reader = new SqliteSessionReader(defaultRerailDatabasePath());
  try {
    await extension.emit("session_start", {});
    await extension.emit("message_update", delta("Guest checkout "));
    // The daemon refuses the session's events, so the breaker opens and counts the second delta.
    const ingest = vi.spyOn(RerailRuntime.prototype, "ingest").mockRejectedValue(new Error("Daemon unavailable"));
    await extension.emit("message_update", delta("remains available."));
    ingest.mockRestore();
    await vi.waitFor(() => expect(stderr).toHaveBeenCalledWith(expect.stringContaining("recovered; 1 session event(s)")), { timeout: 15_000, interval: 100 });

    const message = { role: "assistant", timestamp: 1, responseId: "response", content: [{ type: "text", text: "Guest checkout remains available." }], stopReason: "stop" };
    await extension.emit("message_end", { message });
    expect(reader.events("original").filter(event => event.kind === "assistant-message").map(event => event.payload))
      .toEqual([expect.objectContaining({ text: "Guest checkout remains available." })]);
  } finally {
    reader.close();
    await extension.emit("session_shutdown", {});
  }
}, 30_000);

it("stores the whole assistant message when a daemon restart replaces the runtime between its deltas", async () => {
  const extension = await harness("omp");
  const stderr = vi.spyOn(process.stderr, "write").mockReturnValue(true);
  const reader = new SqliteSessionReader(defaultRerailDatabasePath());
  try {
    await extension.emit("session_start", {});
    await extension.emit("message_update", delta("Guest checkout "));
    // The daemon restarts on its port with a new native token: the runtime's token is refused, so it asks the host for a rebuild.
    const restarted = extension.daemon!.port;
    await extension.daemon!.close();
    await startNativeTestDaemon(extension.context.cwd, restarted);
    await extension.emit("message_update", delta("remains "));
    await vi.waitFor(() => expect(stderr).toHaveBeenCalledWith(expect.stringContaining("recovered")), { timeout: 15_000, interval: 100 });
    // The rebuilt runtime takes this delta: it holds a part of the message that the daemon session never saw the start of.
    await extension.emit("message_update", delta("available."));

    const message = { role: "assistant", timestamp: 1, responseId: "response", content: [{ type: "text", text: "Guest checkout remains available." }], stopReason: "stop" };
    await extension.emit("message_end", { message });
    expect(reader.events("original").filter(event => event.kind === "assistant-message").map(event => event.payload))
      .toEqual([expect.objectContaining({ text: "Guest checkout remains available." })]);
  } finally {
    reader.close();
    await extension.emit("session_shutdown", {});
  }
}, 30_000);

it("gives the marker of a session that never attached its final count when the host shuts down", async () => {
  const extension = await harness("omp");
  const failure = new Error("Session store unavailable");
  vi.spyOn(RerailRuntime.prototype, "openSession").mockRejectedValue(failure);
  await expect(extension.emit("session_start", {})).rejects.toThrow(failure.message);
  await extension.emit("message_update", delta("Guest checkout remains available. Next"));
  await extension.emit("turn_end", { message: { role: "assistant", timestamp: 2, responseId: "response", content: [] } });
  await extension.emit("session_shutdown", {});

  expect(extension.entries.filter(entry => entry.customType === EVENTS_DROPPED_ENTRY_TYPE).map(entry => entry.data)).toEqual([
    expect.objectContaining({ count: 1, kinds: { "assistant-delta": 1 }, recovered: false }),
    expect.objectContaining({ count: 2, kinds: { "assistant-delta": 1, "turn-end": 1 }, lastEventId: "turn-end:2:response", recovered: false }),
  ]);
});

it("gives the marker of a session that never attached its final count before the host switches away from it", async () => {
  const extension = await harness("omp");
  const failure = new Error("Session store unavailable");
  vi.spyOn(RerailRuntime.prototype, "openSession").mockRejectedValue(failure);
  const replacement = {
    ...extension.context,
    sessionManager: { getSessionId: () => "replacement", getBranch: () => [] },
  } as unknown as Context;
  try {
    await expect(extension.emit("session_start", {})).rejects.toThrow(failure.message);
    await extension.emit("message_update", delta("Guest checkout remains available. Next"));
    await extension.emit("turn_end", { message: { role: "assistant", timestamp: 2, responseId: "response", content: [] } });
    // The host asks before it leaves the session, while appending still lands in that session.
    await extension.emit("session_before_switch", { reason: "new" });
    const final = expect.objectContaining({ count: 2, kinds: { "assistant-delta": 1, "turn-end": 1 }, recovered: false });
    expect(extension.entries.filter(entry => entry.customType === EVENTS_DROPPED_ENTRY_TYPE).map(entry => entry.data)).toEqual([
      expect.objectContaining({ count: 1, recovered: false }),
      final,
    ]);
    // The replacement session fails to attach as well: the original session's drops are discarded with no more markers.
    await expect(extension.emit("session_switch", { reason: "new" }, replacement)).rejects.toThrow(failure.message);
    expect(extension.entries.filter(entry => entry.customType === EVENTS_DROPPED_ENTRY_TYPE).map(entry => entry.data)).toEqual([
      expect.objectContaining({ count: 1, recovered: false }),
      final,
    ]);
  } finally {
    await extension.emit("session_shutdown", {}, replacement);
  }
});

it("keeps a closing runtime's final gap marker out of the session the host switched to", async () => {
  const extension = await harness("omp");
  const replacement = {
    ...extension.context,
    sessionManager: { getSessionId: () => "replacement", getBranch: () => [] },
  } as unknown as Context;
  const reader = new SqliteSessionReader(defaultRerailDatabasePath());
  try {
    await extension.emit("session_start", {});
    // The daemon refuses the session's events (not its start), so the attached runtime's gap opens and stays open.
    const ingest = RerailRuntime.prototype.ingest;
    vi.spyOn(RerailRuntime.prototype, "ingest").mockImplementation(function (this: RerailRuntime, event) {
      return event.kind === "session-start" ? ingest.call(this, event) : Promise.reject(new Error("Daemon unavailable"));
    });
    await extension.emit("message_update", delta("Guest checkout remains available. Next"));
    await extension.emit("turn_end", { message: { role: "assistant", timestamp: 2, responseId: "response", content: [] } });
    await extension.emit("session_before_switch", { reason: "new" });
    // The host switches before it retires the old runtime, which writes its closing marker.
    await extension.emit("session_switch", { reason: "new" }, replacement);

    const markers = (session: string) => (extension.entriesBySession.get(session) ?? [])
      .filter(entry => entry.customType === EVENTS_DROPPED_ENTRY_TYPE).map(entry => entry.data);
    expect(markers("original").at(-1)).toEqual(expect.objectContaining({ count: 2, recovered: false }));
    expect(markers("replacement")).toEqual([]);
    await vi.waitFor(() => {
      expect(reader.events("original").filter(event => event.kind === "events-dropped").map(event => event.payload))
        .toEqual([expect.objectContaining({ count: 2 })]);
    }, { timeout: 15_000, interval: 100 });
  } finally {
    reader.close();
    await extension.emit("session_shutdown", {}, replacement);
  }
}, 30_000);
