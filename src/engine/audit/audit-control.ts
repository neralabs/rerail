import { randomUUID } from "node:crypto";
import { lstatSync, realpathSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { and, asc, eq, inArray } from "drizzle-orm";
import { z } from "zod/v4";
import { physicalPath } from "../catalog/checkout-control.js";
import { openRerailDatabase, type RerailDatabase } from "../storage/database.js";
import { runGit } from "../git.js";
import { checkouts, repositoryAudits, auditRequests, policySessionOwners, auditSettings, sessions, audits } from "../storage/schema.js";
import { isOmittedFailuresSummary, isSourceGap } from "./audit.js";
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

function gitPaths(workspaceRoot: string, args: readonly string[]): string[] {
  const result = runGit(workspaceRoot, args, { maxBuffer: GIT_OUTPUT_LIMIT });
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
function declaredSubmodules(workspaceRoot: string, folders: readonly string[]): Set<string> {
  if (folders.length === 0) return new Set();
  return new Set(gitPaths(workspaceRoot, ["--literal-pathspecs", "ls-files", "--stage", "-z", "--", ...folders])
    .filter(entry => entry.startsWith("160000 "))
    .map(entry => entry.slice(entry.indexOf("\t") + 1)));
}

/**
 * Changed, staged and untracked files under `workspaceRoot`, relative to it. A dirty submodule the checkout declares,
 * or a recorded checkout inside it, appears as its folder; Git lists that checkout's own changed files in its place,
 * so each is judged by its owner. Git is never run in any other nested repository: such a folder stays a folder,
 * which the audit walks as part of this checkout.
 */
function gitDiffPaths(workspaceRoot: string, isRecordedCheckout: (path: string) => boolean, prefix = ""): string[] {
  const entries = new Set([
    ...gitPaths(workspaceRoot, ["diff", "--name-only", "--relative", "--ignore-submodules=none", "--diff-filter=ACMRTUXB", "-z", "--"]),
    ...gitPaths(workspaceRoot, ["diff", "--cached", "--name-only", "--relative", "--ignore-submodules=none", "--diff-filter=ACMRTUXB", "-z", "--"]),
    ...gitPaths(workspaceRoot, ["ls-files", "--others", "--exclude-standard", "-z", "--"]),
  ].map(entry => entry.endsWith("/") ? entry.slice(0, -1) : entry));
  const folders = [...entries].filter(path => isDirectory(join(workspaceRoot, path)));
  const submodules = declaredSubmodules(workspaceRoot, folders);
  const paths = [...entries].flatMap(path => {
    const absolute = join(workspaceRoot, path);
    const ownCheckout = folders.includes(path) && (submodules.has(path) || isRecordedCheckout(physicalPath(absolute)));
    return ownCheckout ? gitDiffPaths(absolute, isRecordedCheckout, `${prefix}${path}/`) : [`${prefix}${path}`];
  });
  return [...new Set(paths)].sort((left, right) => left.localeCompare(right));
}

/** The folder and every folder above it, nearest first. */
function ancestors(path: string): string[] {
  const folders = [path];
  for (let folder = dirname(path); folder !== folders.at(-1); folder = dirname(folder)) folders.push(folder);
  return folders;
}
function canonicalWorkspace(path: string): string { try { return realpathSync(path); } catch { return resolve(path); } }
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
function requireSession(database: RerailDatabase, sessionId: string): { cwd: string; endedAt: number | null } {
  const row = database.select({ cwd: sessions.cwd, endedAt: sessions.endedAt }).from(sessions).where(eq(sessions.sessionId, sessionId)).get();
  if (row === undefined) throw new Error(`Unknown Rerail session: ${sessionId}`);
  return row;
}
function parseSettings(value: unknown): AuditSettings { return auditSettingsSchema.parse(value); }
function request(row: typeof auditRequests.$inferSelect): AuditRequest {
  return auditRequestSchema.parse({ ...row, paths: row.paths, auditIds: row.auditIds });
}
function processIsLive(processId: number): boolean {
  if (processId === process.pid) return true;
  try { process.kill(processId, 0); return true; } catch (error) {
    return !(error instanceof Error && "code" in error && error.code === "ESRCH");
  }
}

/** Durable controls keyed by canonical workspace, with requests kept session-local. */
export class SqliteAuditControlStore {
  readonly path: string;
  readonly #database: RerailDatabase;
  constructor(databasePath: string, database?: RerailDatabase) { this.path = databasePath; this.#database = database ?? openRerailDatabase(databasePath); }

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

  getWorkspaceSettings(workspaceRoot: string): AuditSettings | undefined {
    const row = this.#database.select().from(auditSettings)
      .where(eq(auditSettings.workspaceRoot, canonicalWorkspace(workspaceRoot))).get();
    return row === undefined ? undefined : parseSettings(row.settings);
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
      this.getWorkspaceSettings(workspaceRoot),
      repositoryAudit === undefined ? undefined : repositoryAuditSchema.parse(repositoryAudit),
      owner,
    );
  }
  hasUserSettings(sessionId: string): boolean {
    const session = requireSession(this.#database, sessionId);
    return this.getWorkspaceSettings(session.cwd) !== undefined;
  }
  updateSettings(sessionId: string, patch: z.input<typeof auditSettingsPatchSchema>): AuditControls {
    const parsed = auditSettingsPatchSchema.parse(patch);
    this.#database.transaction(() => {
      this.#saveSettings(canonicalWorkspace(requireSession(this.#database, sessionId).cwd), parsed);
    });
    return this.getSettings(sessionId);
  }
  /** `updateSettings` for a checkout by its folder, before any session there exists: what its sessions start with. */
  updateWorkspaceSettings(workspaceRoot: string, patch: z.input<typeof auditSettingsPatchSchema>): AuditSettings {
    const parsed = auditSettingsPatchSchema.parse(patch);
    return this.#database.transaction(() => this.#saveSettings(canonicalWorkspace(workspaceRoot), parsed));
  }
  /**
   * Merges `patch` over the checkout's saved settings, else the defaults, and saves the result. The default strategy,
   * `message`, is left out of the row, as rows read before the key existed, so a release that reads `{ scope, mode }`
   * strictly still reads every row that does not opt into an experimental strategy.
   */
  #saveSettings(workspaceRoot: string, patch: AuditSettingsPatch): AuditSettings {
    const previous = this.#database.select().from(auditSettings).where(eq(auditSettings.workspaceRoot, workspaceRoot)).get();
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
    return this.#database.transaction(() => {
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
      : this.#database.select({ failures: audits.failures, catalogRevision: audits.catalogRevision })
        .from(audits).where(eq(audits.auditId, auditId)).get();
    if (failed === undefined) return false;
    if (failed.catalogRevision !== null && catalogRevision !== undefined && failed.catalogRevision !== catalogRevision) return true;
    return failed.failures.length > 0
      && failed.failures.every(item => isScanLimitMarker(item) || isOmittedFailuresSummary(item) || isSourceGap(item));
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

  registerOwner(sessionId: string, delivery: "background" | "next-event"): PolicyOwnerClaim {
    const token = randomUUID();
    this.#database.transaction(() => {
      requireSession(this.#database, sessionId);
      const owner = this.#owner(sessionId);
      if (owner?.active) throw new Error(`Rerail session ${sessionId} is already owned by a live runtime`);
      if (owner !== undefined) this.#interruptOwned(sessionId, "Owning runtime was replaced before scan could complete");
      this.#database.insert(policySessionOwners).values({ sessionId, delivery, active: true, processId: process.pid, token, updatedAt: Date.now() })
        .onConflictDoUpdate({ target: policySessionOwners.sessionId, set: { delivery, active: true, processId: process.pid, token, updatedAt: Date.now() } }).run();
    });
    return { token };
  }
  releaseOwner(sessionId: string, claim: PolicyOwnerClaim): void {
    this.#database.update(policySessionOwners).set({ active: false, updatedAt: Date.now() })
      .where(and(eq(policySessionOwners.sessionId, sessionId), eq(policySessionOwners.token, claim.token))).run();
  }
  requestAudit(sessionId: string, input: z.input<typeof auditRequestInputSchema>): AuditRequest {
    const parsed = auditRequestInputSchema.parse(input);
    const session = requireSession(this.#database, sessionId);
    const current = this.getSettings(sessionId);
    const requestedScope = parsed.scope ?? "git-diff";
    const scope = requestedScope === "git-diff" ? "touched-files" : "repository";
    let paths: string[] = [];
    let selectionError: string | null = null;
    if (requestedScope === "git-diff") {
      try {
        paths = gitDiffPaths(current.workspaceRoot, path =>
          this.#database.select({ path: checkouts.checkoutPath }).from(checkouts).where(eq(checkouts.checkoutPath, path)).get() !== undefined);
        if (paths.length === 0) selectionError = "Git diff contains no changed files";
      } catch (error) {
        selectionError = `Could not read git diff: ${scrubSecrets(error instanceof Error ? error.message : String(error))}`;
      }
    }
    const id = randomUUID();
    const now = Date.now();
    let error = selectionError;
    if (session.endedAt !== null) error = "Session is inactive";
    else if (!current.active) error = "Session has no live owning runtime";
    const status = error === null ? "queued" : "failed";
    this.#database.insert(auditRequests).values({
      id,
      sessionId,
      workspaceRoot: current.workspaceRoot,
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
  listAuditRequests(sessionId: string): AuditRequest[] {
    requireSession(this.#database, sessionId);
    this.#owner(sessionId);
    return this.#database.select().from(auditRequests).where(eq(auditRequests.sessionId, sessionId)).orderBy(asc(auditRequests.createdAt), asc(auditRequests.id)).all().map(request);
  }
  /** Atomically claims one queued request for the owning live runtime. */
  claim(sessionId: string, claim: PolicyOwnerClaim): AuditRequest | undefined {
    return this.#database.transaction(() => {
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
