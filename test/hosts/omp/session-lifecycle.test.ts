import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import type { ExtensionAPI as OmpAPI, ExtensionContext as OmpContext } from "@oh-my-pi/pi-coding-agent";
import type { ExtensionAPI as PiAPI, ExtensionContext as PiContext } from "@earendil-works/pi-coding-agent";
import { afterEach, describe, expect, it, vi } from "vitest";
import { z } from "zod";
import { createConversationRulesExtension } from "../../../src/hosts/omp/extension.js";
import { createPiConversationRulesExtension } from "../../../src/hosts/pi/index.js";
import { CodeRuleBridge, type ConversationRule, linkSteers, steerBlock, steerOrigin } from "../../../src/engine/index.js";
import { MATCH_ENTRY_TYPE, TURN_ENTRY_TYPE } from "../../../src/runtime/index.js";
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
  if (host === "omp") await startNativeTestDaemon(cwd);
  const handlers = new Map<string, Handler>();
  const commands = new Map<string, { handler(args: string, context: Context): unknown }>();
  const messages: unknown[] = [];
  const entries: Array<{ customType: string; data: unknown }> = [];
  const branch: unknown[] = [];
  const api = {
    zod: z,
    setLabel() {},
    on(name: string, handler: Handler) { handlers.set(name, handler); },
    registerCommand(name: string, command: { handler(args: string, context: Context): unknown }) { commands.set(name, command); },
    registerTool() {},
    registerMessageRenderer() {},
    sendMessage(message: unknown) { messages.push(message); },
    appendEntry(customType: string, data: unknown) { entries.push({ customType, data }); },
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
    context, branch, messages, entries,
    emit: (name: string, event: unknown, ctx = context) => live(() => handlers.get(name)!(event, ctx)),
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
  it("reports startup failure to racing events, ignores later orphan deltas, and attaches on retry", async () => {
    const extension = await harness(host);
    const entered = Promise.withResolvers<void>();
    const release = Promise.withResolvers<void>();
    const failure = new Error("Workspace attachment unavailable");
    vi.spyOn(CodeRuleBridge.prototype, "openWorkspace").mockImplementationOnce(async () => {
      entered.resolve();
      await release.promise;
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
      const failure = new Error("Workspace reload unavailable");
      vi.spyOn(CodeRuleBridge.prototype, "openWorkspace").mockRejectedValueOnce(failure);
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
    const release = Promise.withResolvers<void>();
    const failure = new Error("First workspace unavailable");
    vi.spyOn(CodeRuleBridge.prototype, "openWorkspace").mockImplementationOnce(async () => {
      entered.resolve();
      await release.promise;
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
