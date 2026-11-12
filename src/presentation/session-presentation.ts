import { realpath } from "node:fs/promises";
import { isAbsolute, join } from "node:path";
import { z } from "zod";
import { ADAPTER_CAPABILITIES, type AdapterDelivery, type AdapterSession, type RerailHost } from "../engine/adapter-contract.js";
import {
  presentationDaemonSchema,
  presentationResponseSchema,
  rulesDashboardUrl,
  type RulesDashboardScope,
  type sessionResponseSchema,
} from "../api/session-contracts.js";
import { withinCheckout } from "../engine/catalog/checkout-control.js";
import { DAEMON_STATE_FILE, daemonAccessHeaders, daemonFetch, ownProcessAlive, readDaemonState, type DaemonState } from "../runtime/daemon-control.js";
import type { RepositoryStartState } from "../engine/catalog/repository-model.js";
import type { PresentationPresence } from "../engine/storage/schema.js";
import { conflictUrl, ruleUrl, sessionPageUrl, type SessionNoteRules } from "../engine/code-rule/rerail-steer.js";
import { colourDepth, RERAIL_MARK_TEXT, rerailMark, stateMark, type TerminalEnv } from "./rerail-mark.js";
import { defaultRerailHome } from "../engine/storage/rerail-home.js";

export type { PresentationPresence } from "../engine/storage/schema.js";

/**
 * The footer's state (design A1f): on here, not on here (stopped in this checkout, or never started; the ambient
 * footer draws nothing then, only reads on demand say so), not set up (started, but the repository has no rules
 * configured yet), not running (no live daemon), starting (on, but this session's start is not recorded yet), or on
 * but missing what it needs to count (a session that does not match).
 */
export type PresentationStatus = "on" | "not-on-here" | "not-set-up" | "not-running" | "starting" | "unavailable";

export interface PresentationCounts {
  /** Rules on for this session; null when coverage is unknown. */
  rules: number | null;
  /** Steers delivered in this session; null when the history cannot establish a total. */
  steered: number | null;
  /** Rules and packs on in the session's checkout that run in Log waiting for the person's approval; null when unknown. */
  awaitingApproval: number | null;
}

export interface SessionPresentation {
  status: PresentationStatus;
  /** The footer as plain text, e.g. `R E R A I L ● 26 rules · steered 3×`. */
  text: string;
  /** This session's dashboard page; null without a verified session. */
  url: string | null;
  /** The brand, then the footer's items in order, each with its own link. */
  parts: { text: string; url: string | null }[];
  presence: PresentationPresence;
  /** Whether Rerail runs in the session's repository, as the daemon reports it; null when no daemon answered. */
  repositoryState: RepositoryStartState | null;
  /** Counts behind an `on` footer; null in every other state. */
  counts: PresentationCounts | null;
}

export interface PresentationRequest {
  sessionId: string;
  host: RerailHost;
  cwd: string;
  onDemand?: boolean;
  /**
   * The checkout's start state as the caller reads it locally, consulted only when no daemon answers
   * (this module never opens SQLite); undefined when the caller cannot say without writing. Without it,
   * or when it cannot say, a missing daemon reads as "not running".
   */
  startState?: () => RepositoryStartState | undefined | Promise<RepositoryStartState | undefined>;
}

export interface RulesDashboardRequest {
  cwd: string;
}

export type RulesDashboard = { url: string; reason: null } | { url: null; reason: string };

const READ_TIMEOUT_MS = 2_000;
const MAX_RESPONSE_BYTES = 1_048_576;
const terminalText = (value: string): string => value.replace(/[\p{Cc}\p{Cf}\p{Zl}\p{Zp}]/gu, "");

const STATUS_MARKS: Record<PresentationStatus, string> = {
  on: "●", starting: "●", unavailable: "●", "not-on-here": "○", "not-set-up": "○", "not-running": "✕",
};
/** Between the footer's items (plan §3): single spaces, so a stacked footer stays short. */
const ITEM_SEPARATOR = " · ";

interface PresentationInput {
  status: PresentationStatus;
  brandUrl?: string | null;
  items: SessionPresentation["parts"];
  presence: PresentationPresence;
  repositoryState?: RepositoryStartState | null;
  counts?: PresentationCounts | null;
}

function presentation({ status, brandUrl = null, items, presence, repositoryState = null, counts = null }: PresentationInput): SessionPresentation {
  const parts = [{ text: RERAIL_MARK_TEXT, url: brandUrl }, ...items];
  const text = `${RERAIL_MARK_TEXT} ${STATUS_MARKS[status]} ${items.map(item => item.text).join(ITEM_SEPARATOR)}`;
  return { status, text, url: brandUrl, parts, presence, repositoryState, counts };
}

/** A silent footer: nothing to draw, while the state stays known to explicit reads. */
function silent(presence: PresentationPresence, repositoryState: RepositoryStartState | null): SessionPresentation {
  return { status: "on", text: "", url: null, parts: [], presence, repositoryState, counts: null };
}

/**
 * The ramp-coloured `R E R A I L` and the state-coloured mark are the only colour; each label owns its own immutable
 * OSC 8 target. BEL-terminated to match the host TUI, which frames every line with `ESC ] 8 ; ; BEL`. `env` decides the
 * colour depth (`COLORTERM`, `NO_COLOR`).
 */
export function formatTerminalStatus({ status, parts }: SessionPresentation, env: TerminalEnv = process.env): string {
  const link = (part: SessionPresentation["parts"][number], label: string): string => {
    const target = part.url === null ? null : terminalText(part.url);
    return target ? `\u001b]8;;${target}\u0007${label}\u001b]8;;\u0007` : label;
  };
  const [brand, ...items] = parts;
  if (brand === undefined) return "";
  const depth = colourDepth(env);
  const mark = stateMark(STATUS_MARKS[status], depth);
  return `${link(brand, rerailMark(depth))} ${mark} ${items.map(item => link(item, terminalText(item.text))).join(ITEM_SEPARATOR)}`;
}

/**
 * A footer drawn without asking any daemon, for hosts whose own read failed: `session unavailable` (●, the session is
 * unknown), or `unavailable` or a Node too old to run Rerail (✕, nothing could be read).
 */
export function fallbackPresentation(reason: "session unavailable" | "unavailable" | `needs Node ${string}`): SessionPresentation {
  return reason === "session unavailable"
    ? unavailable("unavailable", reason, "compact")
    : unavailable("not-running", reason, "compact");
}

function unavailable(
  status: Exclude<PresentationStatus, "on">,
  reason: string,
  presence: PresentationPresence,
  repositoryState: RepositoryStartState | null = null,
): SessionPresentation {
  return presentation({ status, items: [{ text: reason, url: null }], presence, repositoryState });
}

/** Validate responses with the same contracts used by the daemon, without opening a database. */
async function get<Schema extends z.ZodType>(daemon: Pick<VerifiedDaemon, "origin" | "state">, path: string, schema: Schema): Promise<z.infer<Schema>> {
  const response = await daemonFetch(daemon.state)(new URL(path, daemon.origin), {
    headers: daemonAccessHeaders(daemon.state), signal: AbortSignal.timeout(READ_TIMEOUT_MS), redirect: "error",
  });
  if (!response.ok || response.body === null) throw new Error("Rerail read unavailable");
  const reader = response.body.getReader();
  const chunks: Uint8Array[] = [];
  let size = 0;
  try {
    for (;;) {
      const chunk = await reader.read();
      if (chunk.done) break;
      size += chunk.value.byteLength;
      if (size > MAX_RESPONSE_BYTES) throw new Error("Rerail response exceeds read budget");
      chunks.push(chunk.value);
    }
  } finally {
    await reader.cancel();
    reader.releaseLock();
  }
  return schema.parse(JSON.parse(Buffer.concat(chunks, size).toString("utf8")));
}

/** The longest conflict title the footer's `◉ decide:` segment shows. */
const FOOTER_TITLE = 48;

/**
 * Merge Assist's footer segment, linked to the conflict's card: `◉ decide: <title>` for the loudest one waiting on the
 * person, else `◐ <summary>` for the newest open one, with `(+N)` for the others open.
 */
function conflictSegment(dashboard: URL, { decide, conflicts, summary }: NonNullable<z.infer<typeof sessionResponseSchema>["hud"]["mergeAssist"]>): SessionPresentation["parts"][number] {
  if (decide === null) return { text: `◐ ${summary.text}${conflicts > 1 ? ` (+${conflicts - 1})` : ""}`, url: conflictUrl(dashboard.href, summary.id) };
  const title = decide.title.length <= FOOTER_TITLE ? decide.title : `${decide.title.slice(0, FOOTER_TITLE - 1).trimEnd()}…`;
  return { text: `◉ decide: ${title}`, url: conflictUrl(dashboard.href, decide.id) };
}

/**
 * `R E R A I L ● 26 rules · worktree · 2 to approve`, with each rule overlay in force in the checkout after the count
 * (linked to its pack page), what runs in Log waiting for a person's approval (linked to Overview, which lists it),
 * `steered N×` only once Rerail has steered in this session (A1f), and Merge Assist's segment while the checkout has a
 * conflict open above logged (`conflictSegment`).
 */
function onPresentation(
  origin: URL,
  { session, hud }: z.infer<typeof sessionResponseSchema>,
  scope: RulesDashboardScope | null,
  presence: PresentationPresence,
  health: string[],
): SessionPresentation {
  // The session page opens on the repository picker unless the link names the repository, as the rules link does.
  const sessionUrl = sessionPageUrl(rulesDashboardUrl(origin, scope), session.host, session.sessionId);
  const ruleNoun = hud.activeRuleCount === 1 ? "rule" : "rules";
  const items: SessionPresentation["parts"] = [
    { text: hud.activeRuleCount === null ? "rules unavailable" : `${hud.activeRuleCount} ${ruleNoun}`, url: rulesDashboardUrl(origin, scope) },
  ];
  for (const label of hud.overlays ?? []) {
    const packPage = new URL(rulesDashboardUrl(origin, scope));
    packPage.hash = `pack/${encodeURIComponent(label)}`;
    items.push({ text: label, url: packPage.href });
  }
  if ((hud.awaitingApproval ?? 0) > 0) items.push({ text: `${hud.awaitingApproval} to approve`, url: rulesDashboardUrl(origin, scope) });
  if (hud.steeringCount === null) items.push({ text: "steering unavailable", url: sessionUrl });
  else if (hud.steeringCount > 0) items.push({ text: `steered ${hud.steeringCount}×`, url: sessionUrl });
  if (hud.mergeAssist !== undefined) {
    const dashboard = new URL(rulesDashboardUrl(origin, scope));
    dashboard.hash = "";
    items.push(conflictSegment(dashboard, hud.mergeAssist));
  }
  items.push(...health.map(text => ({ text, url: null })));
  return presentation({
    status: "on", brandUrl: sessionUrl, items, presence, repositoryState: "started",
    counts: { rules: hud.activeRuleCount, steered: hud.steeringCount, awaitingApproval: hud.awaitingApproval ?? null },
  });
}

/**
 * The dashboard the session-start note links into for `cwd`: the Rerail home daemon's recorded URL when it is
 * loopback, scoped to `cwd`'s repository and checkout (`?repository=&checkout=`) when the daemon names them, so the
 * session page opens in that repository rather than on the picker. Undefined when there is none, or the daemon is
 * bound somewhere a browser should not be sent (`0.0.0.0`). Passive: never starts anything.
 */
export async function dashboardOrigin(cwd: string): Promise<string | undefined> {
  return steerDashboard(cwd);
}

/**
 * Deliveries with each of their rules linked to its page on `cwd`'s dashboard, scoped to its repository: the link the
 * person's `why?` opens; and the conflict a note surfaces linked to its card on the Merge Assist page (`open`). The
 * runtime records no link; each coding agent's adapter links them as it hands them over, and without a dashboard
 * leaves them unlinked.
 */
export async function linkDeliveries<Delivery extends AdapterDelivery>(deliveries: readonly Delivery[], cwd: string | undefined): Promise<Delivery[]> {
  const dashboard = cwd === undefined ? undefined : await steerDashboard(cwd);
  if (dashboard === undefined) return deliveries.map(delivery => ({ ...delivery }));
  return deliveries.map(delivery => ({
    ...delivery,
    rules: delivery.rules.map(rule => ({ ...rule, url: ruleUrl(dashboard, rule.id) })),
    ...(delivery.conflict === undefined ? {} : { conflict: { ...delivery.conflict, url: conflictUrl(dashboard, delivery.conflict.id) } }),
  }));
}

/**
 * The dashboard a steer's `why?` links point into for `cwd`: the origin scoped to `cwd`'s repository and checkout
 * (`?repository=&checkout=`) when the daemon names them, else the bare origin. Never starts anything.
 */
export async function steerDashboard(cwd: string): Promise<string | undefined> {
  try {
    const daemon = await verifyDaemon(cwd);
    if (typeof daemon === "string") return undefined;
    const url = new URL(rulesDashboardUrl(daemon.origin, await dashboardScope(daemon)));
    url.hash = "";
    return url.href;
  } catch {
    return undefined;
  }
}

/**
 * The part of `repository_context_get` the dashboard link needs. Not the operation's own strict schema: a daemon of
 * another version may answer with more or fewer repository fields, and the link needs none of them.
 */
const dashboardScopeSchema = z.object({
  repository: z.object({ repositoryKey: z.string() }),
  checkout: z.object({ checkoutPath: z.string() }).nullable(),
});

/** The repository and checkout the daemon records for the verified checkout; null when it cannot say. */
async function dashboardScope(daemon: VerifiedDaemon): Promise<RulesDashboardScope | null> {
  return get(daemon, `/api/repository?${new URLSearchParams({ checkout: daemon.workspace })}`, dashboardScopeSchema)
    .then(context => ({ repositoryKey: context.repository.repositoryKey, checkoutPath: context.checkout?.checkoutPath ?? null }))
    .catch(() => null);
}

/**
 * What is wrong with the Engine or the latest audit of this checkout; nothing when both are fine. A failed audit says
 * how many failures it had, those its list leaves out included. Coverage gaps of a completed audit are said apart
 * from a failed one: files every rule failed on (blind files), and rules that ran out of work, which the person can
 * simplify, or keep off generated or vendored files.
 */
async function healthItems(
  { audit: run }: z.infer<typeof sessionResponseSchema>, nativeOk: boolean, workspace: string,
): Promise<string[]> {
  const items = nativeOk ? [] : ["engine unavailable"];
  const counted = (count: number, one: string, many: string) => `${count} ${count === 1 ? one : many}`;
  try {
    if (run !== null && await realpath(run.workspaceRoot) !== workspace) throw new Error("Mismatched audit identity");
    if (run === null) return items;
    // A completed audit may still list files it read only in part; only a failed one is trouble.
    if (run.status === "failed") {
      items.push(run.failureCount > 0 ? `audit failed (${counted(run.failureCount, "failure", "failures")})` : "audit failed");
    } else {
      if (run.blindFileCount > 0) items.push(`${counted(run.blindFileCount, "file", "files")} not checked`);
      if (run.outOfWorkRuleCount > 0) items.push(`${counted(run.outOfWorkRuleCount, "rule", "rules")} ran out of work`);
      if (run.fileWorkStoppedRuleCount > 0) {
        items.push(`${counted(run.fileWorkStoppedRuleCount, "rule", "rules")} stopped by file work`);
      }
    }
  } catch {
    items.push("audit unavailable");
  }
  return items;
}

interface VerifiedDaemon {
  workspace: string;
  state: DaemonState;
  origin: URL;
}

/** `/healthz` answers with the daemon's own identity, so a stale locator cannot borrow another listener. */
const healthSchema = presentationDaemonSchema.extend({ status: z.literal("ok") });

/** The footer in a repository where Rerail is not started, or was stopped. */
const NOT_ON_HERE = "not on here";

/** The footer in a started checkout whose repository has no rule configuration yet, so no rules run. */
const NOT_SET_UP = "not set up";
const SET_UP_HINT = "rerail start";

/** `R E R A I L ○ not set up · rerail start`: the rules link opens this repository's rules page to set it up. */
function notSetUpPresentation(origin: URL, scope: RulesDashboardScope | null, presence: PresentationPresence): SessionPresentation {
  return presentation({
    status: "not-set-up", items: [{ text: NOT_SET_UP, url: rulesDashboardUrl(origin, scope) }, { text: SET_UP_HINT, url: null }],
    presence, repositoryState: "not-set-up",
  });
}

/** The footer in a session that has started where the daemon has not recorded the session's start yet. */
const STARTING = "starting";

/**
 * Where Rerail is stopped, or was never started: on demand, `not on here` and how to start it; the ambient footer
 * draws nothing, since a line in a repository Rerail does not run in only takes space (#298).
 */
function notOnHere(presence: PresentationPresence, onDemand: boolean): SessionPresentation {
  return onDemand ? unavailable("not-on-here", NOT_ON_HERE, presence, "stopped")
    : { status: "not-on-here", text: "", url: null, parts: [], presence, repositoryState: "stopped", counts: null };
}

/**
 * A state file records intent, not liveness: the recorded pid must still exist and the listener must
 * answer `/healthz` as that same pid before any caller is handed a URL.
 */
async function liveHealth(daemon: VerifiedDaemon): Promise<z.infer<typeof healthSchema> | undefined> {
  if (!ownProcessAlive(daemon.state.pid)) return undefined;
  try {
    const health = await get(daemon, "/healthz", healthSchema);
    return health.pid === daemon.state.pid ? health : undefined;
  } catch {
    return undefined;
  }
}

/**
 * The daemon serving `cwd`'s checkout, or why there is none. Re-reads the locator on every call:
 * installed updates and daemon restarts may change the listener. The Rerail home's daemon reads the
 * one database every session records into, so it serves sessions of every repository.
 */
async function verifyDaemon(cwd: string): Promise<VerifiedDaemon | string> {
  const workspace = await realpath(cwd);
  const state = await readDaemonState(join(defaultRerailHome(), DAEMON_STATE_FILE));
  if (state === undefined) return "daemon unavailable";
  const origin = new URL(state.url);
  const secure = origin.protocol === "https:" && state.tlsFingerprint256 !== undefined;
  if ((origin.protocol !== "http:" && !secure) || !["127.0.0.1", "[::1]", "localhost"].includes(origin.hostname)
      || origin.username || origin.password || origin.pathname !== "/" || origin.search || origin.hash) {
    return "daemon unavailable";
  }
  return { workspace, state, origin };
}

/**
 * The rules page of the live daemon serving `cwd`'s checkout, or the reason there is none, scoped to the
 * repository and checkout `cwd` is in when the daemon knows them (else the page opens on its repository picker).
 * Probes before answering: a URL a caller is about to open must point at something running. Passive: never
 * starts a daemon and never records the checkout.
 */
export async function readRulesDashboard({ cwd }: RulesDashboardRequest): Promise<RulesDashboard> {
  if (typeof cwd !== "string" || !isAbsolute(cwd)) return { url: null, reason: "daemon unavailable" };
  try {
    const daemon = await verifyDaemon(cwd);
    if (typeof daemon === "string") return { url: null, reason: daemon };
    const health = await liveHealth(daemon);
    if (health === undefined) return { url: null, reason: "daemon unavailable" };
    return { url: rulesDashboardUrl(daemon.origin, await dashboardScope(daemon)), reason: null };
  } catch {
    return { url: null, reason: "unavailable" };
  }
}

/**
 * Whether Rerail is started in `cwd`'s repository, from the live host daemon, or null when none
 * answers. One request: the recorded pid must be alive and the reply must parse as a presentation.
 * Passive, and free of SQLite, so hooks and hosts that cannot load the binding (OMP runs on Bun)
 * ask this first.
 */
export async function readRepositoryState(cwd: string, host: RerailHost): Promise<RepositoryStartState | null> {
  try {
    const daemon = await verifyDaemon(cwd);
    if (typeof daemon === "string" || !ownProcessAlive(daemon.state.pid)) return null;
    const response = await get(daemon, `/api/presentation?${new URLSearchParams({ repository: daemon.workspace, host })}`, presentationResponseSchema);
    return response.repositoryState;
  } catch {
    return null;
  }
}

const NOT_RUNNING = "not running";

/**
 * No daemon answered. Nothing autostarts where Rerail is off, so that is only "not running" in a started
 * checkout; elsewhere the caller's own reading of the start state says "not on here".
 */
async function withoutDaemon(
  startState: PresentationRequest["startState"], presence: PresentationPresence, onDemand: boolean,
): Promise<SessionPresentation> {
  let state: RepositoryStartState | undefined;
  try {
    state = await startState?.();
  } catch {
    state = undefined;
  }
  if (state === undefined || state === "started") return unavailable("not-running", NOT_RUNNING, presence);
  if (state === "not-set-up") return presentation({
    status: "not-set-up", items: [{ text: NOT_SET_UP, url: null }, { text: SET_UP_HINT, url: null }], presence, repositoryState: state,
  });
  return notOnHere(presence, onDemand);
}

/**
 * The repository state `/rerail` shows without a session: the live daemon's answer, else the caller's local
 * reading. A started checkout with no daemon answering stays null, which `/rerail` reports as not running.
 */
export async function readSummaryState(
  cwd: string, host: RerailHost, startState: NonNullable<PresentationRequest["startState"]>,
): Promise<RepositoryStartState | null> {
  const answered = await readRepositoryState(cwd, host);
  if (answered !== null) return answered;
  try {
    const local = await startState();
    return local === undefined || local === "started" ? null : local;
  } catch {
    return null;
  }
}

function isPresentationRequest(sessionId: unknown, host: RerailHost, cwd: unknown): boolean {
  return Object.hasOwn(ADAPTER_CAPABILITIES, host) && typeof sessionId === "string" && sessionId.trim() !== ""
    && typeof cwd === "string" && isAbsolute(cwd);
}

type PresentationResponse = z.infer<typeof presentationResponseSchema>;
type AnsweredSession = NonNullable<NonNullable<PresentationResponse["context"]>["session"]>;
/** A live daemon of this checkout, and what it answered for the session. */
type LiveResponse = { daemon: VerifiedDaemon; response: PresentationResponse };

/** The daemon's answer for the session; undefined when no live daemon of this checkout answers as itself. */
async function readLiveResponse({ sessionId, host, cwd, onDemand = false }: PresentationRequest): Promise<LiveResponse | undefined> {
  // The `/api/presentation` response proves liveness; it must not pay for a second probe.
  const daemon = await verifyDaemon(cwd);
  if (typeof daemon === "string" || !ownProcessAlive(daemon.state.pid)) return undefined;
  const response = await get(daemon, `/api/presentation?${new URLSearchParams({
    repository: daemon.workspace, host, sessionId, onDemand: String(onDemand),
  })}`, presentationResponseSchema);
  const context = response.context;
  return context === undefined || context.daemon.pid !== daemon.state.pid ? undefined : { daemon, response };
}

/** A recorded session as the footer presents it, once it is known to be this session. */
async function answeredSessionPresentation(
  answered: AnsweredSession, native: boolean, { daemon, response }: LiveResponse, { sessionId, host }: PresentationRequest,
): Promise<SessionPresentation> {
  const { session } = answered;
  // The host may sit below its launch root; the session's canonical cwd stays the identity.
  const root = await realpath(session.cwd);
  if (session.sessionId !== sessionId || session.host !== host || !withinCheckout(root, daemon.workspace)) {
    return unavailable("unavailable", "session unavailable", response.presence, "started");
  }
  // History failure must not discard already verified session counters.
  const health = await healthItems(answered, native, root);
  return onPresentation(daemon.origin, answered, response.scope ?? null, response.presence, health);
}

/** What the daemon's answer comes to for this request. */
async function answeredPresentation(answer: LiveResponse, request: PresentationRequest): Promise<SessionPresentation> {
  const { daemon, response } = answer;
  const { presence } = response;
  if (presence === "silent" && request.onDemand !== true) return silent(presence, response.repositoryState);
  // A session in a repository where Rerail is not started is never recorded, so this comes first.
  if (response.repositoryState === "stopped") return notOnHere(presence, request.onDemand === true);
  // A started checkout that is not set up records its sessions, but no rules run there until it is set up.
  if (response.repositoryState === "not-set-up") return notSetUpPresentation(daemon.origin, response.scope ?? null, presence);
  const context = response.context!;
  // The session-start hook records the session; until it has, Rerail is still starting, not failing (#302).
  if (context.session === null) return unavailable("starting", STARTING, presence, "started");
  return await answeredSessionPresentation(context.session, context.daemon.native.ok, answer, request);
}

/** Passive session reads: never start a daemon, mutate discovery, or open a writable database. */
export async function readPresentation(request: PresentationRequest): Promise<SessionPresentation> {
  const { sessionId, host, cwd, startState } = request;
  const onDemand = request.onDemand === true;
  if (!isPresentationRequest(sessionId, host, cwd)) return unavailable("unavailable", "session unavailable", "compact");
  let presence: PresentationPresence | undefined;
  try {
    const answer = await readLiveResponse(request);
    if (answer === undefined) return await withoutDaemon(startState, "compact", onDemand);
    presence = answer.response.presence;
    return await answeredPresentation(answer, request);
  } catch {
    // The locator named a live process, but nothing answered as the daemon.
    return presence !== undefined ? unavailable("not-running", NOT_RUNNING, presence) : await withoutDaemon(startState, "compact", onDemand);
  }
}

/**
 * What the session-start note says about the session's rules, as the daemon presents them (hosts that cannot open the
 * database). Read on demand: the note is the agent's, so the person's footer preference (silent) must not hide that the
 * repository is not set up.
 */
export async function presentedSessionNoteRules(session: AdapterSession): Promise<SessionNoteRules> {
  const presentation = await readPresentation({ sessionId: session.sessionId, host: session.host, cwd: session.cwd, onDemand: true });
  return presentation.status === "not-set-up" ? { setUp: false } : { setUp: true, ruleCount: presentation.counts?.rules ?? null };
}
