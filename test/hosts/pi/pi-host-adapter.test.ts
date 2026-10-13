import type { ExtensionAPI, ExtensionContext } from "@earendil-works/pi-coding-agent";
import { describe, expect, it } from "vitest";
import type { AdapterEvent, AdapterSession } from "../../../src/engine/index.js";
import { registerPiHostAdapter } from "../../../src/hosts/pi/pi-host-adapter.js";

type Handler = (event: unknown, context: ExtensionContext) => unknown;

function lifecycleHarness(changeOwnerBeforeFailure: boolean) {
  const handlers = new Map<string, Handler>();
  const context = {
    cwd: "/workspace",
    sessionManager: { getSessionId: () => "session", getBranch: () => [] },
  } as unknown as ExtensionContext;
  const session: AdapterSession = { host: "pi", sessionId: "session", cwd: context.cwd };
  let owner: object = {};
  let ingested = 0;
  const pi = {
    on(name: string, handler: Handler) { handlers.set(name, handler); },
    registerTool() {},
    registerCommand() {},
  } as unknown as ExtensionAPI;
  registerPiHostAdapter(pi, {
    registerTool: false,
    registerCommand: false,
    activeSession: () => session,
    currentContext: () => context,
    owner: () => owner,
    semantic: () => { throw new Error("not used"); },
    initialize: async () => {
      if (changeOwnerBeforeFailure) owner = {};
      throw new Error("retiring the previous owner failed");
    },
    close: async () => undefined,
    shutdown: async () => undefined,
    sessionFor: async () => session,
    ingest: async (_event: AdapterEvent) => { ingested += 1; },
    conversationRulesTool: {},
    conversationRulesCommand: {},
    packageRoot: "/package-root",
    started: () => true,
    sessionNoteRules: async () => ({ setUp: false }),
  });
  return { context, handlers, ingested: () => ingested };
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
});
