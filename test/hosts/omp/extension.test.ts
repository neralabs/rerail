import { existsSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
/** Attach-retry tests replace daemon discovery, which polls on real timers, with a daemon that is already up. */
const daemonRoute = vi.hoisted(() => ({ start: undefined as (() => Promise<unknown>) | undefined }));
vi.mock("../../../src/runtime/daemon-control.js", async () => {
  const actual = await vi.importActual<typeof import("../../../src/runtime/daemon-control.js")>("../../../src/runtime/daemon-control.js");
  return {
    ...actual,
    startDaemonForPackage: (...args: Parameters<typeof actual.startDaemonForPackage>) =>
      daemonRoute.start?.() ?? actual.startDaemonForPackage(...args),
  };
});
import Database from "better-sqlite3";
import type { CustomMessagePayload, ExtensionAPI, ExtensionContext } from "@oh-my-pi/pi-coding-agent";
import { z } from "zod";
import {
  RULE_MESSAGE_TYPE,
  STEER_LINE_MESSAGE_TYPE,
  defaultRerailDatabasePath,
  resetRepositoryHistory,
  steerBlock,
  steerOrigin,
  type ConversationRule,
  type RuleMode,
  type AdapterDelivery,
  type SteerRule,
  openRerailDatabase,
  setRuleSetting,
  CodeRuleBridge,
} from "../../../src/engine/index.js";
import { createConversationRulesExtension } from "../../../src/hosts/omp/extension.js";
import { NativeSessionClient } from "../../../src/harness/index.js";
import { EVENTS_DROPPED_ENTRY_TYPE, SqliteSessionReader, isRerailStarted } from "../../../src/runtime/index.js";
import { steerDashboard } from "../../../src/presentation/index.js";
import { closeNativeTestDaemons, startNativeTestDaemon } from "../../support/native-daemon.js";
import { setUpTestRepository } from "../../support/repository-rules.js";
import { startTestRepository } from "../../support/started-repository.js";

/** The steer block a `mode` conversation rule delivers for a step of the conversation. */
function deliveredSteer(ruleId: string, guidance: string, mode: RuleMode = "steer"): string {
  return steerBlock({ kind: "tool" }, [{ ruleId, mode, message: guidance }]);
}

/** The rules a `mode` rule's steer names, linked the way the OMP adapter links them for `cwd`. */
async function deliveredRules(cwd: string, ruleId: string, mode: RuleMode = "steer"): Promise<SteerRule[]> {
  const dashboard = await steerDashboard(cwd);
  const origin = steerOrigin(ruleId);
  return [{
    id: ruleId,
    state: mode === "log" ? "logged" : "steered",
    ...(origin === undefined ? {} : { origin }),
    ...(dashboard === undefined ? {} : { url: `${dashboard}#rule/${encodeURIComponent(ruleId)}` }),
  }];
}

// Every test here builds real sessions (native model, engine, rule catalog); on a loaded CI runner a
// build alone can pass the 5 s default, whichever test happens to pay the cold start.
vi.setConfig({ testTimeout: 30_000 });

type Handler = (event: unknown, context: ExtensionContext) => unknown;
type CommandHandler = (args: string, context: { ui: { notify(message: string, level: string): void } }) => Promise<void>;
const temporaryDirectories: string[] = [];
/** Real timer captured before any fake-timer install: lets pending file I/O settle under fake timers. */
const realSetTimeout = setTimeout;
const settleIo = (): Promise<void> => {
  const { promise, resolve: done } = Promise.withResolvers<void>();
  realSetTimeout(done, 50);
  return promise;
};

const originalRerailHome = process.env.RERAIL_HOME;
const shutdowns: Array<() => Promise<unknown>> = [];

afterEach(async () => {
  for (const shutdown of shutdowns.splice(0)) await shutdown();
  daemonRoute.start = undefined;
  await closeNativeTestDaemons();
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

/**
 * A checkout whose repository is started and set up with `rules` added, unless `started` is false. Only rules a
 * repository adds run.
 */
async function createHarness({ started = true, rules = [] }: { started?: boolean; rules?: readonly ConversationRule[] } = {}) {
  const cwd = mkdtempSync(join(tmpdir(), "rerail-omp-extension-"));
  process.env.RERAIL_HOME = join(cwd, "home");
  temporaryDirectories.push(cwd);
  const repositoryKey = started ? setUpTestRepository(cwd, { library: rules.map(rule => ({ kind: "conversation" as const, rule })) }) : undefined;
  await startNativeTestDaemon(cwd);
  const handlers = new Map<string, Handler>();
  const commands = new Map<string, CommandHandler>();
  const messages: Array<{ message: Exclude<CustomMessagePayload, string>; options: unknown }> = [];
  const entries: Array<{ customType: string; data: unknown }> = [];
  const pi = {
    zod: z,
    on(event: string, handler: Handler) {
      handlers.set(event, handler);
    },
    sendMessage(message: Exclude<CustomMessagePayload, string>, options: unknown) {
      messages.push({ message, options });
    },
    appendEntry(customType: string, data: unknown) {
      entries.push({ customType, data });
    },
    setLabel() {},
    registerTool() {},
    registerCommand(name: string, options: { handler: CommandHandler }) {
      commands.set(name, options.handler);
    },
    registerMessageRenderer() {},
  } as unknown as ExtensionAPI;
  const context = {
    cwd,
    sessionManager: { getSessionId: () => "omp-test-session", getBranch: () => [] },
    isIdle: () => false,
    hasPendingMessages: () => true,
  } as unknown as ExtensionContext;
  shutdowns.push(async () => handlers.get("session_shutdown")?.({}, context));
  return { handlers, commands, messages, entries, pi, context, repositoryKey };
}

function receiptCount(context: ExtensionContext): number {
  const database = new Database(defaultRerailDatabasePath(), { readonly: true });
  try {
    const row = database.prepare<[], { total: number }>("SELECT COUNT(*) AS total FROM events WHERE kind = 'steering-delivered'").get();
    return row!.total;
  } finally {
    database.close();
  }
}

const sentenceRule: ConversationRule = {
  id: "checkout",
  mode: "steer",
  reference: "Guest checkout remains available.",
  match: { operator: "gte", threshold: 0.99 },
  sample: { source: "assistant-text", boundary: "sentence", minimumTokens: 1 },
  action: { content: "Check checkout requirements." },
  repeat: { mode: "once" },
};

describe("OMP conversation-rules extension", () => {
  it.each(["steer", "block"] as const)("steers immediately when a streamed sentence matches a %s rule", async mode => {
    const harness = await createHarness({ rules: [{ ...sentenceRule, mode }] });
    createConversationRulesExtension({
      modelPath: resolve("model"),
      registerTool: false,
      registerCommand: false,
    })(harness.pi);

    await harness.handlers.get("session_start")?.({ type: "session_start" }, harness.context);
    await harness.handlers.get("message_update")?.(
      {
        type: "message_update",
        message: {
          role: "assistant",
          timestamp: 100,
          responseId: "response-1",
        },
        assistantMessageEvent: {
          type: "text_delta",
          delta: "Guest checkout remains available. Next",
        },
      },
      harness.context,
    );

    expect(harness.messages).toEqual([
      {
        message: {
          customType: RULE_MESSAGE_TYPE,
          content: deliveredSteer("checkout", "Check checkout requirements.", mode),
          display: true,
          details: { requested: mode, mechanism: "native-queue", sessionId: "omp-test-session", rules: await deliveredRules(harness.context.cwd, "checkout", mode) },
        },
        options: { deliverAs: "steer" },
      },
    ]);
    expect(harness.entries).toEqual([
      expect.objectContaining({
        customType: "rerail.match",
        data: expect.objectContaining({ ruleId: "checkout", mode }),
      }),
    ]);
    expect(receiptCount(harness.context)).toBe(1);
  });

  it("records accepted guidance delivered while the agent is idle", async () => {
    const idleRule: ConversationRule = { ...sentenceRule, sample: { source: "user-input", boundary: "message" } };
    const harness = await createHarness({ rules: [idleRule] });
    Object.assign(harness.context, {
      isIdle: () => true,
      hasPendingMessages: () => false,
    });
    createConversationRulesExtension({
      modelPath: resolve("model"),
      registerTool: false,
      registerCommand: false,
    })(harness.pi);

    await harness.handlers.get("session_start")?.({ type: "session_start" }, harness.context);
    await harness.handlers.get("before_agent_start")?.({
      type: "before_agent_start",
      prompt: "Guest checkout remains available.",
    }, harness.context);

    expect(harness.messages).toHaveLength(1);
    expect(receiptCount(harness.context)).toBe(1);
  });

  it("does not count a rejected native send", async () => {
    const harness = await createHarness({ rules: [sentenceRule] });
    harness.pi.sendMessage = () => { throw new Error("host rejected guidance"); };
    createConversationRulesExtension({
      modelPath: resolve("model"), registerTool: false, registerCommand: false,
    })(harness.pi);
    await harness.handlers.get("session_start")?.({ type: "session_start" }, harness.context);
    await expect(harness.handlers.get("message_update")?.({
      type: "message_update",
      message: { role: "assistant", timestamp: 100, responseId: "response-1" },
      assistantMessageEvent: { type: "text_delta", delta: "Guest checkout remains available. Next" },
    }, harness.context)).rejects.toThrow("host rejected guidance");
    expect(receiptCount(harness.context)).toBe(0);
  });

  it("runs the repository's conversation rules, except one switched off in the repository", async () => {
    const harness = await createHarness({
      rules: [
        { ...sentenceRule, action: { content: "From the library." } },
        { ...sentenceRule, id: "switched-off", action: { content: "Switched off." } },
      ],
    });
    const database = openRerailDatabase(defaultRerailDatabasePath());
    try {
      setRuleSetting(database, {
        owner: { kind: "repository", repositoryKey: harness.repositoryKey! }, folder: "", targetKind: "rule", targetId: "switched-off", on: false, mode: null,
      });
    } finally {
      database.$client.close();
    }
    createConversationRulesExtension({
      modelPath: resolve("model"),
      registerTool: false,
      registerCommand: false,
    })(harness.pi);

    await harness.handlers.get("session_start")?.({ type: "session_start" }, harness.context);
    await harness.handlers.get("message_update")?.(
      {
        type: "message_update",
        message: { role: "assistant", timestamp: 100, responseId: "response-1" },
        assistantMessageEvent: { type: "text_delta", delta: "Guest checkout remains available. Next" },
      },
      harness.context,
    );

    expect(harness.messages.map(sent => sent.message.content)).toEqual([
      deliveredSteer("checkout", "From the library."),
    ]);
    expect(harness.entries.map(entry => (entry.data as { ruleId: string }).ruleId)).toEqual(["checkout"]);
  });

  it("counts after-gap repeats across separate agent runs", async () => {
    const harness = await createHarness({ rules: [{ ...sentenceRule, repeat: { mode: "after-gap", gapTurns: 2 } }] });
    createConversationRulesExtension({
      modelPath: resolve("model"),
      registerTool: false,
      registerCommand: false,
    })(harness.pi);
    await harness.handlers.get("session_start")?.({ type: "session_start" }, harness.context);

    const streamSentence = async (responseId: string, timestamp: number) => {
      await harness.handlers.get("message_update")?.(
        {
          type: "message_update",
          message: { role: "assistant", timestamp, responseId },
          assistantMessageEvent: { type: "text_delta", delta: "Guest checkout remains available. Next" },
        },
        harness.context,
      );
    };
    const finishTurn = async (responseId: string, timestamp: number) => {
      await harness.handlers.get("turn_end")?.(
        {
          type: "turn_end",
          turnIndex: 0,
          message: { role: "assistant", timestamp, responseId, content: [] },
          toolResults: [],
        },
        harness.context,
      );
    };

    await streamSentence("response-1", 1);
    await finishTurn("response-1", 1);
    await streamSentence("response-2", 2);
    await finishTurn("response-2", 2);
    await streamSentence("response-3", 3);

    expect(harness.messages).toHaveLength(2);
  });

  it("samples awaited tool calls and tolerates malformed tool arguments", async () => {
    const toolRule: ConversationRule = {
      ...sentenceRule,
      id: "tool-input",
      reference: '{"checkout":"Guest checkout remains available."}',
      mode: "log",
      sample: { source: "tool:submit", boundary: "tool-call", minimumTokens: 1 },
      action: { content: "Observe the tool." },
    };
    const harness = await createHarness({ rules: [toolRule] });
    createConversationRulesExtension({
      modelPath: resolve("model"),
      registerTool: false,
      registerCommand: false,
    })(harness.pi);
    await harness.handlers.get("session_start")?.({ type: "session_start" }, harness.context);

    await harness.handlers.get("tool_call")?.(
      {
        type: "tool_call",
        toolCallId: "tool-1",
        toolName: "submit",
        input: { checkout: "Guest checkout remains available." },
      },
      harness.context,
    );
    const circular: { self?: unknown } = {};
    circular.self = circular;
    await harness.handlers.get("tool_call")?.(
      {
        type: "tool_call",
        toolCallId: "tool-2",
        toolName: "submit",
        input: circular,
      },
      harness.context,
    );
    await harness.handlers.get("tool_call")?.(
      {
        type: "tool_call",
        toolCallId: "tool-3",
        toolName: "submit",
        input: { checkout: 1n },
      },
      harness.context,
    );

    expect(harness.messages).toEqual([]);
    expect(harness.entries).toEqual([
      expect.objectContaining({
        customType: "rerail.match",
        data: expect.objectContaining({
          ruleId: "tool-input",
          source: "tool:submit",
          boundary: "tool-call",
          mode: "log",
        }),
      }),
    ]);
  });

  it("steers on a structural tool-call rule and ignores the tool result that echoes it", async () => {
    const forcePush: ConversationRule = {
      id: "no-force-push",
      mode: "log",
      match: { field: "input.command", regex: "\\bgit\\s+push\\b[^\\n]*--force\\b" },
      sample: { source: "tool:bash", boundary: "tool-call" },
      action: { content: "Do not force-push." },
      repeat: { mode: "always" },
    };
    const harness = await createHarness({ rules: [forcePush] });
    createConversationRulesExtension({
      modelPath: resolve("model"),
      registerTool: false,
      registerCommand: false,
    })(harness.pi);
    await harness.handlers.get("session_start")?.({ type: "session_start" }, harness.context);

    const call = (toolCallId: string, command: string) => harness.handlers.get("tool_call")?.(
      { type: "tool_call", toolCallId, toolName: "bash", input: { command } },
      harness.context,
    );
    await call("tool-1", "git push origin feature/auth");
    await call("tool-2", "git push --force origin main");
    await harness.handlers.get("tool_result")?.(
      {
        type: "tool_result",
        toolCallId: "tool-2",
        toolName: "bash",
        input: { command: "git status" },
        content: [{ type: "text", text: "hint: git push --force origin main was rejected" }],
        isError: false,
      },
      harness.context,
    );

    expect(harness.entries).toEqual([
      expect.objectContaining({
        customType: "rerail.match",
        data: expect.objectContaining({ ruleId: "no-force-push", source: "tool:bash", boundary: "tool-call", mode: "log" }),
      }),
    ]);
  });

  it("detaches the current OMP runtime when Rerail is stopped in the repository", async () => {
    const harness = await createHarness({ rules: [sentenceRule] });
    const notices: Array<{ message: string; level: string }> = [];
    createConversationRulesExtension({
      modelPath: resolve("model"),
      registerTool: false,
    })(harness.pi);
    await harness.handlers.get("session_start")?.({ type: "session_start" }, harness.context);

    const stop = harness.commands.get("rerail-stop");
    if (stop === undefined) throw new Error("OMP extension did not register rerail-stop");
    await stop("", { ui: { notify(message, level) { notices.push({ message, level }); } } });
    await harness.handlers.get("session_start")?.({ type: "session_start" }, harness.context);
    await harness.handlers.get("before_agent_start")?.(
      { type: "before_agent_start", prompt: "Guest checkout remains available." },
      harness.context,
    );

    expect(isRerailStarted(harness.context.cwd)).toBe(false);
    expect(notices).toEqual([{ message: expect.stringContaining("○ Rerail is stopped in"), level: "info" }]);
    expect(harness.messages).toEqual([]);
  }, 30_000); // `/rerail-stop` runs the real CLI, a Node process of its own.

  it("stops Rerail in the database the extension was configured with, not the Rerail home's", async () => {
    const harness = await createHarness();
    const configured = join(harness.context.cwd, "configured", "rules.sqlite");
    startTestRepository(harness.context.cwd, configured);
    createConversationRulesExtension({
      modelPath: resolve("model"),
      databasePath: configured,
      registerTool: false,
    })(harness.pi);
    await harness.handlers.get("session_start")?.({ type: "session_start" }, harness.context);

    const stop = harness.commands.get("rerail-stop");
    if (stop === undefined) throw new Error("OMP extension did not register rerail-stop");
    await stop("", { ui: { notify() {} } });

    expect(isRerailStarted(harness.context.cwd, configured)).toBe(false);
    // The home's database, which this extension does not read, is left as it was.
    expect(isRerailStarted(harness.context.cwd)).toBe(true);
  }, 30_000); // `/rerail-stop` runs the real CLI, a Node process of its own.

  it("records and steers nothing in a repository where Rerail was never started", async () => {
    const harness = await createHarness({ started: false });
    createConversationRulesExtension({
      modelPath: resolve("model"),
      registerTool: false,
    })(harness.pi);
    await harness.handlers.get("session_start")?.({ type: "session_start" }, harness.context);
    await harness.handlers.get("before_agent_start")?.(
      { type: "before_agent_start", prompt: "Guest checkout remains available." },
      harness.context,
    );

    expect(harness.messages).toEqual([]);
    const reader = new SqliteSessionReader(defaultRerailDatabasePath());
    try {
      expect(reader.session("omp-test-session")).toBeUndefined();
    } finally {
      reader.close();
    }
  });

  it("retires the live owner before rebuilding the same OMP session through a conversation rules reload", async () => {
    const harness = await createHarness({ rules: [sentenceRule] });
    createConversationRulesExtension({
      modelPath: resolve("model"),
      registerTool: false,
    })(harness.pi);
    await harness.handlers.get("session_start")?.({ type: "session_start" }, harness.context);

    const reload = harness.commands.get("cosine-rules");
    if (reload === undefined) throw new Error("OMP extension did not register the cosine-rules command");
    await reload("reload", { ui: { notify() {} } });
    await harness.handlers.get("message_update")?.(
      {
        type: "message_update",
        message: { role: "assistant", timestamp: 101, responseId: "response-after-reload" },
        assistantMessageEvent: { type: "text_delta", delta: "Guest checkout remains available. Next" },
      },
      harness.context,
    );

    expect(harness.messages).toHaveLength(1);
    expect(harness.entries.filter((entry) => entry.customType === "rerail.match")).toHaveLength(1);
  });

  it("keeps recording and delivering guidance after an in-place reset beneath the daemon", async () => {
    const harness = await createHarness({ rules: [{ ...sentenceRule, repeat: { mode: "always" } }] });
    createConversationRulesExtension({
      modelPath: resolve("model"),
      registerTool: false, registerCommand: false,
    })(harness.pi);
    await harness.handlers.get("session_start")?.({}, harness.context);
    const emit = (responseId: string) => harness.handlers.get("message_update")?.({
      message: { role: "assistant", timestamp: 100, responseId },
      assistantMessageEvent: { type: "text_delta", delta: "Guest checkout remains available. Next" },
    }, harness.context);
    await emit("before-reset");
    const databasePath = defaultRerailDatabasePath();
    resetRepositoryHistory(databasePath, harness.repositoryKey!);
    await emit("after-reset");

    expect(harness.messages.map(sent => sent.message.content)).toEqual([
      deliveredSteer("checkout", "Check checkout requirements."),
      deliveredSteer("checkout", "Check checkout requirements."),
    ]);
    const reader = new SqliteSessionReader(databasePath);
    try {
      expect(reader.events("omp-test-session").map(event => event.kind)).toEqual([
        "assistant-delta",
        "steering-delivered",
      ]);
      expect(reader.session("omp-test-session")?.endedAt).toBeNull();
    } finally {
      reader.close();
    }
  });

  /**
   * A running daemon whose native sessions can be refused by name, whose next initialize can be held mid-flight, whose
   * ingests `state.ingest` and yields `state.yield` can hold or answer with deliveries, and which keeps the gaps it is
   * asked to record and the events it took.
   */
  function flakyDaemon(cwd: string) {
    daemonRoute.start = async () => ({
      started: false,
      daemon: { state: { url: "http://127.0.0.1:4737/", nativeToken: "native-token" }, health: { workspaceRoot: cwd } },
    });
    const fetchNative = globalThis.fetch;
    const state = {
      down: new Set<string>(),
      hold: undefined as PromiseWithResolvers<void> | undefined,
      ingest: undefined as (() => Promise<readonly AdapterDelivery[]>) | undefined,
      yield: undefined as (() => Promise<readonly AdapterDelivery[]>) | undefined,
    };
    const requests: Array<{ operation: string; sessionId: string }> = [];
    const gaps: unknown[] = [];
    const ingested: unknown[] = [];
    vi.stubGlobal("fetch", async (url: URL, init: RequestInit) => {
      if (url.port !== "4737") return fetchNative(url, init);
      const body: unknown = JSON.parse(String(init.body));
      const request = z.object({ operation: z.string(), sessionId: z.string() }).parse(body);
      requests.push(request);
      if (request.operation === "initialize" && state.hold !== undefined) {
        const { promise } = state.hold;
        state.hold = undefined;
        await promise;
      }
      if (state.down.has(request.sessionId)) throw new TypeError("fetch failed");
      if (request.operation === "record-gap") gaps.push(z.object({ input: z.unknown() }).parse(body).input);
      if (request.operation === "ingest") ingested.push(z.object({ event: z.object({ json: z.unknown() }) }).parse(body).event.json);
      const answer = request.operation === "ingest" ? state.ingest : request.operation === "yield" ? state.yield : undefined;
      const deliveries = answer === undefined ? [] : await answer();
      return Response.json({ result: null, deliveries, entries: [] });
    });
    return { state, requests, gaps, ingested, initializes: (sessionId: string) => requests.filter(r => r.operation === "initialize" && r.sessionId === sessionId).length };
  }

  describe("attach retry", () => {

    it("surfaces the first failure, retries quietly with backoff, and reports once when it attaches", async () => {
      const harness = await createHarness();
      const daemon = flakyDaemon(harness.context.cwd);
      vi.useFakeTimers();
      const stderr = vi.spyOn(process.stderr, "write").mockReturnValue(true);
      createConversationRulesExtension({ modelPath: resolve("model"), registerTool: false, registerCommand: false })(harness.pi);
      daemon.state.down.add("omp-test-session");
      await expect(harness.handlers.get("session_start")!({ type: "session_start" }, harness.context)).rejects.toThrow("fetch failed");
      expect(daemon.initializes("omp-test-session")).toBe(1);

      await vi.advanceTimersByTimeAsync(1_000);
      await vi.waitFor(() => expect(daemon.initializes("omp-test-session")).toBe(2));
      await vi.advanceTimersByTimeAsync(2_000);
      await vi.waitFor(() => expect(daemon.initializes("omp-test-session")).toBe(3));
      expect(stderr).not.toHaveBeenCalled();

      daemon.state.down.delete("omp-test-session");
      await vi.advanceTimersByTimeAsync(4_000);
      await vi.waitFor(() => expect(daemon.initializes("omp-test-session")).toBe(4));
      // The 4th initialize is counted when it is sent; the report is written once its reply arrives.
      await vi.waitFor(() => expect(stderr).toHaveBeenCalledTimes(1));
      expect(stderr).toHaveBeenCalledWith(expect.stringContaining("Rerail attached after 3 failed attempt(s)"));
      await harness.handlers.get("before_agent_start")!({ type: "before_agent_start", prompt: "hello" }, harness.context);
      expect(daemon.requests.at(-1)).toEqual({ operation: "ingest", sessionId: "omp-test-session" });
      vi.useRealTimers();
    });

    it("drops a retry that was in flight when a newer session attached", async () => {
      const harness = await createHarness();
      const daemon = flakyDaemon(harness.context.cwd);
      vi.useFakeTimers();
      vi.spyOn(process.stderr, "write").mockReturnValue(true);
      createConversationRulesExtension({ modelPath: resolve("model"), registerTool: false, registerCommand: false })(harness.pi);
      const next = { ...harness.context, sessionManager: { getSessionId: () => "omp-next-session", getBranch: () => [] } } as unknown as ExtensionContext;
      daemon.state.down.add("omp-test-session");
      await expect(harness.handlers.get("session_start")!({ type: "session_start" }, harness.context)).rejects.toThrow();

      // The retry fires and its initialize is held open when the host switches sessions.
      const held = Promise.withResolvers<void>();
      daemon.state.hold = held;
      await vi.advanceTimersByTimeAsync(1_000);
      await vi.waitFor(() => expect(daemon.initializes("omp-test-session")).toBe(2));
      const switching = harness.handlers.get("session_switch")!({ type: "session_switch" }, next) as Promise<void>;
      held.resolve();
      await expect(switching).resolves.toBeUndefined();
      expect(daemon.initializes("omp-next-session")).toBe(1);

      await vi.advanceTimersByTimeAsync(60_000);
      await settleIo();
      expect(daemon.initializes("omp-test-session")).toBe(2);
      await harness.handlers.get("before_agent_start")!({ type: "before_agent_start", prompt: "hello" }, next);
      expect(daemon.requests.at(-1)).toEqual({ operation: "ingest", sessionId: "omp-next-session" });
      vi.useRealTimers();
    });

    it("drops a retry that was in flight at shutdown", async () => {
      const harness = await createHarness();
      const daemon = flakyDaemon(harness.context.cwd);
      vi.useFakeTimers();
      vi.spyOn(process.stderr, "write").mockReturnValue(true);
      createConversationRulesExtension({ modelPath: resolve("model"), registerTool: false, registerCommand: false })(harness.pi);
      daemon.state.down.add("omp-test-session");
      await expect(harness.handlers.get("session_start")!({ type: "session_start" }, harness.context)).rejects.toThrow();

      const held = Promise.withResolvers<void>();
      daemon.state.hold = held;
      await vi.advanceTimersByTimeAsync(1_000);
      await vi.waitFor(() => expect(daemon.initializes("omp-test-session")).toBe(2));
      const shutdown = harness.handlers.get("session_shutdown")!({}, harness.context) as Promise<void>;
      held.resolve();
      await shutdown;

      await vi.advanceTimersByTimeAsync(60_000);
      await settleIo();
      expect(daemon.initializes("omp-test-session")).toBe(2);
      vi.useRealTimers();
    });

    it("drops a breaker-driven reattach that fired while a newer session was attaching", async () => {
      const harness = await createHarness();
      const daemon = flakyDaemon(harness.context.cwd);
      vi.useFakeTimers();
      vi.spyOn(process.stderr, "write").mockReturnValue(true);
      createConversationRulesExtension({ modelPath: resolve("model"), registerTool: false, registerCommand: false })(harness.pi);
      const next = { ...harness.context, sessionManager: { getSessionId: () => "omp-next-session", getBranch: () => [] } } as unknown as ExtensionContext;
      await harness.handlers.get("session_start")!({ type: "session_start" }, harness.context);
      expect(daemon.initializes("omp-test-session")).toBe(1);

      // Three failed polls (100, then 100 and 200 of backoff) put the breaker one 400ms wait from asking the host to rebuild.
      daemon.state.down.add("omp-test-session");
      await vi.advanceTimersByTimeAsync(400);
      expect(daemon.requests.filter(r => r.operation === "poll")).toHaveLength(3);

      // The switch's initialize is held open when the old session's rebuild fires; the daemon has healed by then.
      const held = Promise.withResolvers<void>();
      daemon.state.hold = held;
      const switching = harness.handlers.get("session_switch")!({ type: "session_switch" }, next) as Promise<void>;
      await vi.waitFor(() => expect(daemon.initializes("omp-next-session")).toBe(1));
      daemon.state.down.delete("omp-test-session");
      await vi.advanceTimersByTimeAsync(400);
      held.resolve();
      await expect(switching).resolves.toBeUndefined();
      await settleIo();

      expect(daemon.initializes("omp-test-session")).toBe(1);
      await harness.handlers.get("before_agent_start")!({ type: "before_agent_start", prompt: "hello" }, next);
      expect(daemon.requests.at(-1)).toEqual({ operation: "ingest", sessionId: "omp-next-session" });
      vi.useRealTimers();
    });

    const delta = (responseId: string, text: string) => ({
      type: "message_update",
      message: { role: "assistant", timestamp: 1, responseId },
      assistantMessageEvent: { type: "text_delta", delta: text },
    });
    const droppedMarkers = (entries: ReadonlyArray<{ customType: string; data: unknown }>) =>
      entries.filter(entry => entry.customType === EVENTS_DROPPED_ENTRY_TYPE).map(entry => entry.data);

    it("counts an event that waited on a breaker-driven reattach that failed, and records it once the daemon is back", async () => {
      const harness = await createHarness();
      const daemon = flakyDaemon(harness.context.cwd);
      vi.useFakeTimers();
      vi.spyOn(process.stderr, "write").mockReturnValue(true);
      createConversationRulesExtension({ modelPath: resolve("model"), registerTool: false, registerCommand: false })(harness.pi);
      await harness.handlers.get("session_start")!({ type: "session_start" }, harness.context);

      // Three failed polls put the breaker one 400ms wait from asking the host to rebuild; the rebuild's initialize is held open.
      daemon.state.down.add("omp-test-session");
      await vi.advanceTimersByTimeAsync(400);
      expect(daemon.requests.filter(r => r.operation === "poll")).toHaveLength(3);
      const held = Promise.withResolvers<void>();
      daemon.state.hold = held;
      await vi.advanceTimersByTimeAsync(400);
      await settleIo();
      expect(daemon.initializes("omp-test-session")).toBe(2);

      // An event waits on that rebuild, which fails: it rejects with the failure, and the runtime it left current counts it.
      const racing = expect((harness.handlers.get("message_update")!(delta("racing", "while rebuilding"), harness.context) as Promise<void>))
        .rejects.toThrow("fetch failed");
      held.resolve();
      await racing;
      expect(droppedMarkers(harness.entries)).toEqual([
        expect.objectContaining({ count: 1, kinds: { "assistant-delta": 1 }, recovered: false }),
      ]);

      daemon.state.down.delete("omp-test-session");
      await vi.advanceTimersByTimeAsync(5_000);
      await vi.waitFor(() => expect(daemon.gaps).toEqual([expect.objectContaining({ count: 1, kinds: { "assistant-delta": 1 } })]));
      expect(droppedMarkers(harness.entries).at(-1)).toEqual(expect.objectContaining({ count: 1, recovered: true }));
      vi.useRealTimers();
    });

    it("counts an event that waited on a conversation rules reload that failed, and records it once the daemon is back", async () => {
      const harness = await createHarness();
      const daemon = flakyDaemon(harness.context.cwd);
      vi.useFakeTimers();
      vi.spyOn(process.stderr, "write").mockReturnValue(true);
      createConversationRulesExtension({ modelPath: resolve("model"), registerTool: false })(harness.pi);
      await harness.handlers.get("session_start")!({ type: "session_start" }, harness.context);

      // The first failed poll opens the breaker; the reload's initialize is held open.
      daemon.state.down.add("omp-test-session");
      await vi.advanceTimersByTimeAsync(100);
      expect(daemon.requests.filter(r => r.operation === "poll")).toHaveLength(1);
      const held = Promise.withResolvers<void>();
      daemon.state.hold = held;
      const reloading = expect(harness.commands.get("cosine-rules")!("reload", { ui: { notify() {} } })).rejects.toThrow("fetch failed");
      await settleIo();
      expect(daemon.initializes("omp-test-session")).toBe(2);

      const racing = expect((harness.handlers.get("message_update")!(delta("racing", "while reloading"), harness.context) as Promise<void>))
        .rejects.toThrow("fetch failed");
      held.resolve();
      await Promise.all([reloading, racing]);
      expect(droppedMarkers(harness.entries)).toEqual([
        expect.objectContaining({ count: 1, kinds: { "assistant-delta": 1 }, recovered: false }),
      ]);

      daemon.state.down.delete("omp-test-session");
      await vi.advanceTimersByTimeAsync(1_000);
      await vi.waitFor(() => expect(daemon.gaps).toEqual([expect.objectContaining({ count: 1, kinds: { "assistant-delta": 1 } })]));
      vi.useRealTimers();
    });

    it("sends the full assistant text when a delta counted during a failing attach is followed by the attach and the message end", async () => {
      const harness = await createHarness();
      const daemon = flakyDaemon(harness.context.cwd);
      vi.useFakeTimers();
      vi.spyOn(process.stderr, "write").mockReturnValue(true);
      createConversationRulesExtension({ modelPath: resolve("model"), registerTool: false, registerCommand: false })(harness.pi);
      await harness.handlers.get("session_start")!({ type: "session_start" }, harness.context);

      // The host re-attaches the same session while the daemon is out; a delta waits on that attach, which fails and counts it.
      daemon.state.down.add("omp-test-session");
      const held = Promise.withResolvers<void>();
      daemon.state.hold = held;
      const attaching = expect(harness.handlers.get("session_tree")!({ type: "session_tree" }, harness.context) as Promise<void>).rejects.toThrow("fetch failed");
      const counted = expect(harness.handlers.get("message_update")!(delta("streaming", "Guest checkout "), harness.context) as Promise<void>).rejects.toThrow("fetch failed");
      held.resolve();
      await Promise.all([attaching, counted]);
      expect(droppedMarkers(harness.entries)).toEqual([expect.objectContaining({ count: 1, kinds: { "assistant-delta": 1 }, recovered: false })]);

      // The retry attaches onto the runtime the session already had, so nothing resets its streamed messages.
      daemon.state.down.delete("omp-test-session");
      await vi.advanceTimersByTimeAsync(1_000);
      await vi.waitFor(() => expect(daemon.initializes("omp-test-session")).toBe(3));

      const message = { role: "assistant", timestamp: 1, responseId: "streaming", content: [{ type: "text", text: "Guest checkout remains available." }], stopReason: "stop" };
      await harness.handlers.get("message_end")!({ type: "message_end", message }, harness.context);
      expect(daemon.ingested).toEqual([expect.objectContaining({ kind: "assistant-delta", final: true, delta: "Guest checkout remains available." })]);
      vi.useRealTimers();
    });

    it("sends the full assistant text when the open breaker dropped its deltas and the daemon is back by the message end", async () => {
      const harness = await createHarness();
      const daemon = flakyDaemon(harness.context.cwd);
      vi.useFakeTimers();
      const stderr = vi.spyOn(process.stderr, "write").mockReturnValue(true);
      createConversationRulesExtension({ modelPath: resolve("model"), registerTool: false, registerCommand: false })(harness.pi);
      await harness.handlers.get("session_start")!({ type: "session_start" }, harness.context);

      // The first failed poll opens the breaker; the delta of the streaming message is counted instead of sent.
      daemon.state.down.add("omp-test-session");
      await vi.advanceTimersByTimeAsync(100);
      await harness.handlers.get("message_update")!(delta("streaming", "Guest checkout "), harness.context);
      expect(daemon.ingested).toEqual([]);
      expect(droppedMarkers(harness.entries)).toEqual([expect.objectContaining({ count: 1, recovered: false })]);

      daemon.state.down.delete("omp-test-session");
      await vi.advanceTimersByTimeAsync(1_000);
      await vi.waitFor(() => expect(stderr).toHaveBeenCalledWith(expect.stringContaining("recovered; 1 session event(s)")));

      const message = { role: "assistant", timestamp: 1, responseId: "streaming", content: [{ type: "text", text: "Guest checkout remains available." }], stopReason: "stop" };
      await harness.handlers.get("message_end")!({ type: "message_end", message }, harness.context);
      expect(daemon.ingested).toEqual([expect.objectContaining({ kind: "assistant-delta", final: true, delta: "Guest checkout remains available." })]);
      vi.useRealTimers();
    });

    it("aborts the partial stream and sends the whole text as one message when only some deltas were delivered", async () => {
      const harness = await createHarness();
      const daemon = flakyDaemon(harness.context.cwd);
      vi.useFakeTimers();
      const stderr = vi.spyOn(process.stderr, "write").mockReturnValue(true);
      createConversationRulesExtension({ modelPath: resolve("model"), registerTool: false, registerCommand: false })(harness.pi);
      await harness.handlers.get("session_start")!({ type: "session_start" }, harness.context);

      // The first delta reaches the daemon; the breaker then opens and counts the second.
      await harness.handlers.get("message_update")!(delta("streaming", "Guest checkout "), harness.context);
      daemon.state.down.add("omp-test-session");
      await vi.advanceTimersByTimeAsync(100);
      await harness.handlers.get("message_update")!(delta("streaming", "remains available."), harness.context);
      expect(daemon.ingested).toEqual([expect.objectContaining({ kind: "assistant-delta", delta: "Guest checkout ", final: false })]);

      daemon.state.down.delete("omp-test-session");
      await vi.advanceTimersByTimeAsync(1_000);
      await vi.waitFor(() => expect(stderr).toHaveBeenCalledWith(expect.stringContaining("recovered; 1 session event(s)")));

      const message = { role: "assistant", timestamp: 1, responseId: "streaming", content: [{ type: "text", text: "Guest checkout remains available." }], stopReason: "stop" };
      await harness.handlers.get("message_end")!({ type: "message_end", message }, harness.context);
      expect(daemon.ingested.slice(1)).toEqual([
        expect.objectContaining({ kind: "assistant-abort", messageId: "1:streaming" }),
        expect.objectContaining({ kind: "assistant-message", text: "Guest checkout remains available." }),
      ]);
      vi.useRealTimers();
    });

    it("counts a delta whose ingest rejected after the daemon took it as delivered, so the message end does not store its text twice", async () => {
      const harness = await createHarness();
      const daemon = flakyDaemon(harness.context.cwd);
      vi.spyOn(process.stderr, "write").mockReturnValue(true);
      createConversationRulesExtension({ modelPath: resolve("model"), registerTool: false, registerCommand: false })(harness.pi);
      await harness.handlers.get("session_start")!({ type: "session_start" }, harness.context);

      // The daemon takes each delta; the host then refuses something the event asked for, which rejects the ingest.
      const ingest = NativeSessionClient.prototype.ingest;
      const refusing = vi.spyOn(NativeSessionClient.prototype, "ingest").mockImplementation(async function (this: NativeSessionClient, event) {
        const delivered = await ingest.call(this, event);
        if (event.kind === "assistant-delta") throw new Error("host refused a steer delivery");
        return delivered;
      });
      try {
        await expect(harness.handlers.get("message_update")!(delta("streaming", "Guest checkout "), harness.context) as Promise<void>)
          .rejects.toThrow("host refused a steer delivery");
        await expect(harness.handlers.get("message_update")!(delta("streaming", "remains available."), harness.context) as Promise<void>)
          .rejects.toThrow("host refused a steer delivery");

        const message = { role: "assistant", timestamp: 1, responseId: "streaming", content: [{ type: "text", text: "Guest checkout remains available." }], stopReason: "stop" };
        await expect(harness.handlers.get("message_end")!({ type: "message_end", message }, harness.context) as Promise<void>)
          .rejects.toThrow("host refused a steer delivery");
        expect(daemon.ingested).toEqual([
          expect.objectContaining({ kind: "assistant-delta", delta: "Guest checkout ", final: false }),
          expect.objectContaining({ kind: "assistant-delta", delta: "remains available.", final: false }),
          expect.objectContaining({ kind: "assistant-delta", delta: "", final: true }),
        ]);
      } finally {
        refusing.mockRestore();
      }
    });

    it("hands an event that waited on a failed attach to the session OMP moved on to, not the one it left", async () => {
      const harness = await createHarness();
      const daemon = flakyDaemon(harness.context.cwd);
      vi.spyOn(process.stderr, "write").mockReturnValue(true);
      createConversationRulesExtension({ modelPath: resolve("model"), registerTool: false, registerCommand: false })(harness.pi);
      const sessionContext = (sessionId: string) => ({ ...harness.context, sessionManager: { getSessionId: () => sessionId, getBranch: () => [] } }) as unknown as ExtensionContext;
      const second = sessionContext("omp-second-session");
      const third = sessionContext("omp-third-session");
      await harness.handlers.get("session_start")!({ type: "session_start" }, harness.context);

      // OMP switches to a session whose attach fails, and on to another while that attach is held open; a delta waits on the first.
      daemon.state.down.add("omp-second-session");
      const held = Promise.withResolvers<void>();
      daemon.state.hold = held;
      daemon.requests.length = 0;
      const failed = expect(harness.handlers.get("session_switch")!({ type: "session_switch" }, second) as Promise<void>).rejects.toThrow("fetch failed");
      await vi.waitFor(() => expect(daemon.initializes("omp-second-session")).toBe(1));
      const racing = harness.handlers.get("message_update")!(delta("racing", "Guest checkout "), third) as Promise<void>;
      const switching = harness.handlers.get("session_switch")!({ type: "session_switch" }, third) as Promise<void>;
      held.resolve();
      await failed;
      await switching;
      await racing;

      expect(daemon.requests.filter(request => request.operation === "ingest")).toEqual([{ operation: "ingest", sessionId: "omp-third-session" }]);
    });

    it("counts an event of a session whose attach failed in that session's gap, though the previous session's runtime survived", async () => {
      const harness = await createHarness();
      const daemon = flakyDaemon(harness.context.cwd);
      vi.useFakeTimers();
      vi.spyOn(process.stderr, "write").mockReturnValue(true);
      createConversationRulesExtension({ modelPath: resolve("model"), registerTool: false, registerCommand: false })(harness.pi);
      const second = { ...harness.context, sessionManager: { getSessionId: () => "omp-second-session", getBranch: () => [] } } as unknown as ExtensionContext;
      await harness.handlers.get("session_start")!({ type: "session_start" }, harness.context);

      // OMP switches to a session whose attach fails; the first session's runtime is still there.
      daemon.state.down.add("omp-second-session");
      await expect(harness.handlers.get("session_switch")!({ type: "session_switch" }, second) as Promise<void>).rejects.toThrow("fetch failed");
      daemon.requests.length = 0;

      await harness.handlers.get("message_update")!(delta("streaming", "Guest checkout "), second);

      expect(daemon.requests.filter(request => request.operation === "ingest")).toEqual([]);
      expect(daemon.ingested).toEqual([]);
      expect(droppedMarkers(harness.entries)).toEqual([expect.objectContaining({ count: 1, kinds: { "assistant-delta": 1 }, recovered: false })]);
      vi.useRealTimers();
    });
  });

  describe("daemon faults", () => {
    const toolCall = (toolCallId: string) => ({ type: "tool_call", toolCallId, toolName: "write", input: { path: "a.ts", content: "x" } });
    const toolResult = (toolCallId: string) => ({ type: "tool_result", toolCallId, toolName: "write", content: [], details: undefined, isError: false });
    const hookFailures = (stderr: { mock: { calls: unknown[][] } }) => stderr.mock.calls.filter(([line]) => String(line).includes("hook failed"));

    it("never fails a tool call while the daemon is down", async () => {
      const harness = await createHarness();
      const daemon = flakyDaemon(harness.context.cwd);
      const stderr = vi.spyOn(process.stderr, "write").mockReturnValue(true);
      createConversationRulesExtension({ modelPath: resolve("model"), registerTool: false, registerCommand: false })(harness.pi);
      await harness.handlers.get("session_start")!({ type: "session_start" }, harness.context);

      daemon.state.down.add("omp-test-session");
      for (const id of ["call-1", "call-2", "call-3", "call-4"]) {
        await expect(harness.handlers.get("tool_call")!(toolCall(id), harness.context)).resolves.toBeUndefined();
        await expect(harness.handlers.get("tool_result")!(toolResult(id), harness.context)).resolves.toBeUndefined();
      }
      expect(hookFailures(stderr)).toEqual([]);
    });

    it("lets a tool call through when the attach it waited for fails", async () => {
      const harness = await createHarness();
      const daemon = flakyDaemon(harness.context.cwd);
      vi.useFakeTimers();
      const stderr = vi.spyOn(process.stderr, "write").mockReturnValue(true);
      createConversationRulesExtension({ modelPath: resolve("model"), registerTool: false, registerCommand: false })(harness.pi);
      daemon.state.down.add("omp-test-session");
      await expect(harness.handlers.get("session_start")!({ type: "session_start" }, harness.context)).rejects.toThrow("fetch failed");

      // The retry's initialize is held open, so the tool call waits on an attach that is about to fail again.
      const held = Promise.withResolvers<void>();
      daemon.state.hold = held;
      await vi.advanceTimersByTimeAsync(1_000);
      await vi.waitFor(() => expect(daemon.initializes("omp-test-session")).toBe(2));
      const call = harness.handlers.get("tool_call")!(toolCall("call-1"), harness.context);
      held.resolve();
      await expect(call).resolves.toBeUndefined();
      expect(hookFailures(stderr)).toEqual([]);
      vi.useRealTimers();
    });

    it("releases a tool call that a wedged daemon never answers before OMP's own deadline blocks it", async () => {
      const harness = await createHarness();
      const daemon = flakyDaemon(harness.context.cwd);
      createConversationRulesExtension({ modelPath: resolve("model"), registerTool: false, registerCommand: false })(harness.pi);
      await harness.handlers.get("session_start")!({ type: "session_start" }, harness.context);
      vi.useFakeTimers();
      const wedged = Promise.withResolvers<readonly AdapterDelivery[]>();
      daemon.state.ingest = () => wedged.promise;

      let released = false;
      void (harness.handlers.get("tool_call")!(toolCall("call-1"), harness.context) as Promise<void>).then(() => { released = true; });
      await vi.advanceTimersByTimeAsync(24_000);
      expect(released).toBe(false);
      await vi.advanceTimersByTimeAsync(1_000);
      expect(released).toBe(true);
      // Shutdown closes through the same request queue, so the wedge has to clear for the test to end.
      wedged.resolve([]);
      vi.useRealTimers();
    });

    it("holds the end of the run until the turn-end check answers, so its steer is sent while OMP still streams", async () => {
      const harness = await createHarness();
      const daemon = flakyDaemon(harness.context.cwd);
      createConversationRulesExtension({ modelPath: resolve("model"), registerTool: false, registerCommand: false })(harness.pi);
      await harness.handlers.get("session_start")!({ type: "session_start" }, harness.context);
      const checked = Promise.withResolvers<void>();
      daemon.state.ingest = async () => {
        await checked.promise;
        return [{
          sessionId: "omp-test-session",
          requested: "steer",
          mechanism: "native-queue",
          content: deliveredSteer("checkout", "Check checkout requirements."),
          ruleIds: ["checkout"],
          rules: [{ id: "checkout", state: "steered" }],
        }];
      };

      const ended = harness.handlers.get("turn_end")!({
        type: "turn_end",
        message: { role: "assistant", timestamp: 100, responseId: "response-1", content: [{ type: "text", text: "Done." }] },
      }, harness.context);
      let stopped = false;
      const stop = (harness.handlers.get("session_stop")!({ type: "session_stop", signal: new AbortController().signal }, harness.context) as Promise<unknown>)
        .then(result => { stopped = true; return result; });
      await settleIo();
      expect(stopped).toBe(false);
      expect(harness.messages).toEqual([]);

      checked.resolve();
      // Nothing is returned: the steer itself is queued, and OMP continues the run with it once the prompt unwinds.
      await expect(stop).resolves.toBeUndefined();
      expect(harness.messages.map(sent => sent.message.content)).toEqual([
        deliveredSteer("checkout", "Check checkout requirements."),
      ]);
      await ended;
    });

    it("sends the digest of the steers that missed their step at the end of the run, once the turn end has answered", async () => {
      const harness = await createHarness();
      const daemon = flakyDaemon(harness.context.cwd);
      createConversationRulesExtension({ modelPath: resolve("model"), registerTool: false, registerCommand: false })(harness.pi);
      await harness.handlers.get("session_start")!({ type: "session_start" }, harness.context);
      const digested = Promise.withResolvers<void>();
      const digest = steerBlock({ kind: "late" }, [{ ruleId: "checkout", mode: "steer", message: "Check checkout requirements.", location: "src/a.ts:2:7" }]);
      daemon.state.yield = async () => {
        await digested.promise;
        return [{ sessionId: "omp-test-session", requested: "steer", mechanism: "native-queue", content: digest, ruleIds: ["checkout"], rules: [{ id: "checkout", state: "steered" }] }];
      };
      daemon.requests.length = 0;

      const ended = harness.handlers.get("turn_end")!({
        type: "turn_end",
        message: { role: "assistant", timestamp: 100, responseId: "response-1", content: [{ type: "text", text: "Done." }] },
      }, harness.context);
      let stopped = false;
      const stop = (harness.handlers.get("session_stop")!({ type: "session_stop", signal: new AbortController().signal }, harness.context) as Promise<unknown>)
        .then(result => { stopped = true; return result; });
      await vi.waitFor(() => expect(daemon.requests.map(request => request.operation)).toContain("yield"));
      // Asked after the turn end, and the run is held until it answers.
      expect(daemon.requests.map(request => request.operation).filter(operation => operation === "ingest" || operation === "yield")).toEqual(["ingest", "yield"]);
      await settleIo();
      expect(stopped).toBe(false);
      expect(harness.messages).toEqual([]);

      digested.resolve();
      await expect(stop).resolves.toBeUndefined();
      expect(harness.messages.map(sent => [sent.message.content, sent.options])).toEqual([
        [digest, { deliverAs: "steer" }],
      ]);
      await ended;
    });
  });

  describe("while the daemon indexes the repository", () => {
    it("says so in the footer until it is done, while a prompt and its tool call go on at once, without code rules", async () => {
      const opening = Promise.withResolvers<void>();
      const open = CodeRuleBridge.prototype.openWorkspace;
      const spy = vi.spyOn(CodeRuleBridge.prototype, "openWorkspace").mockImplementationOnce(async function (this: CodeRuleBridge, sessionId: string) {
        await opening.promise;
        await open.call(this, sessionId);
      });
      const harness = await createHarness();
      const footer: string[][] = [];
      Object.assign(harness.context, {
        hasUI: true,
        mode: "tui",
        ui: { setWidget: (_key: string, lines: string[] | undefined) => { footer.push(lines ?? []); } },
      });
      const indexingShown = () => footer.at(-1)?.some(line => line.endsWith("indexing")) === true;
      try {
        createConversationRulesExtension({ modelPath: resolve("model"), registerTool: false, registerCommand: false })(harness.pi);
        await harness.handlers.get("session_start")!({ type: "session_start" }, harness.context);
        await vi.waitFor(() => expect(indexingShown()).toBe(true));

        // Neither waits for indexing: both answer while the evaluator is still held in its open.
        await harness.handlers.get("before_agent_start")!({ type: "before_agent_start", prompt: "hello" }, harness.context);
        await harness.handlers.get("tool_call")!({ type: "tool_call", toolCallId: "write-1", toolName: "write", input: { path: "a.ts", content: "x" } }, harness.context);
        expect(indexingShown()).toBe(true);

        opening.resolve();
        await vi.waitFor(() => expect(indexingShown()).toBe(false));
      } finally {
        opening.resolve();
        spy.mockRestore();
      }
    });
  });

  describe("steer strategies", () => {
    const steer = (toolCallId: string): AdapterDelivery => ({
      sessionId: "omp-test-session",
      requested: "steer",
      mechanism: "native-queue",
      content: "Rerail found 1 problem in your last edit.",
      ruleIds: ["no-any"],
      rules: [{ id: "no-any", state: "steered" }],
      strategy: "message",
      toolCallId,
    });
    const rewound = (toolCallId: string): AdapterDelivery => ({
      ...steer(toolCallId),
      content: "Your edit of a.ts was rolled back. Redo it following the rule.",
      strategy: "rewind",
      rewound: ["a.ts"],
    });
    const editResult = { type: "tool_result", toolCallId: "call-1", toolName: "edit", input: {}, content: [{ type: "text", text: "Edited a.ts" }], details: { diff: "+x" }, isError: false };

    it("puts an edit's steer after what that call's result says, past OMP's steer queue, counts it once, and shows the person its line", async () => {
      const harness = await createHarness();
      const daemon = flakyDaemon(harness.context.cwd);
      // A steer that went to OMP's queue would be refused here: nothing ever lands in it.
      Object.assign(harness.context, { hasPendingMessages: () => false });
      createConversationRulesExtension({ modelPath: resolve("model"), registerTool: false, registerCommand: false })(harness.pi);
      await harness.handlers.get("session_start")!({ type: "session_start" }, harness.context);
      daemon.state.ingest = async () => [steer("call-1")];

      await expect(harness.handlers.get("tool_result")!(editResult, harness.context)).resolves.toEqual({
        content: [{ type: "text", text: "Edited a.ts" }, { type: "text", text: "Rerail found 1 problem in your last edit." }],
      });
      // The person's line, linked, as an aside: after the call's results, before the next model call, never interrupting.
      expect(harness.messages).toEqual([{
        message: {
          customType: STEER_LINE_MESSAGE_TYPE,
          content: "Rerail found 1 problem in your last edit.",
          display: true,
          details: { rules: await deliveredRules(harness.context.cwd, "no-any") },
        },
        options: { deliverAs: "aside" },
      }]);
      expect(daemon.requests.filter(request => request.operation === "record-steering-delivery")).toHaveLength(1);
    });

    it("replaces a call's result with its rolled-back edit's steers, marked an error, with its other steers after", async () => {
      const harness = await createHarness();
      const daemon = flakyDaemon(harness.context.cwd);
      // A steer that went to OMP's queue would be refused here: nothing ever lands in it.
      Object.assign(harness.context, { hasPendingMessages: () => false });
      createConversationRulesExtension({ modelPath: resolve("model"), registerTool: false, registerCommand: false })(harness.pi);
      await harness.handlers.get("session_start")!({ type: "session_start" }, harness.context);
      daemon.state.ingest = async () => [rewound("call-1"), steer("call-1"), { ...rewound("call-1"), content: "Second rule." }];

      await expect(harness.handlers.get("tool_result")!(editResult, harness.context)).resolves.toEqual({
        content: [
          { type: "text", text: "Your edit of a.ts was rolled back. Redo it following the rule.\n\nSecond rule." },
          { type: "text", text: "Rerail found 1 problem in your last edit." },
        ],
        isError: true,
      });
      const rules = await deliveredRules(harness.context.cwd, "no-any");
      expect(harness.messages).toEqual([{
        message: {
          customType: STEER_LINE_MESSAGE_TYPE,
          content: "Your edit of a.ts was rolled back. Redo it following the rule.\n\nRerail found 1 problem in your last edit.\n\nSecond rule.",
          display: true,
          details: { rules: [...rules, ...rules, ...rules] },
        },
        options: { deliverAs: "aside" },
      }]);
      expect(daemon.requests.filter(request => request.operation === "record-steering-delivery")).toHaveLength(3);
    });

    it("keeps the person's steer lines out of every model request, and a later steer's message in", async () => {
      const harness = await createHarness();
      createConversationRulesExtension({ modelPath: resolve("model"), registerTool: false, registerCommand: false })(harness.pi);
      await harness.handlers.get("session_start")!({ type: "session_start" }, harness.context);
      const prompt = { role: "user", content: [{ type: "text", text: "Add guest checkout." }], timestamp: 1 };
      const result = { role: "toolResult", toolCallId: "call-1", toolName: "edit", content: [{ type: "text", text: "Edited a.ts\nRerail found 1 problem." }], isError: false, timestamp: 2 };
      const line = { role: "custom", customType: STEER_LINE_MESSAGE_TYPE, content: "Rerail found 1 problem.", display: true, details: { rules: [] }, timestamp: 3 };
      const late = { role: "custom", customType: RULE_MESSAGE_TYPE, content: "Rerail found 1 problem.", display: true, timestamp: 4 };

      await expect(harness.handlers.get("context")!({ type: "context", messages: [prompt, result, line, late] }, harness.context))
        .resolves.toEqual({ messages: [prompt, result, late] });
      await expect(harness.handlers.get("context")!({ type: "context", messages: [prompt, result, late] }, harness.context)).resolves.toBeUndefined();
    });

    it("sends a steer for a call whose result is not being ingested as a message", async () => {
      const harness = await createHarness();
      const daemon = flakyDaemon(harness.context.cwd);
      createConversationRulesExtension({ modelPath: resolve("model"), registerTool: false, registerCommand: false })(harness.pi);
      await harness.handlers.get("session_start")!({ type: "session_start" }, harness.context);
      daemon.state.ingest = async () => [steer("call-closed")];

      await expect(harness.handlers.get("tool_result")!(editResult, harness.context)).resolves.toBeUndefined();
      expect(harness.messages).toEqual([{
        message: {
          customType: RULE_MESSAGE_TYPE,
          content: "Rerail found 1 problem in your last edit.",
          display: true,
          details: { requested: "steer", mechanism: "native-queue", sessionId: "omp-test-session", rules: await deliveredRules(harness.context.cwd, "no-any") },
        },
        options: { deliverAs: "steer" },
      }]);
      expect(daemon.requests.filter(request => request.operation === "record-steering-delivery")).toHaveLength(1);
    });
  });

  describe("compaction", () => {
    /** OMP's preparation for a split turn whose prefix a Rerail steer leads, as `SessionMaintenance.compact` builds it. */
    function steerLedPreparation() {
      const prompt = { role: "user", content: [{ type: "text", text: "Add guest checkout." }], timestamp: 1 };
      const steer = {
        role: "custom", customType: RULE_MESSAGE_TYPE, content: [{ type: "text", text: "Rerail found 1 problem in src/cart.ts." }],
        display: true, attribution: "agent", timestamp: 2,
      };
      const fix = { role: "assistant", content: [{ type: "toolCall", id: "call-2", name: "edit", arguments: {} }], timestamp: 3 };
      return { prompt, steer, fix, preparation: { firstKeptEntryId: "entry-4", messagesToSummarize: [prompt], turnPrefixMessages: [steer, fix], recentMessages: [], isSplitTurn: true } };
    }

    it("summarizes a turn a steer leads as history with the person's prompt, the steer labelled as Rerail's", async () => {
      const harness = await createHarness();
      createConversationRulesExtension({ modelPath: resolve("model"), registerTool: false, registerCommand: false })(harness.pi);
      await harness.handlers.get("session_start")!({ type: "session_start" }, harness.context);
      const { prompt, steer, fix, preparation } = steerLedPreparation();

      expect(await harness.handlers.get("session_before_compact")!({ type: "session_before_compact", preparation }, harness.context)).toBeUndefined();
      expect(preparation).toMatchObject({
        messagesToSummarize: [
          prompt,
          { ...steer, content: [{ type: "text", text: "[Rerail: automated rule check sent to the agent, not a message from the user]\nRerail found 1 problem in src/cart.ts." }] },
          fix,
        ],
        turnPrefixMessages: [],
        isSplitTurn: false,
      });
    });

    it("leaves OMP's speculative compaction on where Rerail never attaches", async () => {
      const harness = await createHarness({ started: false });
      createConversationRulesExtension({ modelPath: resolve("model"), registerTool: false, registerCommand: false })(harness.pi);
      await harness.handlers.get("session_start")!({ type: "session_start" }, harness.context);

      expect(harness.handlers.has("session_before_compact")).toBe(false);
    });
  });
});
