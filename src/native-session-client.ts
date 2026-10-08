import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import superjson from "superjson";
import type { AdapterDelivery, AdapterEvent, AdapterSession } from "./adapter-contract.js";
import type { NativeSemanticToolRuntime } from "./native-semantic-tools.js";
import type { ProjectionSpec, RelationSpec, SelectionSpec, WatermarkChange } from "./semantic-types.js";
import { startDaemonForPackage } from "./daemon-control.js";
import { isMainRerailBuild } from "./rerail-build.js";
import { defaultRerailHome } from "./rerail-home.js";
import { RULE_MODES } from "./rule-mode.js";

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
}

export interface NativeSessionClientOptions {
  instance: NativeSessionEndpoint;
  sessionId: string;
  host: "omp" | "pi";
  entries: () => readonly unknown[];
  token: string;
  appendEntry: (customType: string, data: unknown) => void;
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
  deliver: (client: NativeSessionClient, deliveries: readonly AdapterDelivery[]) => void | Promise<void>;
  /** The host's serialized session (re)build; runs `createNativeSession` for the same context. */
  reattach: (context: Context) => Promise<unknown>;
}

export interface NativeSessionInitializationOptions {
  maxInputTokens?: number;
  maxRules?: number;
  rules?: readonly unknown[];
  ruleFiles?: readonly string[];
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
    packageRoot: resolve(dirname(fileURLToPath(import.meta.url)), ".."),
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
    appendEntry: host.appendEntry,
    deliver: host.deliver,
    reattach: () => host.reattach(context),
  });
  try {
    await client.initialize(input);
    await host.close();
    host.activate(client, context, session);
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
  #dropped = 0;
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

  ingest(event: AdapterEvent): Promise<unknown> {
    return this.#request("ingest", { event: superjson.serialize(event), eventEncoding: "superjson" });
  }

  conversationRules(input: unknown): Promise<unknown> {
    return this.#request("cosine", { input });
  }

  semantic(input: unknown): Promise<unknown> {
    return this.#request("semantic", { input });
  }

  /** Best-effort: a daemon that cannot answer holds nothing the host needs released, and the host must always be able to retire this runtime. */
  close(): Promise<unknown> {
    this.#closing = true;
    clearTimeout(this.#timer);
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
    this.#schedulePoll();
  }

  #request(
    operation: "initialize" | "ingest" | "cosine" | "semantic" | "poll" | "close",
    payload: { event?: unknown; eventEncoding?: "superjson"; input?: unknown; entries?: readonly unknown[] } = {},
  ): Promise<unknown> {
    const run = async () => {
      // The breaker owns only the event stream. Explicit operations (tools, reload, reattach)
      // always reach the daemon and carry their own error to the caller: a rejected reload is a
      // fact about that reload, not about the live session. Any success reopens the stream.
      const streamed = operation === "ingest" || operation === "poll";
      if (streamed && this.#outage?.open === true) {
        if (operation === "poll") throw this.#outage.error;
        this.#dropped += 1;
        return undefined;
      }
      let reply: NativeSessionReply;
      try {
        reply = await this.#send(operation, payload);
      } catch (error) {
        // A session still being built reports through its caller; only a live runtime owns an outage.
        if (!streamed || !this.#active) throw error;
        this.#fail(error);
        if (operation === "poll") throw error;
        this.#dropped += 1;
        return undefined;
      }
      if (operation !== "close") this.#recover();
      const committedRevision = this.#active || reply.resultRevisionEntry !== undefined
        ? await this.#deliver(reply)
        : (this.#buffer.push(reply), undefined);
      return reply.resultRevisionEntry === undefined ? reply.result : watermarkReceipt(reply, committedRevision);
    };
    const pending = this.#pending.then(run, run);
    this.#pending = pending;
    return pending;
  }

  async #send(
    operation: "initialize" | "ingest" | "cosine" | "semantic" | "poll" | "close" | "record-steering-delivery",
    payload: { event?: unknown; eventEncoding?: "superjson"; input?: unknown; entries?: readonly unknown[] } = {},
  ): Promise<NativeSessionReply> {
    const { instance, sessionId, host } = this.#options;
    const { entries, ...rest } = payload;
    const branch = operation === "initialize" || operation === "semantic" || operation === "cosine"
      ? entries ?? this.#options.entries()
      : undefined;
    const response = await fetch(new URL("/native/session", instance.url), {
      method: "POST",
      headers: { "content-type": "application/json", authorization: `Bearer ${this.#options.token}` },
      body: JSON.stringify({
        workspaceRoot: instance.workspaceRoot,
        sessionId,
        host,
        operation,
        ...(branch === undefined ? {} : { entries: superjson.serialize(branch), entriesEncoding: "superjson" }),
        ...rest,
      }),
      signal: AbortSignal.timeout(30_000),
    });
    const text = await response.text();
    if (!response.ok) throw new Error(`Rerail native request failed (${response.status}): ${text}`);
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
    || !value.entries.every(hasCustomType)) {
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
