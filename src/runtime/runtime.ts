import { createHash, randomUUID } from "node:crypto";
import { appendFile, chmod, mkdir, readFile, truncate } from "node:fs/promises";
import { join, resolve } from "node:path";
import { z } from "zod/v4";
import {
  ADAPTER_CAPABILITIES,
  type AdapterDelivery,
  type AdapterEvent,
  type AdapterSample,
  type AdapterSession,
  type RerailEventLoop,
  type RerailHost,
} from "../engine/adapter-contract.js";
import { RerailHostRuntime, type HostRuntimePort, type HostRuntimeSessionState } from "./host-runtime.js";
import { autostartDaemon } from "./daemon-control.js";
import { BOUNDARY_DISCOVERY_WAIT_MS, CodeRuleBridge, generatedEditPaths, type SourceObservationCache } from "../engine/code-rule/code-rule-bridge.js";
import type { AuditorCloseOptions, CodeRuleAuditor, AuditSettings, RuleSteerDelivery, RepositoryAuditHooks } from "../engine/code-rule/code-rule.js";
import type { AuditRecorder, AuditRecord } from "../engine/audit/audit.js";
import { SqliteAuditStore } from "../engine/audit/audit-store.js";
import { SqliteAuditControlStore, type PolicyOwnerClaim } from "../engine/audit/audit-control.js";
import { DEFAULT_AUDIT_SETTINGS } from "../engine/audit/audit-control-schema.js";
import { CheckoutCatalogs, configurationSignatureOf, startingCheckout } from "../engine/code-rule/code-rule-dsl-catalog.js";
import { realpathSync } from "node:fs";
import { CheckoutOwners } from "../engine/catalog/checkout-owner.js";
import { CheckoutCodeRuleRouter } from "../engine/code-rule/checkout-code-rule-router.js";
import { defaultRerailDatabasePath } from "../engine/storage/database.js";
import { lookupCheckout, recordCheckout } from "../engine/catalog/repository-registry.js";
import { RepositoryStartWatch } from "./repository-start.js";
import { cosineSimilarity, NativeSemanticEmbedder, SemanticWorkspace } from "../engine/native/semantic-primitives.js";
import { ConversationRuleRegistry, type RuleMatch } from "../engine/catalog/registry.js";
import { LiveConversationRules } from "../engine/catalog/conversation-rule-sync.js";
import { parseSampleSource, SAMPLE_BOUNDARIES } from "../engine/catalog/rule-schema.js";
import { RULE_MODES } from "../engine/catalog/rule-mode.js";
import { restorableRule, type DroppedEvents, type RuleMutation } from "./session-state.js";
import { SessionLanes } from "./session-lanes.js";
import { steerBlock } from "../engine/code-rule/rerail-steer.js";
import { steerOrigin } from "../engine/code-rule/steer-origin.js";
import { SqliteSessionStore, type SessionAttribution } from "./session-store.js";
import type {
  ContentItem,
  ContentSnapshot,
  ContentSource,
  MarkReceipt,
  ProjectionSpec,
  RelationSpec,
  Selection,
  SelectionSpec,
  SemanticEmbedder,
  SemanticProjection,
  WatermarkAssignment,
  WatermarkChange,
  WatermarkSnapshot,
  WatermarkStore,
} from "../engine/native/semantic-types.js";

function assertEventHost(host: RerailHost): void {
  if (Object.hasOwn(ADAPTER_CAPABILITIES, host)) return;
  throw new Error(`Event runtime does not support host: ${host}`);
}

/**
 * How long a one-shot hook (Claude Code, Codex, Cursor) waits after a tool for that edit's rule check, so a steer lands
 * under the edit and the coding agent can act on it in the same turn. A check usually takes tens of milliseconds; one
 * that runs longer is delivered at the next hook instead (at the turn's end in Claude Code), and the hook never waits
 * past this. OMP and Pi run in process and always wait for the check.
 */
const HOOK_EDIT_CHECK_MS = 1_000;

/**
 * Waits for `work` up to `ms`; past the cap the work carries on in the background. The wait only decides when the
 * hook answers, so a failure of the check is not raised here: the check records its own failures in its audit.
 */
async function settledWithin(work: Promise<void>, ms: number): Promise<void> {
  const capped = Promise.withResolvers<void>();
  const timer = setTimeout(capped.resolve, ms);
  try {
    await Promise.race([work.catch(() => undefined), capped.promise]);
  } finally {
    clearTimeout(timer);
  }
}

function sampleOwner(source: AdapterSample["source"]): "user" | "assistant" | "tool" {
  if (source === "user-input") return "user";
  if (source === "assistant-text") return "assistant";
  return "tool";
}

function validatedLedger(values: readonly unknown[]): LedgerEntry[] {
  return values.flatMap((value) => {
    const parsed = LedgerEntrySchema.safeParse(value);
    return parsed.success ? [compactValidated<LedgerEntry>(parsed.data)] : [];
  });
}

function processedEventIds(ledger: readonly LedgerEntry[]): Set<string> {
  return new Set(ledger.flatMap((entry) => (entry.type === "event" ? [entry.eventId] : [])));
}
interface AuditRequestVerdict {
  status: "completed" | "failed" | "interrupted";
  error: string | null;
}
/** Outcome of an explicit audit request from its recorded audits: any missing or non-completed audit fails the whole request. */
function auditRequestVerdict(auditIds: readonly string[], audits: readonly (AuditRecord | undefined)[]): AuditRequestVerdict {
  const unavailable = audits.indexOf(undefined);
  if (unavailable >= 0) return { status: "failed", error: `Policy run ${auditIds[unavailable]} was not recorded` };
  const completed = audits as AuditRecord[];
  const interrupted = completed.find(audit => audit.status === "interrupted");
  if (interrupted !== undefined) return { status: "interrupted", error: `Policy run ${interrupted.auditId} was interrupted` };
  const failed = completed.find(audit => audit.status !== "completed");
  if (failed !== undefined) {
    const details = failed.failures.map(failure => failure.message).filter(Boolean).join("; ");
    return { status: "failed", error: details || `Policy run ${failed.auditId} ended ${failed.status}` };
  }
  return { status: "completed", error: null };
}

/** The events whose code-rule work needs the session's evaluator open: a tool call, its result, and a turn's end. */
const CODE_RULE_BOUNDARIES: Partial<Record<AdapterEvent["kind"], true>> = { call: true, result: true, "turn-end": true };

/** A turn end that waited longer than this on its settle work is written to the daemon log. */
const SLOW_TURN_END_WAIT_MS = 1_000;

/** Code-rule calls recorded without a matching result, in ledger order. */
function unresolvedCodeRuleCalls(ledger: readonly LedgerEntry[]): Extract<LedgerEntry, { type: "policy-call" }>[] {
  const pending = new Map<string, Extract<LedgerEntry, { type: "policy-call" }>>();
  for (const entry of ledger) {
    if (entry.type === "policy-call") pending.set(entry.toolCallId, entry);
    else if (entry.type === "policy-result") pending.delete(entry.toolCallId);
  }
  return [...pending.values()];
}


const PrimitiveSchema = z.union([z.string(), z.number().finite(), z.boolean(), z.null()]);
const ContentRefSchema = z
  .object({
    source: z.string().min(1),
    id: z.string().min(1),
    span: z.object({ start: z.number().int().nonnegative(), end: z.number().int().positive() }).optional(),
    revision: z.string().optional(),
  })
  .refine((ref) => ref.span === undefined || ref.span.end > ref.span.start, "span end must exceed start");
const WatermarkSchema = z
  .object({
    namespace: z.string().min(1),
    key: z.string().min(1),
    value: PrimitiveSchema.optional(),
    weight: z.number().finite().optional(),
    owner: z.string(),
    visibility: z.enum(["private", "team", "project"]),
    createdAt: z.number().finite(),
    expiresAt: z.number().finite().optional(),
    provenance: z.string().optional(),
  })
  .refine((mark) => mark.expiresAt === undefined || mark.expiresAt > mark.createdAt, "expiresAt must exceed createdAt");

const ContentItemSchema = z.object({
  ref: ContentRefSchema,
  text: z.string(),
  metadata: z.object({
    owner: z.string().optional(),
    session: z.string().optional(),
    timestamp: z.number().finite().optional(),
    fields: z.record(z.string(), PrimitiveSchema).optional(),
  }),
});
const LedgerEntrySchema = z.union([
  z.object({ type: z.literal("event"), eventId: z.string() }),
  z.object({ type: z.literal("hud-tracking-start") }),
  z.object({ type: z.literal("sample"), item: ContentItemSchema }),
  z.object({ type: z.literal("turn"), eventId: z.string() }),
  z.object({
    type: z.literal("delivery"),
    delivery: z.object({
      sessionId: z.string(),
      requested: z.enum(RULE_MODES),
      mechanism: z.enum(["observe", "native-queue", "additional-context", "deferred-context"]),
      content: z.string(),
      ruleIds: z.array(z.string()),
    }),
  }),
  z.object({ type: z.literal("rule-mutation"), mutation: z.unknown() }),
  z.object({ type: z.literal("rule-match"), ruleId: z.string(), turn: z.number().int().nonnegative() }),
  z.object({
    type: z.literal("policy-call"), toolCallId: z.string(), toolName: z.string(), paths: z.array(z.string()),
    preimages: z.record(z.string(), z.object({ text: z.string().optional(), readError: z.string().optional() })).optional(),
  }),
  z.object({ type: z.literal("policy-result"), toolCallId: z.string() }),
]);

type LedgerEntry =
  | { type: "event"; eventId: string }
  | { type: "hud-tracking-start" }
  | { type: "sample"; item: ContentItem }
  | { type: "turn"; eventId: string }
  | { type: "delivery"; delivery: AdapterDelivery }
  | { type: "rule-mutation"; mutation: RuleMutation }
  | { type: "rule-match"; ruleId: string; turn: number }
  | { type: "policy-call"; toolCallId: string; toolName: string; paths: string[]; preimages?: Readonly<Record<string, { text?: string; readError?: string }>> }
  | { type: "policy-result"; toolCallId: string };

export function compactValidated<T>(value: unknown): T {
  // Zod has already validated this value; the round-trip removes explicit undefined keys for exact optional types.
  return JSON.parse(JSON.stringify(value)) as T;
}

class MutableContentSource implements ContentSource {
  readonly id = "host:event";
  readonly #items = new Map<string, ContentItem>();
  #revision = 0;

  constructor(items: readonly ContentItem[]) {
    for (const item of items) this.#items.set(item.ref.id, item);
    this.#revision = items.length;
  }

  append(item: ContentItem): void {
    if (this.#items.has(item.ref.id)) return;
    this.#items.set(item.ref.id, item);
    this.#revision += 1;
  }

  async read(): Promise<ContentSnapshot> {
    return { id: `${this.id}@${this.#revision}`, items: [...this.#items.values()] };
  }
}

interface JsonlAppendState {
  prefix: "" | "\n";
  truncateTo: number | null;
}

interface JsonlReadResult {
  values: unknown[];
  appendState: JsonlAppendState;
}

class JsonlWatermarkStore implements WatermarkStore {
  readonly #path: string;
  readonly #appendState: JsonlAppendState;
  readonly #assignments = new Map<string, WatermarkAssignment>();
  #revision = 0;

  private constructor(path: string, appendState: JsonlAppendState) {
    this.#path = path;
    this.#appendState = appendState;
  }

  static async open(path: string): Promise<JsonlWatermarkStore> {
    const { values: lines, appendState } = await readLines(path);
    const store = new JsonlWatermarkStore(path, appendState);
    const snapshots = lines.flatMap((line) => {
      const parsed = z
        .object({
          revision: z.number().int().nonnegative(),
          assignments: z.array(
            z.object({
              id: z.string(),
              target: ContentRefSchema,
              watermark: WatermarkSchema,
            }),
          ),
        })
        .safeParse(line);
      return parsed.success
        ? [compactValidated<{ revision: number; assignments: WatermarkAssignment[] }>(parsed.data)]
        : [];
    });
    const latest = snapshots.at(-1);
    if (latest !== undefined) {
      store.#revision = latest.revision;
      for (const assignment of latest.assignments) store.#assignments.set(assignment.id, assignment);
    }
    return store;
  }

  async apply(change: WatermarkChange): Promise<MarkReceipt> {
    const changed: string[] = [];
    if (change.kind === "attach") {
      for (const target of change.targets) {
        for (const watermark of change.watermarks) {
          const id = randomUUID();
          this.#assignments.set(id, { id, target: structuredClone(target), watermark: structuredClone(watermark) });
          changed.push(id);
        }
      }
    } else {
      for (const id of change.assignmentIds) if (this.#assignments.delete(id)) changed.push(id);
    }
    if (changed.length > 0) {
      this.#revision += 1;
      await appendJsonLine(
        this.#path,
        { revision: this.#revision, assignments: [...this.#assignments.values()] },
        this.#appendState,
      );
    }
    return { revision: String(this.#revision), assignmentIds: changed };
  }

  async read(): Promise<WatermarkSnapshot> {
    return {
      revision: String(this.#revision),
      assignments: [...this.#assignments.values()].map((assignment) => structuredClone(assignment)),
    };
  }
}

interface PendingSteer {
  delivery: AdapterDelivery;
  revision: number;
  scope: string[];
}

/** The checkout a session started in (physical root), the session folder relative to it, and that folder resolved once. */
interface StartingCheckout {
  root: string;
  folder: string;
  /** The session's folder, resolved once at open; every later use reads this, never the folder name again. */
  cwd: string;
}

/** What the evaluator does for a boundary the session answered without it: begin a tool call, or finish one. */
type DeferredCodeRuleBoundary =
  | { kind: "call"; toolCallId: string; toolName: string; paths: string[] }
  | { kind: "result"; toolCallId: string; failed: boolean };

interface RuntimeSessionState {
  descriptor: AdapterSession;
  /** Decides the session's conversation rules, coverage and repository attribution. */
  checkout: StartingCheckout;
  startedAt: number;
  processed: Set<string>;
  source: MutableContentSource;
  workspace: SemanticWorkspace;
  selections: Map<string, Selection>;
  projections: Map<string, SemanticProjection>;
  ledgerPath: string;
  ledgerAppendState: JsonlAppendState;
  sampleSequence: number;
  registry?: ConversationRuleRegistry;
  ruleSync: LiveConversationRules | undefined;
  turn: number;
  eventDeliveries: Map<string, AdapterDelivery[]>;
  pendingSteers: PendingSteer[];
  codeRuleRevision: number;
  codeRuleSourceVersions: Map<string, number>;
  /** Sources whose queued steer a newer revision of them dropped before delivery, until the auditor asks (`undelivered`). */
  undeliveredSources: Set<string>;
  /** The session's evaluator, opening or open; unset for good once it could not open. */
  codeRuleBridge?: CodeRuleBridge;
  /** Why the evaluator could not open, when it did not: a scan that waited for it reports this. */
  codeRuleOffReason?: string;
  /** Settles once the evaluator has opened (the bridge) or failed to (undefined); code-rule boundaries wait for it. */
  codeRuleWorkspace: Promise<CodeRuleBridge | undefined>;
  /** The evaluator has opened: what it observes may be delivered. */
  codeRuleReady: boolean;
  /** The evaluator is still opening and owns its own release: closing the session does not wait for it. */
  codeRuleOpening: boolean;
  ownerClaim: PolicyOwnerClaim | undefined;
  auditRequestWork: Promise<void> | undefined;
  repositoryAudit: Promise<void> | undefined;
  pendingCodeRuleCalls: Array<{ id: string; toolName: string }>;
  /** Boundaries answered without the evaluator because it was still opening; it begins and finishes them once open. */
  deferredCodeRuleBoundaries: DeferredCodeRuleBoundary[];
  closed: boolean;
}

async function readLines(path: string): Promise<JsonlReadResult> {
  let content: string;
  try {
    content = await readFile(path, "utf8");
  } catch (error) {
    if (error instanceof Error && "code" in error && error.code === "ENOENT") {
      return { values: [], appendState: { prefix: "", truncateTo: null } };
    }
    throw error;
  }

  const appendState: JsonlAppendState = { prefix: "", truncateTo: null };
  if (content.length > 0 && !content.endsWith("\n")) {
    const tailStart = content.lastIndexOf("\n") + 1;
    try {
      JSON.parse(content.slice(tailStart));
      appendState.prefix = "\n";
    } catch {
      const validPrefix = content.slice(0, tailStart);
      appendState.truncateTo = Buffer.byteLength(validPrefix);
      content = validPrefix;
    }
  }
  return {
    values: content
      .split("\n")
      .filter(Boolean)
      .map((line) => JSON.parse(line) as unknown),
    appendState,
  };
}

async function appendJsonLine(path: string, value: unknown, state: JsonlAppendState): Promise<void> {
  await mkdir(join(path, ".."), { recursive: true, mode: 0o700 });
  if (state.truncateTo !== null) await truncate(path, state.truncateTo);
  await appendFile(path, `${state.prefix}${JSON.stringify(value)}\n`, { encoding: "utf8", mode: 0o600 });
  state.prefix = "";
  state.truncateTo = null;
}

function sessionDirectory(root: string, sessionId: string): string {
  return join(root, createHash("sha256").update(sessionId).digest("hex"));
}

/** Replays a session ledger into its registry and returns the turn the session had reached. */
function replayLedger(registry: ConversationRuleRegistry, ledger: readonly LedgerEntry[]): number {
  let turn = 0;
  for (const entry of ledger) {
    if (entry.type === "rule-mutation") {
      applyRuleMutation(registry, entry.mutation);
    } else if (entry.type === "rule-match") {
      registry.restoreDelivery(entry.ruleId, entry.turn);
      turn = Math.max(turn, entry.turn);
    } else if (entry.type === "turn") {
      turn += 1;
    }
  }
  return turn;
}

function applyRuleMutation(registry: ConversationRuleRegistry, mutation: RuleMutation): void {
  if (mutation.operation === "remove") {
    registry.remove(mutation.ids);
    return;
  }
  const rule = restorableRule(mutation.rule, "persisted rule");
  if (rule !== undefined) registry.upsert(rule, { replace: mutation.replace });
}


function publicProjection(projection: SemanticProjection): unknown {
  return {
    projectionId: projection.id,
    model: projection.model,
    lineage: projection.lineage,
    vectors: projection.vectors.map((vector) => ({
      id: vector.id,
      members: vector.members,
      groupKey: vector.groupKey,
      renderedText: vector.renderedText,
      renderedHash: vector.renderedHash,
    })),
  };
}

export interface RerailRuntimeOptions {
  autostart?: boolean;
  /**
   * Resident runtimes (daemon-owned OMP/Pi sessions, the event stream, Pi in-process) outlive a stop
   * issued elsewhere; with this set they drop every event while Rerail is stopped in the repository.
   */
  startGate?: boolean;
  storageRoot: string;
  embedder?: SemanticEmbedder;
  modelPath?: string;
  /** Defaults to `$RERAIL_HOME/rerail.sqlite`, which holds the rules and every repository's history. */
  databasePath?: string;
  /** The folder whose start state gates this runtime (`startGate`); each session's own folder decides its rules. */
  workspaceRoot: string;
  /**
   * False for callers without the daemon token: their sessions only attach to checkouts already recorded, and
   * nothing is recorded, set up or imported for them. Defaults to true.
   */
  trusted?: boolean;
  auditSettings?: AuditSettings;
  /** Delivers a completed background code-rule result without waiting for another host event. */
  deliverBackground?: (delivery: AdapterDelivery) => Promise<void> | void;
  /** Transfers a prepared session's host ownership before durable publication. */
  beforeSessionPublish?: (session: AdapterSession) => Promise<void> | void;
  /** Test and alternate-runtime seam for audits. */
  createAuditor?: (options: {
    workspaceRoot: string;
    steerDelivery: RuleSteerDelivery;
    audits: AuditRecorder;
    /** Where the evaluator writes its storage-attach and repository-audit timing lines; it writes none without it. */
    timingLog?: (line: string) => void;
  } & AuditSettings) => CodeRuleAuditor;
  /** How long a turn end or command result waits for the session's shell-write walk, and a code-rule boundary for the evaluator to open; the bridge's default otherwise. */
  discoveryWaitMs?: number;
  /** Receives the timing lines of session start and slow turn ends (the daemon passes its log); without it they are not written. */
  timingLog?: (line: string) => void;
  nativeSession?: {
    registry: ConversationRuleRegistry;
    ruleSync?: LiveConversationRules;
    turn: number;
    onMatch(match: RuleMatch, sample: AdapterSample, turn: number): void;
    onTurn?(turn: number): void;
  };
}

export class RerailRuntime implements HostRuntimePort, RerailEventLoop {
  readonly #storageRoot: string;
  readonly #embedder: SemanticEmbedder;
  readonly #modelPath: string | undefined;
  readonly #sessionStore: SqliteSessionStore;
  readonly #audits: SqliteAuditStore;
  readonly #steersInFlight = new Set<Promise<AdapterDelivery | undefined>>();
  /** Evaluator opens still running, each with its own release: disposal waits for them before the stores close under them. */
  readonly #evaluatorOpens = new Set<Promise<CodeRuleBridge | undefined>>();
  /** Every checkout this runtime's sessions touch, with its eligibility and compiled rules. */
  readonly #catalogs: CheckoutCatalogs;
  readonly #owners = new CheckoutOwners(() => this.#catalogs.recordedCheckoutsVersion());
  readonly #trusted: boolean;
  readonly #sessions = new Map<string, RuntimeSessionState>();
  /** Kept past disposal for late receipts, with the repository each session was attributed to. */
  readonly #closedSessions = new Map<string, { descriptor: AdapterSession; startedAt: number; endedAt: number; repositoryKey: string | null }>();
  #sessionStoreClosed = false;
  readonly #routers = new Map<RerailHost, RerailHostRuntime>();
  readonly #nativeSession: RerailRuntimeOptions["nativeSession"];
  readonly #createAuditor: NonNullable<RerailRuntimeOptions["createAuditor"]>;
  readonly #deliverBackground: RerailRuntimeOptions["deliverBackground"];
  readonly #beforeSessionPublish: RerailRuntimeOptions["beforeSessionPublish"];
  readonly #auditSettings: Required<AuditSettings>;
  readonly #controls: SqliteAuditControlStore;
  readonly #controlTimer: NodeJS.Timeout;
  readonly #autostart: boolean;
  readonly #ingestLanes = new SessionLanes();
  readonly #sourceObservations: SourceObservationCache = new Map();
  readonly #discoveryWaitMs: number | undefined;
  readonly #timingLog: ((line: string) => void) | undefined;
  readonly #startWatch: RepositoryStartWatch | undefined;

  constructor(options: RerailRuntimeOptions) {
    this.#storageRoot = options.storageRoot;
    this.#modelPath = options.modelPath;
    this.#nativeSession = options.nativeSession;
    this.#deliverBackground = options.deliverBackground;
    this.#beforeSessionPublish = options.beforeSessionPublish;
    this.#auditSettings = { ...DEFAULT_AUDIT_SETTINGS, ...options.auditSettings };
    this.#discoveryWaitMs = options.discoveryWaitMs;
    this.#timingLog = options.timingLog;
    this.#autostart = options.autostart ?? true;
    this.#trusted = options.trusted ?? true;
    this.#createAuditor = options.createAuditor ?? ((auditorOptions) => new CheckoutCodeRuleRouter({
      ...auditorOptions,
      // `workspaceRoot` is the session folder `openSession` resolved once.
      sessionRoot: auditorOptions.workspaceRoot,
      startingCheckout: startingCheckout(this.#owners, auditorOptions.workspaceRoot).checkoutRoot,
      owners: this.#owners,
      catalogs: this.#catalogs,
    }));
    if (options.embedder !== undefined) this.#embedder = options.embedder;
    else if (options.modelPath !== undefined) this.#embedder = new NativeSemanticEmbedder(options.modelPath);
    else throw new Error("RerailRuntime requires embedder or modelPath");
    const databasePath = options.databasePath ?? defaultRerailDatabasePath();
    const workspaceRoot = resolve(options.workspaceRoot);
    let catalogs: CheckoutCatalogs | undefined;
    let sessionStore: SqliteSessionStore | undefined;
    let audits: SqliteAuditStore | undefined;
    let controls: SqliteAuditControlStore | undefined;
    let startWatch: RepositoryStartWatch | undefined;
    try {
      catalogs = new CheckoutCatalogs(databasePath);
      startWatch = options.startGate === true ? new RepositoryStartWatch(databasePath, workspaceRoot) : undefined;
      sessionStore = new SqliteSessionStore(databasePath);
      audits = new SqliteAuditStore(databasePath);
      controls = new SqliteAuditControlStore(databasePath);
      this.#catalogs = catalogs;
      this.#sessionStore = sessionStore;
      this.#audits = audits;
      this.#controls = controls;
      this.#startWatch = startWatch;
    } catch (error) {
      sessionStore?.close();
      audits?.close();
      controls?.close();
      startWatch?.close();
      catalogs?.close();
      throw error;
    }
    this.#controlTimer = setInterval(() => {
      for (const sessionId of this.#sessions.keys()) this.#scheduleAuditRequests(sessionId);
    }, 250);
    this.#controlTimer.unref?.();
  }

  /** Stopped repositories record and steer nothing, even in sessions that attached before the stop. */
  #stopped(): boolean {
    return this.#startWatch !== undefined && !this.#startWatch.started();
  }

  ingest(event: AdapterEvent): Promise<readonly AdapterDelivery[]> {
    const sessionId = event.kind === "session-start" ? event.session.sessionId : event.sessionId;
    if (this.#stopped() && event.kind !== "session-end") return Promise.resolve([]);
    return this.#ingestLanes.run(sessionId, async () => {
      // Checked again in the lane: an event accepted while started can run after a stop committed elsewhere.
      if (this.#stopped()) {
        // A session that attached while started still ends, releasing its code-rule bridge and polling; nothing is recorded.
        if (event.kind === "session-end" && this.#sessions.has(sessionId)) await this.closeSession(sessionId);
        return [];
      }
      const sourceSignature = this.#sessions.get(sessionId)?.ruleSync?.sourceSignature;
      const deliveries = await this.#ingest(event);
      if (event.kind !== "assistant-delta" || event.final
          || sourceSignature !== this.#sessions.get(sessionId)?.ruleSync?.sourceSignature) this.recordRuleCoverage(sessionId);
      return deliveries;
    });
  }

  /** Branch navigation restores repeat/turn state without retiring the live stream or code-rule owner. */
  restoreNativeTurn(sessionId: string, turn: number): void {
    this.#session(sessionId).turn = turn;
  }

  async #ingest(event: AdapterEvent): Promise<readonly AdapterDelivery[]> {
    if (event.kind === "session-start") {
      this.#sessions.get(event.session.sessionId)?.ruleSync?.refresh();
      return this.#ingestSessionStart(event);
    }

    const { active, descriptor } = this.#resolveEventSession(event);
    if (active === undefined) await this.#router(descriptor.host).start(descriptor);
    const state = this.#session(event.sessionId);
    this.#scheduleAuditRequests(state.descriptor.sessionId);
    state.ruleSync?.refresh();
    // A code-rule boundary waits for the session's evaluator to open, within a bound; any other event goes on without it.
    const bridge = CODE_RULE_BOUNDARIES[event.kind] === true ? await this.#awaitCodeRuleWorkspace(state, event) : state.codeRuleBridge;
    bridge?.refreshCodeRules();
    if (event.kind === "call") {
      const id = event.toolCallId;
      const preimages = await bridge?.begin(id, event.toolName, event.input);
      await this.#append(state, {
        type: "policy-call", toolCallId: id, toolName: event.toolName, paths: generatedEditPaths(event.toolName, event.input),
        ...(preimages === undefined ? {} : { preimages }),
      });
      state.pendingCodeRuleCalls.push({ id, toolName: event.toolName });
    }
    await this.#router(descriptor.host).ingest(event);
    if (event.kind === "result") {
      const index = state.pendingCodeRuleCalls.findIndex((call) => call.id === event.toolCallId);
      if (index >= 0) {
        const [call] = state.pendingCodeRuleCalls.splice(index, 1);
        await bridge?.finish(call!.id, event.failed === true);
        await this.#append(state, { type: "policy-result", toolCallId: call!.id });
        const settled = bridge?.settleGeneratedEdits();
        if (ADAPTER_CAPABILITIES[descriptor.host].toolSurface === "native") await settled;
        else if (settled !== undefined) await settledWithin(settled, HOOK_EDIT_CHECK_MS);
      }
    }
    if (event.kind === "turn-end") {
      const walkStartedAt = performance.now();
      await bridge?.checkpoint("turn");
      this.#reportSlowTurnEndWait(event.sessionId, "the source walk", walkStartedAt);
      // The turn's own edits, not unrelated audits: the session-start repository audit runs for minutes on a
      // large checkout, and a turn boundary that waited for it held the session's request lane past the host's timeout.
      const settleStartedAt = performance.now();
      await bridge?.settleGeneratedEdits();
      this.#reportSlowTurnEndWait(event.sessionId, "generated-edit audits", settleStartedAt);
    }
    if (event.kind === "session-end") return [];
    const deliveries = await this.#collectPendingSteers(state);
    deliveries.push(...(state.eventDeliveries.get(event.eventId) ?? []));
    state.eventDeliveries.delete(event.eventId);
    return deliveries;
  }

  /** A turn end that waits long holds the session's request lane: the daemon log says how long and on what; fast ones say nothing. */
  #reportSlowTurnEndWait(sessionId: string, waitedOn: string, startedAt: number): void {
    const milliseconds = performance.now() - startedAt;
    if (milliseconds <= SLOW_TURN_END_WAIT_MS) return;
    this.#timingLog?.(`rerail: turn end of session ${sessionId} waited ${Math.round(milliseconds)} ms for ${waitedOn}\n`);
  }

  /**
   * The session's evaluator for a code-rule boundary, waited for at most `#discoveryWaitMs`: the host aborts a request
   * after 30 s, so an open held up by Git discovery or a slow storage attach must not hold the session's first tool
   * call. Past the wait the boundary is answered without code rules and recorded in the session's history; its begin
   * or finish is queued for the evaluator to run once it has opened, so the edit is still audited and its steer
   * delivered at a later boundary.
   */
  async #awaitCodeRuleWorkspace(state: RuntimeSessionState, event: AdapterEvent): Promise<CodeRuleBridge | undefined> {
    if (!state.codeRuleOpening) return await state.codeRuleWorkspace;
    const budget = Promise.withResolvers<undefined>();
    const timer = setTimeout(() => budget.resolve(undefined), this.#discoveryWaitMs ?? BOUNDARY_DISCOVERY_WAIT_MS);
    try {
      const opened = await Promise.race([state.codeRuleWorkspace.then(bridge => ({ bridge })), budget.promise]);
      if (opened !== undefined) return opened.bridge;
    } finally {
      clearTimeout(timer);
    }
    // Opened since the wait ran out: nothing is pending.
    if (!state.codeRuleOpening) return await state.codeRuleWorkspace;
    // Queued and recorded with no await in between: the evaluator's drain of the queue sees this boundary or the bridge does.
    if (event.kind === "call") {
      state.deferredCodeRuleBoundaries.push({
        kind: "call", toolCallId: event.toolCallId, toolName: event.toolName, paths: generatedEditPaths(event.toolName, event.input),
      });
    } else if (event.kind === "result") {
      state.deferredCodeRuleBoundaries.push({ kind: "result", toolCallId: event.toolCallId, failed: event.failed === true });
    }
    if (event.kind === "call" || event.kind === "result" || event.kind === "turn-end") {
      const boundary = event.kind;
      try {
        this.#recordSessionFact(
          state.descriptor.sessionId,
          (store, descriptor, attribution) => store.recordCodeRulesPending(descriptor, boundary, attribution),
        );
      } catch (error) {
        // Recording a boundary that answered without code rules is best-effort: it never fails the boundary.
        process.stderr.write(`rerail: policy execution recording failed: ${error instanceof Error ? error.message : String(error)}\n`);
      }
    }
    return undefined;
  }

  /**
   * Completed code-rule results not yet handed to the host, for hosts that poll instead of
   * receiving `deliverBackground` pushes. Unknown or closed sessions have nothing pending.
   */
  async collectDeliveries(sessionId: string): Promise<AdapterDelivery[]> {
    const state = this.#sessions.get(sessionId);
    return state === undefined || this.#stopped() ? [] : this.#collectPendingSteers(state);
  }

  async #collectPendingSteers(state: RuntimeSessionState): Promise<AdapterDelivery[]> {
    // An evaluator still opening may yet fail, and what it observed with it.
    if (!state.codeRuleReady) return [];
    // Delivery removes entries from the list, so iterate a snapshot.
    const deliveries: AdapterDelivery[] = [];
    for (const pending of state.pendingSteers.slice()) {
      const delivery = await this.#deliverSteer(state, pending, false);
      if (delivery !== undefined) deliveries.push(delivery);
    }
    return deliveries;
  }

  async mark(sessionId: string, change: WatermarkChange): Promise<MarkReceipt> {
    return this.#session(sessionId).workspace.mark(change);
  }

  async select(sessionId: string, spec: SelectionSpec): Promise<unknown> {
    const state = this.#session(sessionId);
    const selection = await state.workspace.select(spec);
    const selectionId = randomUUID();
    state.selections.set(selectionId, selection);
    return {
      selectionId,
      snapshot: selection.snapshot,
      specificationHash: selection.specificationHash,
      items: selection.items,
    };
  }

  async project(sessionId: string, selectionId: string, spec: ProjectionSpec): Promise<unknown> {
    const state = this.#session(sessionId);
    const selection = state.selections.get(selectionId);
    if (selection === undefined) throw new Error(`Unknown selectionId for session ${sessionId}`);
    const projection = await state.workspace.project(selection, spec);
    state.projections.set(projection.id, projection);
    return publicProjection(projection);
  }

  async relate(sessionId: string, leftId: string, rightId: string, spec: RelationSpec): Promise<unknown> {
    const state = this.#session(sessionId);
    const left = state.projections.get(leftId);
    const right = state.projections.get(rightId);
    if (left === undefined || right === undefined) throw new Error(`Unknown projection id for session ${sessionId}`);
    return state.workspace.relate(left, right, spec);
  }

  async testRule(sessionId: string, reference: string, text: string): Promise<{ similarity: number }> {
    this.#session(sessionId);
    const [left, right] = await this.#embedder.embed([reference, text]);
    if (left === undefined || right === undefined) {
      throw new Error("Embedder returned invalid diagnostic vectors");
    }
    return { similarity: cosineSimilarity(left, right) };
  }

  async manageRules(
    sessionId: string,
    input: {
      action: "add" | "remove" | "list" | "test";
      rule?: unknown;
      replace?: boolean | undefined;
      ids?: string[] | undefined;
      sample?: { source: string; boundary: (typeof SAMPLE_BOUNDARIES)[number]; text: string } | undefined;
    },
  ): Promise<unknown> {
    const state = this.#session(sessionId);
    const registry = state.registry;
    if (registry === undefined) throw new Error("Rule management requires a configured native model");
    if (input.action === "add") {
      if (input.rule === undefined) throw new TypeError("rule is required for action=add");
      const replace = input.replace === true;
      const rule = registry.upsert(input.rule, { replace });
      const mutation: RuleMutation = { operation: "upsert", rule, replace };
      await this.#append(state, { type: "rule-mutation", mutation });
      this.recordRuleCoverage(sessionId);
      return { operation: "add", ruleId: rule.id, replaced: replace };
    }
    if (input.action === "remove") {
      if (input.ids === undefined || input.ids.length === 0) {
        throw new TypeError("ids must contain at least one rule id for action=remove");
      }
      const ids = [...new Set(input.ids)];
      const removed = registry.remove(ids);
      const mutation: RuleMutation = { operation: "remove", ids };
      await this.#append(state, { type: "rule-mutation", mutation });
      this.recordRuleCoverage(sessionId);
      return { operation: "remove", removed, ids };
    }
    if (input.action === "list") {
      return { rules: registry.inspect(), modelFingerprint: registry.fingerprint() };
    }
    if (input.sample === undefined) throw new TypeError("sample is required for action=test");
    const sample = {
      ...input.sample,
      source: parseSampleSource(input.sample.source, "sample.source"),
    };
    return {
      matches: registry.match(sample, { turn: state.turn, respectRepeat: false }),
      modelFingerprint: registry.fingerprint(),
    };
  }

  recordSteer(sessionId: string, receiptId: string, ruleIds: readonly string[] = []): void {
    this.#recordSessionFact(sessionId, (store, descriptor, attribution) => store.recordSteer(descriptor, receiptId, ruleIds, attribution));
  }

  /** The host's record of session events it never delivered, so the session's history reads as incomplete. */
  recordDroppedEvents(sessionId: string, dropped: DroppedEvents): void {
    this.#recordSessionFact(sessionId, (store, descriptor, attribution) => store.recordDroppedEvents(descriptor, dropped, attribution));
  }

  #recordSessionFact(
    sessionId: string,
    record: (store: SqliteSessionStore, descriptor: AdapterSession, attribution: SessionAttribution) => void,
  ): void {
    const state = this.#sessions.get(sessionId);
    const closed = this.#closedSessions.get(sessionId);
    const descriptor = state?.descriptor ?? closed?.descriptor;
    if (descriptor === undefined) throw new Error(`Unknown session: ${sessionId}`);
    // An awaited native send may finish after switching/disposal; what it reports still belongs to this session.
    const sessionStore = this.#sessionStoreClosed ? new SqliteSessionStore(this.#sessionStore.path) : this.#sessionStore;
    try {
      record(sessionStore, descriptor, {
        startedAt: state?.startedAt ?? closed!.startedAt,
        endedAt: closed?.endedAt ?? null,
        repositoryKey: state === undefined ? closed!.repositoryKey : this.#repositoryKey(state.checkout),
      });
    } finally {
      if (this.#sessionStoreClosed) sessionStore.close();
    }
  }

  /** Report loaded sources, never refresh enforcement for presentation metadata. */
  recordRuleCoverage(sessionId: string): void {
    const state = this.#sessions.get(sessionId);
    // Unpublished sessions have no session row yet; the session-start ingest records coverage once they do.
    if (state === undefined || state.closed || state.ownerClaim === undefined) return;
    try {
      const { root } = state.checkout;
      const repositoryId = this.#repositoryKey(state.checkout);
      if (repositoryId === null) return;
      const catalog = this.#catalogs.catalog(root);
      const loaded = catalog.status.sourceSignature;
      // Current only while the code and conversation rules were loaded from the same configuration.
      const synchronized = state.ruleSync === undefined || state.ruleSync.sourceSignature === loaded;
      this.#sessionStore.recordRuleCoverage(state.descriptor, {
        repositoryId,
        ownerCheckoutPaths: this.#catalogs.ownerCheckoutPaths(root),
        activeRuleCount: (state.registry?.count ?? 0) + catalog.status.ruleCount,
        ruleSignature: (synchronized ? configurationSignatureOf(loaded) : undefined) ?? "",
      });
    } catch (error) {
      console.error("rerail rule coverage:", error);
    }
  }

  #repositoryKey(checkout: StartingCheckout): string | null {
    return this.#catalogs.checkout(checkout.root)?.repositoryKey ?? null;
  }

  /** An evaluator is still opening: `dispose` waits for its release, which the caller may leave running behind it. */
  get evaluatorOpening(): boolean {
    return this.#evaluatorOpens.size > 0;
  }

  async dispose(): Promise<void> {
    clearInterval(this.#controlTimer);
    await Promise.all([...this.#sessions.keys()].map(sessionId => this.closeSession(sessionId)));
    await Promise.allSettled(this.#evaluatorOpens);
    await Promise.allSettled(this.#steersInFlight);
    this.#controls.close();
    this.#audits.close();
    this.#sessionStore.close();
    this.#sessionStoreClosed = true;
    this.#catalogs.close();
    this.#startWatch?.close();
  }

  async openSession(session: AdapterSession): Promise<HostRuntimeSessionState> {
    const existing = this.#sessions.get(session.sessionId);
    if (existing !== undefined) return { processedEventIds: [...existing.processed] };
    const recordedSession = this.#sessionStore.session(session.sessionId);
    const startedAt = recordedSession?.startedAt ?? Date.now();
    const directory = sessionDirectory(this.#storageRoot, session.sessionId);
    await mkdir(directory, { recursive: true, mode: 0o700 });
    await chmod(directory, 0o700);
    const ledgerPath = join(directory, "events.jsonl");
    const { values, appendState } = await readLines(ledgerPath);
    const ledger = validatedLedger(values);
    // The ledger marker survives a database reset. Never mistake resumed or reset history for a fresh session.
    // With no DB, ledger, or native replay history, this is the session's first observed Rerail participation.
    const completeTracking = recordedSession === undefined && values.length === 0 && (this.#nativeSession?.turn ?? 0) === 0;
    if (!ledger.some(entry => entry.type === "hud-tracking-start")) {
      await appendJsonLine(ledgerPath, { type: "hud-tracking-start" }, appendState);
    }
    const processed = processedEventIds(ledger);
    const checkout = this.#startingCheckout(session);
    const { registry, turn, ruleSync } = await this.#restoreRegistry(session, ledger, checkout);
    const items = ledger.flatMap((entry) => (entry.type === "sample" ? [entry.item] : []));
    const source = new MutableContentSource(items);
    const watermarks = await JsonlWatermarkStore.open(join(directory, "watermarks.jsonl"));
    const opened: RuntimeSessionState = {
      descriptor: session,
      checkout,
      startedAt,
      processed,
      source,
      workspace: new SemanticWorkspace({ sources: [source], watermarks, embedder: this.#embedder }),
      selections: new Map(),
      projections: new Map(),
      ledgerPath,
      ledgerAppendState: appendState,
      sampleSequence: items.length,
      ...(registry === undefined ? {} : { registry }),
      ruleSync,
      turn,
      eventDeliveries: new Map(),
      pendingSteers: [],
      codeRuleRevision: 0,
      codeRuleSourceVersions: new Map(),
      undeliveredSources: new Set(),
      ownerClaim: undefined,
      codeRuleWorkspace: Promise.resolve(undefined),
      codeRuleReady: false,
      codeRuleOpening: false,
      auditRequestWork: undefined,
      repositoryAudit: undefined,
      // Restored whether or not the evaluator opens, so their results still close them in the ledger.
      pendingCodeRuleCalls: unresolvedCodeRuleCalls(ledger).map(call => ({ id: call.toolCallId, toolName: call.toolName })),
      deferredCodeRuleBoundaries: [],
      closed: false,
    };
    // Set when the evaluator failed to open: what it observed while opening belongs to no live evaluator.
    let retired = false;
    const auditSettings = this.#controls.getWorkspaceSettings(checkout.cwd);
    const steerDelivery: RuleSteerDelivery = {
      requestSteer: async (sessionId, requested, content, metadata) => {
        const activeState = this.#sessions.get(sessionId);
        if (activeState !== opened || activeState.closed || retired) return "discarded";
        const revision = metadata?.revision ?? activeState.codeRuleRevision;
        const pending: PendingSteer = {
          delivery: {
            sessionId,
            requested,
            mechanism: ADAPTER_CAPABILITIES[activeState.descriptor.host].delivery[requested],
            content,
            ruleIds: metadata?.ruleIds ?? [],
          },
          revision,
          scope: metadata?.scope ?? [],
        };
        if (!this.#isCurrentSteer(activeState, pending)) {
          this.#recordUndelivered(activeState, pending);
          return "discarded";
        }
        activeState.pendingSteers.push(pending);
        if (activeState.codeRuleReady && metadata?.background === true && this.#deliverBackground !== undefined) {
          const inFlight = this.#deliverSteer(activeState, pending, true);
          const waitsForNativeBoundary = metadata.generatedEdit === true
            && ADAPTER_CAPABILITIES[activeState.descriptor.host].toolSurface === "native";
          if (waitsForNativeBoundary) await inFlight;
        }
        return "queued";
      },
      revised: (sessionId, revision, paths) => {
        const current = this.#sessions.get(sessionId);
        if (current !== opened || current.closed || paths.length === 0) return [];
        for (const path of paths)
          current.codeRuleSourceVersions.set(path, Math.max(current.codeRuleSourceVersions.get(path) ?? 0, revision));
        // A dropped steer may speak for other sources too: those no later revision revised are the auditor's to steer
        // again.
        const surviving = new Set<string>();
        current.pendingSteers = current.pendingSteers.filter(pending => {
          if (this.#isCurrentSteer(current, pending)) return true;
          this.#recordUndelivered(current, pending);
          for (const path of pending.scope)
            if ((current.codeRuleSourceVersions.get(path) ?? 0) <= pending.revision) surviving.add(path);
          return false;
        });
        return [...surviving];
      },
      undelivered: (sessionId, path) => {
        const current = this.#sessions.get(sessionId);
        return current === opened && current.undeliveredSources.delete(path);
      },
    };
    let auditor: CodeRuleAuditor | undefined;
    let bridge: CodeRuleBridge | undefined;
    this.#sessions.set(session.sessionId, opened);
    try {
      auditor = this.#createAuditor({
        workspaceRoot: checkout.cwd,
        steerDelivery,
        audits: this.#audits,
        ...(this.#timingLog === undefined ? {} : { timingLog: this.#timingLog }),
        ...this.#auditSettings,
        ...auditSettings,
      });
      bridge = new CodeRuleBridge(checkout.cwd, auditor, {
        observations: this.#sourceObservations,
        ...(this.#discoveryWaitMs === undefined ? {} : { discoveryWaitMs: this.#discoveryWaitMs }),
        // A boundary that answered before its shell-write check finished leaves its session's history marked partial.
        onDiscoveryPending: boundary => this.#recordSessionFact(
          session.sessionId,
          (store, descriptor, attribution) => store.recordDiscoveryPending(descriptor, boundary, attribution),
        ),
        onRevision: (sessionId, revision, paths) => {
          const current = this.#sessions.get(sessionId);
          if (current !== opened || current.closed || paths.length === 0) return;
          current.codeRuleRevision = Math.max(current.codeRuleRevision, revision);
          for (const path of paths) current.codeRuleSourceVersions.set(path, revision);
          current.pendingSteers = current.pendingSteers.filter(pending => {
            if (this.#isCurrentSteer(current, pending)) return true;
            this.#recordUndelivered(current, pending);
            return false;
          });
        },
      });
      opened.codeRuleBridge = bridge;
      await this.#beforeSessionPublish?.(session);
      this.#sessionStore.openSession(session, startedAt, this.#repositoryKey(checkout));
      this.#closedSessions.delete(session.sessionId);
      this.#sessionStore.startHudTracking(session, completeTracking);
      opened.ownerClaim = this.#controls.registerOwner(session.sessionId, this.#deliverBackground === undefined ? "next-event" : "background");
    } catch (error) {
      await this.#abandonSession(session.sessionId, opened, auditor);
      throw error;
    }
    const settings = { ...this.#auditSettings, ...auditSettings };
    opened.codeRuleOpening = true;
    opened.codeRuleWorkspace = this.#openCodeRuleWorkspace(opened, bridge, auditor, ledger, settings).then(open => {
      if (open === undefined) {
        retired = true;
        opened.pendingSteers = [];
        return undefined;
      }
      opened.codeRuleReady = true;
      if (this.#deliverBackground !== undefined) {
        // Delivery removes entries from the list, so iterate a snapshot.
        const pendingSteerSnapshot = [...opened.pendingSteers];
        for (const pending of pendingSteerSnapshot) {
          void this.#deliverSteer(opened, pending, true);
        }
      }
      return open;
    }).catch((error: unknown) => {
      // Nothing awaits this until a boundary or the close does: a fault here must not become an unhandled rejection.
      const message = error instanceof Error ? error.message : String(error);
      process.stderr.write(`rerail: code rules are off for session ${session.sessionId}: ${message}\n`);
      retired = true;
      opened.codeRuleOffReason = message;
      opened.pendingSteers = [];
      return undefined;
    }).finally(() => {
      opened.codeRuleOpening = false;
    });
    const workspace = opened.codeRuleWorkspace;
    this.#evaluatorOpens.add(workspace);
    const released = (): void => { this.#evaluatorOpens.delete(workspace); };
    void workspace.then(released, released);
    return { processedEventIds: [...processed] };
  }

  /**
   * Opens a published session's evaluator behind its attach: Git discovery and the content-store snapshot of a large
   * checkout take longer than a host waits for a session to start. The calls a previous runtime left unresolved are
   * begun again before any new boundary sees the bridge, and the repository audit starts once it is open. An
   * evaluator that cannot open is recorded as a failed audit; the session goes on without code rules.
   */
  async #openCodeRuleWorkspace(
    state: RuntimeSessionState,
    bridge: CodeRuleBridge,
    auditor: CodeRuleAuditor,
    ledger: readonly LedgerEntry[],
    settings: Required<AuditSettings>,
  ): Promise<CodeRuleBridge | undefined> {
    const { sessionId } = state.descriptor;
    let opened = false;
    try {
      const openStartedAt = performance.now();
      await bridge.openWorkspace(sessionId);
      this.#timingLog?.(`rerail: code-rule evaluator of session ${sessionId} opened in ${Math.round(performance.now() - openStartedAt)} ms\n`);
      opened = true;
      for (const call of unresolvedCodeRuleCalls(ledger)) {
        if (state.closed) break;
        await bridge.begin(call.toolCallId, call.toolName, { paths: call.paths }, call.preimages ?? {});
      }
      // Boundaries answered without the evaluator while it opened: their begins and finishes run now, in arrival
      // order, until none is left. A begin has no preimage to restore: the edit it audits reads as uncaptured.
      while (state.deferredCodeRuleBoundaries.length > 0 && !state.closed) {
        const next = state.deferredCodeRuleBoundaries[0]!;
        if (next.kind === "call") await bridge.begin(next.toolCallId, next.toolName, { paths: next.paths }, {});
        else await bridge.finish(next.toolCallId, next.failed);
        state.deferredCodeRuleBoundaries.shift();
      }
    } catch (error) {
      delete state.codeRuleBridge;
      state.deferredCodeRuleBoundaries = [];
      const release = this.#lateRelease();
      await (opened ? bridge.close(release) : auditor.close(release)).catch(() => undefined);
      state.codeRuleOffReason = error instanceof Error ? error.message : String(error);
      if (!state.closed) this.#recordWorkspaceFailure(state, settings, error);
      return undefined;
    }
    if (state.closed) {
      // The session closed while the evaluator opened: close did not wait, so the evaluator is released here, unobserved.
      delete state.codeRuleBridge;
      await bridge.close(this.#lateRelease()).catch(() => undefined);
      return undefined;
    }
    // No await from the drain's last check to here: a close from now on finds the bridge published and closes it
    // itself, and a boundary that finds the evaluator still opening has queued for the drain, one that finds it open
    // uses the bridge.
    state.codeRuleOpening = false;
    if (auditor.audit !== undefined) state.repositoryAudit = this.#runRepositoryAudit(sessionId, state);
    return bridge;
  }

  /** A late release of an evaluator must never end the session-wide audits: the id may belong to a replacement by now. */
  #lateRelease(): AuditorCloseOptions {
    return { interruptAudits: false };
  }

  #recordWorkspaceFailure(state: RuntimeSessionState, settings: Required<AuditSettings>, error: unknown): void {
    const message = error instanceof Error ? error.message : String(error);
    process.stderr.write(`rerail: code rules are off for session ${state.descriptor.sessionId}: ${message}\n`);
    try {
      const auditId = this.#audits.start({
        sessionId: state.descriptor.sessionId, workspaceRoot: state.checkout.cwd, scope: settings.scope, mode: settings.mode,
        revision: 0, catalogRevision: null, ruleRevisions: [], paths: [],
      });
      this.#audits.finish(auditId, {
        status: "failed",
        failures: [{ clauseId: "catalog", code: "adapter-error", message, stage: "evaluate" }],
      });
    } catch (recordError) {
      process.stderr.write(`rerail: policy execution recording failed: ${recordError instanceof Error ? recordError.message : String(recordError)}\n`);
    }
  }

  /** Failed attachment releases the evaluator, which never opened, and unpublishes whatever was published. */
  async #abandonSession(sessionId: string, opened: RuntimeSessionState, auditor: CodeRuleAuditor | undefined): Promise<void> {
    opened.closed = true;
    opened.pendingSteers = [];
    this.#sessions.delete(sessionId);
    if (opened.ownerClaim !== undefined) {
      this.#controls.interrupt(sessionId);
      this.#controls.interruptRepositoryAudit(sessionId);
      this.#controls.releaseOwner(sessionId, opened.ownerClaim);
      this.#sessionStore.closeSession(sessionId);
    }
    try {
      await auditor?.close();
    } catch {
      // Cleanup must not replace the initialization error.
    }
  }

  async closeSession(sessionId: string): Promise<void> {
    const state = this.#sessions.get(sessionId);
    if (state !== undefined) {
      state.closed = true;
      state.pendingSteers = [];
      this.#closedSessions.set(sessionId, {
        descriptor: state.descriptor, startedAt: state.startedAt, endedAt: Date.now(), repositoryKey: this.#repositoryKey(state.checkout),
      });
      if (state.ownerClaim !== undefined) {
        this.#controls.interrupt(sessionId);
        this.#controls.interruptRepositoryAudit(sessionId);
        this.#controls.releaseOwner(sessionId, state.ownerClaim);
      }
    }
    let bridge: CodeRuleBridge | undefined;
    try {
      // An evaluator still opening is not waited for: it finds the session closed once it has opened and releases itself,
      // ending only its own audits. The session's other audits end now, while this runtime still owns the id.
      if (state?.codeRuleOpening === true) this.#audits.interrupt(sessionId);
      else {
        bridge = await state?.codeRuleWorkspace;
        bridge?.interrupt();
        await state?.auditRequestWork;
      }
    } finally {
      try {
        try {
          await bridge?.close();
        } finally {
          await state?.repositoryAudit;
        }
      } finally {
        this.#sessions.delete(sessionId);
        this.#sessionStore.closeSession(sessionId, this.#closedSessions.get(sessionId)?.endedAt);
      }
    }
  }

  /** The audit at session start: one repository audit per checkout it reaches, each claimed before it starts. */
  async #runRepositoryAudit(sessionId: string, state: RuntimeSessionState): Promise<void> {
    const claimed = new Set<string>();
    const audit: RepositoryAuditHooks = {
      claim: (checkoutRoot, catalogRevision) => {
        const won = this.#controls.claimRepositoryAudit(sessionId, checkoutRoot, catalogRevision);
        if (won) claimed.add(checkoutRoot);
        return won;
      },
      finish: (checkoutRoot, auditId) => {
        claimed.delete(checkoutRoot);
        const audit = auditId === undefined ? undefined : this.#audits.get(auditId);
        if (audit?.status === "completed") {
          this.#controls.finishRepositoryAudit(sessionId, checkoutRoot, audit.auditId, "completed");
          return;
        }
        const status = audit?.status === "interrupted" ? "interrupted" : "failed";
        const error = audit === undefined
          ? "Repository diagnostic completed without a policy run"
          : audit.failures.map(failure => failure.message).join("; ") || `Repository diagnostic ${audit.status}`;
        this.#controls.finishRepositoryAudit(sessionId, checkoutRoot, auditId ?? null, status, error);
      },
    };
    try {
      await state.codeRuleBridge!.audit({ scope: "repository", mode: "log", audit });
    } catch (error) {
      for (const checkoutRoot of claimed) {
        this.#controls.finishRepositoryAudit(sessionId, checkoutRoot, null, "failed", error instanceof Error ? error.message : String(error));
      }
    }
  }

  async persistEvent(event: AdapterEvent): Promise<void> {
    const sessionId = event.kind === "session-start" ? event.session.sessionId : event.sessionId;
    const state = this.#session(sessionId);
    this.#sessionStore.append({
      sessionId,
      eventId: event.eventId,
      kind: event.kind,
      payload: event,
    }, state.descriptor);
    await this.#append(state, { type: "event", eventId: event.eventId });
    state.processed.add(event.eventId);
  }

  async persistTurn(sessionId: string, eventId: string): Promise<void> {
    const state = this.#session(sessionId);
    await this.#append(state, { type: "turn", eventId });
    state.turn += 1;
    this.#nativeSession?.onTurn?.(state.turn);
  }

  async persistDelivery(delivery: AdapterDelivery): Promise<void> {
    await this.#append(this.#session(delivery.sessionId), { type: "delivery", delivery });
  }

  async acceptSample(sample: AdapterSample): Promise<void> {
    const state = this.#session(sample.sessionId);
    const plainUserMessage = sample.source === "user-input" && sample.boundary === "message";
    const id = plainUserMessage ? sample.eventId : `${sample.eventId}:${sample.boundary}:${state.sampleSequence++}`;
    const owner = sampleOwner(sample.source);
    const item: ContentItem = {
      ref: { source: "host:event", id },
      text: sample.text,
      metadata: {
        owner,
        session: sample.sessionId,
        fields: { eventId: sample.eventId, sampleSource: sample.source, boundary: sample.boundary },
      },
    };
    state.source.append(item);
    await this.#append(state, { type: "sample", item });
    const registry = state.registry;
    if (registry === undefined) return;
    const matches = registry.match(sample, { turn: state.turn });

    if (matches.length === 0) return;
    const deliveries = state.eventDeliveries.get(sample.eventId) ?? [];
    for (const match of matches) {
      registry.markDelivered([match.ruleId], state.turn);
      await this.#append(state, { type: "rule-match", ruleId: match.ruleId, turn: state.turn });
      this.#nativeSession?.onMatch(match, sample, state.turn);
      const delivery: AdapterDelivery = {
        sessionId: sample.sessionId,
        requested: match.mode,
        mechanism: ADAPTER_CAPABILITIES[state.descriptor.host].delivery[match.mode],
        content: steerBlock(match.ruleId, match.action.content, { mode: match.mode, origin: steerOrigin(match.ruleId) }),
        // Conversation rule matches are not code-rule steers, so they carry no code-rule ids.
        ruleIds: [],
      };
      await this.#append(state, { type: "delivery", delivery });
      deliveries.push(delivery);
    }
    state.eventDeliveries.set(sample.eventId, deliveries);
  }

  #scheduleAuditRequests(sessionId: string): void {
    const state = this.#sessions.get(sessionId);
    if (state === undefined || state.closed || state.ownerClaim === undefined) return;
    state.codeRuleBridge?.updateSettings(this.#controls.hasUserSettings(sessionId)
      ? this.#controls.getSettings(sessionId).settings
      : this.#auditSettings);
    if (state.auditRequestWork !== undefined) return;
    const work = this.#runAuditRequests(sessionId);
    state.auditRequestWork = work;
    void work.catch((error: unknown) => {
      process.stderr.write(`rerail: policy control scan failed: ${error instanceof Error ? error.message : String(error)}\n`);
    }).finally(() => {
      if (state.auditRequestWork === work) state.auditRequestWork = undefined;
    });
  }
  async #runAuditRequests(sessionId: string): Promise<void> {
    const state = this.#sessions.get(sessionId);
    if (state === undefined || state.closed || state.ownerClaim === undefined) return;
    const request = this.#controls.claim(sessionId, state.ownerClaim);
    if (request === undefined) return;
    if (request.scope === "ai-output") {
      this.#controls.finish(request.id, "failed", [], "Explicit ai-output scans are not supported");
      return;
    }
    try {
      const workspace = await state.codeRuleWorkspace;
      if (workspace === undefined) {
        // The evaluator never opened: the scan says why, not that a run went unrecorded.
        if (state.closed) this.#controls.finish(request.id, "interrupted", [], "Session closed during scan");
        else this.#controls.finish(request.id, "failed", [], `Code rules are off for this session: ${state.codeRuleOffReason ?? "the policy evaluator did not open"}`);
        return;
      }
      const auditIds = await workspace.audit({
        scope: request.scope,
        mode: request.mode,
        ...(request.paths.length === 0 ? {} : { paths: request.paths }),
      });
      if (state.closed) {
        this.#controls.finish(request.id, "interrupted", auditIds, "Session closed during scan");
        return;
      }
      if (auditIds.length === 0) {
        this.#controls.finish(request.id, "failed", [], "Policy evaluator completed scan without a recorded run");
        return;
      }
      const verdict = auditRequestVerdict(auditIds, auditIds.map(auditId => this.#audits.get(auditId)));
      this.#controls.finish(request.id, verdict.status, [...auditIds], verdict.error);
    } catch (error) {
      this.#controls.finish(
        request.id,
        state.closed ? "interrupted" : "failed",
        [],
        error instanceof Error ? error.message : String(error),
      );
    }
  }

  #isCurrentSteer(state: RuntimeSessionState, pending: PendingSteer): boolean {
    return !state.closed
      && this.#sessions.get(state.descriptor.sessionId) === state
      && (pending.scope.length === 0
        ? pending.revision >= state.codeRuleRevision
        : pending.scope.every(path => (state.codeRuleSourceVersions.get(path) ?? 0) <= pending.revision));
  }

  /** Marks the sources whose newer revision keeps `pending` from the agent (`undeliveredSources`). */
  #recordUndelivered(state: RuntimeSessionState, pending: PendingSteer): void {
    for (const path of pending.scope)
      if ((state.codeRuleSourceVersions.get(path) ?? 0) > pending.revision) state.undeliveredSources.add(path);
  }

  #deliverSteer(
    state: RuntimeSessionState,
    pending: PendingSteer,
    native: boolean,
  ): Promise<AdapterDelivery | undefined> {
    const inFlight = this.#performSteer(state, pending, native);
    this.#steersInFlight.add(inFlight);
    const settled = (): void => { this.#steersInFlight.delete(inFlight); };
    void inFlight.then(settled, settled);
    return inFlight;
  }

  async #performSteer(
    state: RuntimeSessionState,
    pending: PendingSteer,
    native: boolean,
  ): Promise<AdapterDelivery | undefined> {
    const index = state.pendingSteers.indexOf(pending);
    if (index < 0) return undefined;
    state.pendingSteers.splice(index, 1);
    if (!this.#isCurrentSteer(state, pending)) {
      this.#recordUndelivered(state, pending);
      return undefined;
    }
    if (this.#stopped()) return undefined;
    try {
      await this.persistDelivery(pending.delivery);
    } catch (error) {
      process.stderr.write(`rerail: could not record policy delivery: ${error instanceof Error ? error.message : String(error)}\n`);
    }
    // A stop committed elsewhere while the ledger was written withholds the result.
    if (!this.#isCurrentSteer(state, pending)) {
      this.#recordUndelivered(state, pending);
      return undefined;
    }
    if (this.#stopped()) return undefined;
    if (!native) return pending.delivery;
    try {
      await this.#deliverBackground!(pending.delivery);
    } catch (error) {
      process.stderr.write(`rerail: policy delivery failed: ${error instanceof Error ? error.message : String(error)}\n`);
    }
    return undefined;
  }

  async #ingestSessionStart(event: Extract<AdapterEvent, { kind: "session-start" }>): Promise<readonly AdapterDelivery[]> {
    assertEventHost(event.session.host);
    if (this.#autostart) autostartDaemon();
    await this.#router(event.session.host).ingest(event);
    return [];
  }

  #resolveEventSession(event: Exclude<AdapterEvent, { kind: "session-start" }>): {
    active: AdapterSession | undefined;
    descriptor: AdapterSession;
  } {
    const active = this.#sessions.get(event.sessionId)?.descriptor;
    const descriptor = active ?? event.session;
    if (descriptor === undefined) throw new Error(`Inactive session: ${event.sessionId}`);
    if (descriptor.sessionId !== event.sessionId) throw new Error("Event session descriptor does not match sessionId");
    assertEventHost(descriptor.host);
    if (active !== undefined && event.session !== undefined && active.host !== event.session.host) {
      throw new Error("Event session host does not match active session");
    }
    return { active, descriptor };
  }

  /**
   * The checkout the session starts in, from its folder resolved once here. A trusted caller records it. A caller
   * without the daemon token is refused unless that resolved folder is in a recorded,
   * started checkout: the check and every later use share one resolution, so a folder swapped after a caller's own
   * check gains nothing.
   */
  #startingCheckout(session: AdapterSession): StartingCheckout {
    // A folder that does not exist fails here (ENOENT), before anything is recorded or created for it.
    const cwd = realpathSync(session.cwd);
    const { database } = this.#catalogs;
    const known = this.#trusted ? recordCheckout(database, cwd) : lookupCheckout(database, cwd);
    if (known !== undefined && !known.git) this.#owners.addPlainRoot(known.checkoutPath);
    const start = startingCheckout(this.#owners, cwd);
    const eligibility = this.#catalogs.refresh(start.checkoutRoot);
    if (!this.#trusted && (eligibility === "unrecorded" || eligibility === "stopped")) {
      throw new RangeError("session cwd is not in a started checkout");
    }
    return { root: start.checkoutRoot, folder: start.path, cwd };
  }

  async #restoreRegistry(
    session: AdapterSession,
    ledger: readonly LedgerEntry[],
    checkout: StartingCheckout,
  ): Promise<{
    registry: ConversationRuleRegistry | undefined;
    turn: number;
    ruleSync?: LiveConversationRules | undefined;
  }> {
    if (this.#nativeSession !== undefined) return this.#nativeSession;
    const registry = this.#modelPath === undefined ? undefined : new ConversationRuleRegistry({ modelPath: this.#modelPath });
    if (registry === undefined) {
      return { registry, turn: 0, ruleSync: undefined };
    }
    const ruleSync = new LiveConversationRules(this.#catalogs.conversationRules(checkout.root, checkout.folder), registry);
    ruleSync.prime();
    return {
      registry,
      turn: replayLedger(registry, ledger),
      ruleSync,
    };
  }

  async deliver(_delivery: AdapterDelivery): Promise<void> {
    // Hook hosts convert persisted deliveries into hook-specific context responses.
  }

  #router(host: RerailHost): RerailHostRuntime {
    const existing = this.#routers.get(host);
    if (existing !== undefined) return existing;
    const profile = ADAPTER_CAPABILITIES[host];
    const router = new RerailHostRuntime(profile, this);
    this.#routers.set(host, router);
    return router;
  }

  #session(sessionId: string): RuntimeSessionState {
    const state = this.#sessions.get(sessionId);
    if (state === undefined) throw new Error(`Inactive session: ${sessionId}`);
    return state;
  }

  async #append(state: RuntimeSessionState, entry: LedgerEntry): Promise<void> {
    await appendJsonLine(state.ledgerPath, entry, state.ledgerAppendState);
  }
}
