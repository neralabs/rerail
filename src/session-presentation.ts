import { realpath } from "node:fs/promises";
import { isAbsolute, join } from "node:path";
import { z } from "zod";
import { ADAPTER_CAPABILITIES, type AdapterDelivery, type RerailHost } from "./adapter-contract.js";
import { presentationDaemonSchema, presentationResponseSchema, type sessionResponseSchema } from "./api/session-contracts.js";
import { withinCheckout } from "./checkout-control.js";
import { DAEMON_STATE_FILE, processAlive, readDaemonState } from "./daemon-control.js";
import type { DaemonState } from "./daemon.js";
import { readRepositoryId } from "./repository-identity.js";
import type { RepositoryStartState } from "./repository-start.js";
import type { PresentationPresence } from "./schema.js";
import { linkSteers, sessionPageUrl } from "./rerail-steer.js";
import { RERAIL_BRAND_FOREGROUND } from "./terminal-brand.generated.js";
import { defaultRerailHome } from "./rerail-home.js";

export type { PresentationPresence } from "./schema.js";

/**
 * The footer's state (design A1f): on here, not on here (never started or stopped in this repo), not
 * running (no live daemon), or on but missing what it needs to count (an unknown session).
 */
export type PresentationStatus = "on" | "not-on-here" | "not-running" | "unavailable";

export interface PresentationCounts {
  /** Rules on for this session; null when coverage is unknown. */
  rules: number | null;
  /** Steers delivered in this session; null when the history cannot establish a total. */
  steered: number | null;
}

export interface SessionPresentation {
  status: PresentationStatus;
  /** The footer as plain text, e.g. `Rerail ● 26 rules  ·  steered 3×`. */
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

/** `<origin>/#rules` — the rules view of the daemon page. */
export function rulesDashboardUrl(origin: URL): string {
  const url = new URL(origin);
  url.hash = "rules";
  return url.href;
}

const READ_TIMEOUT_MS = 2_000;
const MAX_RESPONSE_BYTES = 1_048_576;
const terminalText = (value: string): string => value.replace(/[\p{Cc}\p{Cf}\p{Zl}\p{Zp}]/gu, "");

const STATUS_MARKS: Record<PresentationStatus, string> = { on: "●", unavailable: "●", "not-on-here": "○", "not-running": "✕" };
/** Between the footer's items; the brand and its mark lead with single spaces. */
const ITEM_SEPARATOR = "  ·  ";

interface PresentationInput {
  status: PresentationStatus;
  brandUrl?: string | null;
  items: SessionPresentation["parts"];
  presence: PresentationPresence;
  repositoryState?: RepositoryStartState | null;
  counts?: PresentationCounts | null;
}

function presentation({ status, brandUrl = null, items, presence, repositoryState = null, counts = null }: PresentationInput): SessionPresentation {
  const parts = [{ text: "Rerail", url: brandUrl }, ...items];
  const text = `Rerail ${STATUS_MARKS[status]} ${items.map(item => item.text).join(ITEM_SEPARATOR)}`;
  return { status, text, url: brandUrl, parts, presence, repositoryState, counts };
}

/** A silent footer: nothing to draw, while the state stays known to explicit reads. */
function silent(presence: PresentationPresence, repositoryState: RepositoryStartState | null): SessionPresentation {
  return { status: "on", text: "", url: null, parts: [], presence, repositoryState, counts: null };
}

/**
 * Only the brand receives color; each label owns its own immutable OSC 8 target.
 * BEL-terminated to match the host TUI, which frames every line with `ESC ] 8 ; ; BEL`.
 */
export function formatTerminalStatus({ status, parts }: SessionPresentation): string {
  const link = (part: SessionPresentation["parts"][number], label: string): string => {
    const target = part.url === null ? null : terminalText(part.url);
    return target ? `\u001b]8;;${target}\u0007${label}\u001b]8;;\u0007` : label;
  };
  const [brand, ...items] = parts;
  if (brand === undefined) return "";
  const name = link(brand, `${RERAIL_BRAND_FOREGROUND}${terminalText(brand.text)}\u001b[39m`);
  return `${name} ${STATUS_MARKS[status]} ${items.map(item => link(item, terminalText(item.text))).join(ITEM_SEPARATOR)}`;
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

/** `Rerail ● 26 rules`, with `steered N×` only once Rerail has steered in this session (A1f). */
function onPresentation(
  origin: URL, { session, hud }: z.infer<typeof sessionResponseSchema>, presence: PresentationPresence, health: string[],
): SessionPresentation {
  const sessionUrl = sessionPageUrl(origin.href, session.host, session.sessionId);
  const ruleNoun = hud.activeRuleCount === 1 ? "rule" : "rules";
  const items: SessionPresentation["parts"] = [
    { text: hud.activeRuleCount === null ? "rules unavailable" : `${hud.activeRuleCount} ${ruleNoun}`, url: rulesDashboardUrl(origin) },
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
 * The dashboard origin people can open for `cwd`'s checkout, resolved as the footer resolves it: the
 * Rerail home daemon's recorded URL when it is loopback. Undefined when there is none, or the daemon is
 * bound somewhere a browser should not be sent (`0.0.0.0`).
 * Passive: reads locator files only and never probes or starts anything, so hooks can afford it per steer.
 */
export async function dashboardOrigin(cwd: string): Promise<string | undefined> {
  try {
    const daemon = await verifyDaemon(cwd);
    return typeof daemon === "string" ? undefined : daemon.origin.href;
  } catch {
    return undefined;
  }
}

/**
 * Deliveries with their steers' `Why?` links made absolute for `cwd`'s dashboard. The runtime keeps
 * links relative; each coding agent's adapter links them as it hands them over.
 */
export async function linkDeliveries(deliveries: readonly AdapterDelivery[], cwd: string | undefined): Promise<AdapterDelivery[]> {
  const origin = cwd === undefined ? undefined : await dashboardOrigin(cwd);
  return deliveries.map(delivery => ({ ...delivery, content: linkSteers(delivery.content, origin) }));
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
 * The rules page of the live daemon serving `cwd`'s checkout, or the reason there is none.
 * Probes before answering: a URL a caller is about to open must point at something running.
 * The page edits the rules of the daemon's repository, so a daemon that cannot show it was
 * started in this checkout's repository is refused rather than handed out. Passive: never starts a daemon.
 */
export async function readRulesDashboard({ cwd }: RulesDashboardRequest): Promise<RulesDashboard> {
  if (typeof cwd !== "string" || !isAbsolute(cwd)) return { url: null, reason: "daemon unavailable" };
  try {
    const daemon = await verifyDaemon(cwd);
    if (typeof daemon === "string") return { url: null, reason: daemon };
    const health = await liveHealth(daemon.origin, daemon.state.pid);
    if (health === undefined) return { url: null, reason: "daemon unavailable" };
    const repositoryId = readRepositoryId(daemon.workspace);
    if (repositoryId === undefined || health.repositoryId !== repositoryId) return { url: null, reason: "daemon serves another repository" };
    return { url: rulesDashboardUrl(daemon.origin), reason: null };
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
  return state === undefined || state === "started"
    ? unavailable("not-running", NOT_RUNNING, presence)
    : unavailable("not-on-here", NOT_ON_HERE, presence, state);
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
    if (response.repositoryState !== "started") return unavailable("not-on-here", NOT_ON_HERE, presence, response.repositoryState);
    if (context.session === null) return unavailable("unavailable", "session unavailable", presence, "started");
    const { session } = context.session;
    // The host may sit below its launch root; the session's canonical cwd stays the identity.
    const root = await realpath(session.cwd);
    if (session.sessionId !== sessionId || session.host !== host || !withinCheckout(root, workspace)) {
      return unavailable("unavailable", "session unavailable", presence, "started");
    }
    // History failure must not discard already verified session counters.
    return onPresentation(origin, context.session, presence, await healthItems(context.session, context.daemon.native.ok, root));
  } catch {
    // The locator named a live process, but nothing answered as the daemon.
    return answered ? unavailable("not-running", NOT_RUNNING, presence) : await withoutDaemon(startState, presence);
  }
}
