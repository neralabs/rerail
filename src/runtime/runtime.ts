import { randomUUID } from "node:crypto";
import { constants, type Stats } from "node:fs";
import { lstat } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { z } from "zod/v4";
import {
  ADAPTER_CAPABILITIES,
  type AdapterDelivery,
  type AdapterEvent,
  type AdapterSample,
  type AdapterSession,
  type HeldSession,
  type RerailEventLoop,
  type RerailHost,
  type SessionNoteRules,
  type SteerStrategy,
} from "../engine/adapter-contract.js";
import { RerailHostRuntime, type HostRuntimePort, type HostRuntimeSessionState } from "./host-runtime.js";
import { autostartDaemon } from "./daemon-control.js";
import { InactiveSessionError } from "./event-protocol.js";
import {
  changedFiles, fileDigest, LedgerDeliverySchema, owedFinding, owedForDisk, OwedEntrySchema, restoredFinding,
  type FileDigests, type OwedEntry, type OwedSteer,
} from "./owed-ledger.js";
import {
  CodeRuleBridge, generatedEditPaths, ledgerPreimage, UNSEEN_START, type HeldCall, type Preimage, type RewoundEdit,
  type SessionWalkResume, checkAtResult, type SourceObservationCache,
} from "../engine/code-rule/code-rule-bridge.js";
import type { IgnoreRulesChange } from "../engine/code-rule/ignore-rule-files.js";
import { claudeCodeReportedPreimage } from "../engine/code-rule/tool-result/reported-preimage.js";
import { RepositoryWorktreeWalks } from "../engine/code-rule/worktree-walks.js";
import { GitIgnoreMatchers } from "../engine/native/git-ignore-matchers.js";
import {
  buildSteer, type AuditorCloseOptions, type CodeRuleAuditor, type AuditSettings, type CodeRuleViolation, type RuleSteerDelivery,
  type RepositoryAuditHooks, type SteerMetadata, type SteerSourceOf,
} from "../engine/code-rule/code-rule.js";
import type { AuditSettings as CheckoutAuditSettings } from "../engine/audit/audit-control-schema.js";
import { conversationRulesEnabled, firstRepositoryAuditEnabled, missedSteerDigestEnabled, requireExperiment, steerStrategiesEnabled } from "../engine/experimental-features.js";
import type { AuditRecorder, AuditRecord } from "../engine/audit/audit.js";
import { SqliteAuditStore } from "../engine/audit/audit-store.js";
import { SqliteAuditControlStore, type PolicyOwnerClaim } from "../engine/audit/audit-control.js";
import { DEFAULT_AUDIT_SETTINGS } from "../engine/audit/audit-control-schema.js";
import { CheckoutCatalogs, configurationSignatureOf, startingCheckout } from "../engine/code-rule/code-rule-dsl-catalog.js";
import { CheckoutOwners } from "../engine/catalog/checkout-owner.js";
import { checkoutRealpath } from "../engine/catalog/checkout-control.js";
import { CheckoutCodeRuleRouter } from "../engine/code-rule/checkout-code-rule-router.js";
import { defaultRerailDatabasePath, failWhileWriteLocked } from "../engine/storage/database.js";
import { makeLedgerFolder, openLedgerFile, readLedgerFile, readLedgerLines, writeLedgerFile } from "../engine/storage/ledger-paths.js";
import { keepFolderPrivate } from "../engine/storage/private-files.js";
import { callNamesSecretPath, WITHHELD_OUTPUT, type Withheld } from "../engine/transcript-boundary.js";
import { scrubSecrets, scrubValue } from "../engine/secret-scrub.js";
import { mergeAssistEnabled, recordCheckout } from "../engine/catalog/repository-registry.js";
import { placeCheckout, placeWorktrees } from "../engine/catalog/checkouts/checkout-placement.js";
import type { WorktreeRegistry } from "../engine/catalog/repository-identity.js";
import { GitReads } from "../engine/git.js";
import type { CheckoutContext } from "../engine/catalog/repository-model.js";
import type { PendingNote } from "./merge-assist/notes.js";
import { createMergeAssistService, type MergeAssistRuntime } from "./merge-assist/service.js";
import { RepositoryStartWatch } from "./repository-start.js";
import { cosineSimilarity, NativeSemanticEmbedder, SemanticWorkspace } from "../engine/native/semantic-primitives.js";
import { ConversationRuleRegistry, type RuleMatch } from "../engine/catalog/registry.js";
import { LiveConversationRules } from "../engine/catalog/conversation-rule-sync.js";
import { parseSampleSource, SAMPLE_BOUNDARIES } from "../engine/catalog/rule-schema.js";
import { restorableRule, type DroppedEvents, type LostEvent, type RuleMutation } from "./session-state.js";
import { SessionLanes } from "./session-lanes.js";
import { rewoundSteer, steerBlock, steerRules, steerShell, subagentSteer, type SteerShell } from "../engine/code-rule/rerail-steer.js";
import { steerOrigin, steerTitle } from "../engine/code-rule/steer-origin.js";
import { checkoutPathBoundary } from "../engine/path-boundary.js";
import { SqliteSessionStore, type RuleCoverage, type SessionAttribution } from "./session-store.js";
import { isDefaultStorageRoot, sessionDirectory } from "./session-paths.js";
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
 * Whether a session of `host` keeps its samples in memory, in its semantic workspace's source: only an agent with the
 * semantic tools (`select`, `project`, `relate`) reads them back. A hook host's session (Claude Code, Codex, Cursor)
 * keeps them in its ledger alone, since it records thousands and nothing queries them.
 */
function keepsSamples(host: RerailHost): boolean {
  return ADAPTER_CAPABILITIES[host].toolSurface === "native";
}

/**
 * Keeps a session's tool calls awaiting their result (`callsAwaitingResult`) current with `event`: a call waits for its
 * result, and the turn's end settles every call left, since a call the user refused gets none.
 */
function noteCallsAwaitingResult(calls: Set<string>, event: AdapterEvent): void {
  if (event.kind === "call") calls.add(event.toolCallId);
  else if (event.kind === "result") calls.delete(event.toolCallId);
  else if (event.kind === "turn-end") calls.clear();
}

/**
 * Which code-rule boundaries of a session whose rules are still opening wait for them, within
 * `REOPENED_RULES_WAIT_MS` (`whileRulesOpen`). `none`: a first start, or a host that does not wait. `needed`: a
 * reopened session in a hook host, whose boundaries that need the rules wait (`needsRules`). `turn-end`: such a session
 * after one of them waited the bound out: only its turn ends still wait, since a turn's end is the last boundary whose
 * answer reaches the agent in that turn, and holding every tool call for rules that take longer would cost each one the
 * bound.
 */
type RulesWait = "none" | "needed" | "turn-end";

/** How a session `recorded` before, opening again in `host`, waits for its rules at its boundaries (`RulesWait`). */
function rulesWaitOnReopen(recorded: boolean, host: RerailHost): RulesWait {
  return recorded && ADAPTER_CAPABILITIES[host].toolSurface !== "native" ? "needed" : "none";
}

/**
 * How long a one-shot hook (Claude Code, Codex, Cursor) waits after a tool for that edit's rule check, so a steer lands
 * under the edit and the coding agent can act on it in the same turn. A check usually takes tens of milliseconds; one
 * that runs longer goes out at the next hook (with the experimental missed-steer digest, in the digest the turn's end
 * carries instead), and the hook never waits past this.
 */
const HOOK_EDIT_CHECK_MS = 1_000;
/**
 * How long an OMP or Pi tool result or turn end waits for the audits of the session's generated edits. An edit's audit
 * takes milliseconds, so its steer still reaches the agent before the next model call; a shell command that wrote
 * thousands of sources keeps its audits running for minutes, and the agent must not stall for them: past the cap the
 * audits carry on and their steers arrive with a later reply or poll (with the experimental missed-steer digest, held
 * for the digest at the end of the agent's run, `yieldDeliveries`). Well inside the host's 25 s hook budget, with room
 * for the source walk (`BOUNDARY_DISCOVERY_WAIT_MS`) that precedes it.
 */
const NATIVE_EDIT_CHECK_MS = 10_000;
/**
 * How long a code-rule boundary of a reopened session waits for its rules to open. A session opens again on its next
 * event after an idle close (`IDLE_SESSION_MS`), a daemon restart or a resume, and its rules may compile again (about
 * 12 s of CPU for a TypeScript repository's five packs on a Windows laptop). Its first edit went unchecked at its own
 * result while they did, and its steer arrived a turn later. The prompt that reopens the session starts the opening, so
 * the first tool call usually finds it nearly done. Inside the 30 s the hook gives the daemon to answer
 * (`ingestEngineEvent`). Only a boundary whose check needs the rules waits (`needsRules`), and once a tool call or
 * result has waited this out only turn ends wait again (`RulesWait`): rules that take longer would hold every tool call
 * for it.
 */
const REOPENED_RULES_WAIT_MS = 20_000;

/**
 * Whether a code-rule boundary of a reopened session needs its rules, so a steer from it rides its own answer: a call
 * the evaluator checks at its result (`checkAtResult`) and that call's result do, and so does a turn's end, whose
 * walk checks what the turn's shell and other tools wrote. A call that writes nothing the evaluator checks, and its
 * result, go on at once, as every boundary does while a session's first index runs.
 */
function needsRules(event: AdapterEvent, pending: readonly PendingCodeRuleCall[]): boolean {
  if (event.kind === "call") return checkAtResult(event.toolName, generatedEditPaths(event.toolName, event.input)) !== undefined;
  if (event.kind === "result") return pending.some(call => call.id === event.toolCallId && call.checked);
  return event.kind === "turn-end";
}

/**
 * What the code-rule boundary `event` of a session whose rules are still opening does (`RulesWait`): `wait` for them
 * within `REOPENED_RULES_WAIT_MS`; go on without them, counted in the session's history among the boundaries they were
 * not yet running at (`skip`); or go on with nothing to count (`pass`): a reopened session's call that writes nothing
 * the evaluator checks at its result, and that call's result, which the rules would not have checked. A first start
 * skips every boundary, since its first index is never waited for.
 */
function whileRulesOpen(wait: RulesWait, event: AdapterEvent, pending: readonly PendingCodeRuleCall[]): "wait" | "skip" | "pass" {
  if (wait === "none") return "skip";
  if (!needsRules(event, pending)) return "pass";
  return wait === "needed" || event.kind === "turn-end" ? "wait" : "skip";
}

/**
 * How many closed sessions a runtime keeps for late receipts. A late receipt comes from a send still in flight as its
 * session closed, so the last few closes are plenty; a checkout's runtime lives while any session there does, and would
 * otherwise keep every session that ever closed in it.
 */
const CLOSED_SESSIONS_KEPT = 16;

/**
 * Waits for `work` up to `ms`, and says whether it finished; past the cap the work carries on in the background. The
 * wait only decides when the hook answers, so a failure of the work is not raised here: an edit check records its own
 * failures in its audit, and an evaluator that cannot open in the session's history.
 */
async function settledWithin(work: Promise<unknown>, ms: number): Promise<boolean> {
  const capped = Promise.withResolvers<false>();
  const timer = setTimeout(() => capped.resolve(false), ms);
  try {
    return await Promise.race([work.then(() => true, () => true), capped.promise]);
  } finally {
    clearTimeout(timer);
  }
}

/** When the event `state` is ingesting happened, for its record: only for one that reaches the runtime later (`IngestOptions.observedAt`). */
function happenedAt(state: Pick<RuntimeSessionState, "observedAt">): { observedAt?: number } {
  return state.observedAt === undefined ? {} : { observedAt: state.observedAt };
}

/**
 * Waits for `work` until `signal` aborts (no one waits for the answer any more), and says whether it finished first;
 * without a signal, until it finishes. A failure of `work` before then is raised, as waiting on it raises it.
 */
async function settledBefore(work: Promise<void>, signal: AbortSignal | undefined): Promise<boolean> {
  if (signal === undefined) {
    await work;
    return true;
  }
  if (signal.aborted) return false;
  const gaveUp = Promise.withResolvers<false>();
  const giveUp = (): void => gaveUp.resolve(false);
  signal.addEventListener("abort", giveUp, { once: true });
  try {
    return await Promise.race([work.then(() => true as const), gaveUp.promise]);
  } finally {
    signal.removeEventListener("abort", giveUp);
  }
}

/**
 * The edit checks a boundary stopped waiting for, until they drain, by what it waited for: `all`, every edit (a turn
 * end, a yield), or `steering`, the edits that may steer (a tool result).
 */
type OverdueEditChecks = { all?: Promise<void>; steering?: Promise<void> };

/**
 * Whether a wait for the edit checks `only` names would sit behind checks an earlier boundary stopped waiting for. A
 * wait for every edit waits for those too. A `steering` wait does only behind an overdue `steering` wait: the evaluator
 * checks an edit that may steer before a walk's `log` edits queued ahead of it, or with them in one engine call, so a
 * `log` backlog an overdue wait for every edit stopped on holds it back no more than it held the earlier one. Where a
 * `steering` wait waits for those edits too, an overdue wait for every edit holds it back as well (`overdueKinds`).
 */
function waitsBehind(overdue: OverdueEditChecks, only: "steering" | undefined): boolean {
  return overdue.steering !== undefined || (only === undefined && overdue.all !== undefined);
}

/**
 * The kinds of wait an overdue wait for `only` holds back: its own, and a `steering` one behind a wait for every edit
 * where a `steering` wait waits for a walk's `log` edits too (`CodeRuleBridge.steeringWaitsForLog`).
 */
function overdueKinds(only: "steering" | undefined, steeringWaitsForLog: boolean): ReadonlyArray<keyof OverdueEditChecks> {
  if (only !== undefined) return [only];
  return steeringWaitsForLog ? ["all", "steering"] : ["all"];
}

/** Whether a boundary stopped waiting for edit checks that still drain. */
function anyOverdue(overdue: OverdueEditChecks): boolean {
  return overdue.all !== undefined || overdue.steering !== undefined;
}

/** The strategy a session steers with: the saved one only while the experimental switch is on, else `message`. */
function effectiveSteerStrategy(settings: Pick<CheckoutAuditSettings, "steerStrategy">): SteerStrategy {
  return steerStrategiesEnabled() ? settings.steerStrategy : "message";
}

/** The tool a sample came from (`tool:<name>`); undefined for the conversation's own text. */
function sampleTool(source: AdapterSample["source"]): string | undefined {
  return source.startsWith("tool:") ? source.slice("tool:".length) : undefined;
}

function sampleOwner(source: AdapterSample["source"]): "user" | "assistant" | "tool" {
  if (source === "user-input") return "user";
  if (source === "assistant-text") return "assistant";
  return "tool";
}

/** A ledger line that says it is an `owed` entry, whether or not this build can read the rest of it. */
const OwedTypeSchema = z.object({ type: z.literal("owed") });

/** A value of a ledger line as this build reads it: its entry, or undefined for a line it does not know. */
function ledgerEntryOf(value: unknown): LedgerEntry | undefined {
  const parsed = LedgerEntrySchema.safeParse(value);
  if (parsed.success) return compactValidated<LedgerEntry>(parsed.data);
  if (!OwedTypeSchema.safeParse(value).success) return undefined;
  // An `owed` line this build cannot read (a later build's) may say that what an earlier one listed has gone out: the
  // session owes nothing from either, rather than give it out again.
  process.stderr.write("rerail: a session's ledger holds an owed entry this version cannot read; the session owes nothing from it\n");
  return { type: "owed", steers: [], findings: [] };
}

/** A session's ledger as it was read back, and where its next line goes. */
export interface ReadLedger {
  readonly entries: readonly LedgerEntry[];
  /** How many lines held a value, entries or not. */
  readonly lines: number;
  readonly appendState: JsonlAppendState;
}

/**
 * Reads a session's ledger back a line at a time (`readLines`), each value taken to its entry as it is read
 * (`ledgerEntryOf`), keeping only what an open reads of it (`KeptLedger`). A session open for weeks has written a line
 * for every prompt, tool call and result, and a file's whole text for most edits.
 */
export async function readSessionLedger(path: string, keep: { readonly sampleText: boolean }): Promise<ReadLedger> {
  const kept = new KeptLedger(keep.sampleText);
  let lines = 0;
  const appendState = await readLines(path, value => {
    lines += 1;
    const entry = ledgerEntryOf(value);
    if (entry !== undefined) kept.add(entry);
  });
  return { entries: kept.entries, lines, appendState };
}

/**
 * A session's ledger entries as an open keeps them. A code-rule call's preimages, the text each file it edits held
 * before the edit, are let go once its result is read, or once a later call takes its id: only a call still open is
 * begun again with them (`unresolvedCodeRuleCalls`). Without `sampleText`, a sample keeps no text: a session whose
 * content source keeps none (`keepsSamples`) counts its samples and reads nothing of them.
 */
class KeptLedger {
  readonly entries: LedgerEntry[] = [];
  /** Where among `entries` each call whose result has not been read is, by its id. */
  readonly #openCalls = new Map<string, number>();
  readonly #sampleText: boolean;

  constructor(sampleText: boolean) {
    this.#sampleText = sampleText;
  }

  add(entry: LedgerEntry): void {
    if (entry.type === "policy-call" || entry.type === "policy-result") this.#close(entry.toolCallId);
    if (entry.type === "policy-call") this.#openCalls.set(entry.toolCallId, this.entries.length);
    this.entries.push(entry.type === "sample" && !this.#sampleText ? { ...entry, item: { ...entry.item, text: "" } } : entry);
  }

  /** Replaces the entry of the open call `toolCallId` with one without its preimages; nothing when none is open. */
  #close(toolCallId: string): void {
    const index = this.#openCalls.get(toolCallId);
    if (index === undefined) return;
    this.#openCalls.delete(toolCallId);
    const call = this.entries[index];
    if (call?.type !== "policy-call" || call.preimages === undefined) return;
    const { preimages: _preimages, ...closed } = call;
    this.entries[index] = closed;
  }
}

function processedEventIds(ledger: readonly LedgerEntry[]): Set<string> {
  return new Set(ledger.flatMap((entry) => (entry.type === "event" ? [entry.eventId] : [])));
}

/** When the ledger's first event that carries its row's time was recorded; undefined when none does. */
function firstDatedEvent(ledger: readonly LedgerEntry[]): number | undefined {
  for (const entry of ledger) if (entry.type === "event" && entry.observedAt !== undefined) return entry.observedAt;
  return undefined;
}

/** The event kinds the ledger's lines name (`persistEvent` writes each line with its row's kind). */
function ledgerKinds(ledger: readonly LedgerEntry[]): Set<string> {
  return new Set(ledger.flatMap((entry) => (entry.type === "event" && entry.kind !== undefined ? [entry.kind] : [])));
}

/**
 * The events the ledger holds whose rows a power cut or an operating system crash took, oldest first. An event's row is
 * committed before its ledger line is written, one event after another, or, while another process holds the write lock,
 * waits in the record writer behind the rows before it (`openRecordWriter`); such a loss, like a daemon killed with rows
 * still waiting, takes only the store's last rows: so the lost events are those after `recorded`
 * (`SqliteSessionStore.lastRecordedEvent`), the newest one whose row is there, or the last of a stretch a record of lost
 * events already counted. A ledger that does not hold `recorded` gives none, whatever lines an older build wrote in it:
 * `rerail reset` deleted the session's history, or the loss took every row of it, which nothing tells apart. Between
 * `recorded` and the ledger's end, a line an older build wrote (no kind or time) ends the walk, since those builds
 * synced every commit. The walk reads only the ledger in memory.
 */
function lostEvents(ledger: readonly LedgerEntry[], recorded: string | undefined): LostEvent[] {
  const from = ledger.findLastIndex(entry => entry.type === "event" && entry.eventId === recorded);
  if (from < 0) return [];
  const lost: LostEvent[] = [];
  for (let index = ledger.length - 1; index > from; index -= 1) {
    const entry = ledger[index]!;
    if (entry.type !== "event") continue;
    if (entry.kind === undefined || entry.observedAt === undefined) break;
    lost.push({ eventId: entry.eventId, kind: entry.kind, observedAt: entry.observedAt });
  }
  return lost.reverse();
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
/** The window `#noteTurnWindow` opens for a turn; no host names a tool call so. */
const TURN_WINDOW_ID = "rerail:turn";

/** A turn end that waited longer than this on its settle work is written to the daemon log. */
const SLOW_TURN_END_WAIT_MS = 1_000;

/**
 * The most an edit's result waits for Merge Assist by default, where the person's hook command may hold its notes (one
 * is set): the detection the edit ran and the command's note together, from the moment the detection starts, so
 * Rerail's note and the person's line ride that result when they are ready in time. Without a hook command it never
 * waits.
 */
const MERGE_ASSIST_WAIT_MS = 1_000;
/** A code-rule call waiting for its result. */
interface PendingCodeRuleCall {
  id: string;
  toolName: string;
  /**
   * Whether its result is checked (`checkAtResult`), and so needs a reopened session's rules (`needsRules`): such a
   * result waits for them when its call was held in the ledger across a restart, and is counted among the boundaries
   * they were not yet running at once a tool call waited them out; any other result does neither.
   */
  checked: boolean;
  /** The subagent that made it (Claude Code's `agent_id`), absent for the session's own agent. */
  agentId?: string | undefined;
  /** Still waiting after a yield of the session's agent, which a subagent's call may outlive. */
  outlivedYield?: boolean;
}

/**
 * Takes from `pending` the calls whose result will not come, which `#settleUnresolvedCalls` finishes. When the agent
 * yields: its own calls, and a subagent's already pending at its previous yield. Claude Code runs a subagent in the
 * background and sends its hooks under the parent's session, so a subagent's call in flight at the parent's stop gets
 * its result later and is checked then; one still waiting at the next stop was refused, or its result lost. At the
 * session's end (`sessionEnds`), every call: no result comes after it, for any agent, and a session can end with no
 * stop (Claude Code interrupted at a permission prompt), which left the agent's own call to be begun again at a reopen.
 * Each call kept is marked as having outlived this yield.
 */
function takeUnanswered(pending: PendingCodeRuleCall[], sessionEnds: boolean): PendingCodeRuleCall[] {
  const due = sessionEnds ? [...pending] : pending.filter(call => call.agentId === undefined || call.outlivedYield === true);
  const kept = pending.filter(call => !due.includes(call));
  pending.splice(0, pending.length, ...kept);
  for (const call of kept) call.outlivedYield = true;
  return due;
}

/** Code-rule calls recorded without a matching result, in ledger order. */
function unresolvedCodeRuleCalls(ledger: readonly LedgerEntry[]): Extract<LedgerEntry, { type: "policy-call" }>[] {
  const pending = new Map<string, Extract<LedgerEntry, { type: "policy-call" }>>();
  for (const entry of ledger) {
    if (entry.type === "policy-call") pending.set(entry.toolCallId, entry);
    else if (entry.type === "policy-result") pending.delete(entry.toolCallId);
  }
  return [...pending.values()];
}

/**
 * The session's secret calls still awaiting their result, and the results already stored without their output, as the
 * ledger records them: a result that arrives after the runtime state was rebuilt is judged by its call all the same.
 */
function secretCallsOf(ledger: readonly LedgerEntry[]): { calls: Set<string>; results: Set<string> } {
  const calls = new Set<string>();
  const results = new Set<string>();
  for (const entry of ledger) {
    if (entry.type === "secret-call") calls.add(entry.toolCallId);
    else if (entry.type === "secret-result") {
      calls.delete(entry.toolCallId);
      results.add(entry.eventId);
    }
  }
  return { calls, results };
}

/**
 * What the ledger says the session owed its agent when it last closed without its end, as the session takes it back
 * when it opens: at revision 0, so the first revision this opening makes of a source drops what spoke for it, and with
 * what each file it names held then, so a file changed while the session was closed drops it too (`#stillOwed`).
 */
function restoredOwed(ledger: readonly LedgerEntry[]): OwedState {
  const owed = ledger.findLast((entry): entry is OwedEntry => entry.type === "owed");
  if (owed === undefined) return { steers: [], findings: [], recorded: 0 };
  return {
    steers: owed.steers.map(steer => ({
      delivery: steer.delivery, revision: 0, scope: [...steer.scope], digests: { ...steer.digests }, ...(steer.recorded === true ? { recorded: true as const } : {}),
    })),
    findings: owed.findings.map(finding => ({ ...restoredFinding(finding), revision: 0 })),
    recorded: owed.steers.length + owed.findings.length,
  };
}

/**
 * How many of the steers and held findings the session's ledger gave back it can still give its agent while it stays
 * open. Its findings join its evaluator's, so none once the evaluator could not open: they stay in its ledger for its
 * next open, whose evaluator may.
 */
function owedNow(state: RuntimeSessionState): number {
  return state.owed.steers.length + (state.codeRuleOffReason === undefined ? state.owed.findings.length : 0);
}

/**
 * The steers of `steers` the session owes its agent itself: not a Merge Assist note (`mergeAssistNote`), which records
 * itself as given once a delivery takes it and which Merge Assist hands the session's next open again until one does.
 */
function ownSteers(steers: readonly PendingSteer[]): PendingSteer[] {
  return steers.filter(steer => steer.mergeAssistNote !== true);
}

/** Whether `item` is the steer of an edit Rerail rolled back: the agent's only word that its edit is gone. */
function rollsBack(item: PendingSteer | MissedFinding): boolean {
  return "delivery" in item && item.delivery.rewound !== undefined;
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
  // `kind` and `observedAt` are the event's row's own, which a ledger written before they were added lacks.
  z.object({ type: z.literal("event"), eventId: z.string(), kind: z.string().optional(), observedAt: z.number().int().optional() }),
  z.object({ type: z.literal("hud-tracking-start") }),
  z.object({ type: z.literal("sample"), item: ContentItemSchema }),
  z.object({ type: z.literal("turn"), eventId: z.string() }),
  z.object({ type: z.literal("delivery"), delivery: LedgerDeliverySchema }),
  z.object({ type: z.literal("rule-mutation"), mutation: z.unknown() }),
  z.object({ type: z.literal("rule-match"), ruleId: z.string(), turn: z.number().int().nonnegative() }),
  z.object({
    type: z.literal("policy-call"), toolCallId: z.string(), toolName: z.string(), paths: z.array(z.string()),
    preimages: z.record(z.string(), z.object({
      text: z.string().optional(),
      readError: z.string().optional(),
      withheld: z.object({ sha256: z.string(), bytes: z.number().int().nonnegative() }).optional(),
    })).optional(),
    // Absent for the session's own agent, and in a ledger written before calls recorded their subagent.
    agentId: z.string().optional(),
    // The folder the call ran in, where its host said; absent too in a ledger written before calls recorded it.
    cwd: z.string().optional(),
  }),
  z.object({ type: z.literal("policy-result"), toolCallId: z.string() }),
  z.object({ type: z.literal("secret-call"), toolCallId: z.string() }),
  z.object({ type: z.literal("secret-result"), toolCallId: z.string(), eventId: z.string() }),
  OwedEntrySchema,
]);

type LedgerEntry =
  | { type: "event"; eventId: string; kind?: string; observedAt?: number }
  | { type: "hud-tracking-start" }
  | { type: "sample"; item: ContentItem }
  | { type: "turn"; eventId: string }
  | { type: "delivery"; delivery: AdapterDelivery }
  | { type: "rule-mutation"; mutation: RuleMutation }
  | { type: "rule-match"; ruleId: string; turn: number }
  | { type: "policy-call"; toolCallId: string; toolName: string; paths: string[]; preimages?: Readonly<Record<string, Preimage>>; agentId?: string; cwd?: string }
  | { type: "policy-result"; toolCallId: string }
  | { type: "secret-call"; toolCallId: string }
  | { type: "secret-result"; toolCallId: string; eventId: string }
  | OwedEntry;

export function compactValidated<T>(value: unknown): T {
  // Zod has already validated this value; the round-trip removes explicit undefined keys for exact optional types.
  return JSON.parse(JSON.stringify(value)) as T;
}

/**
 * A ledger entry as it is written to disk: a sample's text and a delivery's content are scrubbed of secrets
 * (`scrubSecrets`), and a code-rule call's preimages take their ledger form (`ledgerPreimage`). The session keeps the
 * original in memory, so matching and steers see it. Other entries are written as they are; `entry` itself when
 * nothing changed.
 */
function ledgerEntryForDisk(entry: LedgerEntry): LedgerEntry {
  if (entry.type === "sample") {
    const text = scrubSecrets(entry.item.text);
    return text === entry.item.text ? entry : { ...entry, item: { ...entry.item, text } };
  }
  if (entry.type === "delivery") {
    const content = scrubSecrets(entry.delivery.content);
    return content === entry.delivery.content ? entry : { ...entry, delivery: { ...entry.delivery, content } };
  }
  if (entry.type === "policy-call" && entry.preimages !== undefined) {
    const { preimages } = entry;
    const stored = Object.entries(preimages).map(([path, preimage]) => [path, ledgerPreimage(preimage)] as const);
    return stored.every(([path, preimage]) => preimage === preimages[path]) ? entry : { ...entry, preimages: Object.fromEntries(stored) };
  }
  return entry;
}

/** A ledger entry as it is written to disk: `ledgerEntryForDisk`, or what a closed session owes in `owedForDisk`'s form. */
function storedLedgerEntry(entry: LedgerEntry): LedgerEntry {
  return entry.type === "owed" ? owedForDisk(entry) : ledgerEntryForDisk(entry);
}

/**
 * One line of an existing session ledger as `rerail scrub` rewrites it: an entry this build reads, in its disk form
 * (`storedLedgerEntry`); undefined when the line stays as it is (unchanged, or not an entry this build knows).
 */
export function scrubbedLedgerLine(line: string): string | undefined {
  let value: unknown;
  try {
    value = JSON.parse(line);
  } catch {
    return undefined;
  }
  const parsed = LedgerEntrySchema.safeParse(value);
  if (!parsed.success) return undefined;
  const entry = compactValidated<LedgerEntry>(parsed.data);
  const stored = storedLedgerEntry(entry);
  return stored === entry ? undefined : JSON.stringify(stored);
}

/**
 * A session's samples, as its semantic workspace reads them. One that does not keep them (a hook host's session,
 * `keepsSamples`) holds none: the session's ledger records every sample.
 */
class MutableContentSource implements ContentSource {
  readonly id = "host:event";
  readonly #items = new Map<string, ContentItem>();
  readonly #keeps: boolean;
  #revision = 0;

  constructor(items: readonly ContentItem[], keeps: boolean) {
    this.#keeps = keeps;
    if (!keeps) return;
    for (const item of items) this.#items.set(item.ref.id, item);
    this.#revision = items.length;
  }

  append(item: ContentItem): void {
    if (!this.#keeps || this.#items.has(item.ref.id)) return;
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

/** A line of `watermarks.jsonl`: every assignment as of one revision. */
const WatermarkSnapshotSchema = z.object({
  revision: z.number().int().nonnegative(),
  assignments: z.array(z.object({ id: z.string(), target: ContentRefSchema, watermark: WatermarkSchema })),
});

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
    // Each line holds every assignment as of its revision, so only the last one that parses is kept.
    const read: { latest?: { revision: number; assignments: WatermarkAssignment[] } } = {};
    const appendState = await readLines(path, value => {
      const parsed = WatermarkSnapshotSchema.safeParse(value);
      if (parsed.success) read.latest = compactValidated<{ revision: number; assignments: WatermarkAssignment[] }>(parsed.data);
    });
    const store = new JsonlWatermarkStore(path, appendState);
    const { latest } = read;
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
  /** Called once the steer leaves the queue, with whether a delivery took it (a Merge Assist note records itself then). */
  settled?: (delivered: boolean) => void;
  /** A Merge Assist note: the conflict store keeps it until a delivery takes it, so closing the session loses nothing. */
  mergeAssistNote?: true;
  /** Its delivery is in the ledger already: an answer no client heard took it (`returnUndelivered`). */
  recorded?: true;
}

/**
 * A tool call's result being ingested: while it is open, a steer from the call's own edit check rides that result
 * (`toolCallId`), and a `rewind` one rolls the edit back. `steers` settle once each steer that began a rollback in it
 * is queued, so the result answers with that steer.
 */
interface ResultWindow {
  readonly toolCallId: string;
  readonly steers: Promise<void>[];
}

/**
 * One finding of a generated-edit steer that arrived while no boundary of its session was waiting for it, held for the
 * session's yield digest. `scope` is the finding's own source when the steer spoke for it, so a later edit of one file
 * drops only that file's findings; a finding elsewhere is stale once any source of its steer is.
 */
interface MissedFinding {
  violation: CodeRuleViolation;
  sourceOf: SteerSourceOf;
  revision: number;
  scope: string[];
}

/**
 * A steer or held finding the session owes while it is closed, with what each file it names held as the session last
 * saw it (`#withDigests`).
 */
type Kept<T extends PendingSteer | MissedFinding> = T & { digests: FileDigests };

/**
 * The steer an answer's delivery came from, and its session: one of the session's pending steers, or one the session
 * owed as it opened (`owed`), which goes back there if no client heard the answer (`returnUndelivered`).
 */
type HandedOut =
  | { state: RuntimeSessionState; pending: PendingSteer; owed?: undefined }
  | { state: RuntimeSessionState; pending: Kept<PendingSteer>; owed: true };

/** What a closing session owes its agent: what it took back from its ledger (`kept`), and what this opening found. */
interface Owing {
  kept: { steers: Kept<PendingSteer>[]; findings: Kept<MissedFinding>[] };
  found: { steers: PendingSteer[]; findings: MissedFinding[] };
}

/**
 * What the session owed its agent when it last closed without its end, taken back from its ledger (`restoredOwed`)
 * until it goes out. Its steers go out with the session's next answer without waiting for its evaluator, which did not
 * find them; its findings join the evaluator's once it is open, for the next yield's digest.
 */
interface OwedState {
  steers: Kept<PendingSteer>[];
  findings: Kept<MissedFinding>[];
  /** How many steers and findings the ledger's last `owed` entry lists. */
  recorded: number;
}

/** A tool call the ledger held open as the session opened: its call was recorded, its result not yet. */
type RestoredCall = Extract<LedgerEntry, { type: "policy-call" }>;

/** The checkout a session started in (physical root), the session folder relative to it, and that folder resolved once. */
interface StartingCheckout {
  root: string;
  folder: string;
  /** The session's folder, resolved once at open; every later use reads this, never the folder name again. */
  cwd: string;
}

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
  /** How the session's code-rule steers reach the agent; read with its audit settings. */
  steerStrategy: SteerStrategy;
  /** Open while a tool call's result is ingested; unset otherwise. */
  resultWindow: ResultWindow | undefined;
  /** While an event that happened earlier is ingested, when it happened (`IngestOptions.observedAt`); unset otherwise. */
  observedAt: number | undefined;
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
  /**
   * Which of its code-rule boundaries wait for its rules while they open (`RulesWait`): `needed` once it opened again
   * (after an idle close, a daemon restart or a resume) in a hook host, `turn-end` once a tool call or result waited
   * `REOPENED_RULES_WAIT_MS` out, so the opening holds no later tool call.
   */
  rulesWait: RulesWait;
  ownerClaim: PolicyOwnerClaim | undefined;
  /**
   * Set while this runtime's claim on the session, written again when a write put the session's row back, has not
   * landed (`#claimAgain`): the session's next event writes it. Kept here, so an event of a session whose claim is in
   * place reads nothing to find out.
   */
  claimOwed?: true;
  /** The database's `data_version` this session's settings and audit queue were last read at; undefined to read on the next tick. */
  controlsVersion: number | undefined;
  auditRequestWork: Promise<void> | undefined;
  /** The marks of the Merge Assist notes queued for a delivery that has not taken them yet; none is queued twice. */
  mergeAssistNotesInFlight: Set<string>;
  /** The session's tool calls that edit files, until their results: each result finds the checkout's conflicts again. */
  mergeAssistEdits: Set<string>;
  repositoryAudit: Promise<void> | undefined;
  /** Set while `repositoryAudit` runs: the idle close lets it finish (`#owesScan`). */
  repositoryAuditRunning?: true;
  pendingCodeRuleCalls: PendingCodeRuleCall[];
  /**
   * The tool calls of the session's current turn still waiting for their result, which may come long after the call
   * (a permission prompt, a long command). The turn's end settles them all: a call the user refused gets no result.
   */
  callsAwaitingResult: Set<string>;
  /** The edit checks a boundary stopped waiting for, until they drain, by what it waited for (`OverdueEditChecks`). */
  overdueEditChecks: OverdueEditChecks;
  /**
   * The boundaries of this session waiting for its edit checks right now: a generated-edit steer that arrives while
   * one waits rides that boundary's answer, and one that arrives while none does is held for the yield digest.
   */
  editCheckWaits: number;
  /**
   * How many of those waits are the agent's yield. While checks an earlier boundary gave up on are still draining, a
   * steer belongs to a step the agent has left, so only a yield carries it as it comes; any other boundary holds it.
   */
  yieldWaits: number;
  /** Generated-edit findings no boundary waited for, in the order they arrived, until the session's next yield. */
  missedFindings: MissedFinding[];
  owed: OwedState;
  /**
   * Whether closing the session keeps what it owes its agent in its ledger: set for a close without its end, as when it
   * has gone quiet or its runtime closes. Its end drops what it owes, as it always has.
   */
  keepsOwed: boolean;
  /**
   * The calls the ledger held open as the session opened, by id. One whose result arrives while the evaluator opens is
   * checked against what the call captured once the evaluator is open (`#checkWhenOpen`).
   */
  restoredCalls: Map<string, RestoredCall>;
  /**
   * The session's tool calls whose input names a secret-like path (`callNamesSecretPath`), until their result is stored.
   * The ledger records both ends (`secret-call`, `secret-result`), so a session reopened between them still knows.
   */
  secretCalls: Set<string>;
  /** The results stored without their output (`withheldOutput`): their samples are recorded without text too. */
  withheldResults: Set<string>;
  closed: boolean;
  /** Set as `closeIdleSession` lets the session go: its close keeps it open in the database, owned (`#markClosed`). */
  suspending?: true;
  /** Set as `closeIdleSession` ends the session (`end`): when its record ends, by the caller's clock (`#markClosed`). */
  endsAt?: number;
}

/**
 * What a session the engine service let go of while it was quiet (`closeIdleSession`) keeps until it opens again: the
 * owner claim it still holds in the database, so it stays live, takes scan requests and reopens without interrupting
 * them, and where its source walks stood, so its next walk records another writer's change made meanwhile.
 */
export interface SessionSuspension {
  readonly descriptor: AdapterSession;
  readonly ownerClaim: PolicyOwnerClaim | undefined;
  /** Undefined when the session's evaluator had not opened, or could not. */
  readonly walk: SessionWalkResume | undefined;
  /** The database that records the session, whose record `endSuspendedSessions` ends if it never opens again. */
  readonly databasePath: string;
  /**
   * When the idle close let the session go. A record no runtime opens again ends at this time when its daemon stops
   * (`endSuspendedSessions`), not at the stop, which can come days after the session went quiet.
   */
  readonly suspendedAt: number;
  /**
   * The checkout whose start state gates the session's runtime, as its start watch found it at the suspension: the
   * engine service ends a suspended session once Rerail is stopped there. Undefined where the runtime has no start gate,
   * or Rerail records no checkout for it.
   */
  readonly startGate: CheckoutContext | undefined;
}

/**
 * The suspended sessions a runtime keeps and opens again, by session id: an engine service keeps them across its
 * runtimes.
 */
export interface SuspendedSessions {
  /**
   * Keeps the suspension of a session `closeIdleSession` is letting go, before its close runs: a scan asked for it while
   * it closes finds it suspended, and a close that fails leaves it suspended, not open in the database with nobody
   * holding it.
   */
  keep(suspension: SessionSuspension): void;
  get(sessionId: string): SessionSuspension | undefined;
  /** Ends the session's suspension as a runtime claims it again, and returns it; undefined once another has taken it. */
  take(sessionId: string): SessionSuspension | undefined;
}

/** What `closeIdleSession` did with a quiet session. */
export type IdleClose =
  /** It still owes its agent something its ledger does not hold, and stays open. */
  | { readonly kind: "owing" }
  /** This runtime does not hold it open. */
  | { readonly kind: "not-held" }
  /** Its memory is freed; it stays open in the database until it opens again or its daemon stops. */
  | { readonly kind: "suspended"; readonly suspension: SessionSuspension }
  /** Its memory is freed and its record ended, as its end would (`end`); its next event opens it again. */
  | { readonly kind: "ended" };

/**
 * Whether the session of `state` records its rule coverage: published (unpublished sessions have no session row yet;
 * the session-start ingest records coverage once they do) and not closed.
 */
function recordsCoverage(state: RuntimeSessionState | undefined): state is RuntimeSessionState {
  return state !== undefined && !state.closed && state.ownerClaim !== undefined;
}

/** The bridge option that resumes the session's walks where `suspension` left them; none for a fresh open. */
function walkResumeOption(suspension: SessionSuspension | undefined): { resume?: SessionWalkResume } {
  return suspension?.walk === undefined ? {} : { resume: suspension.walk };
}

/**
 * The calls the session's ledger holds open as it opens, with what each captured: the evaluator's first walk leaves
 * their files to each call's own check (`CodeRuleBridge`'s `heldCalls`), begun once the evaluator is open, or at the
 * call's result, or by `#checkWhenOpen` for a result that came while it opened.
 */
function heldCallsOf(restored: ReadonlyMap<string, RestoredCall>): HeldCall[] {
  return [...restored.values()].map(call => ({ toolCallId: call.toolCallId, paths: call.paths, preimages: call.preimages ?? {} }));
}

/**
 * Ends suspended sessions no runtime opened again, as `closeSession` ends an open one: their queued scans are
 * interrupted, their owner released and their record ended at `endedAt`, the time their end came. Without one, as when
 * the daemon stops, each record ends when its session was suspended (`suspendedAt`): nothing was heard from it after. One
 * that fails is reported and the rest are still ended, as a runtime's close ends each of its sessions whatever another's
 * does.
 */
export function endSuspendedSessions(databasePath: string, suspensions: readonly SessionSuspension[], endedAt?: number): void {
  const sessions = new SqliteSessionStore(databasePath);
  const controls = new SqliteAuditControlStore(databasePath);
  try {
    for (const { descriptor, ownerClaim, suspendedAt } of suspensions) {
      try {
        if (ownerClaim !== undefined) {
          controls.interrupt(descriptor.sessionId);
          controls.releaseOwner(descriptor.sessionId, ownerClaim);
        }
        sessions.closeSession(descriptor.sessionId, endedAt ?? suspendedAt);
      } catch (error) {
        process.stderr.write(`rerail: ending quiet session ${descriptor.sessionId} failed: ${error instanceof Error ? error.message : String(error)}\n`);
      }
    }
  } finally {
    controls.close();
    sessions.close();
  }
}

/**
 * Fails the scans queued for a suspended session that could not be opened again to run them, saying why, as an open
 * session fails them when its evaluator cannot open. Its record and owner stay as they are, for its next event.
 */
export function failQueuedScans(suspension: SessionSuspension, reason: string): void {
  const { descriptor: { sessionId }, ownerClaim, databasePath } = suspension;
  if (ownerClaim === undefined) return;
  const controls = new SqliteAuditControlStore(databasePath);
  try {
    for (let request = controls.claim(sessionId, ownerClaim); request !== undefined; request = controls.claim(sessionId, ownerClaim)) {
      controls.finish(request.id, "failed", [], `The session could not be opened again to run the scan: ${reason}`);
    }
  } finally {
    controls.close();
  }
}

/** A session's ledger as read back when the session opens. */
interface OpenedLedger {
  readonly directory: string;
  readonly path: string;
  readonly entries: readonly LedgerEntry[];
  readonly appendState: JsonlAppendState;
  /** Whether this opening is the session's first observed Rerail participation. */
  readonly completeTracking: boolean;
}

/** The conversation rules and turn a session opens with. */
interface RestoredRegistry {
  registry: ConversationRuleRegistry | undefined;
  turn: number;
  ruleSync?: LiveConversationRules | undefined;
}

/** A steer now among the pending steers. */
interface PlacedSteer {
  /** Its background delivery, when the native boundary that waits for the edit's check waits for it too. */
  readonly boundaryDelivery: Promise<unknown> | undefined;
}

/** What a session's initial state is built from. */
interface SessionStart {
  readonly session: AdapterSession;
  readonly checkout: StartingCheckout;
  readonly startedAt: number;
  readonly ledger: OpenedLedger;
  readonly restored: RestoredRegistry;
  readonly watermarks: WatermarkStore;
  readonly steerStrategy: SteerStrategy;
}

/**
 * Hands `take` the value of each line of a JSON-lines record file, read a line at a time (`readLedgerLines`), and says
 * where its next line goes. A line that is no JSON fails the read, except the last when no newline ends it: a write cut
 * short, which the next append cuts off (`truncateTo`, its first byte); a last line that is whole gets its newline first
 * (`prefix`).
 */
async function readLines(path: string, take: (value: unknown) => void): Promise<JsonlAppendState> {
  const last = await readLedgerLines(path, dirname(path), line => {
    if (line !== "") take(JSON.parse(line));
  });
  const appendState: JsonlAppendState = { prefix: "", truncateTo: null };
  if (last === undefined || last.text === "") return appendState;
  let value: unknown;
  try {
    value = JSON.parse(last.text);
  } catch {
    appendState.truncateTo = last.start;
    return appendState;
  }
  appendState.prefix = "\n";
  take(value);
  return appendState;
}

/** Appends one line through a descriptor that follows no symlink, in a folder checked as `makeLedgerFolder` checks it. */
async function appendJsonLine(path: string, value: unknown, state: JsonlAppendState): Promise<void> {
  await makeLedgerFolder(dirname(path));
  const handle = await openLedgerFile(path, constants.O_WRONLY | constants.O_APPEND | constants.O_CREAT);
  try {
    if (state.truncateTo !== null) await handle.truncate(state.truncateTo);
    await handle.appendFile(`${state.prefix}${JSON.stringify(value)}\n`, "utf8");
  } finally {
    await handle.close();
  }
  state.prefix = "";
  state.truncateTo = null;
}

/**
 * What the session store keeps of `event`: scrubbed of secrets (`scrubValue`), and a result whose call named a
 * secret-like path (`withheld`) without its output. The live event, which rules match, keeps them.
 */
function storedPayload(event: AdapterEvent, withheld: boolean): unknown {
  return event.kind === "result" && withheld ? { ...event, output: withheldOutput(event.output) } : scrubValue(event);
}

/**
 * What the session store keeps of a result whose call named a secret-like path: that its output was left out, and how
 * large it was. People read the transcript whole, but a `cat .env` output is a secret Rerail recognizes, so it is never
 * written down; the call itself is kept.
 */
function withheldOutput(output: unknown): Withheld & { bytes?: number } {
  let text: string | undefined;
  try {
    text = typeof output === "string" ? output : JSON.stringify(output);
  } catch {
    // An output JSON cannot hold (the session store encodes those another way) is left without a size.
  }
  return {
    ...WITHHELD_OUTPUT,
    reason: "the call names a secret-like path, so Rerail does not store what it returned",
    ...(text === undefined ? {} : { bytes: Buffer.byteLength(text) }),
  };
}

const SESSION_RECORDS_GITIGNORE = "# Created by Rerail: session records stay out of Git.\n*\n";

/**
 * Session records hold what the person typed, and the storage root defaults to `<checkout>/.rerail/sessions`. A
 * `.gitignore` of `*` keeps them out of `git status` and `git add -A`, as `.pytest_cache` does, without touching the
 * repository's own ignores. Only the default root is Rerail's alone: a custom one (`RERAIL_STORAGE_ROOT`) may hold the
 * person's own files, so it is left as it is. In Rerail's own folder the file is rewritten whenever it holds anything
 * else, or is no regular file: a repository that commits one with `!*` would otherwise have the person's next
 * `git add -A` commit their prompts and pre-edit source. It is written `0644` whatever the umask: it holds only
 * Rerail's `*`, and every account that records there, whose Git reads it, may then read it whoever wrote it. A rewrite
 * that fails fails the session, as one onto a `.gitignore` folder or onto a file another program holds open does,
 * except where the file is another account's to write. One this account may not read is rewritten when this account
 * owns it or owns the records folder, whose owner may rename over any entry in it, sticky or not; it is kept, not
 * replaced, only when another account owns both the file and the folder, as that account's Rerail wrote it. On
 * Windows, whose lstat names no owner, one this account may not read is rewritten too, and kept only when this account
 * may not add a file to the folder, as in a records folder another account repaired. There, with no file at the path
 * either, the account that may list the folder, the one whose Git could add what is in it, writes it. Each time this
 * account leaves the file to another one, stderr names it.
 */
async function ignoreInGit(storageRoot: string): Promise<void> {
  const path = join(storageRoot, ".gitignore");
  if (keptAsFound(await foundGitignore(path, storageRoot))) return;
  try {
    await writeLedgerFile(path, storageRoot, SESSION_RECORDS_GITIGNORE, 0o644);
  } catch (error) {
    if (!newFileRefused(error) || !leftToAnother(await foundGitignore(path, storageRoot))) throw error;
    process.stderr.write(`rerail: left ${path} to the account that may add files to ${storageRoot}: this account may not, and that account's Rerail writes it\n`);
  }
}

/** Whether a `.gitignore` found before any write is left as it is: Rerail's own, or another account's to write. */
function keptAsFound(found: FoundGitignore): boolean {
  return found.kind === "another account's" || (found.kind === "text" && found.text === SESSION_RECORDS_GITIGNORE);
}

/**
 * Whether what is at a `.gitignore` this account may not add a file beside is another account's to write: nothing, a
 * file another account keeps, or on Windows, where no owner is read, a file this account may not read.
 */
function leftToAnother({ kind }: FoundGitignore): boolean {
  return kind === "missing" || kind === "another account's" || (kind === "unreadable" && process.platform === "win32");
}

/** Whether a file operation failed because this account may not do it there (EACCES, or EPERM as Windows reports it). */
function accessRefused(error: unknown): boolean {
  const code = (error as NodeJS.ErrnoException).code;
  return code === "EACCES" || code === "EPERM";
}

/**
 * Whether `error`, from `writeLedgerFile`, is the refusal to make its new file beside the one it replaces: this account
 * may not add a file to the folder. That new file is the only one it opens; its rename fails with another call's error.
 */
function newFileRefused(error: unknown): boolean {
  return accessRefused(error) && (error as NodeJS.ErrnoException).syscall === "open";
}

/**
 * What is at a storage root's `.gitignore`: its text, nothing, something that is no regular file, a file this account
 * may not read that another account keeps ({@link whoseUnreadable}), or one it may not read that this account may
 * replace, not known. A file Windows refuses even to lstat, as one in a folder this account may not list, is one this
 * account may not read.
 */
type FoundGitignore =
  | { readonly kind: "text"; readonly text: string }
  | { readonly kind: "missing" | "not a file" | "another account's" | "unreadable" };

async function foundGitignore(path: string, storageRoot: string): Promise<FoundGitignore> {
  let stats: Stats;
  try {
    stats = await lstat(path);
  } catch (error) {
    return lstatRefusal(error);
  }
  if (!stats.isFile()) return { kind: "not a file" };
  let text: string | undefined;
  try {
    text = await readLedgerFile(path, storageRoot);
  } catch (error) {
    if (!accessRefused(error)) throw error;
    return { kind: await whoseUnreadable(stats, storageRoot) };
  }
  return text === undefined ? { kind: "missing" } : { kind: "text", text };
}

/** What a `.gitignore` whose lstat failed with `error` is: nothing, or one this account may not read; else it throws. */
function lstatRefusal(error: unknown): FoundGitignore {
  if ((error as NodeJS.ErrnoException).code === "ENOENT") return { kind: "missing" };
  if (accessRefused(error)) return { kind: "unreadable" };
  throw error;
}

/**
 * Whose to write a `.gitignore` this account may not read is, given its lstat `stats`: another account's when another
 * account owns both it and the records folder `storageRoot`; this account's to replace when it owns either, since a
 * folder's owner may rename over any entry in it, and on Windows, whose lstat names no owner (its uid is always 0).
 */
async function whoseUnreadable(stats: Stats, storageRoot: string): Promise<"another account's" | "unreadable"> {
  if (process.platform === "win32" || stats.uid === process.getuid?.()) return "unreadable";
  return (await lstat(storageRoot)).uid === process.getuid?.() ? "unreadable" : "another account's";
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

/** A Merge Assist note as the steer that carries it to `session`: a message naming no rule, with its conflict's line for the person. */
function mergeAssistDelivery({ sessionId, host }: AdapterSession, note: PendingNote): AdapterDelivery {
  return {
    sessionId, requested: "steer", mechanism: ADAPTER_CAPABILITIES[host].delivery.steer, content: note.content, ruleIds: [], rules: [], strategy: "message",
    ...(note.conflict === undefined ? {} : { conflict: note.conflict }),
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
  /** Defaults for checkouts without saved audit settings; saved settings win. */
  auditSettings?: Partial<CheckoutAuditSettings>;
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
    /**
     * Leave out what Git ignores: the session checkout's repository setting as the session opened, fixed for its life
     * (a flip mid-session would make the shell-write walk report every ignored file it starts or stops seeing as edited).
     */
    followGitignore: boolean;
    /** The shell the session's coding agent runs commands in, which its steers' commands are written for. */
    steerShell: SteerShell;
  } & AuditSettings) => CodeRuleAuditor;
  /** How long a turn end or command result waits for the session's shell-write walk; the bridge's default otherwise. */
  discoveryWaitMs?: number;
  /**
   * The walk of the other worktrees of each repository, shared by every session watching them: the engine service passes
   * one to all its runtimes; a runtime makes its own otherwise, which its sessions share.
   */
  worktreeWalks?: RepositoryWorktreeWalks;
  /**
   * Git's ignore rules of each checkout, one matcher for every session (`GitIgnoreMatchers`): the engine service passes
   * one to all its runtimes; a runtime makes its own otherwise, which its sessions share.
   */
  gitIgnoreMatchers?: GitIgnoreMatchers;
  /** Receives the timing lines of session start and slow turn ends (the daemon passes its log); without it they are not written. */
  timingLog?: (line: string) => void;
  /** The sessions an engine service suspended (`closeIdleSession`), which this runtime takes back as it opens one. */
  suspendedSessions?: SuspendedSessions;
  /**
   * The lock the engine service this runtime runs in holds for its life (`owner-lock.ts`), which the owner rows of its
   * sessions name: a later start tells from it whether their writer still runs, whatever process holds its pid then.
   */
  ownerLockId?: string | undefined;
  nativeSession?: {
    registry: ConversationRuleRegistry;
    ruleSync?: LiveConversationRules;
    turn: number;
    onMatch(match: RuleMatch, sample: AdapterSample, turn: number): void;
    onTurn?(turn: number): void;
  };
  /**
   * The Merge Assist the daemon's sessions and API share; without one, the runtime makes its own over its database
   * the first time a session runs in a repository where Merge Assist is on.
   */
  mergeAssist?: MergeAssistRuntime;
  /** The most an edit's result waits for Merge Assist where a hook command is set; {@link MERGE_ASSIST_WAIT_MS} by default. */
  mergeAssistWaitMs?: number;
}

/** How an event's answer is waited for (`RerailRuntime.ingest`). */
export interface IngestOptions {
  /**
   * Aborts once whoever waits for the answer stops waiting: the engine service's hook request closed before its answer
   * was written (the hook gave up at its 30 s, or its process ended). Such an event stops waiting for edit checks and
   * takes none of what the session holds for its agent, which waits for the session's next event. Without one, the
   * answer counts as read. One aborted when the event begins says its tool may have run already: a call then captures
   * no "before" of its files (`#beginCall`).
   */
  signal?: AbortSignal | undefined;
  /**
   * When the event happened (epoch ms), where it reaches the runtime later: a hook left it while the daemon did not read
   * (`untaken-events.ts`). Its record keeps that time. Without one, the event happens as it is ingested.
   */
  observedAt?: number | undefined;
}

export class RerailRuntime implements HostRuntimePort, RerailEventLoop {
  readonly #storageRoot: string;
  /** The root is the workspace's default (`isDefaultStorageRoot`), so its records get a `.gitignore`. */
  readonly #storageRootIsDefault: boolean;
  readonly #embedder: SemanticEmbedder;
  readonly #modelPath: string | undefined;
  readonly #sessionStore: SqliteSessionStore;
  readonly #audits: SqliteAuditStore;
  readonly #steersInFlight = new Set<Promise<AdapterDelivery | undefined>>();
  /** The steer each answer's delivery came from, and its session, until the answer reaches its asker or comes back (`returnUndelivered`). */
  readonly #handedOut = new WeakMap<AdapterDelivery, HandedOut>();
  /** Evaluator opens still running, each with its own release: disposal waits for them before the stores close under them. */
  readonly #evaluatorOpens = new Set<Promise<CodeRuleBridge | undefined>>();
  /** Every checkout this runtime's sessions touch, with its eligibility and compiled rules. */
  readonly #catalogs: CheckoutCatalogs;
  readonly #owners = new CheckoutOwners(() => this.#catalogs.recordedCheckoutsVersion());
  /**
   * The Git reads that place a session's checkout, then the other worktrees Git lists for it (`placeCheckout`,
   * `placeWorktrees`): a disposed runtime ends them, and returns once each Git they started has exited, so none is left
   * running in a checkout (Windows refuses to remove a folder a process holds). A read another runtime started and
   * shares with them goes on, and is not waited for (`sharedRead`).
   */
  readonly #placing = new GitReads();
  readonly #sessions = new Map<string, RuntimeSessionState>();
  /**
   * While an event that happened earlier opens its session (`IngestOptions.observedAt`), when it happened, by session id:
   * a session it opens starts then (`#asOf`).
   */
  readonly #startsAt = new Map<string, number>();
  /** The sessions `resumeSession` is opening again for a scan, whose open fails rather than wait for a write lock. */
  readonly #resumingForScan = new Set<string>();
  /**
   * Kept past disposal for late receipts, with the repository each session was attributed to; the latest few only. A
   * suspended session's `endedAt` is null: it is still open.
   */
  readonly #closedSessions = new Map<string, { descriptor: AdapterSession; startedAt: number; endedAt: number | null; repositoryKey: string | null }>();
  #sessionStoreClosed = false;
  readonly #routers = new Map<RerailHost, RerailHostRuntime>();
  readonly #nativeSession: RerailRuntimeOptions["nativeSession"];
  readonly #createAuditor: NonNullable<RerailRuntimeOptions["createAuditor"]>;
  readonly #deliverBackground: RerailRuntimeOptions["deliverBackground"];
  readonly #beforeSessionPublish: RerailRuntimeOptions["beforeSessionPublish"];
  readonly #auditSettings: CheckoutAuditSettings;
  readonly #controls: SqliteAuditControlStore;
  /** The control timer (`#pollControlsWhileSessionsOpen`): set only while some session is open here. */
  #controlTimer: NodeJS.Timeout | undefined;
  /** Set as `dispose` begins: the sessions it closes start no control timer again. */
  #disposing = false;
  readonly #autostart: boolean;
  readonly #ingestLanes = new SessionLanes();
  readonly #sourceObservations: SourceObservationCache = new Map();
  readonly #worktreeWalks: RepositoryWorktreeWalks;
  readonly #gitIgnoreMatchers: GitIgnoreMatchers;
  readonly #discoveryWaitMs: number | undefined;
  readonly #timingLog: ((line: string) => void) | undefined;
  readonly #startWatch: RepositoryStartWatch | undefined;
  readonly #suspendedSessions: SuspendedSessions | undefined;
  readonly #ownerLockId: string | undefined;
  #mergeAssist: MergeAssistRuntime | undefined;
  /** Whether this runtime made its Merge Assist (none was passed in), so it stops it when it is disposed. */
  readonly #ownsMergeAssist: boolean;
  readonly #mergeAssistWaitMs: number;
  /** Stops hearing Merge Assist's conflict changes; set once this runtime listens. */
  #mergeAssistUnlisten: (() => void) | undefined;

  constructor(options: RerailRuntimeOptions) {
    this.#storageRoot = options.storageRoot;
    this.#suspendedSessions = options.suspendedSessions;
    this.#ownerLockId = options.ownerLockId;
    this.#storageRootIsDefault = isDefaultStorageRoot(options.storageRoot, options.workspaceRoot);
    this.#modelPath = options.modelPath;
    this.#nativeSession = options.nativeSession;
    this.#deliverBackground = options.deliverBackground;
    this.#mergeAssist = options.mergeAssist;
    this.#ownsMergeAssist = options.mergeAssist === undefined;
    this.#mergeAssistWaitMs = options.mergeAssistWaitMs ?? MERGE_ASSIST_WAIT_MS;
    this.#beforeSessionPublish = options.beforeSessionPublish;
    this.#auditSettings = { ...DEFAULT_AUDIT_SETTINGS, ...options.auditSettings };
    this.#discoveryWaitMs = options.discoveryWaitMs;
    this.#worktreeWalks = options.worktreeWalks ?? new RepositoryWorktreeWalks();
    this.#gitIgnoreMatchers = options.gitIgnoreMatchers ?? new GitIgnoreMatchers();
    this.#timingLog = options.timingLog;
    this.#autostart = options.autostart ?? true;
    this.#createAuditor = options.createAuditor ?? ((auditorOptions) => new CheckoutCodeRuleRouter({
      ...auditorOptions,
      // `workspaceRoot` is the session folder `openSession` resolved once.
      sessionRoot: auditorOptions.workspaceRoot,
      startingCheckout: startingCheckout(this.#owners, auditorOptions.workspaceRoot).checkoutRoot,
      owners: this.#owners,
      catalogs: this.#catalogs,
      gitIgnoreMatchers: this.#gitIgnoreMatchers,
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
      sessionStore = new SqliteSessionStore(databasePath, { onRowPutBack: sessionId => this.#claimAgain(sessionId) });
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
  }

  /**
   * Runs the control timer (`#pollAuditControls`, every 250 ms) while some session is open in this runtime, and stops it
   * once none is: each tick reads the database's `data_version` to find a session whose settings or scans changed, and a
   * runtime with no session, which the engine service keeps open for its checkout's next session, has none to find.
   * Asked each time a session enters or leaves the runtime.
   */
  #pollControlsWhileSessionsOpen(): void {
    if (this.#sessions.size > 0 && !this.#disposing) {
      if (this.#controlTimer !== undefined) return;
      this.#controlTimer = setInterval(() => this.#pollAuditControls(), 250);
      this.#controlTimer.unref?.();
      return;
    }
    clearInterval(this.#controlTimer);
    this.#controlTimer = undefined;
  }

  /**
   * One tick of the control timer. A session's saved settings and its queued audit requests change only through a
   * commit by another connection (`rerail settings`, `rerail scan`, the daemon's API), so a session read at the
   * database's current `data_version` is not read again until it moves: an idle daemon then makes one pragma call per
   * tick, however many sessions it holds open.
   */
  #pollAuditControls(): void {
    const version = this.#controls.dataVersion();
    for (const [sessionId, state] of this.#sessions) {
      if (state.controlsVersion !== version) this.#scheduleAuditRequests(sessionId, version);
    }
  }

  /** Stopped repositories record and steer nothing, even in sessions that attached before the stop. */
  #stopped(): boolean {
    return this.#startWatch !== undefined && !this.#startWatch.started();
  }

  ingest(event: AdapterEvent, options: IngestOptions = {}): Promise<readonly AdapterDelivery[]> {
    const sessionId = event.kind === "session-start" ? event.session.sessionId : event.sessionId;
    if (this.#stopped() && event.kind !== "session-end") return Promise.resolve([]);
    return this.#ingestLanes.run(sessionId, async () => {
      // Checked again in the lane: an event accepted while started can run after a stop committed elsewhere.
      if (this.#stopped()) {
        if (event.kind === "session-end") await this.#endWhileStopped(sessionId);
        return [];
      }
      const sourceSignature = this.#sessions.get(sessionId)?.ruleSync?.sourceSignature;
      const deliveries = await this.#ingestOrDropOwed(event, options);
      if (event.kind !== "assistant-delta" || event.final
          || sourceSignature !== this.#sessions.get(sessionId)?.ruleSync?.sourceSignature) {
        await this.#sessionRulesBuilt(sessionId);
        this.recordRuleCoverage(sessionId);
      }
      return deliveries;
    });
  }

  /**
   * A session's end in a stopped repository records nothing, but ends what the session holds and drops what it owed its
   * agent, as any end does. A session open here closes, releasing its code-rule bridge and polling. One that is not (a
   * suspended one, which is not opened again for it and whose service ends its record, or one a runtime before a daemon
   * restart held) has the `owed` entry its ledger keeps emptied: the session's next start under the same id would take
   * it back. An empty entry records nothing.
   */
  async #endWhileStopped(sessionId: string): Promise<void> {
    if (this.#sessions.has(sessionId)) await this.closeSession(sessionId);
    else await this.#dropLedgerOwed(sessionId);
  }

  /**
   * `#ingest`, and for a session's end that could not open the session (its folder gone, its evaluator failing), the
   * `owed` entry its ledger keeps emptied before the failure goes on: an end drops what the session owed, as any end
   * does, and the session's next start under the same id would take it back. Its engine service ends its record.
   */
  async #ingestOrDropOwed(event: AdapterEvent, options: IngestOptions): Promise<readonly AdapterDelivery[]> {
    try {
      return await this.#ingest(event, options);
    } catch (error) {
      if (event.kind === "session-end" && !this.#sessions.has(event.sessionId)) await this.#dropLedgerOwed(event.sessionId);
      throw error;
    }
  }

  /** Writes an emptied `owed` entry to the ledger of a session not open here, when its last one owes something. */
  async #dropLedgerOwed(sessionId: string): Promise<void> {
    const path = join(sessionDirectory(this.#storageRoot, sessionId), "events.jsonl");
    try {
      const { entries, appendState } = await readSessionLedger(path, { sampleText: false });
      if (restoredOwed(entries).recorded === 0) return;
      await appendJsonLine(path, { type: "owed", steers: [], findings: [] } satisfies OwedEntry, appendState);
    } catch (error) {
      process.stderr.write(`rerail: could not drop what session ${sessionId} owed its agent: ${error instanceof Error ? error.message : String(error)}\n`);
    }
  }

  /** Branch navigation restores repeat/turn state without retiring the live stream or code-rule owner. */
  restoreNativeTurn(sessionId: string, turn: number): void {
    this.#session(sessionId).turn = turn;
  }

  /** One event in its session's lane, waited for and timed as `IngestOptions` says. */
  async #ingest(event: AdapterEvent, { signal, observedAt }: IngestOptions): Promise<readonly AdapterDelivery[]> {
    if (event.kind === "session-start") return await this.#ingestSessionStart(event, observedAt);

    const { active, descriptor } = this.#resolveEventSession(event);
    if (active === undefined) await this.#asOf(descriptor.sessionId, observedAt, () => this.#router(descriptor.host).start(descriptor));
    const state = this.#session(event.sessionId);
    // A hook host's stop hook is the end of the agent's run; OMP and Pi end a turn at every model call, and yield later.
    const yields = event.kind === "turn-end" && ADAPTER_CAPABILITIES[descriptor.host].toolSurface !== "native";
    // The event's records, a boundary it met before the code rules opened included, and the run of its tool call, at the
    // time it happened.
    state.observedAt = observedAt;
    try {
      await this.#ingestInSession(state, event, yields, signal);
    } finally {
      state.observedAt = undefined;
    }
    if (event.kind === "session-end") return [];
    return this.#boundaryDeliveries(state, event.eventId, yields, signal);
  }

  /** The work of `event` in its open session, before what its answer delivers. */
  async #ingestInSession(state: RuntimeSessionState, event: Exclude<AdapterEvent, { kind: "session-start" }>, yields: boolean, signal: AbortSignal | undefined): Promise<void> {
    this.#writeOwedClaim(state);
    this.#scheduleAuditRequests(state.descriptor.sessionId);
    state.ruleSync?.refresh();
    // A code-rule boundary reached while the session's evaluator is still opening (indexing) goes on without code rules.
    const bridge = CODE_RULE_BOUNDARIES[event.kind] === true ? await this.#codeRuleWorkspaceNow(state, event) : state.codeRuleBridge;
    bridge?.refreshCodeRules();
    await this.#noteTurnWindow(state, event);
    await this.#ingestEventWork(state, bridge, event, yields, signal);
  }

  /**
   * Runs `work`, which opens the session `sessionId` or records its start, as of `observedAt` (`IngestOptions.observedAt`):
   * a session it opens starts then, and its records made meanwhile keep that time.
   */
  async #asOf(sessionId: string, observedAt: number | undefined, work: () => Promise<void>): Promise<void> {
    if (observedAt === undefined) return await work();
    this.#startsAt.set(sessionId, observedAt);
    const open = this.#sessions.get(sessionId);
    if (open !== undefined) open.observedAt = observedAt;
    try {
      await work();
    } finally {
      this.#startsAt.delete(sessionId);
      const state = this.#sessions.get(sessionId);
      if (state !== undefined) state.observedAt = undefined;
    }
  }

  /** When a session not yet recorded starts as it opens now: when the event opening it happened (`#asOf`), else now. */
  #startTime(sessionId: string): number {
    return this.#startsAt.get(sessionId) ?? Date.now();
  }

  /**
   * When `sessionId` started as recorded, undefined when it never was: its row as it is on disk. The read makes none of
   * this process's waiting record writes first, which would hold this thread, and so every session's hooks, for the lock
   * until the oldest of them is 5 s old. A session this process opened has its row on disk already: its open ends with
   * the owner claim, which this process's waiting writes are made before (`requireRecordWritesMade`), or fails.
   */
  #recordedStart(sessionId: string): number | undefined {
    return this.#sessionStore.storedSession(sessionId)?.startedAt;
  }

  /**
   * The event's own work around the host router's: a call's record, a session end's settling of the calls still waiting
   * (no result comes after it), a result's edit check, a turn end's waits.
   */
  async #ingestEventWork(state: RuntimeSessionState, bridge: CodeRuleBridge | undefined, event: AdapterEvent, yields: boolean, signal: AbortSignal | undefined): Promise<void> {
    if (event.kind === "call") await this.#recordCall(state, bridge, event, signal);
    if (event.kind === "session-end") await this.#settleUnresolvedCalls(state, true);
    await this.#router(state.descriptor.host).ingest(event);
    noteCallsAwaitingResult(state.callsAwaitingResult, event);
    if (event.kind === "result") await this.#ingestResult(state, bridge, event);
    if (event.kind === "turn-end") await this.#ingestTurnEnd(state, bridge, yields, signal);
  }

  /**
   * A turn's end: the boundary waits for the source walk, then for the turn's own edit checks (all of them where it
   * `yields`, until `signal` says no one waits for the answer).
   */
  async #ingestTurnEnd(state: RuntimeSessionState, bridge: CodeRuleBridge | undefined, yields: boolean, signal: AbortSignal | undefined): Promise<void> {
    const { sessionId } = state.descriptor;
    // The source walk checks the shell writes it finds as generated edits, so the boundary waits from its start.
    await this.#waitingForEditChecks(state, yields, async () => {
      // Before the walk, which checks their files again from here on.
      if (yields) await this.#settleUnresolvedCalls(state);
      const walkStartedAt = performance.now();
      await bridge?.checkpoint("turn");
      this.#reportSlowTurnEndWait(sessionId, "the source walk", walkStartedAt);
      // The turn's own edits, not unrelated audits: the session-start repository audit runs for minutes on a
      // large checkout, and a turn boundary that waited for it held the session's request lane past the host's timeout.
      // A hook host's stop hook carries the turn's steers, so it waits for all of them; OMP and Pi wait within the cap.
      const settleStartedAt = performance.now();
      if (yields) await this.#settleWhileAnswerWaited(state, bridge, signal);
      else await this.#awaitEditChecks(state, bridge, NATIVE_EDIT_CHECK_MS);
      this.#reportSlowTurnEndWait(sessionId, "generated-edit audits", settleStartedAt);
    });
  }

  /**
   * What a boundary answers: what the session holds for its agent (`#heldForAgent`), and the deliveries held for the
   * event itself. An event no one waits for any more (`signal` aborted) takes nothing the session holds: no one reads its
   * answer, so its notes and steers wait for the session's next event, unmarked and unrecorded.
   */
  async #boundaryDeliveries(state: RuntimeSessionState, eventId: string, yields: boolean, signal: AbortSignal | undefined): Promise<AdapterDelivery[]> {
    const deliveries = signal?.aborted === true ? [] : await this.#heldForAgent(state, yields);
    deliveries.push(...(state.eventDeliveries.get(eventId) ?? []));
    state.eventDeliveries.delete(eventId);
    return deliveries;
  }

  /** The Merge Assist notes and steers pending for the session, and the missed digest where the agent `yields`. */
  async #heldForAgent(state: RuntimeSessionState, yields: boolean): Promise<AdapterDelivery[]> {
    this.#offerMergeAssistNotes(state);
    const deliveries = await this.#collectPendingSteers(state);
    if (yields) {
      const digest = await this.#takeMissedDigest(state);
      if (digest !== undefined) deliveries.push(digest);
    }
    return deliveries;
  }

  /**
   * A tool call's result: the call's edit check finishes while the result is ingested, so a steer from it rides the
   * result; and an edit's result runs Merge Assist's detection, waiting for it only as `#detectAfterEditResult` says.
   */
  async #ingestResult(state: RuntimeSessionState, bridge: CodeRuleBridge | undefined, event: Extract<AdapterEvent, { kind: "result" }>): Promise<void> {
    const heldNotes = this.#detectAfterEditResult(state, event.toolCallId);
    const index = state.pendingCodeRuleCalls.findIndex((call) => call.id === event.toolCallId);
    if (index >= 0) {
      const [call] = state.pendingCodeRuleCalls.splice(index, 1);
      // While the result is ingested, a steer from the call's edit check rides it, and a `rewind` one may roll the edit
      // back (`#rewind`).
      const window: ResultWindow | undefined = bridge === undefined ? undefined : { toolCallId: call!.id, steers: [] };
      state.resultWindow = window;
      try {
        // From the finish on, which may check the edit before it returns, to the end of the wait.
        await this.#waitingForEditChecks(state, false, async () => {
          await this.#finishCall(state, bridge, event);
          await this.#append(state, { type: "policy-result", toolCallId: call!.id });
          // The call's own edit may steer; a walk's `log` edits queued around it steer no one, so the result waits for none.
          await this.#awaitEditChecks(state, bridge, ADAPTER_CAPABILITIES[state.descriptor.host].toolSurface === "native" ? NATIVE_EDIT_CHECK_MS : HOOK_EDIT_CHECK_MS, "steering");
        });
      } finally {
        if (window !== undefined) {
          // A steer from now on is a message; one that already rolled the edit back is queued before this answers.
          state.resultWindow = undefined;
          bridge!.release(call!.id);
          await Promise.all(window.steers);
        }
      }
    }
    await heldNotes;
  }

  /**
   * Finishes the calls still waiting for their result when the agent yields, as their results would: none will come.
   * Claude Code asks before a write and sends no PostToolUse when the person refuses it, and a hook's result is lost
   * while the daemon is down. A pending call keeps its files out of every source walk, so a shell write to one went
   * unchecked for the session's life, and a reopen began the call again from the ledger. Each is checked against what
   * it captured and closed in the ledger; a result that comes after all checks nothing. An evaluator still opening
   * begins the calls it restores from the ledger once open, so they wait for the next yield. Which calls are due, a
   * subagent's among them, and the session end's (`sessionEnds`): `takeUnanswered`.
   */
  async #settleUnresolvedCalls(state: RuntimeSessionState, sessionEnds = false): Promise<void> {
    if (state.codeRuleOpening) return;
    const calls = takeUnanswered(state.pendingCodeRuleCalls, sessionEnds);
    if (calls.length === 0) return;
    await state.codeRuleBridge?.finishUnanswered(calls.map(call => call.id));
    for (const call of calls) {
      state.mergeAssistEdits.delete(call.id);
      await this.#append(state, { type: "policy-result", toolCallId: call.id });
    }
  }

  /** Runs `wait` as a boundary waiting for the session's edit checks: the steers they produce meanwhile ride its answer. */
  async #waitingForEditChecks(state: RuntimeSessionState, yields: boolean, wait: () => Promise<void>): Promise<void> {
    state.editCheckWaits += 1;
    if (yields) state.yieldWaits += 1;
    try {
      await wait();
    } finally {
      state.editCheckWaits -= 1;
      if (yields) state.yieldWaits -= 1;
    }
  }

  /**
   * A boundary's wait for the session's edit checks, capped at `capMs`; with `steering`, for those that may steer only.
   * While checks an earlier boundary stopped waiting for still drain, a wait that would sit behind them answers at once
   * rather than spend the cap again (`waitsBehind`), and those steers wait for the yield digest.
   */
  async #awaitEditChecks(state: RuntimeSessionState, bridge: CodeRuleBridge | undefined, capMs: number, only?: "steering"): Promise<void> {
    if (bridge === undefined || waitsBehind(state.overdueEditChecks, only)) return;
    const settled = bridge.settleGeneratedEdits(only);
    if (await settledWithin(settled, capMs)) return;
    this.#leaveOverdue(state, settled, overdueKinds(only, bridge.steeringWaitsForLog()));
  }

  /**
   * A hook host's stop waits for every edit check of the session, so the turn's steers ride its answer, for as long as
   * its answer is waited for (`signal`): once the hook gives up, the stop stops waiting and the session's next event runs
   * at once. The checks carry on, overdue as past a cap (`#awaitEditChecks`), and their steers wait for that next event.
   */
  async #settleWhileAnswerWaited(state: RuntimeSessionState, bridge: CodeRuleBridge | undefined, signal: AbortSignal | undefined): Promise<void> {
    if (bridge === undefined) return;
    const settled = bridge.settleGeneratedEdits();
    if (await settledBefore(settled, signal)) return;
    this.#leaveOverdue(state, settled, overdueKinds(undefined, bridge.steeringWaitsForLog()));
  }

  /**
   * The checks `settled` waits for carry on past the boundary that stopped waiting for them, overdue under `kinds`: a
   * later wait of those kinds would sit behind them, so it answers at once (`waitsBehind`).
   */
  #leaveOverdue(state: RuntimeSessionState, settled: Promise<void>, kinds: ReadonlyArray<keyof OverdueEditChecks>): void {
    const overdue: Promise<void> = settled.catch(() => undefined).finally(() => {
      for (const kind of kinds) if (state.overdueEditChecks[kind] === overdue) delete state.overdueEditChecks[kind];
    });
    for (const kind of kinds) state.overdueEditChecks[kind] = overdue;
  }

  /** A turn end that waits long holds the session's request lane: the daemon log says how long and on what; fast ones say nothing. */
  #reportSlowTurnEndWait(sessionId: string, waitedOn: string, startedAt: number): void {
    const milliseconds = performance.now() - startedAt;
    if (milliseconds <= SLOW_TURN_END_WAIT_MS) return;
    this.#timingLog?.(`rerail: turn end of session ${sessionId} waited ${Math.round(milliseconds)} ms for ${waitedOn}\n`);
  }

  /**
   * Begins a tool call on the open evaluator. One answered while the evaluator opens is never begun, but what it writes
   * is the session's own, so it opens only its window, which names the files its tool names (`CodeRuleBridge.unwatchedCall`).
   * One no one waited for as it began (`signal` aborted: its hook gave up, or left it while the daemon did not read) may
   * have run already, so what its files hold now is no "before": it captures none, and its edit is read against the text
   * it was made on where its own text edits make that certain, else whole (`UNSEEN_START`).
   */
  async #beginCall(
    state: RuntimeSessionState,
    bridge: CodeRuleBridge | undefined,
    event: Extract<AdapterEvent, { kind: "call" }>,
    signal: AbortSignal | undefined,
  ): Promise<Readonly<Record<string, Preimage>> | undefined> {
    const unseen = signal?.aborted === true ? UNSEEN_START : undefined;
    if (bridge !== undefined) return await bridge.begin(event.toolCallId, event.toolName, event.input, unseen, event.cwd);
    await state.codeRuleBridge?.unwatchedCall(event.toolCallId, { toolName: event.toolName, input: event.input, cwd: event.cwd });
    return undefined;
  }

  /**
   * Begins a tool call and records it in the ledger and among the session's pending calls, with the subagent that made
   * it: a subagent's call outlives the main agent's stop, after a daemon restart too, which settled it at the first stop.
   * An edit is marked for Merge Assist's detection at its result.
   */
  async #recordCall(state: RuntimeSessionState, bridge: CodeRuleBridge | undefined, event: Extract<AdapterEvent, { kind: "call" }>, signal: AbortSignal | undefined): Promise<void> {
    const id = event.toolCallId;
    const agent = event.agentId === undefined ? {} : { agentId: event.agentId };
    const preimages = await this.#beginCall(state, bridge, event, signal);
    const paths = generatedEditPaths(event.toolName, event.input);
    await this.#append(state, {
      type: "policy-call", toolCallId: id, toolName: event.toolName, paths,
      ...(preimages === undefined ? {} : { preimages }), ...agent, ...(event.cwd === undefined ? {} : { cwd: event.cwd }),
    });
    const checked = checkAtResult(event.toolName, paths) !== undefined;
    state.pendingCodeRuleCalls.push({ id, toolName: event.toolName, checked, ...agent });
    if (paths.length > 0) state.mergeAssistEdits.add(id);
  }

  /**
   * A host whose own edit tool no hook shows (`observedFileWrite: "shell"`: Codex's `apply_patch`) writes outside every
   * call's window, so its turn, from the prompt to the turn's end, is one: what is stamped then is the session's own.
   * A call opens it too, as a turn has one only at its prompt, which a daemon restart in the middle of the turn loses;
   * a window already open keeps its start. Two of its sessions' turns in one folder at once both take a change made
   * then, as overlapping calls do. It runs in the folder the hook names (Codex's `cwd`), as the turn's calls do.
   */
  async #noteTurnWindow(state: RuntimeSessionState, event: AdapterEvent): Promise<void> {
    if (ADAPTER_CAPABILITIES[state.descriptor.host].observedFileWrite !== "shell") return;
    if (event.kind === "user-prompt" || event.kind === "call") await state.codeRuleBridge?.unwatchedCall(TURN_WINDOW_ID, { cwd: event.session?.cwd });
    else if (event.kind === "turn-end") await state.codeRuleBridge?.unwatchedResult(TURN_WINDOW_ID);
  }

  /**
   * Finishes a tool call on the open evaluator, with what the tool's result says the file it wrote held: only Claude
   * Code's Write and Edit results say it, so no other agent's result is read as theirs. One answered while the evaluator
   * opened ends its window (`unwatchedResult`), and is checked once it is open when the ledger held it open
   * (`#checkWhenOpen`).
   */
  async #finishCall(state: RuntimeSessionState, bridge: CodeRuleBridge | undefined, result: Extract<AdapterEvent, { kind: "result" }>): Promise<void> {
    if (bridge === undefined) {
      await this.#finishUnwatched(state, result.toolCallId, result.failed === true);
      return;
    }
    const reported = state.descriptor.host === "claude-code" ? claudeCodeReportedPreimage(result.output) : undefined;
    await bridge.finish(result.toolCallId, result.failed === true, { rewindable: true, reported });
  }

  async #finishUnwatched(state: RuntimeSessionState, toolCallId: string, failed: boolean): Promise<void> {
    await state.codeRuleBridge?.unwatchedResult(toolCallId);
    const call = state.restoredCalls.get(toolCallId);
    if (call !== undefined) this.#checkWhenOpen(state, call, failed);
  }

  /**
   * Checks the edit of a call the ledger held open as the session opened, whose result came while the evaluator opened,
   * against what the call captured before the session closed, once the evaluator is open: as `finishUnanswered` checks
   * a call whose result will not come. The session reopened for that result (its agent was left at a permission prompt
   * past the day the engine service holds such a session open, or the daemon restarted meanwhile), so the boundary
   * itself went on without code rules, and the edit, made before the evaluator's first walk, would never be checked. The
   * evaluator held the call's files from that walk (`heldCallsOf`), so the edit is read once, here, as the call's, not
   * first as another writer's change. Its steer goes out with a later answer, or with the digest, as a check that
   * outlasted its boundary does.
   */
  #checkWhenOpen(state: RuntimeSessionState, call: RestoredCall, failed: boolean): void {
    state.restoredCalls.delete(call.toolCallId);
    void state.codeRuleWorkspace.then(async bridge => {
      if (bridge !== undefined && !state.closed) await this.#checkRestoredCall(state, bridge, call, failed);
    });
  }

  async #checkRestoredCall(state: RuntimeSessionState, bridge: CodeRuleBridge, call: RestoredCall, failed: boolean): Promise<void> {
    try {
      await bridge.begin(call.toolCallId, call.toolName, { paths: call.paths }, call.preimages ?? {}, call.cwd);
      await bridge.finish(call.toolCallId, failed);
    } catch (error) {
      // A session closed meanwhile closed its evaluator too.
      if (!state.closed) process.stderr.write(`rerail: could not check tool call ${call.toolCallId}: ${error instanceof Error ? error.message : String(error)}\n`);
    }
  }

  /**
   * The session's evaluator for a code-rule boundary. One still opening (indexing the repository, compiling its rules)
   * is not waited for at a session's first start, so the agent is never held up by a first index: such a boundary is
   * answered without code rules and recorded in the session's history. The boundary's own check is skipped, but the file
   * stays checked: the first pass once the evaluator is open finds the edit and checks it, so its steer can arrive a
   * turn later. A reopened session waits for its rules instead at a boundary that needs them (`whileRulesOpen`), within
   * `REOPENED_RULES_WAIT_MS`, so its first edit is checked at its own result as every later one is. Once a tool call or
   * result has waited that out, later ones go on without code rules as at a first start, and only turn ends still wait.
   * A reopened session's boundary that needs no rules goes on at once and is not recorded: it skipped no check.
   */
  async #codeRuleWorkspaceNow(state: RuntimeSessionState, event: AdapterEvent): Promise<CodeRuleBridge | undefined> {
    if (!state.codeRuleOpening) return await state.codeRuleWorkspace;
    const step = whileRulesOpen(state.rulesWait, event, state.pendingCodeRuleCalls);
    if (step === "wait") {
      if (await settledWithin(state.codeRuleWorkspace, REOPENED_RULES_WAIT_MS)) return await state.codeRuleWorkspace;
      state.rulesWait = "turn-end";
    }
    if (step !== "pass") this.#recordRulesPending(state, event);
    return undefined;
  }

  /** Records in the session's history that a code-rule boundary was answered without code rules; best-effort. */
  #recordRulesPending(state: RuntimeSessionState, event: AdapterEvent): void {
    if (event.kind !== "call" && event.kind !== "result" && event.kind !== "turn-end") return;
    const boundary = event.kind;
    try {
      this.#recordSessionFact(
        state.descriptor.sessionId,
        (store, descriptor, attribution) => store.recordCodeRulesPending(descriptor, boundary, attribution),
        happenedAt(state),
      );
    } catch (error) {
      // Recording a boundary that answered without code rules is best-effort: it never fails the boundary.
      process.stderr.write(`rerail: policy execution recording failed: ${error instanceof Error ? error.message : String(error)}\n`);
    }
  }

  /**
   * Completed code-rule results not yet handed to the host, for hosts that poll instead of
   * receiving `deliverBackground` pushes. Unknown or closed sessions have nothing pending. The findings held for the
   * yield digest are not among them.
   */
  collectDeliveries(sessionId: string): Promise<AdapterDelivery[]> {
    // In the session's lane, as `yieldDeliveries`: one answer at a time takes what the session owes.
    return this.#ingestLanes.run(sessionId, async () => {
      const state = this.#sessions.get(sessionId);
      return state === undefined || this.#stopped() ? [] : await this.#collectPendingSteers(state);
    });
  }

  /**
   * The end of an OMP or Pi agent's run, which their turn end is not (they end a turn at every model call). Finishes
   * the calls still waiting for their result, as a hook host's stop does; waits, as a turn end does, for the session's
   * edit checks within `NATIVE_EDIT_CHECK_MS`, then answers with the steers still pending and the digest of the
   * findings no earlier boundary waited for. Findings that arrive later wait for the session's next yield.
   */
  yieldDeliveries(sessionId: string): Promise<AdapterDelivery[]> {
    if (this.#stopped()) return Promise.resolve([]);
    return this.#ingestLanes.run(sessionId, async () => {
      const state = this.#sessions.get(sessionId);
      if (state === undefined || this.#stopped()) return [];
      const bridge = state.codeRuleReady ? state.codeRuleBridge : undefined;
      await this.#waitingForEditChecks(state, true, async () => {
        await this.#settleUnresolvedCalls(state);
        await this.#awaitEditChecks(state, bridge, NATIVE_EDIT_CHECK_MS);
      });
      const deliveries = await this.#collectPendingSteers(state);
      const digest = await this.#takeMissedDigest(state);
      if (digest !== undefined) deliveries.push(digest);
      return deliveries;
    });
  }

  async #collectPendingSteers(state: RuntimeSessionState): Promise<AdapterDelivery[]> {
    const deliveries = await this.#takeOwed(state);
    // An evaluator still opening may yet fail, and what it observed with it.
    if (!state.codeRuleReady) return deliveries;
    // Delivery removes entries from the list, so iterate a snapshot.
    for (const pending of state.pendingSteers.slice()) {
      const delivery = await this.#deliverSteer(state, pending, false);
      if (delivery === undefined) continue;
      deliveries.push(delivery);
      this.#handedOut.set(delivery, { state, pending });
    }
    return deliveries;
  }

  /**
   * Gives the session back the steers `deliveries` took when their answer never reached its asker: a hook past its own
   * time limit closes its connection while the daemon still runs its event, and the answer goes nowhere. Each goes back
   * where the answer took it from (`#giveBack`), so the session's next answer carries it; that answer drops one a newer
   * revision has overtaken, as it drops any (`#performSteer`, `#stillOwed`). The ledger already holds its delivery, so
   * it is not recorded again, nor after a restart: a close that keeps it owed says so (`recorded`); and a Merge Assist
   * note was marked given as the answer took it, so it is not marked again. A session closed or opened again since keeps
   * none: its close counted them as delivered.
   * Never fails: a failure is reported, and the steers stay as the answer left them.
   */
  returnUndelivered(sessionId: string, deliveries: readonly AdapterDelivery[]): Promise<void> {
    return this.#ingestLanes.run(sessionId, async () => {
      try {
        const state = this.#sessions.get(sessionId);
        const returned = deliveries.flatMap(delivery => this.#takeHandedOut(delivery, state));
        if (state === undefined || returned.length === 0) return;
        for (const { pending } of returned) {
          pending.recorded = true;
          // It settled as the answer took it.
          delete pending.settled;
        }
        this.#giveBack(state, returned);
      } catch (error) {
        process.stderr.write(`rerail: could not keep the steers of an answer session ${sessionId} never got: ${error instanceof Error ? error.message : String(error)}\n`);
      }
    });
  }

  /** The steer `delivery` came from when an answer of `state`'s took it, no longer counted as handed out; else none. */
  #takeHandedOut(delivery: AdapterDelivery, state: RuntimeSessionState | undefined): HandedOut[] {
    const handed = this.#handedOut.get(delivery);
    this.#handedOut.delete(delivery);
    return handed === undefined || handed.state !== state ? [] : [handed];
  }

  /**
   * Puts each steer of `returned` back in front of the list it came from, in its order: one the session owed as it
   * opened with what it still owes (`#takeOwed`), which the session's next answer takes whether or not its evaluator has
   * opened, so an evaluator still opening does not hold it and one that fails to open does not drop it; any other with
   * the session's pending steers.
   */
  #giveBack(state: RuntimeSessionState, returned: readonly HandedOut[]): void {
    const owed = returned.flatMap(handed => (handed.owed === true ? [handed.pending] : []));
    const pending = returned.flatMap(handed => (handed.owed === true ? [] : [handed.pending]));
    state.owed.steers.unshift(...owed);
    state.pendingSteers.unshift(...pending);
  }

  /**
   * The steers the session owed its agent when it last closed, recorded as delivered, for the answer collecting them;
   * one that no longer speaks for its files is dropped (`#stillOwed`). Its held findings join the evaluator's once it is
   * open, checked the same way. The ledger's entry is written again with what is still owed. They are taken only once
   * checked: a close meanwhile, as the runtime's own, finds them still owed and keeps them for the session's next open.
   */
  async #takeOwed(state: RuntimeSessionState): Promise<AdapterDelivery[]> {
    const steers = [...state.owed.steers];
    const findings = state.codeRuleReady ? [...state.owed.findings] : [];
    if (steers.length + findings.length === 0) return [];
    const [heldSteers, heldFindings] = await Promise.all([this.#stillOwedOf(state, steers), this.#stillOwedOf(state, findings)]);
    if (state.closed) return [];
    state.owed.steers = state.owed.steers.filter(steer => !steers.includes(steer));
    state.owed.findings = state.owed.findings.filter(finding => !findings.includes(finding));
    state.missedFindings.unshift(...heldFindings);
    await this.#recordOwed(state, this.#owedEntry(state, [], state.owed.findings));
    const deliveries: AdapterDelivery[] = [];
    for (const steer of heldSteers) {
      // One an answer no client heard took before the session closed is in the ledger already.
      const delivery = steer.recorded === true ? steer.delivery : await this.#recordedDelivery(steer.delivery);
      deliveries.push(delivery);
      this.#handedOut.set(delivery, { state, pending: steer, owed: true });
    }
    return deliveries;
  }

  /** `owed` without what no longer speaks for its files (`#stillOwed`), in its order. */
  async #stillOwedOf<T extends Kept<PendingSteer> | Kept<MissedFinding>>(state: RuntimeSessionState, owed: readonly T[]): Promise<T[]> {
    const holds = await Promise.all(owed.map(async item => await this.#stillOwed(state, item)));
    return owed.filter((_, index) => holds[index]);
  }

  /**
   * Whether a steer or held finding taken back from the ledger still speaks for the files it names: no revision since
   * the session opened again overtook it (`#isCurrentSteer`), and each holds what the session last saw in it before it
   * closed. The opening's first walk takes files as it finds them, so a file the person or another session changed while
   * this one was closed would otherwise leave it current. A rolled-back edit's steer goes all the same, as
   * `#isCurrentSteer` says. One that does not is dropped as a revision drops it. One read per file it names.
   */
  async #stillOwed(state: RuntimeSessionState, item: Kept<PendingSteer> | Kept<MissedFinding>): Promise<boolean> {
    const holds = this.#isCurrentSteer(state, item) && (rollsBack(item) || (await changedFiles(state.checkout.cwd, item.digests)).length === 0);
    if (!holds) this.#recordUndelivered(state, item);
    return holds;
  }

  /** `delivery`, recorded in its session's ledger; a failure to record it is reported and does not withhold it. */
  async #recordedDelivery(delivery: AdapterDelivery): Promise<AdapterDelivery> {
    try {
      await this.persistDelivery(delivery);
    } catch (error) {
      process.stderr.write(`rerail: could not record policy delivery: ${error instanceof Error ? error.message : String(error)}\n`);
    }
    return delivery;
  }

  /**
   * What the session owes its agent as it closes, taken from it: the steers and held findings no newer revision has
   * overtaken, when the close keeps them (`keepsOwed`); none at its end. A Merge Assist note waits with Merge Assist
   * instead (`ownSteers`).
   */
  #owedAtClose(state: RuntimeSessionState): Owing {
    const current = <T extends PendingSteer | MissedFinding>(owed: T[]): T[] => owed.filter(item => this.#isCurrentSteer(state, item));
    const owing: Owing = {
      kept: { steers: current(state.owed.steers), findings: current(state.owed.findings) },
      found: { steers: current(ownSteers(state.pendingSteers)), findings: current(state.missedFindings) },
    };
    state.owed.steers = [];
    state.owed.findings = [];
    state.pendingSteers = [];
    state.missedFindings = [];
    return state.keepsOwed ? owing : { kept: { steers: [], findings: [] }, found: { steers: [], findings: [] } };
  }

  #owedEntry(state: RuntimeSessionState, steers: readonly Kept<PendingSteer>[], findings: readonly Kept<MissedFinding>[]): OwedEntry {
    // Judged as the digest judges them, so a file there that links to a secret is one.
    const sensitive = findings.length === 0 ? () => false : checkoutPathBoundary(state.checkout.cwd).sensitive;
    return {
      type: "owed",
      steers: steers.map((steer): OwedSteer => ({
        delivery: steer.delivery, scope: [...steer.scope], digests: { ...steer.digests }, ...(steer.recorded === true ? { recorded: true } : {}),
      })),
      findings: findings.map(finding => owedFinding(finding.violation, finding.sourceOf(finding.violation.ruleId), finding, sensitive)),
    };
  }

  /**
   * Writes what the session owes as it closes (`#owedAtClose`) when it owes something, or its ledger says it did. Runs
   * before the session's evaluator closes, which says what the files held as the session last saw them.
   */
  async #keepOwed(state: RuntimeSessionState, owed: Owing): Promise<void> {
    const steers = [...owed.kept.steers, ...await this.#withDigests(state, owed.found.steers)];
    const findings = [...owed.kept.findings, ...await this.#withDigests(state, owed.found.findings)];
    if (steers.length + findings.length > 0 || state.owed.recorded > 0) await this.#recordOwed(state, this.#owedEntry(state, steers, findings));
  }

  /**
   * `items` with what each file they name held as the session last saw it: as its evaluator last observed it, which a
   * file changed outside the session since the evaluator's last walk has not changed, else as the file reads now.
   */
  async #withDigests<T extends PendingSteer | MissedFinding>(state: RuntimeSessionState, items: readonly T[]): Promise<Kept<T>[]> {
    const seen = async (path: string): Promise<string | null> =>
      state.codeRuleBridge?.observedDigest(path) ?? await fileDigest(join(state.checkout.cwd, path));
    return await Promise.all(items.map(async item => {
      const digests = await Promise.all(item.scope.map(async path => [path, await seen(path)] as const));
      return { ...item, digests: Object.fromEntries(digests) };
    }));
  }

  /**
   * Writes `entry` to the session's ledger, whose last `owed` entry says what the session owes while it is closed. A
   * stopped repository records nothing, but an empty entry, which drops what the ledger kept, is still written: a
   * session that ends there owes nothing after. A failure is reported, never raised: it must not fail the close or the
   * answer.
   */
  async #recordOwed(state: RuntimeSessionState, entry: OwedEntry): Promise<void> {
    if (this.#stopped() && entry.steers.length + entry.findings.length > 0) return;
    try {
      await this.#append(state, entry);
      state.owed.recorded = entry.steers.length + entry.findings.length;
    } catch (error) {
      process.stderr.write(`rerail: could not record what session ${state.descriptor.sessionId} owes its agent: ${error instanceof Error ? error.message : String(error)}\n`);
    }
  }

  /**
   * One steer listing, per rule, every finding held since the session's last yield that no later edit made stale,
   * each once, in the order they arrived; recorded like any steer. Undefined when there is none.
   */
  async #takeMissedDigest(state: RuntimeSessionState): Promise<AdapterDelivery | undefined> {
    // An evaluator still opening may yet fail, and what it observed with it.
    if (!state.codeRuleReady || this.#stopped()) return undefined;
    const findings = state.missedFindings.filter(finding => this.#isCurrentSteer(state, finding));
    state.missedFindings = [];
    const seen = new Set<string>();
    const violations: CodeRuleViolation[] = [];
    const sources = new Map<string, SteerSourceOf>();
    for (const { violation, sourceOf } of findings) {
      const key = JSON.stringify([violation.ruleId, violation.message, violation.path, violation.span?.startLine, violation.span?.startColumn]);
      if (seen.has(key)) continue;
      seen.add(key);
      violations.push(violation);
      if (!sources.has(violation.ruleId)) sources.set(violation.ruleId, sourceOf);
    }
    if (violations.length === 0) return undefined;
    // Its lead says the findings belong to steps the agent has already moved past. Their paths are the session
    // folder's, so a file there that links to a secret is judged as one.
    const sensitive = checkoutPathBoundary(state.checkout.cwd).sensitive;
    const steer = buildSteer(violations, { kind: "late" }, ruleId => sources.get(ruleId)?.(ruleId), sensitive, steerShell(state.descriptor.host));
    const digest: AdapterDelivery = {
      sessionId: state.descriptor.sessionId,
      requested: "steer",
      mechanism: ADAPTER_CAPABILITIES[state.descriptor.host].delivery.steer,
      content: steer.content,
      ruleIds: steer.ruleIds,
      rules: steer.rules,
    };
    try {
      await this.persistDelivery(digest);
    } catch (error) {
      process.stderr.write(`rerail: could not record policy delivery: ${error instanceof Error ? error.message : String(error)}\n`);
    }
    // A stop committed elsewhere while the ledger was written withholds it, as it does any steer.
    return state.closed || this.#stopped() ? undefined : digest;
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

  /** Internal testing: refused while conversation rules are off. */
  async testRule(sessionId: string, reference: string, text: string): Promise<{ similarity: number }> {
    requireExperiment("CONVERSATION_RULES");
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
    // Internal testing: refused while conversation rules are off.
    requireExperiment("CONVERSATION_RULES");
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

  /** Records the receipt of a steer delivered in `sessionId`, crediting `ruleIds`; throws for a session it does not hold. */
  recordSteer(sessionId: string, receiptId: string, ruleIds: readonly string[] = []): void {
    if (!this.recordHeldSteer(sessionId, receiptId, ruleIds)) throw new Error(`Unknown session: ${sessionId}`);
  }

  /**
   * `recordSteer` for a caller that may hand over a receipt of a session this runtime does not hold (the engine service
   * routes a hook's receipt by its folder): false, recording nothing, for such a session, open or closed by it.
   */
  recordHeldSteer(sessionId: string, receiptId: string, ruleIds: readonly string[] = []): boolean {
    if (!this.#sessions.has(sessionId) && !this.#closedSessions.has(sessionId)) return false;
    this.#recordSessionFact(sessionId, (store, descriptor, attribution) => store.recordSteer(descriptor, receiptId, ruleIds, attribution));
    return true;
  }

  /** The host's record of session events it never delivered, so the session's history reads as incomplete. */
  recordDroppedEvents(sessionId: string, dropped: DroppedEvents): void {
    this.#recordSessionFact(sessionId, (store, descriptor, attribution) => store.recordDroppedEvents(descriptor, dropped, attribution));
  }

  #recordSessionFact(
    sessionId: string,
    record: (store: SqliteSessionStore, descriptor: AdapterSession, attribution: SessionAttribution) => void,
    // A fact of the event the session is ingesting: recorded at the time that event happened (`happenedAt`).
    happened?: { observedAt?: number },
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
        ...happened,
      });
    } finally {
      if (this.#sessionStoreClosed) sessionStore.close();
    }
  }

  /**
   * Settles once the rules of the session's checkout are built, compiled off the daemon's thread (`CheckoutCatalogs.prepare`):
   * at once after the first time. The coverage this event records then reads them as built, and the answer that waits for
   * it names the session's rules as before, while every other session's events run.
   */
  async #sessionRulesBuilt(sessionId: string): Promise<void> {
    const state = this.#sessions.get(sessionId);
    if (!recordsCoverage(state)) return;
    // A build that fails here is the coverage's to report, as it builds the rules itself then.
    await this.#catalogs.prepare(state.checkout.root).catch(() => undefined);
  }

  /** Report loaded sources, never refresh enforcement for presentation metadata. */
  recordRuleCoverage(sessionId: string): void {
    const state = this.#sessions.get(sessionId);
    if (!recordsCoverage(state)) return;
    try {
      const coverage = this.#ruleCoverage(state);
      if (coverage !== undefined) this.#sessionStore.recordRuleCoverage(state.descriptor, coverage);
    } catch (error) {
      console.error("rerail rule coverage:", error);
    }
  }

  /** The rules `state`'s session runs, as its checkout's catalog loaded them; undefined in a checkout Rerail does not record. */
  #ruleCoverage(state: RuntimeSessionState): RuleCoverage | undefined {
    const { root } = state.checkout;
    const repositoryId = this.#repositoryKey(state.checkout);
    if (repositoryId === null) return undefined;
    const catalog = this.#catalogs.catalog(root);
    const loaded = catalog.status.sourceSignature;
    // Current only while the code and conversation rules were loaded from the same configuration.
    const synchronized = state.ruleSync === undefined || state.ruleSync.sourceSignature === loaded;
    return {
      repositoryId,
      ownerCheckoutPaths: this.#catalogs.ownerCheckoutPaths(root),
      activeRuleCount: (state.registry?.count ?? 0) + catalog.status.ruleCount,
      ruleSignature: (synchronized ? configurationSignatureOf(loaded) : undefined) ?? "",
    };
  }

  /**
   * What this runtime holds of `event`'s session once it has taken the event, for the facts of its answer: the checkout
   * the session works in and, at a session's start, what the note says of its rules. Undefined for a session not open
   * here. Read from memory, but for the checkout's start state, which every event reads (`#heldState`).
   */
  heldSession(event: AdapterEvent): HeldSession | undefined {
    const state = this.#heldState(event);
    if (state === undefined) return undefined;
    const checkout = this.#catalogs.checkout(state.checkout.root);
    return {
      checkout: checkout === undefined ? null : { repositoryKey: checkout.repositoryKey, checkoutPath: checkout.checkoutPath },
      ...(event.kind === "session-start" ? { sessionNote: this.#sessionNoteRules(state) } : {}),
    };
  }

  /**
   * `event`'s session while it is open here in a checkout where Rerail is started; undefined otherwise. A stopped
   * checkout's runtime takes no event, so what it holds of a session opened before the stop is not read again.
   */
  #heldState(event: AdapterEvent): RuntimeSessionState | undefined {
    const state = this.#sessions.get(event.kind === "session-start" ? event.session.sessionId : event.sessionId);
    return state === undefined || state.closed || this.#stopped() ? undefined : state;
  }

  /**
   * What the session-start note says of `state`'s rules, as the session's footer reads its coverage
   * (`SqliteSessionReader.hud`): its checkout is not set up; the rules it runs while they are current; or no count where
   * the checkout is stopped or the rules were loaded before the configuration last changed, which the session's next
   * tool call loads again. Asked once the session's start is taken, when its coverage is recorded.
   */
  #sessionNoteRules(state: RuntimeSessionState): SessionNoteRules {
    const eligibility = this.#catalogs.eligibility(state.checkout.root);
    if (eligibility === "not-set-up") return { setUp: false };
    return { setUp: true, ruleCount: eligibility === "ok" ? this.#currentRuleCount(state) : null };
  }

  /**
   * The rules `state`'s session runs while its coverage is current as the footer reads it: the configuration signature
   * it records is the configuration's as the session's start has just read it. The start's ingest reads it once, as its
   * conversation rules load or refresh (`LiveConversationRules.sourceSignature`), in this session's lane, so the count
   * reads nothing more; only a runtime with no conversation rules (no model) reads it here. A session whose code and
   * conversation rules were loaded from different configurations records none, so it reads no count either. Null
   * otherwise.
   */
  #currentRuleCount(state: RuntimeSessionState): number | null {
    const coverage = this.#ruleCoverage(state);
    if (coverage === undefined) return null;
    const read = state.ruleSync?.sourceSignature ?? this.#catalogs.catalog(state.checkout.root).sourceSignature();
    return coverage.ruleSignature === configurationSignatureOf(read) ? coverage.activeRuleCount : null;
  }

  #repositoryKey(checkout: StartingCheckout): string | null {
    return this.#catalogs.checkout(checkout.root)?.repositoryKey ?? null;
  }

  /** An evaluator is still opening: `dispose` waits for its release, which the caller may leave running behind it. */
  get evaluatorOpening(): boolean {
    return this.#evaluatorOpens.size > 0;
  }

  /**
   * Settles once the session's code-rule evaluator has opened, or failed to: its repository indexed and its rules
   * compiled, so an edit is checked as it is made. At once for a session that has none or is unknown.
   */
  async codeRulesOpened(sessionId: string): Promise<void> {
    await this.#sessions.get(sessionId)?.codeRuleWorkspace.catch(() => undefined);
  }

  /**
   * Settles once the session's evaluator is open and has built the rules it builds ahead of any call: those of the
   * repositories the session's earlier checks ran in.
   */
  async rulesBuiltAhead(sessionId: string): Promise<void> {
    const bridge = await this.#sessions.get(sessionId)?.codeRuleWorkspace.catch(() => undefined);
    await bridge?.rulesBuiltAhead();
  }

  async dispose(): Promise<void> {
    this.#disposing = true;
    this.#pollControlsWhileSessionsOpen();
    const placingEnded = this.#placing.stop();
    // No conflict change is offered to a session that is closing.
    this.#mergeAssistUnlisten?.();
    // The sessions still open did not end: what each owes its agent waits in its ledger for its next event, as across
    // the daemon's own restart at its heap ceiling.
    for (const state of this.#sessions.values()) state.keepsOwed = true;
    await Promise.all([...this.#sessions.keys()].map(sessionId => this.closeSession(sessionId)));
    await Promise.allSettled(this.#evaluatorOpens);
    await Promise.allSettled(this.#steersInFlight);
    await placingEnded;
    // A Merge Assist this runtime made works over its database: its hook's late answers stop before that closes.
    if (this.#ownsMergeAssist) await this.#mergeAssist?.stop();
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
    // A session the engine service suspended goes on from where its walks stood (`SessionSuspension`).
    const suspension = this.#suspendedSessions?.get(session.sessionId);
    // A resume for a scan stops here while another process holds the write lock, before any of the open's work.
    this.#failWhileWriteLockedForScan(session.sessionId);
    const recordedStart = this.#recordedStart(session.sessionId);
    // Read before the wait below, which can last as long as Git's reads of the checkout.
    const startTime = this.#startTime(session.sessionId);
    // What the checkout's first events ask Git is learnt here without blocking the daemon (`placeCheckout`), so the
    // steps below find it kept: every other session's events are answered meanwhile, and this session's wait in order.
    const ledger = await this.#ledgerWhilePlaced(session, recordedStart === undefined);
    // A session whose row is gone (a power cut took it with its first commits, or `rerail reset` took an ended one's)
    // started when its ledger's first dated event came, not now.
    const startedAt = recordedStart ?? firstDatedEvent(ledger.entries) ?? startTime;
    const checkout = this.#startingCheckout(session);
    const restored = await this.#restoreRegistry(session, ledger.entries, checkout);
    const watermarks = await JsonlWatermarkStore.open(join(ledger.directory, "watermarks.jsonl"));
    // Saved settings win over the runtime's defaults; `#scheduleAuditRequests` keeps the strategy current.
    const auditSettings = this.#controls.getWorkspaceSettings(checkout.cwd);
    const steerStrategy = effectiveSteerStrategy(auditSettings ?? this.#auditSettings);
    const opened = this.#initialSessionState({ session, checkout, startedAt, ledger, restored, watermarks, steerStrategy });
    // Set when the evaluator failed to open: what it observed while opening belongs to no live evaluator.
    let retired = false;
    const steerDelivery: RuleSteerDelivery = {
      requestSteer: async (sessionId, requested, content, metadata) => {
        const activeState = this.#sessions.get(sessionId);
        if (activeState !== opened || activeState.closed || retired) return "discarded";
        const pending: PendingSteer = {
          delivery: {
            sessionId,
            requested,
            mechanism: ADAPTER_CAPABILITIES[activeState.descriptor.host].delivery[requested],
            content,
            ruleIds: metadata?.ruleIds ?? [],
            rules: metadata?.rules ?? [],
            strategy: "message",
          },
          revision: metadata?.revision ?? activeState.codeRuleRevision,
          scope: metadata?.scope ?? [],
        };
        const placed = await this.#placeSteer(activeState, pending, content, metadata, () => retired);
        if (typeof placed === "string") return placed;
        if (placed.boundaryDelivery !== undefined) await placed.boundaryDelivery;
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
        // A held finding is its source's own, so only the revised sources' findings go.
        current.missedFindings = current.missedFindings.filter(finding => {
          if (this.#isCurrentSteer(current, finding)) return true;
          this.#recordUndelivered(current, finding);
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
    this.#pollControlsWhileSessionsOpen();
    try {
      const followsGitignore = this.#catalogs.followsGitignore(checkout.root);
      auditor = this.#createAuditor({
        workspaceRoot: checkout.cwd,
        steerDelivery,
        audits: this.#audits,
        ...(this.#timingLog === undefined ? {} : { timingLog: this.#timingLog }),
        ...this.#auditSettings,
        ...auditSettings,
        followGitignore: followsGitignore,
        steerShell: steerShell(session.host),
      });
      bridge = this.#codeRuleBridge({ session, checkout, auditor, opened, followsGitignore, suspension });
      opened.codeRuleBridge = bridge;
      await this.#beforeSessionPublish?.(session);
      this.#failWhileWriteLockedForScan(session.sessionId);
      this.#sessionStore.openSession(session, startedAt, this.#repositoryKey(checkout));
      const recorded = this.#sessionStore.lastRecordedEvent(session.sessionId, ledgerKinds(ledger.entries));
      this.#sessionStore.recordLostEvents(session, lostEvents(ledger.entries, recorded));
      this.#closedSessions.delete(session.sessionId);
      this.#sessionStore.startHudTracking(session, ledger.completeTracking);
      opened.ownerClaim = this.#registerOwner(session.sessionId);
    } catch (error) {
      await this.#abandonSession(session.sessionId, opened, auditor);
      throw error;
    }
    const settings = { ...this.#auditSettings, ...auditSettings };
    opened.codeRuleOpening = true;
    opened.rulesWait = rulesWaitOnReopen(recordedStart !== undefined, session.host);
    opened.codeRuleWorkspace = this.#openCodeRuleWorkspace(opened, bridge, auditor, ledger.entries, settings).then(open => {
      if (open === undefined) {
        retired = true;
        opened.pendingSteers = [];
        opened.missedFindings = [];
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
      opened.missedFindings = [];
      return undefined;
    }).finally(() => {
      opened.codeRuleOpening = false;
    });
    const workspace = opened.codeRuleWorkspace;
    this.#evaluatorOpens.add(workspace);
    const released = (): void => { this.#evaluatorOpens.delete(workspace); };
    void workspace.then(released, released);
    this.#scanMergeAssist(opened);
    // A suspended session's first event is answered once the text of the files its agent edited is read back, which
    // waits for no rule to compile: the agent's next command may rewrite one, and is measured from that text.
    await bridge.heldTextsRead();
    return { processedEventIds: [...opened.processed] };
  }

  /**
   * The code-rule bridge of `opened`, the session's state as it opens, over `auditor` in the session's folder. It goes on
   * from where `suspension` left the session's walks, when the engine service suspended it. What its walks and its tool
   * edits leave unchecked goes on the session's record; a revision drops what it makes stale.
   */
  #codeRuleBridge({ session, checkout, auditor, opened, followsGitignore, suspension }: {
    session: AdapterSession; checkout: StartingCheckout; auditor: CodeRuleAuditor; opened: RuntimeSessionState;
    followsGitignore: boolean; suspension: SessionSuspension | undefined;
  }): CodeRuleBridge {
    return new CodeRuleBridge(checkout.cwd, auditor, {
      observations: this.#sourceObservations,
      // A call checked after it ran (`IngestOptions.observedAt`) ran when its hooks met it.
      eventTime: () => opened.observedAt,
      ...walkResumeOption(suspension),
      heldCalls: heldCallsOf(opened.restoredCalls),
      // Every checkout of a clone has its clone id; a checkout the catalog does not know walks on its own. The walk is
      // looked up at the session's first walk, as one dropped when its last session left is no longer the repository's.
      worktreeWalks: () => this.#worktreeWalks.of(this.#catalogs.checkout(checkout.root)?.cloneId ?? checkout.root),
      ...(this.#discoveryWaitMs === undefined ? {} : { discoveryWaitMs: this.#discoveryWaitMs }),
      // A boundary that answered before its shell-write check finished leaves its session's history marked partial.
      onDiscoveryPending: boundary => this.#recordSessionFact(
        session.sessionId,
        (store, descriptor, attribution) => store.recordDiscoveryPending(descriptor, boundary, attribution),
      ),
      // What a boundary's walks could not check goes on the session's record, once per gap; the agent is not told of it.
      onDiscoveryGaps: gaps => this.#recordSessionFact(
        session.sessionId,
        (store, descriptor, attribution) => store.recordDiscoveryGaps(descriptor, gaps, attribution),
      ),
      // Read as the workspace opens, so a reopened session or a restarted daemon goes on from what the record holds.
      recordedDiscoveryGaps: (sessionId, limit) => this.#sessionStore.discoveryGaps(sessionId, limit),
      // A tool edit Git's ignore rules leave unchecked, and a change that makes them ignore more, go on the session's record.
      onIgnoredEdit: edit => this.#recordSessionFact(
        session.sessionId,
        (store, descriptor, attribution) => store.recordIgnoredEdit(descriptor, edit, attribution),
      ),
      ...(followsGitignore ? {
        onIgnoreRulesChanged: (change: IgnoreRulesChange) => this.#recordSessionFact(
          session.sessionId,
          (store, descriptor, attribution) => store.recordIgnoreRulesChanged(descriptor, change, attribution),
        ),
      } : {}),
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
        current.missedFindings = current.missedFindings.filter(finding => {
          if (this.#isCurrentSteer(current, finding)) return true;
          this.#recordUndelivered(current, finding);
          return false;
        });
      },
    });
  }

  /**
   * Claims the session for this runtime, which delivers its steers in the background when it can, else at the next event.
   * A suspended session takes back the claim it kept while this process still holds it, so the scans queued for it while
   * it was suspended stay queued for it to run. Taking it ends the suspension, so a second runtime opening the session
   * meanwhile registers as before, and is refused while this one owns it. It is taken only once the claim is made: an
   * open that fails leaves the session suspended, so its end, or its daemon's stop, still ends its record.
   */
  #registerOwner(sessionId: string): PolicyOwnerClaim {
    const kept = this.#suspendedSessions?.get(sessionId)?.ownerClaim;
    const claim = kept !== undefined && this.#controls.holdsOwner(sessionId, kept)
      ? kept
      : this.#controls.registerOwner(sessionId, this.#ownerDelivery(), this.#ownerLockId);
    this.#suspendedSessions?.take(sessionId);
    return claim;
  }

  /** How this runtime delivers the steers of the sessions it claims: in the background when it can, else at the next event. */
  #ownerDelivery(): "background" | "next-event" {
    return this.#deliverBackground === undefined ? "next-event" : "background";
  }

  /**
   * Writes this runtime's claim on a session it holds again once a write found the session's row gone and put it back
   * (`SqliteSessionStore`'s `onRowPutBack`): another program that removes a running session's rows removes its claim
   * with them, and a scan asked for in the session needs a live owner. A daemon started again would claim the session
   * as it opened it; this one keeps the claim it made then. A claim that fails to land is owed (`claimOwed`), and the
   * session's next event writes it (`#writeOwedClaim`): once the row is back, no write puts it back again.
   */
  #claimAgain(sessionId: string): void {
    const state = this.#sessions.get(sessionId);
    if (state === undefined || state.closed || state.ownerClaim === undefined) return;
    try {
      this.#controls.claimAgain(sessionId, this.#ownerDelivery(), state.ownerClaim);
      delete state.claimOwed;
    } catch (error) {
      // The write that put the row back saved its event first: the claim's failure is said here, as the control scan's
      // are, and never reaches that write's caller, which would report a failed write for an event Rerail holds.
      state.claimOwed = true;
      process.stderr.write(`rerail: could not write the claim on session ${sessionId} again: ${error instanceof Error ? error.message : String(error)}\n`);
    }
  }

  /**
   * Writes the claim a failed `#claimAgain` left owed, at the session's next event and before that event is recorded:
   * a claim write that failed while an event put the row back is tried again at the following event, not at once
   * after its failure, which can have waited out the database's busy timeout already. A session whose claim is in
   * place reads and writes nothing here.
   */
  #writeOwedClaim(state: RuntimeSessionState): void {
    if (state.claimOwed === true) this.#claimAgain(state.descriptor.sessionId);
  }

  /**
   * The session's ledger ({@link #openLedger}), read while its checkout is placed ({@link #placeCheckout}): both settle
   * before it answers or fails, so an open that fails leaves no Git of its placement running in the checkout. Once both
   * stand, the other worktrees Git lists are placed ({@link placeWorktrees}) under the same reads, and the open does not
   * wait for them: its first hook waits for its own checkout only, and a dispose ends that pass and waits for its Git.
   */
  async #ledgerWhilePlaced(session: AdapterSession, unrecorded: boolean): Promise<OpenedLedger> {
    const [ledger, placed] = await Promise.allSettled([this.#openLedger(session, unrecorded), this.#placeCheckout(session)]);
    if (ledger.status === "rejected") throw ledger.reason;
    if (placed.status === "rejected") throw placed.reason;
    const registry = placed.value;
    if (registry !== undefined) void this.#placing.run(signal => placeWorktrees(registry, { signal }));
    return ledger.value;
  }

  /**
   * {@link placeCheckout} for the session's folder, with the registry Git listed for it; a runtime disposed meanwhile
   * fails the open before any Git finds the checkout.
   */
  async #placeCheckout(session: AdapterSession): Promise<WorktreeRegistry | undefined> {
    const registry = await this.#placing.run(signal => placeCheckout(session.cwd, { signal }));
    if (this.#placing.stopped) throw new Error(`the session ${session.sessionId} opened while Rerail was stopping`);
    return registry;
  }

  /** Makes the session's ledger folder, reads its ledger back and marks where history tracking began. */
  async #openLedger(session: AdapterSession, unrecorded: boolean): Promise<OpenedLedger> {
    const directory = sessionDirectory(this.#storageRoot, session.sessionId);
    // A root RERAIL_STORAGE_ROOT names elsewhere is the person's own and used as it is: no .gitignore is written there,
    // and the folder below a `.rerail` on its path keeps its mode and ACL. In the default root the .gitignore is
    // written before the folder's repair starts, so on Windows the new ACL Windows passes on reaches it too.
    if (this.#storageRootIsDefault) await ignoreInGit(this.#storageRoot);
    await makeLedgerFolder(directory, { repair: this.#storageRootIsDefault });
    keepFolderPrivate(directory);
    const path = join(directory, "events.jsonl");
    const { entries, lines, appendState } = await readSessionLedger(path, { sampleText: keepsSamples(session.host) });
    // The ledger marker survives a database reset. Never mistake resumed or reset history for a fresh session.
    // With no DB, ledger, or native replay history, this is the session's first observed Rerail participation.
    const completeTracking = unrecorded && lines === 0 && (this.#nativeSession?.turn ?? 0) === 0;
    if (!entries.some(entry => entry.type === "hud-tracking-start")) {
      await appendJsonLine(path, { type: "hud-tracking-start" }, appendState);
    }
    return { directory, path, entries, appendState, completeTracking };
  }

  /** A session's state as its ledger restores it, before anything of the session is live. */
  #initialSessionState({ session, checkout, startedAt, ledger, restored, watermarks, steerStrategy }: SessionStart): RuntimeSessionState {
    const { registry, turn, ruleSync } = restored;
    const items = ledger.entries.flatMap((entry) => (entry.type === "sample" ? [entry.item] : []));
    const source = new MutableContentSource(items, keepsSamples(session.host));
    const secrets = secretCallsOf(ledger.entries);
    const openCalls = unresolvedCodeRuleCalls(ledger.entries);
    return {
      descriptor: session,
      checkout,
      startedAt,
      processed: processedEventIds(ledger.entries),
      source,
      workspace: new SemanticWorkspace({ sources: [source], watermarks, embedder: this.#embedder }),
      selections: new Map(),
      projections: new Map(),
      ledgerPath: ledger.path,
      ledgerAppendState: ledger.appendState,
      sampleSequence: items.length,
      ...(registry === undefined ? {} : { registry }),
      ruleSync,
      turn,
      eventDeliveries: new Map(),
      pendingSteers: [],
      steerStrategy,
      resultWindow: undefined,
      observedAt: this.#startsAt.get(session.sessionId),
      codeRuleRevision: 0,
      codeRuleSourceVersions: new Map(),
      undeliveredSources: new Set(),
      ownerClaim: undefined,
      controlsVersion: undefined,
      codeRuleWorkspace: Promise.resolve(undefined),
      codeRuleReady: false,
      codeRuleOpening: false,
      rulesWait: "none",
      auditRequestWork: undefined,
      mergeAssistNotesInFlight: new Set(),
      mergeAssistEdits: new Set(),
      repositoryAudit: undefined,
      // Restored whether or not the evaluator opens, so their results still close them in the ledger.
      pendingCodeRuleCalls: openCalls.map(call => ({
        id: call.toolCallId, toolName: call.toolName, checked: checkAtResult(call.toolName, call.paths) !== undefined,
        ...(call.agentId === undefined ? {} : { agentId: call.agentId }),
      })),
      callsAwaitingResult: new Set(),
      overdueEditChecks: {},
      editCheckWaits: 0,
      yieldWaits: 0,
      missedFindings: [],
      owed: restoredOwed(ledger.entries),
      keepsOwed: false,
      restoredCalls: new Map(openCalls.map(call => [call.toolCallId, call])),
      secretCalls: secrets.calls,
      withheldResults: secrets.results,
      closed: false,
    };
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
        // A result that arrived while the evaluator opened went on without code rules: its call is not begun late, or
        // the bridge would hold it pending, and skip its paths in every later shell-write walk, for the session's life.
        if (!state.pendingCodeRuleCalls.some(pending => pending.id === call.toolCallId)) continue;
        await bridge.begin(call.toolCallId, call.toolName, { paths: call.paths }, call.preimages ?? {}, call.cwd);
      }
    } catch (error) {
      delete state.codeRuleBridge;
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
    if (auditor.audit !== undefined && firstRepositoryAuditEnabled()) state.repositoryAudit = this.#runRepositoryAudit(sessionId, state);
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
    opened.missedFindings = [];
    this.#sessions.delete(sessionId);
    this.#pollControlsWhileSessionsOpen();
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

  /**
   * Suspends a session that sent no end and has gone quiet: frees all it holds in memory, as its end would, without
   * recording an event, and its host router forgets it too, so its next event opens it again from its ledger. Its agent
   * may still be open, so it stays open in the database and keeps its owner claim: it reads as live, and a scan asked for
   * it queues for it (`SessionSuspension`). A session that still owes its agent something (`#owesAgent`) stays as it is,
   * unless `evenIfOwing`: the engine service suspends it anyway once it has been quiet a day (`OWING_SESSION_IDLE_MS` in
   * engine-service.ts). The steers it has not delivered and the findings it holds then wait in its ledger, and go out
   * with its next event while their files still hold what they spoke for (`#takeOwed`). `at` is the time by the
   * caller's clock, now by default: the time the session's record ends at if it never opens again
   * (`SessionSuspension.suspendedAt`). With `end` the session is ended instead (`#endIdle`), its record at `at`.
   */
  closeIdleSession(sessionId: string, options: { evenIfOwing?: boolean; at?: number; end?: boolean } = {}): Promise<IdleClose> {
    return this.#ingestLanes.run(sessionId, async (): Promise<IdleClose> => {
      const state = this.#sessions.get(sessionId);
      if (state === undefined) return { kind: "not-held" };
      const owing = (): boolean => options.evenIfOwing !== true && this.#owesAgent(state);
      if (owing()) return { kind: "owing" };
      const at = options.at ?? Date.now();
      if (options.end === true) return await this.#endIdle(state, at);
      const suspension = await this.#suspension(state, at);
      // Asked again: the control timer may have claimed a scan while the walks were read, which the session now runs.
      if (owing()) return { kind: "owing" };
      await this.#suspend(state, suspension);
      return { kind: "suspended", suspension };
    });
  }

  /**
   * Ends a quiet session as its end would: its memory freed, its record ended, its scans interrupted and its owner
   * released, so it reads as ended until its next event opens it again. For a runtime whose owner rows name no lock
   * (`end` of `closeIdleSession`): after a kill, a later start judges such a row only by its pid, which another process
   * can hold, so a session kept open would read as live and owned for good. What the session owes its agent waits in
   * its ledger for that next event, as across its daemon's stop. Its record ends at `at`, the idle close's time.
   */
  async #endIdle(state: RuntimeSessionState, at: number): Promise<IdleClose> {
    state.keepsOwed = true;
    state.endsAt = at;
    await this.#router(state.descriptor.host).stop(state.descriptor.sessionId);
    return { kind: "ended" };
  }

  /**
   * Closes `state`'s session in memory and keeps it open in the database, owned. Its suspension is kept first
   * (`SuspendedSessions.keep`), with no await from the last check of what it owes: a scan asked for it from now on queues
   * for its next open, and a close that fails still leaves it suspended, ended when its daemon stops.
   */
  async #suspend(state: RuntimeSessionState, suspension: SessionSuspension): Promise<void> {
    state.suspending = true;
    state.keepsOwed = true;
    this.#suspendedSessions?.keep(suspension);
    await this.#router(state.descriptor.host).stop(state.descriptor.sessionId);
  }

  /**
   * What `state`'s session keeps while suspended from `suspendedAt` on: its owner claim, and where its walks stand once its
   * evaluator is open.
   */
  async #suspension(state: RuntimeSessionState, suspendedAt: number): Promise<SessionSuspension> {
    const walk = state.codeRuleReady ? await state.codeRuleBridge?.walkResume() : undefined;
    return {
      descriptor: state.descriptor, ownerClaim: state.ownerClaim, walk, databasePath: this.#sessionStore.path, suspendedAt,
      startGate: this.#startWatch?.checkout(),
    };
  }

  /**
   * Opens again a session the engine service suspended, as its next event would, so it claims the scans queued for it
   * meanwhile; nothing is recorded. The engine service asks only while Rerail runs in the session's checkout: a stopped
   * repository records no scan, so it ends such a session instead. A session already open is left as it is. While
   * another process holds the write lock, rejects with SQLite's `SQLITE_BUSY` rather than let a write of the open wait
   * for it (`#failWhileWriteLockedForScan`), leaving the session suspended for the service to try again.
   */
  resumeSession(session: AdapterSession): Promise<void> {
    return this.#ingestLanes.run(session.sessionId, async () => {
      if (this.#sessions.has(session.sessionId)) return;
      this.#resumingForScan.add(session.sessionId);
      try {
        await this.#router(session.host).start(session);
      } finally {
        this.#resumingForScan.delete(session.sessionId);
      }
      this.#scheduleAuditRequests(session.sessionId);
    });
  }

  /**
   * Throws SQLite's `SQLITE_BUSY` at once while another process holds the write lock and `resumeSession` is opening the
   * session again for a scan (`failWhileWriteLocked`): asked right before each run of the open's writes, which would
   * wait for that lock up to 5 s each on the daemon's thread.
   */
  #failWhileWriteLockedForScan(sessionId: string): void {
    if (this.#resumingForScan.has(sessionId)) failWhileWriteLocked(this.#sessionStore.path);
  }

  /**
   * Whether the session still owes its agent something that would reach it later, or not at all, were it suspended and
   * opened again: a tool call of its turn waiting for its result (the result of a session opened again comes while its
   * evaluator opens, so the call's edit is not checked at it), an edit check still running, a steer of its own not yet
   * delivered (`ownSteers`) or a finding held for the yield's digest, among them those its ledger gave back as it opened
   * again (`owedNow`: a scan opens a suspended session with no event to take them), or a scan (`#owesScan`). A steer
   * would reach the agent with the next event all the same, but the session that event opened again would hold the
   * agent's next edit or command while its rules compile again (`REOPENED_RULES_WAIT_MS`), or check its edit only at
   * the turn end in a host that does not wait: a session left in memory checks it at once.
   */
  #owesAgent(state: RuntimeSessionState): boolean {
    return state.callsAwaitingResult.size > 0 || anyOverdue(state.overdueEditChecks)
      || ownSteers(state.pendingSteers).length > 0 || state.missedFindings.length > 0 || owedNow(state) > 0 || this.#owesScan(state);
  }

  /**
   * Whether the session runs a scan or its checkout audit, which the evaluator's close would interrupt, or has a scan
   * queued that its control timer has not claimed yet: the open session claims it within a tick, where a suspended one
   * would leave it queued until its next open. The queue costs one indexed read, made only by the idle close.
   */
  #owesScan(state: RuntimeSessionState): boolean {
    return state.auditRequestWork !== undefined || state.repositoryAuditRunning === true
      || this.#controls.hasQueuedRequest(state.descriptor.sessionId);
  }

  async closeSession(sessionId: string): Promise<void> {
    const state = this.#sessions.get(sessionId);
    // Read or taken before any await: the cap may evict the closed entry while the close below awaits, which gave the
    // session's row the time its close finished rather than its recorded end, and would end a suspended session's row.
    let endedAt = this.#closedSessions.get(sessionId)?.endedAt;
    if (state !== undefined) {
      // Taken first: closing empties what the session holds, and the close keeps what it owes before its evaluator goes.
      const owed = this.#owedAtClose(state);
      endedAt = this.#markClosed(sessionId, state);
      await this.#keepOwed(state, owed);
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
        this.#pollControlsWhileSessionsOpen();
        // The runtime outlives its sessions, and their walks of the other worktrees looked up every folder of each.
        if (this.#sessions.size === 0) this.#owners.forgetFolders();
        // A suspended session's record stays open: its agent may still be at work.
        if (endedAt !== null) this.#sessionStore.closeSession(sessionId, endedAt);
      }
    }
  }

  /**
   * Stops what `state`'s session delivers and what it runs, ends its ownership (`#endOwnership`) and returns the time its
   * record ends at: null while it is being suspended (`closeIdleSession`), whose record stays open; the idle close's
   * time when that close ends it (`endsAt`); now otherwise.
   */
  #markClosed(sessionId: string, state: RuntimeSessionState): number | null {
    state.closed = true;
    const endedAt = state.suspending ? null : state.endsAt ?? Date.now();
    this.#closedSessions.set(sessionId, {
      descriptor: state.descriptor, startedAt: state.startedAt, endedAt, repositoryKey: this.#repositoryKey(state.checkout),
    });
    // A map keeps insertion order, and a session that opens again leaves it: the first key closed longest ago.
    if (this.#closedSessions.size > CLOSED_SESSIONS_KEPT) this.#closedSessions.delete(this.#closedSessions.keys().next().value!);
    this.#endOwnership(sessionId, state);
    return endedAt;
  }

  /**
   * Interrupts the session's queued and running scans and releases its owner, unless it is being suspended, which keeps
   * both for its next open. Its checkout audit is interrupted either way: the evaluator running it closes.
   */
  #endOwnership(sessionId: string, state: RuntimeSessionState): void {
    if (state.ownerClaim === undefined) return;
    this.#controls.interruptRepositoryAudit(sessionId);
    if (state.suspending) return;
    this.#controls.interrupt(sessionId);
    this.#controls.releaseOwner(sessionId, state.ownerClaim);
  }

  /**
   * The audit at session start: one repository audit per checkout it reaches, each claimed before it starts.
   * Internal testing: the automatic first full-repository scan, log-only, off unless {@link firstRepositoryAuditEnabled}.
   */
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
    state.repositoryAuditRunning = true;
    try {
      await state.codeRuleBridge!.audit({ scope: "repository", mode: "log", audit });
    } catch (error) {
      for (const checkoutRoot of claimed) {
        this.#controls.finishRepositoryAudit(sessionId, checkoutRoot, null, "failed", error instanceof Error ? error.message : String(error));
      }
    } finally {
      delete state.repositoryAuditRunning;
    }
  }

  async persistEvent(event: AdapterEvent): Promise<void> {
    const sessionId = event.kind === "session-start" ? event.session.sessionId : event.sessionId;
    const state = this.#session(sessionId);
    await this.#noteSecretCall(state, event);
    const withheld = event.kind === "result" && state.secretCalls.has(event.toolCallId);
    // When the event happened: for one that reaches the runtime later, its own time (`happenedAt`), else now.
    const observedAt = happenedAt(state).observedAt ?? Date.now();
    this.#sessionStore.append({ sessionId, eventId: event.eventId, kind: event.kind, observedAt, payload: storedPayload(event, withheld) }, state.descriptor);
    if (withheld) await this.#noteWithheldResult(state, event);
    // The row's kind and time go with the line, so the session's next open can record which events a power cut took
    // (`lostEvents`).
    await this.#append(state, { type: "event", eventId: event.eventId, kind: event.kind, observedAt });
    state.processed.add(event.eventId);
  }

  /** A call that names a secret-like path, recorded before the event counts as processed: sent again after a crash here, it is judged again. */
  async #noteSecretCall(state: RuntimeSessionState, event: AdapterEvent): Promise<void> {
    if (event.kind !== "call" || !callNamesSecretPath(event.input, state.descriptor.cwd)) return;
    await this.#append(state, { type: "secret-call", toolCallId: event.toolCallId });
    state.secretCalls.add(event.toolCallId);
  }

  /** A result of such a call, its output left out of its row, recorded once the row is written. */
  async #noteWithheldResult(state: RuntimeSessionState, event: Extract<AdapterEvent, { kind: "result" }>): Promise<void> {
    await this.#append(state, { type: "secret-result", toolCallId: event.toolCallId, eventId: event.eventId });
    state.secretCalls.delete(event.toolCallId);
    state.withheldResults.add(event.eventId);
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

  /**
   * `sample` as the session's content source keeps it: a person's plain message under its event's id, any other sample
   * under one that also names its boundary and its place in the session.
   */
  #sampleItem(state: RuntimeSessionState, sample: AdapterSample): ContentItem {
    const plainUserMessage = sample.source === "user-input" && sample.boundary === "message";
    const id = plainUserMessage ? sample.eventId : `${sample.eventId}:${sample.boundary}:${state.sampleSequence++}`;
    return {
      ref: { source: "host:event", id },
      text: sample.text,
      metadata: {
        owner: sampleOwner(sample.source),
        session: sample.sessionId,
        fields: { eventId: sample.eventId, sampleSource: sample.source, boundary: sample.boundary },
      },
    };
  }

  async acceptSample(sample: AdapterSample): Promise<void> {
    const state = this.#session(sample.sessionId);
    const item = this.#sampleItem(state, sample);
    state.source.append(item);
    // A result stored without its output leaves its text out of the ledger as well; matching still reads it.
    await this.#append(state, { type: "sample", item: state.withheldResults.has(sample.eventId) ? { ...item, text: "" } : item });
    const registry = state.registry;
    // Internal testing: no conversation rule is checked while they are off, a session's own included.
    if (registry === undefined || !conversationRulesEnabled()) return;
    const matches = registry.match(sample, { turn: state.turn });

    if (matches.length === 0) return;
    await this.#deliverMatches(state, sample, registry, matches);
  }

  /** A delivery for each of `matches` on `sample`, recorded and kept for the sample's event. */
  async #deliverMatches(state: RuntimeSessionState, sample: AdapterSample, registry: ConversationRuleRegistry, matches: readonly RuleMatch[]): Promise<void> {
    const deliveries = state.eventDeliveries.get(sample.eventId) ?? [];
    for (const match of matches) {
      registry.markDelivered([match.ruleId], state.turn);
      await this.#append(state, { type: "rule-match", ruleId: match.ruleId, turn: state.turn });
      this.#nativeSession?.onMatch(match, sample, state.turn);
      const findings = [{ ruleId: match.ruleId, mode: match.mode, message: match.action.content }];
      const delivery: AdapterDelivery = {
        sessionId: sample.sessionId,
        requested: match.mode,
        mechanism: ADAPTER_CAPABILITIES[state.descriptor.host].delivery[match.mode],
        content: steerBlock({ kind: "tool", tool: sampleTool(sample.source) }, findings, steerShell(state.descriptor.host)),
        ruleIds: [match.ruleId],
        rules: steerRules(findings, steerOrigin, ruleId => steerTitle(ruleId, undefined, match.title)),
      };
      await this.#append(state, { type: "delivery", delivery });
      deliveries.push(delivery);
    }
    state.eventDeliveries.set(sample.eventId, deliveries);
  }

  /**
   * Internal testing: explicit audit requests, the work behind `rerail scan` and `audit_requests_create`. `version` is
   * the database's `data_version` the control timer read; the session counts as read at it once its queue is claimed
   * from, and a session busy with a request is read again on the next tick.
   */
  #scheduleAuditRequests(sessionId: string, version?: number): void {
    const state = this.#sessions.get(sessionId);
    if (state === undefined || state.closed || state.ownerClaim === undefined) return;
    // Read by the folder as the session spelt it and the repository it was recorded in, the facts its row holds, never
    // through the row: another program can remove a running session's row (a database put back from a copy), and
    // neither the control timer nor the session's events need it. The session's next recorded event puts the row back,
    // and this runtime's claim on the session with it (`#claimAgain`).
    const settings = this.#controls.getWorkspaceSettings(state.descriptor.cwd, this.#repositoryKey(state.checkout)) ?? this.#auditSettings;
    state.steerStrategy = effectiveSteerStrategy(settings);
    state.codeRuleBridge?.updateSettings(settings);
    if (state.auditRequestWork !== undefined) return;
    if (version !== undefined) state.controlsVersion = version;
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
    // Claimed once the evaluator is open, which can take seconds: a claim made at once ran the scan's writes whenever
    // its rules had compiled, each waiting up to 5 s on the daemon's thread for a write lock another process took
    // meanwhile. A claim itself fails at once under such a lock (it reads first), so the scan stays queued until the
    // next commit. The next tick asks again.
    if (state.codeRuleOpening === true) {
      state.controlsVersion = undefined;
      return;
    }
    const request = this.#controls.claim(sessionId, state.ownerClaim);
    if (request === undefined) return;
    // A claim takes the oldest queued request; one queued behind it is announced by no later commit, so the next tick
    // reads the queue again.
    state.controlsVersion = undefined;
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

  /**
   * A session started where a person turned Merge Assist on: scan its checkout for in-flight conflicts in the
   * background (docs/merge-assist-v1-build.md). No boundary waits for it; its notes go out at a later one.
   */
  #scanMergeAssist(state: RuntimeSessionState): void {
    if (state.closed || this.#stopped()) return;
    const running = this.#mergeAssistService(state);
    if (running === undefined) return;
    const { sessionId, host } = state.descriptor;
    void running.openSession(state.checkout.root, host, sessionId).then(() => this.#queueMergeAssistNotes(state, running)).catch((error: unknown) => {
      process.stderr.write(`rerail: Merge Assist failed in session ${sessionId}: ${error instanceof Error ? error.message : String(error)}\n`);
    });
  }

  /**
   * Queue the Merge Assist notes the session has not been handed yet, at a boundary, before its steers are collected:
   * news written outside the session's own background work (a detection or a dismissal from the dashboard, the CLI or
   * another process) reaches the session at its very next boundary. Reads only; never fails the boundary.
   */
  #offerMergeAssistNotes(state: RuntimeSessionState): void {
    try {
      const running = this.#mergeAssistService(state);
      if (running !== undefined) this.#queueMergeAssistNotes(state, running);
    } catch (error) {
      process.stderr.write(`rerail: could not read Merge Assist notes for session ${state.descriptor.sessionId}: ${error instanceof Error ? error.message : String(error)}\n`);
    }
  }

  /**
   * Merge Assist for the session's checkout, or undefined while a person has not turned it on there or the experiment
   * is off (`mergeAssistEnabled` answers off for every repository then). The first time,
   * this runtime starts listening for conflict changes, so a hook handler's late answer reaches its sessions' next
   * boundary.
   */
  #mergeAssistService(state: RuntimeSessionState): MergeAssistRuntime | undefined {
    const repositoryKey = this.#repositoryKey(state.checkout);
    if (repositoryKey === null || !mergeAssistEnabled(this.#catalogs.database, repositoryKey)) return undefined;
    this.#mergeAssist ??= createMergeAssistService({ database: this.#catalogs.database });
    const service = this.#mergeAssist;
    this.#mergeAssistUnlisten ??= service.onConflictsChanged(checkoutPath => {
      for (const session of this.#sessions.values()) {
        if (session.checkout.root === checkoutPath) this.#queueMergeAssistNotes(session, service);
      }
    });
    return service;
  }

  /**
   * The agent edited files: find the checkout's in-flight conflicts again in the background (the service runs it at
   * most every 30 s per checkout) and queue the notes they call for, again once the notes the person's hook command
   * holds are released. Resolves once that is done; never rejects. A note that is ready in time rides the edit's own
   * boundary, else a later one.
   */
  #detectMergeAssistConflicts(state: RuntimeSessionState): Promise<void> {
    if (state.closed || this.#stopped()) return Promise.resolve();
    const running = this.#mergeAssistService(state);
    if (running === undefined) return Promise.resolve();
    const root = state.checkout.root;
    return running.detectAfterEdit(root, state.descriptor.sessionId, state.descriptor.host).then(async ran => {
      if (!ran) return;
      this.#queueMergeAssistNotes(state, running);
      await running.notesReleased(root);
      this.#queueMergeAssistNotes(state, running);
    }).catch((error: unknown) => {
      process.stderr.write(`rerail: Merge Assist failed after an edit in session ${state.descriptor.sessionId}: ${error instanceof Error ? error.message : String(error)}\n`);
    });
  }

  /**
   * The detection an edit's result runs ({@link detectMergeAssistConflicts}), none for a call that edited nothing; and
   * what the result waits for: where the person's hook command may hold the checkout's notes, that detection, at most
   * `mergeAssistWaitMs` from now, so Rerail's note, the person's line and the hook's note ride this result when they are
   * ready in time, else a later boundary. Without a hook command the result waits for nothing.
   */
  #detectAfterEditResult(state: RuntimeSessionState, toolCallId: string): Promise<boolean> | undefined {
    if (!state.mergeAssistEdits.delete(toolCallId)) return undefined;
    const detection = this.#detectMergeAssistConflicts(state);
    try {
      if (this.#mergeAssist?.hookHoldsNotes(state.checkout.root) !== true) return undefined;
    } catch (error) {
      process.stderr.write(`rerail: could not read Merge Assist settings for session ${state.descriptor.sessionId}: ${error instanceof Error ? error.message : String(error)}\n`);
      return undefined;
    }
    return settledWithin(detection, this.#mergeAssistWaitMs);
  }

  /**
   * A Merge Assist note's person's line as it reads when the note is handed over, not when it was queued: the
   * conflict's status line text may have changed since (the conflict hook's status). Never fails the delivery.
   */
  #currentConflictNotice(state: RuntimeSessionState, delivery: AdapterDelivery): AdapterDelivery {
    if (delivery.conflict === undefined || this.#mergeAssist === undefined) return delivery;
    try {
      return { ...delivery, conflict: this.#mergeAssist.conflictNotice(state.checkout.root, delivery.conflict, state.descriptor.host) };
    } catch (error) {
      process.stderr.write(`rerail: could not read a Merge Assist conflict for session ${state.descriptor.sessionId}: ${error instanceof Error ? error.message : String(error)}\n`);
      return delivery;
    }
  }

  /**
   * The session's Merge Assist notes, queued as steers that carry no rule, for its next boundary (or at once to a
   * native host). A note speaks for no source, so no edit makes it stale; it is recorded as given only once a delivery
   * takes it. A session whose evaluator could not open takes none: they stay for one that can deliver them.
   */
  #queueMergeAssistNotes(state: RuntimeSessionState, service: MergeAssistRuntime): void {
    const { sessionId, host } = state.descriptor;
    if (state.closed || this.#stopped() || this.#sessions.get(sessionId) !== state) return;
    if (!state.codeRuleReady && !state.codeRuleOpening) return;
    const checkoutPath = state.checkout.root;
    for (const note of service.pendingNotes(sessionId, checkoutPath, state.mergeAssistNotesInFlight, host)) {
      for (const mark of note.marks) state.mergeAssistNotesInFlight.add(mark);
      const pending: PendingSteer = {
        delivery: mergeAssistDelivery(state.descriptor, note),
        // A note speaks for no source: no edit makes it stale.
        revision: Number.POSITIVE_INFINITY,
        scope: [],
        mergeAssistNote: true,
        settled: delivered => {
          for (const mark of note.marks) state.mergeAssistNotesInFlight.delete(mark);
          if (!delivered) return;
          try {
            service.markNotesDelivered(sessionId, checkoutPath, note.marks);
          } catch (error) {
            process.stderr.write(`rerail: could not record a Merge Assist note as given: ${error instanceof Error ? error.message : String(error)}\n`);
          }
        },
      };
      state.pendingSteers.push(pending);
      if (state.codeRuleReady && this.#deliverBackground !== undefined) void this.#deliverSteer(state, pending, true);
    }
  }

  /**
   * Whether a steer, or a held finding, still speaks for the current revision of every source it names. A steer whose
   * edit was rolled back always does: it is the agent's only word that its edit is gone.
   */
  #isCurrentSteer(state: RuntimeSessionState, pending: Pick<PendingSteer, "revision" | "scope"> & { delivery?: AdapterDelivery }): boolean {
    return !state.closed
      && this.#sessions.get(state.descriptor.sessionId) === state
      && (pending.delivery?.rewound !== undefined
        || (pending.scope.length === 0
          ? pending.revision >= state.codeRuleRevision
          : pending.scope.every(path => (state.codeRuleSourceVersions.get(path) ?? 0) <= pending.revision)));
  }

  /**
   * Rolls the window's tool call back for `pending` when the bridge can (`CodeRuleBridge.rewind`), and makes `pending`
   * the rollback's steer: the notice of what was put back and the steer, tagged with the call, at the rollback's
   * revision. Otherwise `pending` stays the steer it is.
   */
  async #rewind(state: RuntimeSessionState, toolCallId: string, pending: PendingSteer, metadata: SteerMetadata): Promise<void> {
    const bridge = state.codeRuleBridge;
    if (bridge === undefined) return;
    let rewound: RewoundEdit | undefined;
    try {
      rewound = await bridge.rewind(toolCallId, metadata.revision, metadata.scope);
    } catch (error) {
      process.stderr.write(`rerail: could not roll back tool call ${toolCallId}: ${error instanceof Error ? error.message : String(error)}\n`);
      return;
    }
    const paths = rewound === undefined ? [] : [...rewound.restored, ...rewound.deleted];
    if (rewound === undefined || paths.length === 0) return;
    pending.revision = Math.max(pending.revision, rewound.revision);
    pending.delivery = {
      ...pending.delivery,
      content: rewoundSteer(pending.delivery.content, rewound),
      strategy: "rewind",
      toolCallId,
      rewound: paths,
    };
  }

  /**
   * Where a steer waits: for its rollback, in the pending steers, or with the findings held for the digest. A steer a
   * newer revision has overtaken is discarded.
   */
  async #placeSteer(
    state: RuntimeSessionState, pending: PendingSteer, content: string, metadata: SteerMetadata | undefined, isRetired: () => boolean,
  ): Promise<"discarded" | "queued" | PlacedSteer> {
    if (!this.#isCurrentSteer(state, pending)) {
      this.#recordUndelivered(state, pending);
      return "discarded";
    }
    this.#applySubagentStrategy(state, pending, content);
    const window = this.#ownResultWindow(state, metadata);
    // The steer rides that result: its delivery names the result's tool call.
    if (window !== undefined) pending.delivery = { ...pending.delivery, toolCallId: window.toolCallId };
    if (state.steerStrategy === "rewind" && window !== undefined) return await this.#rewindSteer(state, window, pending, metadata!, isRetired);
    if (this.#holdForDigest(state, pending, metadata)) return "queued";
    return this.#pendSteer(state, pending, metadata);
  }

  /**
   * Pends the steer and starts its background delivery in the same step. With a wait between the two, a boundary that
   * collects the pending steers meanwhile takes the steer into its own reply, not the background.
   */
  #pendSteer(state: RuntimeSessionState, pending: PendingSteer, metadata: SteerMetadata | undefined): PlacedSteer {
    state.pendingSteers.push(pending);
    return { boundaryDelivery: this.#deliverInBackground(state, pending, metadata) };
  }

  /** The open result window, when the steer is the check of the very edit whose result is being ingested: the host can put the steer into that result. */
  #ownResultWindow(state: RuntimeSessionState, metadata: SteerMetadata | undefined): ResultWindow | undefined {
    const window = state.resultWindow;
    const own = window !== undefined && metadata?.generatedEdit !== undefined
      && state.codeRuleBridge?.checks(window.toolCallId, metadata.revision, metadata.scope) === true;
    return own ? window : undefined;
  }

  /** Under the `subagent` steer strategy, the steer goes out asking the coding agent to hand the fix to a subagent; under any other, it is left as it is. */
  #applySubagentStrategy(state: RuntimeSessionState, pending: PendingSteer, content: string): void {
    if (state.steerStrategy === "subagent")
      pending.delivery = { ...pending.delivery, content: subagentSteer(content, state.descriptor.host), strategy: state.steerStrategy };
  }

  /** Rolls the edit back, then pends the steer; queued before the result answers, so the steer goes out with the edit it rolled back. */
  async #rewindSteer(
    state: RuntimeSessionState, window: ResultWindow, pending: PendingSteer, metadata: SteerMetadata, isRetired: () => boolean,
  ): Promise<"discarded" | PlacedSteer> {
    const queued = Promise.withResolvers<void>();
    window.steers.push(queued.promise);
    try {
      await this.#rewind(state, window.toolCallId, pending, metadata);
      if (state.closed || isRetired()) return "discarded";
      if (!this.#isCurrentSteer(state, pending)) {
        this.#recordUndelivered(state, pending);
        return "discarded";
      }
      return this.#pendSteer(state, pending, metadata);
    } finally {
      queued.resolve();
    }
  }

  /**
   * Whether the boundary that waited for this edit has already answered, so a steer now would reach the agent at some
   * later step: none waits, or the one that does skipped its wait behind an overdue `steering` wait (`waitsBehind`).
   */
  #boundaryPassed(state: RuntimeSessionState): boolean {
    return state.editCheckWaits === 0 || (state.overdueEditChecks.steering !== undefined && state.yieldWaits === 0);
  }

  /**
   * Experimental (off unless enabled): the boundary that waited for this edit has answered, so a steer now would reach
   * the agent at some later, unrelated step; its findings wait for the digest at the end of the agent's run.
   */
  #holdForDigest(state: RuntimeSessionState, pending: PendingSteer, metadata: SteerMetadata | undefined): boolean {
    const findings = metadata?.generatedEdit;
    if (findings === undefined || !this.#boundaryPassed(state) || !missedSteerDigestEnabled()) return false;
    this.#holdFindings(state, pending, findings);
    return true;
  }

  #holdFindings(state: RuntimeSessionState, pending: PendingSteer, findings: NonNullable<SteerMetadata["generatedEdit"]>): void {
    const scope = new Set(pending.scope);
    for (const violation of findings.violations) {
      const own = violation.path !== undefined && scope.has(violation.path);
      state.missedFindings.push({
        violation, sourceOf: findings.sourceOf, revision: pending.revision, scope: own ? [violation.path!] : pending.scope,
      });
    }
  }

  /** Starts the steer's background delivery, where it has one; returns it when the native boundary waits for it. */
  #deliverInBackground(state: RuntimeSessionState, pending: PendingSteer, metadata: SteerMetadata | undefined): Promise<unknown> | undefined {
    if (!state.codeRuleReady || metadata?.background !== true || this.#deliverBackground === undefined) return undefined;
    const inFlight = this.#deliverSteer(state, pending, true);
    const waitsForNativeBoundary = metadata.generatedEdit !== undefined
      && ADAPTER_CAPABILITIES[state.descriptor.host].toolSurface === "native";
    return waitsForNativeBoundary ? inFlight : undefined;
  }

  /** Marks the sources whose newer revision keeps a steer, or a held finding, from the agent (`undeliveredSources`). */
  #recordUndelivered(state: RuntimeSessionState, pending: Pick<PendingSteer, "revision" | "scope">): void {
    for (const path of pending.scope)
      if ((state.codeRuleSourceVersions.get(path) ?? 0) > pending.revision) state.undeliveredSources.add(path);
  }

  /**
   * Whether `pending` goes to the agent no more: a newer revision has overtaken it (its sources are marked, as
   * `#recordUndelivered` says), or the runtime stops.
   */
  #withheld(state: RuntimeSessionState, pending: PendingSteer): boolean {
    if (!this.#isCurrentSteer(state, pending)) {
      this.#recordUndelivered(state, pending);
      return true;
    }
    return this.#stopped();
  }

  /** Hands `delivery` to a native host at once (`deliverBackground`); false when that failed, which is reported. */
  async #deliveredInBackground(delivery: AdapterDelivery): Promise<boolean> {
    try {
      await this.#deliverBackground!(delivery);
      return true;
    } catch (error) {
      process.stderr.write(`rerail: policy delivery failed: ${error instanceof Error ? error.message : String(error)}\n`);
      return false;
    }
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
    let delivered = false;
    try {
      if (this.#withheld(state, pending)) return undefined;
      pending.delivery = this.#currentConflictNotice(state, pending.delivery);
      // One an answer no client heard took is in the ledger already (`returnUndelivered`).
      if (pending.recorded !== true) await this.#recordedDelivery(pending.delivery);
      // A stop committed elsewhere while the ledger was written withholds the result.
      if (this.#withheld(state, pending)) return undefined;
      if (!native) {
        delivered = true;
        return pending.delivery;
      }
      delivered = await this.#deliveredInBackground(pending.delivery);
      return undefined;
    } finally {
      pending.settled?.(delivered);
    }
  }

  /** A session's start, which happened at `observedAt` when it reaches the runtime later (`IngestOptions.observedAt`). */
  async #ingestSessionStart(event: Extract<AdapterEvent, { kind: "session-start" }>, observedAt: number | undefined): Promise<readonly AdapterDelivery[]> {
    const { session } = event;
    this.#sessions.get(session.sessionId)?.ruleSync?.refresh();
    assertEventHost(session.host);
    if (this.#autostart) autostartDaemon();
    await this.#asOf(session.sessionId, observedAt, () => this.#router(session.host).ingest(event));
    return [];
  }

  #resolveEventSession(event: Exclude<AdapterEvent, { kind: "session-start" }>): {
    active: AdapterSession | undefined;
    descriptor: AdapterSession;
  } {
    const active = this.#sessions.get(event.sessionId)?.descriptor;
    const descriptor = active ?? event.session;
    if (descriptor === undefined) throw new InactiveSessionError(event.sessionId);
    if (descriptor.sessionId !== event.sessionId) throw new Error("Event session descriptor does not match sessionId");
    assertEventHost(descriptor.host);
    if (active !== undefined && event.session !== undefined && active.host !== event.session.host) {
      throw new Error("Event session host does not match active session");
    }
    return { active, descriptor };
  }

  /**
   * The checkout the session starts in, from its folder resolved once here, and recorded: the check and every later
   * use share one resolution, so a folder swapped after a caller's own check gains nothing.
   */
  #startingCheckout(session: AdapterSession): StartingCheckout {
    // A folder that does not exist fails here (ENOENT), before anything is recorded or created for it.
    const cwd = checkoutRealpath(session.cwd);
    const known = recordCheckout(this.#catalogs.database, cwd);
    if (known !== undefined && !known.git) this.#owners.addPlainRoot(known.checkoutPath);
    const start = startingCheckout(this.#owners, cwd);
    // The session's rule coverage reads the checkout's catalog before its evaluator opens. Other sessions' lanes in the
    // checkout share that catalog and read it again only at their own use, so its rules are read here. A catalog not
    // built yet is built off the daemon's thread before the coverage reads it (`#sessionRulesBuilt`).
    if (this.#catalogs.refresh(start.checkoutRoot) !== "unrecorded") this.#catalogs.refreshCatalog(start.checkoutRoot);
    return { root: start.checkoutRoot, folder: start.path, cwd };
  }

  async #restoreRegistry(
    session: AdapterSession,
    ledger: readonly LedgerEntry[],
    checkout: StartingCheckout,
  ): Promise<RestoredRegistry> {
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
    if (state === undefined) throw new InactiveSessionError(sessionId);
    return state;
  }

  async #append(state: RuntimeSessionState, entry: LedgerEntry): Promise<void> {
    await appendJsonLine(state.ledgerPath, storedLedgerEntry(entry), state.ledgerAppendState);
  }
}
