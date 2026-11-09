/**
 * Merge Assist's facade: what the runtime, the API and the CLI call for one checkout. See docs/merge-assist-v1-build.md.
 */
import { and, eq } from "drizzle-orm";
import { z } from "zod";
import type { RerailHost } from "../../engine/adapter-contract.js";
import type { CheckoutContext, RuleCatalog } from "../../engine/catalog/repository-model.js";
import { repositoryIdentityLocation } from "../../engine/catalog/repository-identity.js";
import { lookupCheckout, mergeAssistEnabled, recordCheckout } from "../../engine/catalog/repository-registry.js";
import { gitText } from "../../engine/git.js";
import { baseBranch, currentBranch, githubRepositoryOf } from "../../engine/merge-assist/branch-git.js";
import { candidateHash, candidatesInForce, type Candidate, type CandidateState } from "../../engine/merge-assist/candidate.js";
import { candidatesIgnored, committedCandidates, readWorktreeCandidates, writeWorktreeCandidate } from "../../engine/merge-assist/candidate-files.js";
import { decisionLogItems, orderConflicts } from "../../engine/merge-assist/conflicts.js";
import { readDecisions } from "../../engine/merge-assist/decision-log.js";
import {
  findingSchema,
  githubReadinessSchema,
  lastScanSchema,
  type ConflictAction,
  type Finding,
  type FindingAction,
  type FindingStatus,
  type FindingTier,
  type GithubReadiness,
  type MergeAssistStatus,
  type OwnDecision,
} from "../../engine/merge-assist/model.js";
import type { RerailDatabase } from "../../engine/storage/database.js";
import { mergeAssistItems, repositories, type MergeAssistItemRow } from "../../engine/storage/schema.js";
import { activationFailures, deactivateRepository, itemRuleId, ownRuleId, reconcileCheckout, rulesActive } from "./activation.js";
import { createAgentJudge, type AgentJudge } from "./agent-judge.js";
import { captureCorrections, type CaptureResult, type SessionEventSource } from "./capture.js";
import { detectConflicts } from "./conflict-detect.js";
import { answerConflict } from "./conflict-resolve.js";
import { readConflicts, replaceConflict } from "./conflict-store.js";
import { createGithubClient, type GithubClient, type PullRequestSummary } from "./github.js";
import { markNotesDelivered, pendingNotes, type PendingNote } from "./notes.js";
import { authorName, osUser, personName } from "./people.js";
import { scanPeers, type ItemEvidence } from "./scan.js";
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
   * Settle an open in-flight conflict: keep this branch's side, take theirs, write a new decision (`text` required),
   * or dismiss it as not a conflict. Each verdict appends to the decision log and writes a decision on this branch.
   * Only a person decides: a coding agent is refused.
   */
  resolveConflict(checkoutPath: string, conflictId: string, action: ConflictAction, text?: string, actor?: MergeAssistActor): Promise<MergeAssistStatus>;
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
  openSession(checkoutPath: string, host: RerailHost): Promise<void>;
  /** A turn ended: capture the session's new corrections and apply a saved decision. The scan that follows is {@link scanSession}. */
  captureSession(session: MergeAssistSession): Promise<CaptureResult>;
  /** Scan the other branches for a session's checkout, as after a saved decision (it may contradict another branch's). */
  scanSession(checkoutPath: string, host: RerailHost): Promise<void>;
  /** The notes for the session it has not been handed, leaving out items whose marks are in `inFlight`. Reads only. */
  pendingNotes(sessionId: string, checkoutPath: string, inFlight?: ReadonlySet<string>): PendingNote[];
  /** Record notes as handed to the session, once a delivery took them. */
  markNotesDelivered(sessionId: string, checkoutPath: string, marks: readonly string[]): void;
  /**
   * The agent edited files in the checkout: find its in-flight conflicts again, at most once per
   * {@link CONFLICT_DETECT_INTERVAL_MS} per checkout. Resolves to whether it ran.
   */
  detectAfterEdit(checkoutPath: string): Promise<boolean>;
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
}

/** How long `gh`'s readiness is reused before `gh auth status` runs again; a scan always reads it afresh. */
const GITHUB_READINESS_MS = 5 * 60_000;
/** A scan marked running for longer than this was cut short (its process died); another may start. */
const SCAN_STALE_MS = 10 * 60_000;
/** The least time between two detections of a checkout's conflicts after the agent's edits. */
export const CONFLICT_DETECT_INTERVAL_MS = 30_000;
/** How long a listing of open pull requests is reused for finding conflicts; a scan lists them afresh. */
const PULLS_REUSE_MS = 5 * 60_000;
const TIER_ORDER: Readonly<Record<FindingTier, number>> = { talk: 0, review: 1, absorb: 2 };

const githubCacheSchema = z.object({ at: z.number(), readiness: githubReadinessSchema });
const scanningSchema = z.object({ since: z.number() });
const findingEvidenceSchema = findingSchema.shape.evidence;

class DatabaseMergeAssist implements MergeAssistRuntime {
  readonly #options: MergeAssistServiceOptions;
  readonly #now: () => number;
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

  constructor(options: MergeAssistServiceOptions) {
    this.#options = options;
    this.#now = options.now ?? Date.now;
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

  /** Scan the checkout once at a time across processes; a request while one runs joins it here, or leaves it be elsewhere. */
  #scanOnce(checkout: CheckoutContext, host?: RerailHost): Promise<void> {
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
        await this.#detect(checkout);
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

  /** Find the checkout's in-flight conflicts; apply what precedent decided for this branch. */
  async #detect(checkout: CheckoutContext): Promise<void> {
    const pulls = await this.#openPulls(checkout);
    const decided = await detectConflicts({ database: this.#db, checkout, pulls, now: this.#now });
    if (decided) await this.#reconcile(checkout);
  }

  /** This branch's in-flight conflicts as the status lists them, and the decision log newest first. */
  async #conflictsView(root: string): Promise<Pick<MergeAssistStatus, "conflicts" | "decisionLog">> {
    const stored = readConflicts(this.#db, root);
    const log = await readDecisions(root);
    const titles = new Map(stored.map(entry => [entry.conflict.question, entry.conflict.title]));
    const mine = currentBranch(root) ?? gitText(root, ["rev-parse", "--verify", "--quiet", "HEAD^{commit}"]) ?? null;
    return {
      conflicts: orderConflicts(stored.map(entry => entry.conflict)),
      decisionLog: { entries: decisionLogItems(log.entries, mine, titles), problems: log.problems },
    };
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
      ...await this.#conflictsView(root),
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
    if (actor === "agent") {
      throw new MergeAssistError("invalid-state", "The person decides in-flight conflicts, not a coding agent. Tell them about it in one line; they resolve it on the Merge Assist page.");
    }
    const stored = readConflicts(this.#db, root).find(entry => entry.conflict.id === conflictId);
    if (stored === undefined) throw new MergeAssistError("not-found", `This checkout has no conflict ${conflictId}.`);
    if (stored.conflict.status !== "open") throw new MergeAssistError("invalid-state", `Conflict ${conflictId} is already ${stored.conflict.status}.`);
    if (action === "new" && (text ?? "").trim() === "") throw new MergeAssistError("invalid-state", "A new decision needs its text.");
    const context = { database: this.#db, checkout, now: this.#now };
    const answered = await answerConflict(context, stored, { action, text, name: personName(root) });
    this.#db.transaction(tx => replaceConflict(tx, root, answered, this.#now()), { behavior: "immediate" });
    if (action !== "dismiss") await this.#reconcile(checkout);
    return await this.status(root);
  }

  async detectAfterEdit(checkoutPath: string): Promise<boolean> {
    const checkout = lookupCheckout(this.#db, checkoutPath);
    if (checkout === undefined || !mergeAssistEnabled(this.#db, checkout.repositoryKey)) return false;
    const last = this.#detectedAt.get(checkout.checkoutPath);
    if (last !== undefined && this.#now() - last < CONFLICT_DETECT_INTERVAL_MS) return false;
    this.#detectedAt.set(checkout.checkoutPath, this.#now());
    await this.#detect(checkout);
    return true;
  }

  async openSession(checkoutPath: string, host: RerailHost): Promise<void> {
    const checkout = lookupCheckout(this.#db, checkoutPath);
    if (checkout === undefined || !mergeAssistEnabled(this.#db, checkout.repositoryKey)) return;
    await this.#reconcile(checkout);
    await this.#scanOnce(checkout, host);
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

  async scanSession(checkoutPath: string, host: RerailHost): Promise<void> {
    const checkout = lookupCheckout(this.#db, checkoutPath);
    if (checkout === undefined || !mergeAssistEnabled(this.#db, checkout.repositoryKey)) return;
    await this.#scanOnce(checkout, host);
  }

  pendingNotes(sessionId: string, checkoutPath: string, inFlight: ReadonlySet<string> = new Set()): PendingNote[] {
    return pendingNotes(this.#db, sessionId, checkoutPath, inFlight);
  }

  markNotesDelivered(sessionId: string, checkoutPath: string, marks: readonly string[]): void {
    markNotesDelivered(this.#db, sessionId, checkoutPath, marks, this.#now());
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

/** An item from another branch as the status shows it. */
function findingOf(item: MergeAssistItemRow, tier: FindingTier, status: FindingStatus, failed: Readonly<Record<string, string>>): Finding {
  const stored = item.evidence as Record<string, unknown>;
  const evidence = findingEvidenceSchema.safeParse({
    violations: stored.violations ?? [],
    conflicts: stored.conflicts ?? [],
    mine: stored.mine ?? null,
    judgment: stored.judgment ?? null,
  });
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
    peer: {
      kind: item.peerKind ?? "branch",
      ref: typeof stored.branch === "string" ? stored.branch : item.peerRef,
      commit: item.peerCommit ?? "",
      number: item.peerNumber,
      title: item.peerTitle,
      url: item.peerUrl,
      author: item.peerKind === "pull-request" ? item.peerAuthor : null,
      trusted: item.trusted,
    },
    reason: item.reason ?? "",
    evidence: evidence.success ? evidence.data : { violations: [], conflicts: [], mine: null, judgment: null },
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
