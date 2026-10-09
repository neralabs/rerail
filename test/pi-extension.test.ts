import { EventEmitter } from "node:events";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import type { ChildProcess } from "node:child_process";
import type { ExtensionAPI, ExtensionContext } from "@earendil-works/pi-coding-agent";
import { afterEach, describe, expect, it, vi } from "vitest";

const launcher = vi.hoisted(() => ({ spawn: vi.fn() }));
vi.mock("node:child_process", async () => ({
  ...await vi.importActual<Record<string, unknown>>("node:child_process"),
  spawn: launcher.spawn,
}));
/** A main build's Pi goes through the daemon; tests that set `main` stand in for that daemon. */
const daemonRoute = vi.hoisted(() => ({ main: undefined as boolean | undefined, startDaemonForPackage: vi.fn() }));
vi.mock("../src/rerail-build.js", async () => {
  const actual = await vi.importActual<typeof import("../src/rerail-build.js")>("../src/rerail-build.js");
  return { ...actual, isMainRerailBuild: () => daemonRoute.main ?? actual.isMainRerailBuild() };
});
vi.mock("../src/daemon-control.js", async () => ({
  ...await vi.importActual<Record<string, unknown>>("../src/daemon-control.js"),
  startDaemonForPackage: daemonRoute.startDaemonForPackage,
}));
import Database from "better-sqlite3";
import { RULE_MESSAGE_TYPE } from "../src/adapter-contract.js";
import { defaultRerailDatabasePath, openRerailDatabase, resetRepositoryHistory } from "../src/database.js";
import { resolveRepositoryContext } from "../src/scoped-rule-catalog.js";
import { createPiConversationRulesExtension } from "../src/pi-extension.js";
import { SqliteAuditControlStore } from "../src/audit-control.js";
import { steerBlock } from "../src/rerail-steer.js";
import { MATCH_ENTRY_TYPE, TURN_ENTRY_TYPE } from "../src/session-state.js";
import { SqliteSessionStore } from "../src/session-store.js";
import type { ConversationRule } from "../src/rule-schema.js";
import type { HostCommandContext } from "../src/host-commands.js";
import { startTestRepository, stopTestRepository } from "./support/started-repository.js";

// Every test here builds real sessions (native model, engine, rule catalog); on a loaded CI runner a
// build alone can pass the 5 s default, whichever test happens to pay the cold start.
vi.setConfig({ testTimeout: 30_000 });
type Handler = (event: unknown, context: ExtensionContext) => unknown;
type CommandHandler = (args: string, context: HostCommandContext) => Promise<void>;
interface PiHarness {
  handlers: Map<string, Handler>;
  commands: Map<string, CommandHandler>;
  messages: Array<{ message: unknown; options: unknown }>;
  entries: Array<{ customType: string; data: unknown }>;
  pi: ExtensionAPI;
  context: ExtensionContext;
}
const temporaryDirectories: string[] = [];

/** Real timer captured before any fake-timer install: lets pending file I/O settle under fake timers. */
const realSetTimeout = setTimeout;
const settleIo = (): Promise<void> => {
  const { promise, resolve: done } = Promise.withResolvers<void>();
  realSetTimeout(done, 50);
  return promise;
};

const originalRerailHome = process.env.RERAIL_HOME;
/** Every harness is shut down before its fetch stub goes: a live client left polling would bleed its close into the next test. */
const shutdowns: Array<() => Promise<unknown>> = [];

afterEach(async () => {
  for (const shutdown of shutdowns.splice(0)) await shutdown();
  daemonRoute.main = undefined;
  daemonRoute.startDaemonForPackage.mockReset();
  vi.unstubAllGlobals();
  vi.useRealTimers();
  if (originalRerailHome === undefined) delete process.env.RERAIL_HOME;
  else process.env.RERAIL_HOME = originalRerailHome;
  for (const path of temporaryDirectories.splice(0)) rmSync(path, { recursive: true, force: true });
});

interface RegisteredTool {
  name: string;
  execute(id: string, input: unknown): Promise<unknown>;
}

function createHarness(branch: unknown[] = []): PiHarness {
  const cwd = mkdtempSync(join(tmpdir(), "rerail-pi-extension-"));
  process.env.RERAIL_HOME = join(cwd, "home");
  temporaryDirectories.push(cwd);
  startTestRepository(cwd);
  const handlers = new Map<string, Handler>();
  const commands = new Map<string, CommandHandler>();
  const messages: Array<{ message: unknown; options: unknown }> = [];
  const entries: Array<{ customType: string; data: unknown }> = [];
  const pi = {
    on(event: string, handler: Handler) {
      handlers.set(event, handler);
    },
    sendMessage(message: unknown, options: unknown) {
      messages.push({ message, options });
    },
    appendEntry(customType: string, data: unknown) {
      entries.push({ customType, data });
    },
    registerTool() {},
    registerCommand(name: string, options: { handler: CommandHandler }) {
      commands.set(name, options.handler);
    },
  } as unknown as ExtensionAPI;
  const context = {
    cwd,
    sessionManager: { getSessionId: () => "pi-test-session", getBranch: () => branch },
  } as unknown as ExtensionContext;
  shutdowns.push(async () => handlers.get("session_shutdown")?.({ type: "session_shutdown", reason: "quit" }, context));
  return { handlers, commands, messages, entries, pi, context };
}

function receiptCount(harness: PiHarness, sessionId = "pi-test-session"): number {
  const database = new Database(defaultRerailDatabasePath(), { readonly: true });
  try {
    const row = database.prepare<[string], { total: number }>("SELECT COUNT(*) AS total FROM events WHERE kind = 'steering-delivered' AND session_id = ?").get(sessionId);
    return row!.total;
  } finally {
    database.close();
  }
}

const messageRule: ConversationRule = {
  id: "checkout",
  mode: "steer",
  reference: "Guest checkout remains available.",
  match: { operator: "gte", threshold: 0.99 },
  sample: { source: "assistant-text", boundary: "message", minimumTokens: 1 },
  action: { content: "Check checkout requirements." },
  repeat: { mode: "always" },
};

function install(harness: PiHarness, rules: readonly ConversationRule[] = [messageRule]) {
  createPiConversationRulesExtension({
    modelPath: resolve("model"),
    rules,
    registerTool: false,
    registerCommand: false,
  })(harness.pi);
  return harness.handlers.get("session_start")?.({ type: "session_start", reason: "startup" }, harness.context);
}

describe("Pi conversation-rules extension", () => {
  it.each(["steer", "block"] as const)("evaluates a finalized assistant message when Pi emits no text deltas, and ignores a replayed end event (%s)", async mode => {
    const harness = createHarness();
    await install(harness, [{ ...messageRule, mode }]);
    const event = {
      type: "message_end",
      message: {
        role: "assistant",
        timestamp: 100,
        responseId: "response-1",
        content: [{ type: "text", text: "Guest checkout remains available." }],
      },
    };

    await harness.handlers.get("message_end")?.(event, harness.context);
    await harness.handlers.get("message_end")?.(event, harness.context);

    expect(harness.messages).toEqual([
      {
        message: {
          customType: RULE_MESSAGE_TYPE,
          content: steerBlock("checkout", "Check checkout requirements."),
          display: true,
          details: { requested: mode, mechanism: "native-queue", sessionId: expect.any(String) },
        },
        options: { deliverAs: "steer" },
      },
    ]);
    expect(harness.entries.filter((entry) => entry.customType === MATCH_ENTRY_TYPE)).toHaveLength(1);
    expect(receiptCount(harness)).toBe(1);
  });

  it("says Rerail is not on here in a stopped checkout, without asking a daemon", async () => {
    const harness = createHarness();
    stopTestRepository(harness.context.cwd);
    const widgets: Array<string[] | undefined> = [];
    const fetchSpy = vi.fn(async () => { throw new Error("no daemon may be asked"); });
    vi.stubGlobal("fetch", fetchSpy);
    try {
      Object.assign(harness.context, { hasUI: true, ui: { setWidget: (_key: string, content: string[] | undefined) => { widgets.push(content); } } });
      await install(harness);
      expect(widgets.at(-1)?.map(line => line.replace(/\u001b\[[0-9;]*m|\u001b\]8;;[^\u0007]*\u0007/gu, ""))).toEqual(["Rerail ○ not on here"]);
      expect(fetchSpy).not.toHaveBeenCalled();
    } finally {
      vi.unstubAllGlobals();
    }
  });

  it.each([
    { rejected: false, lifecycle: "active" },
    { rejected: true, lifecycle: "active" },
    { rejected: false, lifecycle: "closed" },
    { rejected: false, lifecycle: "switched" },
  ])("records native completion against its originating session (rejected: $rejected, lifecycle: $lifecycle)", async ({ rejected, lifecycle }) => {
    const harness = createHarness();
    const sending = Promise.withResolvers<void>();
    const entered = Promise.withResolvers<void>();
    harness.pi.sendMessage = async () => {
      entered.resolve();
      await sending.promise;
    };
    await install(harness);
    const handled = harness.handlers.get("message_end")?.({
      type: "message_end",
      message: {
        role: "assistant", timestamp: 100, responseId: "response-1",
        content: [{ type: "text", text: "Guest checkout remains available." }],
      },
    }, harness.context);
    await entered.promise;
    expect(receiptCount(harness)).toBe(0);
    if (lifecycle === "closed") await harness.handlers.get("session_shutdown")?.({ type: "session_shutdown" }, harness.context);
    if (lifecycle === "switched") {
      const next = { ...harness.context, sessionManager: { ...harness.context.sessionManager, getSessionId: () => "pi-next-session" } } as ExtensionContext;
      await harness.handlers.get("session_start")?.({ type: "session_start", reason: "switch" }, next);
      expect(receiptCount(harness, "pi-next-session")).toBe(0);
    }
    if (rejected) {
      sending.reject(new Error("host rejected guidance"));
      await expect(handled).rejects.toThrow("host rejected guidance");
    } else {
      sending.resolve();
      await handled;
    }
    expect(receiptCount(harness)).toBe(rejected ? 0 : 1);
    if (lifecycle === "switched") expect(receiptCount(harness, "pi-next-session")).toBe(0);
  });

  it("hands off remaining native guidance when an earlier receipt cannot be stored", async () => {
    const harness = createHarness();
    await install(harness, [messageRule, { ...messageRule, id: "second", action: { content: "Second guidance." } }]);
    const receipt = vi.spyOn(SqliteSessionStore.prototype, "recordSteer").mockImplementationOnce(() => { throw new Error("receipt unavailable"); });
    const diagnostic = vi.spyOn(console, "error").mockImplementation(() => undefined);
    try {
      await harness.handlers.get("message_end")?.({
        type: "message_end",
        message: { role: "assistant", timestamp: 100, responseId: "response-1", content: [{ type: "text", text: "Guest checkout remains available." }] },
      }, harness.context);
      expect(harness.messages).toHaveLength(2);
      expect(receiptCount(harness)).toBe(1);
    } finally {
      receipt.mockRestore();
      diagnostic.mockRestore();
    }
  });

  it("does not send or count log-only guidance", async () => {
    const harness = createHarness();
    await install(harness, [{ ...messageRule, mode: "log", action: { content: "For observation only." } }]);
    await harness.handlers.get("message_end")?.({
      type: "message_end",
      message: {
        role: "assistant", timestamp: 100, responseId: "response-1",
        content: [{ type: "text", text: "Guest checkout remains available." }],
      },
    }, harness.context);
    expect(harness.messages).toEqual([]);
    expect(receiptCount(harness)).toBe(0);
  });

  it("records a turn once when Pi replays its turn_end lifecycle event", async () => {
    const harness = createHarness();
    await install(harness);
    const event = {
      type: "turn_end",
      turnIndex: 0,
      message: { role: "assistant", timestamp: 101, responseId: "response-2", content: [] },
      toolResults: [],
    };

    await harness.handlers.get("turn_end")?.(event, harness.context);
    await harness.handlers.get("turn_end")?.(event, harness.context);

    expect(harness.entries.filter((entry) => entry.customType === TURN_ENTRY_TYPE)).toEqual([
      { customType: TURN_ENTRY_TYPE, data: { turn: 1 } },
    ]);
  });

  it("persists a replayed Pi turn once with its stable message identifier", async () => {
    const harness = createHarness();
    await install(harness);
    const event = {
      type: "turn_end",
      turnIndex: 0,
      message: { role: "assistant", timestamp: 101, responseId: "response-2", content: [] },
      toolResults: [],
    };

    await harness.handlers.get("turn_end")?.(event, harness.context);
    await harness.handlers.get("turn_end")?.(event, harness.context);
    await Promise.all([
      harness.handlers.get("tool_result")?.(
        {
          type: "tool_result",
          toolCallId: "call-1",
          toolName: "read",
          output: "first",
        },
        harness.context,
      ),
      harness.handlers.get("tool_result")?.(
        {
          type: "tool_result",
          toolCallId: "call-2",
          toolName: "read",
          output: "second",
        },
        harness.context,
      ),
    ]);
    await harness.handlers.get("session_shutdown")?.({ type: "session_shutdown", reason: "quit" }, harness.context);

    const database = new Database(defaultRerailDatabasePath(), { readonly: true });
    expect(database.prepare(
      "SELECT event_id AS eventId FROM events WHERE kind IN ('turn-end', 'result') ORDER BY sequence",
    ).all()).toEqual([
      { eventId: "turn-end:101:response-2" },
      { eventId: "tool-result:call-1" },
      { eventId: "tool-result:call-2" },
    ]);
    database.close();
  });

  it("restores persisted once-only delivery state on a new Pi extension runtime", async () => {
    const harness = createHarness([
      { type: "custom", customType: MATCH_ENTRY_TYPE, data: { ruleId: "checkout", turn: 3 } },
      { type: "custom", customType: TURN_ENTRY_TYPE, data: { turn: 3 } },
    ]);
    createPiConversationRulesExtension({
      modelPath: resolve("model"),
      rules: [{ ...messageRule, repeat: { mode: "once" } }],
      registerTool: false,
      registerCommand: false,
    })(harness.pi);
    await harness.handlers.get("session_start")?.({ type: "session_start", reason: "resume" }, harness.context);

    await harness.handlers.get("message_end")?.(
      {
        type: "message_end",
        message: {
          role: "assistant",
          timestamp: 102,
          responseId: "response-3",
          content: [{ type: "text", text: "Guest checkout remains available." }],
        },
      },
      harness.context,
    );

    expect(harness.messages).toEqual([]);
    expect(harness.entries.filter((entry) => entry.customType === MATCH_ENTRY_TYPE)).toEqual([]);
  });
  it("contains malformed call inputs without recording a sample", async () => {
    const harness = createHarness();
    const toolRule: ConversationRule = {
      ...messageRule,
      id: "tool-input",
      reference: '{"a":"Guest checkout remains available.","z":1}',
      mode: "log",
      sample: { source: "tool:submit", boundary: "tool-call", minimumTokens: 1 },
      action: { content: "Observe the tool." },
    };
    await install(harness, [toolRule]);
    const circular: { self?: unknown } = {};
    circular.self = circular;

    await harness.handlers.get("tool_call")?.(
      {
        type: "tool_call",
        toolName: "submit",
        input: { z: 1, a: "Guest checkout remains available." },
      },
      harness.context,
    );
    expect(harness.entries.filter((entry) => entry.customType === MATCH_ENTRY_TYPE)).toHaveLength(1);

    const call = harness.handlers.get("tool_call");
    if (call === undefined) throw new Error("Pi extension did not register tool_call handler");
    await expect(Promise.resolve(call({ type: "tool_call", toolName: "submit", input: circular }, harness.context))).resolves.toBeUndefined();
    await expect(
      Promise.resolve(call({ type: "tool_call", toolName: "submit", input: { checkout: 1n } }, harness.context)),
    ).resolves.toBeUndefined();
    expect(harness.entries.filter((entry) => entry.customType === MATCH_ENTRY_TYPE)).toHaveLength(1);

    await harness.handlers.get("session_shutdown")?.({ type: "session_shutdown", reason: "quit" }, harness.context);
    const input = harness.handlers.get("input");
    if (input === undefined) throw new Error("Pi extension did not register input handler");
    await expect(Promise.resolve(input({ type: "input", text: "still active" }, harness.context))).rejects.toThrow(
      "before session_start",
    );
  });


  it("drops a breaker-driven reattach that fired while a newer session was attaching", async () => {
    const harness = createHarness();
    daemonRoute.main = true;
    daemonRoute.startDaemonForPackage.mockResolvedValue({
      started: false,
      daemon: { state: { url: "http://127.0.0.1:4737/", nativeToken: "native-token" }, health: { workspaceRoot: harness.context.cwd } },
    });
    // The daemon can refuse the old session, and its next initialize can be held mid-flight.
    const state = { down: new Set<string>(), hold: undefined as PromiseWithResolvers<void> | undefined };
    const requests: Array<{ operation: string; sessionId: string }> = [];
    vi.stubGlobal("fetch", async (_url: URL, init: { body: string }) => {
      const request = JSON.parse(init.body) as { operation: string; sessionId: string };
      requests.push({ operation: request.operation, sessionId: request.sessionId });
      if (request.operation === "initialize" && state.hold !== undefined) {
        const { promise } = state.hold;
        state.hold = undefined;
        await promise;
      }
      if (state.down.has(request.sessionId)) throw new TypeError("fetch failed");
      return Response.json({ result: null, deliveries: [], entries: [] });
    });
    const initializes = (sessionId: string) => requests.filter(r => r.operation === "initialize" && r.sessionId === sessionId).length;
    vi.useFakeTimers();
    vi.spyOn(process.stderr, "write").mockReturnValue(true);
    await install(harness);
    const next = { ...harness.context, sessionManager: { getSessionId: () => "pi-next-session", getBranch: () => [] } } as unknown as ExtensionContext;
    expect(initializes("pi-test-session")).toBe(1);

    // Three failed polls (100, then 100 and 200 of backoff) put the breaker one 400ms wait from asking the host to rebuild.
    state.down.add("pi-test-session");
    await vi.advanceTimersByTimeAsync(400);
    expect(requests.filter(r => r.operation === "poll")).toHaveLength(3);

    // The switch's initialize is held open when the old session's rebuild fires; the daemon has healed by then.
    const held = Promise.withResolvers<void>();
    state.hold = held;
    const switching = harness.handlers.get("session_start")!({ type: "session_start", reason: "resume" }, next) as Promise<void>;
    await vi.waitFor(() => expect(initializes("pi-next-session")).toBe(1));
    state.down.delete("pi-test-session");
    await vi.advanceTimersByTimeAsync(400);
    held.resolve();
    await expect(switching).resolves.toBeUndefined();
    await settleIo();

    expect(initializes("pi-test-session")).toBe(1);
    await harness.handlers.get("input")!({ type: "input", text: "hello" }, next);
    expect(requests.at(-1)).toEqual({ operation: "ingest", sessionId: "pi-next-session" });
    vi.useRealTimers();
  });
});
