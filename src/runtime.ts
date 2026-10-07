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
} from "./adapter-contract.js";
import { RerailHostRuntime, type HostRuntimePort, type HostRuntimeSessionState } from "./host-runtime.js";
import { autostartDaemon } from "./daemon-control.js";
import { ExecutablePolicyBridge, generatedEditPaths, type SourceObservationCache } from "./executable-policy-bridge.js";
import { NativeExecutablePolicyEnforcer } from "./executable-policy-enforcer.js";
import type { ExecutablePolicyEvaluator, PolicyEvaluationSettings, PolicySteeringDelivery } from "./executable-policy.js";
import type { PolicyExecutionRecorder, PolicyRunRecord } from "./policy-execution.js";
import { SqlitePolicyExecutionStore } from "./policy-execution-store.js";
import { DEFAULT_POLICY_SETTINGS, SqliteScanControlStore, type PolicyOwnerClaim } from "./scan-control.js";
import { bootstrapPolicyDslCatalog, type PolicyDslCatalog } from "./policy-dsl-catalog.js";
import { loadBundledRules } from "./rule-bootstrap.js";
import { defaultRerailDatabasePath } from "./database.js";
import { resolveRepositoryContext, ScopedRuleCatalog, type RepositoryContext } from "./scoped-rule-catalog.js";
import { effectiveRepositoryStartState, RepositoryStartWatch } from "./repository-start.js";
import { cosineSimilarity, NativeSemanticEmbedder, SemanticWorkspace } from "./semantic-primitives.js";
import { CosineRuleRegistry, type RuleMatch } from "./registry.js";
import { LiveCosineRules } from "./cosine-rule-sync.js";
import { parseSampleSource, SAMPLE_BOUNDARIES } from "./rule-schema.js";
import { cosineRuleLoader } from "./rule-sources.js";
import { RULE_MODES } from "./rule-mode.js";
import { restorableRule, type RuleMutation } from "./session-state.js";
import { SessionLanes } from "./session-lanes.js";
import { SqliteTranscriptStore } from "./transcript-store.js";
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
} from "./semantic-types.js";

const RULE_DIRECTORIES: Readonly<Partial<Record<RerailHost, string>>> = {
  "claude-code": ".claude",
  cursor: ".cursor",
  omp: ".omp",
  pi: ".pi",
};

function assertEventHost(host: RerailHost): void {
  if (Object.hasOwn(ADAPTER_CAPABILITIES, host)) return;
  throw new Error(`Event runtime does not support host: ${host}`);
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
interface PolicyScanVerdict {
  status: "completed" | "failed" | "interrupted";
  error: string | null;
}
/** Outcome of an explicit scan from its recorded runs: any missing or non-completed run fails the whole request. */
function policyScanVerdict(runIds: readonly string[], runs: readonly (PolicyRunRecord | undefined)[]): PolicyScanVerdict {
  const unavailable = runs.indexOf(undefined);
  if (unavailable >= 0) return { status: "failed", error: `Policy run ${runIds[unavailable]} was not recorded` };
  const completed = runs as PolicyRunRecord[];
  const interrupted = completed.find(run => run.status === "interrupted");
  if (interrupted !== undefined) return { status: "interrupted", error: `Policy run ${interrupted.auditId} was interrupted` };
  const failed = completed.find(run => run.status !== "completed");
  if (failed !== undefined) {
    const details = failed.failures.map(failure => failure.message).filter(Boolean).join("; ");
    return { status: "failed", error: details || `Policy run ${failed.auditId} ended ${failed.status}` };
  }
  return { status: "completed", error: null };
}
/** Policy calls recorded without a matching result, in ledger order. */
function unresolvedPolicyCalls(ledger: readonly LedgerEntry[]): Extract<LedgerEntry, { type: "policy-call" }>[] {
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
      // Ledger entries written before re-rail attribution carry no rule ids.
      ruleIds: z.array(z.string()).default([]),
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

interface PendingPolicyDelivery {
  delivery: AdapterDelivery;
  revision: number;
  scope: string[];
}

interface RuntimeSessionState {
  descriptor: AdapterSession;
  startedAt: number;
  processed: Set<string>;
  source: MutableContentSource;
  workspace: SemanticWorkspace;
  selections: Map<string, Selection>;
  projections: Map<string, SemanticProjection>;
  ledgerPath: string;
  ledgerAppendState: JsonlAppendState;
  sampleSequence: number;
  registry?: CosineRuleRegistry;
  ruleSync: LiveCosineRules | undefined;
  turn: number;
  eventDeliveries: Map<string, AdapterDelivery[]>;
  backgroundDeliveries: PendingPolicyDelivery[];
  policyRevision: number;
  policySourceVersions: Map<string, number>;
  policyBridge?: ExecutablePolicyBridge;
  ownerClaim: PolicyOwnerClaim | undefined;
  policyControl: Promise<void> | undefined;
  repositoryDiagnostic: Promise<void> | undefined;
  pendingPolicyCalls: Array<{ id: string; toolName: string }>;
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
function replayLedger(registry: CosineRuleRegistry, ledger: readonly LedgerEntry[]): number {
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

function applyRuleMutation(registry: CosineRuleRegistry, mutation: RuleMutation): void {
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
  /** Checkout root and stable repository identity used to resolve project rules. */
  workspaceRoot: string;
  repositoryContext?: RepositoryContext;
  policy?: PolicyEvaluationSettings;
  /** Delivers a completed background policy result without waiting for another host event. */
  deliverBackground?: (delivery: AdapterDelivery) => Promise<void> | void;
  /** Transfers a prepared session's host ownership before durable publication. */
  beforeSessionPublish?: (session: AdapterSession) => Promise<void> | void;
  /** Test and alternate-runtime seam for policy execution. */
  createPolicyEvaluator?: (options: {
    workspaceRoot: string;
    delivery: PolicySteeringDelivery;
    executions: PolicyExecutionRecorder;
  } & PolicyEvaluationSettings) => ExecutablePolicyEvaluator;
  nativeSession?: {
    registry: CosineRuleRegistry;
    ruleSync?: LiveCosineRules;
    turn: number;
    onMatch(match: RuleMatch, sample: AdapterSample, turn: number): void;
    onTurn?(turn: number): void;
  };
}

export class RerailRuntime implements HostRuntimePort, RerailEventLoop {
  readonly #storageRoot: string;
  readonly #embedder: SemanticEmbedder;
  readonly #modelPath: string | undefined;
  readonly #transcripts: SqliteTranscriptStore;
  readonly #executions: SqlitePolicyExecutionStore;
  readonly #policyHandoffs = new Set<Promise<AdapterDelivery | undefined>>();
  readonly #policyCatalog: PolicyDslCatalog;
  readonly #ruleCatalog: ScopedRuleCatalog;
  /** Attribution for every session this runtime records, kept past disposal for late receipts. */
  readonly #repositoryId: string;
  readonly #sessions = new Map<string, RuntimeSessionState>();
  readonly #closedSessions = new Map<string, { descriptor: AdapterSession; startedAt: number; endedAt: number }>();
  #transcriptsClosed = false;
  readonly #routers = new Map<RerailHost, RerailHostRuntime>();
  readonly #nativeSession: RerailRuntimeOptions["nativeSession"];
  readonly #createPolicyEvaluator: NonNullable<RerailRuntimeOptions["createPolicyEvaluator"]>;
  readonly #deliverBackground: RerailRuntimeOptions["deliverBackground"];
  readonly #beforeSessionPublish: RerailRuntimeOptions["beforeSessionPublish"];
  readonly #policySettings: Required<PolicyEvaluationSettings>;
  readonly #controls: SqliteScanControlStore;
  readonly #controlTimer: NodeJS.Timeout;
  readonly #autostart: boolean;
  readonly #ingestLanes = new SessionLanes();
  readonly #sourceObservations: SourceObservationCache = new Map();
  readonly #startWatch: RepositoryStartWatch | undefined;

  constructor(options: RerailRuntimeOptions) {
    this.#storageRoot = options.storageRoot;
    this.#modelPath = options.modelPath;
    this.#nativeSession = options.nativeSession;
    this.#deliverBackground = options.deliverBackground;
    this.#beforeSessionPublish = options.beforeSessionPublish;
    this.#policySettings = { ...DEFAULT_POLICY_SETTINGS, ...options.policy };
    this.#autostart = options.autostart ?? true;
    this.#createPolicyEvaluator = options.createPolicyEvaluator ?? ((policy) => new NativeExecutablePolicyEnforcer(
      this.#policyCatalog,
      policy,
    ));
    if (options.embedder !== undefined) this.#embedder = options.embedder;
    else if (options.modelPath !== undefined) this.#embedder = new NativeSemanticEmbedder(options.modelPath);
    else throw new Error("RerailRuntime requires embedder or modelPath");
    const databasePath = options.databasePath ?? defaultRerailDatabasePath();
    const workspaceRoot = resolve(options.workspaceRoot);
    let transcripts: SqliteTranscriptStore | undefined;
    let executions: SqlitePolicyExecutionStore | undefined;
    let controls: SqliteScanControlStore | undefined;
    let ruleCatalog: ScopedRuleCatalog | undefined;
    let policyCatalog: PolicyDslCatalog | undefined;
    let startWatch: RepositoryStartWatch | undefined;
    try {
      const catalog = ScopedRuleCatalog.open(
        databasePath,
        options.repositoryContext ?? resolveRepositoryContext(workspaceRoot),
      );
      ruleCatalog = catalog;
      // Bundled rules arrive only where Rerail is started; `rerail start` seeds a new repository.
      if (effectiveRepositoryStartState(catalog.path, catalog.context()) === "started") catalog.seed(loadBundledRules());
      startWatch = options.startGate === true ? new RepositoryStartWatch(catalog.path, catalog.context()) : undefined;
      policyCatalog = bootstrapPolicyDslCatalog({
        databasePath: catalog.path, store: catalog,
        onRefresh: () => { for (const sessionId of this.#sessions.keys()) this.recordRuleCoverage(sessionId); },
      });
      this.#repositoryId = catalog.context().repositoryId;
      transcripts = new SqliteTranscriptStore(databasePath, { repositoryId: this.#repositoryId });
      executions = new SqlitePolicyExecutionStore(databasePath);
      controls = new SqliteScanControlStore(databasePath);
      this.#ruleCatalog = catalog;
      this.#policyCatalog = policyCatalog;
      this.#transcripts = transcripts;
      this.#executions = executions;
      this.#controls = controls;
      this.#startWatch = startWatch;
    } catch (error) {
      transcripts?.close();
      executions?.close();
      controls?.close();
      startWatch?.close();
      if (policyCatalog !== undefined) policyCatalog.close();
      else ruleCatalog?.close();
      throw error;
    }
    this.#controlTimer = setInterval(() => {
      for (const sessionId of this.#sessions.keys()) this.#schedulePolicyControls(sessionId);
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
        // A session that attached while started still ends, releasing its policy bridge and polling; nothing is recorded.
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

  /** Branch navigation restores repeat/turn state without retiring the live stream or policy owner. */
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
    this.#schedulePolicyControls(state.descriptor.sessionId);
    state.ruleSync?.refresh();
    state.policyBridge?.refreshPolicies();
    if (event.kind === "call") {
      const id = event.toolCallId;
      const preimages = await state.policyBridge?.begin(id, event.toolName, event.input);
      await this.#append(state, {
        type: "policy-call", toolCallId: id, toolName: event.toolName, paths: generatedEditPaths(event.toolName, event.input),
        ...(preimages === undefined ? {} : { preimages }),
      });
      state.pendingPolicyCalls.push({ id, toolName: event.toolName });
    }
    await this.#router(descriptor.host).ingest(event);
    if (event.kind === "result") {
      const index = state.pendingPolicyCalls.findIndex((call) => call.id === event.toolCallId);
      if (index >= 0) {
        const [call] = state.pendingPolicyCalls.splice(index, 1);
        await state.policyBridge?.finish(call!.id, event.failed === true);
        await this.#append(state, { type: "policy-result", toolCallId: call!.id });
        if (ADAPTER_CAPABILITIES[descriptor.host].toolSurface === "native") {
          await state.policyBridge?.settleGeneratedEdits();
        }
      }
    }
    if (event.kind === "turn-end") {
      await state.policyBridge?.checkpoint("turn");
      await state.policyBridge?.settle();
    }
    if (event.kind === "session-end") return [];
    const deliveries = await this.#collectPolicyDeliveries(state);
    deliveries.push(...(state.eventDeliveries.get(event.eventId) ?? []));
    state.eventDeliveries.delete(event.eventId);
    return deliveries;
  }

  /**
   * Completed policy results not yet handed to the host, for hosts that poll instead of
   * receiving `deliverBackground` pushes. Unknown or closed sessions have nothing pending.
   */
  async collectDeliveries(sessionId: string): Promise<AdapterDelivery[]> {
    const state = this.#sessions.get(sessionId);
    return state === undefined || this.#stopped() ? [] : this.#collectPolicyDeliveries(state);
  }

  async #collectPolicyDeliveries(state: RuntimeSessionState): Promise<AdapterDelivery[]> {
    // Delivery removes entries from the list, so iterate a snapshot.
    const deliveries: AdapterDelivery[] = [];
    for (const pending of state.backgroundDeliveries.slice()) {
      const delivery = await this.#deliverPolicyResult(state, pending, false);
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

  recordSteeringDelivery(sessionId: string, receiptId: string, ruleIds: readonly string[] = []): void {
    const state = this.#sessions.get(sessionId);
    const closed = this.#closedSessions.get(sessionId);
    const descriptor = state?.descriptor ?? closed?.descriptor;
    if (descriptor === undefined) throw new Error(`Unknown session: ${sessionId}`);
    // An awaited native send may finish after switching/disposal; its receipt still belongs to this session.
    const transcripts = this.#transcriptsClosed
      ? new SqliteTranscriptStore(this.#transcripts.path, { repositoryId: this.#repositoryId })
      : this.#transcripts;
    try {
      transcripts.recordSteeringDelivery(descriptor, receiptId, ruleIds, {
        startedAt: state?.startedAt ?? closed!.startedAt,
        endedAt: closed?.endedAt ?? null,
      });
    } finally {
      if (this.#transcriptsClosed) transcripts.close();
    }
  }

  /** Report loaded sources, never refresh enforcement for presentation metadata. */
  recordRuleCoverage(sessionId: string): void {
    const state = this.#sessions.get(sessionId);
    // Unpublished sessions have no transcript row yet; the session-start ingest records coverage once they do.
    if (state === undefined || state.closed || state.ownerClaim === undefined) return;
    try {
      this.#transcripts.recordRuleCoverage(state.descriptor, {
        repositoryId: this.#ruleCatalog.context().repositoryId,
        activeRuleCount: (state.registry?.count ?? 0) + this.#policyCatalog.status.ruleCount,
        conversationRuleSignature: state.ruleSync?.sourceSignature ?? "",
        fileRuleSignature: this.#policyCatalog.status.sourceSignature,
      });
    } catch (error) {
      console.error("rerail rule coverage:", error);
    }
  }
  async dispose(): Promise<void> {
    clearInterval(this.#controlTimer);
    await Promise.all([...this.#sessions.keys()].map(sessionId => this.closeSession(sessionId)));
    await Promise.allSettled(this.#policyHandoffs);
    this.#controls.close();
    this.#executions.close();
    this.#transcripts.close();
    this.#transcriptsClosed = true;
    this.#policyCatalog.close();
    this.#startWatch?.close();
  }

  async openSession(session: AdapterSession): Promise<HostRuntimeSessionState> {
    const existing = this.#sessions.get(session.sessionId);
    if (existing !== undefined) return { processedEventIds: [...existing.processed] };
    const recordedSession = this.#transcripts.session(session.sessionId);
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
    const { registry, turn, ruleSync } = this.#restoreRegistry(session, ledger);
    const items = ledger.flatMap((entry) => (entry.type === "sample" ? [entry.item] : []));
    const source = new MutableContentSource(items);
    const watermarks = await JsonlWatermarkStore.open(join(directory, "watermarks.jsonl"));
    const opened: RuntimeSessionState = {
      descriptor: session,
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
      backgroundDeliveries: [],
      policyRevision: 0,
      policySourceVersions: new Map(),
      ownerClaim: undefined,
      policyControl: undefined,
      repositoryDiagnostic: undefined,
      pendingPolicyCalls: [],
      closed: false,
    };
    let ready = false;
    const policySettings = this.#controls.getWorkspaceSettings(session.cwd);
    const delivery: PolicySteeringDelivery = {
      requestDelivery: async (sessionId, requested, content, metadata) => {
        const activeState = this.#sessions.get(sessionId);
        if (activeState !== opened || activeState.closed) return "discarded";
        const revision = metadata?.revision ?? activeState.policyRevision;
        const pending: PendingPolicyDelivery = {
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
        if (!this.#currentPolicyResult(activeState, pending)) return "discarded";
        activeState.backgroundDeliveries.push(pending);
        if (ready && metadata?.background === true && this.#deliverBackground !== undefined) {
          const handoff = this.#deliverPolicyResult(activeState, pending, true);
          const waitsForNativeBoundary = metadata.generatedEdit === true
            && ADAPTER_CAPABILITIES[activeState.descriptor.host].toolSurface === "native";
          if (waitsForNativeBoundary) await handoff;
        }
        return "queued";
      },
    };
    let evaluator: ExecutablePolicyEvaluator | undefined;
    let workspaceOpened = false;
    this.#sessions.set(session.sessionId, opened);
    try {
      evaluator = this.#createPolicyEvaluator({
        workspaceRoot: session.cwd,
        delivery,
        executions: this.#executions,
        ...this.#policySettings,
        ...policySettings,
      });
      opened.policyBridge = new ExecutablePolicyBridge(session.cwd, evaluator, {
        observations: this.#sourceObservations,
        onRevision: (sessionId, revision, paths) => {
          const current = this.#sessions.get(sessionId);
          if (current !== opened || current.closed || paths.length === 0) return;
          current.policyRevision = Math.max(current.policyRevision, revision);
          for (const path of paths) current.policySourceVersions.set(path, revision);
          current.backgroundDeliveries = current.backgroundDeliveries.filter(pending => this.#currentPolicyResult(current, pending));
        },
      });
      await opened.policyBridge.openWorkspace(session.sessionId);
      workspaceOpened = true;
      for (const call of unresolvedPolicyCalls(ledger)) {
        opened.pendingPolicyCalls.push({ id: call.toolCallId, toolName: call.toolName });
        await opened.policyBridge.begin(call.toolCallId, call.toolName, { paths: call.paths }, call.preimages ?? {});
      }
      await this.#beforeSessionPublish?.(session);
      this.#transcripts.openSession(session, startedAt);
      this.#closedSessions.delete(session.sessionId);
      this.#transcripts.startHudTracking(session, completeTracking);
      opened.ownerClaim = this.#controls.registerOwner(session.sessionId, this.#deliverBackground === undefined ? "next-event" : "background");
      if (this.#controls.claimRepositoryDiagnostic(session.sessionId)) {
        opened.repositoryDiagnostic = this.#runRepositoryDiagnostic(session.sessionId, opened);
      }
      ready = true;
      if (this.#deliverBackground !== undefined) {
        // Delivery removes entries from the list, so iterate a snapshot.
        const backgroundDeliverySnapshot = [...opened.backgroundDeliveries];
        for (const pending of backgroundDeliverySnapshot) {
          void this.#deliverPolicyResult(opened, pending, true);
        }
      }
      return { processedEventIds: [...processed] };
    } catch (error) {
      await this.#abandonSession(session.sessionId, opened, evaluator, workspaceOpened);
      throw error;
    }
  }

  /** Failed attachment cancels discovery and releases the evaluator without a final checkpoint. */
  async #abandonSession(
    sessionId: string,
    opened: RuntimeSessionState,
    evaluator: ExecutablePolicyEvaluator | undefined,
    workspaceOpened: boolean,
  ): Promise<void> {
    opened.closed = true;
    opened.backgroundDeliveries = [];
    this.#sessions.delete(sessionId);
    if (opened.ownerClaim !== undefined) {
      this.#controls.interrupt(sessionId);
      this.#controls.interruptRepositoryDiagnostic(sessionId);
      this.#controls.releaseOwner(sessionId, opened.ownerClaim);
      this.#transcripts.closeSession(sessionId);
    }
    try {
      if (workspaceOpened) await opened.policyBridge!.close();
      else await evaluator?.close();
    } catch {
      // Cleanup must not replace the initialization error.
    }
  }

  async closeSession(sessionId: string): Promise<void> {
    const state = this.#sessions.get(sessionId);
    if (state !== undefined) {
      state.closed = true;
      state.backgroundDeliveries = [];
      this.#closedSessions.set(sessionId, { descriptor: state.descriptor, startedAt: state.startedAt, endedAt: Date.now() });
      if (state.ownerClaim !== undefined) {
        this.#controls.interrupt(sessionId);
        this.#controls.interruptRepositoryDiagnostic(sessionId);
        this.#controls.releaseOwner(sessionId, state.ownerClaim);
      }
      state.policyBridge?.interrupt();
    }
    try {
      await state?.policyControl;
    } finally {
      try {
        try {
          await state?.policyBridge?.close();
        } finally {
          await state?.repositoryDiagnostic;
        }
      } finally {
        this.#sessions.delete(sessionId);
        this.#transcripts.closeSession(sessionId, this.#closedSessions.get(sessionId)?.endedAt);
      }
    }
  }

  async #runRepositoryDiagnostic(sessionId: string, state: RuntimeSessionState): Promise<void> {
    try {
      const runIds = await state.policyBridge!.scan({ scope: "repository", mode: "log" });
      const runId = runIds[0] ?? null;
      const run = runId === null ? undefined : this.#executions.get(runId);
      if (run?.status === "completed") {
        this.#controls.finishRepositoryDiagnostic(sessionId, runId, "completed");
        return;
      }
      const status = run?.status === "interrupted" ? "interrupted" : "failed";
      const error = run === undefined
        ? "Repository diagnostic completed without a policy run"
        : run.failures.map(failure => failure.message).join("; ") || `Repository diagnostic ${run.status}`;
      this.#controls.finishRepositoryDiagnostic(sessionId, runId, status, error);
    } catch (error) {
      this.#controls.finishRepositoryDiagnostic(
        sessionId,
        null,
        "failed",
        error instanceof Error ? error.message : String(error),
      );
    }
  }

  async persistEvent(event: AdapterEvent): Promise<void> {
    const sessionId = event.kind === "session-start" ? event.session.sessionId : event.sessionId;
    const state = this.#session(sessionId);
    this.#transcripts.append({
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
        content: match.action.content,
        // Cosine matches are not policy steering, so they carry no policy rule ids.
        ruleIds: [],
      };
      await this.#append(state, { type: "delivery", delivery });
      deliveries.push(delivery);
    }
    state.eventDeliveries.set(sample.eventId, deliveries);
  }

  #schedulePolicyControls(sessionId: string): void {
    const state = this.#sessions.get(sessionId);
    if (state === undefined || state.closed || state.ownerClaim === undefined) return;
    state.policyBridge?.updateSettings(this.#controls.hasUserSettings(sessionId)
      ? this.#controls.getSettings(sessionId).settings
      : this.#policySettings);
    if (state.policyControl !== undefined) return;
    const work = this.#applyPolicyControls(sessionId);
    state.policyControl = work;
    void work.catch((error: unknown) => {
      process.stderr.write(`rerail: policy control scan failed: ${error instanceof Error ? error.message : String(error)}\n`);
    }).finally(() => {
      if (state.policyControl === work) state.policyControl = undefined;
    });
  }
  async #applyPolicyControls(sessionId: string): Promise<void> {
    const state = this.#sessions.get(sessionId);
    if (state === undefined || state.closed || state.ownerClaim === undefined) return;
    const request = this.#controls.claim(sessionId, state.ownerClaim);
    if (request === undefined) return;
    if (request.scope === "ai-output") {
      this.#controls.finish(request.id, "failed", [], "Explicit ai-output scans are not supported");
      return;
    }
    try {
      const runIds = await state.policyBridge?.scan({
        scope: request.scope,
        mode: request.mode,
        ...(request.paths.length === 0 ? {} : { paths: request.paths }),
      });
      if (state.closed) {
        this.#controls.finish(request.id, "interrupted", runIds ?? [], "Session closed during scan");
        return;
      }
      if (runIds === undefined || runIds.length === 0) {
        this.#controls.finish(request.id, "failed", [], "Policy evaluator completed scan without a recorded run");
        return;
      }
      const verdict = policyScanVerdict(runIds, runIds.map(runId => this.#executions.get(runId)));
      this.#controls.finish(request.id, verdict.status, [...runIds], verdict.error);
    } catch (error) {
      this.#controls.finish(
        request.id,
        state.closed ? "interrupted" : "failed",
        [],
        error instanceof Error ? error.message : String(error),
      );
    }
  }

  #currentPolicyResult(state: RuntimeSessionState, pending: PendingPolicyDelivery): boolean {
    return !state.closed
      && this.#sessions.get(state.descriptor.sessionId) === state
      && (pending.scope.length === 0
        ? pending.revision >= state.policyRevision
        : pending.scope.every(path => (state.policySourceVersions.get(path) ?? 0) <= pending.revision));
  }

  #deliverPolicyResult(
    state: RuntimeSessionState,
    pending: PendingPolicyDelivery,
    native: boolean,
  ): Promise<AdapterDelivery | undefined> {
    const handoff = this.#performPolicyDelivery(state, pending, native);
    this.#policyHandoffs.add(handoff);
    const settled = (): void => { this.#policyHandoffs.delete(handoff); };
    void handoff.then(settled, settled);
    return handoff;
  }

  async #performPolicyDelivery(
    state: RuntimeSessionState,
    pending: PendingPolicyDelivery,
    native: boolean,
  ): Promise<AdapterDelivery | undefined> {
    const index = state.backgroundDeliveries.indexOf(pending);
    if (index < 0) return undefined;
    state.backgroundDeliveries.splice(index, 1);
    if (!this.#currentPolicyResult(state, pending) || this.#stopped()) return undefined;
    try {
      await this.persistDelivery(pending.delivery);
    } catch (error) {
      process.stderr.write(`rerail: could not record policy delivery: ${error instanceof Error ? error.message : String(error)}\n`);
    }
    // A stop committed elsewhere while the ledger was written withholds the result.
    if (!this.#currentPolicyResult(state, pending) || this.#stopped()) return undefined;
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

  #restoreRegistry(
    session: AdapterSession,
    ledger: readonly LedgerEntry[],
  ): {
    registry: CosineRuleRegistry | undefined;
    turn: number;
    ruleSync?: LiveCosineRules | undefined;
  } {
    if (this.#nativeSession !== undefined) return this.#nativeSession;
    const registry = this.#modelPath === undefined ? undefined : new CosineRuleRegistry({ modelPath: this.#modelPath });
    if (registry === undefined) {
      return { registry, turn: 0, ruleSync: undefined };
    }
    const directory = RULE_DIRECTORIES[session.host] ?? ".codex";
    const ruleSync = new LiveCosineRules(this.#ruleCatalog, registry, cosineRuleLoader({}, session.cwd, join(directory, "conversation-rules.json")));
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
