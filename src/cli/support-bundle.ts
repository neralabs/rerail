/**
 * `rerail support-bundle`: one JSON file a person attaches to an issue, so a maintainer can diagnose it without seeing
 * their computer, their dashboard or their history. It is private diagnostic context for a report; a share for showing
 * people what Rerail did would be another command, with another audience and other content.
 *
 * What goes in is an allowlist: each field below is named, so a new column or event field stays out until someone adds
 * it here. Prompts, replies, tool input and output, source and rule text are never read into it. Free text that has to
 * stay (a log line, an engine error, a steer) goes through {@link scrubText}, which replaces the values this computer
 * knows, then what looks like a secret. Patterns cannot catch every secret, so they are a backstop: the person reads the
 * file before attaching it. The command only writes a local file; nothing here touches the network.
 */
import { createHash } from "node:crypto";
import { closeSync, existsSync, fstatSync, mkdirSync, openSync, readFileSync, readSync } from "node:fs";
import { replaceFileSync } from "../engine/file-replace.js";
import { arch, homedir, hostname, platform, release, userInfo } from "node:os";
import { dirname, join, resolve } from "node:path";
import { and, asc, count, desc, eq, inArray, sql } from "drizzle-orm";
import { parseArguments } from "../daemon/daemon.js";
import { SqliteAuditReader } from "../engine/audit/audit-store.js";
import type { AuditRecord } from "../engine/audit/audit.js";
import { repositoryIdentityLocation } from "../engine/catalog/repository-identity.js";
import {
  listRepositories,
  lookupCheckout,
  parentCheckouts,
  repositoryHistoryIds,
  type RepositoryListing,
} from "../engine/catalog/repository-registry.js";
import { activeCoreVersion, loadRuleCatalog } from "../engine/catalog/rule-catalog.js";
import { listRepositoryRules } from "../engine/catalog/rule-configuration.js";
import { sensitiveByName } from "../engine/path-boundary.js";
import { defaultRerailDatabasePath, openRerailDatabase, type RerailDatabase } from "../engine/storage/database.js";
import { defaultRerailHome } from "../engine/storage/rerail-home.js";
import { events, policySessionOwners, sessions } from "../engine/storage/schema.js";
import {
  DAEMON_ACCESS_PARAM,
  DAEMON_ACCESS_TOKEN_FILE,
  DAEMON_LOG_FILE,
  DAEMON_STATE_FILE,
  lookupDaemon,
} from "../runtime/daemon-control.js";
import { workspaceStartState } from "../runtime/repository-start.js";
import { defaultStorageRoot } from "../runtime/session-paths.js";
import { SqliteSessionReader, type SessionGaps } from "../runtime/session-store.js";
import { seenCodingAgents } from "../setup/coding-agents.js";

export const SUPPORT_BUNDLE_FORMAT = "rerail.support-bundle/v1";
/** Beside the person's other files, where a browser's file picker finds it; never inside a repository a commit could take it from. */
const DEFAULT_OUTPUT_NAME = "rerail-support-bundle.json";
const ISSUE_URL = "https://github.com/neralabs/rerail/issues/new/choose";
const WITHHELD_PATH = "<withheld: named like a secret>";
const REDACTED = "<redacted>";

// The caps keep the file small enough to read before attaching it. Each section says how much it left out.
const MAX_TIMELINE_ENTRIES = 1000;
const MAX_STEERS = 50;
const MAX_STEER_CHARACTERS = 2000;
const MAX_AUDITS = 50;
const MAX_AUDIT_ENTRIES = 100;
const MAX_LOG_LINES = 200;
const MAX_LOG_LINE_CHARACTERS = 2000;
const MAX_LOG_BYTES = 1024 * 1024;

export interface KnownValue {
  value: string;
  label: string;
  /** Replace it only as a whole word: a user name like `dan` is not in `redundant`. */
  word?: boolean;
  /** Replace it only right after a path separator: a user name like `dev` is also a word in a log line. */
  pathOnly?: boolean;
}

export interface SupportBundleTimelineEntry {
  at: number;
  kind: string;
  tool?: string;
  failed?: boolean;
  ruleIds?: string[];
  count?: number;
  boundary?: string;
  activeRuleCount?: number;
}

export interface SupportBundleSteer {
  mechanism: string;
  requested: string;
  ruleIds: string[];
  text: string;
}

export interface SupportBundleViolation {
  ruleId: string;
  clauseId: string;
  code: string;
  mode: string;
  path: string | null;
  line: number | null;
  column: number | null;
}

export interface SupportBundleFailure {
  ruleId: string | null;
  clauseId: string;
  code: string;
  stage: string | null;
  path: string | null;
  message: string | null;
}

export interface SupportBundleAudit {
  scope: string;
  mode: string;
  status: string;
  startedAt: number;
  finishedAt: number | null;
  /** How many files and rules the audit covered; the files themselves are named only where a violation is. */
  pathCount: number;
  ruleCount: number;
  violations: SupportBundleViolation[];
  violationsOmitted: number;
  failures: SupportBundleFailure[];
  failuresOmitted: number;
  blindFileCount: number;
}

export interface SupportBundleSession {
  host: string;
  startedAt: number;
  endedAt: number | null;
  eventCounts: Record<string, number>;
  gaps: SessionGaps;
  timeline: SupportBundleTimelineEntry[];
  timelineOmitted: number;
  steers: SupportBundleSteer[];
  steersOmitted: number;
  audits: SupportBundleAudit[];
  auditsOmitted: number;
}

export interface SupportBundle {
  format: typeof SUPPORT_BUNDLE_FORMAT;
  createdAt: number;
  leftOut: string;
  rerail: {
    version: string;
    activeVersion: string | null;
    managedInstall: boolean;
    /** Every pack installed on this computer, whichever repositories use it. */
    installedPacks: Array<{ id: string; version: string | null }>;
    skippedPacks: Array<{ package: string | null; version: string | null; reason: string }>;
  };
  platform: { os: string; arch: string; release: string; node: string };
  daemon: {
    status: string;
    version?: string;
    ui?: string;
    uptimeMs?: number;
    engine?: { ok: boolean; version?: string; error?: string };
  };
  codingAgents: string[] | null;
  repository: {
    state: string;
    git: boolean;
    rules: Array<{ id: string; kind: string; mode: string; on: boolean; pack: string | null }>;
    unavailableRules: number;
  };
  session: SupportBundleSession | null;
  daemonLog: string[];
  daemonLogOmittedLines: number;
  /** A section Rerail could not read, and why: on a broken install the reason is often the report. */
  problems: string[];
}

export interface SupportBundleSummary {
  path: string;
  session: { host: string; startedAt: number } | null;
  events: number;
  steers: number;
  violations: number;
  /** Rule ids the person's team chose, which can name a project or a client. */
  ownRules: string[];
  logLines: number;
  problems: string[];
}

export class SupportBundleError extends Error {}

const LEFT_OUT = "Prompts, replies, tool input and output, source code and rule text are not in this file; paths under your home folder and in your Rerail checkouts are replaced, and other paths may remain.";

/** Replaced whoever's they are, in this order: a value can sit inside a longer one (a URL's password inside a line that also holds an email). */
const SECRET_PATTERNS: ReadonlyArray<readonly [RegExp, string]> = [
  [/-----BEGIN [A-Z ]*PRIVATE KEY-----[\s\S]*?-----END [A-Z ]*PRIVATE KEY-----/gu, REDACTED],
  [/\b(?:gh[pousr]_[A-Za-z0-9]{20,}|github_pat_\w{20,}|sk-[\w-]{16,}|sk_(?:live|test)_[A-Za-z0-9]{8,}|AIza[\w-]{35}|AKIA[0-9A-Z]{16}|xox[abeoprs]-[A-Za-z0-9-]{10,}|npm_[A-Za-z0-9]{20,}|eyJ[\w-]+\.[\w-]+\.[\w-]+)/gu, REDACTED],
  // `user:password@host`, or `:password@host` with no user.
  [/:\/\/[^\s/:@]*:[^\s/@]+@/gu, `://${REDACTED}@`],
  // An auth header's credential, whatever its scheme; the `key: value` pattern below then folds `Authorization: Basic <redacted>` into one.
  // Case matters and the credential has a floor, so "a token was expected" and "Basic settings" stay.
  [/\b((?:Bearer|Basic|Digest|Token)\s+)[\w.~+/=-]{8,}/gu, `$1${REDACTED}`],
  [new RegExp(`\\b([\\w-]*(?:token|secret|passw(?:or)?d|pwd|api[_-]?key|access[_-]?key|credential|authorization|${DAEMON_ACCESS_PARAM})[\\w-]*["']?\\s*[:=]\\s*)(?:"[^"]*"|'[^']*'|(?:Bearer|Basic|Digest|Token)\\s+[^\\s"',;&]+|[^\\s"',;&]+)`, "giu"), `$1${REDACTED}`],
  [/[\w.%+-]+@[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)*\.[A-Za-z]{2,}/gu, REDACTED],
  [/\b[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\b/giu, "<id>"],
  // Home directories, whoever's: Windows first (`C:\Users\x`, `C:/Users/x`, JSON-escaped `C:\\Users\\x`), then POSIX
  // and WSL's mount of a Windows home (`/mnt/c/Users/x`), plain or JSON-escaped (`\/home\/x`).
  [/\b[A-Za-z]:(?:\\\\|\\|\/)+Users(?:\\\\|\\|\/)+[^\\/\s"':]+/gu, "~"],
  [/(?:(?:\\\/|\/)mnt(?:\\\/|\/)[a-z])?(?:\\\/|\/)(?:Users|home)(?:\\\/|\/)[^\\/\s"':]+/gu, "~"],
];

function escapeRegExp(text: string): string {
  return text.replace(/[.*+?^${}()|[\]\\]/gu, "\\$&");
}

/**
 * A path as a log or an error can also write it: with the other separator (Git and URLs write `C:/Users/x`), and
 * JSON-escaped inside a logged JSON string (`C:\\Users\\x`, `\/home\/x`).
 */
function pathForms(value: string): string[] {
  if (!/[\\/]/u.test(value)) return [value];
  const forward = value.replaceAll("\\", "/");
  const backward = value.replaceAll("/", "\\");
  return [...new Set([value, forward, backward, backward.replaceAll("\\", "\\\\"), forward.replaceAll("/", "\\/")])];
}

/** Takes what this computer knows out of `text`, longest value first so a path goes before a folder inside it, then what looks like a secret. */
export function scrubText(text: string, { knownValues }: { knownValues: readonly KnownValue[] }): string {
  let scrubbed = text;
  const forms = knownValues.flatMap(literal => pathForms(literal.value).map(value => ({ ...literal, value })));
  for (const { value, label, word, pathOnly } of forms.sort((left, right) => right.value.length - left.value.length)) {
    // A home or checkout of `/`, as a container can have, would take every slash with it.
    if (value.length < 2) continue;
    // Windows paths ignore case, and a host can spell one with another drive letter case than Git does (VS Code's `d:\`).
    const flags = /^[A-Za-z]:[\\/]/u.test(value) ? "giu" : "gu";
    if (pathOnly === true) scrubbed = scrubbed.replace(new RegExp(`(?<=[\\\\/])${escapeRegExp(value)}(?![\\w-])`, flags), () => label);
    else if (word === true) scrubbed = scrubbed.replace(new RegExp(`(?<![\\w-])${escapeRegExp(value)}(?![\\w-])`, flags), () => label);
    else scrubbed = scrubbed.replace(new RegExp(escapeRegExp(value), flags), () => label);
  }
  for (const [pattern, replacement] of SECRET_PATTERNS) scrubbed = scrubbed.replace(pattern, replacement);
  return scrubbed;
}

function scrubValue<T>(value: T, knownValues: readonly KnownValue[]): T {
  if (typeof value === "string") return scrubText(value, { knownValues }) as T;
  if (Array.isArray(value)) return value.map(item => scrubValue(item as unknown, knownValues)) as T;
  if (typeof value !== "object" || value === null) return value;
  return Object.fromEntries(Object.entries(value).map(([key, item]) => [scrubText(key, { knownValues }), scrubValue(item as unknown, knownValues)])) as T;
}

function readQuietly(path: string): string | undefined {
  try {
    return readFileSync(path, "utf8");
  } catch {
    return undefined;
  }
}

/** The capabilities in the Rerail home. A broken state file is still read for what it holds: this runs when things are broken. */
function homeSecrets(home: string): string[] {
  const secrets = [readQuietly(join(home, DAEMON_ACCESS_TOKEN_FILE))?.trim()];
  try {
    const state = JSON.parse(readQuietly(join(home, DAEMON_STATE_FILE)) ?? "{}") as { accessToken?: unknown; nativeToken?: unknown };
    for (const token of [state.accessToken, state.nativeToken]) if (typeof token === "string") secrets.push(token);
  } catch {
    // An unreadable state file has no token to take out.
  }
  return secrets.filter(secret => secret !== undefined && secret !== "") as string[];
}

/** `github.com/acme/app` from a repository key or alias: logs and errors often name a repository by its address alone. */
function repositoryAddresses(key: string): string[] {
  const address = key.replace(/^[a-z]+:/u, "");
  const segments = address.split("/");
  return segments.length >= 2 ? [key, address, segments.slice(-2).join("/")] : [key];
}

function repositoryLiterals(repository: RepositoryListing): KnownValue[] {
  return [
    ...[repository.repositoryKey, ...repository.aliases].flatMap(key => repositoryAddresses(key).map(value => ({ value, label: "<repository>" }))),
    ...repository.checkouts.map(row => ({ value: row.checkoutPath, label: "<checkout>" })),
    ...repository.checkouts.map(row => ({ value: row.cloneId, label: "<repository>" })),
  ];
}

/**
 * Names of this person and this computer, as whole words where long enough not to be an ordinary one (`tim`, `dev`
 * and `bob` are replaced inside paths only). The computer's name goes by its first label too: macOS reports
 * `Name.local` and logs `Name`.
 */
function machineLiterals(home: string): KnownValue[] {
  const names: KnownValue[] = [];
  try {
    const { username } = userInfo();
    if (username !== "") names.push({ value: username, label: "<user>", word: true, ...(username.length < 4 ? { pathOnly: true } : {}) });
  } catch {
    // Some containers run as a user with no name.
  }
  const host = hostname();
  for (const name of new Set([host, host.split(".")[0] ?? host])) if (name.length >= 4) names.push({ value: name, label: "<host>", word: true });
  return [
    ...names,
    ...homeSecrets(home).map(value => ({ value, label: REDACTED })),
    { value: home, label: "<rerail-home>" },
    { value: homedir(), label: "~" },
  ];
}

const SESSION_NOT_FOUND = (sessionId: string): string => `No session ${sessionId} in this repository. A session id is in its dashboard address (sessionId=…).`;

/** The session to report: the one named, or this repository's latest. Never another repository's, even when named: its history is not this report's. */
function selectSession(database: RerailDatabase, historyIds: readonly string[], sessionId: string | undefined): typeof sessions.$inferSelect | undefined {
  const inRepository = inArray(sessions.repositoryId, [...historyIds]);
  const session = sessionId === undefined
    ? database.select().from(sessions).where(inRepository).orderBy(desc(sessions.startedAt)).limit(1).get()
    : database.select().from(sessions).where(and(eq(sessions.sessionId, sessionId), inRepository)).get();
  if (session === undefined && sessionId !== undefined) throw new SupportBundleError(SESSION_NOT_FOUND(sessionId));
  return session;
}

/**
 * The fields of a stored event that may leave it, by kind. They are read with `json_extract`, so a payload's text never
 * reaches this process, and a kind not named here is recorded by its name alone.
 */
function timelineEntry(row: {
  kind: string; at: number; tool: string | null; failed: number | null; ruleIds: string | null; count: number | null; boundary: string | null; activeRuleCount: number | null;
}): SupportBundleTimelineEntry {
  const entry: SupportBundleTimelineEntry = { at: row.at, kind: row.kind };
  if ((row.kind === "call" || row.kind === "result") && row.tool !== null) entry.tool = row.tool;
  if (row.kind === "result" && row.failed === 1) entry.failed = true;
  if (row.kind === "steering-delivered" && row.ruleIds !== null) entry.ruleIds = JSON.parse(row.ruleIds) as string[];
  if (row.kind === "events-dropped" && row.count !== null) entry.count = row.count;
  if ((row.kind === "source-discovery-pending" || row.kind === "code-rules-pending") && row.boundary !== null) entry.boundary = row.boundary;
  if (row.kind === "rule-coverage" && row.activeRuleCount !== null) entry.activeRuleCount = row.activeRuleCount;
  return entry;
}

function readTimeline(database: RerailDatabase, sessionId: string): SupportBundleTimelineEntry[] {
  const field = (path: string) => sql<string | number | null>`json_extract(${events.payloadJson}, ${path})`;
  const rows = database.select({
    kind: events.kind,
    at: events.observedAt,
    tool: field("$.toolName"),
    failed: field("$.failed"),
    ruleIds: field("$.ruleIds"),
    count: field("$.count"),
    boundary: field("$.boundary"),
    activeRuleCount: field("$.activeRuleCount"),
  }).from(events).where(eq(events.sessionId, sessionId)).orderBy(desc(events.sequence)).limit(MAX_TIMELINE_ENTRIES).all();
  return rows.reverse().map(row => timelineEntry(row as Parameters<typeof timelineEntry>[0]));
}

/** The steers as delivered, from the session's ledger: the one thing in it that leaves. Its samples and preimages hold the person's files. */
function readSteers(roots: readonly string[], sessionId: string): { steers: SupportBundleSteer[]; total: number; found: boolean } {
  const directory = createHash("sha256").update(sessionId).digest("hex");
  const ledger = roots.map(root => readQuietly(join(defaultStorageRoot(root), directory, "events.jsonl"))).find(text => text !== undefined);
  if (ledger === undefined) return { steers: [], total: 0, found: false };
  const steers: SupportBundleSteer[] = [];
  for (const line of ledger.split("\n")) {
    if (line === "") continue;
    let entry: { type?: unknown; delivery?: { mechanism: string; requested: string; ruleIds: string[]; content: string } };
    try {
      entry = JSON.parse(line) as typeof entry;
    } catch {
      continue;
    }
    if (entry.type !== "delivery" || entry.delivery === undefined) continue;
    const { mechanism, requested, ruleIds, content } = entry.delivery;
    steers.push({ mechanism, requested, ruleIds, text: content.length > MAX_STEER_CHARACTERS ? `${content.slice(0, MAX_STEER_CHARACTERS)}…` : content });
  }
  return { steers: steers.slice(-MAX_STEERS), total: steers.length, found: true };
}

function reportedPath(path: string | undefined): string | null {
  if (path === undefined) return null;
  return sensitiveByName(path) ? WITHHELD_PATH : path;
}

/** An audit by rule and place. A violation's message, captures and fix can quote the file, so none of them leave. */
function reportedAudit(audit: AuditRecord): SupportBundleAudit {
  return {
    scope: audit.scope,
    mode: audit.mode,
    status: audit.status,
    startedAt: audit.startedAt,
    finishedAt: audit.finishedAt,
    pathCount: audit.paths.length,
    ruleCount: audit.ruleRevisions.length,
    violations: audit.violations.slice(0, MAX_AUDIT_ENTRIES).map(violation => ({
      ruleId: violation.ruleId,
      clauseId: violation.clauseId,
      code: violation.code,
      mode: violation.mode,
      path: reportedPath(violation.path),
      line: violation.span?.startLine ?? null,
      column: violation.span?.startColumn ?? null,
    })),
    violationsOmitted: Math.max(0, audit.violations.length - MAX_AUDIT_ENTRIES),
    failures: audit.failures.slice(0, MAX_AUDIT_ENTRIES).map(failure => ({
      ruleId: failure.ruleId ?? null,
      clauseId: failure.clauseId,
      code: failure.code,
      stage: failure.stage ?? null,
      path: reportedPath(failure.path),
      // An engine error is worth keeping; about a file named like a secret, its place is all that leaves.
      message: failure.path !== undefined && sensitiveByName(failure.path) ? null : failure.message,
    })),
    // The record lists at most 20 failures and counts the rest; the cap here cuts no further.
    failuresOmitted: audit.failureCount - Math.min(audit.failures.length, MAX_AUDIT_ENTRIES),
    blindFileCount: audit.blindFileCount,
  };
}

function readSession(database: RerailDatabase, databasePath: string, session: typeof sessions.$inferSelect, workspaceRoot: string, problems: string[]): SupportBundleSession {
  const eventCounts = Object.fromEntries(database.select({ kind: events.kind, total: count() }).from(events)
    .where(eq(events.sessionId, session.sessionId)).groupBy(events.kind).orderBy(asc(events.kind)).all().map(row => [row.kind, row.total]));
  const eventTotal = Object.values(eventCounts).reduce((sum, total) => sum + total, 0);
  const timeline = readTimeline(database, session.sessionId);
  // The ledger is where the session's hooks ran, which is the checkout's root or a folder the agent started in.
  const { steers, total: steerTotal, found } = readSteers([session.cwd, workspaceRoot], session.sessionId);
  const steered = eventCounts["steering-delivered"] ?? 0;
  if (!found && steered > 0) problems.push(`steers: the session steered ${steered} time${steered === 1 ? "" : "s"} and its ledger was not found, so their text is missing.`);
  const sessionReader = new SqliteSessionReader(databasePath);
  const auditReader = new SqliteAuditReader(databasePath);
  try {
    const { audits, total: auditTotal } = auditReader.list({ sessionId: session.sessionId, limit: MAX_AUDITS });
    return {
      host: session.host,
      startedAt: session.startedAt,
      endedAt: session.endedAt,
      eventCounts,
      gaps: sessionReader.gaps(session.sessionId),
      timeline,
      timelineOmitted: eventTotal - timeline.length,
      steers,
      steersOmitted: steerTotal - steers.length,
      audits: audits.map(reportedAudit),
      auditsOmitted: auditTotal - audits.length,
    };
  } finally {
    sessionReader.close();
    auditReader.close();
  }
}

/** The end of the daemon's log. Only the last megabyte is read, so `omitted` counts what that held past the cap. */
function readLogTail(home: string): { lines: string[]; omitted: number } {
  let fd: number;
  try {
    fd = openSync(join(home, DAEMON_LOG_FILE), "r");
  } catch {
    return { lines: [], omitted: 0 };
  }
  try {
    const { size } = fstatSync(fd);
    const length = Math.min(size, MAX_LOG_BYTES);
    const buffer = Buffer.alloc(length);
    readSync(fd, buffer, 0, length, size - length);
    const all = buffer.toString("utf8").split("\n");
    // A read that starts mid-file starts mid-line.
    if (size > length) all.shift();
    if (all.at(-1) === "") all.pop();
    return {
      lines: all.slice(-MAX_LOG_LINES).map(line => (line.length > MAX_LOG_LINE_CHARACTERS ? `${line.slice(0, MAX_LOG_LINE_CHARACTERS)}…` : line)),
      omitted: Math.max(0, all.length - MAX_LOG_LINES),
    };
  } finally {
    closeSync(fd);
  }
}

async function readDaemon(home: string): Promise<SupportBundle["daemon"]> {
  const lookup = await lookupDaemon(home);
  if (lookup.status !== "running") return { status: lookup.status };
  const { version, ui, uptimeMs, native } = lookup.health;
  return {
    status: lookup.status,
    version,
    ui,
    uptimeMs,
    engine: native.ok ? { ok: true, version: native.kernelVersion } : { ok: false, error: native.error },
  };
}

function emptyRepository(state: string, git: boolean): SupportBundle["repository"] {
  return { state, git, rules: [], unavailableRules: 0 };
}

interface CollectInput {
  repo: string;
  home: string;
  databasePath: string;
  sessionId: string | undefined;
  packageVersion: string;
}

/**
 * Everything but the session is read section by section, and a section that fails says why in `problems`: the bundle
 * matters most when Rerail cannot start, its database is damaged or set-up never finished.
 */
export async function collectSupportBundle({ repo, home, databasePath, sessionId, packageVersion }: CollectInput): Promise<{ bundle: SupportBundle; ownRules: string[] }> {
  const problems: string[] = [];
  const attempt = <T>(section: string, read: () => T, fallback: T): T => {
    try {
      return read();
    } catch (error) {
      if (error instanceof SupportBundleError) throw error;
      problems.push(`${section}: ${error instanceof Error ? error.message : String(error)}`);
      return fallback;
    }
  };

  const location = attempt("repository", () => repositoryIdentityLocation(repo), undefined);
  const workspaceRoot = location?.workspaceRoot ?? repo;
  const state = attempt("start state", () => workspaceStartState(repo, databasePath) as string, "unknown");
  let repository = emptyRepository(state, location?.git ?? false);
  let session: SupportBundleSession | null = null;
  const ownRules: string[] = [];
  const knownValues: KnownValue[] = [...machineLiterals(home), { value: workspaceRoot, label: "<checkout>" }];
  const catalog = attempt("packs", () => loadRuleCatalog(home), undefined);

  if (existsSync(databasePath)) {
    const database = attempt("database", () => openRerailDatabase(databasePath, { readonly: true }), undefined);
    if (database !== undefined) {
      try {
        const repositories = attempt("repositories", () => listRepositories(database), []);
        knownValues.push(...repositories.flatMap(repositoryLiterals));
        knownValues.push(...attempt("session owners", () => database.select({ token: policySessionOwners.token }).from(policySessionOwners).all(), [])
          .map(row => ({ value: row.token, label: REDACTED })));
        const checkout = attempt("checkout", () => lookupCheckout(database, repo), undefined);
        const listing = checkout === undefined ? undefined : repositories.find(candidate => candidate.repositoryKey === checkout.repositoryKey);
        if (checkout !== undefined) {
          repository = attempt("rules", () => {
            if (catalog === undefined) return repository;
            const view = listRepositoryRules(database, catalog, checkout.repositoryKey, {
              checkoutPath: checkout.checkoutPath,
              parentCheckoutPaths: parentCheckouts(database, checkout).map(row => row.checkoutPath),
            });
            ownRules.push(...view.rules.filter(rule => rule.source.packId === null).map(rule => rule.document.rule.id));
            return {
              ...repository,
              rules: view.rules.map(rule => ({ id: rule.document.rule.id, kind: rule.document.kind, mode: rule.mode, on: rule.on, pack: rule.source.packId })),
              unavailableRules: view.unavailable.length,
            };
          }, repository);
        }
        const historyIds = listing === undefined ? [] : repositoryHistoryIds(listing);
        const selected = attempt("session", () => selectSession(database, historyIds, sessionId), undefined);
        if (selected !== undefined) {
          knownValues.push({ value: selected.sessionId, label: "<session>" }, { value: selected.cwd, label: "<checkout>" });
          session = attempt("session", () => readSession(database, databasePath, selected, workspaceRoot, problems), null);
        }
      } finally {
        database.$client.close();
      }
    }
  } else if (sessionId !== undefined) {
    throw new SupportBundleError(SESSION_NOT_FOUND(sessionId));
  }

  const { lines, omitted } = attempt("daemon log", () => readLogTail(home), { lines: [], omitted: 0 });
  const daemon = await readDaemon(home).catch((error: unknown) => {
    problems.push(`daemon: ${error instanceof Error ? error.message : String(error)}`);
    return { status: "unknown" };
  });
  const bundle: SupportBundle = {
    format: SUPPORT_BUNDLE_FORMAT,
    createdAt: Date.now(),
    leftOut: LEFT_OUT,
    rerail: {
      version: packageVersion,
      activeVersion: attempt("active version", () => activeCoreVersion(home), null),
      managedInstall: existsSync(join(home, "current")),
      installedPacks: [...(catalog?.packs.values() ?? [])].map(pack => ({ id: pack.id, version: pack.version })),
      skippedPacks: (catalog?.skipped ?? []).map(pack => ({ package: pack.packageName, version: pack.version, reason: pack.reason })),
    },
    platform: { os: platform(), arch: arch(), release: release(), node: process.versions.node },
    daemon,
    codingAgents: seenCodingAgents(home) ?? null,
    repository,
    session,
    daemonLog: lines,
    daemonLogOmittedLines: omitted,
    problems,
  };
  return { bundle: scrubValue(bundle, knownValues), ownRules: scrubValue(ownRules, knownValues) };
}

function summaryOf(path: string, bundle: SupportBundle, ownRules: readonly string[]): SupportBundleSummary {
  const { session } = bundle;
  return {
    path,
    session: session === null ? null : { host: session.host, startedAt: session.startedAt },
    events: session === null ? 0 : session.timeline.length + session.timelineOmitted,
    steers: session?.steers.length ?? 0,
    violations: session?.audits.reduce((total, audit) => total + audit.violations.length, 0) ?? 0,
    ownRules: [...ownRules],
    logLines: bundle.daemonLog.length,
    problems: bundle.problems,
  };
}

function summaryText(summary: SupportBundleSummary): string {
  return [
    `● Wrote a support bundle to ${summary.path}`,
    summary.session === null
      ? "  It holds this computer's versions and Rerail's state, with no session: none was recorded for this repository."
      : `  It holds versions, Rerail's state and a ${summary.session.host} session: ${summary.events} events by kind and tool name, ${summary.steers} steers, ${summary.violations} violations by rule and line.`,
    `  Also the last ${summary.logLines} lines of the daemon's log. ${LEFT_OUT}`,
    "",
    "  Nothing has been sent anywhere. Read the file before you attach it: secrets and names are taken out by value and",
    "  by pattern, which cannot catch everything. Steer text can quote your code, and file paths and rule ids can name a project.",
    `  Then drag it into your issue: ${ISSUE_URL}`,
    "",
  ].join("\n");
}

function writeBundle(output: string, bundle: SupportBundle): void {
  mkdirSync(dirname(output), { recursive: true, mode: 0o700 });
  replaceFileSync(output, `${JSON.stringify(bundle, null, 2)}\n`, { mode: 0o600 });
}

export interface SupportBundleCommandIo {
  packageVersion: string;
  stdout?: Pick<NodeJS.WriteStream, "write">;
  stderr?: Pick<NodeJS.WriteStream, "write">;
}

/** `rerail support-bundle [--session <id>] [--repo <dir>] [--output <file>] [--json]`. */
export async function runSupportBundleCommand(args: readonly string[], usage: () => string, io: SupportBundleCommandIo): Promise<number> {
  const stdout = io.stdout ?? process.stdout;
  const { flags, options } = parseArguments(
    args,
    { flags: ["--json"], options: ["--repo", "--session", "--output"] },
    argument => `Unknown support-bundle option: ${argument}\n\n${usage()}`,
  );
  const output = resolve(options.get("--output") ?? join(homedir(), DEFAULT_OUTPUT_NAME));
  try {
    const { bundle, ownRules } = await collectSupportBundle({
      repo: resolve(options.get("--repo") ?? process.cwd()),
      home: defaultRerailHome(),
      databasePath: defaultRerailDatabasePath(),
      sessionId: options.get("--session"),
      packageVersion: io.packageVersion,
    });
    writeBundle(output, bundle);
    const summary = summaryOf(output, bundle, ownRules);
    stdout.write(flags.has("--json") ? `${JSON.stringify(summary)}\n` : summaryText(summary));
    return 0;
  } catch (error) {
    if (!(error instanceof SupportBundleError)) throw error;
    (io.stderr ?? process.stderr).write(`● ${error.message}\n`);
    return 1;
  }
}
