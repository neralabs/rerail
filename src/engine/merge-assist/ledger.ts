/**
 * The decision ledger: computed from Git, never written. Entries are files on the branches whose people decided them
 * (`decision-entry.ts`); the view reads the base branch's tree, this branch (HEAD's tree and the working files) and
 * every live ref the conflict detector lists (local and remote-tracking branches, open pull request heads held
 * locally), deduplicated by id. An entry's status comes from where it sits:
 *
 * - in the base tree: `in-force`, unless a later base entry on its question supersedes it (`superseded`; a precedent
 *   copy never supersedes the verdict it followed) or the winner's position was not in the base where the entry
 *   merged (`flagged`: the merge contradicted it);
 * - only on this branch or a live ref: `provisional`, or `reopened` when the winner's position at its committed tip no
 *   longer has the fingerprint the entry stored (never for the checkout whose branch holds the winning position: it
 *   tells the side that followed); one another entry from the same place supersedes is `superseded`;
 * - seen before (the checkout's memory) and now on no live ref and not in the base: `void`. This is the merge check: a
 *   ref that merged into the base leaves the live refs, so each of its entries ends in force, flagged or void.
 *
 * Reads Git only, and little of it: live refs come from one `for-each-ref --no-merged`, each tip's decisions folder
 * from one `cat-file --batch-check`, and a folder tree already read, a merge already checked or a winner tip already
 * compared comes from the memory, which the runtime keeps.
 */
import { z } from "zod";
import { gitText, runGit } from "../git.js";
import { baseBranch, commitExists, currentBranch, isAncestor, remoteBranches } from "./branch-git.js";
import { otherSide, SIDES, type SideId, type StepResult } from "./conflict-record.js";
import { readTreeCandidates } from "./candidate-files.js";
import { decisionEntrySchema, deciderName, type DecisionEntry, decisionFileName, DECISIONS_DIRECTORY, type Follows, winnerKey, winnerOf } from "./decision-entry.js";
import { positionFingerprint, readDecisionFolder, readWorktreeDecisions } from "./decision-files.js";
import { type DecisionLogItem, type DecisionLogQuestion, LEDGER_STATUSES, type LedgerStatus } from "./model.js";

/** How many entries the status lists. */
export const LEDGER_ITEMS_SHOWN = 50;
/** How many entries no longer in Git the memory keeps, newest first; older void entries are forgotten. */
const MEMORY_GONE_KEPT = 200;
const LIST_OPTIONS = { maxBuffer: 64 * 1024 * 1024, timeout: 60_000 };

/** A ref entries are read from, as the person knows it. */
export const ledgerRefSchema = z.object({ ref: z.string(), label: z.string(), sha: z.string() });
export type LedgerRef = z.infer<typeof ledgerRefSchema>;

/** Where an entry was read: the base branch, this branch (its tip, or null with no commit yet), or a live ref. */
export const ledgerSourceSchema = z.object({ ref: z.string(), label: z.string(), sha: z.string().nullable() });
export type LedgerSource = z.infer<typeof ledgerSourceSchema>;

/** base: in the base tree; branch: this branch only; live: another live ref only; gone: only in the memory. */
export const LEDGER_PLACES = ["base", "branch", "live", "gone"] as const;
export type LedgerPlace = (typeof LEDGER_PLACES)[number];

/**
 * What a checkout saw of the ledger, kept between reads:
 * - `entries`: each entry it found, with where (what makes a vanished one `void`);
 * - `pulls`: the pull request heads it last read;
 * - `trees`: the entry ids each `.rerail/decisions` tree object holds, with what could not be read there (trees never
 *   change, so a ref whose folder is unchanged costs no read);
 * - `held`: per merged entry, whether the base held its winner's position where it merged (decided once);
 * - `moved`: per entry and winner tip (`<id>:<sha>`), whether the winner's position moved there;
 * - `view`: the statuses of the last read, which status reads show without touching Git.
 */
export const ledgerMemorySchema = z.object({
  entries: z.record(z.string(), z.object({ entry: decisionEntrySchema, source: ledgerSourceSchema })),
  pulls: z.array(ledgerRefSchema),
  trees: z.record(z.string(), z.object({ ids: z.array(z.string()), problems: z.array(z.string()) })),
  held: z.record(z.string(), z.boolean()),
  moved: z.record(z.string(), z.boolean()),
  view: z.object({
    items: z.array(z.object({ id: z.string(), status: z.enum(LEDGER_STATUSES), place: z.enum(LEDGER_PLACES), source: ledgerSourceSchema })),
    problems: z.array(z.string()),
  }),
});
export type LedgerMemory = z.infer<typeof ledgerMemorySchema>;

export const EMPTY_LEDGER_MEMORY: LedgerMemory = { entries: {}, pulls: [], trees: {}, held: {}, moved: {}, view: { items: [], problems: [] } };

export interface LedgerItem {
  readonly entry: DecisionEntry;
  readonly status: LedgerStatus;
  readonly place: LedgerPlace;
  readonly source: LedgerSource;
}

export interface LedgerView {
  /** Every entry, oldest first. */
  readonly items: LedgerItem[];
  /** Entries that could not be read, `<ref>: <file>: <problem>`. */
  readonly problems: string[];
  /** The memory after this read: what the next read compares with. */
  readonly memory: LedgerMemory;
}

export interface ReadLedgerOptions {
  /** What the checkout saw before; nothing by default. */
  readonly memory?: LedgerMemory;
  /** Open pull request heads, as the detector lists them; the memory's last ones by default. */
  readonly pulls?: readonly LedgerRef[];
}

/** The view the last read stored in `memory`, rebuilt without touching Git. */
export function storedLedger(memory: LedgerMemory): Pick<LedgerView, "items" | "problems"> {
  const items = memory.view.items.flatMap(item => {
    const known = memory.entries[item.id];
    return known === undefined ? [] : [{ entry: known.entry, status: item.status, place: item.place, source: item.source }];
  });
  return { items, problems: [...memory.view.problems] };
}

interface Found {
  readonly entry: DecisionEntry;
  readonly place: Exclude<LedgerPlace, "gone">;
  readonly source: LedgerSource;
}

/** A short ref name: `feature` for `refs/heads/feature`, `origin/feature` for `refs/remotes/origin/feature`. */
const shortRef = (ref: string): string => ref.replace(/^refs\/(?:heads|remotes)\//u, "");

/**
 * The live refs entries are read from: local and remote-tracking branches the base does not hold, listed by one
 * `for-each-ref --no-merged`, but this branch and the base; and the pull request heads this repository holds that
 * the base does not. One per tip.
 */
function liveRefs(root: string, base: { ref: string; sha: string } | null, branch: string | null, pulls: readonly LedgerRef[]): LedgerRef[] {
  const skip = new Set([base?.ref, branch === null ? undefined : `refs/heads/${branch}`]);
  const taken = new Set<string>();
  const refs: LedgerRef[] = [];
  const add = (ref: LedgerRef): void => {
    if (taken.has(ref.sha)) return;
    taken.add(ref.sha);
    refs.push(ref);
  };
  for (const pull of pulls) if (commitExists(root, pull.sha) && (base === null || !isAncestor(root, pull.sha, base.sha))) add(pull);
  const merged = base === null ? [] : [`--no-merged=${base.sha}`];
  const listed = runGit(root, ["for-each-ref", ...merged, "--format=%(refname)%00%(objectname)%00%(symref)", "refs/heads", "refs/remotes"], LIST_OPTIONS);
  for (const line of listed.status === 0 ? listed.stdout.split("\n") : []) {
    const [name, sha, symref] = line.split("\0");
    if (name === undefined || name === "" || sha === undefined || sha === "" || symref !== "" || skip.has(name)) continue;
    add({ ref: shortRef(name), label: shortRef(name), sha });
  }
  return refs;
}

/** Each commit's `.rerail/decisions` tree object, by one `cat-file --batch-check`; commits without the folder are left out. */
function decisionFolders(root: string, commits: readonly string[]): Map<string, string> {
  const folders = new Map<string, string>();
  if (commits.length === 0) return folders;
  const result = runGit(root, ["cat-file", "--batch-check=%(objectname) %(objecttype)"], { ...LIST_OPTIONS, input: commits.map(commit => `${commit}:${DECISIONS_DIRECTORY}\n`).join("") });
  if (result.status !== 0) return folders;
  result.stdout.split("\n").filter(line => line !== "").forEach((line, index) => {
    const [object, type] = line.split(" ");
    if (type === "tree" && object !== undefined && commits[index] !== undefined) folders.set(commits[index], object);
  });
  return folders;
}

/** Whether two refs name the same branch: equal, or one is the other's remote-tracking name (`origin/x` and `x`). */
export function sameRef(left: string, right: string): boolean {
  const a = shortRef(left);
  const b = shortRef(right);
  return a === b || a.endsWith(`/${b}`) || b.endsWith(`/${a}`);
}

/** Whether the tree at `sha` holds `path`. */
function inTree(root: string, sha: string, path: string): boolean {
  return runGit(root, ["cat-file", "-e", `${sha}:${path}`]).status === 0;
}

/**
 * Whether the merge held the winner's position: evaluated at the first base commit (along its first parents) that holds
 * the entry file, so later base work never flips it. Side a carries the entry, so its position is expected there; side
 * b only if its commit was in the base by then. A new decision's position is its rule file. A position that cannot
 * be read counts as held: only a contradiction Rerail can see flags an entry.
 */
async function winnerHeld(root: string, entry: DecisionEntry, baseSha: string): Promise<boolean> {
  const added = (gitText(root, ["log", "--first-parent", "--diff-filter=A", "--format=%H", baseSha, "--", `${DECISIONS_DIRECTORY}/${decisionFileName(entry.id)}`]) ?? "").split("\n").filter(line => line !== "").at(-1);
  const merged = added ?? baseSha;
  const winner = winnerOf(entry);
  if (entry.result.rule !== null && inTree(root, merged, entry.result.rule) && (winner === null || entry.sides[winner].at.kind === "candidate")) return true;
  if (winner === null) return entry.result.rule === null;
  const side = entry.sides[winner];
  if (winner === "b" && !isAncestor(root, side.sha, merged)) return true;
  const now = await positionFingerprint(root, entry.base, merged, side.at);
  return now === null || now === side.fingerprint;
}

/** The commit a side's ref points at now (this branch's: HEAD, never the working files); undefined when unknown. */
function sideTip(root: string, ref: string, branch: string | null, head: string | null, live: readonly LedgerRef[]): string | undefined {
  if (branch !== null && sameRef(ref, branch)) return head ?? undefined;
  const exact = ref.startsWith("-") ? undefined : gitText(root, ["rev-parse", "--verify", "--quiet", "--end-of-options", `${ref}^{commit}`]);
  if (exact !== undefined) return exact;
  return live.find(candidate => candidate.ref === ref || sameRef(candidate.ref, ref))?.sha;
}

interface StatusContext {
  readonly baseSha: string | null;
  readonly baseSuperseded: ReadonlySet<string>;
  readonly replaced: ReadonlyMap<string, string>;
  readonly branch: string | null;
  readonly head: string | null;
  readonly live: readonly LedgerRef[];
  readonly memory: LedgerMemory;
  /** What this read decided, kept for the next. */
  readonly held: Record<string, boolean>;
  readonly moved: Record<string, boolean>;
}

/**
 * Whether the winner's position moved since the entry was recorded: its fingerprint at the winner's committed tip
 * differs. Reopening tells the side that followed, so never for the checkout whose branch holds the winning position.
 */
async function winnerMoved(root: string, found: Found, context: StatusContext): Promise<boolean> {
  const winner = winnerOf(found.entry);
  if (winner === null) return false;
  const side = found.entry.sides[winner];
  if ((winner === "a" && found.place === "branch") || (context.branch !== null && sameRef(side.ref, context.branch))) return false;
  // Side a is the branch that carries the entry: where it was found.
  const tip = winner === "a" ? found.source.sha ?? undefined : sideTip(root, side.ref, context.branch, context.head, context.live);
  if (tip === undefined) return false;
  const key = `${found.entry.id}:${tip}`;
  const known = context.memory.moved[key];
  if (known !== undefined) return (context.moved[key] = known);
  const now = await positionFingerprint(root, found.entry.base, tip, side.at);
  return (context.moved[key] = now !== null && now !== side.fingerprint);
}

/** A human verdict before a precedent copy of one, then the newer, then the larger id: which entry on a question is latest. */
function later(left: DecisionEntry, right: DecisionEntry): boolean {
  const human = (entry: DecisionEntry): number => (entry.decidedBy.kind === "precedent" ? 0 : 1);
  return human(left) - human(right) > 0 || (human(left) === human(right) && (left.recordedAt > right.recordedAt || (left.recordedAt === right.recordedAt && left.id > right.id)));
}

/**
 * The base entries each later base entry on its question replaces: named in `supersedes`, or not the latest on the
 * question. A precedent copy never replaces the verdict it followed.
 */
function supersededInBase(entries: readonly DecisionEntry[]): Set<string> {
  const superseded = new Set(entries.flatMap(entry => (entry.supersedes === null ? [] : [entry.supersedes])));
  const latest = new Map<string, DecisionEntry>();
  for (const entry of entries) {
    const known = latest.get(entry.question);
    if (known === undefined || later(entry, known)) latest.set(entry.question, entry);
  }
  for (const entry of entries) if (latest.get(entry.question) !== entry) superseded.add(entry.id);
  return superseded;
}

async function statusOf(root: string, found: Found, context: StatusContext): Promise<LedgerStatus> {
  const { entry } = found;
  if (found.place === "base") {
    if (context.baseSuperseded.has(entry.id)) return "superseded";
    const held = context.memory.held[entry.id] ?? await winnerHeld(root, entry, context.baseSha!);
    context.held[entry.id] = held;
    return held ? "in-force" : "flagged";
  }
  const by = context.replaced.get(entry.id);
  if (by === "base" || by === found.source.ref) return "superseded";
  return (await winnerMoved(root, found, context)) ? "reopened" : "provisional";
}

/** Read the ledger view of the checkout at `root`. See the module comment for the statuses. */
export async function readLedger(root: string, options: ReadLedgerOptions = {}): Promise<LedgerView> {
  const memory = options.memory ?? EMPTY_LEDGER_MEMORY;
  const pulls = options.pulls ?? memory.pulls;
  const branchName = currentBranch(root);
  const head = gitText(root, ["rev-parse", "--verify", "--quiet", "HEAD^{commit}"]) ?? null;
  const baseRef = baseBranch(root);
  const baseSha = baseRef === null ? null : gitText(root, ["rev-parse", "--verify", "--quiet", "--end-of-options", `${baseRef.ref}^{commit}`]) ?? null;
  const base = baseRef === null || baseSha === null ? null : { ref: baseRef.ref, name: baseRef.name, sha: baseSha };
  const live = liveRefs(root, base, branchName, pulls);
  const problems: string[] = [];
  const found = new Map<string, Found>();
  const trees: LedgerMemory["trees"] = {};
  const take = (entries: readonly DecisionEntry[], place: Found["place"], source: LedgerSource): void => {
    for (const entry of entries) if (!found.has(entry.id)) found.set(entry.id, { entry, place, source });
  };
  const here: LedgerSource = { ref: branchName ?? head ?? "HEAD", label: "This branch", sha: head };
  const sources: Array<{ sha: string; place: Found["place"]; source: LedgerSource }> = [
    ...(base === null ? [] : [{ sha: base.sha, place: "base" as const, source: { ref: base.ref, label: base.name, sha: base.sha } }]),
    ...(head === null ? [] : [{ sha: head, place: "branch" as const, source: here }]),
    ...live.map(ref => ({ sha: ref.sha, place: "live" as const, source: { ref: ref.ref, label: ref.label, sha: ref.sha } })),
  ];
  const folders = decisionFolders(root, sources.map(source => source.sha));
  const worktree = await readWorktreeDecisions(root);
  for (const { sha, place, source } of sources) {
    // The working files come right after the base: they are this branch as it stands.
    if (place === "branch") take(worktree.entries, "branch", here);
    const folder = folders.get(sha);
    if (folder === undefined) continue;
    let tree = trees[folder] ?? memory.trees[folder];
    if (tree === undefined || tree.ids.some(id => memory.entries[id] === undefined && !found.has(id))) {
      const set = await readDecisionFolder(root, folder);
      take(set.entries, place, source);
      tree = { ids: set.entries.map(entry => entry.id), problems: set.problems };
    } else {
      take(tree.ids.map(id => found.get(id)?.entry ?? memory.entries[id]!.entry), place, source);
    }
    trees[folder] = tree;
    problems.push(...tree.problems.map(problem => `${source.label}: ${problem}`));
  }
  if (head === null) take(worktree.entries, "branch", here);
  problems.push(...worktree.problems.map(problem => `${here.label}: ${problem}`));

  const baseEntries = [...found.values()].filter(item => item.place === "base").map(item => item.entry);
  // Who replaces an entry outside the base: a base entry ("base"), or another entry from the same place (its ref).
  const replaced = new Map<string, string>();
  for (const item of found.values()) {
    if (item.entry.supersedes !== null) replaced.set(item.entry.supersedes, item.place === "base" ? "base" : item.source.ref);
  }
  const context: StatusContext = { baseSha: base?.sha ?? null, baseSuperseded: supersededInBase(baseEntries), replaced, branch: branchName, head, live, memory, held: {}, moved: {} };
  const items: LedgerItem[] = [];
  for (const item of found.values()) items.push({ ...item, status: await statusOf(root, item, context) });
  for (const [id, remembered] of Object.entries(memory.entries)) {
    if (found.has(id)) continue;
    items.push({ entry: remembered.entry, place: "gone", source: remembered.source, status: replaced.has(id) ? "superseded" : "void" });
  }
  items.sort((left, right) => left.entry.recordedAt.localeCompare(right.entry.recordedAt) || left.entry.id.localeCompare(right.entry.id));
  const kept = [...items.filter(item => item.place !== "gone"), ...items.filter(item => item.place === "gone").slice(-MEMORY_GONE_KEPT)];
  return {
    items,
    problems,
    memory: {
      entries: Object.fromEntries(kept.map(item => [item.entry.id, { entry: item.entry, source: item.source }])),
      pulls: live.filter(ref => pulls.some(pull => pull.sha === ref.sha)),
      trees,
      held: context.held,
      moved: context.moved,
      view: { items: kept.map(item => ({ id: item.entry.id, status: item.status, place: item.place, source: item.source })), problems },
    },
  };
}

/** Statuses an entry can be precedent in: in force, or provisional with the winner's position unchanged. */
const PRECEDENT_STATUSES: Readonly<Record<LedgerStatus, boolean>> = {
  "in-force": true, provisional: true, reopened: false, flagged: false, superseded: false, void: false,
};

/** Whether the ledger holds a verdict on `question` that counts: one in force or provisional. */
export function hasVerdict(ledger: Pick<LedgerView, "items">, question: string): boolean {
  return ledger.items.some(item => item.entry.question === question && PRECEDENT_STATUSES[item.status]);
}

/** A conflict precedent is asked about: its question, and each side's ref and position fingerprint (null: unknown). */
export interface PrecedentTarget {
  readonly question: string;
  readonly sides: Readonly<Record<SideId, { readonly ref: string; readonly fingerprint: string | null }>>;
}

export interface PrecedentResult extends StepResult {
  /** The entry that decided; or, when none applies, the verdict on the question that would come first, as context. */
  readonly entry: { readonly id: string } | null;
}

/**
 * The side of `target` an earlier verdict picks, only when it is the same dispute: one side holds the winner's position
 * and the other the loser's (by fingerprint; by ref where the target does not know a side's position).
 */
function sidePicked(target: PrecedentTarget, entry: DecisionEntry, winner: SideId): SideId | undefined {
  const holds = (side: SideId, position: DecisionEntry["sides"]["a"]): boolean => {
    const { fingerprint, ref } = target.sides[side];
    return fingerprint === null ? sameRef(ref, position.ref) : fingerprint === position.fingerprint;
  };
  return SIDES.find(side => holds(side, entry.sides[winner]) && holds(otherSide(side), entry.sides[otherSide(winner)]));
}

/**
 * Precedent from the view: a verdict on the same question decides only the same dispute (`sidePicked`). A verdict in
 * force comes first, then a provisional one by a person (its winner's position unchanged), then the latest. Never one
 * that is void, superseded, reopened or flagged, and never a new decision, which picks no side. When none applies, the
 * first of them is returned as context for the person ("Earlier: … chose … on …"), deciding nothing.
 */
export function decidePrecedent(target: PrecedentTarget, ledger: Pick<LedgerView, "items">): PrecedentResult {
  const verdicts = ledger.items.filter(item => item.entry.question === target.question && PRECEDENT_STATUSES[item.status] && winnerOf(item.entry) !== null);
  if (verdicts.length === 0) return { step: "precedent", winner: null, reason: "No verdict on this question is in force or provisional.", entry: null };
  const rank = (item: LedgerItem): number => (item.status === "in-force" ? 2 : item.entry.decidedBy.kind === "person" ? 1 : 0);
  const ordered = [...verdicts].sort((left, right) => rank(right) - rank(left) || right.entry.recordedAt.localeCompare(left.entry.recordedAt));
  for (const item of ordered) {
    const winner = winnerOf(item.entry)!;
    const side = sidePicked(target, item.entry, winner);
    if (side === undefined) continue;
    const { entry } = item;
    return {
      step: "precedent",
      winner: side,
      reason: `Decision ${entry.id} (${item.status}, by ${deciderName(entry)}) chose ${entry.sides[winner].label} over ${entry.sides[otherSide(winner)].label} in this same dispute.`,
      entry: { id: entry.id },
    };
  }
  const first = ordered[0]!;
  const chosen = first.entry.sides[winnerOf(first.entry)!].label;
  return { step: "precedent", winner: null, reason: `Earlier: ${deciderName(first.entry)} chose ${chosen} on ${first.entry.recordedAt.slice(0, 10)}.`, entry: { id: first.entry.id } };
}

/** Two provisional entries on one question, this branch's and a live ref's, whose winners differ. */
export interface VerdictDisagreement {
  readonly mine: LedgerItem;
  readonly theirs: LedgerItem;
}

/**
 * Each live ref's latest provisional entry on a question where this branch's latest provisional entry picked another
 * winner; a human verdict counts before a precedent copy of one.
 */
export function verdictDisagreements(ledger: Pick<LedgerView, "items">): VerdictDisagreement[] {
  const newest = (items: readonly LedgerItem[], key: (item: LedgerItem) => string): Map<string, LedgerItem> => {
    const map = new Map<string, LedgerItem>();
    for (const item of items) {
      const known = map.get(key(item));
      if (known === undefined || later(item.entry, known.entry)) map.set(key(item), item);
    }
    return map;
  };
  const provisional = ledger.items.filter(item => item.status === "provisional");
  const mine = newest(provisional.filter(item => item.place === "branch"), item => item.entry.question);
  const theirs = newest(provisional.filter(item => item.place === "live"), item => `${item.entry.question}\0${item.source.ref}`);
  return [...theirs.values()].flatMap(other => {
    const own = mine.get(other.entry.question);
    return own === undefined || winnerKey(own.entry) === winnerKey(other.entry) ? [] : [{ mine: own, theirs: other }];
  });
}

/** What an entry decided, from this branch (`mine`, its ref): Mine, Theirs (who), or New decision. */
function outcomeOf(item: LedgerItem, mine: string | null): string {
  const winner = winnerOf(item.entry);
  if (winner === null) return "New decision";
  const side = item.entry.sides[winner];
  // Side a is the branch carrying the entry: this branch when the entry is here.
  if ((winner === "a" && item.place === "branch") || (mine !== null && sameRef(side.ref, mine))) return "Mine";
  return `Theirs (${winner === "a" && item.place === "live" ? item.source.label : side.label})`;
}

/** The ledger as the status lists it: grouped by question, the newest question first, at most {@link LEDGER_ITEMS_SHOWN} entries. */
export function ledgerLog(ledger: Pick<LedgerView, "items">, mine: string | null): DecisionLogQuestion[] {
  const shown = ledger.items.slice(-LEDGER_ITEMS_SHOWN).reverse();
  const groups = new Map<string, DecisionLogItem[]>();
  for (const item of shown) {
    const { entry } = item;
    const logged: DecisionLogItem = {
      id: entry.id,
      question: entry.question,
      title: entry.title,
      status: item.status,
      verdict: entry.verdict,
      outcome: outcomeOf(item, mine),
      by: { kind: entry.decidedBy.kind, name: deciderName(entry) },
      source: { ref: item.source.ref, label: item.source.label },
      recordedAt: entry.recordedAt,
      rule: entry.result.rule,
      text: entry.result.text,
      follows: entry.result.follows,
    };
    groups.set(entry.question, [...(groups.get(entry.question) ?? []), logged]);
  }
  return [...groups].map(([question, items]) => ({ question, title: items[0]!.title, items }));
}

/**
 * The latest text of a decision another one follows by reference: the followed candidate at its ref's tip, else in the
 * base branch, carried through the active amendments there that supersede it. Null when neither holds it.
 */
export async function followedDecision(root: string, follows: Follows): Promise<string | null> {
  const remotes = remoteBranches(root).map(remote => ({ ref: shortRef(remote.ref), label: shortRef(remote.ref), sha: remote.commit }));
  const base = baseBranch(root);
  const tips = [sideTip(root, follows.ref, null, null, remotes), base === null ? undefined : base.ref];
  for (const tip of tips) {
    if (tip === undefined || tip === null) continue;
    const { candidates } = await readTreeCandidates(root, tip);
    const active = candidates.filter(candidate => candidate.state === "active");
    let current = active.find(candidate => candidate.id === follows.candidateId);
    const seen = new Set<string>([follows.candidateId]);
    for (let next = active.find(candidate => candidate.supersedes === follows.candidateId); next !== undefined && !seen.has(next.id); next = active.find(candidate => candidate.supersedes === current!.id)) {
      seen.add(next.id);
      current = next;
    }
    if (current !== undefined) return current.decision;
  }
  return null;
}
