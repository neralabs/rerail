import { randomUUID } from "node:crypto";
import type { ExtensionAPI, ExtensionContext } from "@earendil-works/pi-coding-agent";
import {
  assistantCompletionEvent,
  assistantDeltaEvent,
  toolCallEvent,
  toolResultEvent,
  turnEndEvent,
  userPromptEvent,
  type AdapterEvent,
  type AdapterSession,
} from "./adapter-contract.js";
import { registerPiSemanticTools, type NativeSemanticToolRuntime } from "./native-semantic-tools.js";
import { registerPiCosineRulesCommand, type PiCosineRulesCommandRuntime } from "./pi-command.js";
import { registerRerailCommands } from "./rerail-command.js";
import { registerPiCosineRulesTool, type PiCosineRulesToolRuntime } from "./pi-tool.js";

interface PiMessage {
  role: string;
  timestamp: number;
  responseId?: string;
  provider?: string;
  model?: string;
  content?: unknown;
}

export interface PiHostAdapterRuntime {
  registerTool?: boolean;
  registerCommand?: boolean;
  activeSession(): AdapterSession | undefined;
  currentContext(): ExtensionContext | undefined;
  owner(): object | undefined;
  semantic(): NativeSemanticToolRuntime;
  initialize(context: ExtensionContext, rebuild: boolean): Promise<void>;
  close(): Promise<void>;
  shutdown(): Promise<void>;
  sessionFor(): Promise<AdapterSession | undefined>;
  ingest(event: AdapterEvent): Promise<void>;
  cosineTool: PiCosineRulesToolRuntime;
  cosineCommand: Omit<PiCosineRulesCommandRuntime, "reload">;
  packageRoot: string;
  /** Whether Rerail runs in `cwd`; where it does not, input arriving without a session is ignored. */
  started(cwd: string): boolean;
  /** A database configured in place of the Rerail home's, which `/rerail-start` and `/rerail-stop` must change. */
  configuredDatabase?(cwd: string): string | undefined;
}

function streamId(message: PiMessage): string {
  if (message.role !== "assistant") return `${message.role}:${message.timestamp}`;
  const responseId = message.responseId ?? `${message.provider}:${message.model}`;
  return `${message.timestamp}:${responseId}`;
}

function messageText(message: PiMessage): string {
  const content = message.content;
  if (typeof content === "string") return content;
  if (!Array.isArray(content)) return "";
  return content.flatMap((item: unknown) => (
    typeof item === "object"
      && item !== null
      && "type" in item
      && item.type === "text"
      && "text" in item
      && typeof item.text === "string"
      ? [item.text]
      : []
  )).join("\n");
}

function rememberCompleted(ids: Map<string, undefined>, id: string): void {
  ids.delete(id);
  ids.set(id, undefined);
  if (ids.size > 1_024) ids.delete(ids.keys().next().value!);
}


/** Registers Pi's host surface once; runtime ownership decides between local and development-container execution. */
export function registerPiHostAdapter(pi: ExtensionAPI, runtime: PiHostAdapterRuntime): void {
  const streamedMessages = new Map<string, undefined>();
  const completedStreams = new Map<string, undefined>();
  const completedTurns = new Map<string, undefined>();
  const resetEventState = (): void => {
    streamedMessages.clear();
    completedStreams.clear();
    completedTurns.clear();
  };
  const initialize = async (context: ExtensionContext, rebuild = false): Promise<void> => {
    const previousOwner = runtime.owner();
    let initialized = false;
    try {
      await runtime.initialize(context, rebuild);
      initialized = true;
    } finally {
      if (previousOwner !== runtime.owner() || (initialized && rebuild)) resetEventState();
    }
  };
  const close = async (): Promise<void> => {
    try {
      await runtime.close();
    } finally {
      resetEventState();
    }
  };

  if (runtime.registerTool !== false) {
    registerPiCosineRulesTool(pi, runtime.cosineTool);
    registerPiSemanticTools(pi, {
      mark: change => runtime.semantic().mark(change),
      select: spec => runtime.semantic().select(spec),
      project: (selectionId, spec) => runtime.semantic().project(selectionId, spec),
      relate: (leftProjectionId, rightProjectionId, spec) => runtime.semantic().relate(leftProjectionId, rightProjectionId, spec),
    });
  }
  if (runtime.registerCommand !== false) {
    registerPiCosineRulesCommand(pi, {
      ...runtime.cosineCommand,
      reload: async () => {
        const context = runtime.currentContext();
        if (context === undefined) throw new Error("cannot reload cosine rules before session_start");
        await initialize(context, true);
      },
    });
    registerRerailCommands(pi, {
      packageRoot: runtime.packageRoot,
      host: "pi",
      cwd: () => runtime.currentContext()?.cwd ?? process.cwd(),
      databasePath: () => runtime.configuredDatabase?.(runtime.currentContext()?.cwd ?? process.cwd()),
      sessionId: () => runtime.activeSession()?.sessionId,
      detach: close,
      reattach: async () => {
        const context = runtime.currentContext();
        if (context !== undefined) await initialize(context, true);
      },
    });
  }

  pi.on("session_start", (_event, context) => initialize(context));
  pi.on("session_tree", (_event, context) => initialize(context));
  pi.on("input", async event => {
    const session = await runtime.sessionFor();
    if (session === undefined) {
      const context = runtime.currentContext();
      if (context !== undefined && !runtime.started(context.cwd)) return;
      throw new Error("cosine-rules extension received input before session_start");
    }
    await runtime.ingest(userPromptEvent(session.sessionId, `input:${randomUUID()}`, event.text));
  });
  pi.on("message_update", async event => {
    if (event.message.role !== "assistant" || event.assistantMessageEvent.type !== "text_delta") return;
    const session = await runtime.sessionFor();
    if (session === undefined) return;
    const messageId = streamId(event.message);
    streamedMessages.set(messageId, undefined);
    await runtime.ingest(assistantDeltaEvent(
      session.sessionId,
      `assistant-delta:${randomUUID()}`,
      messageId,
      event.assistantMessageEvent.delta,
      false,
    ));
  });
  pi.on("message_end", async event => {
    if (event.message.role !== "assistant") return;
    const session = await runtime.sessionFor();
    if (session === undefined) return;
    const messageId = streamId(event.message);
    if (completedStreams.has(messageId)) return;
    rememberCompleted(completedStreams, messageId);
    const streamed = streamedMessages.delete(messageId);
    await runtime.ingest(assistantCompletionEvent(
      session.sessionId,
      messageId,
      streamed,
      () => messageText(event.message),
      event.message.stopReason,
      randomUUID(),
    ));
  });
  pi.on("tool_call", async event => {
    const session = await runtime.sessionFor();
    if (session === undefined) return;
    await runtime.ingest(toolCallEvent(
      session.sessionId,
      `tool-call:${event.toolCallId}`,
      event.toolCallId,
      event.toolName,
      event.input,
    ));
  });
  pi.on("tool_result", async event => {
    const session = await runtime.sessionFor();
    if (session === undefined) return;
    await runtime.ingest(toolResultEvent(
      session.sessionId,
      `tool-result:${event.toolCallId}`,
      event.toolCallId,
      event.toolName,
      event.content,
      event.isError,
    ));
  });
  pi.on("turn_end", async event => {
    const session = await runtime.sessionFor();
    if (session === undefined) return;
    const messageId = streamId(event.message);
    if (completedTurns.has(messageId)) return;
    rememberCompleted(completedTurns, messageId);
    await runtime.ingest(turnEndEvent(session.sessionId, `turn-end:${messageId}`, messageText(event.message)));
  });
  pi.on("session_shutdown", async () => {
    try {
      await runtime.shutdown();
    } finally {
      resetEventState();
    }
  });
}
