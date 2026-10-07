import { randomUUID } from "node:crypto";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import type { AgentMessage } from "@oh-my-pi/pi-agent-core";
import type { ExtensionAPI, ExtensionContext, ExtensionFactory } from "@oh-my-pi/pi-coding-agent";
import {
  assistantCompletionEvent,
  assistantDeltaEvent,
  sendNativeDeliveries,
  toolCallEvent,
  toolResultEvent,
  turnEndEvent,
  userPromptEvent,
  type AdapterDelivery,
  type AdapterEvent,
  type AdapterSession,
} from "./adapter-contract.js";
import { registerCosineRulesCommand } from "./command.js";
import { isRerailDisabled } from "./checkout-control.js";
import { registerOmpSemanticTools, type NativeSemanticToolRuntime } from "./native-semantic-tools.js";
import { createDevNativeSession, DevNativeClient, type DevNativeSessionHost } from "./dev-native-client.js";
import type { ExtensionTranscriptOptions } from "./extension-transcript.js";
import type { CosineRuleSourceOptions } from "./rule-sources.js";
import { registerRerailCommands } from "./rerail-command.js";
import { createOmpPresentation } from "./omp-presentation.js";
import { registerCosineRulesTool } from "./tool.js";

export { MATCH_ENTRY_TYPE, MUTATION_ENTRY_TYPE, TURN_ENTRY_TYPE } from "./session-state.js";


export interface CosineRulesOptions extends ExtensionTranscriptOptions, CosineRuleSourceOptions {
  modelPath?: string;
  maxInputTokens?: number;
  maxRules?: number;
  registerTool?: boolean;
  registerCommand?: boolean;
}


/** Installed package root: dist/ and model/ live beside src/. */
export const PACKAGE_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");

export function bundledModelPath(): string {
  return resolve(PACKAGE_ROOT, "model");
}

function streamId(message: AgentMessage): string {
  if (message.role !== "assistant") return `${message.role}:${message.timestamp}`;
  const responseId = message.responseId ?? `${message.provider}:${message.model}`;
  return `${message.timestamp}:${responseId}`;
}

function messageText(message: AgentMessage): string {
  if (!("content" in message)) return "";
  const content = message.content;
  if (typeof content === "string") return content;
  return content.flatMap((item) => (item.type === "text" ? [item.text] : [])).join("\n");
}

/** Attaching starts or resolves a daemon, so retries begin at a second and settle at half a minute. */
const ATTACH_RETRY_MS = 1_000;
const ATTACH_RETRY_CEILING_MS = 30_000;
function attachBackoffMs(failures: number): number {
  return Math.min(ATTACH_RETRY_MS * 2 ** Math.max(failures - 1, 0), ATTACH_RETRY_CEILING_MS);
}

interface InitializeOptions {
  rebuild?: boolean;
  /** Only (re)attach while the context is still the host's current one; a stale caller becomes a no-op. */
  onlyIfCurrent?: boolean;
}

/** The OMP adapter's delivery boundary, shared by live detection and local previews. */
export function deliverOmpGuidance(pi: Pick<ExtensionAPI, "sendMessage">, delivery: AdapterDelivery): Promise<void> {
  // A preview bypasses detection; it records no steering receipt because no runtime session owns it.
  return sendNativeDeliveries([delivery], (message, options) => pi.sendMessage(message, options), () => undefined);
}

export function createCosineRulesExtension(options: CosineRulesOptions = {}): ExtensionFactory {
  return function cosineRulesExtension(pi: ExtensionAPI): void {
    pi.setLabel("Cosine Rules");

    let currentContext: ExtensionContext | undefined;
    let activeSession: AdapterSession | undefined;
    const presentation = createOmpPresentation();
    const streamedMessages = new Set<string>();
    const policyTools = new Map<string, string>();
    let semantic: NativeSemanticToolRuntime | undefined;
    let remote: DevNativeClient | undefined;

    const activeSemantic = (): NativeSemanticToolRuntime => {
      if (semantic === undefined) throw new Error("rerail semantic tools received an event before session_start");
      return semantic;
    };


    const closeRuntimeNow = async (): Promise<void> => {
      const closing = remote;
      remote = undefined;
      activeSession = undefined;
      semantic = undefined;
      streamedMessages.clear();
      policyTools.clear();
      await closing?.close();
    };

    const closeRuntime = (): Promise<void> => {
      const pending = (attaching ?? Promise.resolve()).then(closeRuntimeNow, closeRuntimeNow).finally(() => {
        if (attaching === pending) attaching = undefined;
      });
      attaching = pending;
      return pending;
    };

    const sendDeliveries = (current: DevNativeClient, deliveries: readonly AdapterDelivery[]): Promise<void> =>
      sendNativeDeliveries(deliveries, async (message, options_) => {
        const context = currentContext;
        pi.sendMessage(message, options_);
        if (options_.deliverAs !== "steer" || context === undefined
            || typeof context.hasPendingMessages !== "function"
            || context.isIdle()) return;
        // OMP's public sendMessage is fire-and-forget while the agent is active,
        // but policy guidance must enter the native steer queue before the tool
        // hook releases the agent loop. A microtask checkpoint drains the
        // string-only normalization path; the hook prevents queue consumption.
        await new Promise<void>(resolve_ => queueMicrotask(resolve_));
        if (!context.hasPendingMessages()) throw new Error("OMP did not accept Rerail policy guidance");
      }, (sessionId, receiptId, ruleIds) => current.recordSteeringDelivery(sessionId, receiptId, ruleIds));

    const ingest = async (event: AdapterEvent): Promise<void> => {
      if (remote === undefined) throw new Error("cosine-rules extension received an event before session_start");
      await remote.ingest(event);
    };
    const nativeHost = {
      current: () => remote,
      matchesCurrent: session => activeSession?.sessionId === session.sessionId && activeSession.cwd === session.cwd,
      // Runs inside initializeNow, which already holds `attaching`; the serialized close would deadlock.
      close: closeRuntimeNow,
      activate: (client, context, session) => {
        remote = client;
        currentContext = context;
        activeSession = session;
        semantic = client;
      },
      entries: context => (context ?? currentContext)?.sessionManager.getBranch() ?? [],
      appendEntry: (customType, data) => pi.appendEntry(customType, data),
      deliver: sendDeliveries,
      // Serialized through `attaching` and dropped once its context stopped being current, so a
      // probe that lost the race to a session switch cannot resurrect the old session.
      reattach: context => initialize(context, { onlyIfCurrent: true }),
    } satisfies DevNativeSessionHost<ExtensionContext>;

    /** The initialize in flight, if any: an event that arrives first waits for it instead of failing. */
    let attaching: Promise<void> | undefined;
    // OMP may begin building the next provider context before an async tool hook
    // has crossed its first await. Claim tool boundaries synchronously so the
    // context hook can join all edit work that was announced before it.
    let policyBoundary: Promise<void> = Promise.resolve();
    const sequencePolicyEvent = (operation: () => Promise<void>): Promise<void> => {
      const work = policyBoundary.then(operation, operation);
      policyBoundary = work.then(() => undefined, () => undefined);
      return work;
    };
    const settlePolicyBoundary = async (): Promise<void> => {
      for (;;) {
        const pending = policyBoundary;
        await pending;
        if (pending === policyBoundary) return;
      }
    };

    const initialize = (ctx: ExtensionContext, options_: InitializeOptions = {}): Promise<void> => {
      const start = () => initializeNow(ctx, options_);
      const pending = (attaching ?? Promise.resolve()).then(start, start).finally(() => {
        if (attaching === pending) attaching = undefined;
      });
      attaching = pending;
      return pending;
    };

    // A session that failed to attach (daemon down or wedged at startup) surfaces that failure to
    // the host exactly once, then keeps trying quietly in the background until Rerail is back.
    // Each host-driven attach opens a generation; a retry whose build was in flight when a newer
    // session (or shutdown) arrived belongs to a stale generation and must not reschedule itself.
    let attachTimer: ReturnType<typeof setTimeout> | undefined;
    let attachGeneration = 0;
    let attachFailures = 0;
    const attachAs = async (ctx: ExtensionContext, generation: number): Promise<void> => {
      try {
        await initialize(ctx);
      } catch (error) {
        if (generation !== attachGeneration) throw error;
        attachFailures += 1;
        attachTimer = setTimeout(() => { attachAs(ctx, generation).catch(() => undefined); }, attachBackoffMs(attachFailures));
        attachTimer.unref();
        throw error;
      }
      if (generation !== attachGeneration) return;
      if (attachFailures > 0) process.stderr.write(`Rerail attached after ${attachFailures} failed attempt(s)\n`);
      attachFailures = 0;
    };
    const attach = (ctx: ExtensionContext): Promise<void> => {
      clearTimeout(attachTimer);
      attachFailures = 0;
      return attachAs(ctx, ++attachGeneration);
    };

    /**
     * The session an event belongs to, or undefined when this extension instance owns no session.
     * OMP loads the extension once per agent; a secondary agent (the one that carries a steer
     * delivery in print mode) streams message_update without ever dispatching session_start, and
     * those events are simply not ours. An event that arrives while a session is being built waits
     * for the build, so a delta racing session_start is recorded rather than dropped.
     */
    const sessionFor = async (): Promise<AdapterSession | undefined> => {
      while (attaching !== undefined) await attaching;
      return activeSession;
    };

    const initializeNow = async (ctx: ExtensionContext, options_: InitializeOptions = {}): Promise<void> => {
      if (options_.onlyIfCurrent === true && currentContext !== ctx) return;
      const session = { host: "omp", sessionId: ctx.sessionManager.getSessionId(), cwd: ctx.cwd } satisfies AdapterSession;
      if (isRerailDisabled(ctx.cwd)) {
        presentation.stop();
        await closeRuntimeNow();
        currentContext = ctx;
        return;
      }
      const activation = await createDevNativeSession(nativeHost, ctx, session, options_.rebuild === true, {
        ...options,
        modelPath: resolve(options.modelPath ?? process.env.RERAIL_MODEL_PATH ?? bundledModelPath()),
        ...(options.databasePath === undefined ? {} : { databasePath: resolve(ctx.cwd, options.databasePath) }),
        storageRoot: resolve(ctx.cwd, process.env.RERAIL_STORAGE_ROOT ?? ".rerail/sessions"),
      });
      if (activation === "disabled") {
        presentation.stop();
        currentContext = ctx;
        return;
      }
      if (activation !== "attached") throw new Error("OMP Rerail routing did not attach a runtime");
      presentation.start(ctx);
    };

    if (options.registerTool !== false) {
      registerCosineRulesTool(pi, {
        remoteExecute: () => {
          const current = remote;
          return current === undefined ? undefined : input => current.cosine(input);
        },
      });
      registerOmpSemanticTools(pi, {
        mark: change => activeSemantic().mark(change),
        select: spec => activeSemantic().select(spec),
        project: (selectionId, spec) => activeSemantic().project(selectionId, spec),
        relate: (leftProjectionId, rightProjectionId, spec) => activeSemantic().relate(leftProjectionId, rightProjectionId, spec),
      });
    }
    if (options.registerCommand !== false) {
      registerCosineRulesCommand(pi, {
        remoteCommand: () => {
          const current = remote;
          return current === undefined ? undefined : async args => {
            if (args.trim() === "reload") {
              if (currentContext === undefined) throw new Error("cannot reload cosine rules before session_start");
              await initialize(currentContext, { rebuild: true });
              return remote!.cosine({ action: "list" });
            }
            return current.cosine({ action: "command", args });
          };
        },
        reload: async () => {
          if (currentContext === undefined) throw new Error("cannot reload cosine rules before session_start");
          await initialize(currentContext, { rebuild: true });
        },
      });
      registerRerailCommands(pi, {
        sessionId: () => activeSession?.sessionId,
        packageRoot: PACKAGE_ROOT,
        cwd: () => currentContext?.cwd ?? process.cwd(),
        // Reset retires this session's daemon-owned runtime and reattaches even on failure.
        detach: async () => {
          presentation.stop();
          await closeRuntime();
        },
        reattach: async () => {
          if (currentContext !== undefined) await initialize(currentContext, { rebuild: true });
        },
      });
    }
    pi.on("session_start", (_event, ctx) => attach(ctx));
    pi.on("session_switch", (_event, ctx) => attach(ctx));
    pi.on("session_branch", (_event, ctx) => attach(ctx));
    pi.on("session_tree", (_event, ctx) => attach(ctx));

    pi.on("before_agent_start", async (event) => {
      const session = await sessionFor();
      if (session === undefined) return;
      await ingest(userPromptEvent(session.sessionId, `input:${randomUUID()}`, event.prompt));
    });

    pi.on("context", async (event) => {
      await settlePolicyBoundary();
      const session = activeSession;
      if (session !== undefined) {
        for (const message of event.messages) {
          if (message.role !== "toolResult") continue;
          const toolName = policyTools.get(message.toolCallId);
          if (toolName === undefined) continue;
          policyTools.delete(message.toolCallId);
          await sequencePolicyEvent(() => ingest(toolResultEvent(
            session.sessionId,
            `tool-result:${message.toolCallId}`,
            message.toolCallId,
            toolName,
            { content: message.content, details: message.details },
            message.isError,
          )));
        }
      }
      await settlePolicyBoundary();
    });

    pi.on("message_update", async (event) => {
      if (event.message.role !== "assistant" || event.assistantMessageEvent.type !== "text_delta") return;
      const session = await sessionFor();
      if (session === undefined) return;
      const messageId = streamId(event.message);
      streamedMessages.add(messageId);
      await ingest(assistantDeltaEvent(
        session.sessionId,
        `assistant-delta:${randomUUID()}`,
        messageId,
        event.assistantMessageEvent.delta,
        false,
      ));
    });

    pi.on("message_end", async (event) => {
      if (event.message.role !== "assistant") return;
      const session = await sessionFor();
      if (session === undefined) return;
      const messageId = streamId(event.message);
      const streamed = streamedMessages.delete(messageId);
      await ingest(assistantCompletionEvent(
        session.sessionId,
        messageId,
        streamed,
        () => messageText(event.message),
        event.message.stopReason,
        randomUUID(),
      ));
    });

    // Awaited hooks bracket the actual tool; execution notifications can arrive after the write.
    pi.on("tool_call", (event) => {
      policyTools.set(event.toolCallId, event.toolName);
      return sequencePolicyEvent(async () => {
        const session = await sessionFor();
        if (session === undefined) return;
        await ingest(toolCallEvent(
          session.sessionId,
          `tool-call:${event.toolCallId}`,
          event.toolCallId,
          event.toolName,
          event.input,
        ));
      });
    });
    pi.on("tool_result", (event) => {
      policyTools.delete(event.toolCallId);
      return sequencePolicyEvent(async () => {
        const session = await sessionFor();
        if (session === undefined) return;
        await ingest(toolResultEvent(
          session.sessionId,
          `tool-result:${event.toolCallId}`,
          event.toolCallId,
          event.toolName,
          { content: event.content, details: event.details },
          event.isError,
        ));
      });
    });

    pi.on("turn_end", async (event) => {
      const session = await sessionFor();
      if (session === undefined) return;
      await ingest(turnEndEvent(
        session.sessionId,
        `turn-end:${streamId(event.message)}`,
        messageText(event.message),
      ));
    });

    pi.on("session_shutdown", async (_event) => {
      clearTimeout(attachTimer);
      attachGeneration += 1;
      presentation.stop();
      try {
        await closeRuntime();
      } finally {
        currentContext = undefined;
        semantic = undefined;
      }
    });
  };
}
