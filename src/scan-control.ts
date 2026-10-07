import { spawnSync } from "node:child_process";
import { randomUUID } from "node:crypto";
import { existsSync, realpathSync } from "node:fs";
import { resolve } from "node:path";
import { and, asc, eq } from "drizzle-orm";
import { z } from "zod/v4";
import { openRerailDatabase, type RerailDatabase } from "./database.js";
import { policyRepositoryDiagnostics, policyScanRequests, policySessionOwners, policySettings, sessions } from "./schema.js";

import {
  policySettingsSchema, policySettingsPatchSchema, repositoryDiagnosticSchema, scanInputSchema, scanRequestSchema,
  type PolicySettings, type RepositoryDiagnostic, type PolicyControls, type ScanRequest,
} from "./scan-control-schema.js";
export interface PolicyOwnerClaim { readonly token: string; }

export const DEFAULT_POLICY_SETTINGS: Readonly<PolicySettings> = { scope: "touched-files", mode: "steer" };
const GIT_OUTPUT_LIMIT = 10 * 1024 * 1024;
const GIT_EXECUTABLE = (
  process.platform === "win32"
    ? [String.raw`C:\Program Files\Git\cmd\git.exe`, String.raw`C:\Program Files (x86)\Git\cmd\git.exe`]
    : ["/usr/bin/git", "/usr/local/bin/git", "/opt/homebrew/bin/git"]
).find(existsSync);

function gitPaths(workspaceRoot: string, args: readonly string[]): string[] {
  if (GIT_EXECUTABLE === undefined) throw new Error("Git is not installed in a supported system location");
  const result = spawnSync(GIT_EXECUTABLE, ["-C", workspaceRoot, ...args], {
    encoding: "utf8",
    maxBuffer: GIT_OUTPUT_LIMIT,
  });
  if (result.error !== undefined) throw result.error;
  if (result.status !== 0) throw new Error(result.stderr.trim() || `git exited with code ${result.status ?? "unknown"}`);
  return result.stdout.split("\0").filter(Boolean);
}

function gitDiffPaths(workspaceRoot: string): string[] {
  return [...new Set([
    ...gitPaths(workspaceRoot, ["diff", "--name-only", "--relative", "--diff-filter=ACMRTUXB", "-z", "--"]),
    ...gitPaths(workspaceRoot, ["diff", "--cached", "--name-only", "--relative", "--diff-filter=ACMRTUXB", "-z", "--"]),
    ...gitPaths(workspaceRoot, ["ls-files", "--others", "--exclude-standard", "-z", "--"]),
  ])].sort((left, right) => left.localeCompare(right));
}
function canonicalWorkspace(path: string): string { try { return realpathSync(path); } catch { return resolve(path); } }
function controls(
  databasePath: string,
  row: { cwd: string; endedAt: number | null },
  settings: PolicySettings | undefined,
  diagnostic: RepositoryDiagnostic | undefined,
  owner?: { delivery: "background" | "next-event"; active: boolean },
): PolicyControls {
  return {
    workspaceRoot: canonicalWorkspace(row.cwd),
    settings: settings ?? DEFAULT_POLICY_SETTINGS,
    settingsPath: databasePath,
    delivery: owner?.delivery ?? "next-event",
    active: row.endedAt === null && (owner?.active ?? false),
    repositoryAudit: diagnostic ?? null,
  };
}
function requireSession(database: RerailDatabase, sessionId: string): { cwd: string; endedAt: number | null } {
  const row = database.select({ cwd: sessions.cwd, endedAt: sessions.endedAt }).from(sessions).where(eq(sessions.sessionId, sessionId)).get();
  if (row === undefined) throw new Error(`Unknown Rerail session: ${sessionId}`);
  return row;
}
function parseSettings(value: unknown): PolicySettings { return policySettingsSchema.parse(value); }
function request(row: typeof policyScanRequests.$inferSelect): ScanRequest {
  return scanRequestSchema.parse({ ...row, paths: row.paths, auditIds: row.auditIds });
}
function processIsLive(processId: number): boolean {
  if (processId === process.pid) return true;
  try { process.kill(processId, 0); return true; } catch (error) {
    return !(error instanceof Error && "code" in error && error.code === "ESRCH");
  }
}

/** Durable controls keyed by canonical workspace, with requests kept session-local. */
export class SqliteScanControlStore {
  readonly path: string;
  readonly #database: RerailDatabase;
  constructor(databasePath: string, database?: RerailDatabase) { this.path = databasePath; this.#database = database ?? openRerailDatabase(databasePath); }

  #interruptOwned(sessionId: string, error: string): void {
    const update = { status: "interrupted" as const, finishedAt: Date.now(), error };
    for (const status of ["queued", "running"] as const)
      this.#database.update(policyScanRequests).set(update).where(and(eq(policyScanRequests.sessionId, sessionId), eq(policyScanRequests.status, status))).run();
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

  getWorkspaceSettings(workspaceRoot: string): PolicySettings | undefined {
    const row = this.#database.select().from(policySettings)
      .where(eq(policySettings.workspaceRoot, canonicalWorkspace(workspaceRoot))).get();
    return row === undefined ? undefined : parseSettings(row.settings);
  }

  getSettings(sessionId: string): PolicyControls {
    const session = requireSession(this.#database, sessionId);
    const workspaceRoot = canonicalWorkspace(session.cwd);
    const diagnostic = this.#database.select({
      sessionId: policyRepositoryDiagnostics.sessionId,
      auditId: policyRepositoryDiagnostics.auditId,
      status: policyRepositoryDiagnostics.status,
      startedAt: policyRepositoryDiagnostics.startedAt,
      finishedAt: policyRepositoryDiagnostics.finishedAt,
      error: policyRepositoryDiagnostics.error,
    }).from(policyRepositoryDiagnostics).where(eq(policyRepositoryDiagnostics.workspaceRoot, workspaceRoot)).get();
    const owner = this.#owner(sessionId);
    return controls(
      this.path,
      session,
      this.getWorkspaceSettings(workspaceRoot),
      diagnostic === undefined ? undefined : repositoryDiagnosticSchema.parse(diagnostic),
      owner,
    );
  }
  hasUserSettings(sessionId: string): boolean {
    const session = requireSession(this.#database, sessionId);
    return this.getWorkspaceSettings(session.cwd) !== undefined;
  }
  updateSettings(sessionId: string, patch: z.input<typeof policySettingsPatchSchema>): PolicyControls {
    const parsed = policySettingsPatchSchema.parse(patch);
    this.#database.transaction(() => {
      const session = requireSession(this.#database, sessionId);
      const workspaceRoot = canonicalWorkspace(session.cwd);
      const previous = this.#database.select().from(policySettings).where(eq(policySettings.workspaceRoot, workspaceRoot)).get();
      const settings = policySettingsSchema.parse({ ...DEFAULT_POLICY_SETTINGS, ...(previous === undefined ? {} : parseSettings(previous.settings)), ...parsed });
      this.#database.insert(policySettings).values({ workspaceRoot, settings, updatedAt: Date.now() })
        .onConflictDoUpdate({ target: policySettings.workspaceRoot, set: { settings, updatedAt: Date.now() } }).run();
    });
    return this.getSettings(sessionId);
  }
  claimRepositoryDiagnostic(sessionId: string): boolean {
    return this.#database.transaction(() => {
      const session = requireSession(this.#database, sessionId);
      const workspaceRoot = canonicalWorkspace(session.cwd);
      const diagnostic = {
        workspaceRoot,
        sessionId,
        auditId: null,
        status: "running" as const,
        startedAt: Date.now(),
        finishedAt: null,
        error: null,
      };
      const inserted = this.#database.insert(policyRepositoryDiagnostics).values(diagnostic).onConflictDoNothing().run() as { changes: number };
      if (inserted.changes > 0) return true;
      const current = this.#database.select().from(policyRepositoryDiagnostics)
        .where(eq(policyRepositoryDiagnostics.workspaceRoot, workspaceRoot)).get();
      if (current?.status === "running") {
        const owner = this.#database.select().from(policySessionOwners)
          .where(eq(policySessionOwners.sessionId, current.sessionId)).get();
        if (owner?.active && processIsLive(owner.processId)) return false;
        this.#database.update(policyRepositoryDiagnostics).set({
          status: "interrupted",
          finishedAt: Date.now(),
          error: "Owning runtime exited before repository diagnostic could complete",
        }).where(eq(policyRepositoryDiagnostics.workspaceRoot, workspaceRoot)).run();
      }
      return false;
    });
  }
  finishRepositoryDiagnostic(
    sessionId: string,
    runId: string | null,
    status: "completed" | "failed" | "interrupted",
    error: string | null = null,
  ): void {
    const session = requireSession(this.#database, sessionId);
    this.#database.update(policyRepositoryDiagnostics).set({
      auditId: runId, status, error, finishedAt: Date.now(),
    }).where(and(
      eq(policyRepositoryDiagnostics.workspaceRoot, canonicalWorkspace(session.cwd)),
      eq(policyRepositoryDiagnostics.sessionId, sessionId),
      eq(policyRepositoryDiagnostics.status, "running"),
    )).run();
  }
  interruptRepositoryDiagnostic(sessionId: string): void {
    const session = requireSession(this.#database, sessionId);
    this.#database.update(policyRepositoryDiagnostics).set({
      status: "interrupted", finishedAt: Date.now(), error: "Session closed before repository diagnostic completed",
    }).where(and(
      eq(policyRepositoryDiagnostics.workspaceRoot, canonicalWorkspace(session.cwd)),
      eq(policyRepositoryDiagnostics.sessionId, sessionId),
      eq(policyRepositoryDiagnostics.status, "running"),
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
  requestScan(sessionId: string, input: z.input<typeof scanInputSchema>): ScanRequest {
    const parsed = scanInputSchema.parse(input);
    const session = requireSession(this.#database, sessionId);
    const current = this.getSettings(sessionId);
    const requestedScope = parsed.scope ?? "git-diff";
    const scope = requestedScope === "git-diff" ? "touched-files" : "repository";
    let paths: string[] = [];
    let selectionError: string | null = null;
    if (requestedScope === "git-diff") {
      try {
        paths = gitDiffPaths(current.workspaceRoot);
        if (paths.length === 0) selectionError = "Git diff contains no changed files";
      } catch (error) {
        selectionError = `Could not read git diff: ${error instanceof Error ? error.message : String(error)}`;
      }
    }
    const id = randomUUID();
    const now = Date.now();
    let error = selectionError;
    if (session.endedAt !== null) error = "Session is inactive";
    else if (!current.active) error = "Session has no live owning runtime";
    const status = error === null ? "queued" : "failed";
    this.#database.insert(policyScanRequests).values({
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
    return this.listScans(sessionId).find(scan => scan.id === id)!;
  }
  listScans(sessionId: string): ScanRequest[] {
    requireSession(this.#database, sessionId);
    this.#owner(sessionId);
    return this.#database.select().from(policyScanRequests).where(eq(policyScanRequests.sessionId, sessionId)).orderBy(asc(policyScanRequests.createdAt), asc(policyScanRequests.id)).all().map(request);
  }
  /** Atomically claims one queued request for the owning live runtime. */
  claim(sessionId: string, claim: PolicyOwnerClaim): ScanRequest | undefined {
    return this.#database.transaction(() => {
      const owner = this.#owner(sessionId);
      if (owner?.active !== true || owner.token !== claim.token || owner.processId !== process.pid) return undefined;
      const queued = this.#database.select().from(policyScanRequests).where(and(eq(policyScanRequests.sessionId, sessionId), eq(policyScanRequests.status, "queued"))).orderBy(asc(policyScanRequests.createdAt), asc(policyScanRequests.id)).limit(1).get();
      if (queued === undefined) return undefined;
      const claimed = this.#database.update(policyScanRequests).set({ status: "running", startedAt: Date.now() }).where(and(eq(policyScanRequests.id, queued.id), eq(policyScanRequests.status, "queued"))).returning().get();
      return claimed === undefined ? undefined : request(claimed);
    });
  }
  finish(id: string, status: "completed" | "failed" | "interrupted", runIds: readonly string[], error: string | null = null): void {
    const finishedAt = Date.now();
    const finished = this.#database.update(policyScanRequests).set({ status, auditIds: [...runIds], error, finishedAt })
      .where(and(eq(policyScanRequests.id, id), eq(policyScanRequests.status, "running"))).returning({ id: policyScanRequests.id }).get();
    if (finished === undefined && status === "interrupted" && runIds.length > 0) {
      // Closing the session can record interruption before native work returns its audit IDs.
      this.#database.update(policyScanRequests).set({ auditIds: [...runIds] })
        .where(and(eq(policyScanRequests.id, id), eq(policyScanRequests.status, "interrupted"))).run();
    }
    const scan = this.#database.select().from(policyScanRequests).where(eq(policyScanRequests.id, id)).get();
    const runId = scan?.auditIds.at(-1);
    if (scan === undefined || runId === undefined || scan.startedAt === null || scan.finishedAt === null) return;
    this.#database.insert(policyRepositoryDiagnostics).values({
      workspaceRoot: scan.workspaceRoot,
      sessionId: scan.sessionId,
      auditId: runId,
      status: scan.status === "queued" || scan.status === "running" ? "interrupted" : scan.status,
      startedAt: scan.startedAt,
      finishedAt: scan.finishedAt,
      error: scan.error,
    }).onConflictDoUpdate({
      target: policyRepositoryDiagnostics.workspaceRoot,
      set: {
        sessionId: scan.sessionId,
        auditId: runId,
        status: scan.status === "queued" || scan.status === "running" ? "interrupted" : scan.status,
        startedAt: scan.startedAt,
        finishedAt: scan.finishedAt,
        error: scan.error,
      },
    }).run();
  }
  interrupt(sessionId: string): void { this.#interruptOwned(sessionId, "Session closed before scan could complete"); }
  close(): void { this.#database.$client.close(); }
}
