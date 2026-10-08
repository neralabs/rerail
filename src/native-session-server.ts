import { randomUUID } from "node:crypto";
import { resolve } from "node:path";
import { z } from "zod";
import superjson from "superjson";
import type { AdapterDelivery, AdapterEvent, AdapterSample, AdapterSession, RerailHost } from "./adapter-contract.js";
import { LiveConversationRules } from "./conversation-rule-sync.js";
import { AdapterEventSchema } from "./event-protocol.js";
import type { OmpSessionEntryLike } from "./omp-semantic.js";
import type { ProjectionSpec, RelationSpec, SelectionSpec, WatermarkChange } from "./semantic-types.js";
import { SessionLanes } from "./session-lanes.js";
import { NativeSemanticRuntime } from "./native-semantic-runtime.js";
import { ConversationRuleRegistry, type RuleMatch } from "./registry.js";
import { parseSampleSource, SAMPLE_BOUNDARIES } from "./rule-schema.js";
import { conversationRuleLoader, type ConversationRuleSourceOptions } from "./rule-sources.js";
import { openCheckoutConversationRules } from "./code-rule-dsl-catalog.js";
import { MATCH_ENTRY_TYPE, MUTATION_ENTRY_TYPE, replaySessionState, TURN_ENTRY_TYPE, type RuleMutation } from "./session-state.js";
import { RerailRuntime } from "./runtime.js";

export type NativeSessionRequest = z.infer<typeof requestSchema>;

export interface NativeSessionResponse {
  result?: unknown;
  deliveries: readonly AdapterDelivery[];
  resultRevisionEntry?: number;
  entries: Array<{ customType: string; data: unknown }>;
}
const NATIVE_SESSION_LEASE_MS = 60_000;
const NATIVE_SESSION_SWEEP_INTERVAL_MS = 5_000;


interface NativeSession {
  session: AdapterSession;
  registry: ConversationRuleRegistry;
  ruleSync: LiveConversationRules;
  runtime: RerailRuntime;
  semantic: NativeSemanticRuntime;
  entries: readonly SessionEntry[];
  turn: number;
  pendingDeliveries: AdapterDelivery[];
  appendEntries: Array<{ customType: string; data: unknown }>;
  leaseExpiresAt: number;
}

type SessionEntry = OmpSessionEntryLike;
type ConversationRulesInput = { action: string; rule?: unknown; replace?: boolean; ids?: unknown; sample?: unknown; args?: unknown };

const objectSchema = z.object({}).passthrough();
const requestSchema = objectSchema.extend({
  workspaceRoot: z.string(),
  sessionId: z.string().min(1),
  host: z.union([z.literal("omp"), z.literal("pi")]),
  operation: z.enum(["initialize", "ingest", "cosine", "semantic", "poll", "close", "record-steering-delivery"]),
  entries: z.unknown().optional(),
  entriesEncoding: z.literal("superjson").optional(),
  event: z.unknown().optional(),
  eventEncoding: z.literal("superjson").optional(),
  input: z.unknown().optional(),
});

const entriesSchema = z.array(z.object({ type: z.string(), id: z.string(), timestamp: z.string() }).passthrough());
/** A client from a build without steer attribution sends the bare receipt id. */
const steerReceiptSchema = z.union([
  z.string().min(1).transform(receiptId => ({ receiptId, ruleIds: [] as string[] })),
  z.object({ receiptId: z.string().min(1), ruleIds: z.array(z.string()).default([]) }),
]);
const conversationRulesInputSchema = objectSchema.extend({ action: z.string() });


/** Branch entries carry arbitrary host payloads, so the client encodes them; plain JSON stays accepted. */
function decodeEntries(input: NativeSessionRequest): unknown {
  if (input.entries === undefined || input.entriesEncoding !== "superjson") return input.entries;
  return superjson.deserialize(input.entries as Parameters<typeof superjson.deserialize>[0]);
}

function parseEvent(value: unknown, session: AdapterSession): AdapterEvent {
  const event = AdapterEventSchema.parse(value);
  const sessionId = event.kind === "session-start" ? event.session.sessionId : event.sessionId;
  if (sessionId !== session.sessionId) throw new TypeError("event sessionId does not match request sessionId");
  if (event.kind === "session-start" && (event.session.host !== session.host || resolve(event.session.cwd) !== session.cwd)) {
    throw new TypeError("session-start does not match initialized session");
  }
  return event;
}
export class NativeSessionServer {
  readonly #workspaceRoot: string;
  readonly #modelPath: string;
  readonly #databasePath: string;
  readonly #sessionLeaseMs: number;
  readonly #sessions = new Map<string, NativeSession>();
  readonly #lanes = new SessionLanes();
  readonly #sweepTimer: NodeJS.Timeout;
  #sweepTask: Promise<void> | undefined;

  constructor(options: {
    workspaceRoot: string;
    modelPath: string;
    databasePath: string;
    sessionLeaseMs?: number;
    sessionSweepIntervalMs?: number;
  }) {
    this.#workspaceRoot = resolve(options.workspaceRoot);
    this.#modelPath = resolve(options.modelPath);
    this.#databasePath = resolve(options.databasePath);
    this.#sessionLeaseMs = options.sessionLeaseMs ?? NATIVE_SESSION_LEASE_MS;
    const sweepIntervalMs = options.sessionSweepIntervalMs ?? NATIVE_SESSION_SWEEP_INTERVAL_MS;
    if (this.#sessionLeaseMs <= 0 || sweepIntervalMs <= 0) throw new RangeError("native session lease intervals must be positive");
    this.#sweepTimer = setInterval(() => {
      if (this.#sweepTask !== undefined) return;
      this.#sweepTask = this.#sweepExpired().finally(() => {
        this.#sweepTask = undefined;
      });
    }, sweepIntervalMs);
    this.#sweepTimer.unref();
  }

  async handle(input: unknown): Promise<unknown> {
    const parsed = requestSchema.parse(input);
    if (resolve(parsed.workspaceRoot) !== this.#workspaceRoot) throw new RangeError("native workspace mismatch");
    return this.#lanes.run(parsed.sessionId, async () => {
      const response = await this.#handle(parsed);
      if (parsed.operation !== "close") {
        const active = this.#sessions.get(parsed.sessionId);
        if (active !== undefined) active.leaseExpiresAt = Date.now() + this.#sessionLeaseMs;
      }
      return response;
    });
  }

  async close(): Promise<void> {
    clearInterval(this.#sweepTimer);
    await this.#sweepTask;
    await Promise.all([...this.#sessions.keys()].map(async sessionId =>
      this.#lanes.run(sessionId, () => this.#close(sessionId))));
  }

  async #handle(input: NativeSessionRequest): Promise<NativeSessionResponse> {
    const session = this.#sessionForRequest(input);
    if (input.operation === "initialize") return this.#initialize(input);
    if (input.operation === "close") {
      await this.#close(input.sessionId);
      return session === undefined ? { deliveries: [], entries: [] } : this.#reply(session);
    }
    if (session === undefined) throw new Error("native session is not initialized");
    if (input.entries !== undefined) session.entries = entriesSchema.parse(decodeEntries(input)) as SessionEntry[];
    if (input.operation === "record-steering-delivery") {
      const receipt = steerReceiptSchema.parse(input.input);
      session.runtime.recordSteer(input.sessionId, receipt.receiptId, receipt.ruleIds);
      return { deliveries: [], entries: [] };
    }
    if (input.operation === "poll") {
      session.pendingDeliveries.push(...await session.runtime.collectDeliveries(input.sessionId));
      return this.#reply(session);
    }
    session.ruleSync.refresh();
    if (input.operation === "ingest") {
      const event = input.eventEncoding === "superjson"
        ? superjson.deserialize(input.event as Parameters<typeof superjson.deserialize>[0])
        : input.event;
      const deliveries = await session.runtime.ingest(parseEvent(event, session.session));
      session.pendingDeliveries.push(...deliveries);
      return this.#reply(session);
    }
    if (input.operation === "cosine") {
      const result = await this.#conversationRules(session, conversationRulesInputSchema.parse(input.input) as ConversationRulesInput, (customType, data) => session.appendEntries.push({ customType, data }));
      return { ...this.#reply(session), result };
    }
    const semantic = await this.#semantic(session, input.input);
    const reply = this.#reply(session);
    if (semantic.revisionEntry === undefined) return { ...reply, result: semantic.result };
    const resultRevisionEntry = reply.entries.indexOf(semantic.revisionEntry);
    if (resultRevisionEntry < 0) throw new Error("native watermark acknowledgement was not included in its reply");
    return { ...reply, result: semantic.result, resultRevisionEntry };
  }

  #sessionForRequest(input: NativeSessionRequest): NativeSession | undefined {
    const session = this.#sessions.get(input.sessionId);
    if (session !== undefined && session.session.host !== input.host) {
      throw new Error("native session owner mismatch");
    }
    return session;
  }

  #reply(session: NativeSession): NativeSessionResponse {
    return { deliveries: session.pendingDeliveries.splice(0), entries: session.appendEntries.splice(0) };
  }

  /** Model, database, and session storage paths resolve against the session's cwd. */
  #sessionPaths(
    options: { modelPath?: string; databasePath?: string; storageRoot?: string },
    cwd: string,
  ): { modelPath: string; databasePath: string; storageRoot: string } {
    return {
      modelPath: resolve(cwd, options.modelPath ?? this.#modelPath),
      databasePath: options.databasePath === undefined ? this.#databasePath : resolve(cwd, options.databasePath),
      storageRoot: resolve(cwd, options.storageRoot ?? ".rerail/sessions"),
    };
  }

  async #initialize(input: NativeSessionRequest): Promise<NativeSessionResponse> {
    const branch = entriesSchema.parse(decodeEntries(input)) as SessionEntry[];
    const options = objectSchema.extend({
      cwd: z.string().optional(),
      modelPath: z.string().optional(),
      databasePath: z.string().optional(),
      storageRoot: z.string().optional(),
      maxInputTokens: z.number().int().positive().optional(),
      maxRules: z.number().int().positive().optional(),
      rebuild: z.boolean().optional(),
    }).parse(input.input ?? {}) as ConversationRuleSourceOptions & {
      cwd?: string; modelPath?: string; databasePath?: string; storageRoot?: string;
      maxInputTokens?: number; maxRules?: number; rebuild?: boolean;
    };
    const cwd = resolve(options.cwd ?? this.#workspaceRoot);
    const existing = this.#sessions.get(input.sessionId);
    if (existing !== undefined && existing.session.cwd === cwd && options.rebuild !== true) {
      existing.ruleSync.prime();
      existing.turn = replaySessionState(existing.registry, branch);
      existing.runtime.restoreNativeTurn(input.sessionId, existing.turn);
      existing.entries = branch;
      return this.#reply(existing);
    }
    const { modelPath, databasePath, storageRoot } = this.#sessionPaths(options, cwd);
    const session: AdapterSession = { host: input.host as RerailHost, sessionId: input.sessionId, cwd };
    const registry = new ConversationRuleRegistry({
      modelPath,
      ...(options.maxInputTokens === undefined ? {} : { maxInputTokens: options.maxInputTokens }),
      ...(options.maxRules === undefined ? {} : { maxRules: options.maxRules }),
    });
    const loadRules = conversationRuleLoader(options, cwd, input.host === "omp" ? ".omp/conversation-rules.json" : ".pi/conversation-rules.json");
    const ruleSync = new LiveConversationRules(openCheckoutConversationRules(databasePath, cwd, loadRules), registry);
    let runtime: RerailRuntime | undefined;
    let semantic: NativeSemanticRuntime | undefined;
    try {
      ruleSync.prime();
      const turn = replaySessionState(registry, branch);
      semantic = new NativeSemanticRuntime({ modelPath, sourceId: input.host === "omp" ? "omp:message" : "pi:message" });
      runtime = new RerailRuntime({
        storageRoot, modelPath, databasePath, workspaceRoot: cwd,
        autostart: false, startGate: true,
        beforeSessionPublish: async () => {
          await this.#close(input.sessionId);
          if (existing !== undefined) {
            attached.pendingDeliveries.push(...existing.pendingDeliveries.splice(0));
            attached.appendEntries.push(...existing.appendEntries.splice(0));
          }
        },
        // No deliverBackground: a result completing during an event rides that event's reply, so the
        // host sees it before its next model call; results with no event are drained by `poll`.
        nativeSession: {
          registry, ruleSync, turn,
          onMatch: (match, sample, matchedTurn) => attached.appendEntries.push({ customType: MATCH_ENTRY_TYPE, data: matchEntry(registry, match, sample, matchedTurn) }),
          onTurn: value => { attached.turn = value; attached.appendEntries.push({ customType: TURN_ENTRY_TYPE, data: { turn: value } }); },
        },
      });
      const attached: NativeSession = {
        session, registry, ruleSync, runtime, semantic, entries: branch, turn,
        pendingDeliveries: [], appendEntries: [], leaseExpiresAt: Date.now() + this.#sessionLeaseMs,
      };
      semantic.initialize({
        sessionId: session.sessionId,
        entries: () => attached.entries,
        appendEntry: (customType, data) => attached.appendEntries.push({ customType, data }),
      });
      attached.pendingDeliveries.push(...await runtime.ingest({ kind: "session-start", eventId: `session-start:${session.sessionId}:${randomUUID()}`, session }));
      // A stopped checkout drops the session start before publication, so `beforeSessionPublish` never closed
      // the session this one replaces: close it here, or its runtime, handles and timers outlive the map entry.
      if (this.#sessions.has(input.sessionId)) await this.#close(input.sessionId);
      this.#sessions.set(input.sessionId, attached);
      return this.#reply(attached);
    } catch (error) {
      try { await runtime?.dispose(); } catch { /* Preserve the initialization error. */ }
      semantic?.close();
      ruleSync.close();
      throw error;
    }
  }

  async #conversationRules(session: NativeSession, input: ConversationRulesInput, append: (customType: string, data: unknown) => void): Promise<unknown> {
    const registry = session.registry;
    if (input.action === "add") {
      if (input.rule === undefined) throw new TypeError("rule is required for action=add");
      const replace = input.replace === true;
      const rule = registry.upsert(input.rule, { replace });
      const mutation: RuleMutation = { operation: "upsert", rule, replace };
      append(MUTATION_ENTRY_TYPE, mutation);
      return { operation: "add", ruleId: rule.id, replaced: replace };
    }
    if (input.action === "remove") {
      if (!Array.isArray(input.ids) || input.ids.length === 0 || !input.ids.every(id => typeof id === "string")) throw new TypeError("ids must contain at least one rule id for action=remove");
      const ids = [...new Set(input.ids)];
      const removed = registry.remove(ids);
      append(MUTATION_ENTRY_TYPE, { operation: "remove", ids } satisfies RuleMutation);
      return { operation: "remove", removed, ids };
    }
    if (input.action === "list") return { rules: registry.inspect(), modelFingerprint: registry.fingerprint() };
    if (input.action === "test") return this.#test(registry, session.turn, input.sample);
    if (input.action === "command") return this.#command(session, input.args);
    throw new TypeError(`unsupported cosine action: ${input.action}`);
  }

  #test(registry: ConversationRuleRegistry, turn: number, sample: unknown): unknown {
    const parsed = objectSchema.extend({
      source: z.string(),
      boundary: z.enum(SAMPLE_BOUNDARIES),
      text: z.string(),
    }).parse(sample);
    return { matches: registry.match({ ...parsed, source: parseSampleSource(parsed.source, "sample.source") } as AdapterSample, { turn, respectRepeat: false }), modelFingerprint: registry.fingerprint() };
  }

  #command(session: NativeSession, raw: unknown): unknown {
    if (typeof raw !== "string") throw new TypeError("command args must be a string");
    const [operation = "list", ...rest] = (raw.trim() || "list").split(/\s+/);
    if (operation === "list") return { operation, rules: session.registry.inspect() };
    if (operation === "model") return { operation, modelFingerprint: session.registry.fingerprint() };
    if (operation === "reload") {
      session.ruleSync.prime();
      session.turn = replaySessionState(session.registry, session.entries);
      return { operation, reloaded: session.registry.inspect().length };
    }
    if (operation === "test") {
      const [ruleId, ...text] = rest;
      const inspection = session.registry.inspect().find(item => item.rule.id === ruleId);
      if (inspection === undefined) throw new Error(`Unknown cosine rule: ${ruleId ?? ""}`);
      return { operation, ruleId, matches: session.registry.match({ source: inspection.rule.sample.source, boundary: inspection.rule.sample.boundary, text: text.join(" ") }, { turn: session.turn, respectRepeat: false }).filter(match => match.ruleId === ruleId) };
    }
    throw new TypeError("Usage: /cosine-rules [list|model|reload|test <rule-id> <text>]");
  }

  async #semantic(
    session: NativeSession,
    input: unknown,
  ): Promise<{ result: unknown; revisionEntry?: NativeSession["appendEntries"][number] }> {
    const parsed = objectSchema.extend({ action: z.string() }).parse(input);
    if (parsed.action === "mark") {
      const previousEntries = session.appendEntries.length;
      const result = await session.semantic.mark(parsed.change as WatermarkChange);
      const revisionEntry = session.appendEntries[previousEntries];
      return revisionEntry === undefined ? { result } : { result, revisionEntry };
    }
    if (parsed.action === "select") return { result: await session.semantic.select(parsed.spec as SelectionSpec) };
    if (parsed.action === "project" && typeof parsed.selectionId === "string") {
      return { result: await session.semantic.project(parsed.selectionId, parsed.spec as ProjectionSpec) };
    }
    if (parsed.action === "relate" && typeof parsed.leftProjectionId === "string" && typeof parsed.rightProjectionId === "string") {
      return { result: await session.semantic.relate(parsed.leftProjectionId, parsed.rightProjectionId, parsed.spec as RelationSpec) };
    }
    throw new TypeError(`unsupported semantic action: ${parsed.action}`);
  }

  async #sweepExpired(): Promise<void> {
    const expired = [...this.#sessions]
      .filter(([, session]) => session.leaseExpiresAt <= Date.now())
      .map(([sessionId]) => sessionId);
    const results = await Promise.allSettled(expired.map(async sessionId =>
      this.#lanes.run(sessionId, async () => {
        const session = this.#sessions.get(sessionId);
        if (session !== undefined && session.leaseExpiresAt <= Date.now()) await this.#close(sessionId);
      })));
    for (const result of results) {
      if (result.status === "rejected") {
        process.stderr.write(`Rerail native session lease cleanup failed: ${String(result.reason)}\n`);
      }
    }
  }

  async #close(sessionId: string): Promise<void> {
    const session = this.#sessions.get(sessionId);
    if (session === undefined) return;
    this.#sessions.delete(sessionId);
    try {
      await session.runtime.ingest({ kind: "session-end", eventId: `session-end:${sessionId}:${randomUUID()}`, sessionId, session: session.session });
    } finally {
      try {
        await session.runtime.dispose();
      } finally {
        session.semantic.close();
        session.ruleSync.close();
      }
    }
  }
}

function matchEntry(registry: ConversationRuleRegistry, match: RuleMatch, sample: AdapterSample, turn: number): unknown {
  return { ruleId: match.ruleId, kind: match.kind, similarity: match.similarity, threshold: match.threshold, operator: match.operator, tokenCount: match.tokenCount, source: sample.source, boundary: sample.boundary, mode: match.mode, turn, modelFingerprint: registry.fingerprint() };
}
