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
import { closeSync, existsSync, fstatSync, mkdirSync, openSync, readFileSync, readSync, renameSync, rmSync, writeFileSync } from "node:fs";
import { arch, homedir, hostname, platform, release, userInfo } from "node:os";
import { dirname, join, resolve } from "node:path";
import { and, asc, count, desc, eq, inArray, sql } from "drizzle-orm";
import { parseArguments } from "../daemon/daemon.js";
import { SqliteAuditReader } from "../engine/audit/audit-store.js";
import type { AuditRecord } from "../engine/audit/audit.js";
import { repositoryIdentityLocation } from "../engine/catalog/repository-identity.js";
import type { CheckoutContext, RuleCatalog } from "../engine/catalog/repository-model.js";
import {
  listRepositories,
  lookupCheckout,
  parentCheckouts,
  repositoryHistoryIds,
  type RepositoryListing,
} from "../engine/catalog/repository-registry.js";
import { activeCoreVersion, loadRuleCatalog } from "../engine/catalog/rule-catalog.js";
import { listRepositoryRules } from "../engine/catalog/rule-configuration.js";
import {
  contentStoreOverride,
  isInMainContentStoreDirectory,
  mainContentStoreDirectory,
  type RerailBuild,
  resolveRerailBuild,
} from "../engine/native/rerail-build.js";
import { PACKAGE_ROOT } from "../engine/package-root.js";
import { sensitiveByName } from "../engine/path-boundary.js";
import { defaultRerailDatabasePath, openRerailDatabase, type RerailDatabase } from "../engine/storage/database.js";
import { defaultRerailHome } from "../engine/storage/rerail-home.js";
import { events, policySessionOwners, sessions } from "../engine/storage/schema.js";
import {
  DAEMON_ACCESS_PARAM,
  DAEMON_ACCESS_TOKEN_FILE,
  DAEMON_LOG_FILE,
  DAEMON_STATE_FILE,
  type DaemonLookup,
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
/**
 * How long the bundle waits for a daemon that is alive but missed its first 2 s health probe, as one does while an
 * audit or a long garbage collection holds its one event loop.
 */
const BUSY_DAEMON_WAIT_MS = 10_000;

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
    /** How long the daemon took to answer its health probe, a first probe it missed included. */
    answeredInMs?: number;
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

/** One spelling of a known value ({@link pathForms}), as the scrub replaces it. */
interface KnownForm extends KnownValue {
  /** How it is found when not as it is spelled: a user name only as a whole word, or only inside a path. */
  pattern: RegExp | undefined;
  /** Its place in the order forms are replaced in: longest first, so a path goes before a folder inside it. */
  rank: number;
}

function knownPattern({ value, word, pathOnly }: KnownValue): RegExp | undefined {
  if (pathOnly === true) return new RegExp(`(?<=[\\\\/])${escapeRegExp(value)}(?![\\w-])`, "gu");
  if (word === true) return new RegExp(`(?<![\\w-])${escapeRegExp(value)}(?![\\w-])`, "gu");
  return undefined;
}

function knownForms(knownValues: readonly KnownValue[]): KnownForm[] {
  return knownValues
    .flatMap(literal => pathForms(literal.value).map(value => ({ ...literal, value })))
    // A home or checkout of `/`, as a container can have, would take every slash with it.
    .filter(({ value }) => value.length >= 2)
    .sort((left, right) => right.value.length - left.value.length)
    .map((form, rank) => ({ ...form, pattern: knownPattern(form), rank }));
}

function replaceForm(text: string, { value, label, pattern }: KnownForm): string {
  return pattern === undefined ? text.replaceAll(value, () => label) : text.replace(pattern, () => label);
}

/**
 * The forms a replacement can bring into a text that did not hold them. A replacement leaves the text's own characters
 * and the characters of labels, which are never empty, and a label's inner characters touch only labels' characters.
 * So a form that a replacement brings in holds the first or last character of a label, or is made of labels'
 * characters alone: a user or host name at most, or a short Windows path (`RUNNER~1`).
 */
function formsLabelsCanMake(forms: readonly KnownForm[]): KnownForm[] {
  const labels = [...new Set(forms.map(form => form.label))];
  if (labels.includes("")) return [...forms];
  const edges = new Set(labels.map(label => [...label]).flatMap(characters => [characters[0], characters.at(-1)]));
  const inLabels = new Set(labels.join(""));
  return forms.filter(({ value }) => [...value].some(character => edges.has(character)) || [...value].every(character => inLabels.has(character)));
}

/** The forms that `found` starts with, itself included. */
function formsStarting(found: string, byValue: ReadonlyMap<string, readonly KnownForm[]>): KnownForm[] {
  const starting: KnownForm[] = [];
  for (let length = 1; length <= found.length; length += 1) starting.push(...(byValue.get(found.slice(0, length)) ?? []));
  return starting;
}

/** The place after the character at `index`: a match starts on a character, and one outside the BMP is two units. */
function nextPlace(text: string, index: number): number {
  return index + ((text.codePointAt(index) ?? 0) > 0xffff ? 2 : 1);
}

/**
 * The forms a text holds, in the order they are replaced in. One pattern of every form, longest first, is tried at each
 * place in the text, inside a form it found too, and finds the longest form there; the shorter forms found at the same
 * place are the ones that form starts with. Forms a replacement can bring in ({@link formsLabelsCanMake}) are always in.
 */
function formFinder(forms: readonly KnownForm[]): (text: string) => KnownForm[] {
  const byValue = Map.groupBy(forms, form => form.value);
  if (byValue.size === 0) return () => [];
  const anywhere = new RegExp([...byValue.keys()].map(escapeRegExp).join("|"), "gu");
  const always = formsLabelsCanMake(forms);
  const startingWith = new Map<string, KnownForm[]>();
  return text => {
    const found = new Set(always);
    anywhere.lastIndex = 0;
    for (let match = anywhere.exec(text); match !== null; match = anywhere.exec(text)) {
      const starting = startingWith.get(match[0]) ?? formsStarting(match[0], byValue);
      startingWith.set(match[0], starting);
      for (const form of starting) found.add(form);
      anywhere.lastIndex = nextPlace(text, match.index);
    }
    return [...found].sort((left, right) => left.rank - right.rank);
  };
}

/**
 * The scrub of `knownValues`, built once for every text it is given. It replaces as a scrub of one value at a time
 * does: each form of each value in turn, longest first, over the whole text, so a path goes before a folder inside it
 * and a value that overlaps another still goes. But a text gets only the forms it holds, which one pattern finds in one
 * read: a bundle holds thousands of texts, and a database can record hundreds of checkouts. Then what looks like a
 * secret goes.
 */
function scrubFor(knownValues: readonly KnownValue[]): (text: string) => string {
  const formsIn = formFinder(knownForms(knownValues));
  return text => {
    let scrubbed = text;
    for (const form of formsIn(text)) scrubbed = replaceForm(scrubbed, form);
    for (const [pattern, replacement] of SECRET_PATTERNS) scrubbed = scrubbed.replace(pattern, replacement);
    return scrubbed;
  };
}

/** Takes what this computer knows out of `text`, then what looks like a secret ({@link scrubFor}). */
export function scrubText(text: string, { knownValues }: { knownValues: readonly KnownValue[] }): string {
  return scrubFor(knownValues)(text);
}

function scrubValue<T>(value: T, scrub: (text: string) => string): T {
  if (typeof value === "string") return scrub(value) as T;
  if (Array.isArray(value)) return value.map(item => scrubValue(item as unknown, scrub)) as T;
  if (typeof value !== "object" || value === null) return value;
  return Object.fromEntries(Object.entries(value).map(([key, item]) => [scrub(key), scrubValue(item as unknown, scrub)])) as T;
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

/**
 * The daemon recorded in `home` as `lookupDaemon` finds it, with a busy daemon waited for: one that is alive but missed
 * the 2 s probe is looked up once more, with up to `busyWaitMs` for its answer. That lookup reads `daemon.json` again,
 * so a daemon that took the busy one's place meanwhile is the one reported. One 2 s probe called the same daemon running
 * in one bundle and unresponsive in the next.
 */
async function patientLookup(home: string, busyWaitMs: number): Promise<DaemonLookup> {
  const lookup = await lookupDaemon(home);
  return lookup.status === "unresponsive" ? await lookupDaemon(home, busyWaitMs) : lookup;
}

async function readDaemon(home: string, busyWaitMs: number): Promise<SupportBundle["daemon"]> {
  const started = performance.now();
  const lookup = await patientLookup(home, busyWaitMs);
  if (lookup.status !== "running") return { status: lookup.status };
  const { version, ui, uptimeMs, native } = lookup.health;
  return {
    status: lookup.status,
    version,
    ui,
    uptimeMs,
    answeredInMs: Math.round(performance.now() - started),
    engine: native.ok ? { ok: true, version: native.kernelVersion } : { ok: false, error: native.error },
  };
}

/**
 * The build the package at `packageRoot` is, or an error that says why it has no content store its rules can open. The
 * managed installation is a version under the home's `versions/`, which `current` usually points at; `current` can also
 * point at a checkout or another package, and one that is neither has no store of its own. `RERAIL_CONTENT_STORE_PATH`
 * names a store for any build, as `contentStorePath` takes it first, except a file in the machine-wide store directory:
 * `assertContentStoreOwner` lets only the main daemon open that, so there it counts for the managed installation alone.
 */
function installBuild(packageRoot: string, home: string): RerailBuild | undefined {
  const override = contentStoreOverride();
  let build: RerailBuild | undefined;
  try {
    build = resolveRerailBuild(packageRoot, { ...process.env, RERAIL_HOME: home });
  } catch (error) {
    if (override === undefined) throw error;
  }
  if (override === undefined || build?.kind === "main" || !isInMainContentStoreDirectory(override)) return build;
  throw new Error(
    `RERAIL_CONTENT_STORE_PATH names ${override}, in the machine-wide content store directory ` +
      `${mainContentStoreDirectory()}, which only the main Rerail daemon opens`,
  );
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
  /** The package this command runs from; the bundle says whether it is the managed installation. */
  packageRoot: string;
  /** How long to wait for a busy daemon (`patientLookup`); `BUSY_DAEMON_WAIT_MS` unless a test shortens it. */
  busyDaemonWaitMs?: number;
}

/**
 * Everything but the session is read section by section, and a section that fails says why in `problems`: the bundle
 * matters most when Rerail cannot start, its database is damaged or set-up never finished.
 */
export async function collectSupportBundle(input: CollectInput): Promise<{ bundle: SupportBundle; ownRules: string[] }> {
  const { repo, home, databasePath, sessionId, packageVersion, packageRoot, busyDaemonWaitMs = BUSY_DAEMON_WAIT_MS } = input;
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
    const context = { attempt, repo, databasePath, sessionId, workspaceRoot, catalog, problems, knownValues, ownRules };
    ({ repository, session } = readDatabaseSections(context, repository));
  } else if (sessionId !== undefined) {
    throw new SupportBundleError(SESSION_NOT_FOUND(sessionId));
  }

  const { lines, omitted } = attempt("daemon log", () => readLogTail(home), { lines: [], omitted: 0 });
  const daemon = await readDaemon(home, busyDaemonWaitMs).catch((error: unknown) => {
    problems.push(`daemon: ${error instanceof Error ? error.message : String(error)}`);
    return { status: "unknown" };
  });
  const build = attempt("install", () => installBuild(packageRoot, home), undefined);
  const bundle: SupportBundle = {
    format: SUPPORT_BUNDLE_FORMAT,
    createdAt: Date.now(),
    leftOut: LEFT_OUT,
    rerail: {
      version: packageVersion,
      activeVersion: attempt("active version", () => activeCoreVersion(home), null),
      managedInstall: build?.kind === "main",
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
  const scrub = scrubFor(knownValues);
  return { bundle: scrubValue(bundle, scrub), ownRules: scrubValue(ownRules, scrub) };
}

/** Reads one section of the bundle; a section that throws gives `fallback` and says why in `problems`. */
type SectionAttempt = <T>(section: string, read: () => T, fallback: T) => T;

/** What `collectSupportBundle` reads the database with, and the lists its reads add to. */
interface DatabaseReadContext {
  attempt: SectionAttempt;
  repo: string;
  databasePath: string;
  sessionId: string | undefined;
  workspaceRoot: string;
  catalog: RuleCatalog | undefined;
  problems: string[];
  knownValues: KnownValue[];
  ownRules: string[];
}

/** The bundle's sections from the database: the checkout's repository with its rules, and the session. */
interface DatabaseSections {
  repository: SupportBundle["repository"];
  session: SupportBundleSession | null;
}

function readDatabaseSections(context: DatabaseReadContext, repository: SupportBundle["repository"]): DatabaseSections {
  const database = context.attempt("database", () => openRerailDatabase(context.databasePath, { readonly: true }), undefined);
  if (database === undefined) return { repository, session: null };
  try {
    return readOpenDatabase(database, context, repository);
  } finally {
    database.$client.close();
  }
}

function readOpenDatabase(database: RerailDatabase, context: DatabaseReadContext, repository: SupportBundle["repository"]): DatabaseSections {
  const { attempt, knownValues } = context;
  const repositories = attempt("repositories", () => listRepositories(database), []);
  knownValues.push(...repositories.flatMap(repositoryLiterals));
  knownValues.push(...attempt("session owners", () => database.select({ token: policySessionOwners.token }).from(policySessionOwners).all(), [])
    .map(row => ({ value: row.token, label: REDACTED })));
  const checkout = attempt("checkout", () => lookupCheckout(database, context.repo), undefined);
  const listing = checkout === undefined ? undefined : repositories.find(candidate => candidate.repositoryKey === checkout.repositoryKey);
  const withRules = checkout === undefined ? repository : readRules(database, context, checkout, repository);
  const historyIds = listing === undefined ? [] : repositoryHistoryIds(listing);
  return { repository: withRules, session: readSelectedSession(database, context, historyIds) };
}

/** `repository` with `checkout`'s rules, and its own rules' ids added to `context.ownRules`. */
function readRules(database: RerailDatabase, context: DatabaseReadContext, checkout: CheckoutContext, repository: SupportBundle["repository"]): SupportBundle["repository"] {
  const { catalog, ownRules } = context;
  return context.attempt("rules", () => {
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

/** The session the bundle is for, if one is found, with its id and folder added to `context.knownValues`. */
function readSelectedSession(database: RerailDatabase, context: DatabaseReadContext, historyIds: readonly string[]): SupportBundleSession | null {
  const { attempt, databasePath, workspaceRoot, problems } = context;
  const selected = attempt("session", () => selectSession(database, historyIds, context.sessionId), undefined);
  if (selected === undefined) return null;
  context.knownValues.push({ value: selected.sessionId, label: "<session>" }, { value: selected.cwd, label: "<checkout>" });
  return attempt("session", () => readSession(database, databasePath, selected, workspaceRoot, problems), null);
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
  const staging = `${output}.${process.pid}.tmp`;
  mkdirSync(dirname(output), { recursive: true, mode: 0o700 });
  try {
    writeFileSync(staging, `${JSON.stringify(bundle, null, 2)}\n`, { mode: 0o600 });
    renameSync(staging, output);
  } finally {
    rmSync(staging, { force: true });
  }
}

export interface SupportBundleCommandIo {
  packageVersion: string;
  /** The package this command runs from: this one's unless a test names another. */
  packageRoot?: string;
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
      packageRoot: io.packageRoot ?? PACKAGE_ROOT,
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
