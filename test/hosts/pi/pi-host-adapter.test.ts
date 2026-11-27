import type { ExtensionAPI, ExtensionContext } from "@earendil-works/pi-coding-agent";
import { describe, expect, it, vi } from "vitest";
import { type AdapterDelivery, type AdapterEvent, type AdapterSession, sendNativeDeliveries } from "../../../src/engine/index.js";
import { registerPiHostAdapter } from "../../../src/hosts/pi/pi-host-adapter.js";

/** The detach the adapter hands Pi's commands, which `/rerail-uninstall` runs before it removes Rerail. */
const hostCommands = vi.hoisted(() => ({ detach: undefined as (() => Promise<void>) | undefined }));
vi.mock("../../../src/harness/host-commands.js", async importOriginal => ({
  ...await importOriginal<Record<string, unknown>>(),
  registerHostCommands: (_pi: unknown, options: { detach?(): Promise<void> }) => { hostCommands.detach = options.detach; },
}));

type Handler = (event: unknown, context: ExtensionContext) => unknown;

function lifecycleHarness(changeOwnerBeforeFailure: boolean, registerCommand = false) {
  const handlers = new Map<string, Handler>();
  const context = {
    cwd: "/workspace",
    sessionManager: { getSessionId: () => "session", getBranch: () => [] },
  } as unknown as ExtensionContext;
  const session: AdapterSession = { host: "pi", sessionId: "session", cwd: context.cwd };
  let owner: object = {};
  let ingested = 0;
  let tries = 0;
  const pi = {
    on(name: string, handler: Handler) { handlers.set(name, handler); },
    registerTool() {},
    registerCommand() {},
  } as unknown as ExtensionAPI;
  registerPiHostAdapter(pi, {
    registerTool: false,
    registerCommand,
    activeSession: () => session,
    currentContext: () => context,
    owner: () => owner,
    semantic: () => { throw new Error("not used"); },
    initialize: async () => {
      tries += 1;
      if (changeOwnerBeforeFailure) owner = {};
      throw new Error("retiring the previous owner failed");
    },
    close: async () => undefined,
    shutdown: async () => undefined,
    sessionFor: async () => session,
    attachGaps: { reportDropped() {}, failed() {}, recorder: () => undefined },
    markGapFinal() {},
    ingest: async (_event: AdapterEvent) => { ingested += 1; },
    yieldDeliveries: async () => undefined,
    conversationRulesTool: {},
    conversationRulesCommand: {},
    packageRoot: "/package-root",
    started: () => true,
    sessionNoteRules: async () => ({ setUp: false }),
    openToolResult: () => () => [],
  });
  return { context, handlers, ingested: () => ingested, tries: () => tries };
}

const completedMessage = {
  type: "message_end",
  message: {
    role: "assistant",
    timestamp: 100,
    responseId: "response-1",
    content: [{ type: "text", text: "Completed answer" }],
  },
};

describe("Pi host adapter owner transitions", () => {
  it("resets completed-event state when initialization attaches a new owner before failing", async () => {
    const harness = lifecycleHarness(true);
    await harness.handlers.get("message_end")?.(completedMessage, harness.context);
    await expect(Promise.resolve(harness.handlers.get("session_start")?.({}, harness.context))).rejects.toThrow(
      "retiring the previous owner failed",
    );
    await harness.handlers.get("message_end")?.(completedMessage, harness.context);

    expect(harness.ingested()).toBe(2);
  });

  it("retains completed-event state when failed initialization leaves the owner unchanged", async () => {
    const harness = lifecycleHarness(false);
    await harness.handlers.get("message_end")?.(completedMessage, harness.context);
    await expect(Promise.resolve(harness.handlers.get("session_start")?.({}, harness.context))).rejects.toThrow(
      "retiring the previous owner failed",
    );
    await harness.handlers.get("message_end")?.(completedMessage, harness.context);

    expect(harness.ingested()).toBe(1);
  });

  it("tries no more to attach once Pi detaches for Rerail's removal, after a failed start armed a retry", async () => {
    vi.useFakeTimers();
    try {
      const harness = lifecycleHarness(false, true);
      await expect(Promise.resolve(harness.handlers.get("session_start")?.({}, harness.context))).rejects.toThrow(
        "retiring the previous owner failed",
      );
      await hostCommands.detach!();
      await vi.advanceTimersByTimeAsync(60_000);

      expect(harness.tries()).toBe(1);
    } finally {
      vi.useRealTimers();
    }
  });
});

describe("native steers", () => {
  it("sends a note naming no rule (a Merge Assist note), with its conflict for the person's line, and records no steer for it, as it does for a rule's steer", async () => {
    const steer = (content: string, ruleIds: readonly string[]): AdapterDelivery => ({
      sessionId: "session", requested: "steer", mechanism: "native-queue", content, ruleIds, rules: [],
    });
    const conflict = { id: "conflict:a", text: "your edit overlaps jdoe-1: retry.ts" };
    const sent: Array<{ content: string; details: object }> = [];
    const receipts: Array<readonly string[]> = [];
    await sendNativeDeliveries(
      [{ ...steer("Your edit overlaps open work.", []), conflict }, steer("Fix a.", ["a"])],
      message => { sent.push({ content: message.content, details: message.details }); },
      (_sessionId, _receiptId, ruleIds) => { receipts.push(ruleIds); },
    );
    expect(sent).toEqual([
      { content: "Your edit overlaps open work.", details: expect.objectContaining({ conflict }) },
      { content: "Fix a.", details: expect.not.objectContaining({ conflict: expect.anything() }) },
    ]);
    expect(receipts).toEqual([["a"]]);
  });
});
