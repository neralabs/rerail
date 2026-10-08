import { realpath } from "node:fs/promises";
import { isAbsolute, join } from "node:path";
import { z } from "zod";
import { ADAPTER_CAPABILITIES, type RerailHost } from "./adapter-contract.js";
import { presentationDaemonSchema, presentationResponseSchema, type sessionResponseSchema } from "./api/session-contracts.js";
import { withinCheckout } from "./checkout-control.js";
import { DAEMON_STATE_FILE, processAlive, readDaemonState } from "./daemon-control.js";
import type { DaemonState } from "./daemon.js";
import { readRepositoryId } from "./repository-identity.js";
import type { RepositoryStartState } from "./repository-start.js";
import type { PresentationPresence } from "./schema.js";
import { RERAIL_BRAND_FOREGROUND } from "./terminal-brand.generated.js";
import { defaultRerailHome } from "./rerail-home.js";

export type { PresentationPresence } from "./schema.js";

export interface SessionPresentation {
  text: string;
  url: string | null;
  parts: { text: string; url: string | null }[];
  presence: PresentationPresence;
  /** Whether Rerail runs in the session's repository, as the daemon reports it; null when no daemon answered. */
  repositoryState: RepositoryStartState | null;
}

export interface PresentationRequest {
  sessionId: string;
  host: RerailHost;
  cwd: string;
  onDemand?: boolean;
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

function presentation(
  parts: SessionPresentation["parts"],
  presence: PresentationPresence,
  repositoryState: RepositoryStartState | null = null,
): SessionPresentation {
  return { text: parts.map(part => part.text).join(" | "), url: parts[0]?.url ?? null, parts, presence, repositoryState };
}

/**
 * Only the brand receives color; each label owns its own immutable OSC 8 target.
 * BEL-terminated to match the host TUI, which frames every line with `ESC ] 8 ; ; BEL`.
 */
export function formatTerminalStatus({ parts }: SessionPresentation): string {
  return parts.map((part, index) => {
    const text = terminalText(part.text);
    const label = index === 0 ? `${RERAIL_BRAND_FOREGROUND}${text}\u001b[39m` : text;
    const target = part.url === null ? null : terminalText(part.url);
    return target ? `\u001b]8;;${target}\u0007${label}\u001b]8;;\u0007` : label;
  }).join(" | ");
}

function unavailable(reason: string, presence: PresentationPresence, repositoryState: RepositoryStartState | null = null): SessionPresentation {
  return presentation([{ text: "Rerail", url: null }, { text: reason, url: null }], presence, repositoryState);
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

function sessionParts(origin: URL, { session, hud }: z.infer<typeof sessionResponseSchema>): SessionPresentation["parts"] {
  const sessionLink = new URL(origin);
  sessionLink.searchParams.set("sessionId", session.sessionId);
  sessionLink.searchParams.set("host", session.host);
  sessionLink.hash = "sessions";
  const rulesUrl = rulesDashboardUrl(origin);
  const ruleNoun = hud.activeRuleCount === 1 ? "rule" : "rules";
  return [
    { text: "Rerail", url: sessionLink.href },
    { text: hud.activeRuleCount === null ? "rules unavailable" : `${hud.activeRuleCount} ${ruleNoun}`, url: rulesUrl },
    { text: hud.steeringCount === null ? "steering unavailable" : `steered ${hud.steeringCount}×`, url: sessionLink.href },
  ];
}

async function appendAudit(
  parts: SessionPresentation["parts"], { audit: audit }: z.infer<typeof sessionResponseSchema>,
  nativeOk: boolean, workspace: string, onDemand: boolean,
): Promise<void> {
  try {
    if (audit !== null && await realpath(audit.workspaceRoot) !== workspace) throw new Error("Mismatched evaluation identity");
    if (onDemand) {
      const evaluation = audit === null ? "not evaluated"
        : `${audit.scope}: ${audit.status} | ${audit.violationCount} findings | ${audit.failureCount} failures`;
      parts.push({ text: `${nativeOk ? "healthy" : "kernel unavailable"} | ${evaluation}`, url: null });
      return;
    }
    if (!nativeOk) parts.push({ text: "kernel unavailable", url: null });
    if (audit?.status === "running") parts.push({ text: "checking…", url: null });
    else if (audit?.status === "failed" || (audit !== null && audit.failureCount > 0)) parts.push({ text: "evaluation failed", url: null });
  } catch {
    parts.push({ text: "evaluation unavailable", url: null });
  }
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
  return unavailable(NOT_ON_HERE, "compact");
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

/** Passive session reads: never start a daemon, mutate discovery, or open a writable database. */
export async function readPresentation({ sessionId, host, cwd, onDemand = false }: PresentationRequest): Promise<SessionPresentation> {
  let presence: PresentationPresence = "compact";
  if (!Object.hasOwn(ADAPTER_CAPABILITIES, host) || typeof sessionId !== "string" || !sessionId.trim()
      || typeof cwd !== "string" || !isAbsolute(cwd)) {
    return unavailable("session unavailable", presence);
  }
  try {
    // The `/api/presentation` response proves liveness; it must not pay for a second probe.
    const daemon = await verifyDaemon(cwd);
    if (typeof daemon === "string") return unavailable(daemon, presence);
    const { workspace, state, origin } = daemon;
    const response = await get(origin, `/api/presentation?${new URLSearchParams({
      repository: workspace, host, sessionId, onDemand: String(onDemand),
    })}`, presentationResponseSchema);
    const context = response.context;
    if (context === undefined || context.daemon.pid !== state.pid) return unavailable("daemon unavailable", presence);
    presence = response.presence;
    if (presence === "silent" && !onDemand) return presentation([], presence, response.repositoryState);
    // A session in a repository where Rerail is not started is never recorded, so this comes first.
    if (response.repositoryState !== "started") return unavailable(NOT_ON_HERE, presence, response.repositoryState);
    if (context.session === null) return unavailable("session unavailable", presence, "started");
    const { session } = context.session;
    // The host may sit below its launch root; the session's canonical cwd stays the identity.
    const root = await realpath(session.cwd);
    if (session.sessionId !== sessionId || session.host !== host || !withinCheckout(root, workspace)) {
      return unavailable("session unavailable", presence, "started");
    }
    const parts = sessionParts(origin, context.session);
    // History failure must not discard already verified session counters.
    await appendAudit(parts, context.session, context.daemon.native.ok, root, onDemand);
    return presentation(parts, presence, "started");
  } catch {
    return unavailable("unavailable", presence);
  }
}
