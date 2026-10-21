import { randomUUID } from "node:crypto";
import { PACKAGE_ROOT } from "../engine/package-root.js";
import superjson from "superjson";
import type { AdapterDelivery, AdapterEvent, AdapterSession } from "../engine/adapter-contract.js";
import type { NativeSemanticToolRuntime } from "./native-semantic-tools.js";
import type { ProjectionSpec, RelationSpec, SelectionSpec, WatermarkChange } from "../engine/native/semantic-types.js";
import { startDaemonForPackage } from "../runtime/daemon-control.js";
import { isMainRerailBuild } from "../engine/native/rerail-build.js";
import { defaultRerailHome } from "../engine/storage/rerail-home.js";
import { RULE_MODES } from "../engine/catalog/rule-mode.js";
import { EVENTS_DROPPED_ENTRY_TYPE, type DroppedEvents } from "../runtime/session-state.js";

/** The daemon a client talks to: its URL and the checkout its native sessions address. */
interface NativeSessionEndpoint {
  workspaceRoot: string;
  url: URL;
}

const POLL_INTERVAL_MS = 100;
const POLL_BACKOFF_CEILING_MS = 5_000;
/** Cheap polls cover a blip; from here on the host rebuilds the session, which re-resolves the daemon. */
const REATTACH_AFTER_FAILURES = 3;
/** A daemon that survives this many host rebuilds is not coming back on its own; retrying past it just respawns daemons forever. */
const REBUILD_ATTEMPT_LIMIT = 3;
const REBUILD_BACKOFF_MS = 5_000;
/** How long a close waits for its gap record; the gap keeps its final-count marker in the host session either way. */
const CLOSE_RECORD_WAIT_MS = 5_000;
/** First wait before a failed gap record is tried again; it doubles with each further failure up to the ceiling, and a success resets it. */
const RECORD_BACKOFF_MS = 1_000;
const RECORD_BACKOFF_CEILING_MS = 60_000;
/**
 * How long one request waits for the daemon's answer. The daemon answers within its reply bound, with the reply or
 * with a ticket for an operation it is still working on, so only a daemon that is not answering runs past this.
 */
const REQUEST_TIMEOUT_MS = 30_000;
/**
 * How long one operation may keep a live daemon working before the client gives up on it as an outage, so a session
 * whose daemon never finishes does not queue events without end. The daemon caps its own waits (the source walk, the
 * audits of a turn's edits) at seconds, so only a pathologically slow or stuck operation runs this long; the margin is
 * for a loaded machine. A poll does no work of its own: one still waiting is stuck behind an operation this client
 * already gave up on, so it gets the request timeout, and an outage probe costs the events behind it no more than that.
 */
const OPERATION_BUDGET_MS = 120_000;
/** What a streamed request resolves to when the stream was shut and the event was counted in a gap instead of sent. */
const DROPPED = Symbol("dropped");

/**
 * A daemon fault as the client sees it. While `open`, the event stream (ingest and poll) is
 * shut off so a dead or wedged daemon costs the host one line of stderr, not one error per tool
 * call; a probe re-opens the stream once the backoff for `failures` elapses. `rebuilds` counts only
 * the probes that asked the host for a whole new session, so that expensive escalation runs on its
 * own budget and gives up, while the cheap polling that precedes it is unchanged. `error` stays the
 * fault that shut the stream — it is what classifies the outage — so the last rebuild's own failure
 * is kept beside it, to be named when the budget runs out.
 */
interface Outage {
  error: unknown;
  rebuildError?: unknown;
  failures: number;
  rebuilds: number;
  open: boolean;
}

export interface NativeSessionReply {
  result?: unknown;
  resultRevisionEntry?: number;
  deliveries: AdapterDelivery[];
  entries: Array<{ customType: string; data: unknown }>;
  /** The daemon is still working on the operation; its reply is asked for again with this ticket. */
  pendingReply?: string;
}

export interface NativeSessionClientOptions {
  instance: NativeSessionEndpoint;
  sessionId: string;
  host: "omp" | "pi";
  entries: () => readonly unknown[];
  token: string;
  appendEntry: (customType: string, data: unknown) => void;
  /**
   * Told how many events the open gap holds each time one is dropped, and 0 once the gap closes; the host
   * shows it while it lasts. Its record stays in the session's history.
   */
  reportDropped?: ((count: number) => void) | undefined;
  deliver: (client: NativeSessionClient, deliveries: readonly AdapterDelivery[]) => void | Promise<void>;
  /**
   * Builds a fresh session when this client cannot heal itself: the daemon restarted with a new
   * token, or the workspace binding changed. Omitted, such a fault leaves the stream shut for good.
   */
  reattach?: () => Promise<unknown>;
}

export interface NativeSessionHost<Context> {
  current: () => NativeSessionClient | undefined;
  matchesCurrent: (session: AdapterSession) => boolean;
  close: () => Promise<void>;
  activate: (client: NativeSessionClient, context: Context, session: AdapterSession) => void;
  entries: (context?: Context) => readonly unknown[];
  appendEntry: (customType: string, data: unknown) => void;
  /**
   * The session the host is in right now, when it can say. A closing runtime's final gap marker is host-session history,
   * and the host appends to whichever session is current, so a runtime whose session the host already left writes none.
   */
  liveSessionId?: (() => string | undefined) | undefined;
  reportDropped?: ((count: number) => void) | undefined;
  deliver: (client: NativeSessionClient, deliveries: readonly AdapterDelivery[]) => void | Promise<void>;
  /** The host's serialized session (re)build; runs `createNativeSession` for the same context. */
  reattach: (context: Context) => Promise<unknown>;
}

export interface NativeSessionInitializationOptions {
  maxInputTokens?: number;
  maxRules?: number;
  modelPath?: string;
  databasePath?: string;
  storageRoot?: string;
}

export type NativeSessionRoute = "attached" | "host";

export async function createNativeSession<Context>(
  host: NativeSessionHost<Context>,
  context: Context,
  session: AdapterSession & Pick<NativeSessionClientOptions, "host">,
  rebuild: boolean,
  initialization: NativeSessionInitializationOptions,
): Promise<NativeSessionRoute> {
  const current = host.current();
  // A pi session from a checkout keeps its in-process runtime over that checkout's own store. The main build's
  // pi goes through the daemon like omp: the daemon is the machine's one main instance, and an in-process main
  // runtime would be a second one opening the machine-wide content store.
  if (session.host === "pi" && !isMainRerailBuild()) {
    if (current !== undefined) await host.close();
    return "host";
  }
  const { daemon } = await startDaemonForPackage({
    home: defaultRerailHome(),
    packageRoot: PACKAGE_ROOT,
  }, {
    dev: false,
    port: 0,
    cwd: session.cwd,
  });
  const instance = { workspaceRoot: daemon.health.workspaceRoot, url: new URL(daemon.state.url) };
  const token = daemon.state.nativeToken;
  if (token === undefined) throw new Error("Rerail daemon lacks native session support; run rerail daemon stop and retry.");
  const input = { ...initialization, cwd: session.cwd, rebuild };
  if (current !== undefined && host.matchesCurrent(session) && current.matches(instance, token)) {
    await current.initialize(input, host.entries(context));
    current.setEntries(() => host.entries(context));
    host.activate(current, context, session);
    return "attached";
  }
  const client = new NativeSessionClient({
    instance,
    token,
    sessionId: session.sessionId,
    host: session.host,
    entries: () => host.entries(context),
    appendEntry: (customType, data) => {
      const live = host.liveSessionId?.();
      if (customType === EVENTS_DROPPED_ENTRY_TYPE && live !== undefined && live !== session.sessionId) return;
      host.appendEntry(customType, data);
    },
    reportDropped: host.reportDropped,
    deliver: host.deliver,
    reattach: () => host.reattach(context),
  });
  try {
    await client.initialize(input);
    // The runtime this one replaces in the same session hands over its gaps once it has drained its requests, so a
    // daemon restart's outage is still recorded; another session's gaps keep only their host-session markers.
    const predecessor = current !== undefined && host.matchesCurrent(session) ? current : undefined;
    await host.close();
    const dropped = predecessor?.takeDropped() ?? [];
    host.activate(client, context, session);
    // Recorded before activating, whose delivery may throw: the catch below closes this client, which offers the gaps
    // to the daemon on its way out, where they would otherwise be lost with the predecessor that handed them over.
    client.recordDropped(dropped);
    await client.activate();
  } catch (error) {
    await client.close().catch(() => undefined);
    throw error;
  }
  return "attached";
}

/** Native facade for the machine's Node daemon. */
export class NativeSessionClient implements NativeSemanticToolRuntime {
  readonly #options: NativeSessionClientOptions;
  #pending: Promise<unknown> = Promise.resolve();
  readonly #buffer: NativeSessionReply[] = [];
  #active = false;
  #closing = false;
  #timer: ReturnType<typeof setTimeout> | undefined;
  #outage: Outage | undefined;
  /** Events dropped during the current outage, for its recovery line. */
  #dropped = 0;
  /** The gap the events dropped since the stream last carried one are counted in; closed at recovery or close. */
  #gap: DroppedEvents | undefined;
  /** Closed gaps the daemon has not acknowledged yet, oldest first; every successful streamed request tries to record them. */
  readonly #unrecorded: DroppedEvents[] = [];
  /** The recording in flight, which also takes any gap closed while it runs. */
  #recording: Promise<void> | undefined;
  /** A record failure was reported and no record has succeeded since: the retries after it stay quiet. */
  #recordFailing = false;
  /** Failed gap records in a row, and when the next try may run: a streamed request before then does not retry. */
  #recordFailures = 0;
  #recordRetryAt = 0;
  #initialization: unknown;

  constructor(options: NativeSessionClientOptions) {
    this.#options = options;
  }

  matches(instance: NativeSessionEndpoint, token: string): boolean {
    return instance.url.href === this.#options.instance.url.href && instance.workspaceRoot === this.#options.instance.workspaceRoot && token === this.#options.token;
  }

  setEntries(entries: () => readonly unknown[]): void {
    this.#options.entries = entries;
  }

  async activate(): Promise<void> {
    this.#active = true;
    for (const reply of this.#buffer.splice(0)) await this.#deliver(reply);
    this.#schedulePoll();
  }

  initialize(input?: unknown, entries?: readonly unknown[]): Promise<unknown> {
    this.#initialization = input;
    return this.#request("initialize", { input, ...(entries === undefined ? {} : { entries }) });
  }

  /** Resolves to whether the daemon took the event; false means the shut stream counted it as dropped instead. */
  async ingest(event: AdapterEvent): Promise<boolean> {
    const reply = await this.#request("ingest", { event: superjson.serialize(event), eventEncoding: "superjson" }, event);
    return reply !== DROPPED;
  }

  conversationRules(input: unknown): Promise<unknown> {
    return this.#request("cosine", { input });
  }

  semantic(input: unknown): Promise<unknown> {
    return this.#request("semantic", { input });
  }

  /**
   * Best-effort: a daemon that cannot answer holds nothing the host needs released, and the host must always be able to retire this runtime.
   * Once the requests already queued have drained, the open gap is closed and its record offered before the daemon forgets the session.
   */
  close(): Promise<unknown> {
    this.#closing = true;
    clearTimeout(this.#timer);
    // The footer stops showing an open gap with this runtime, and a retiring runtime reports nothing more to it:
    // the footer belongs to whichever runtime replaces this one.
    if (this.#gap !== undefined) this.#options.reportDropped?.(0);
    return this.#request("close").catch(() => undefined);
  }

  mark(change: WatermarkChange): Promise<unknown> {
    return this.semantic({ action: "mark", change });
  }

  select(spec: SelectionSpec): Promise<unknown> {
    return this.semantic({ action: "select", spec });
  }

  project(selectionId: string, spec: ProjectionSpec): Promise<unknown> {
    return this.semantic({ action: "project", selectionId, spec });
  }

  relate(leftProjectionId: string, rightProjectionId: string, spec: RelationSpec): Promise<unknown> {
    return this.semantic({ action: "relate", leftProjectionId, rightProjectionId, spec });
  }
  async recordSteer(sessionId: string, receiptId: string, ruleIds: readonly string[] = []): Promise<void> {
    if (sessionId !== this.#options.sessionId) throw new Error("native steering receipt session mismatch");
    await this.#send("record-steering-delivery", { input: { receiptId, ruleIds: [...ruleIds] } });
  }

  /**
   * Settles once the daemon has indexed the session's repository and opened its code rules (or failed to), however
   * long that takes; rejects when the daemon cannot say, or once this runtime closes. Not queued behind the session's
   * events, which go on meanwhile.
   */
  async awaitCodeRules(): Promise<void> {
    await this.#send("await-code-rules");
  }

  /**
   * The end of the agent's run. Once the daemon has waited for the session's edit checks, as a turn end does, the
   * steers it still holds, with the digest of the findings that missed their step, are delivered like any reply's.
   * Streamed like a poll: while the stream is shut it carries nothing, and a failure is the outage's to report.
   */
  async yieldDeliveries(): Promise<void> {
    await this.#request("yield");
  }

  /**
   * Closed gaps of this session that were counted elsewhere (by the host while the session was failing
   * to attach, or by the runtime this one replaced), to be recorded in the session's history once the
   * daemon acknowledges them. Whoever closed a gap has already marked it in the host session.
   */
  recordDropped(gaps: readonly DroppedEvents[]): void {
    if (gaps.length === 0) return;
    this.#unrecorded.push(...gaps);
    void this.#recordGaps();
  }

  /** Gives the open gap's marker its count so far, for a host about to leave the session; the gap stays open if the host stays. */
  markGapFinal(): void {
    if (this.#gap !== undefined) this.#options.appendEntry(EVENTS_DROPPED_ENTRY_TYPE, { ...this.#gap, recovered: false });
  }

  /** This runtime's gaps, the open one closed, for the runtime that replaces it in the same session. */
  takeDropped(): DroppedEvents[] {
    this.#closeGap();
    return this.#unrecorded.splice(0);
  }

  /** The single background timer: either the next poll or the outage probe, never both. */
  #schedulePoll(delay = POLL_INTERVAL_MS): void {
    if (this.#closing) return;
    clearTimeout(this.#timer);
    this.#timer = setTimeout(() => this.#poll(), delay);
    this.#timer.unref();
  }

  /** Background delivery; a failure hands over to the outage breaker, which resumes the loop. */
  #poll(): void {
    void this.#request("poll").then(() => this.#schedulePoll(), () => undefined);
  }

  /**
   * First failure of an outage: report once, shut the stream, and arm a probe. A failure while the
   * stream is already shut is a request queued before the fault; it neither logs nor re-arms.
   */
  #fail(error: unknown): void {
    const previous = this.#outage;
    if (previous?.open === true || this.#closing) return;
    if (previous === undefined) {
      process.stderr.write(`Rerail native runtime unavailable; session events are dropped until it recovers: ${errorText(error)}\n`);
    }
    this.#armProbe({ error, failures: (previous?.failures ?? 0) + 1, rebuilds: previous?.rebuilds ?? 0, open: true });
  }

  /**
   * A blip is probed by a cheap poll, or by a fresh initialize when the daemon dropped the session.
   * A persistent outage, or a fault this client can never heal (a restarted daemon issues a new
   * token; a workspace rebinding), goes to the host: it re-resolves or restarts the daemon and
   * either re-initializes this client or retires it for a new one. Rebuilds are budgeted, because
   * each one spawns and repairs a daemon: spending the budget is terminal.
   */
  #armProbe(outage: Outage): void {
    this.#outage = outage;
    clearTimeout(this.#timer);
    if (this.#closing) return;
    const { reattach } = this.#options;
    const rebuild = reattach !== undefined && (unrecoverable(outage.error) || outage.failures >= REATTACH_AFTER_FAILURES);
    if (rebuild && outage.rebuilds >= REBUILD_ATTEMPT_LIMIT) {
      // Terminal, and said out loud so it is not the silent twin of the return below: the host has
      // rebuilt as often as spawning a daemon is worth, so the breaker stops rather than respawning
      // one every few seconds for the life of the session. The stream stays shut, so events keep
      // being dropped and counted; only an explicit operation reaching the daemon re-opens it.
      process.stderr.write(`Rerail could not restart its daemon after ${outage.rebuilds} attempts; session events are dropped until it recovers: ${errorText(outage.rebuildError ?? outage.error)}. Run rerail status, or /rerail-start to retry.\n`);
      return;
    }
    if (!rebuild && unrecoverable(outage.error)) return;
    this.#timer = setTimeout(() => {
      // A poll probe must pass the shut breaker, and it reports through the breaker itself. The
      // other probes are explicit operations: the stream stays shut around them, and their
      // rejection carries its own error, so it has to re-arm here.
      if (!rebuild && !sessionMissing(outage.error)) {
        outage.open = false;
        this.#poll();
        return;
      }
      const probe = rebuild ? reattach() : this.#request("initialize", { input: this.#initialization });
      void probe.then(() => {
        if (this.#outage === outage) this.#recover();
      }, (probeError: unknown) => {
        if (this.#outage !== outage) return;
        this.#armProbe({
          ...outage,
          ...(rebuild ? { rebuildError: probeError, rebuilds: outage.rebuilds + 1 } : {}),
          failures: outage.failures + 1,
        });
      });
    }, rebuild ? rebuildBackoffMs(outage) : pollBackoffMs(outage.failures));
    this.#timer.unref();
  }

  /** Any successful daemon round trip ends the outage, rebuild budget included, and resumes background delivery. */
  #recover(): void {
    if (this.#outage === undefined) return;
    this.#outage = undefined;
    process.stderr.write(`Rerail native runtime recovered; ${this.#dropped} session event(s) were dropped during the outage\n`);
    this.#dropped = 0;
    this.#closeGap();
    if (this.#unrecorded.length > 0 && !this.#closing) void this.#recordGaps();
    this.#schedulePoll();
  }

  /** Ends the open gap: its host-session marker gets the final count, and it waits to be recorded. */
  #closeGap(): void {
    const gap = this.#gap;
    if (gap === undefined) return;
    this.#gap = undefined;
    this.#options.appendEntry(EVENTS_DROPPED_ENTRY_TYPE, { ...gap, recovered: true });
    if (!this.#closing) this.#options.reportDropped?.(0);
    this.#unrecorded.push(gap);
  }

  /** An event the shut stream did not carry: counted in the open gap, which the host session marks as it opens. */
  #drop(event: AdapterEvent): void {
    this.#dropped += 1;
    const opening = this.#gap === undefined;
    this.#gap = countDroppedEvent(this.#gap, event.kind, event.eventId);
    if (opening) this.#options.appendEntry(EVENTS_DROPPED_ENTRY_TYPE, { ...this.#gap, recovered: false });
    if (!this.#closing) this.#options.reportDropped?.(this.#gap.count);
  }

  /**
   * Hands the closed gaps to the daemon, oldest first. A gap it does not acknowledge stays for a later successful
   * streamed request (no sooner than the backoff after the failure), or for the runtime that replaces this one; the gap's id
   * keeps one that was stored before its reply was lost from being stored twice. A gap the daemon refuses outright is dropped,
   * so it cannot hold up the ones behind it; its marker stays in the host session. A closing runtime ignores the backoff and
   * tries once more. Never rejects; a closing runtime tries quietly, like its close.
   */
  #recordGaps(): Promise<void> {
    this.#recording ??= this.#recordUnrecorded().finally(() => {
      this.#recording = undefined;
    });
    return this.#recording;
  }

  async #recordUnrecorded(): Promise<void> {
    if (!this.#closing && Date.now() < this.#recordRetryAt) return;
    for (let gap = this.#unrecorded[0]; gap !== undefined; gap = this.#unrecorded[0]) {
      try {
        await this.#send("record-gap", { input: gap });
      } catch (error) {
        if (permanentRejection(error)) {
          const index = this.#unrecorded.indexOf(gap);
          if (index >= 0) this.#unrecorded.splice(index, 1);
          if (!this.#closing) {
            process.stderr.write(`Rerail daemon refused the record of ${gap.count} dropped session event(s); it is not retried: ${errorText(error)}\n`);
          }
          continue;
        }
        if (!this.#recordFailing && !this.#closing) {
          process.stderr.write(`Rerail could not record ${gap.count} dropped session event(s); it retries after a pause: ${errorText(error)}\n`);
        }
        this.#recordFailing = true;
        this.#recordFailures += 1;
        this.#recordRetryAt = Date.now() + Math.min(RECORD_BACKOFF_MS * 2 ** (this.#recordFailures - 1), RECORD_BACKOFF_CEILING_MS);
        return;
      }
      this.#recordFailing = false;
      this.#recordFailures = 0;
      this.#recordRetryAt = 0;
      const index = this.#unrecorded.indexOf(gap);
      if (index >= 0) this.#unrecorded.splice(index, 1);
    }
  }

  #request(
    operation: "initialize" | "ingest" | "cosine" | "semantic" | "poll" | "yield" | "close",
    payload: { event?: unknown; eventEncoding?: "superjson"; input?: unknown; entries?: readonly unknown[] } = {},
    event?: AdapterEvent,
  ): Promise<unknown> {
    const run = async () => {
      // The breaker owns only the event stream. Explicit operations (tools, reload, reattach)
      // always reach the daemon and carry their own error to the caller: a rejected reload is a
      // fact about that reload, not about the live session. Any success reopens the stream.
      const streamed = operation === "ingest" || operation === "poll" || operation === "yield";
      if (operation === "close") {
        // Events dropped while the queue drained are in the gap too; the daemon accepts its record only before the close.
        // A daemon that does not answer must not double the time the host waits to retire this runtime.
        this.#closeGap();
        let timer: NodeJS.Timeout | undefined;
        await Promise.race([
          this.#recordGaps(),
          new Promise<void>((resolve) => {
            timer = setTimeout(resolve, CLOSE_RECORD_WAIT_MS);
            timer.unref();
          }),
        ]);
        clearTimeout(timer);
      }
      if (streamed && this.#outage?.open === true) {
        if (operation === "poll") throw this.#outage.error;
        if (event !== undefined) this.#drop(event);
        return DROPPED;
      }
      let reply: NativeSessionReply;
      try {
        reply = await this.#send(operation, payload);
      } catch (error) {
        // A daemon older than `yield` refuses it; the steers it holds still arrive by poll.
        if (operation === "yield" && permanentRejection(error)) return DROPPED;
        // A session still being built reports through its caller; only a live runtime owns an outage.
        if (!streamed || !this.#active) throw error;
        this.#fail(error);
        if (operation === "poll") throw error;
        if (event !== undefined) this.#drop(event);
        return DROPPED;
      }
      if (operation !== "close") this.#recover();
      if (streamed && this.#unrecorded.length > 0) void this.#recordGaps();
      const committedRevision = this.#active || reply.resultRevisionEntry !== undefined
        ? await this.#deliver(reply)
        : (this.#buffer.push(reply), undefined);
      return reply.resultRevisionEntry === undefined ? reply.result : watermarkReceipt(reply, committedRevision);
    };
    const pending = this.#pending.then(run, run);
    this.#pending = pending;
    return pending;
  }

  /**
   * One operation's reply. A daemon still working on it answers with a ticket instead, and is asked again with that
   * ticket until the reply is ready: a busy daemon is waited for, never taken for a lost one, for up to the operation's budget.
   */
  async #send(
    operation: "initialize" | "ingest" | "cosine" | "semantic" | "poll" | "yield" | "close" | "record-steering-delivery" | "record-gap" | "await-code-rules",
    payload: { event?: unknown; eventEncoding?: "superjson"; input?: unknown; entries?: readonly unknown[] } = {},
  ): Promise<NativeSessionReply> {
    const { entries, ...rest } = payload;
    const branch = operation === "initialize" || operation === "semantic" || operation === "cosine"
      ? entries ?? this.#options.entries()
      : undefined;
    const startedAt = Date.now();
    // Indexing a repository has no bound of its own: its wait ends when this runtime does.
    const budget = operation === "poll" ? REQUEST_TIMEOUT_MS : operation === "await-code-rules" ? Number.POSITIVE_INFINITY : OPERATION_BUDGET_MS;
    let reply = await this.#post({
      operation,
      ...(branch === undefined ? {} : { entries: superjson.serialize(branch), entriesEncoding: "superjson" }),
      ...rest,
      deferReply: true,
    });
    while (reply.pendingReply !== undefined) {
      if (operation === "await-code-rules" && this.#closing) throw new Error("Rerail runtime closed while its repository was still being indexed");
      if (Date.now() - startedAt >= budget) {
        throw new Error(`Rerail daemon was still working on ${operation} after ${budget / 1000}s`);
      }
      reply = await this.#post({ operation: "await-reply", input: { ticket: reply.pendingReply } });
    }
    return reply;
  }

  async #post(request: Record<string, unknown>): Promise<NativeSessionReply> {
    const { instance, sessionId, host } = this.#options;
    const response = await fetch(new URL("/native/session", instance.url), {
      method: "POST",
      headers: { "content-type": "application/json", authorization: `Bearer ${this.#options.token}` },
      body: JSON.stringify({ workspaceRoot: instance.workspaceRoot, sessionId, host, ...request }),
      signal: AbortSignal.timeout(REQUEST_TIMEOUT_MS),
    });
    const text = await response.text();
    if (!response.ok) throw new NativeRequestError(response.status, text);
    let reply: unknown;
    try {
      reply = JSON.parse(text);
    } catch {
      throw new Error("Rerail native response was not JSON");
    }
    if (!isReply(reply)) throw new Error("Rerail native response has an invalid shape");
    return reply;
  }

  async #deliver(reply: NativeSessionReply): Promise<string | undefined> {
    let committedRevision: string | undefined;
    for (const [index, entry] of reply.entries.entries()) {
      const previousEntries = index === reply.resultRevisionEntry ? [...this.#options.entries()] : undefined;
      this.#options.appendEntry(entry.customType, entry.data);
      if (previousEntries !== undefined) {
        committedRevision = appendedEntryId(previousEntries, this.#options.entries());
      }
    }
    await this.#options.deliver(this, reply.deliveries);
    return committedRevision;
  }
}

function pollBackoffMs(failures: number): number {
  return Math.min(POLL_INTERVAL_MS * 2 ** Math.max(failures - 1, 0), POLL_BACKOFF_CEILING_MS);
}

/**
 * Failed rebuilds are spaced far wider than the polls that precede them, because each one spawns a
 * daemon and repairs its database: the first escalation keeps the poll cadence so a token or
 * workspace fault reaches the host at once, then 5s and 10s, which is room for a daemon that is
 * merely slow to start before the budget runs out.
 */
function rebuildBackoffMs(outage: Outage): number {
  return outage.rebuilds === 0 ? pollBackoffMs(outage.failures) : REBUILD_BACKOFF_MS * 2 ** (outage.rebuilds - 1);
}

/** `gap` with one more dropped event; a new gap when there is none. `eventId` is null when the host had not named the event yet. */
export function countDroppedEvent(gap: DroppedEvents | undefined, kind: string, eventId: string | null, at = Date.now()): DroppedEvents {
  if (gap === undefined) {
    return { id: randomUUID(), count: 1, kinds: { [kind]: 1 }, firstEventId: eventId, lastEventId: eventId, firstDroppedAt: at, lastDroppedAt: at };
  }
  return {
    ...gap,
    count: gap.count + 1,
    kinds: { ...gap.kinds, [kind]: (gap.kinds[kind] ?? 0) + 1 },
    firstEventId: gap.firstEventId ?? eventId,
    lastEventId: eventId ?? gap.lastEventId,
    lastDroppedAt: at,
  };
}

/** A watermark operation answers with its receipt stamped by the revision the host committed. */
function watermarkReceipt(reply: NativeSessionReply, committedRevision: string | undefined): Record<string, unknown> {
  if (committedRevision === undefined) {
    throw new Error("Rerail did not observe the host-assigned watermark revision");
  }
  if (reply.result === null || typeof reply.result !== "object" || Array.isArray(reply.result)) {
    throw new Error("Rerail native watermark receipt has an invalid shape");
  }
  return { ...reply.result, revision: committedRevision };
}

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

/** The daemon answered with a failure status; its message keeps the status and body for the checks that read them. */
class NativeRequestError extends Error {
  readonly status: number;

  constructor(status: number, body: string) {
    super(`Rerail native request failed (${status}): ${body}`);
    this.status = status;
  }
}

/**
 * A refusal no retry changes: the daemon read the request and rejected it (an older daemon without `record-gap`, a payload its
 * schema refuses). It answers every handler failure with 400, so the ones that describe the session's or the store's state at
 * that moment (the session is not initialized or unknown yet, the store is busy) are not refusals; neither is a transport fault.
 */
function permanentRejection(error: unknown): boolean {
  if (!(error instanceof NativeRequestError) || (error.status !== 400 && error.status !== 404 && error.status !== 422)) return false;
  return !sessionMissing(error) && !unrecoverable(error) && !/Unknown session|busy|locked/iu.test(error.message);
}

function sessionMissing(error: unknown): boolean {
  return errorText(error).includes("native session is not initialized");
}

/** Authorization, workspace, and ownership faults never heal on their own; everything else is worth another poll. */
function unrecoverable(error: unknown): boolean {
  const message = errorText(error);
  return message.includes("native-authorization-invalid")
    || message.includes("native-workspace-mismatch")
    || message.includes("native session owner mismatch");
}

function isReply(value: unknown): value is NativeSessionReply {
  if (value === null || typeof value !== "object" || Array.isArray(value)
    || !("deliveries" in value) || !Array.isArray(value.deliveries) || !value.deliveries.every(hasRuleMode)
    || !("entries" in value) || !Array.isArray(value.entries)
    || !value.entries.every(hasCustomType)
    || ("pendingReply" in value && value.pendingReply !== undefined && (typeof value.pendingReply !== "string" || value.pendingReply === ""))) {
    return false;
  }
  if (!("resultRevisionEntry" in value) || value.resultRevisionEntry === undefined) return true;
  return Number.isInteger(value.resultRevisionEntry)
    && typeof value.resultRevisionEntry === "number"
    && value.resultRevisionEntry >= 0
    && value.resultRevisionEntry < value.entries.length;
}

/** A daemon from another protocol version names a requested kind this client would misdeliver. */
function hasRuleMode(value: unknown): boolean {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    && "requested" in value && (RULE_MODES as readonly unknown[]).includes(value.requested);
}

function hasCustomType(value: unknown): value is { customType: string; data?: unknown } {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    && "customType" in value && typeof value.customType === "string";
}

function appendedEntryId(previous: readonly unknown[], current: readonly unknown[]): string | undefined {
  const previousIds = new Set(previous.map(entryId).filter((id): id is string => id !== undefined));
  for (let index = current.length - 1; index >= 0; index -= 1) {
    const id = entryId(current[index]);
    if (id !== undefined && !previousIds.has(id)) return id;
  }
  return undefined;
}

function entryId(value: unknown): string | undefined {
  if (value === null || typeof value !== "object" || Array.isArray(value) || !("id" in value)) return undefined;
  return typeof value.id === "string" && value.id !== "" ? value.id : undefined;
}
