import { randomUUID } from "node:crypto";
import { lstatSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { and, asc, eq, getTableColumns, inArray, isNull, like, sql } from "drizzle-orm";
import type { SQLiteColumn } from "drizzle-orm/sqlite-core";
import { z } from "zod/v4";
import { checkoutRealpath, earlierRealpath, physicalPath } from "../catalog/checkout-control.js";
import { outsideGit } from "../catalog/repository-identity.js";
import { openRerailDatabase, type RerailDatabase } from "../storage/database.js";
import { requireRecordWritesMade } from "../storage/record-writes.js";
import { apartFromLockWaits } from "../storage/write-lock-wait.js";
import { runGitWithoutBlocking } from "../git.js";
import { processState } from "../process-liveness.js";
import { checkouts, repositoryAudits, auditRequests, policySessionOwners, auditSettings, sessions, audits, events } from "../storage/schema.js";
import { isOmittedFailuresSummary, isSourceGap, type AuditFailure } from "./audit.js";
import { parsedList } from "./audit-store.js";
import { ownerLockFreed, ownerLockOf, ownerToken } from "./owner-lock.js";
import { scrubSecrets } from "../secret-scrub.js";

import {
  auditSettingsSchema, auditSettingsPatchSchema, repositoryAuditSchema, auditRequestInputSchema, auditRequestSchema,
  DEFAULT_AUDIT_SETTINGS, type AuditSettings, type AuditSettingsPatch, type RepositoryAudit, type AuditControls, type AuditRequest,
} from "./audit-control-schema.js";
export interface PolicyOwnerClaim { readonly token: string; }
const GIT_OUTPUT_LIMIT = 10 * 1024 * 1024;

/**
 * The scan-level failure recorded when a repository scan stops at its failure limit ("Repository scan stopped after
 * reaching N failures"). It is not a cause: the failures that reached the limit are recorded beside it, and an earlier
 * version also counted files read only in part towards the limit, so a failed audit with this marker and nothing else
 * but such files failed only for them.
 */
function isScanLimitMarker(failure: { stage?: string | undefined; code: string; message: string }): boolean {
  return failure.stage === "scan" && failure.code === "budget-exceeded"
    && /^Repository scan stopped after reaching \d+ failures$/u.test(failure.message);
}

/**
 * Whether the failures `text` holds are some and each one a failed audit tolerates today. They are read as an audit
 * record reads them (`parsedList`): text that holds no JSON array of failures is a damaged record, left for a person to
 * mend rather than audited again at every session. So is a failure with no string `code` or `message`.
 */
function onlyToleratedFailures(text: string): boolean {
  const failures = parsedList(text, "failures");
  return Array.isArray(failures) && failures.length > 0
    && failures.every(item => hasCodeAndMessage(item) && (isScanLimitMarker(item) || isOmittedFailuresSummary(item) || isSourceGap(item)));
}

/**
 * Whether a stored failure, an object as `parsedList` reads it, has the string `code` and `message` every failure Rerail
 * writes has. The tests of a tolerated failure call string methods on `message`; its other fields they only compare.
 */
function hasCodeAndMessage(item: unknown): item is AuditFailure {
  const { code, message } = item as Record<string, unknown>; // an object: `parsedList` reads no other item as a failure
  return typeof code === "string" && typeof message === "string";
}

/** The paths Git lists for `args` in `workspaceRoot`, asked without blocking the thread; throws how Git failed. */
async function gitPaths(workspaceRoot: string, args: readonly string[]): Promise<string[]> {
  const result = await runGitWithoutBlocking(workspaceRoot, args, { maxBuffer: GIT_OUTPUT_LIMIT });
  if (result.error !== undefined) throw result.error;
  if (result.status !== 0) throw new Error(result.stderr.trim() || `git exited with code ${result.status ?? "unknown"}`);
  return result.stdout.split("\0").filter(Boolean);
}

function isDirectory(path: string): boolean {
  try {
    return lstatSync(path).isDirectory();
  } catch {
    return false;
  }
}

/** Which of `folders` (relative to `workspaceRoot`) are submodules its index declares: gitlinks, mode 160000. */
async function declaredSubmodules(workspaceRoot: string, folders: readonly string[]): Promise<Set<string>> {
  if (folders.length === 0) return new Set();
  return new Set((await gitPaths(workspaceRoot, ["--literal-pathspecs", "ls-files", "--stage", "-z", "--", ...folders]))
    .filter(entry => entry.startsWith("160000 "))
    .map(entry => entry.slice(entry.indexOf("\t") + 1)));
}

/**
 * Changed, staged and untracked files under `workspaceRoot`, relative to it. A dirty submodule the checkout declares,
 * or a recorded checkout inside it, appears as its folder; Git lists that checkout's own changed files in its place,
 * so each is judged by its owner. Git is never run in any other nested repository: such a folder stays a folder,
 * which the audit walks as part of this checkout.
 */
async function gitDiffPaths(workspaceRoot: string, isRecordedCheckout: (path: string) => boolean, prefix = ""): Promise<string[]> {
  const entries = new Set([
    ...await gitPaths(workspaceRoot, ["diff", "--name-only", "--no-renames", "--relative", "--ignore-submodules=none", "--diff-filter=ACMRTUXB", "-z", "--"]),
    ...await gitPaths(workspaceRoot, ["diff", "--cached", "--name-only", "--no-renames", "--relative", "--ignore-submodules=none", "--diff-filter=ACMRTUXB", "-z", "--"]),
    ...await gitPaths(workspaceRoot, ["ls-files", "--others", "--exclude-standard", "-z", "--"]),
  ].map(entry => entry.endsWith("/") ? entry.slice(0, -1) : entry));
  const folders = [...entries].filter(path => isDirectory(join(workspaceRoot, path)));
  const submodules = await declaredSubmodules(workspaceRoot, folders);
  const paths: string[] = [];
  for (const path of entries) {
    const absolute = join(workspaceRoot, path);
    const ownCheckout = folders.includes(path) && (submodules.has(path) || isRecordedCheckout(physicalPath(absolute)));
    paths.push(...ownCheckout ? await gitDiffPaths(absolute, isRecordedCheckout, `${prefix}${path}/`) : [`${prefix}${path}`]);
  }
  return [...new Set(paths)].sort((left, right) => left.localeCompare(right));
}

/** The folder and every folder above it, nearest first. */
function ancestors(path: string): string[] {
  const folders = [path];
  for (let folder = dirname(path); folder !== folders.at(-1); folder = dirname(folder)) folders.push(folder);
  return folders;
}
function canonicalWorkspace(path: string): string { try { return checkoutRealpath(path); } catch { return resolve(path); } }
/** {@link canonicalWorkspace} as Rerail 0.1.4 and earlier gave it, the key they saved a workspace's settings under. */
function earlierWorkspace(path: string): string { try { return earlierRealpath(path); } catch { return resolve(path); } }
/** The repository of the checkout recorded under the folder `key` names, null where none is, as a column. */
function recordedRepository(key: SQLiteColumn | ReturnType<typeof sql.placeholder>) {
  return sql<string | null>`(SELECT ${checkouts.repositoryKey} FROM ${checkouts} WHERE ${checkouts.checkoutPath} = ${key})`;
}
/**
 * The audit settings rows saved under either of two keys, `stored` and `earlier`, as a prepared statement, each with
 * the repository of the checkout recorded under its own key (`recorded`) and under `stored` (`own`).
 */
function settingsUnderEither(database: RerailDatabase) {
  return database.select({
    ...getTableColumns(auditSettings), recorded: recordedRepository(auditSettings.workspaceRoot), own: recordedRepository(sql.placeholder("stored")),
  }).from(auditSettings).where(inArray(auditSettings.workspaceRoot, [sql.placeholder("stored"), sql.placeholder("earlier")])).prepare();
}
/** A session's folder as it spelt it, and the repository Rerail recorded the session in; a caller's folder has none. */
interface SpeltFolder { readonly cwd: string; readonly repositoryId?: string | null }
function controls(
  databasePath: string,
  row: { cwd: string; endedAt: number | null },
  settings: AuditSettings | undefined,
  repositoryAudit: RepositoryAudit | undefined,
  owner?: { delivery: "background" | "next-event"; active: boolean },
): AuditControls {
  return {
    workspaceRoot: canonicalWorkspace(row.cwd),
    settings: settings ?? DEFAULT_AUDIT_SETTINGS,
    settingsPath: databasePath,
    delivery: owner?.delivery ?? "next-event",
    active: row.endedAt === null && (owner?.active ?? false),
    repositoryAudit: repositoryAudit ?? null,
  };
}
function requireSession(database: RerailDatabase, sessionId: string): SpeltFolder & { cwd: string; endedAt: number | null } {
  const row = database.select({ cwd: sessions.cwd, endedAt: sessions.endedAt, repositoryId: sessions.repositoryId }).from(sessions).where(eq(sessions.sessionId, sessionId)).get();
  if (row === undefined) throw new Error(`Unknown Rerail session: ${sessionId}`);
  return row;
}
function parseSettings(value: unknown): AuditSettings { return auditSettingsSchema.parse(value); }
function request(row: typeof auditRequests.$inferSelect): AuditRequest {
  return auditRequestSchema.parse({ ...row, paths: row.paths, auditIds: row.auditIds });
}
function processIsLive(processId: number): boolean {
  return processId === process.pid || processState(processId) !== "gone";
}

/** An open session with its owner row, as a start finds it, and when it was last heard from. */
interface OwnedOpenSession {
  readonly sessionId: string;
  readonly processId: number;
  readonly token: string;
  readonly heardAt: number;
}

/** `answer`, asked once per distinct key. */
function askedOnce<K, V>(answer: (key: K) => V): (key: K) => V {
  const answers = new Map<K, V>();
  return key => {
    if (!answers.has(key)) answers.set(key, answer(key));
    return answers.get(key)!;
  };
}

/**
 * The tokens of the claims this process made under no lock (`registerOwner`) and has not let go (`releaseOwner`): a
 * row that names this process and none of them is a process's that the system gave the same pid before. The row's
 * time cannot tell: the wall clock can step back after this process started (NTP, a virtual machine resumed), and a
 * claim made since then reads as made before the start. Holds one token per session this process holds under no lock.
 */
const claimsHereWithoutLock = new Set<string>();

/**
 * Whether the runtime that wrote `owner` has ended, so the session it holds is no one's: the process the row names is
 * gone; or the row was written under an engine service's lock that is free now (`ownerLockFreed`), however that service
 * ended and whatever process holds its pid; or, written under none, it names this process but this process did not
 * make the claim (`claimsHereWithoutLock`), a process the system gave the same pid did.
 */
function writerEnded(owner: OwnedOpenSession, live: (processId: number) => boolean, freed: (lockId: string) => boolean): boolean {
  if (!live(owner.processId)) return true;
  const lockId = ownerLockOf(owner.token);
  if (lockId !== undefined) return freed(lockId);
  return owner.processId === process.pid && !claimsHereWithoutLock.has(owner.token);
}

/** Durable controls keyed by canonical workspace, with requests kept session-local. */
export class SqliteAuditControlStore {
  readonly path: string;
  readonly #database: RerailDatabase;
  constructor(databasePath: string, database?: RerailDatabase) { this.path = databasePath; this.#database = database ?? openRerailDatabase(databasePath); }

  /**
   * `work` in one transaction, which no lock wait of this thread's takes the lock during (`apartFromLockWaits`): each
   * transaction here reads before it writes, so SQLite fails its write at once against a holder rather than wait, and
   * a lock wait takes the lock to learn it is free.
   */
  #transactionApart<T>(work: () => T): T {
    return apartFromLockWaits(this.path, () => this.#database.transaction(work));
  }

  /**
   * SQLite's `PRAGMA data_version` on this store's connection: it changes when any other connection commits to the
   * database, and stays put otherwise, so a poller can tell that nothing it reads here changed without reading it.
   */
  dataVersion(): number {
    return z.number().int().parse(this.#database.$client.pragma("data_version", { simple: true }));
  }

  #interruptOwned(sessionId: string, error: string): void {
    const update = { status: "interrupted" as const, finishedAt: Date.now(), error };
    for (const status of ["queued", "running"] as const)
      this.#database.update(auditRequests).set(update).where(and(eq(auditRequests.sessionId, sessionId), eq(auditRequests.status, status))).run();
  }
  #owner(sessionId: string): { delivery: "background" | "next-event"; active: boolean; processId: number; token: string } | undefined {
    const owner = this.#database.select({
      delivery: policySessionOwners.delivery, active: policySessionOwners.active,
      processId: policySessionOwners.processId, token: policySessionOwners.token,
    }).from(policySessionOwners).where(eq(policySessionOwners.sessionId, sessionId)).get();
    if (owner === undefined || !owner.active) return owner;
    if (processIsLive(owner.processId)) return owner;
    this.#interruptOwned(sessionId, "Owning runtime exited before scan could complete");
    this.#database.update(policySessionOwners).set({ active: false, updatedAt: Date.now() }).where(eq(policySessionOwners.sessionId, sessionId)).run();
    return { ...owner, active: false };
  }

  /**
   * The settings saved for the folder `workspaceRoot` as spelt here. A session reading its own passes the repository
   * Rerail recorded it in, which ties a row Rerail 0.1.4 saved under that spelling to the folder as its row would
   * ({@link #savedHere}), so the read needs no session row.
   */
  getWorkspaceSettings(workspaceRoot: string, repositoryId: string | null = null): AuditSettings | undefined {
    return this.#settings(canonicalWorkspace(workspaceRoot), { cwd: workspaceRoot, repositoryId });
  }

  #settings(workspaceRoot: string, spelt: SpeltFolder): AuditSettings | undefined {
    const row = this.#settingsRow(workspaceRoot, spelt);
    return row === undefined ? undefined : parseSettings(row.settings);
  }

  /**
   * The row saved under `workspaceRoot`, the canonical key, else the one Rerail 0.1.4 and earlier saved for the same
   * folder under the key `spelt.cwd` (the folder as the session or caller spelt it) gave it on Windows. An older Rerail
   * sharing the database goes on writing that row, so it is read where it is rather than copied. Both keys are read in
   * one statement, prepared once per store, so a folder with no row under either costs no more than a read of the
   * canonical key alone.
   */
  #settingsRow(workspaceRoot: string, spelt: SpeltFolder): typeof auditSettings.$inferSelect | undefined {
    const earlier = this.#earlierKey(workspaceRoot, spelt.cwd);
    if (earlier === workspaceRoot) return this.#database.select().from(auditSettings).where(eq(auditSettings.workspaceRoot, workspaceRoot)).get();
    this.#settingsUnderEither ??= settingsUnderEither(this.#database);
    const rows = this.#settingsUnderEither.all({ stored: workspaceRoot, earlier });
    return rows.find(row => row.workspaceRoot === workspaceRoot) ?? rows.find(row => this.#savedHere(row, spelt));
  }

  #settingsUnderEither: ReturnType<typeof settingsUnderEither> | undefined;

  /**
   * Whether the row Rerail 0.1.4 saved under the spelling `spelt.cwd` was saved for this folder. A letter or 8.3 name
   * can lead elsewhere than when 0.1.4 saved through it, so the row counts only where a fact ties it to this folder's
   * repository: the session's, else the one recorded for the folder under its stored spelling. Outside Git 0.1.4
   * recorded the folder under that same spelling, so the checkout recorded there names the repository the spelling
   * reached then; with none recorded there, nothing ties the row to this folder. A Git checkout was recorded under
   * Git's top level instead, so nothing says whose row it is, and it is read as 0.1.4 read it there.
   */
  #savedHere(row: { readonly recorded: string | null; readonly own: string | null }, spelt: SpeltFolder): boolean {
    if (row.recorded !== null) return row.recorded === (spelt.repositoryId ?? row.own);
    return !this.#outsideGit(spelt.cwd);
  }

  /** {@link outsideGit} once per folder spelling, as {@link #earlierKeys} holds its key: a `git init` there later keeps the answer. */
  readonly #outsideGit = askedOnce(outsideGit);

  /**
   * The keys Rerail 0.1.4 and earlier gave each folder spelling this store was asked about. The runtime reads a session's
   * settings before each of its events, so the realpath runs once per spelling rather than once per event; a link in
   * the path changed while this process runs keeps the key it had.
   */
  readonly #earlierKeys = new Map<string, string>();

  /** The key 0.1.4 gave `spelled` on Windows; `workspaceRoot` itself where the two cannot differ. */
  #earlierKey(workspaceRoot: string, spelled: string): string {
    if (process.platform !== "win32" || spelled === workspaceRoot) return workspaceRoot;
    const known = this.#earlierKeys.get(spelled);
    if (known !== undefined) return known;
    const earlier = earlierWorkspace(spelled);
    this.#earlierKeys.set(spelled, earlier);
    return earlier;
  }

  getSettings(sessionId: string): AuditControls {
    const session = requireSession(this.#database, sessionId);
    const workspaceRoot = canonicalWorkspace(session.cwd);
    // The audit of the checkout the session is in: the repository audit of the nearest folder at or above it.
    const folders = ancestors(workspaceRoot);
    const repositoryAudit = this.#database.select({
      workspaceRoot: repositoryAudits.workspaceRoot,
      sessionId: repositoryAudits.sessionId,
      auditId: repositoryAudits.auditId,
      status: repositoryAudits.status,
      startedAt: repositoryAudits.startedAt,
      finishedAt: repositoryAudits.finishedAt,
      error: repositoryAudits.error,
    }).from(repositoryAudits).where(inArray(repositoryAudits.workspaceRoot, folders)).all()
      .sort((left, right) => folders.indexOf(left.workspaceRoot) - folders.indexOf(right.workspaceRoot))
      .map(({ workspaceRoot: _root, ...row }) => row)[0];
    const owner = this.#owner(sessionId);
    return controls(
      this.path,
      session,
      this.#settings(workspaceRoot, session),
      repositoryAudit === undefined ? undefined : repositoryAuditSchema.parse(repositoryAudit),
      owner,
    );
  }
  hasUserSettings(sessionId: string): boolean {
    const session = requireSession(this.#database, sessionId);
    return this.#settings(canonicalWorkspace(session.cwd), session) !== undefined;
  }
  updateSettings(sessionId: string, patch: z.input<typeof auditSettingsPatchSchema>): AuditControls {
    const parsed = auditSettingsPatchSchema.parse(patch);
    this.#transactionApart(() => {
      const session = requireSession(this.#database, sessionId);
      this.#saveSettings(canonicalWorkspace(session.cwd), parsed, session);
    });
    return this.getSettings(sessionId);
  }
  /**
   * `updateSettings` for a checkout by its folder, before any session there exists: what its sessions start with. The
   * change starts from the settings {@link getWorkspaceSettings} reads for the folder as spelt here.
   */
  updateWorkspaceSettings(workspaceRoot: string, patch: z.input<typeof auditSettingsPatchSchema>): AuditSettings {
    const parsed = auditSettingsPatchSchema.parse(patch);
    return this.#transactionApart(() => this.#saveSettings(canonicalWorkspace(workspaceRoot), parsed, { cwd: workspaceRoot }));
  }
  /**
   * Merges `patch` over the checkout's saved settings, else the defaults, and saves the result under the canonical key.
   * The saved settings are read as the folder spelt `spelt` finds them, so a session's own spelling starts from what
   * Rerail 0.1.4 saved under it; that row stays for 0.1.4 to read. The default strategy, `message`, is left out of the
   * row, as rows read before the key existed, so a release that reads `{ scope, mode }` strictly still reads every row
   * that does not opt into an experimental strategy.
   */
  #saveSettings(workspaceRoot: string, patch: AuditSettingsPatch, spelt: SpeltFolder = { cwd: workspaceRoot }): AuditSettings {
    const previous = this.#settingsRow(workspaceRoot, spelt);
    const settings = auditSettingsSchema.parse({ ...DEFAULT_AUDIT_SETTINGS, ...(previous === undefined ? {} : parseSettings(previous.settings)), ...patch });
    const { steerStrategy, ...saved } = settings;
    const row = steerStrategy === "message" ? saved : settings;
    this.#database.insert(auditSettings).values({ workspaceRoot, settings: row, updatedAt: Date.now() })
      .onConflictDoUpdate({ target: auditSettings.workspaceRoot, set: { settings: row, updatedAt: Date.now() } }).run();
    return settings;
  }
  /**
   * Claim the audit of one checkout for this session: false while another live session audits it or once one completed.
   * An interrupted attempt was cut short and always runs again. A failed one runs again only when it can now come out
   * differently: it failed only for files read in part, which no longer fail an audit, or the compiled
   * catalog is not the one it ran against (`catalogRevision`; unknown when omitted). Anything else would fail the same
   * way at every session start. The audit runs off the session's request path, so a retry costs background work,
   * never a turn.
   */
  claimRepositoryAudit(sessionId: string, checkoutRoot: string, catalogRevision?: string): boolean {
    return this.#transactionApart(() => {
      requireSession(this.#database, sessionId);
      const workspaceRoot = canonicalWorkspace(checkoutRoot);
      const repositoryAudit = {
        workspaceRoot,
        sessionId,
        auditId: null,
        status: "running" as const,
        startedAt: Date.now(),
        finishedAt: null,
        error: null,
      };
      const inserted = this.#database.insert(repositoryAudits).values(repositoryAudit).onConflictDoNothing().run() as { changes: number };
      if (inserted.changes > 0) return true;
      const current = this.#database.select().from(repositoryAudits)
        .where(eq(repositoryAudits.workspaceRoot, workspaceRoot)).get();
      if (current === undefined || current.status === "completed") return false;
      if (current.status === "failed" && !this.#failedAuditCanDifferNow(current.auditId, catalogRevision)) return false;
      if (current.status === "running") {
        const owner = this.#database.select().from(policySessionOwners)
          .where(eq(policySessionOwners.sessionId, current.sessionId)).get();
        if (owner?.active && processIsLive(owner.processId)) return false;
      }
      this.#database.update(repositoryAudits).set(repositoryAudit)
        .where(eq(repositoryAudits.workspaceRoot, workspaceRoot)).run();
      return true;
    });
  }

  /**
   * Whether a failed repository audit may come out differently now. Only the failures it recorded tell: a recorded
   * failure no audit tolerates today (a budget, an adapter error) fails the same way again, and a failed audit with
   * nothing recorded, or none at all, has nothing to retry on.
   */
  #failedAuditCanDifferNow(auditId: string | null, catalogRevision: string | undefined): boolean {
    const failed = auditId === null
      ? undefined
      : this.#database.select({ failures: sql<string>`${audits.failures}`, catalogRevision: audits.catalogRevision })
        .from(audits).where(eq(audits.auditId, auditId)).get();
    if (failed === undefined) return false;
    if (failed.catalogRevision !== null && catalogRevision !== undefined && failed.catalogRevision !== catalogRevision) return true;
    return onlyToleratedFailures(failed.failures);
  }
  finishRepositoryAudit(
    sessionId: string,
    checkoutRoot: string,
    auditId: string | null,
    status: "completed" | "failed" | "interrupted",
    error: string | null = null,
  ): void {
    this.#database.update(repositoryAudits).set({
      // An error's text may quote what failed; it is kept scrubbed of secrets.
      auditId: auditId, status, error: error === null ? null : scrubSecrets(error), finishedAt: Date.now(),
    }).where(and(
      eq(repositoryAudits.workspaceRoot, canonicalWorkspace(checkoutRoot)),
      eq(repositoryAudits.sessionId, sessionId),
      eq(repositoryAudits.status, "running"),
    )).run();
  }
  /** Every checkout audit the session still runs ends interrupted. */
  interruptRepositoryAudit(sessionId: string): void {
    this.#database.update(repositoryAudits).set({
      status: "interrupted", finishedAt: Date.now(), error: "Session closed before repository diagnostic completed",
    }).where(and(
      eq(repositoryAudits.sessionId, sessionId),
      eq(repositoryAudits.status, "running"),
    )).run();
  }

  /** Claims the session for this process, under the lock `lockId` of the engine service it runs in when it has one. */
  registerOwner(sessionId: string, delivery: "background" | "next-event", lockId?: string | undefined): PolicyOwnerClaim {
    const token = ownerToken(lockId);
    // The session's row, just opened, may still wait in this process's record writer behind a holder of a moment; and the
    // claim's reopen of the row must not commit ahead of a waiting close of it, so the claim fails as the lock does.
    requireRecordWritesMade(this.path);
    this.#transactionApart(() => {
      requireSession(this.#database, sessionId);
      const owner = this.#owner(sessionId);
      if (owner?.active) throw new Error(`Rerail session ${sessionId} is already owned by a live runtime`);
      if (owner !== undefined) this.#interruptOwned(sessionId, "Owning runtime was replaced before scan could complete");
      this.#database.insert(policySessionOwners).values({ sessionId, delivery, active: true, processId: process.pid, token, updatedAt: Date.now() })
        .onConflictDoUpdate({ target: policySessionOwners.sessionId, set: { delivery, active: true, processId: process.pid, token, updatedAt: Date.now() } }).run();
      // The runtime opened the record just before this claim; another daemon's sweep may have ended it in between, as
      // the row this claim replaces was a writer's that ended (`endSessionsOfEndedOwners`). The session is open.
      this.#database.update(sessions).set({ endedAt: null }).where(eq(sessions.sessionId, sessionId)).run();
    });
    if (lockId === undefined) claimsHereWithoutLock.add(token);
    return { token };
  }
  /**
   * Writes the owner row of `claim`, which this process made (`registerOwner`) and still holds, again where another
   * connection removed it with the session's rows, as putting back a database copied before the session began does, so
   * the session's scans are claimed and its end is swept as before. A row already there, this claim's or another
   * runtime's, is left as it is: a live runtime's claim is never taken.
   */
  claimAgain(sessionId: string, delivery: "background" | "next-event", claim: PolicyOwnerClaim): void {
    this.#database.insert(policySessionOwners)
      .values({ sessionId, delivery, active: true, processId: process.pid, token: claim.token, updatedAt: Date.now() })
      .onConflictDoNothing().run();
  }
  /**
   * Whether `claim` still owns the session for this process: the claim a suspended session kept, which its next open
   * takes back without interrupting the scans queued for it. False once its owner was released or replaced.
   */
  holdsOwner(sessionId: string, claim: PolicyOwnerClaim): boolean {
    const owner = this.#owner(sessionId);
    return owner?.active === true && owner.token === claim.token && owner.processId === process.pid;
  }
  releaseOwner(sessionId: string, claim: PolicyOwnerClaim): void {
    this.#database.update(policySessionOwners).set({ active: false, updatedAt: Date.now() })
      .where(and(eq(policySessionOwners.sessionId, sessionId), eq(policySessionOwners.token, claim.token))).run();
    claimsHereWithoutLock.delete(claim.token);
  }
  /**
   * Ends each open session whose owner row's writer has ended (`writerEnded`), as a daemon stopped by a machine
   * restart, its heap limit or a kill leaves every session it held or had let go of while quiet: its scans still queued
   * are interrupted and its owner released, as when its owner is found gone, and its record ends `idleMs` after it was
   * last heard from (its newest recorded event, else its start), as the idle close of a daemon still running would have
   * ended it, and at the latest at `now`. Each process and each lock is asked once. One query and one transaction;
   * returns how many sessions it ended.
   */
  endSessionsOfEndedOwners(idleMs: number, now: number): number {
    return this.#transactionApart(() => {
      const open = this.#database.all<OwnedOpenSession>(sql`
        SELECT ${sessions.sessionId} AS sessionId, ${policySessionOwners.processId} AS processId,
          ${policySessionOwners.token} AS token,
          coalesce((SELECT ${events.observedAt} FROM ${events} WHERE ${events.sessionId} = ${sessions.sessionId}
            ORDER BY ${events.sequence} DESC LIMIT 1), ${sessions.startedAt}) AS heardAt
        FROM ${sessions} JOIN ${policySessionOwners} ON ${policySessionOwners.sessionId} = ${sessions.sessionId}
        WHERE ${sessions.endedAt} IS NULL
      `);
      const live = askedOnce(processIsLive);
      const freed = askedOnce((lockId: string) => ownerLockFreed(this.path, lockId));
      const ended = open.filter(owner => writerEnded(owner, live, freed));
      if (ended.length > 0) this.#endAbandoned(ended, idleMs, now);
      return ended.length;
    });
  }
  /**
   * Whether an open session's owner row was written under the lock `lockId` (`ownerToken`), so its lock file must stay
   * until a sweep has ended that session: a row whose lock has no file reads as a writer's that may still run.
   */
  namesOwnerLock(lockId: string): boolean {
    return this.#database.select({ sessionId: policySessionOwners.sessionId }).from(policySessionOwners)
      .innerJoin(sessions, eq(sessions.sessionId, policySessionOwners.sessionId))
      .where(and(isNull(sessions.endedAt), like(policySessionOwners.token, `${lockId}.%`))).limit(1).get() !== undefined;
  }
  #endAbandoned(ended: readonly OwnedOpenSession[], idleMs: number, now: number): void {
    const ids = ended.map(owner => owner.sessionId);
    const at = Date.now();
    this.#database.update(auditRequests).set({ status: "interrupted", finishedAt: at, error: "Owning runtime exited before scan could complete" })
      .where(and(inArray(auditRequests.sessionId, ids), inArray(auditRequests.status, ["queued", "running"]))).run();
    this.#database.update(policySessionOwners).set({ active: false, updatedAt: at }).where(inArray(policySessionOwners.sessionId, ids)).run();
    for (const { sessionId, heardAt } of ended) {
      this.#database.update(sessions).set({ endedAt: Math.min(heardAt + idleMs, now) }).where(eq(sessions.sessionId, sessionId)).run();
    }
  }
  /**
   * Queues an audit of the session's workspace. A git-diff one reads Git's diff first without holding the thread, so
   * the process serves other requests while Git runs; the session's state is read once that read has answered.
   */
  async requestAudit(sessionId: string, input: z.input<typeof auditRequestInputSchema>): Promise<AuditRequest> {
    const parsed = auditRequestInputSchema.parse(input);
    requireSession(this.#database, sessionId);
    const { workspaceRoot } = this.getSettings(sessionId);
    const diff = (parsed.scope ?? "git-diff") === "git-diff";
    const selection = diff ? await this.#diffSelection(workspaceRoot) : { paths: [], error: null };
    return this.#recordRequest(sessionId, { workspaceRoot, scope: diff ? "touched-files" : "repository", ...selection });
  }
  /** The files Git's diff lists in `workspaceRoot`, or why it gave none. */
  async #diffSelection(workspaceRoot: string): Promise<{ paths: string[]; error: string | null }> {
    try {
      const paths = await gitDiffPaths(workspaceRoot, path =>
        this.#database.select({ path: checkouts.checkoutPath }).from(checkouts).where(eq(checkouts.checkoutPath, path)).get() !== undefined);
      return { paths, error: paths.length === 0 ? "Git diff contains no changed files" : null };
    } catch (error) {
      return { paths: [], error: `Could not read git diff: ${scrubSecrets(error instanceof Error ? error.message : String(error))}` };
    }
  }
  /** Records `request` for the session: queued, or failed with why its files or the session leave nothing to audit. */
  #recordRequest(sessionId: string, request: { workspaceRoot: string; scope: "touched-files" | "repository"; paths: string[]; error: string | null }): AuditRequest {
    const session = requireSession(this.#database, sessionId);
    const current = this.getSettings(sessionId);
    const { workspaceRoot, scope, paths } = request;
    const id = randomUUID();
    const now = Date.now();
    let error = request.error;
    if (session.endedAt !== null) error = "Session is inactive";
    else if (!current.active) error = "Session has no live owning runtime";
    const status = error === null ? "queued" : "failed";
    this.#database.insert(auditRequests).values({
      id,
      sessionId,
      workspaceRoot,
      scope,
      mode: "log",
      paths,
      status,
      createdAt: now,
      startedAt: null,
      finishedAt: error === null ? null : now,
      auditIds: [],
      error,
    }).run();
    return this.listAuditRequests(sessionId).find(request => request.id === id)!;
  }
  /** Whether a scan request for the session waits in its queue, not yet claimed. */
  hasQueuedRequest(sessionId: string): boolean {
    return this.#database.select({ id: auditRequests.id }).from(auditRequests)
      .where(and(eq(auditRequests.sessionId, sessionId), eq(auditRequests.status, "queued"))).limit(1).get() !== undefined;
  }
  listAuditRequests(sessionId: string): AuditRequest[] {
    requireSession(this.#database, sessionId);
    this.#owner(sessionId);
    return this.#database.select().from(auditRequests).where(eq(auditRequests.sessionId, sessionId)).orderBy(asc(auditRequests.createdAt), asc(auditRequests.id)).all().map(request);
  }
  /** Atomically claims one queued request for the owning live runtime. */
  claim(sessionId: string, claim: PolicyOwnerClaim): AuditRequest | undefined {
    return this.#transactionApart(() => {
      const owner = this.#owner(sessionId);
      if (owner?.active !== true || owner.token !== claim.token || owner.processId !== process.pid) return undefined;
      const queued = this.#database.select().from(auditRequests).where(and(eq(auditRequests.sessionId, sessionId), eq(auditRequests.status, "queued"))).orderBy(asc(auditRequests.createdAt), asc(auditRequests.id)).limit(1).get();
      if (queued === undefined) return undefined;
      const claimed = this.#database.update(auditRequests).set({ status: "running", startedAt: Date.now() }).where(and(eq(auditRequests.id, queued.id), eq(auditRequests.status, "queued"))).returning().get();
      return claimed === undefined ? undefined : request(claimed);
    });
  }
  finish(id: string, status: "completed" | "failed" | "interrupted", auditIds: readonly string[], error: string | null = null): void {
    const finishedAt = Date.now();
    const finished = this.#database.update(auditRequests).set({ status, auditIds: [...auditIds], error: error === null ? null : scrubSecrets(error), finishedAt })
      .where(and(eq(auditRequests.id, id), eq(auditRequests.status, "running"))).returning({ id: auditRequests.id }).get();
    if (finished === undefined && status === "interrupted" && auditIds.length > 0) {
      // Closing the session can record interruption before native work returns its audit IDs.
      this.#database.update(auditRequests).set({ auditIds: [...auditIds] })
        .where(and(eq(auditRequests.id, id), eq(auditRequests.status, "interrupted"))).run();
    }
    const request = this.#database.select().from(auditRequests).where(eq(auditRequests.id, id)).get();
    const auditId = request?.auditIds.at(-1);
    if (request === undefined || auditId === undefined || request.startedAt === null || request.finishedAt === null) return;
    this.#database.insert(repositoryAudits).values({
      workspaceRoot: request.workspaceRoot,
      sessionId: request.sessionId,
      auditId: auditId,
      status: request.status === "queued" || request.status === "running" ? "interrupted" : request.status,
      startedAt: request.startedAt,
      finishedAt: request.finishedAt,
      error: request.error,
    }).onConflictDoUpdate({
      target: repositoryAudits.workspaceRoot,
      set: {
        sessionId: request.sessionId,
        auditId: auditId,
        status: request.status === "queued" || request.status === "running" ? "interrupted" : request.status,
        startedAt: request.startedAt,
        finishedAt: request.finishedAt,
        error: request.error,
      },
    }).run();
  }
  interrupt(sessionId: string): void { this.#interruptOwned(sessionId, "Session closed before scan could complete"); }
  close(): void { this.#database.$client.close(); }
}
