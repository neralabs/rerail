/**
 * Merge Assist's facade: what the runtime, the API and the CLI call for one checkout. See docs/merge-assist-v1-build.md.
 * Every conflict event a detection pass (or a dismissal) produces goes to the conflict hook (`conflict-hook.ts`).
 */
import { eq } from "drizzle-orm";
import { z } from "zod";
import { ADAPTER_CAPABILITIES, type ConflictNotice, type RerailHost } from "../../engine/adapter-contract.js";
import type { CheckoutContext } from "../../engine/catalog/repository-model.js";
import { repositoryIdentityLocation } from "../../engine/catalog/repository-identity.js";
import { lookupCheckout, mergeAssistEnabled, recordCheckout } from "../../engine/catalog/repository-registry.js";
import { baseBranch, currentBranch, githubRepositoryOf } from "../../engine/merge-assist/branch-git.js";
import { orderConflicts } from "../../engine/merge-assist/conflicts.js";
import {
  githubReadinessSchema,
  type ConflictHookCommand,
  type GithubReadiness,
  type LastScan,
  type MergeAssistSettings,
  type MergeAssistStatus,
  lastScanSchema,
} from "../../engine/merge-assist/model.js";
import type { RerailDatabase } from "../../engine/storage/database.js";
import { checkouts, repositories } from "../../engine/storage/schema.js";
import { detectConflicts } from "./conflict-detect.js";
import { deliveryHandler, readSettings, settingsAt, writeSettings } from "./conflict-delivery.js";
import { ConflictHook, conflictEvents, type ConflictFire, type ConflictHandlerContext } from "./conflict-hook.js";
import { dismissedConflict, readConflicts, recordHandlerResult, replaceConflict } from "./conflict-store.js";
import { createGithubClient, type GithubClient, type PullRequestSummary } from "./github.js";
import { commandHandler, RunningCommands } from "./hook-command.js";
import { currentNotice, markNotesDelivered, pendingNotes, type ConflictNoteOptions, type PendingNote } from "./notes.js";
import { personName } from "./people.js";
import { STATE_KEYS, deleteState, isoSeconds, readState, writeState } from "./state.js";

export const MERGE_ASSIST_ERROR_CODES = ["not-a-checkout", "not-found", "invalid-state", "disabled"] as const;
export type MergeAssistErrorCode = (typeof MERGE_ASSIST_ERROR_CODES)[number];

/** A request Merge Assist refuses; `code` says which, the message says why for a person. */
export class MergeAssistError extends Error {
  constructor(readonly code: MergeAssistErrorCode, message: string) {
    super(message);
    this.name = "MergeAssistError";
  }
}

/** Who asks for a change: a person (dashboard, CLI), or a coding agent (MCP), which may only read and scan. */
export type MergeAssistActor = "person" | "agent";

export interface MergeAssistService {
  /** The checkout's state and its in-flight conflicts. Reads only; runs nothing. */
  status(checkoutPath: string): Promise<MergeAssistStatus>;
  /** Turn Merge Assist on or off for the checkout's repository. Off clears every checkout's conflicts and scans. */
  setEnabled(checkoutPath: string, enabled: boolean): Promise<MergeAssistStatus>;
  /** Find the checkout's in-flight conflicts now, with the open pull requests listed afresh. */
  scan(checkoutPath: string): Promise<MergeAssistStatus>;
  /** Dismiss an open in-flight conflict as not a conflict. Only a person does: a coding agent is refused. */
  dismissConflict(checkoutPath: string, conflictId: string, actor?: MergeAssistActor): Promise<MergeAssistStatus>;
  /** The person's settings for the checkout's repository: the conflict hook command. Reads only. */
  settings(checkoutPath: string): Promise<MergeAssistSettings>;
  /** Change the person's settings for the checkout's repository; a field left out stays, `hook: null` removes the command. */
  updateSettings(checkoutPath: string, change: MergeAssistSettingsChange): Promise<MergeAssistStatus>;
}

/** A change to the person's Merge Assist settings for a repository. */
export interface MergeAssistSettingsChange {
  readonly hook?: ConflictHookCommand | null;
}

/** What a session's runtime asks of Merge Assist, besides the facade. Each does nothing while Merge Assist is off. */
export interface MergeAssistRuntime extends MergeAssistService {
  /** A session started in the checkout: scan it for in-flight conflicts. */
  openSession(checkoutPath: string, host: RerailHost, sessionId: string): Promise<void>;
  /**
   * The notes for the session it has not been handed, leaving out items whose marks are in `inFlight`, for a session
   * whose host is `host` (whether it draws a status line decides whether the person's line carries the conflict's
   * line). Reads only.
   */
  pendingNotes(sessionId: string, checkoutPath: string, inFlight?: ReadonlySet<string>, host?: RerailHost): PendingNote[];
  /** Record notes as handed to the session, once a delivery took them. */
  markNotesDelivered(sessionId: string, checkoutPath: string, marks: readonly string[]): void;
  /**
   * Resolves once no conflict's agent note in the checkout waits for the person's hook command: at once without one,
   * else when it answers or the hook's note wait passes, whichever is first.
   */
  notesReleased(checkoutPath: string): Promise<void>;
  /** Whether the person's hook command may hold the checkout's conflict notes: one is set. */
  hookHoldsNotes(checkoutPath: string): boolean;
  /** A queued note's person's line as it reads now, from the conflict as stored (the hook's status included). Reads only. */
  conflictNotice(checkoutPath: string, notice: ConflictNotice, host?: RerailHost): ConflictNotice;
  /**
   * The agent edited files in the checkout: find its in-flight conflicts again, at most once per
   * {@link CONFLICT_DETECT_INTERVAL_MS} per checkout. `sessionId` is the session whose edit it was, on `host`: the
   * conflict hook hears it. Resolves to whether it ran; the conflict hook's handlers run after, in the background.
   */
  detectAfterEdit(checkoutPath: string, sessionId?: string | null, host?: RerailHost): Promise<boolean>;
  /**
   * Hear when a checkout's sessions may have something new to take: a conflict event, or a hook handler's answer.
   * Returns the call that stops listening.
   */
  onConflictsChanged(listener: (checkoutPath: string) => void): () => void;
  /** Resolves once the conflict hook's handlers running now have finished. */
  settled(): Promise<void>;
  /**
   * Stop: no conflict event is handed to the hook any more, and a handler that answers later records nothing. Every
   * hook command still running gets SIGTERM on its process group, then SIGKILL after a short grace; waits at most
   * `waitMs` in all, the handlers running now included.
   */
  stop(waitMs?: number): Promise<void>;
}

export interface MergeAssistServiceOptions {
  /** The Rerail database, or how to open it on first use (a daemon creates none until asked to). */
  readonly database: RerailDatabase | (() => RerailDatabase);
  /** By default the person's `gh` (`createGithubClient`). */
  readonly github?: GithubClient;
  readonly now?: () => number;
  readonly env?: NodeJS.ProcessEnv;
}

/** How long `gh`'s readiness is reused before `gh auth status` runs again; a scan always reads it afresh. */
const GITHUB_READINESS_MS = 5 * 60_000;
/** A scan marked running for longer than this was cut short (its process died); another may start. */
const SCAN_STALE_MS = 10 * 60_000;
/** The least time between two detections of a checkout's conflicts after the agent's edits. */
export const CONFLICT_DETECT_INTERVAL_MS = 30_000;
/** How long a listing of open pull requests is reused for detecting conflicts; a scan lists them afresh. */
const PULLS_REUSE_MS = 5 * 60_000;
/** The grace a hook command still running at stop gets after SIGTERM before SIGKILL (at most half the stop's wait). */
const STOP_KILL_GRACE_MS = 500;

const githubCacheSchema = z.object({ at: z.number(), readiness: githubReadinessSchema });
const scanningSchema = z.object({ since: z.number() });

/** The open pull requests as listed, and why they could not be, when `gh` failed. */
interface PullListing {
  readonly pulls: readonly PullRequestSummary[];
  readonly problem: string | null;
}

class DatabaseMergeAssist implements MergeAssistRuntime {
  readonly #options: MergeAssistServiceOptions;
  readonly #now: () => number;
  /** The conflict hook: delivery and the person's command. */
  readonly #hook: ConflictHook;
  /** The person's hook commands running for this Merge Assist, which {@link stop} ends. */
  readonly #commands = new RunningCommands();
  #database: RerailDatabase | undefined;
  #github: GithubClient | undefined;
  /** Scans running in this process, by checkout; a second request joins the first. */
  readonly #scans = new Map<string, Promise<void>>();
  /** `gh auth status` runs in this process, by repository; concurrent status reads share one. */
  readonly #githubReads = new Map<string, Promise<GithubReadiness>>();
  /** When this process last found a checkout's conflicts after an edit, by checkout. */
  readonly #detectedAt = new Map<string, number>();
  /** The open pull requests this process last listed, by repository, and when. */
  readonly #pulls = new Map<string, { at: number; listing: Promise<PullListing> }>();
  /** Who hears that a checkout's sessions may have something new to take. */
  readonly #listeners = new Set<(checkoutPath: string) => void>();
  /** Set once stopped: the database may be closing, so the hook's late answers are dropped. */
  #stopped = false;

  constructor(options: MergeAssistServiceOptions) {
    this.#options = options;
    this.#now = options.now ?? Date.now;
    this.#hook = new ConflictHook([deliveryHandler, commandHandler(this.#commands)]);
  }

  get #db(): RerailDatabase {
    const { database } = this.#options;
    this.#database ??= typeof database === "function" ? database() : database;
    return this.#database;
  }

  get #githubClient(): GithubClient {
    this.#github ??= this.#options.github ?? createGithubClient(this.#options.env === undefined ? {} : { env: this.#options.env });
    return this.#github;
  }

  /**
   * The Git checkout at `checkoutPath`. A change records it, as a session start does, so its settings and conflicts
   * have a recorded owner; a read only looks it up.
   */
  #checkout(checkoutPath: string, write: boolean): CheckoutContext {
    let git = false;
    try {
      git = repositoryIdentityLocation(checkoutPath).git;
    } catch {
      // A folder that is gone is no checkout.
    }
    if (!git) throw new MergeAssistError("not-a-checkout", `Merge Assist works in a Git checkout, and ${checkoutPath} is not one.`);
    const checkout = write ? recordCheckout(this.#db, checkoutPath, this.#now()) : lookupCheckout(this.#db, checkoutPath);
    if (checkout === undefined) {
      throw new MergeAssistError(
        "not-a-checkout",
        `Rerail has not recorded the checkout at ${checkoutPath} yet: turn Merge Assist on there, or start a coding agent session there.`,
      );
    }
    return checkout;
  }

  #enabledCheckout(checkoutPath: string): CheckoutContext {
    const checkout = this.#checkout(checkoutPath, true);
    if (!mergeAssistEnabled(this.#db, checkout.repositoryKey)) {
      throw new MergeAssistError("disabled", "Merge Assist is off for this repository; a person turns it on on the Merge Assist page or with rerail merge-assist on.");
    }
    return checkout;
  }

  /** The recorded checkout at `checkoutPath` when Merge Assist is on for its repository. */
  #activeCheckout(checkoutPath: string): CheckoutContext | undefined {
    const checkout = lookupCheckout(this.#db, checkoutPath);
    return checkout === undefined || !mergeAssistEnabled(this.#db, checkout.repositoryKey) ? undefined : checkout;
  }

  /** Tell the listeners the checkout's sessions may have something new to take. */
  #emit(checkoutPath: string): void {
    for (const listener of this.#listeners) {
      try {
        listener(checkoutPath);
      } catch (error) {
        process.stderr.write(`rerail: a Merge Assist listener failed: ${error instanceof Error ? error.message : String(error)}\n`);
      }
    }
  }

  /** Hand conflict events in the checkout to the conflict hook's handlers, which run in the background. */
  #fire(checkout: CheckoutContext, fires: readonly ConflictFire[], session: { id: string; host: RerailHost | null } | null): void {
    if (fires.length === 0 || this.#stopped) return;
    const root = checkout.checkoutPath;
    const context: ConflictHandlerContext = {
      checkout: { path: root, repositoryKey: checkout.repositoryKey, branch: currentBranch(root), base: baseBranch(root)?.name ?? null },
      session,
      settings: readSettings(this.#db, checkout.repositoryKey),
      env: this.#options.env ?? process.env,
      now: this.#now,
      deliver: () => this.#emit(root),
    };
    this.#hook.fire(fires, context, (conflictId, result) => {
      if (this.#stopped) return;
      this.#db.transaction(tx => recordHandlerResult(tx, root, conflictId, result, this.#now()), { behavior: "immediate" });
    });
  }

  /** `gh`'s readiness for the repository: the one read in the last five minutes by any process, else read now. */
  async #githubReadiness(repositoryKey: string, fresh: boolean): Promise<GithubReadiness> {
    const repository = githubRepositoryOf(repositoryKey);
    if (repository === null) return { state: "not-github", login: null, repository: null };
    const key = STATE_KEYS.github(repositoryKey);
    if (!fresh) {
      const cached = readState(this.#db, key, githubCacheSchema);
      if (cached !== undefined && this.#now() - cached.at < GITHUB_READINESS_MS) return cached.readiness;
    }
    let reading = this.#githubReads.get(repositoryKey);
    if (reading === undefined) {
      reading = this.#githubClient.readiness(repository)
        .then(readiness => {
          writeState(this.#db, key, { at: this.#now(), readiness }, this.#now());
          return readiness;
        })
        .finally(() => this.#githubReads.delete(repositoryKey));
      this.#githubReads.set(repositoryKey, reading);
    }
    return await reading;
  }

  /** Whether a scan of the checkout runs in this process or, by its fresh mark, in another. */
  #scanning(checkoutPath: string): boolean {
    if (this.#scans.has(checkoutPath)) return true;
    const mark = readState(this.#db, STATE_KEYS.scanning(checkoutPath), scanningSchema);
    return mark !== undefined && this.#now() - mark.since < SCAN_STALE_MS;
  }

  /**
   * Scan the checkout once at a time across processes; a request while one runs joins it here, or leaves it be elsewhere.
   * `session` is the session that asked for it, if any: the conflict hook hears it.
   */
  #scanOnce(checkout: CheckoutContext, session: { id: string; host: RerailHost | null } | null = null): Promise<void> {
    const path = checkout.checkoutPath;
    const running = this.#scans.get(path);
    if (running !== undefined) return running;
    const markKey = STATE_KEYS.scanning(path);
    const claimed = this.#db.transaction(tx => {
      const mark = readState(tx, markKey, scanningSchema);
      if (mark !== undefined && this.#now() - mark.since < SCAN_STALE_MS) return false;
      writeState(tx, markKey, { since: this.#now() }, this.#now());
      return true;
    }, { behavior: "immediate" });
    if (!claimed) return Promise.resolve();
    const work = (async () => {
      try {
        await this.#detect(checkout, session, true);
      } finally {
        deleteState(this.#db, markKey);
        this.#scans.delete(path);
      }
    })();
    this.#scans.set(path, work);
    return work;
  }

  /**
   * The open pull requests for detecting conflicts: the listing of the last five minutes unless `fresh`, else read now;
   * none without `gh`. A listing that failed says why.
   */
  #openPulls(checkout: CheckoutContext, fresh: boolean): Promise<PullListing> {
    const repository = githubRepositoryOf(checkout.repositoryKey);
    if (repository === null) return Promise.resolve({ pulls: [], problem: null });
    const cached = this.#pulls.get(checkout.repositoryKey);
    if (!fresh && cached !== undefined && this.#now() - cached.at < PULLS_REUSE_MS) return cached.listing;
    const listing = this.#githubReadiness(checkout.repositoryKey, fresh)
      .then(async (readiness): Promise<PullListing> => ({
        pulls: readiness.state === "signed-in" ? await this.#githubClient.openPullRequests(repository) : [],
        problem: null,
      }))
      .catch((error: unknown): PullListing => ({
        pulls: [],
        problem: `Rerail could not list open pull requests: ${error instanceof Error ? error.message : String(error)}`,
      }));
    this.#pulls.set(checkout.repositoryKey, { at: this.#now(), listing });
    return listing;
  }

  /**
   * Find the checkout's in-flight conflicts for `session` (null outside a session), then hand the pass's events to the
   * conflict hook, without waiting for its handlers. A scan (`scan`) lists the pull requests afresh and records itself
   * as the checkout's last scan.
   */
  async #detect(checkout: CheckoutContext, session: { id: string; host: RerailHost | null } | null, scan = false): Promise<void> {
    const { pulls, problem } = await this.#openPulls(checkout, scan);
    const detected = await detectConflicts({ database: this.#db, checkout, pulls, now: this.#now });
    if (detected === null) return;
    if (scan) {
      const last: LastScan = { at: isoSeconds(this.#now()), peers: detected.peers, problems: [...(problem === null ? [] : [problem]), ...detected.problems] };
      this.#db.transaction(tx => {
        if (mergeAssistEnabled(tx, checkout.repositoryKey)) writeState(tx, STATE_KEYS.scan(checkout.checkoutPath), last, this.#now());
      }, { behavior: "immediate" });
    }
    this.#fire(checkout, conflictEvents(detected.before, detected.after), session);
  }

  async status(checkoutPath: string): Promise<MergeAssistStatus> {
    const checkout = this.#checkout(checkoutPath, false);
    const db = this.#db;
    const root = checkout.checkoutPath;
    return {
      repositoryKey: checkout.repositoryKey,
      checkout: root,
      branch: currentBranch(root),
      baseBranch: baseBranch(root)?.name ?? null,
      enabled: mergeAssistEnabled(db, checkout.repositoryKey),
      settings: readSettings(db, checkout.repositoryKey),
      readiness: {
        github: await this.#githubReadiness(checkout.repositoryKey, false),
      },
      scanning: this.#scanning(root),
      lastScan: readState(db, STATE_KEYS.scan(root), lastScanSchema) ?? null,
      conflicts: orderConflicts(readConflicts(db, root).map(entry => entry.conflict)),
    };
  }

  async setEnabled(checkoutPath: string, enabled: boolean): Promise<MergeAssistStatus> {
    const checkout = this.#checkout(checkoutPath, true);
    this.#db.transaction(tx => {
      tx.update(repositories).set({ mergeAssist: enabled, updatedAt: this.#now() })
        .where(eq(repositories.repositoryKey, checkout.repositoryKey)).run();
      if (enabled) return;
      // Off clears what Merge Assist found in every checkout of the repository, dismissals included; the person's
      // settings stay.
      for (const { path } of tx.select({ path: checkouts.checkoutPath }).from(checkouts).where(eq(checkouts.repositoryKey, checkout.repositoryKey)).all()) {
        deleteState(tx, STATE_KEYS.scan(path));
        deleteState(tx, STATE_KEYS.conflicts(path));
      }
    }, { behavior: "immediate" });
    return await this.status(checkout.checkoutPath);
  }

  async scan(checkoutPath: string): Promise<MergeAssistStatus> {
    const checkout = this.#enabledCheckout(checkoutPath);
    await this.#scanOnce(checkout);
    return await this.status(checkout.checkoutPath);
  }

  async dismissConflict(checkoutPath: string, conflictId: string, actor: MergeAssistActor = "person"): Promise<MergeAssistStatus> {
    if (actor === "agent") {
      throw new MergeAssistError("invalid-state", "The person dismisses in-flight conflicts, not a coding agent. Tell them about it in one line; they dismiss it on the Merge Assist page.");
    }
    const checkout = this.#enabledCheckout(checkoutPath);
    const root = checkout.checkoutPath;
    const stored = readConflicts(this.#db, root).find(entry => entry.conflict.id === conflictId);
    if (stored === undefined) throw new MergeAssistError("not-found", `This checkout has no conflict ${conflictId}.`);
    if (stored.conflict.status !== "open") throw new MergeAssistError("invalid-state", `Conflict ${conflictId} is already dismissed.`);
    const dismissed = dismissedConflict(stored, personName(root), this.#now());
    this.#db.transaction(tx => replaceConflict(tx, root, dismissed, this.#now()), { behavior: "immediate" });
    this.#fire(checkout, [{ event: "closed", conflict: dismissed }], null);
    return await this.status(root);
  }

  async settings(checkoutPath: string): Promise<MergeAssistSettings> {
    return readSettings(this.#db, this.#checkout(checkoutPath, false).repositoryKey);
  }

  async updateSettings(checkoutPath: string, change: MergeAssistSettingsChange): Promise<MergeAssistStatus> {
    const checkout = this.#checkout(checkoutPath, true);
    this.#db.transaction(tx => {
      const settings = readSettings(tx, checkout.repositoryKey);
      writeSettings(tx, checkout.repositoryKey, { hook: change.hook === undefined ? settings.hook : change.hook }, this.#now());
    }, { behavior: "immediate" });
    return await this.status(checkout.checkoutPath);
  }

  async detectAfterEdit(checkoutPath: string, sessionId: string | null = null, host?: RerailHost): Promise<boolean> {
    const checkout = this.#activeCheckout(checkoutPath);
    if (checkout === undefined) return false;
    const last = this.#detectedAt.get(checkout.checkoutPath);
    if (last !== undefined && this.#now() - last < CONFLICT_DETECT_INTERVAL_MS) return false;
    this.#detectedAt.set(checkout.checkoutPath, this.#now());
    await this.#detect(checkout, sessionId === null ? null : { id: sessionId, host: host ?? null });
    return true;
  }

  async openSession(checkoutPath: string, host: RerailHost, sessionId: string): Promise<void> {
    const checkout = this.#activeCheckout(checkoutPath);
    if (checkout === undefined) return;
    await this.#scanOnce(checkout, { id: sessionId, host });
  }

  /** How a session on `host` takes the checkout's conflict notes: whether the host draws the footer. */
  #noteOptions(host: RerailHost | undefined): ConflictNoteOptions {
    return { statusLine: host === undefined || ADAPTER_CAPABILITIES[host].statusLine };
  }

  pendingNotes(sessionId: string, checkoutPath: string, inFlight: ReadonlySet<string> = new Set(), host?: RerailHost): PendingNote[] {
    return pendingNotes(this.#db, sessionId, checkoutPath, inFlight, {
      ...this.#noteOptions(host), awaiting: this.#hook.awaiting(checkoutPath),
    });
  }

  markNotesDelivered(sessionId: string, checkoutPath: string, marks: readonly string[]): void {
    markNotesDelivered(this.#db, sessionId, checkoutPath, marks, this.#now());
  }

  conflictNotice(checkoutPath: string, notice: ConflictNotice, host?: RerailHost): ConflictNotice {
    return currentNotice(this.#db, checkoutPath, notice, this.#noteOptions(host));
  }

  async notesReleased(checkoutPath: string): Promise<void> {
    await this.#hook.released(checkoutPath);
  }

  hookHoldsNotes(checkoutPath: string): boolean {
    return settingsAt(this.#db, checkoutPath).hook !== null;
  }

  onConflictsChanged(listener: (checkoutPath: string) => void): () => void {
    this.#listeners.add(listener);
    return () => this.#listeners.delete(listener);
  }

  async settled(): Promise<void> {
    await this.#hook.settled();
  }

  async stop(waitMs = 2_000): Promise<void> {
    this.#stopped = true;
    this.#listeners.clear();
    const started = Date.now();
    await this.#commands.stop(Math.min(STOP_KILL_GRACE_MS, waitMs / 2));
    const timer = Promise.withResolvers<void>();
    const timeout = setTimeout(timer.resolve, Math.max(0, waitMs - (Date.now() - started)));
    try {
      await Promise.race([this.#hook.settled(), timer.promise]);
    } finally {
      clearTimeout(timeout);
    }
  }
}

/**
 * Merge Assist over one database. It runs `gh` only to list open pull requests, and the person's hook command on
 * conflict events; a status read runs neither, apart from `gh auth status` at most every five minutes per repository.
 */
export function createMergeAssistService(options: MergeAssistServiceOptions): MergeAssistRuntime {
  return new DatabaseMergeAssist(options);
}
