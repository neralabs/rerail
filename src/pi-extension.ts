import { randomUUID } from "node:crypto";
import { join, resolve } from "node:path";
import type { ExtensionAPI, ExtensionContext, ExtensionFactory } from "@earendil-works/pi-coding-agent";
import {
  sendNativeDeliveries,
  type AdapterDelivery,
  type AdapterSample,
  type AdapterSession,
} from "./adapter-contract.js";
import { LiveCosineRules } from "./cosine-rule-sync.js";
import { isRerailStarted } from "./repository-start.js";
import { bundledModelPath, PACKAGE_ROOT } from "./extension.js";
import type { ExtensionTranscriptOptions } from "./extension-transcript.js";
import type { ModelFingerprint } from "./native.js";
import type { NativeSemanticToolRuntime } from "./native-semantic-tools.js";
import { NativeSemanticRuntime } from "./native-semantic-runtime.js";
import type { NativeSessionClient } from "./native-session-client.js";
import { PiNativeSession } from "./pi-native-session.js";
import { registerPiHostAdapter } from "./pi-host-adapter.js";
import { CosineRuleRegistry, type RuleMatch } from "./registry.js";
import { cosineRuleLoader, type CosineRuleSourceOptions } from "./rule-sources.js";
import { loadBundledRules } from "./rule-bootstrap.js";
import { openScopedRuleCatalog } from "./scoped-rule-catalog.js";
import { defaultRerailDatabasePath } from "./database.js";
import { RerailRuntime } from "./runtime.js";
import { MATCH_ENTRY_TYPE, replaySessionState, TURN_ENTRY_TYPE } from "./session-state.js";

export interface PiCosineRulesOptions extends ExtensionTranscriptOptions, CosineRuleSourceOptions {
  modelPath?: string;
  maxInputTokens?: number;
  maxRules?: number;
  registerTool?: boolean;
  registerCommand?: boolean;
}
interface MatchEntryData {
  ruleId: string;
  kind: RuleMatch["kind"];
  similarity?: number | undefined;
  threshold?: number | undefined;
  operator?: RuleMatch["operator"];
  tokenCount?: number | undefined;
  source: AdapterSample["source"];
  boundary: AdapterSample["boundary"];
  mode: RuleMatch["mode"];
  turn: number;
  modelFingerprint: ModelFingerprint;
}


function configuredDatabasePath(options: PiCosineRulesOptions, cwd: string): string {
  return options.databasePath === undefined ? defaultRerailDatabasePath() : resolve(cwd, options.databasePath);
}

interface InitializeOptions {
  rebuild?: boolean;
  /** Only (re)attach while the context is still the host's current one; a stale caller becomes a no-op. */
  onlyIfCurrent?: boolean;
}



export function createPiCosineRulesExtension(options: PiCosineRulesOptions = {}): ExtensionFactory {
  return function piCosineRulesExtension(pi: ExtensionAPI): void {
    let registry: CosineRuleRegistry | undefined;
    let runtime: RerailRuntime | undefined;
    let ruleSync: LiveCosineRules | undefined;
    let currentContext: ExtensionContext | undefined;
    let activeSession: AdapterSession | undefined;
    let turn = 0;
    const pendingBackgroundDeliveries = new Map<RerailRuntime, AdapterDelivery[]>();
    let semantic: NativeSemanticRuntime | undefined;

    const activeRegistry = (): CosineRuleRegistry => {
      if (registry === undefined) throw new Error("cosine-rules extension received an event before session_start");
      return registry;
    };
    const activeSemantic = (): NativeSemanticToolRuntime => {
      const current = native.current() ?? semantic;
      if (current === undefined) throw new Error("rerail semantic tools received an event before session_start");
      return current;
    };
    const appendMatch = (match: RuleMatch, sample: AdapterSample, matchedTurn: number): void => {
      pi.appendEntry(MATCH_ENTRY_TYPE, {
        ruleId: match.ruleId,
        kind: match.kind,
        similarity: match.similarity,
        threshold: match.threshold,
        operator: match.operator,
        tokenCount: match.tokenCount,
        source: sample.source,
        boundary: sample.boundary,
        mode: match.mode,
        turn: matchedTurn,
        modelFingerprint: activeRegistry().fingerprint(),
      } satisfies MatchEntryData);
    };
    interface Attached {
      registry: CosineRuleRegistry;
      runtime: RerailRuntime;
      ruleSync: LiveCosineRules;
      session: AdapterSession;
      context: ExtensionContext;
      turn: number;
    }

    const retire = async (attached: Attached | undefined): Promise<void> => {
      if (attached === undefined) return;
      try {
        await attached.runtime.ingest({
          kind: "session-end",
          eventId: `session-end:${attached.session.sessionId}:${randomUUID()}`,
          sessionId: attached.session.sessionId,
          session: attached.session,
        });
      } finally {
        try {
          await attached.runtime.dispose();
        } finally {
          attached.ruleSync.close();
        }
      }
    };

    const detach = (): Attached | undefined => {
      if (runtime === undefined || activeSession === undefined || registry === undefined || ruleSync === undefined || currentContext === undefined) return undefined;
      const current: Attached = { registry, runtime, ruleSync, session: activeSession, context: currentContext, turn };
      runtime = undefined;
      activeSession = undefined;

      ruleSync = undefined;
      return current;
    };

    const attach = (next: Attached): void => {
      registry = next.registry;
      runtime = next.runtime;
      ruleSync = next.ruleSync;
      activeSession = next.session;
      currentContext = next.context;
      turn = next.turn;
      const pending = pendingBackgroundDeliveries.get(next.runtime) ?? [];
      pendingBackgroundDeliveries.delete(next.runtime);
      void sendDeliveries(next.runtime, pending);
    };
    /** Releases the daemon client or the local runtime immediately; callers already inside `attaching` use this. */
    const closeRuntimeNow = async (): Promise<void> => {
      if (native.current() !== undefined) {
        await native.close();
        return;
      }
      await retire(detach());
    };
    const closeRuntime = (): Promise<void> => {
      const pending = (attaching ?? Promise.resolve()).then(closeRuntimeNow, closeRuntimeNow).finally(() => {
        if (attaching === pending) attaching = undefined;
      });
      attaching = pending;
      return pending;
    };
    const sendDeliveries = (current: RerailRuntime | NativeSessionClient, deliveries: readonly AdapterDelivery[]): Promise<void> =>
      sendNativeDeliveries(deliveries, (message, options_) => pi.sendMessage(message, options_),
        (sessionId, receiptId, ruleIds) => current.recordSteeringDelivery(sessionId, receiptId, ruleIds));
    const native = new PiNativeSession({
      entries: context => (context ?? currentContext)?.sessionManager.getBranch() ?? [],
      appendEntry: (customType, data) => pi.appendEntry(customType, data),
      deliver: sendDeliveries,
      // Serialized through `attaching` and dropped once its context stopped being current, so a
      // probe that lost the race to a session switch cannot resurrect the old session.
      reattach: context => initialize(context, { onlyIfCurrent: true }),
      closeFallback: () => retire(detach()),
    });
    const ingest = async (event: Parameters<RerailRuntime["ingest"]>[0]): Promise<void> => {
      if (native.current() !== undefined) {
        await native.ingest(event);
        return;
      }
      const current = runtime;
      if (current === undefined) throw new Error("cosine-rules extension received an event before session_start");
      await sendDeliveries(current, await current.ingest(event));
    };

    let attaching: Promise<void> | undefined;

    const initialize = (ctx: ExtensionContext, options_: InitializeOptions = {}): Promise<void> => {
      const start = () => initializeNow(ctx, options_);
      const pending = (attaching ?? Promise.resolve()).then(start, start).finally(() => {
        if (attaching === pending) attaching = undefined;
      });
      attaching = pending;
      return pending;
    };

    const sessionFor = async (): Promise<AdapterSession | undefined> => {
      while (attaching !== undefined) await attaching;
      return native.session() ?? activeSession;
    };

    const initializeNow = async (ctx: ExtensionContext, options_: InitializeOptions = {}): Promise<void> => {
      if (options_.onlyIfCurrent === true && (native.context() ?? currentContext) !== ctx) return;
      if (!isRerailStarted(ctx.cwd, configuredDatabasePath(options, ctx.cwd))) {
        await closeRuntimeNow();
        currentContext = ctx;
        return;
      }
      const session = { host: "pi", sessionId: ctx.sessionManager.getSessionId(), cwd: ctx.cwd } satisfies AdapterSession & { host: "pi" };
      const activation = await native.initialize(ctx, session, options_.rebuild === true, options);
      if (activation === "attached") return;
      semantic ??= new NativeSemanticRuntime({
        modelPath: options.modelPath ?? process.env.RERAIL_MODEL_PATH ?? bundledModelPath(),
        sourceId: "pi:message",
      });
      // Navigating within the same session keeps the runtime; only the turn counter is replayed.
      const current = activeSession;
      if (options_.rebuild !== true && current !== undefined && registry !== undefined && current.sessionId === session.sessionId && current.cwd === session.cwd) {
        turn = replaySessionState(registry, ctx.sessionManager.getBranch());
        currentContext = ctx;
        runtime?.recordRuleCoverage(current.sessionId);
        return;
      }
      const built = await build(ctx, session);
      try {
        await built.runtime.ingest({ kind: "session-start", eventId: `session-start:${session.sessionId}:${randomUUID()}`, session });
        if (semantic instanceof NativeSemanticRuntime) {
          semantic.initialize({
            sessionId: session.sessionId,
            entries: () => ctx.sessionManager.getBranch(),
            appendEntry: (customType, data) => pi.appendEntry(customType, data),
          });
        }
      } catch (error) {
        pendingBackgroundDeliveries.delete(built.runtime);
        try {
          await built.runtime.dispose();
        } catch {
          // Cleanup must not replace the startup error.
        }
        try {
          built.ruleSync.close();
        } catch {
          // Still report the startup error if closing the rule store fails.
        }
        throw error;
      }
      const previous = detach();
      attach(built);
      await retire(previous);
    };

    const build = async (ctx: ExtensionContext, session: AdapterSession): Promise<Attached> => {
      const databasePath = configuredDatabasePath(options, ctx.cwd);
      const next = new CosineRuleRegistry({
        modelPath: options.modelPath ?? process.env.RERAIL_MODEL_PATH ?? bundledModelPath(),
        ...(options.maxInputTokens === undefined ? {} : { maxInputTokens: options.maxInputTokens }),
        ...(options.maxRules === undefined ? {} : { maxRules: options.maxRules }),
      });
      const loadRules = cosineRuleLoader(options, ctx.cwd, ".pi/conversation-rules.json");
      const ruleStore = openScopedRuleCatalog(ctx.cwd, databasePath);
      const sync = new LiveCosineRules(ruleStore, next, loadRules);
      try {
        ruleStore.seed(loadBundledRules());
        sync.prime();
        const nextTurn = replaySessionState(next, ctx.sessionManager.getBranch());
        const nextRuntime = new RerailRuntime({
          storageRoot: process.env.RERAIL_STORAGE_ROOT === undefined
            ? join(ctx.cwd, ".rerail", "sessions")
            : resolve(ctx.cwd, process.env.RERAIL_STORAGE_ROOT),
          modelPath: options.modelPath ?? process.env.RERAIL_MODEL_PATH ?? bundledModelPath(),
          databasePath,
          workspaceRoot: ctx.cwd,
          repositoryContext: ruleStore.context(),
          startGate: true,
          beforeSessionPublish: async nextSession => {
            if (activeSession?.host === nextSession.host && activeSession.sessionId === nextSession.sessionId) {
              await closeRuntimeNow();
            }
          },
          deliverBackground: async delivery => {
            if (runtime === nextRuntime) await sendDeliveries(nextRuntime, [delivery]);
            else pendingBackgroundDeliveries.get(nextRuntime)?.push(delivery);
          },
          nativeSession: {
            registry: next, ruleSync: sync, turn: nextTurn, onMatch: appendMatch,
            onTurn: value => { turn = value; pi.appendEntry(TURN_ENTRY_TYPE, { turn }); },
          },
        });
        pendingBackgroundDeliveries.set(nextRuntime, []);
        return { registry: next, runtime: nextRuntime, ruleSync: sync, session, context: ctx, turn: nextTurn };
      } catch (error) {
        sync.close();
        throw error;
      }

    };

    registerPiHostAdapter(pi, {
      registerTool: options.registerTool !== false,
      registerCommand: options.registerCommand !== false,
      activeSession: () => native.session() ?? activeSession,
      currentContext: () => native.context() ?? currentContext,
      owner: () => native.current() ?? runtime,
      semantic: activeSemantic,
      initialize: (context, rebuild) => initialize(context, { rebuild }),
      close: closeRuntime,
      shutdown: async () => {
        try {
          await closeRuntime();
        } finally {
          currentContext = undefined;
          native.clearContext();
          registry = undefined;
          if (semantic instanceof NativeSemanticRuntime) semantic.close();
          semantic = undefined;
        }
      },
      sessionFor,
      ingest,
      cosineTool: {
        registry: activeRegistry,
        turn: () => turn,
        remoteExecute: () => {
          const current = native.current();
          return current === undefined ? undefined : input => current.cosine(input);
        },
      },
      cosineCommand: {
        registry: activeRegistry,
        turn: () => turn,
        remoteCommand: () => {
          const current = native.current();
          return current === undefined ? undefined : args => current.cosine({ action: "command", args });
        },
      },
      packageRoot: PACKAGE_ROOT,
      started: cwd => isRerailStarted(cwd, configuredDatabasePath(options, cwd)),
      configuredDatabase: cwd => options.databasePath === undefined ? undefined : resolve(cwd, options.databasePath),
    });
  };
}

export default createPiCosineRulesExtension();
