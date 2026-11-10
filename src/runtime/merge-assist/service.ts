/**
 * Merge Assist's facade: what the runtime, the API and the CLI call for one checkout. See docs/merge-assist-v1-build.md.
 * Every conflict event a detection pass (or a dismissal) produces goes to the conflict hook (`conflict-hook.ts`); the
 * experimental verdicts and chat answers take part only through the handlers registered on it.
 */
import { and, eq } from "drizzle-orm";
import { z } from "zod";
import { ADAPTER_CAPABILITIES, type ConflictNotice, type RerailHost } from "../../engine/adapter-contract.js";
import type { CheckoutContext, RuleCatalog } from "../../engine/catalog/repository-model.js";
import { repositoryIdentityLocation } from "../../engine/catalog/repository-identity.js";
import { lookupCheckout, mergeAssistEnabled, recordCheckout } from "../../engine/catalog/repository-registry.js";
import { conflictVerdictsEnabled } from "../../engine/experimental-features.js";
import { gitText } from "../../engine/git.js";
import { baseBranch, currentBranch, githubRepositoryOf } from "../../engine/merge-assist/branch-git.js";
import { candidateHash, candidatesInForce, type Candidate, type CandidateState } from "../../engine/merge-assist/candidate.js";
import { candidatesIgnored, committedCandidates, readWorktreeCandidates, writeWorktreeCandidate } from "../../engine/merge-assist/candidate-files.js";
import { orderConflicts } from "../../engine/merge-assist/conflicts.js";
import { commitStagingNote, gitCommitIn, shellCommandOf, unstagedDecisionFiles } from "../../engine/merge-assist/commit-staging.js";
import type { ChatAnswer } from "../../engine/merge-assist/chat-answer.js";
import {
  findingSchema,
  githubReadinessSchema,
  lastScanSchema,
  type ConflictAction,
  type ConflictDelivery,
  type ConflictHookCommand,
  type Finding,
  type FindingAction,
  type FindingStatus,
  type FindingTier,
  type GithubReadiness,
  type MergeAssistSettings,
  type MergeAssistStatus,
  type OwnDecision,
} from "../../engine/merge-assist/model.js";
import type { RerailDatabase } from "../../engine/storage/database.js";
import { mergeAssistItems, repositories, type MergeAssistItemRow } from "../../engine/storage/schema.js";
import { activationFailures, deactivateRepository, itemRuleId, ownRuleId, reconcileCheckout, rulesActive } from "./activation.js";
import { createAgentJudge, type AgentJudge } from "./agent-judge.js";
import { captureCorrections, type CaptureResult, type SessionEventSource } from "./capture.js";
import { detectConflicts } from "./conflict-detect.js";
import { deliveryHandler, readSettings, settingsAt, viaChat, writeSettings } from "./conflict-delivery.js";
import { ConflictHook, conflictEvents, type ConflictFire, type ConflictHandlerContext } from "./conflict-hook.js";
import { dismissedConflict, readConflicts, recordHandlerResult, replaceConflict, type StoredConflict } from "./conflict-store.js";
import { summaryHandler } from "./conflict-summary.js";
import { experimentalHandlers } from "./experiment-handlers.js";
import { createGithubClient, type GithubClient, type PullRequestSummary } from "./github.js";
import { commandHandler, RunningCommands } from "./hook-command.js";
import { currentNotice, markNotesDelivered, pendingNotes, stopNotes, type ConflictNoteOptions, type EditCall, type PendingNote } from "./notes.js";
import { authorName, osUser, personName } from "./people.js";
import { scanPeers, type ItemEvidence } from "./scan.js";
import { STATE_KEYS, deleteState, isoSeconds, readState, writeState } from "./state.js";

/** The refusal of a verdict while the experiment is off. */
const VERDICTS_OFF = "experimental: set RERAIL_EXPERIMENTAL_CONFLICT_VERDICTS=1";

export const MERGE_ASSIST_ERROR_CODES = ["not-a-checkout", "not-found", "invalid-state", "disabled"] as const;
export type MergeAssistErrorCode = (typeof MERGE_ASSIST_ERROR_CODES)[number];

/** A request Merge Assist refuses; `code` says which, the message says why for a person. */
export class MergeAssistError extends Error {
  constructor(readonly code: MergeAssistErrorCode, message: string) {
    super(message);
    this.name = "MergeAssistError";
  }
}

/** Who asks for a change: a person (dashboard, CLI), or a coding agent (MCP), which may only do what §7 allows. */
export type MergeAssistActor = "person" | "agent";

export interface MergeAssistService {
  /** The checkout's state, decisions and findings. Reads only; runs nothing. */
  status(checkoutPath: string): Promise<MergeAssistStatus>;
  /** Turn Merge Assist on or off for the checkout's repository. Off removes every candidate it applied. */
  setEnabled(checkoutPath: string, enabled: boolean): Promise<MergeAssistStatus>;
  /** Read other branches and open pull requests, sort their candidates into tiers, and apply the absorbed ones. */
  scan(checkoutPath: string): Promise<MergeAssistStatus>;
  /** Reject or restore one of this branch's own candidates (rewrites its file's `state`). */
  setCandidateState(checkoutPath: string, candidateId: string, state: CandidateState): Promise<MergeAssistStatus>;
  /**
   * Adopt a review finding, dismiss any finding, or undo an applied one. An agent may only dismiss a finding that is
   * still open: dismissing an applied one would undo it, which is the person's call.
   */
  resolveFinding(checkoutPath: string, findingId: string, action: FindingAction, actor?: MergeAssistActor): Promise<MergeAssistStatus>;
  /**
   * Settle an open in-flight conflict. `dismiss` (not a conflict) always; keep this branch's side, take theirs or write
   * a new decision (`text` required) only with the experimental verdicts on, else `invalid-state`. Only a person
   * decides: a coding agent is refused.
   */
  resolveConflict(checkoutPath: string, conflictId: string, action: ConflictAction, text?: string, actor?: MergeAssistActor): Promise<MergeAssistStatus>;
  /** The person's settings for the checkout's repository: conflict delivery and the conflict hook command. Reads only. */
  settings(checkoutPath: string): Promise<MergeAssistSettings>;
  /** Change the person's settings for the checkout's repository; a field left out stays, `hook: null` removes the command. */
  updateSettings(checkoutPath: string, change: MergeAssistSettingsChange): Promise<MergeAssistStatus>;
}

/** A change to the person's Merge Assist settings for a repository. */
export interface MergeAssistSettingsChange {
  readonly delivery?: ConflictDelivery;
  readonly hook?: ConflictHookCommand | null;
}

/** A session's capture request: its id and host, the checkout it runs in, and where its stored events are read. */
export interface MergeAssistSession {
  readonly sessionId: string;
  readonly host: RerailHost;
  readonly checkoutPath: string;
  readonly events: SessionEventSource;
}

/** What a session's runtime asks of Merge Assist, besides the facade. Each does nothing while Merge Assist is off. */
export interface MergeAssistRuntime extends MergeAssistService {
  /** A session started in the checkout: apply what the candidates call for, then scan the other branches. */
  openSession(checkoutPath: string, host: RerailHost, sessionId: string): Promise<void>;
  /** A turn ended: capture the session's new corrections and apply a saved decision. The scan that follows is {@link scanSession}. */
  captureSession(session: MergeAssistSession): Promise<CaptureResult>;
  /** Scan the other branches for a session's checkout, as after a saved decision (it may contradict another branch's). */
  scanSession(checkoutPath: string, host: RerailHost, sessionId: string): Promise<void>;
  /**
   * The notes for the session it has not been handed, leaving out items whose marks are in `inFlight`, for a session
   * whose host is `host` (what it draws decides how a conflict reaches the person in terminal delivery). Reads only.
   */
  pendingNotes(sessionId: string, checkoutPath: string, inFlight?: ReadonlySet<string>, host?: RerailHost): PendingNote[];
  /**
   * The stop notes for a tool call about to edit contested files, once per conflict and session; `asks` when the
   * session's host can ask the person before the call. Reads only.
   */
  stopNotes(sessionId: string, checkoutPath: string, call: EditCall, asks: boolean, inFlight?: ReadonlySet<string>, host?: RerailHost): PendingNote[];
  /** Record notes as handed to the session, once a delivery took them. */
  markNotesDelivered(sessionId: string, checkoutPath: string, marks: readonly string[]): void;
  /**
   * Resolves once no conflict's agent note in the checkout waits for the person's hook command: at once without one,
   * else when it answers or the hook's note wait passes, whichever is first.
   */
  notesReleased(checkoutPath: string): Promise<void>;
  /** Whether the person's hook command may hold the checkout's conflict notes: one is set, and the delivery includes chat. */
  hookHoldsNotes(checkoutPath: string): boolean;
  /** A queued note's person's line as it reads now, from the conflict as stored (its status line text included). Reads only. */
  conflictNotice(checkoutPath: string, notice: ConflictNotice, host?: RerailHost): ConflictNotice;
  /**
   * The agent edited files in the checkout: find its in-flight conflicts again, at most once per
   * {@link CONFLICT_DETECT_INTERVAL_MS} per checkout. `sessionId` is the session whose edit it was, whose stop budget a
   * stop is granted from, on `host`. Resolves to whether it ran; the conflict hook's handlers run after, in the background.
   */
  detectAfterEdit(checkoutPath: string, sessionId?: string | null, host?: RerailHost): Promise<boolean>;
  /**
   * The person's prompt in a session: with the experimental chat answers on, when it plainly answers a checkpoint or
   * stop card that session was handed (`parseChatAnswer`), record it as theirs, as the dashboard does: a verdict, or
   * "later", which snoozes the conflict. Off, it reads nothing. Resolves to what it read.
   */
  answerFromChat(checkoutPath: string, prompt: string, sessionId: string): Promise<ChatAnswer>;
  /**
   * A tool call in the checkout: when it runs `git commit` and decision files under `.rerail/candidates/` would be left
   * out of that commit, the context naming them; else null. Reads only.
   */
  commitStaging(checkoutPath: string, toolName: string, input: unknown): string | null;
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
  readonly ruleCatalog: RuleCatalog | (() => RuleCatalog);
  /** The judge, or the judge for a session's host; by default the person's agent CLI (`createAgentJudge`). */
  readonly judge?: AgentJudge | ((host?: RerailHost) => AgentJudge);
  /** By default the person's `gh` (`createGithubClient`). */
  readonly github?: GithubClient;
  readonly now?: () => number;
  readonly env?: NodeJS.ProcessEnv;
  /** Experimental conflict verdicts; by default `RERAIL_EXPERIMENTAL_CONFLICT_VERDICTS` in `env` (`conflictVerdictsEnabled`). */
  readonly conflictVerdicts?: boolean;
}

/** How long `gh`'s readiness is reused before `gh auth status` runs again; a scan always reads it afresh. */
const GITHUB_READINESS_MS = 5 * 60_000;
/** A scan marked running for longer than this was cut short (its process died); another may start. */
const SCAN_STALE_MS = 10 * 60_000;
/** The least time between two detections of a checkout's conflicts after the agent's edits. */
export const CONFLICT_DETECT_INTERVAL_MS = 30_000;
/** How long a listing of open pull requests is reused for finding conflicts; a scan lists them afresh. */
const PULLS_REUSE_MS = 5 * 60_000;
/** The grace a hook command still running at stop gets after SIGTERM before SIGKILL (at most half the stop's wait). */
const STOP_KILL_GRACE_MS = 500;
const TIER_ORDER: Readonly<Record<FindingTier, number>> = { talk: 0, review: 1, absorb: 2 };

const githubCacheSchema = z.object({ at: z.number(), readiness: githubReadinessSchema });
const scanningSchema = z.object({ since: z.number() });
const findingEvidenceSchema = findingSchema.shape.evidence;

class DatabaseMergeAssist implements MergeAssistRuntime {
  readonly #options: MergeAssistServiceOptions;
  readonly #now: () => number;
  /** The conflict hook: the built-in handlers, then the experimental ones whose flag is on in this process. */
  readonly #hook: ConflictHook;
  /** The person's hook commands running for this Merge Assist, which {@link stop} ends. */
  readonly #commands = new RunningCommands();
  readonly #judges = new Map<RerailHost | undefined, AgentJudge>();
  #database: RerailDatabase | undefined;
  #github: GithubClient | undefined;
  /** Scans running in this process, by checkout; a second request joins the first. */
  readonly #scans = new Map<string, Promise<void>>();
  /** `gh auth status` runs in this process, by repository; concurrent status reads share one. */
  readonly #githubReads = new Map<string, Promise<GithubReadiness>>();
  /** When this process last found a checkout's conflicts after an edit, by checkout. */
  readonly #detectedAt = new Map<string, number>();
  /** The open pull requests this process last listed, by repository, and when. */
  readonly #pulls = new Map<string, { at: number; pulls: Promise<readonly PullRequestSummary[]> }>();
  /** Who hears that a checkout's sessions may have something new to take. */
  readonly #listeners = new Set<(checkoutPath: string) => void>();
  /** Set once stopped: the database may be closing, so the hook's late answers are dropped. */
  #stopped = false;

  constructor(options: MergeAssistServiceOptions) {
    this.#options = options;
    this.#now = options.now ?? Date.now;
    const env = options.env ?? process.env;
    const conflictVerdicts = options.conflictVerdicts ?? conflictVerdictsEnabled(env);
    this.#hook = new ConflictHook([deliveryHandler, summaryHandler, commandHandler(this.#commands), ...experimentalHandlers(conflictVerdicts)]);
  }

  get #db(): RerailDatabase {
    const { database } = this.#options;
    this.#database ??= typeof database === "function" ? database() : database;
    return this.#database;
  }

  get #ruleCatalog(): RuleCatalog {
    const { ruleCatalog } = this.#options;
    return typeof ruleCatalog === "function" ? ruleCatalog() : ruleCatalog;
  }

  get #githubClient(): GithubClient {
    this.#github ??= this.#options.github ?? createGithubClient(this.#options.env === undefined ? {} : { env: this.#options.env });
    return this.#github;
  }

  #judge(host?: RerailHost): AgentJudge {
    const { judge } = this.#options;
    if (judge !== undefined && typeof judge !== "function") return judge;
    let made = this.#judges.get(host);
    if (made === undefined) {
      made = judge !== undefined
        ? judge(host)
        : createAgentJudge({
          database: this.#db,
          ...(host === undefined ? {} : { preferredHost: host }),
          ...(this.#options.env === undefined ? {} : { env: this.#options.env }),
        });
      this.#judges.set(host, made);
    }
    return made;
  }

  /**
   * The Git checkout at `checkoutPath`. A change records it, as a session start does, so its settings and items have
   * a recorded owner; a read only looks it up.
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

  async #reconcile(checkout: CheckoutContext): Promise<void> {
    await reconcileCheckout(this.#db, this.#ruleCatalog, checkout, this.#now());
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
    const settings = readSettings(this.#db, checkout.repositoryKey);
    const context: ConflictHandlerContext = {
      checkout: { path: root, repositoryKey: checkout.repositoryKey, branch: currentBranch(root), base: baseBranch(root)?.name ?? null },
      session,
      delivery: settings.delivery,
      settings,
      judge: this.#judge(session?.host ?? undefined),
      env: this.#options.env ?? process.env,
      now: this.#now,
      deliver: () => this.#emit(root),
      detected: fires.filter(fire => fire.event === "detected").map(fire => fire.conflict.conflict)
        .sort((left, right) => right.detectedAt.localeCompare(left.detectedAt)).map(conflict => conflict.id),
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

  /** Who a new candidate names: the `gh` login, else Git's `user.name`, else the account's user name. */
  async #author(checkout: CheckoutContext): Promise<string> {
    const github = await this.#githubReadiness(checkout.repositoryKey, false);
    const author = authorName(github.login ?? undefined)
      ?? authorName(gitText(checkout.checkoutPath, ["config", "user.name"]))
      ?? authorName(osUser());
    if (author === undefined) throw new Error("Merge Assist found no usable author name: no gh login, Git user.name or account name");
    return author;
  }

  /** Whether a scan of the checkout runs in this process or, by its fresh mark, in another. */
  #scanning(checkoutPath: string): boolean {
    if (this.#scans.has(checkoutPath)) return true;
    const mark = readState(this.#db, STATE_KEYS.scanning(checkoutPath), scanningSchema);
    return mark !== undefined && this.#now() - mark.since < SCAN_STALE_MS;
  }

  /**
   * Scan the checkout once at a time across processes; a request while one runs joins it here, or leaves it be elsewhere.
   * `sessionId` is the session that asked for it, if any: its stop budget is the one the detection that follows draws on.
   */
  #scanOnce(checkout: CheckoutContext, host?: RerailHost, sessionId: string | null = null): Promise<void> {
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
        await scanPeers({
          database: this.#db,
          checkout,
          judge: this.#judge(host),
          github: this.#githubClient,
          githubReadiness: () => this.#githubReadiness(checkout.repositoryKey, true),
          now: this.#now,
        });
        await this.#detect(checkout, sessionId === null ? null : { id: sessionId, host: host ?? null });
        await this.#reconcile(checkout);
      } finally {
        deleteState(this.#db, markKey);
        this.#scans.delete(path);
      }
    })();
    this.#scans.set(path, work);
    return work;
  }

  /** The open pull requests for finding conflicts: the listing of the last five minutes, else read now; none without `gh`. */
  #openPulls(checkout: CheckoutContext): Promise<readonly PullRequestSummary[]> {
    const repository = githubRepositoryOf(checkout.repositoryKey);
    if (repository === null) return Promise.resolve([]);
    const cached = this.#pulls.get(checkout.repositoryKey);
    if (cached !== undefined && this.#now() - cached.at < PULLS_REUSE_MS) return cached.pulls;
    const pulls = this.#githubReadiness(checkout.repositoryKey, false)
      .then(readiness => (readiness.state === "signed-in" ? this.#githubClient.openPullRequests(repository) : []))
      .catch((): PullRequestSummary[] => []);
    this.#pulls.set(checkout.repositoryKey, { at: this.#now(), pulls });
    return pulls;
  }

  /**
   * Find the checkout's in-flight conflicts for `session` (null outside a session), then hand the pass's events to the
   * conflict hook, without waiting for its handlers.
   */
  async #detect(checkout: CheckoutContext, session: { id: string; host: RerailHost | null } | null): Promise<void> {
    const pulls = await this.#openPulls(checkout);
    const detected = await detectConflicts({
      database: this.#db, checkout, pulls, sessionId: session?.id ?? null, now: this.#now,
    });
    if (detected !== null) this.#fire(checkout, conflictEvents(detected.before, detected.after), session);
  }

  async status(checkoutPath: string): Promise<MergeAssistStatus> {
    const checkout = this.#checkout(checkoutPath, false);
    const db = this.#db;
    const root = checkout.checkoutPath;
    const items = db.select().from(mergeAssistItems).where(eq(mergeAssistItems.checkoutPath, root)).all();
    const failed = activationFailures(db, root);
    const ownItems = new Map(items.filter(item => item.origin === "own").map(item => [item.candidateId, item]));
    const { candidates } = await readWorktreeCandidates(root);
    const committed = committedCandidates(root, candidates.map(candidate => candidate.id));
    const inForce = new Set(candidatesInForce(candidates).map(candidate => candidate.id));
    const own: OwnDecision[] = candidates.map(candidate => {
      const amended = candidates.find(entry => entry.id === candidate.supersedes);
      // Active but not in force: a later amendment replaced it.
      const amendedBy = candidate.state === "active" && !inForce.has(candidate.id)
        ? candidates.find(entry => entry.state === "active" && entry.supersedes === candidate.id)
        : undefined;
      const item = ownItems.get(candidate.id);
      return {
        id: candidate.id,
        decision: candidate.decision,
        generality: candidate.generality,
        scope: candidate.scope,
        level: candidate.rule !== undefined && failed[ownRuleId(root, candidate.id)] === undefined ? "code" : "note",
        state: candidate.state,
        createdAt: candidate.createdAt,
        committed: committed.has(candidate.id),
        amends: amended === undefined || amended.id === candidate.id ? null : { id: amended.id, decision: amended.decision },
        amendedBy: amendedBy?.id ?? null,
        source: { sessionId: item?.sessionId ?? null, at: item === undefined ? null : isoSeconds(item.createdAt) },
      };
    });
    // Items from other branches are never `rejected` (a withdrawn candidate's item is removed); own items are not findings.
    const findings = items
      .flatMap(item => (item.origin === "peer" && item.tier !== null && item.status !== "rejected" ? [findingOf(item, item.tier, item.status, failed)] : []))
      .sort((left, right) => TIER_ORDER[left.tier] - TIER_ORDER[right.tier] || right.updatedAt.localeCompare(left.updatedAt));
    const judge = this.#judge();
    const base = baseBranch(root);
    return {
      repositoryKey: checkout.repositoryKey,
      checkout: root,
      branch: currentBranch(root),
      baseBranch: base?.name ?? null,
      enabled: mergeAssistEnabled(db, checkout.repositoryKey),
      experimental: { conflictVerdicts: this.#hook.experiments.some(experiment => experiment.resolve !== undefined) },
      settings: readSettings(db, checkout.repositoryKey),
      readiness: {
        judge: judge.readiness(),
        github: await this.#githubReadiness(checkout.repositoryKey, false),
        candidatesIgnored: candidatesIgnored(root),
        rulesActive: rulesActive(db, checkout),
      },
      budget: judge.budget(checkout.repositoryKey),
      scanning: this.#scanning(root),
      lastScan: readState(db, STATE_KEYS.scan(root), lastScanSchema) ?? null,
      own,
      findings,
      conflicts: orderConflicts(readConflicts(db, root).map(entry => entry.conflict)),
    };
  }

  async setEnabled(checkoutPath: string, enabled: boolean): Promise<MergeAssistStatus> {
    const checkout = this.#checkout(checkoutPath, true);
    this.#db.update(repositories).set({ mergeAssist: enabled, updatedAt: this.#now() })
      .where(eq(repositories.repositoryKey, checkout.repositoryKey)).run();
    if (enabled) await this.#reconcile(checkout);
    else deactivateRepository(this.#db, checkout.repositoryKey);
    return await this.status(checkout.checkoutPath);
  }

  async scan(checkoutPath: string): Promise<MergeAssistStatus> {
    const checkout = this.#enabledCheckout(checkoutPath);
    await this.#scanOnce(checkout);
    return await this.status(checkout.checkoutPath);
  }

  async setCandidateState(checkoutPath: string, candidateId: string, state: CandidateState): Promise<MergeAssistStatus> {
    const checkout = this.#checkout(checkoutPath, true);
    const root = checkout.checkoutPath;
    const { candidates } = await readWorktreeCandidates(root);
    const candidate = candidates.find(entry => entry.id === candidateId);
    if (candidate === undefined) throw new MergeAssistError("not-found", `This branch has no decision ${candidateId} in .rerail/candidates/.`);
    if (candidate.state !== state) {
      const changed: Candidate = { ...candidate, state };
      await writeWorktreeCandidate(root, changed);
      this.#db.update(mergeAssistItems)
        .set({ status: state === "active" ? "applied" : "rejected", candidate: changed, candidateHash: candidateHash(changed), updatedAt: this.#now() })
        .where(and(eq(mergeAssistItems.checkoutPath, root), eq(mergeAssistItems.origin, "own"), eq(mergeAssistItems.candidateId, candidateId)))
        .run();
      await this.#reconcile(checkout);
    }
    return await this.status(root);
  }

  async resolveFinding(checkoutPath: string, findingId: string, action: FindingAction, actor: MergeAssistActor = "person"): Promise<MergeAssistStatus> {
    const checkout = this.#enabledCheckout(checkoutPath);
    const root = checkout.checkoutPath;
    const item = this.#db.select().from(mergeAssistItems)
      .where(and(eq(mergeAssistItems.id, findingId), eq(mergeAssistItems.checkoutPath, root), eq(mergeAssistItems.origin, "peer"))).get();
    if (item === undefined) throw new MergeAssistError("not-found", `This checkout has no finding ${findingId}.`);
    if (actor === "agent" && (action !== "dismiss" || item.status !== "open")) {
      throw new MergeAssistError(
        "invalid-state",
        action === "dismiss"
          ? `Finding ${item.id} is already applied or dismissed; only the person can undo it. Ask them to, on the Merge Assist page.`
          : `Only the person can ${action} finding ${item.id}. Ask them to, on the Merge Assist page.`,
      );
    }
    const next = nextStatus(item, action);
    if (typeof next !== "string") throw new MergeAssistError("invalid-state", next.refusal);
    const stored = item.evidence as Partial<ItemEvidence>;
    const { adopted: _adopted, ...unadopted } = stored;
    // An adoption is the person's: the scan keeps it until the candidate changes, where it re-checks an absorption.
    const evidence = action === "adopt" ? { ...stored, adopted: true } : unadopted;
    this.#db.update(mergeAssistItems).set({ status: next, evidence, updatedAt: this.#now() }).where(eq(mergeAssistItems.id, item.id)).run();
    await this.#reconcile(checkout);
    return await this.status(root);
  }

  async resolveConflict(checkoutPath: string, conflictId: string, action: ConflictAction, text?: string, actor: MergeAssistActor = "person"): Promise<MergeAssistStatus> {
    const checkout = this.#enabledCheckout(checkoutPath);
    const root = checkout.checkoutPath;
    const stored = this.#openConflict(root, conflictId, action, text, actor);
    const answered = action === "dismiss" ? dismissedConflict(stored, personName(root), this.#now()) : await this.#decide(checkout, stored, action, text);
    this.#db.transaction(tx => replaceConflict(tx, root, answered, this.#now()), { behavior: "immediate" });
    this.#fire(checkout, [{ event: "closed", conflict: answered }], null);
    if (action !== "dismiss") await this.#reconcile(checkout);
    return await this.status(root);
  }

  /** Why a person's answer cannot be taken at all: a coding agent never resolves a conflict; a verdict needs the experimental verdicts. */
  #refuse(action: ConflictAction, actor: MergeAssistActor): void {
    if (actor === "agent") {
      throw new MergeAssistError("invalid-state", "The person decides in-flight conflicts, not a coding agent. Tell them about it in one line; they resolve it on the Merge Assist page.");
    }
    if (action !== "dismiss" && !this.#hook.experiments.some(experiment => experiment.resolve !== undefined)) throw new MergeAssistError("invalid-state", VERDICTS_OFF);
  }

  /** The open conflict a person's answer settles, or why it cannot (see {@link #refuse}); a new decision needs its text. */
  #openConflict(root: string, conflictId: string, action: ConflictAction, text: string | undefined, actor: MergeAssistActor): StoredConflict {
    this.#refuse(action, actor);
    const stored = readConflicts(this.#db, root).find(entry => entry.conflict.id === conflictId);
    if (stored === undefined) throw new MergeAssistError("not-found", `This checkout has no conflict ${conflictId}.`);
    if (stored.conflict.status !== "open") throw new MergeAssistError("invalid-state", `Conflict ${conflictId} is already ${stored.conflict.status}.`);
    if (action === "new" && (text ?? "").trim() === "") throw new MergeAssistError("invalid-state", "A new decision needs its text.");
    return stored;
  }

  /** A person's verdict, through the experimental verdicts handler registered on the conflict hook. */
  async #decide(checkout: CheckoutContext, stored: StoredConflict, action: Exclude<ConflictAction, "dismiss">, text: string | undefined): Promise<StoredConflict> {
    const verdicts = this.#hook.experiments.find(experiment => experiment.resolve !== undefined);
    if (verdicts?.resolve === undefined) throw new MergeAssistError("invalid-state", VERDICTS_OFF);
    return await verdicts.resolve({ database: this.#db, checkout, now: this.#now, stored, action, text, name: personName(checkout.checkoutPath) });
  }

  async settings(checkoutPath: string): Promise<MergeAssistSettings> {
    return readSettings(this.#db, this.#checkout(checkoutPath, false).repositoryKey);
  }

  async updateSettings(checkoutPath: string, change: MergeAssistSettingsChange): Promise<MergeAssistStatus> {
    const checkout = this.#checkout(checkoutPath, true);
    this.#db.transaction(tx => {
      const settings = readSettings(tx, checkout.repositoryKey);
      writeSettings(tx, checkout.repositoryKey, { delivery: change.delivery ?? settings.delivery, hook: change.hook === undefined ? settings.hook : change.hook }, this.#now());
    }, { behavior: "immediate" });
    // The mode decides what the checkout's running sessions take at their next boundary.
    this.#emit(checkout.checkoutPath);
    return await this.status(checkout.checkoutPath);
  }

  async detectAfterEdit(checkoutPath: string, sessionId: string | null = null, host?: RerailHost): Promise<boolean> {
    const checkout = lookupCheckout(this.#db, checkoutPath);
    if (checkout === undefined || !mergeAssistEnabled(this.#db, checkout.repositoryKey)) return false;
    const last = this.#detectedAt.get(checkout.checkoutPath);
    if (last !== undefined && this.#now() - last < CONFLICT_DETECT_INTERVAL_MS) return false;
    this.#detectedAt.set(checkout.checkoutPath, this.#now());
    await this.#detect(checkout, sessionId === null ? null : { id: sessionId, host: host ?? null });
    return true;
  }

  async answerFromChat(checkoutPath: string, prompt: string, sessionId: string): Promise<ChatAnswer> {
    const chat = this.#hook.experiments.find(experiment => experiment.answer !== undefined);
    // Without the experimental chat answers, a prompt is read for nothing.
    if (chat?.answer === undefined) return { kind: "none" };
    const checkout = lookupCheckout(this.#db, checkoutPath);
    if (checkout === undefined || !mergeAssistEnabled(this.#db, checkout.repositoryKey)) return { kind: "none" };
    return await chat.answer({
      database: this.#db, checkout, now: this.#now, prompt, sessionId,
      resolve: async (conflictId, action, text) => {
        await this.resolveConflict(checkout.checkoutPath, conflictId, action, text, "person");
      },
    });
  }

  async openSession(checkoutPath: string, host: RerailHost, sessionId: string): Promise<void> {
    const checkout = lookupCheckout(this.#db, checkoutPath);
    if (checkout === undefined || !mergeAssistEnabled(this.#db, checkout.repositoryKey)) return;
    await this.#reconcile(checkout);
    await this.#scanOnce(checkout, host, sessionId);
  }

  async captureSession(session: MergeAssistSession): Promise<CaptureResult> {
    const checkout = lookupCheckout(this.#db, session.checkoutPath);
    if (checkout === undefined || !mergeAssistEnabled(this.#db, checkout.repositoryKey)) return { saved: [], unjudged: [] };
    const result = await captureCorrections({
      database: this.#db,
      checkout,
      judge: this.#judge(session.host),
      author: () => this.#author(checkout),
      now: this.#now,
    }, { sessionId: session.sessionId, events: session.events });
    if (result.saved.length > 0) await this.#reconcile(checkout);
    return result;
  }

  async scanSession(checkoutPath: string, host: RerailHost, sessionId: string): Promise<void> {
    const checkout = lookupCheckout(this.#db, checkoutPath);
    if (checkout === undefined || !mergeAssistEnabled(this.#db, checkout.repositoryKey)) return;
    await this.#scanOnce(checkout, host, sessionId);
  }

  /** How a session on `host` takes the checkout's conflict notes: the person's mode, the cards, and what the host draws. */
  #noteOptions(checkoutPath: string, host: RerailHost | undefined): ConflictNoteOptions {
    const checkout = lookupCheckout(this.#db, checkoutPath);
    const delivery: ConflictDelivery = checkout === undefined ? "both" : readSettings(this.#db, checkout.repositoryKey).delivery;
    const surfaces = host === undefined ? { personLine: true, statusLine: true } : { personLine: ADAPTER_CAPABILITIES[host].personLine, statusLine: ADAPTER_CAPABILITIES[host].statusLine };
    return { delivery, verdictCards: this.#hook.experiments.some(experiment => experiment.verdictCards === true), surfaces, now: this.#now() };
  }

  pendingNotes(sessionId: string, checkoutPath: string, inFlight: ReadonlySet<string> = new Set(), host?: RerailHost): PendingNote[] {
    return pendingNotes(this.#db, sessionId, checkoutPath, inFlight, {
      ...this.#noteOptions(checkoutPath, host), awaiting: this.#hook.awaiting(checkoutPath),
    });
  }

  stopNotes(sessionId: string, checkoutPath: string, call: EditCall, asks: boolean, inFlight: ReadonlySet<string> = new Set(), host?: RerailHost): PendingNote[] {
    return stopNotes(this.#db, sessionId, checkoutPath, call, asks, inFlight, this.#noteOptions(checkoutPath, host));
  }

  markNotesDelivered(sessionId: string, checkoutPath: string, marks: readonly string[]): void {
    markNotesDelivered(this.#db, sessionId, checkoutPath, marks, this.#now());
  }

  conflictNotice(checkoutPath: string, notice: ConflictNotice, host?: RerailHost): ConflictNotice {
    return currentNotice(this.#db, checkoutPath, notice, this.#noteOptions(checkoutPath, host));
  }

  async notesReleased(checkoutPath: string): Promise<void> {
    await this.#hook.released(checkoutPath);
  }

  hookHoldsNotes(checkoutPath: string): boolean {
    const settings = settingsAt(this.#db, checkoutPath);
    return settings.hook !== null && viaChat(settings.delivery);
  }

  commitStaging(checkoutPath: string, toolName: string, input: unknown): string | null {
    const command = shellCommandOf(toolName, input);
    const commit = command === null ? null : gitCommitIn(command);
    if (commit === null) return null;
    const files = unstagedDecisionFiles(checkoutPath, commit.all);
    return files.length === 0 ? null : commitStagingNote(files);
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

/** The status an action moves a finding to, or why it cannot. */
function nextStatus(item: MergeAssistItemRow, action: FindingAction): MergeAssistItemRow["status"] | { refusal: string } {
  if (action === "adopt") {
    return item.tier === "review" && item.status === "open"
      ? "applied"
      : { refusal: `Finding ${item.id} is not waiting for review, so there is nothing to adopt.` };
  }
  if (action === "dismiss") {
    return item.status === "open" || item.status === "applied"
      ? "dismissed"
      : { refusal: `Finding ${item.id} is already dismissed.` };
  }
  return item.status === "applied"
    ? "dismissed"
    : { refusal: `Finding ${item.id} is not applied, so there is nothing to undo.` };
}

/** A finding's evidence as stored with its item; empty where what was stored no longer parses. */
function findingEvidence(stored: Record<string, unknown>): Finding["evidence"] {
  const evidence = findingEvidenceSchema.safeParse({
    violations: stored.violations ?? [],
    conflicts: stored.conflicts ?? [],
    mine: stored.mine ?? null,
    judgment: stored.judgment ?? null,
  });
  return evidence.success ? evidence.data : { violations: [], conflicts: [], mine: null, judgment: null };
}

/** The branch or pull request a finding came from. */
function findingPeer(item: MergeAssistItemRow, stored: Record<string, unknown>): Finding["peer"] {
  return {
    kind: item.peerKind ?? "branch",
    ref: typeof stored.branch === "string" ? stored.branch : item.peerRef,
    commit: item.peerCommit ?? "",
    number: item.peerNumber,
    title: item.peerTitle,
    url: item.peerUrl,
    author: item.peerKind === "pull-request" ? item.peerAuthor : null,
    trusted: item.trusted,
  };
}

/** An item from another branch as the status shows it. */
function findingOf(item: MergeAssistItemRow, tier: FindingTier, status: FindingStatus, failed: Readonly<Record<string, string>>): Finding {
  const stored = item.evidence as Record<string, unknown>;
  const level = item.candidate.rule !== undefined && typeof stored.ruleProblem !== "string" && failed[itemRuleId(item)] === undefined ? "code" : "note";
  return {
    id: item.id,
    tier,
    status,
    candidate: {
      id: item.candidate.id,
      decision: item.candidate.decision,
      scope: item.candidate.scope,
      level,
      author: item.candidate.author,
      createdAt: item.candidate.createdAt,
    },
    peer: findingPeer(item, stored),
    reason: item.reason ?? "",
    evidence: findingEvidence(stored),
    message: item.message,
    updatedAt: isoSeconds(item.updatedAt),
  };
}

/**
 * Merge Assist over one database. Judgment runs the person's agent CLI and `gh` only on capture and scan; a status
 * read runs neither, apart from `gh auth status` at most every five minutes per repository.
 */
export function createMergeAssistService(options: MergeAssistServiceOptions): MergeAssistRuntime {
  return new DatabaseMergeAssist(options);
}
