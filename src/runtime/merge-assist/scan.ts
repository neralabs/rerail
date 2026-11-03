/**
 * The scan (docs/merge-assist-v1-build.md §5.3–5.4): the candidates on the repository's other local branches and open pull
 * requests, each sorted into a tier for this checkout. Exact checks come first: this branch's changes run against a
 * peer's code rule, and `git merge-tree` names the files the two branches would conflict on. The agent CLI is asked
 * once per peer whether its decisions contradict this branch's own, and only when this branch has decisions.
 *
 * Absorb: a trusted author, this branch's changes pass the rule, no contradiction; it applies. Review: this branch
 * breaks the rule, the rule does not compile here, there is no base branch to check it against, the author is not
 * trusted, or the judgment cannot tell; it waits for the person. Talk: a trusted author's decision contradicts one of
 * this branch's; both are shown, nothing applies (an untrusted author's contradiction waits in review). A duplicate of
 * one of this branch's decisions is not a finding.
 *
 * Trust: a pull request's author has write access. A local branch is trusted unless it tracks a remote other than
 * origin, or holds the commits of an open pull request whose author is not trusted (`gh pr checkout`); pull requests
 * are listed first so local branches can be checked against them.
 *
 * The person's dismissal and adoption hold until the candidate changes. An absorbed item is checked again at every
 * scan: once its author is no longer trusted or this branch breaks its rule, it goes back to review (and so unapplies).
 * A candidate its author rejected, or one whose branch or pull request is gone, is removed. A person's change made
 * while the scan ran wins over the scan's, and a scan that finishes after Merge Assist was turned off writes nothing.
 */
import { createHash } from "node:crypto";
import { and, eq } from "drizzle-orm";
import type { RerailDatabase } from "../../engine/storage/database.js";
import { mergeAssistItems, type MergeAssistItemRow, type NewMergeAssistItemRow } from "../../engine/storage/schema.js";
import type { CheckoutContext } from "../../engine/catalog/repository-model.js";
import { followsGitignore, mergeAssistEnabled } from "../../engine/catalog/repository-registry.js";
import { globPattern } from "../../engine/glob-pattern.js";
import type { CodeRule } from "../../engine/code-rule/code-rule-schema.js";
import { checkCodeRule } from "../../engine/code-rule/rule-check.js";
import { candidateHash, type Candidate } from "../../engine/merge-assist/candidate.js";
import { readTreeCandidates, readWorktreeCandidates, type CandidateSet } from "../../engine/merge-assist/candidate-files.js";
import {
  baseBranch,
  changedPaths,
  commitExists,
  currentBranch,
  githubRepositoryOf,
  isAncestor,
  localBranches,
  mergeTreeConflicts,
  type LocalBranch,
} from "../../engine/merge-assist/branch-git.js";
import type { CandidateLevel, FindingTier, GithubReadiness, LastScan } from "../../engine/merge-assist/model.js";
import { gitText } from "../../engine/git.js";
import { stableJsonText } from "../../engine/stable-json.js";
import type { AgentJudge } from "./agent-judge.js";
import type { AuthorPermission, GithubClient, GithubRepository, PullRequestSummary } from "./github.js";
import { compareJsonSchemaFor, comparePrompt, compareVerdictSchema, newJudgeNonce, type CompareInput, type CompareVerdict } from "./judge-prompts.js";
import { STATE_KEYS, isoSeconds, itemId, writeState } from "./state.js";

/** How many places in this branch's changes a peer's rule lists as broken. */
const MAX_VIOLATIONS = 20;
const WRITE_ACCESS: ReadonlySet<AuthorPermission> = new Set(["admin", "maintain", "write"]);
/** Remotes a trusted local branch may track besides none: a local branch (`.`), or the repository's origin. */
const TRUSTED_UPSTREAMS: Readonly<Record<string, true>> = { ".": true, origin: true };

export const NO_BASE_PROBLEM = "No base branch found (origin/HEAD, main or master); rules from other branches wait for review.";

export interface ScanContext {
  readonly database: RerailDatabase;
  readonly checkout: CheckoutContext;
  readonly judge: AgentJudge;
  readonly github: GithubClient;
  /** `gh`'s readiness for the repository, read now. */
  readonly githubReadiness: () => Promise<GithubReadiness>;
  readonly now: () => number;
}

/** The evidence an item keeps: the finding's, plus why its rule cannot be checked here and the peer's branch name. */
export interface ItemEvidence {
  readonly violations: Array<{ path: string; line: number; message: string }>;
  readonly conflicts: string[];
  readonly mine: { id: string; decision: string } | null;
  readonly judgment: { verdict: string; explanation: string } | null;
  /** Why the candidate's code rule cannot be checked here (it does not compile); null when it can, or has none. */
  readonly ruleProblem: string | null;
  /** The branch the candidate is on: a local branch's name, or a pull request's head branch. */
  readonly branch: string;
  /** Set when the person adopted it from review: it holds until the candidate changes, unlike an absorption. */
  readonly adopted?: true;
}

/** Whether a person adopted the item: its status is theirs, not the scan's. */
export function adoptedByPerson(item: Pick<MergeAssistItemRow, "status" | "evidence">): boolean {
  return item.status === "applied" && (item.evidence as Partial<ItemEvidence>).adopted === true;
}

interface Peer {
  readonly kind: "branch" | "pull-request";
  /** The item key's peer part: the branch name, or `pull/<number>` (a fork's branch may share a local name). */
  readonly ref: string;
  readonly branch: string;
  readonly commit: string;
  readonly number: number | null;
  readonly title: string | null;
  readonly url: string | null;
  /** GitHub's author of the pull request; null for a local branch, where only the candidate names one. */
  readonly author: string | null;
  /** Null when trusted; else why not, as one sentence. */
  readonly distrust: string | null;
  readonly candidates: readonly Candidate[];
}

interface RuleCheck {
  readonly level: CandidateLevel;
  readonly problem: string | null;
  readonly violations: ItemEvidence["violations"];
}

/** This branch's changed paths, or why they are not known (a rule cannot be checked against them then). */
type Changes = { readonly paths: readonly string[] } | { readonly paths: null; readonly reason: string };

type Relation = CompareVerdict["items"][number];
type Comparison = { readonly ok: true; readonly relations: ReadonlyMap<string, Relation> } | { readonly ok: false; readonly reason: string };

function message(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function readProblems(peer: Pick<Peer, "kind" | "branch" | "number">, set: CandidateSet): string[] {
  const name = peer.kind === "branch" ? `Branch ${peer.branch}` : `Pull request #${peer.number}`;
  return set.problems.map(problem => `${name} has a candidate Rerail could not read (${problem}).`);
}

function githubProblem(readiness: GithubReadiness): string | null {
  if (readiness.state === "missing") return "gh is not installed, so Rerail did not read open pull requests.";
  if (readiness.state === "signed-out") return "gh is not signed in, so Rerail did not read open pull requests.";
  return null;
}

function distrustOf(author: string, permission: AuthorPermission): string | null {
  if (WRITE_ACCESS.has(permission)) return null;
  return permission === "unknown"
    ? `Rerail could not tell whether ${author} has write access to the repository.`
    : `${author} has no write access to the repository.`;
}

/** Run the peer's rule over this branch's changes in its scope: where they break it, or why it cannot run here. */
async function checkPeerRule(candidate: Candidate, root: string, changes: Changes, followGitignore: boolean): Promise<RuleCheck> {
  const rule = candidate.rule as CodeRule | undefined;
  if (rule === undefined) return { level: "note", problem: null, violations: [] };
  const inScope = globPattern(candidate.scope);
  const paths = (changes.paths ?? []).filter(path => inScope.test(path));
  try {
    const result = await checkCodeRule(rule, { root, cases: [], paths, maxMatches: MAX_VIOLATIONS, followGitignore });
    if (!result.valid) return { level: "note", problem: result.error ?? "it does not compile", violations: [] };
    return { level: "code", problem: null, violations: result.matches.map(({ path, line, message: text }) => ({ path, line, message: text })) };
  } catch (error) {
    return { level: "note", problem: message(error), violations: [] };
  }
}

/** One judgment per peer: how each of its fresh candidates relates to this branch's active decisions. */
async function compare(
  context: ScanContext,
  mine: readonly Candidate[],
  theirs: readonly Candidate[],
  conflicts: readonly string[],
): Promise<Comparison> {
  if (mine.length === 0) return { ok: true, relations: new Map() };
  const input: CompareInput = {
    mine: mine.map(({ id, decision, scope }) => ({ id, decision, scope })),
    theirs: theirs.map(({ id, decision, scope, author }) => ({ id, decision, scope, author })),
    conflicts: [...conflicts],
  };
  const outcome = await context.judge.judge({
    kind: "compare",
    repositoryKey: context.checkout.repositoryKey,
    prompt: comparePrompt(input, newJudgeNonce()),
    jsonSchema: compareJsonSchemaFor(input),
    parse: value => compareVerdictSchema.parse(value),
    cacheKey: createHash("sha256").update(`compare\0${stableJsonText(input) ?? ""}`).digest("hex"),
  });
  if (!outcome.ok) {
    const reason = outcome.reason === "unavailable"
      ? "No agent CLI was available to compare it with this branch's decisions."
      : outcome.reason === "budget"
        ? "The hour's judgment budget ran out before Rerail could compare it with this branch's decisions."
        : "Your agent's CLI did not answer usably when comparing it with this branch's decisions.";
    return { ok: false, reason };
  }
  return { ok: true, relations: new Map(outcome.value.items.map(item => [item.candidateId, item])) };
}

interface Triage {
  readonly tier: FindingTier;
  readonly reason: string;
  readonly mine: ItemEvidence["mine"];
  readonly judgment: ItemEvidence["judgment"];
  readonly message: string | null;
}

/** Why the exact checks hold a candidate for review (its rule, this branch's changes, its author), or null when they pass. */
function exactReview(peer: Peer, candidate: Candidate, check: RuleCheck, changes: Changes): string | null {
  if (check.problem !== null) return "Its code rule does not compile here, so Rerail cannot check it exactly.";
  if (candidate.rule !== undefined && changes.paths === null) return changes.reason;
  if (check.violations.length > 0) return "Your branch's changes break it.";
  return peer.distrust;
}

/** The tier for one fresh candidate, or undefined for a duplicate of one of this branch's decisions. */
function triage(peer: Peer, candidate: Candidate, check: RuleCheck, changes: Changes, comparison: Comparison, mine: readonly Candidate[]): Triage | undefined {
  let judgment: ItemEvidence["judgment"] = null;
  if (!comparison.ok) return { tier: "review", reason: comparison.reason, mine: null, judgment, message: null };
  if (mine.length > 0) {
    const relation = comparison.relations.get(candidate.id);
    const ours = relation?.mineId === null || relation?.mineId === undefined ? undefined : mine.find(own => own.id === relation.mineId);
    if (relation !== undefined) judgment = { verdict: relation.relation, explanation: relation.explanation };
    if (relation?.relation === "duplicate" && ours !== undefined) return undefined;
    if (relation?.relation === "contradicts" && ours !== undefined) {
      const contradicted = { id: ours.id, decision: ours.decision };
      // An untrusted author's text never reaches the agent: the contradiction waits for the person, counted.
      if (peer.distrust !== null) return { tier: "review", reason: peer.distrust, mine: contradicted, judgment, message: null };
      return { tier: "talk", reason: "It contradicts a decision on this branch.", mine: contradicted, judgment, message: relation.message };
    }
    if (relation === undefined || relation.relation !== "compatible") {
      return { tier: "review", reason: "Your agent's CLI could not tell whether it agrees with this branch's decisions.", mine: null, judgment, message: null };
    }
  }
  const held = exactReview(peer, candidate, check, changes);
  if (held !== null) return { tier: "review", reason: held, mine: null, judgment, message: null };
  const trust = peer.kind === "branch" ? "It comes from a branch on this machine" : `${peer.author} has write access to the repository`;
  return {
    tier: "absorb",
    reason: check.level === "code" ? `${trust}, and your branch's changes pass its rule.` : `${trust}.`,
    mine: null,
    judgment,
    message: null,
  };
}

/** The open pull requests, read first; `listed` is false when they could not be read (no `gh`, or it failed). */
interface PullRequestList {
  readonly repository: GithubRepository | null;
  readonly pulls: readonly PullRequestSummary[];
  readonly listed: boolean;
}

async function listPullRequests(context: ScanContext, problems: string[]): Promise<PullRequestList> {
  const repository = githubRepositoryOf(context.checkout.repositoryKey);
  if (repository === null) return { repository, pulls: [], listed: true };
  const problem = githubProblem(await context.githubReadiness());
  if (problem !== null) {
    problems.push(problem);
    return { repository, pulls: [], listed: false };
  }
  try {
    return { repository, pulls: await context.github.openPullRequests(repository), listed: true };
  } catch (error) {
    problems.push(`Rerail could not list open pull requests: ${message(error)}`);
    return { repository, pulls: [], listed: false };
  }
}

/** Why a pull request's author is not trusted, or null; each author's permission is read once per scan. */
type AuthorDistrust = (author: string) => Promise<string | null>;

function authorDistrust(context: ScanContext, repository: GithubRepository | null): AuthorDistrust {
  const permissions = new Map<string, Promise<AuthorPermission>>();
  return async author => {
    let permission = permissions.get(author);
    if (permission === undefined) {
      permission = repository === null
        ? Promise.resolve<AuthorPermission>("unknown")
        : context.github.authorPermission(repository, author).catch((): AuthorPermission => "unknown");
      permissions.set(author, permission);
    }
    return distrustOf(author, await permission);
  };
}

/**
 * Why a local branch is not trusted: it tracks a remote other than origin, or it holds an open pull request's commits
 * (its tip is the pull request's head, contains it, or is contained in it) and that pull request's author is not
 * trusted. Null when neither holds.
 */
async function localDistrust(root: string, branch: LocalBranch, list: PullRequestList, distrust: AuthorDistrust, present: Map<string, boolean>): Promise<string | null> {
  if (branch.upstream !== null && TRUSTED_UPSTREAMS[branch.upstream] !== true) {
    return `Branch ${branch.name} tracks a remote other than origin, so Rerail cannot tell whether its author has write access to the repository.`;
  }
  for (const pull of list.pulls) {
    let holds = pull.headCommit === branch.commit;
    if (!holds) {
      let exists = present.get(pull.headCommit);
      if (exists === undefined) {
        exists = commitExists(root, pull.headCommit);
        present.set(pull.headCommit, exists);
      }
      holds = exists && (isAncestor(root, pull.headCommit, branch.commit) || isAncestor(root, branch.commit, pull.headCommit));
    }
    if (!holds) continue;
    const reason = await distrust(pull.author);
    if (reason !== null) return `It holds the commits of pull request #${pull.number}, and ${reason.charAt(0).toLowerCase()}${reason.slice(1)}`;
  }
  return null;
}

/**
 * Whether another branch's tip is stacked with this branch, so its decisions are this branch's own: its tip is
 * already in this branch, or it builds on commits this branch added. A branch that merely starts where this one
 * does (this branch has no commits of its own past the base) is not stacked on it.
 */
function stackedWith(root: string, head: string, commit: string, base: { ref: string } | null): boolean {
  if (isAncestor(root, commit, head)) return true;
  if (!isAncestor(root, head, commit)) return false;
  return base === null || !isAncestor(root, head, base.ref);
}

/** The local branches to read: not this one or the base, not merged into the base, not stacked with this branch. */
async function localPeers(
  root: string,
  head: string,
  current: string | null,
  base: { ref: string } | null,
  list: PullRequestList,
  distrust: AuthorDistrust,
  problems: string[],
): Promise<Peer[]> {
  const peers: Peer[] = [];
  const present = new Map<string, boolean>();
  for (const branch of localBranches(root)) {
    if (branch.name === current || `refs/heads/${branch.name}` === base?.ref) continue;
    if (base !== null && isAncestor(root, branch.commit, base.ref)) continue;
    if (stackedWith(root, head, branch.commit, base)) continue;
    const peer = { kind: "branch" as const, branch: branch.name, number: null };
    const set = await readTreeCandidates(root, branch.commit);
    problems.push(...readProblems(peer, set));
    peers.push({
      ...peer,
      ref: branch.name,
      commit: branch.commit,
      title: null,
      url: null,
      author: null,
      distrust: set.candidates.length === 0 ? null : await localDistrust(root, branch, list, distrust, present),
      candidates: set.candidates,
    });
  }
  return peers;
}

interface PullRequestPeers {
  readonly peers: Peer[];
  /** False when the list itself could not be read: every known pull request's items stay as they are. */
  readonly listed: boolean;
  /** Pull requests whose candidates could not be read: their items stay as they are. */
  readonly unread: Set<string>;
}

/** The open pull requests to read: not this branch's, not stacked with it, not a local branch already read. */
async function pullRequestPeers(
  context: ScanContext,
  list: PullRequestList,
  head: string,
  current: string | null,
  base: { ref: string } | null,
  local: readonly Peer[],
  distrust: AuthorDistrust,
  problems: string[],
): Promise<PullRequestPeers> {
  const unread = new Set<string>();
  const { repository } = list;
  if (repository === null || !list.listed) return { peers: [], listed: list.listed, unread };
  const root = context.checkout.checkoutPath;
  const localCommits = new Set(local.map(peer => peer.commit));
  const peers: Peer[] = [];
  for (const pull of list.pulls) {
    if (pull.headRefName === current || pull.baseRefName === current || localCommits.has(pull.headCommit)) continue;
    if (commitExists(root, pull.headCommit) && stackedWith(root, head, pull.headCommit, base)) continue;
    const ref = `pull/${pull.number}`;
    const named = { kind: "pull-request" as const, branch: pull.headRefName, number: pull.number };
    let set: CandidateSet;
    try {
      set = await context.github.candidatesAt(repository, pull.headCommit);
    } catch (error) {
      problems.push(`Rerail could not read the candidates of pull request #${pull.number}: ${message(error)}`);
      unread.add(ref);
      continue;
    }
    problems.push(...readProblems(named, set));
    peers.push({
      ...named,
      ref,
      commit: pull.headCommit,
      title: pull.title,
      url: pull.url,
      author: pull.author,
      distrust: set.candidates.length === 0 ? null : await distrust(pull.author),
      candidates: set.candidates,
    });
  }
  return { peers, listed: true, unread };
}

const itemKey = (peerRef: string, candidateId: string): string => `${peerRef}\0${candidateId}`;

/** Whether the row changed since the scan read it (a person's adopt, dismiss or undo, or another writer): the scan leaves it. */
function changedSince(read: MergeAssistItemRow | undefined, now: MergeAssistItemRow | undefined): boolean {
  if (read === undefined || now === undefined) return read !== now;
  return now.status !== read.status || now.updatedAt !== read.updatedAt || now.candidateHash !== read.candidateHash;
}

/** The checkout's items from other branches, by peer and candidate. */
function peerItems(db: Pick<RerailDatabase, "select">, checkoutPath: string): Map<string, MergeAssistItemRow> {
  return new Map(db.select().from(mergeAssistItems)
    .where(and(eq(mergeAssistItems.checkoutPath, checkoutPath), eq(mergeAssistItems.origin, "peer"))).all()
    .map(item => [itemKey(item.peerRef, item.candidateId), item]));
}

/**
 * Read the peers, sort their candidates into tiers and store them as this checkout's items; then record the scan.
 * Applying what was absorbed is the caller's (`reconcileCheckout`).
 */
export async function scanPeers(context: ScanContext): Promise<LastScan> {
  const { database, checkout } = context;
  const root = checkout.checkoutPath;
  const problems: string[] = [];
  const head = gitText(root, ["rev-parse", "--verify", "--quiet", "HEAD^{commit}"]);
  if (head === undefined) {
    const scan: LastScan = { at: isoSeconds(context.now()), peers: { branches: 0, pullRequests: 0 }, problems: ["This branch has no commits yet, so Rerail has nothing to compare it with."] };
    database.transaction(tx => {
      if (mergeAssistEnabled(tx, checkout.repositoryKey)) writeState(tx, STATE_KEYS.scan(root), scan, context.now());
    }, { behavior: "immediate" });
    return scan;
  }
  const current = currentBranch(root);
  const base = baseBranch(root);
  const list = await listPullRequests(context, problems);
  const distrust = authorDistrust(context, list.repository);
  const local = await localPeers(root, head, current, base, list, distrust, problems);
  const pulls = await pullRequestPeers(context, list, head, current, base, local, distrust, problems);

  const own = await readWorktreeCandidates(root);
  const ownIds = new Set(own.candidates.map(candidate => candidate.id));
  const mine = own.candidates.filter(candidate => candidate.state === "active");
  let changes: Changes;
  if (base === null) {
    problems.push(NO_BASE_PROBLEM);
    changes = { paths: null, reason: "Rerail found no base branch, so it could not check your branch's changes against its rule." };
  } else {
    try {
      changes = { paths: changedPaths(root, base.ref, head, { workingTree: true }) };
    } catch {
      problems.push("Rerail could not list this branch's changes; rules from other branches wait for review.");
      changes = { paths: null, reason: "Rerail could not list your branch's changes, so it could not check them against its rule." };
    }
  }
  const followGitignore = followsGitignore(database, checkout);
  const existing = peerItems(database, root);

  const writes: NewMergeAssistItemRow[] = [];
  const kept = new Set<string>();
  const versions = new Set<string>();
  for (const peer of [...local, ...pulls.peers]) {
    const fresh: Array<{ candidate: Candidate; hash: string; prior: MergeAssistItemRow | undefined }> = [];
    const peerFields = { peerCommit: peer.commit, peerTitle: peer.title, peerUrl: peer.url, peerAuthor: peer.author, trusted: peer.distrust === null };
    for (const candidate of peer.candidates) {
      if (ownIds.has(candidate.id) || candidate.state !== "active") continue;
      const hash = candidateHash(candidate);
      // The same version on a local branch and its pull request is one finding: the local branch's, read first.
      if (versions.has(`${candidate.id}\0${hash}`)) continue;
      versions.add(`${candidate.id}\0${hash}`);
      const key = itemKey(peer.ref, candidate.id);
      const prior = existing.get(key);
      if (prior !== undefined && prior.candidateHash === hash && (prior.status === "dismissed" || adoptedByPerson(prior))) {
        // The person's dismissal, adoption or undo holds until the candidate changes.
        kept.add(key);
        writes.push({ ...prior, ...peerFields });
        continue;
      }
      // Everything else is judged again: an absorption holds only while trust, this branch's changes, its rule and
      // this branch's own decisions (a new one may contradict it) all still allow it. The comparison is cached.
      fresh.push({ candidate, hash, prior });
    }
    if (fresh.length === 0) continue;
    const conflicts = commitExists(root, peer.commit) ? mergeTreeConflicts(root, head, peer.commit) ?? [] : [];
    const checks = new Map<string, RuleCheck>();
    for (const { candidate } of fresh) checks.set(candidate.id, await checkPeerRule(candidate, root, changes, followGitignore));
    const scopes = globPattern(fresh.flatMap(({ candidate }) => candidate.scope));
    const comparison = await compare(context, mine, fresh.map(({ candidate }) => candidate), conflicts.filter(path => scopes.test(path)));
    const judged = context.now();
    for (const { candidate, hash, prior } of fresh) {
      const check = checks.get(candidate.id)!;
      const tier = triage(peer, candidate, check, changes, comparison, mine);
      if (tier === undefined) continue;
      const inScope = globPattern(candidate.scope);
      const evidence: ItemEvidence = {
        violations: check.violations,
        conflicts: conflicts.filter(path => inScope.test(path)),
        mine: tier.mine,
        judgment: tier.judgment,
        ruleProblem: check.problem,
        branch: peer.branch,
      };
      kept.add(itemKey(peer.ref, candidate.id));
      writes.push({
        id: itemId(root, "peer", peer.ref, candidate.id),
        checkoutPath: root,
        repositoryKey: checkout.repositoryKey,
        candidateId: candidate.id,
        candidateHash: hash,
        origin: "peer",
        peerKind: peer.kind,
        peerRef: peer.ref,
        peerNumber: peer.number,
        ...peerFields,
        tier: tier.tier,
        status: tier.tier === "absorb" ? "applied" : "open",
        candidate,
        evidence: { ...evidence },
        reason: tier.reason,
        message: tier.message,
        sessionId: null,
        eventSequence: null,
        createdAt: prior?.createdAt ?? judged,
        updatedAt: judged,
      });
    }
  }

  const scan: LastScan = { at: isoSeconds(context.now()), peers: { branches: local.length, pullRequests: pulls.peers.length }, problems };
  database.transaction(tx => {
    // Turned off while the scan ran: its findings were removed, and nothing here brings them back.
    if (!mergeAssistEnabled(tx, checkout.repositoryKey)) return;
    const latest = peerItems(tx, root);
    for (const row of writes) {
      const key = itemKey(row.peerRef ?? "", row.candidateId);
      if (changedSince(existing.get(key), latest.get(key))) continue;
      const { id: _id, createdAt: _createdAt, ...changes } = row;
      tx.insert(mergeAssistItems).values(row).onConflictDoUpdate({
        target: [mergeAssistItems.checkoutPath, mergeAssistItems.origin, mergeAssistItems.peerRef, mergeAssistItems.candidateId],
        set: changes,
      }).run();
    }
    for (const [key, item] of existing) {
      if (kept.has(key) || pulls.unread.has(item.peerRef) || (item.peerKind === "pull-request" && !pulls.listed)) continue;
      if (changedSince(item, latest.get(key))) continue;
      tx.delete(mergeAssistItems).where(eq(mergeAssistItems.id, item.id)).run();
    }
    writeState(tx, STATE_KEYS.scan(root), scan, context.now());
  }, { behavior: "immediate" });
  return scan;
}
