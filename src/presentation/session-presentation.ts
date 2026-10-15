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
import { repositoryContextSchema } from "../api/repository-contracts.js";
import { withinCheckout } from "../engine/catalog/checkout-control.js";
import { DAEMON_STATE_FILE, processAlive, readDaemonState, type DaemonState } from "../runtime/daemon-control.js";
import type { RepositoryStartState } from "../engine/catalog/repository-model.js";
import type { PresentationPresence } from "../engine/storage/schema.js";
import { linkSteers, sessionPageUrl, type SessionNoteRules } from "../engine/code-rule/rerail-steer.js";
import { colourDepth, RERAIL_MARK_TEXT, rerailMark, stateMark, type TerminalEnv } from "./rerail-mark.js";
import { defaultRerailHome } from "../engine/storage/rerail-home.js";

export type { PresentationPresence } from "../engine/storage/schema.js";

/**
 * The footer's state (design A1f): on here, not on here (stopped in this checkout), not set up (started,
 * but the repository has no rules configured yet), not running (no live daemon), or on but missing what
 * it needs to count (an unknown session).
 */
export type PresentationStatus = "on" | "not-on-here" | "not-set-up" | "not-running" | "unavailable";

export interface PresentationCounts {
  /** Rules on for this session; null when coverage is unknown. */
  rules: number | null;
  /** Steers delivered in this session; null when the history cannot establish a total. */
  steered: number | null;
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

const STATUS_MARKS: Record<PresentationStatus, string> = { on: "●", unavailable: "●", "not-on-here": "○", "not-set-up": "○", "not-running": "✕" };
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
 * unknown) or `unavailable` (✕, nothing could be read).
 */
export function fallbackPresentation(reason: "session unavailable" | "unavailable"): SessionPresentation {
  return reason === "unavailable"
    ? unavailable("not-running", reason, "compact")
    : unavailable("unavailable", reason, "compact");
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
async function get<Schema extends z.ZodType>(origin: URL, path: string, schema: Schema): Promise<z.infer<Schema>> {
  const response = await fetch(new URL(path, origin), {
    signal: AbortSignal.timeout(READ_TIMEOUT_MS), redirect: "error",
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

/** `R E R A I L ● 26 rules`, with `steered N×` only once Rerail has steered in this session (A1f). */
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
  if (hud.steeringCount === null) items.push({ text: "steering unavailable", url: sessionUrl });
  else if (hud.steeringCount > 0) items.push({ text: `steered ${hud.steeringCount}×`, url: sessionUrl });
  items.push(...health.map(text => ({ text, url: null })));
  return presentation({
    status: "on", brandUrl: sessionUrl, items, presence, repositoryState: "started",
    counts: { rules: hud.activeRuleCount, steered: hud.steeringCount },
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
 * Deliveries with their steers' `Why?` links made absolute for `cwd`'s dashboard, scoped to its repository. The runtime keeps
 * links relative; each coding agent's adapter links them as it hands them over.
 */
export async function linkDeliveries<Delivery extends AdapterDelivery>(deliveries: readonly Delivery[], cwd: string | undefined): Promise<Delivery[]> {
  const dashboard = cwd === undefined ? undefined : await steerDashboard(cwd);
  return deliveries.map(delivery => ({ ...delivery, content: linkSteers(delivery.content, dashboard) }));
}

/**
 * The dashboard `Why?` links point into for `cwd`: the origin scoped to `cwd`'s repository and checkout
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

/** The repository and checkout the daemon records for the verified checkout; null when it cannot say. */
async function dashboardScope(daemon: VerifiedDaemon): Promise<RulesDashboardScope | null> {
  return get(daemon.origin, `/api/repository?${new URLSearchParams({ checkout: daemon.workspace })}`, repositoryContextSchema)
    .then(context => ({ repositoryKey: context.repository.repositoryKey, checkoutPath: context.checkout?.checkoutPath ?? null }))
    .catch(() => null);
}

/** What is wrong with the Engine or the latest audit of this checkout; nothing when both are fine. */
async function healthItems(
  { audit: run }: z.infer<typeof sessionResponseSchema>, nativeOk: boolean, workspace: string,
): Promise<string[]> {
  const items = nativeOk ? [] : ["engine unavailable"];
  try {
    if (run !== null && await realpath(run.workspaceRoot) !== workspace) throw new Error("Mismatched audit identity");
    if (run?.status === "failed" || (run !== null && run.failureCount > 0)) items.push("audit failed");
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

/** The footer for a session whose host already knows Rerail is off here; no daemon is asked. */
export function notOnHerePresentation(): SessionPresentation {
  return unavailable("not-on-here", NOT_ON_HERE, "compact", "stopped");
}

/**
 * A state file records intent, not liveness: the recorded pid must still exist and the listener must
 * answer `/healthz` as that same pid before any caller is handed a URL.
 */
async function liveHealth(origin: URL, pid: number): Promise<z.infer<typeof healthSchema> | undefined> {
  if (!processAlive(pid)) return undefined;
  try {
    const health = await get(origin, "/healthz", healthSchema);
    return health.pid === pid ? health : undefined;
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
  if (origin.protocol !== "http:" || !["127.0.0.1", "[::1]", "localhost"].includes(origin.hostname)
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
    const health = await liveHealth(daemon.origin, daemon.state.pid);
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
    if (typeof daemon === "string" || !processAlive(daemon.state.pid)) return null;
    const response = await get(daemon.origin, `/api/presentation?${new URLSearchParams({ repository: daemon.workspace, host })}`, presentationResponseSchema);
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
async function withoutDaemon(startState: PresentationRequest["startState"], presence: PresentationPresence): Promise<SessionPresentation> {
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
  return unavailable("not-on-here", NOT_ON_HERE, presence, state);
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

/** Passive session reads: never start a daemon, mutate discovery, or open a writable database. */
export async function readPresentation({ sessionId, host, cwd, onDemand = false, startState }: PresentationRequest): Promise<SessionPresentation> {
  let presence: PresentationPresence = "compact";
  if (!Object.hasOwn(ADAPTER_CAPABILITIES, host) || typeof sessionId !== "string" || !sessionId.trim()
      || typeof cwd !== "string" || !isAbsolute(cwd)) {
    return unavailable("unavailable", "session unavailable", presence);
  }
  let answered = false;
  try {
    // The `/api/presentation` response proves liveness; it must not pay for a second probe.
    const daemon = await verifyDaemon(cwd);
    if (typeof daemon === "string" || !processAlive(daemon.state.pid)) return await withoutDaemon(startState, presence);
    const { workspace, state, origin } = daemon;
    const response = await get(origin, `/api/presentation?${new URLSearchParams({
      repository: workspace, host, sessionId, onDemand: String(onDemand),
    })}`, presentationResponseSchema);
    const context = response.context;
    if (context === undefined || context.daemon.pid !== state.pid) return await withoutDaemon(startState, presence);
    answered = true;
    presence = response.presence;
    if (presence === "silent" && !onDemand) return silent(presence, response.repositoryState);
    // A session in a repository where Rerail is not started is never recorded, so this comes first.
    if (response.repositoryState === "stopped") return unavailable("not-on-here", NOT_ON_HERE, presence, "stopped");
    // A started checkout that is not set up records its sessions, but no rules run there until it is set up.
    if (response.repositoryState === "not-set-up") return notSetUpPresentation(origin, response.scope ?? null, presence);
    if (context.session === null) return unavailable("unavailable", "session unavailable", presence, "started");
    const { session } = context.session;
    // The host may sit below its launch root; the session's canonical cwd stays the identity.
    const root = await realpath(session.cwd);
    if (session.sessionId !== sessionId || session.host !== host || !withinCheckout(root, workspace)) {
      return unavailable("unavailable", "session unavailable", presence, "started");
    }
    // History failure must not discard already verified session counters.
    const health = await healthItems(context.session, context.daemon.native.ok, root);
    return onPresentation(origin, context.session, response.scope ?? null, presence, health);
  } catch {
    // The locator named a live process, but nothing answered as the daemon.
    return answered ? unavailable("not-running", NOT_RUNNING, presence) : await withoutDaemon(startState, presence);
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
