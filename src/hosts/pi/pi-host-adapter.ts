import { randomUUID } from "node:crypto";
import type { ExtensionAPI, ExtensionContext } from "@earendil-works/pi-coding-agent";
import {
  assistantCompletionEvent,
  assistantDeltaEvent,
  callEvent,
  resultEvent,
  turnEndEvent,
  userPromptEvent,
  resultWithSteers,
  type AdapterDelivery,
  type AdapterEvent,
  type AdapterSession,
} from "../../engine/adapter-contract.js";
import { AttachRetry, type AttachRetryHost } from "../../harness/attach-retry.js";
import type { NativeSemanticToolRuntime } from "../../harness/native-semantic-tools.js";
import { registerPiSemanticTools } from "./pi-semantic-tools.js";
import { registerPiConversationRulesCommand, type PiConversationRulesCommandRuntime } from "./pi-command.js";
import { contained } from "../../harness/contained-hook.js";
import { guardCompaction } from "../../harness/compaction-guard.js";
import { registerHostCommands } from "../../harness/host-commands.js";
import { SESSION_NOTE_MESSAGE_TYPE, sessionStartNote, type SessionNoteRules } from "../../engine/code-rule/rerail-steer.js";
import { dashboardOrigin } from "../../presentation/session-presentation.js";
import { registerPiConversationRulesTool, type PiConversationRulesToolRuntime } from "./pi-tool.js";
import { conversationRulesEnabled, semanticToolsEnabled } from "../../engine/experimental-features.js";

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
  /** How the session Pi is in shows and records what it could not hand over while its attach is failing. */
  attachGaps: Pick<AttachRetryHost<ExtensionContext>, "reportDropped" | "failed" | "recorder">;
  /** Gives the attached client's open gap marker its count so far, for Pi about to leave session `sessionId`. */
  markGapFinal(sessionId: string): void;
  ingest(event: AdapterEvent): Promise<void>;
  /** Hands Pi the session's steers due at the end of the agent's run, with the digest of those that missed their step. */
  yieldDeliveries(sessionId: string): Promise<void>;
  conversationRulesTool: PiConversationRulesToolRuntime;
  conversationRulesCommand: Omit<PiConversationRulesCommandRuntime, "reload">;
  packageRoot: string;
  /** Whether Rerail runs in `cwd`; where it does not, input arriving without a session is ignored. */
  started(cwd: string): boolean;
  /** A database configured in place of the Rerail home's, which `/rerail-start` and `/rerail-stop` must change. */
  configuredDatabase?(cwd: string): string | undefined;
  /** What the session-start note says about `session`'s rules. */
  sessionNoteRules(session: AdapterSession): Promise<SessionNoteRules>;
  /**
   * Opens `toolCallId`'s result to the steers for that call while it is being ingested; the returned function closes
   * it and yields what was placed, so a later steer for the call goes out as a message.
   */
  openToolResult(toolCallId: string): () => readonly AdapterDelivery[];
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


/** Registers Pi's host surface once; runtime ownership decides between in-process and daemon execution. */
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
  const attachRetry = new AttachRetry<ExtensionContext>({
    initialize,
    sessionId: context => context.sessionManager.getSessionId(),
    appendEntry: (customType, data) => pi.appendEntry(customType, data),
    ...runtime.attachGaps,
  });
  /** The session an event of Pi's goes to: none while the session Pi is in is failing to attach, though the runtime of a session Pi left survived. */
  const liveSession = async (): Promise<AdapterSession | undefined> => {
    const session = await runtime.sessionFor();
    const failing = attachRetry.failing();
    return failing === undefined || failing.sessionManager.getSessionId() === session?.sessionId ? session : undefined;
  };
  /**
   * The live session for the event `build` makes. While the session Pi is in is failing to attach, the event is counted
   * as dropped instead, and the runtime that attaches records the count; one that waited on a try that failed still
   * rejects with that failure, as OMP's does.
   */
  const sessionOrDrop = async (build: (sessionId: string) => AdapterEvent): Promise<AdapterSession | undefined> => {
    let session: AdapterSession | undefined;
    try {
      session = await liveSession();
    } catch (error) {
      attachRetry.countDrop(build);
      throw error;
    }
    if (session === undefined) attachRetry.countDrop(build);
    return session;
  };
  const ingestEvent = async (build: (sessionId: string) => AdapterEvent): Promise<AdapterSession | undefined> => {
    const session = await sessionOrDrop(build);
    if (session !== undefined) await runtime.ingest(build(session.sessionId));
    return session;
  };
  const close = async (): Promise<void> => {
    // A detached Pi stays detached: a retry armed by a failed attach would start Rerail again while it is being removed.
    attachRetry.cancel();
    try {
      await runtime.close();
    } finally {
      resetEventState();
    }
  };

  // Internal testing: each family of tools and `/cosine-rules` only while its experiment is on in Pi's process.
  if (runtime.registerTool !== false && conversationRulesEnabled()) registerPiConversationRulesTool(pi, runtime.conversationRulesTool);
  if (runtime.registerTool !== false && semanticToolsEnabled()) {
    registerPiSemanticTools(pi, {
      mark: change => runtime.semantic().mark(change),
      select: spec => runtime.semantic().select(spec),
      project: (selectionId, spec) => runtime.semantic().project(selectionId, spec),
      relate: (leftProjectionId, rightProjectionId, spec) => runtime.semantic().relate(leftProjectionId, rightProjectionId, spec),
    });
  }
  if (runtime.registerCommand !== false && conversationRulesEnabled()) {
    registerPiConversationRulesCommand(pi, {
      ...runtime.conversationRulesCommand,
      reload: async () => {
        const context = runtime.currentContext();
        if (context === undefined) throw new Error("cannot reload cosine rules before session_start");
        await initialize(context, true);
      },
    });
  }
  if (runtime.registerCommand !== false) {
    registerHostCommands(pi, {
      packageRoot: runtime.packageRoot,
      host: "pi",
      cwd: () => runtime.currentContext()?.cwd ?? process.cwd(),
      databasePath: () => runtime.configuredDatabase?.(runtime.currentContext()?.cwd ?? process.cwd()),
      sessionId: () => runtime.activeSession()?.sessionId,
      detach: close,
      reattach: () => attachRetry.resume(runtime.currentContext()),
    });
  }

  pi.on("session_start", (_event, context) => attachRetry.attach(context));
  pi.on("session_tree", (_event, context) => attachRetry.attach(context));
  // The session Pi is leaving gets its gap markers' counts while it is still the one Pi appends to: a failing attach's,
  // and the attached client's open one, which the switch that follows closes into a session it no longer writes to.
  const markGapsFinal = (context: ExtensionContext): void => {
    attachRetry.markFinal(context);
    runtime.markGapFinal(context.sessionManager.getSessionId());
  };
  pi.on("session_before_switch", (_event, context) => { markGapsFinal(context); });
  pi.on("session_before_fork", (_event, context) => { markGapsFinal(context); });
  // Pi compacts the preparation its handlers received (pi-coding-agent 1.1.0 `AgentSession.compact`/`_runAutoCompaction`).
  pi.on("session_before_compact", event => { guardCompaction(event.preparation); });
  pi.on("input", async event => {
    const session = await ingestEvent(sessionId => userPromptEvent(sessionId, `input:${randomUUID()}`, event.text));
    // Pi sends the prompt on whatever this does: one of a session failing to attach was counted as dropped.
    if (session !== undefined || attachRetry.failing() !== undefined) return;
    const context = runtime.currentContext();
    if (context !== undefined && !runtime.started(context.cwd)) return;
    throw new Error("cosine-rules extension received input before session_start");
  });
  // The first prompt of each session also carries the session-start note, hidden from the person (design A1).
  const notedSessions = new Set<string>();
  pi.on("before_agent_start", async () => {
    const session = await liveSession();
    if (session === undefined || notedSessions.has(session.sessionId)) return;
    notedSessions.add(session.sessionId);
    const content = sessionStartNote(await dashboardOrigin(session.cwd), "pi", session.sessionId, await runtime.sessionNoteRules(session));
    return { message: { customType: SESSION_NOTE_MESSAGE_TYPE, content, display: false } };
  });
  pi.on("message_update", async event => {
    if (event.message.role !== "assistant" || event.assistantMessageEvent.type !== "text_delta") return;
    const messageId = streamId(event.message);
    const text = event.assistantMessageEvent.delta;
    const delta = (sessionId: string): AdapterEvent => assistantDeltaEvent(sessionId, `assistant-delta:${randomUUID()}`, messageId, text, false);
    const session = await sessionOrDrop(delta);
    if (session === undefined) return;
    streamedMessages.set(messageId, undefined);
    await runtime.ingest(delta(session.sessionId));
  });
  pi.on("message_end", async event => {
    const message = event.message;
    if (message.role !== "assistant") return;
    const messageId = streamId(message);
    const completion = (sessionId: string, streamed = false): AdapterEvent => assistantCompletionEvent(
      sessionId,
      messageId,
      streamed,
      () => messageText(message),
      message.stopReason,
      randomUUID(),
    );
    const session = await sessionOrDrop(completion);
    if (session === undefined) return;
    if (completedStreams.has(messageId)) return;
    rememberCompleted(completedStreams, messageId);
    await runtime.ingest(completion(session.sessionId, streamedMessages.delete(messageId)));
  });
  // A tool hook must never block the tool: Pi fails a `tool_call` handler that throws closed.
  pi.on("tool_call", event => contained("tool_call", (async () => {
    await ingestEvent(sessionId => callEvent(
      sessionId,
      `tool-call:${event.toolCallId}`,
      event.toolCallId,
      event.toolName,
      event.input,
    ));
  })()));
  pi.on("tool_result", async event => {
    const close = runtime.openToolResult(event.toolCallId);
    await contained("tool_result", (async () => {
      await ingestEvent(sessionId => resultEvent(
        sessionId,
        `tool-result:${event.toolCallId}`,
        event.toolCallId,
        event.toolName,
        event.content,
        event.isError,
      ));
    })());
    // The call's steers ride its result: after what it says, or in its place when its edit was rolled back.
    return resultWithSteers(event.content, close());
  });
  pi.on("turn_end", async event => {
    const messageId = streamId(event.message);
    const turnEnd = (sessionId: string): AdapterEvent => turnEndEvent(sessionId, `turn-end:${messageId}`, messageText(event.message));
    const session = await sessionOrDrop(turnEnd);
    if (session === undefined) return;
    if (completedTurns.has(messageId)) return;
    rememberCompleted(completedTurns, messageId);
    await runtime.ingest(turnEnd(session.sessionId));
  });
  // The end of the agent's run, which Pi's turn end is not (it ends one at every model call). Pi's last actionable
  // boundary: a steer sent here (`deliverAs: "steer"`, still inside the run) is queued, and Pi continues the run with
  // it (pi-coding-agent 1.1.0 `AgentSession._runBeforeSettleBoundary`: `result.continue || agent.hasQueuedMessages()`).
  pi.on("agent_before_settle", () => contained("agent_before_settle", (async () => {
    const session = await liveSession();
    if (session === undefined) return;
    await runtime.yieldDeliveries(session.sessionId);
  })()));
  pi.on("session_shutdown", async (_event, context) => {
    attachRetry.stop(context);
    try {
      await runtime.shutdown();
    } finally {
      resetEventState();
    }
  });
}
