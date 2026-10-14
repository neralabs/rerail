import { execFile } from "node:child_process";
import { randomUUID } from "node:crypto";
import { existsSync } from "node:fs";
import { join, resolve } from "node:path";
import { promisify } from "node:util";
import { pathToFileURL } from "node:url";
import type { AgentMessage } from "@oh-my-pi/pi-agent-core";
import type { ExtensionAPI, ExtensionContext, ExtensionFactory } from "@oh-my-pi/pi-coding-agent";
import type * as RepositoryStart from "../../runtime/repository-start.js";
import {
  assistantAbortEvent,
  assistantAborted,
  assistantCompletionEvent,
  assistantDeltaEvent,
  assistantMessageEvent,
  sendNativeDeliveries,
  callEvent,
  resultEvent,
  turnEndEvent,
  userPromptEvent,
  type AdapterDelivery,
  type AdapterEvent,
  type AdapterSession,
} from "../../engine/adapter-contract.js";
import { bundledModelPath, ENTRY_POINTS, PACKAGE_ROOT } from "../../engine/package-root.js";
import { registerConversationRulesCommand } from "./command.js";
import { nodeExecutable } from "../../runtime/daemon-control.js";
import { registerOmpSemanticTools, type NativeSemanticToolRuntime } from "../../harness/native-semantic-tools.js";
import { countDroppedEvent, createNativeSession, NativeSessionClient, type NativeSessionHost } from "../../harness/native-session-client.js";
import type { SessionDatabaseOptions } from "../../harness/session-database-options.js";
import { contained } from "../../harness/contained-hook.js";
import { registerHostCommands } from "../../harness/host-commands.js";
import { createOmpPresentation } from "../../presentation/omp-presentation.js";
import { dashboardOrigin, linkDeliveries, presentedSessionNoteRules, readRepositoryState } from "../../presentation/session-presentation.js";
import { SESSION_NOTE_MESSAGE_TYPE, sessionStartNote, type SessionNoteRules } from "../../engine/code-rule/rerail-steer.js";
import { registerConversationRulesTool } from "./tool.js";
import { registerOmpSteerRenderer } from "./steer-renderer.js";
import { EVENTS_DROPPED_ENTRY_TYPE, type DroppedEvents } from "../../runtime/session-state.js";

export { EVENTS_DROPPED_ENTRY_TYPE, MATCH_ENTRY_TYPE, MUTATION_ENTRY_TYPE, TURN_ENTRY_TYPE } from "../../runtime/session-state.js";


export interface ConversationRulesOptions extends SessionDatabaseOptions {
  modelPath?: string;
  maxInputTokens?: number;
  maxRules?: number;
  registerTool?: boolean;
  registerCommand?: boolean;
}

const execFileAsync = promisify(execFile);
/** A stuck `rerail state` (a held SQLite lock, a slow git) fails the attach, which retries, instead of hanging it. */
const STATE_CHECK_TIMEOUT_MS = 15_000;

/**
 * The built package's start module under Node, loaded at run time so this bundle never links the SQLite binding;
 * undefined on Bun (OMP), which cannot load that binding, and without a built `dist/`.
 */
async function databaseReader(): Promise<typeof RepositoryStart | undefined> {
  const readerPath = join(PACKAGE_ROOT, ENTRY_POINTS.repositoryStart);
  if (process.versions.bun !== undefined || !existsSync(readerPath)) return undefined;
  // Runtime-selected on purpose: a static import would link better-sqlite3 into the Bun-loaded bundle.
  return await import(pathToFileURL(readerPath).href) as typeof RepositoryStart;
}

/**
 * Whether Rerail is started in `cwd`. Under Node the built package reads the database itself. On Bun a
 * running daemon answers, and otherwise the CLI does under Node, reading the configured database when one
 * is given. Without a built CLI, Rerail is off.
 */
async function rerailStarted(cwd: string, databasePath: string | undefined): Promise<boolean> {
  const reader = await databaseReader();
  if (reader !== undefined) return reader.isRerailStarted(cwd, databasePath);
  if (databasePath === undefined) {
    const answered = await readRepositoryState(cwd, "omp");
    if (answered !== null) return answered !== "stopped";
  }
  const cli = join(PACKAGE_ROOT, ENTRY_POINTS.cli);
  if (!existsSync(cli)) return false;
  const databaseArgs = databasePath === undefined ? [] : ["--database", databasePath];
  const { stdout } = await execFileAsync(nodeExecutable(), [cli, "state", "--repo", cwd, ...databaseArgs], { cwd, timeout: STATE_CHECK_TIMEOUT_MS });
  return ["started", "not-set-up"].includes(stdout.trim());
}

/**
 * What the session-start note says about the session's rules: the database's answer under Node, else the
 * daemon's, which counts the rules on for the session as the footer does. The daemon reads only its own database,
 * so with a configured one the CLI answers under Node instead (set up or not; the count is left out).
 */
async function sessionNoteRulesFor(session: AdapterSession, databasePath: string | undefined): Promise<SessionNoteRules> {
  const reader = await databaseReader();
  if (reader !== undefined) return reader.sessionNoteRules(session.cwd, databasePath);
  if (databasePath === undefined) return presentedSessionNoteRules(session);
  const cli = join(PACKAGE_ROOT, ENTRY_POINTS.cli);
  if (!existsSync(cli)) return { setUp: false };
  const { stdout } = await execFileAsync(nodeExecutable(), [cli, "state", "--set-up", "--repo", session.cwd, "--database", databasePath], {
    cwd: session.cwd, timeout: STATE_CHECK_TIMEOUT_MS,
  });
  return stdout.trim() === "set-up" ? { setUp: true, ruleCount: null } : { setUp: false };
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
  // A preview bypasses detection; it records no steer receipt because no runtime session owns it.
  return sendNativeDeliveries([delivery], (message, options) => pi.sendMessage(message, options), () => undefined);
}

export function createConversationRulesExtension(options: ConversationRulesOptions = {}): ExtensionFactory {
  return function conversationRulesExtension(pi: ExtensionAPI): void {
    pi.setLabel("Cosine Rules");
    registerOmpSteerRenderer(pi);

    let currentContext: ExtensionContext | undefined;
    let activeSession: AdapterSession | undefined;
    const presentation = createOmpPresentation();
    /** Messages with a delta the daemon took, and with a delta counted as dropped instead. */
    const streamedMessages = new Set<string>();
    const droppedDeltaMessages = new Set<string>();
    /** The delta deliveries of each streaming message that have not settled yet. */
    const deltasInFlight = new Map<string, Promise<unknown>>();
    const codeRuleTools = new Map<string, string>();
    let semantic: NativeSemanticToolRuntime | undefined;
    let remote: NativeSessionClient | undefined;
    /** The session OMP is in: set when a session event delivers it, so a runtime of a session OMP left cannot write into this one. */
    let liveSessionId: string | undefined;

    const activeSemantic = (): NativeSemanticToolRuntime => {
      if (semantic === undefined) throw new Error("rerail semantic tools received an event before session_start");
      return semantic;
    };


    const closeRuntimeNow = async (): Promise<void> => {
      const closing = remote;
      remote = undefined;
      activeSession = undefined;
      semantic = undefined;
      // The runtime that replaces this one saw none of the deltas of the messages streaming now, so each is marked dropped
      // for its end to send the whole text again; the deltas still in flight settle against a runtime that is no longer live.
      for (const messageId of streamedMessages) droppedDeltaMessages.add(messageId);
      streamedMessages.clear();
      codeRuleTools.clear();
      await closing?.close();
    };

    const closeRuntime = (): Promise<void> => {
      const pending = (attaching ?? Promise.resolve()).then(closeRuntimeNow, closeRuntimeNow).finally(() => {
        if (attaching === pending) attaching = undefined;
      });
      attaching = pending;
      return pending;
    };

    // The runtime keeps steer links relative; they point at this checkout's dashboard only here.
    const sendDeliveries = async (current: NativeSessionClient, deliveries: readonly AdapterDelivery[]): Promise<void> =>
      sendNativeDeliveries(await linkDeliveries(deliveries, currentContext?.cwd), async (message, options_) => {
        const context = currentContext;
        pi.sendMessage(message, options_);
        if (context === undefined
            || typeof context.hasPendingMessages !== "function"
            || context.isIdle()) return;
        // OMP's public sendMessage is fire-and-forget while the agent is active,
        // but code-rule guidance must enter the native steer queue before the tool
        // hook releases the agent loop. A microtask checkpoint drains the
        // string-only normalization path; the hook prevents queue consumption.
        await new Promise<void>(resolve_ => queueMicrotask(resolve_));
        if (!context.hasPendingMessages()) throw new Error("OMP did not accept Rerail policy guidance");
      }, (sessionId, receiptId, ruleIds) => current.recordSteer(sessionId, receiptId, ruleIds));

    /**
     * Hands the event to the live runtime. `outcome` learns whether the event reached the daemon through the runtime that is
     * still live afterwards: false is a resolved false (the open breaker counted it as dropped) or a runtime replaced meanwhile,
     * whose successor never saw it. A rejection comes only after the daemon took the event (the host refused something it
     * asked for), so it counts as delivered.
     */
    const ingestDelivered = async (event: AdapterEvent, outcome?: (delivered: boolean) => void): Promise<void> => {
      const client = remote;
      if (client === undefined) throw new Error("cosine-rules extension received an event before session_start");
      let delivered: boolean;
      try {
        delivered = await client.ingest(event);
      } catch (error) {
        outcome?.(remote === client);
        throw error;
      }
      outcome?.(delivered && remote === client);
    };
    const ingest = (event: AdapterEvent): Promise<void> => ingestDelivered(event);
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
      liveSessionId: () => liveSessionId,
      reportDropped: count => presentation.dropped(count),
      deliver: sendDeliveries,
      // Serialized through `attaching` and dropped once its context stopped being current, so a
      // probe that lost the race to a session switch cannot resurrect the old session.
      reattach: context => initialize(context, { onlyIfCurrent: true }),
    } satisfies NativeSessionHost<ExtensionContext>;

    /** The initialize in flight, if any: an event that arrives first waits for it instead of failing. */
    let attaching: Promise<void> | undefined;
    // OMP may begin building the next provider context before an async tool hook
    // has crossed its first await. Claim tool boundaries synchronously so the
    // context hook can join all edit work that was announced before it.
    let codeRuleBoundary: Promise<void> = Promise.resolve();
    const sequenceCodeRuleEvent = (operation: () => Promise<void>): Promise<void> => {
      const work = codeRuleBoundary.then(operation, operation);
      codeRuleBoundary = work.then(() => undefined, () => undefined);
      return work;
    };
    const settleCodeRuleBoundary = async (): Promise<void> => {
      for (;;) {
        const pending = codeRuleBoundary;
        await pending;
        if (pending === codeRuleBoundary) return;
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
    /** The context whose attach failed and is being retried; undefined once one attaches. */
    let failingAttach: ExtensionContext | undefined;
    /** What arrived for `failingAttach`'s session meanwhile: the attached client records it as a gap. */
    let attachDrops: { sessionId: string; gap: DroppedEvents } | undefined;
    const attachAs = async (ctx: ExtensionContext, generation: number): Promise<void> => {
      try {
        await initialize(ctx);
      } catch (error) {
        if (generation !== attachGeneration) throw error;
        attachFailures += 1;
        failingAttach = ctx;
        // Another session's drops keep their marker in that session, which got their final count before the host left
        // it, but the footer counts only this one's.
        if (attachDrops !== undefined && attachDrops.sessionId !== ctx.sessionManager.getSessionId()) {
          attachDrops = undefined;
          presentation.dropped(0);
        }
        // With no runtime left, the footer follows the failing session, so it can say what is being dropped.
        if (remote === undefined) presentation.start(ctx);
        attachTimer = setTimeout(() => { attachAs(ctx, generation).catch(() => undefined); }, attachBackoffMs(attachFailures));
        attachTimer.unref();
        throw error;
      }
      if (generation !== attachGeneration) return;
      if (attachFailures > 0) process.stderr.write(`Rerail attached after ${attachFailures} failed attempt(s)\n`);
      attachFailures = 0;
      failingAttach = undefined;
      const dropped = attachDrops;
      attachDrops = undefined;
      if (dropped === undefined) return;
      presentation.dropped(0);
      // Events of a session that never attached have nowhere to be recorded but their marker, which got its final count
      // before the host left that session.
      if (remote === undefined || activeSession?.sessionId !== dropped.sessionId) return;
      pi.appendEntry(EVENTS_DROPPED_ENTRY_TYPE, { ...dropped.gap, recovered: true });
      remote.recordDropped([dropped.gap]);
    };
    const attach = (ctx: ExtensionContext): Promise<void> => {
      liveSessionId = ctx.sessionManager.getSessionId();
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

    /**
     * Counts the event `build` makes for the session failing to attach, if there is one; the attached client records it.
     * `build` stays pure: nothing counted here changes what a later event of the same message carries.
     */
    const countAttachDrop = (build: (sessionId: string) => AdapterEvent): boolean => {
      if (failingAttach === undefined) return false;
      const sessionId = failingAttach.sessionManager.getSessionId();
      const event = build(sessionId);
      const previous = attachDrops?.sessionId === sessionId ? attachDrops.gap : undefined;
      const gap = countDroppedEvent(previous, event.kind, event.eventId);
      attachDrops = { sessionId, gap };
      if (previous === undefined) pi.appendEntry(EVENTS_DROPPED_ENTRY_TYPE, { ...gap, recovered: false });
      presentation.dropped(gap.count);
      return true;
    };

    /**
     * The gap of a session that never attached is recorded only by its host-session marker, so before the host leaves that
     * session the marker gets the final count. The host appends to whichever session is current, so this runs while it is
     * still `ctx`'s; a switch that is then cancelled keeps counting, and a later marker carries the higher count.
     */
    const markAttachDropsFinal = (ctx: ExtensionContext): void => {
      if (attachDrops === undefined || attachDrops.sessionId !== ctx.sessionManager.getSessionId()) return;
      pi.appendEntry(EVENTS_DROPPED_ENTRY_TYPE, { ...attachDrops.gap, recovered: false });
    };

    /** The attached runtime's open gap marker gets its count so far while the host session is still current (see markAttachDropsFinal). */
    const markGapFinal = (ctx: ExtensionContext): void => {
      markAttachDropsFinal(ctx);
      if (activeSession?.sessionId === ctx.sessionManager.getSessionId()) remote?.markGapFinal();
    };

    /**
     * Hands the event `build` makes for its session to Rerail, and resolves to that session; `outcome` learns whether the
     * runtime carried the event to the daemon (an event its breaker, or the failing attach, counted as dropped never did).
     * While the session is failing to attach, the event is counted as dropped instead, and the attached client records
     * the count as a gap in the session's history; with no session at all it is not ours. An event that waited on a
     * rebuild that failed still rejects with its failure, and is not lost: while the session has no runtime it is counted
     * like the others, and a runtime that outlived a failed rebuild or reload takes it as an ordinary event, so its
     * breaker counts it if the daemon is out. That runtime takes it only when it is the session OMP is in, and a failed
     * attach with a newer one queued behind it is not the event's outcome: the event waits for the newer one. With
     * `quietAttachFailure` (the tool hooks, which must never fail) that failure resolves to no session instead: it has
     * already reported itself, to `session_start` or as the daemon client's one outage line.
     */
    const ingestEvent = async (
      build: (sessionId: string) => AdapterEvent,
      outcome?: (delivered: boolean) => void,
      { quietAttachFailure = false }: { quietAttachFailure?: boolean } = {},
    ): Promise<AdapterSession | undefined> => {
      let session: AdapterSession | undefined;
      for (;;) {
        try {
          session = await sessionFor();
          break;
        } catch (error) {
          if (attaching !== undefined) continue;
          if (failingAttach !== undefined) {
            if (countAttachDrop(build)) outcome?.(false);
          } else if (remote !== undefined && activeSession !== undefined && activeSession.sessionId === liveSessionId) {
            // The failure of the rebuild is the event's outcome; a delivery failure of its own does not replace it.
            await ingestDelivered(build(activeSession.sessionId), outcome).catch(() => undefined);
          }
          if (quietAttachFailure) return undefined;
          throw error;
        }
      }
      // A runtime that outlived a failed switch belongs to the session OMP left: what happens in the one it is in is not its.
      if (session !== undefined && (failingAttach === undefined || session.sessionId === liveSessionId)) {
        await ingestDelivered(build(session.sessionId), outcome);
        return session;
      }
      if (countAttachDrop(build)) outcome?.(false);
      return undefined;
    };

    const initializeNow = async (ctx: ExtensionContext, options_: InitializeOptions = {}): Promise<void> => {
      if (options_.onlyIfCurrent === true && currentContext !== ctx) return;
      const session = { host: "omp", sessionId: ctx.sessionManager.getSessionId(), cwd: ctx.cwd } satisfies AdapterSession;
      const databasePath = options.databasePath === undefined ? undefined : resolve(ctx.cwd, options.databasePath);
      if (!await rerailStarted(ctx.cwd, databasePath)) {
        // Nothing runs here; the footer still says so, without a daemon to ask.
        await closeRuntimeNow();
        currentContext = ctx;
        presentation.start(ctx, { off: true });
        return;
      }
      const activation = await createNativeSession(nativeHost, ctx, session, options_.rebuild === true, {
        ...options,
        modelPath: resolve(options.modelPath ?? process.env.RERAIL_MODEL_PATH ?? bundledModelPath()),
        ...(databasePath === undefined ? {} : { databasePath }),
        storageRoot: resolve(ctx.cwd, process.env.RERAIL_STORAGE_ROOT ?? ".rerail/sessions"),
      });
      if (activation !== "attached") throw new Error("OMP Rerail routing did not attach a runtime");
      presentation.start(ctx);
    };

    if (options.registerTool !== false) {
      registerConversationRulesTool(pi, {
        remoteExecute: () => {
          const current = remote;
          return current === undefined ? undefined : input => current.conversationRules(input);
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
      registerConversationRulesCommand(pi, {
        remoteCommand: () => {
          const current = remote;
          return current === undefined ? undefined : async args => {
            if (args.trim() === "reload") {
              if (currentContext === undefined) throw new Error("cannot reload cosine rules before session_start");
              await initialize(currentContext, { rebuild: true });
              return remote!.conversationRules({ action: "list" });
            }
            return current.conversationRules({ action: "command", args });
          };
        },
        reload: async () => {
          if (currentContext === undefined) throw new Error("cannot reload cosine rules before session_start");
          await initialize(currentContext, { rebuild: true });
        },
      });
      registerHostCommands(pi, {
        sessionId: () => activeSession?.sessionId,
        packageRoot: PACKAGE_ROOT,
        cwd: () => currentContext?.cwd ?? process.cwd(),
        databasePath: () => options.databasePath === undefined
          ? undefined
          : resolve(currentContext?.cwd ?? process.cwd(), options.databasePath),
        // Rage-quit retires this session's runtime before removing Rerail; reattach rebuilds it if the removal fails.
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
    pi.on("session_before_switch", (_event, ctx) => { markGapFinal(ctx); });
    pi.on("session_before_branch", (_event, ctx) => { markGapFinal(ctx); });
    pi.on("session_switch", (_event, ctx) => attach(ctx));
    pi.on("session_branch", (_event, ctx) => attach(ctx));
    pi.on("session_tree", (_event, ctx) => attach(ctx));

    // The first prompt of each session also carries the session-start note, hidden from the person (design A1).
    const notedSessions = new Set<string>();
    pi.on("before_agent_start", async (event) => {
      const session = await ingestEvent(sessionId => userPromptEvent(sessionId, `input:${randomUUID()}`, event.prompt));
      if (session === undefined) return;
      if (notedSessions.has(session.sessionId)) return;
      notedSessions.add(session.sessionId);
      const databasePath = options.databasePath === undefined ? undefined : resolve(session.cwd, options.databasePath);
      const content = sessionStartNote(await dashboardOrigin(session.cwd), "omp", session.sessionId, await sessionNoteRulesFor(session, databasePath));
      return { message: { customType: SESSION_NOTE_MESSAGE_TYPE, content, display: false } };
    });

    pi.on("context", async (event) => {
      await settleCodeRuleBoundary();
      // A result of a session failing to attach stays claimed here; its tool_result counts it as dropped.
      const session = activeSession;
      if (session !== undefined) {
        for (const message of event.messages) {
          if (message.role !== "toolResult") continue;
          const toolName = codeRuleTools.get(message.toolCallId);
          if (toolName === undefined) continue;
          codeRuleTools.delete(message.toolCallId);
          await sequenceCodeRuleEvent(() => ingest(resultEvent(
            session.sessionId,
            `tool-result:${message.toolCallId}`,
            message.toolCallId,
            toolName,
            { content: message.content, details: message.details },
            message.isError,
          )));
        }
      }
      await settleCodeRuleBoundary();
    });

    pi.on("message_update", async (event) => {
      if (event.message.role !== "assistant" || event.assistantMessageEvent.type !== "text_delta") return;
      const messageId = streamId(event.message);
      const { delta } = event.assistantMessageEvent;
      // A delta the runtime carried to the daemon makes its message streamed, so its final carries no text again; one counted
      // as dropped is remembered too. The final waits for the message's deltas still in flight to learn which they were.
      const done = ingestEvent(
        sessionId => assistantDeltaEvent(sessionId, `assistant-delta:${randomUUID()}`, messageId, delta, false),
        delivered => { (delivered ? streamedMessages : droppedDeltaMessages).add(messageId); },
      );
      deltasInFlight.set(messageId, Promise.allSettled([deltasInFlight.get(messageId), done]));
      await done;
    });

    pi.on("message_end", async (event) => {
      if (event.message.role !== "assistant") return;
      const messageId = streamId(event.message);
      const { stopReason } = event.message;
      try {
        await deltasInFlight.get(messageId);
        // The daemon holds only part of a message whose deltas were some delivered and some dropped: it discards that
        // partial stream and takes the whole text as one message.
        if (streamedMessages.has(messageId) && droppedDeltaMessages.has(messageId)) {
          await ingestEvent(sessionId => assistantAbortEvent(sessionId, `assistant-abort:${randomUUID()}`, messageId));
          if (!assistantAborted(stopReason)) {
            await ingestEvent(sessionId => assistantMessageEvent(sessionId, `assistant-message:${randomUUID()}`, messageId, messageText(event.message)));
          }
          return;
        }
        await ingestEvent(sessionId => assistantCompletionEvent(
          sessionId,
          messageId,
          streamedMessages.has(messageId),
          () => messageText(event.message),
          stopReason,
          randomUUID(),
        ));
      } finally {
        streamedMessages.delete(messageId);
        droppedDeltaMessages.delete(messageId);
        deltasInFlight.delete(messageId);
      }
    });

    // Awaited hooks bracket the actual tool; execution notifications can arrive after the write.
    pi.on("tool_call", (event) => {
      codeRuleTools.set(event.toolCallId, event.toolName);
      return contained("tool_call", sequenceCodeRuleEvent(async () => {
        await ingestEvent(sessionId => callEvent(
          sessionId,
          `tool-call:${event.toolCallId}`,
          event.toolCallId,
          event.toolName,
          event.input,
        ), undefined, { quietAttachFailure: true });
      }));
    });
    pi.on("tool_result", (event) => {
      codeRuleTools.delete(event.toolCallId);
      return contained("tool_result", sequenceCodeRuleEvent(async () => {
        await ingestEvent(sessionId => resultEvent(
          sessionId,
          `tool-result:${event.toolCallId}`,
          event.toolCallId,
          event.toolName,
          { content: event.content, details: event.details },
          event.isError,
        ), undefined, { quietAttachFailure: true });
      }));
    });

    /** The latest turn's end, still being checked: the daemon settles the turn's code rules before it answers. */
    let turnEnding: Promise<void> = Promise.resolve();
    pi.on("turn_end", (event) => {
      const ending = (async () => {
        await ingestEvent(sessionId => turnEndEvent(
          sessionId,
          `turn-end:${streamId(event.message)}`,
          messageText(event.message),
        ));
      })();
      turnEnding = ending;
      return ending;
    });

    // The late steer (plan §4.4). OMP runs extension `turn_end` handlers alongside the run's end, not before it
    // (`AgentSession.#emitSessionEvent`), so the steers the turn-end check returns can arrive once the session is
    // idle, and an idle `sendMessage` only appends: the model would not see them until the person's next message
    // (`AgentSession.sendCustomMessage`). `session_stop` runs inside the prompt it ends (the prompt waits for its
    // post-prompt work), so holding it until the check answers sends those steers while the session is still
    // streaming; OMP queues them and, once the prompt unwinds, continues the run with them
    // (`#endInFlight` → `#drainStrandedQueuedMessages`). This is Codex's Stop hold, with the steer kept as a
    // `rerail.rule` message rather than a hidden continuation.
    // A failed turn-end check has already reported itself through its own `turn_end` handler. Escape ends the hold.
    pi.on("session_stop", event => contained("session_stop", Promise.race([
      turnEnding.catch(() => undefined),
      new Promise<void>(resolve_ => {
        if (event.signal.aborted) resolve_();
        else event.signal.addEventListener("abort", () => resolve_(), { once: true });
      }),
    ])));

    pi.on("session_shutdown", async (_event, ctx) => {
      clearTimeout(attachTimer);
      attachGeneration += 1;
      failingAttach = undefined;
      markAttachDropsFinal(ctx);
      attachDrops = undefined;
      presentation.dropped(0);
      presentation.stop();
      try {
        await closeRuntime();
      } finally {
        currentContext = undefined;
        semantic = undefined;
        // Messages of this session that never ended leave no marks behind.
        streamedMessages.clear();
        droppedDeltaMessages.clear();
        deltasInFlight.clear();
      }
    });
  };
}
