import { existsSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
/** Attach-retry tests replace daemon discovery, which polls on real timers, with a daemon that is already up. */
const daemonRoute = vi.hoisted(() => ({ start: undefined as (() => Promise<unknown>) | undefined }));
vi.mock("../src/daemon-control.js", async () => {
  const actual = await vi.importActual<typeof import("../src/daemon-control.js")>("../src/daemon-control.js");
  return {
    ...actual,
    startDaemonForPackage: (...args: Parameters<typeof actual.startDaemonForPackage>) =>
      daemonRoute.start?.() ?? actual.startDaemonForPackage(...args),
  };
});
import Database from "better-sqlite3";
import type { CustomMessagePayload, ExtensionAPI, ExtensionContext } from "@oh-my-pi/pi-coding-agent";
import { z } from "zod";
import { RULE_MESSAGE_TYPE } from "../src/adapter-contract.js";
import { createCosineRulesExtension } from "../src/extension.js";
import type { CosineRule } from "../src/rule-schema.js";
import { closeNativeTestDaemons, startNativeTestDaemon } from "./native-daemon-fixture.js";
import { defaultRerailDatabasePath, resetRepositoryHistory } from "../src/database.js";
import { openScopedRuleCatalog, resolveRepositoryContext } from "../src/scoped-rule-catalog.js";
import { SqliteTranscriptReader } from "../src/transcript-store.js";
import { isRerailStarted } from "../src/repository-start.js";
import { startTestRepository } from "./support/started-repository.js";

// Every test here builds real sessions (native model, kernel, rule catalog); on a loaded CI runner a
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

async function createHarness({ started = true }: { started?: boolean } = {}) {
  const cwd = mkdtempSync(join(tmpdir(), "rerail-omp-extension-"));
  process.env.RERAIL_HOME = join(cwd, "home");
  temporaryDirectories.push(cwd);
  if (started) startTestRepository(cwd);
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
  return { handlers, commands, messages, entries, pi, context };
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

const sentenceRule: CosineRule = {
  id: "checkout",
  mode: "steer",
  reference: "Guest checkout remains available.",
  match: { operator: "gte", threshold: 0.99 },
  sample: { source: "assistant-text", boundary: "sentence", minimumTokens: 1 },
  action: { content: "Check checkout requirements." },
  repeat: { mode: "once" },
};

describe("OMP cosine-rules extension", () => {
  it.each(["steer", "block"] as const)("steers immediately when a streamed sentence matches a %s rule", async mode => {
    const harness = await createHarness();
    createCosineRulesExtension({
      modelPath: resolve("model"),
      rules: [{ ...sentenceRule, mode }],
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
          content: "Check checkout requirements.",
          display: true,
          details: { requested: mode, mechanism: "native-queue", sessionId: "omp-test-session" },
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
    const harness = await createHarness();
    Object.assign(harness.context, {
      isIdle: () => true,
      hasPendingMessages: () => false,
    });
    createCosineRulesExtension({
      modelPath: resolve("model"),
      rules: [{
        ...sentenceRule,
        sample: { source: "user-input", boundary: "message" },
      }],
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
    const harness = await createHarness();
    harness.pi.sendMessage = () => { throw new Error("host rejected guidance"); };
    createCosineRulesExtension({
      modelPath: resolve("model"), rules: [sentenceRule], registerTool: false, registerCommand: false,
    })(harness.pi);
    await harness.handlers.get("session_start")?.({ type: "session_start" }, harness.context);
    await expect(harness.handlers.get("message_update")?.({
      type: "message_update",
      message: { role: "assistant", timestamp: 100, responseId: "response-1" },
      assistantMessageEvent: { type: "text_delta", delta: "Guest checkout remains available. Next" },
    }, harness.context)).rejects.toThrow("host rejected guidance");
    expect(receiptCount(harness.context)).toBe(0);
  });

  it("loads enabled cosine rules from the project database and lets them override file rules", async () => {
    const harness = await createHarness();
    mkdirSync(join(harness.context.cwd, ".omp"));
    writeFileSync(join(harness.context.cwd, ".omp", "conversation-rules.json"), JSON.stringify({
      version: 1,
      rules: [
        { ...sentenceRule, action: { content: "From the JSON file." } },
        { ...sentenceRule, id: "file-only", action: { content: "First file." } },
      ],
    }), { flag: "wx" });
    writeFileSync(join(harness.context.cwd, ".omp", "second-rules.json"), JSON.stringify({
      version: 1,
      rules: [{ ...sentenceRule, id: "file-only", action: { content: "File-only rule." } }],
    }), { flag: "wx" });
    const store = openScopedRuleCatalog(harness.context.cwd);
    store.upsertCosine({ ...sentenceRule, action: { content: "From the database." } });
    store.upsertCosine({ ...sentenceRule, id: "disabled", action: { content: "Disabled rule." } }, { enabled: false });
    store.close();
    createCosineRulesExtension({
      modelPath: resolve("model"),
      rules: [{ ...sentenceRule, id: "file-only", action: { content: "Host fallback." } }],
      ruleFiles: [".omp/conversation-rules.json", ".omp/second-rules.json"],
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

    expect(harness.messages.map(sent => sent.message.content)).toEqual(["From the database.", "File-only rule."]);
    expect(harness.entries.map(entry => (entry.data as { ruleId: string }).ruleId)).toEqual(["checkout", "file-only"]);
  });

  it("counts after-gap repeats across separate agent runs", async () => {
    const harness = await createHarness();
    createCosineRulesExtension({
      modelPath: resolve("model"),
      rules: [{ ...sentenceRule, repeat: { mode: "after-gap", gapTurns: 2 } }],
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
    const harness = await createHarness();
    const toolRule: CosineRule = {
      ...sentenceRule,
      id: "tool-input",
      reference: '{"checkout":"Guest checkout remains available."}',
      mode: "log",
      sample: { source: "tool:submit", boundary: "tool-call", minimumTokens: 1 },
      action: { content: "Observe the tool." },
    };
    createCosineRulesExtension({
      modelPath: resolve("model"),
      rules: [toolRule],
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
    const harness = await createHarness();
    const forcePush: CosineRule = {
      id: "no-force-push",
      mode: "log",
      match: { field: "input.command", regex: "\\bgit\\s+push\\b[^\\n]*--force\\b" },
      sample: { source: "tool:bash", boundary: "tool-call" },
      action: { content: "Do not force-push." },
      repeat: { mode: "always" },
    };
    createCosineRulesExtension({
      modelPath: resolve("model"),
      rules: [forcePush],
      registerTool: false,
      registerCommand: false,
    })(harness.pi);
    await harness.handlers.get("session_start")?.({ type: "session_start" }, harness.context);

    const toolCall = (toolCallId: string, command: string) => harness.handlers.get("tool_call")?.(
      { type: "tool_call", toolCallId, toolName: "bash", input: { command } },
      harness.context,
    );
    await toolCall("tool-1", "git push origin feature/auth");
    await toolCall("tool-2", "git push --force origin main");
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
    const harness = await createHarness();
    const notices: Array<{ message: string; level: string }> = [];
    createCosineRulesExtension({
      modelPath: resolve("model"),
      rules: [sentenceRule],
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
    createCosineRulesExtension({
      modelPath: resolve("model"),
      databasePath: configured,
      rules: [sentenceRule],
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
    createCosineRulesExtension({
      modelPath: resolve("model"),
      rules: [sentenceRule],
      registerTool: false,
    })(harness.pi);
    await harness.handlers.get("session_start")?.({ type: "session_start" }, harness.context);
    await harness.handlers.get("before_agent_start")?.(
      { type: "before_agent_start", prompt: "Guest checkout remains available." },
      harness.context,
    );

    expect(harness.messages).toEqual([]);
    const reader = new SqliteTranscriptReader(defaultRerailDatabasePath());
    try {
      expect(reader.session("omp-test-session")).toBeUndefined();
    } finally {
      reader.close();
    }
  });

  it("retires the live owner before rebuilding the same OMP session through cosine reload", async () => {
    const harness = await createHarness();
    createCosineRulesExtension({
      modelPath: resolve("model"),
      rules: [sentenceRule],
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
    const harness = await createHarness();
    createCosineRulesExtension({
      modelPath: resolve("model"),
      rules: [{ ...sentenceRule, repeat: { mode: "always" } }],
      registerTool: false, registerCommand: false,
    })(harness.pi);
    await harness.handlers.get("session_start")?.({}, harness.context);
    const emit = (responseId: string) => harness.handlers.get("message_update")?.({
      message: { role: "assistant", timestamp: 100, responseId },
      assistantMessageEvent: { type: "text_delta", delta: "Guest checkout remains available. Next" },
    }, harness.context);
    await emit("before-reset");
    const databasePath = defaultRerailDatabasePath();
    resetRepositoryHistory(databasePath, resolveRepositoryContext(harness.context.cwd).repositoryId);
    await emit("after-reset");

    expect(harness.messages.map(sent => sent.message.content)).toEqual([
      "Check checkout requirements.", "Check checkout requirements.",
    ]);
    const reader = new SqliteTranscriptReader(databasePath);
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

  describe("attach retry", () => {
    /** A running daemon whose native sessions can be refused by name, and whose next initialize can be held mid-flight. */
    function flakyDaemon(cwd: string) {
      daemonRoute.start = async () => ({
        started: false,
        daemon: { state: { url: "http://127.0.0.1:4737/", nativeToken: "native-token" }, health: { workspaceRoot: cwd } },
      });
      const fetchNative = globalThis.fetch;
      const state = { down: new Set<string>(), hold: undefined as PromiseWithResolvers<void> | undefined };
      const requests: Array<{ operation: string; sessionId: string }> = [];
      vi.stubGlobal("fetch", async (url: URL, init: RequestInit) => {
        if (url.port !== "4737") return fetchNative(url, init);
        const request = z.object({ operation: z.string(), sessionId: z.string() }).parse(JSON.parse(String(init.body)));
        requests.push(request);
        if (request.operation === "initialize" && state.hold !== undefined) {
          const { promise } = state.hold;
          state.hold = undefined;
          await promise;
        }
        if (state.down.has(request.sessionId)) throw new TypeError("fetch failed");
        return Response.json({ result: null, deliveries: [], entries: [] });
      });
      return { state, requests, initializes: (sessionId: string) => requests.filter(r => r.operation === "initialize" && r.sessionId === sessionId).length };
    }

    it("surfaces the first failure, retries quietly with backoff, and reports once when it attaches", async () => {
      const harness = await createHarness();
      const daemon = flakyDaemon(harness.context.cwd);
      vi.useFakeTimers();
      const stderr = vi.spyOn(process.stderr, "write").mockReturnValue(true);
      createCosineRulesExtension({ modelPath: resolve("model"), rules: [sentenceRule], registerTool: false, registerCommand: false })(harness.pi);
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
      expect(stderr).toHaveBeenCalledTimes(1);
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
      createCosineRulesExtension({ modelPath: resolve("model"), rules: [sentenceRule], registerTool: false, registerCommand: false })(harness.pi);
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
      createCosineRulesExtension({ modelPath: resolve("model"), rules: [sentenceRule], registerTool: false, registerCommand: false })(harness.pi);
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
      createCosineRulesExtension({ modelPath: resolve("model"), rules: [sentenceRule], registerTool: false, registerCommand: false })(harness.pi);
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
  });
});
