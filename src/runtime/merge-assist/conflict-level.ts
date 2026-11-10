/**
 * The dimmer's pass over a checkout's open conflicts at every detection (CONTRACT: Dimmer): measure each one's signals
 * from Git, decide its level with `dimConflict` (never below the level it showed, within the interruption budgets),
 * and end a snooze its level, a pull request or a merge has outrun. The budgets are the person's, across checkouts:
 * at most `checkpointsPerDay` checkpoint prompts a day and one stop per session, kept under `budget:<person>`.
 */
import { z } from "zod";
import { gitText } from "../../engine/git.js";
import { measureSignals, type ContestedChange } from "../../engine/merge-assist/conflict-signals.js";
import { dimConflict, expectedRework, STOPS_PER_SESSION } from "../../engine/merge-assist/dimmer.js";
import { CONFLICT_LEVELS, type Conflict, type ConflictLevel, type ConflictSignals, type ConflictSnooze } from "../../engine/merge-assist/model.js";
import { hunkLinesAt, ownBase } from "../../engine/merge-assist/overlap.js";
import { defaultPolicy, loadPolicy, type DimmerConfig } from "../../engine/merge-assist/policy.js";
import type { StoredConflict } from "./conflict-store.js";
import type { PullRequestSummary } from "./github.js";
import { STATE_KEYS, isoSeconds, readState, writeState, type StateQueries } from "./state.js";

/** How long a granted interruption is kept: longer than any day or session it counts against. */
const GRANT_KEPT_MS = 7 * 24 * 3_600_000;

const grantSchema = z.object({
  conflictId: z.string(),
  level: z.enum(["checkpoint", "stop"]),
  sessionId: z.string().nullable(),
  /** The UTC day it was granted on: `2026-11-09`. */
  day: z.string(),
  at: z.number(),
});
type Grant = z.infer<typeof grantSchema>;
const budgetSchema = z.array(grantSchema);

/** Where this checkout stands, and who and what the pass answers to. */
export interface LevelContext {
  readonly db: StateQueries;
  readonly root: string;
  /** The base branch's ref, where this branch's work starts (the merge base), and its head. */
  readonly base: { readonly ref: string };
  readonly start: string;
  readonly head: string;
  /** The person the checkout's prompts go to (Git's `user.name`, else the account's). */
  readonly person: string;
  /** The session whose boundary ran the detection; a detection with none (the dashboard, the CLI) grants no stop. */
  readonly sessionId: string | null;
  readonly pulls: readonly PullRequestSummary[];
  /** Whether this checkout holds a resolution of a conflict on the question. */
  readonly verdict: (question: string) => boolean;
  /** Each contested file's live refs that change it. */
  readonly touches: ReadonlyMap<string, ReadonlySet<string>>;
  readonly now: number;
}

const rank = (level: ConflictLevel): number => CONFLICT_LEVELS.indexOf(level);

/** The dimmer's settings at the base: its `.rerail/policy.yaml`, else the defaults (an invalid file too). */
export function dimmerConfig(root: string, baseRef: string): DimmerConfig {
  const loaded = loadPolicy(root, baseRef);
  return loaded.source === "invalid" ? defaultPolicy().dimmer : loaded.policy.dimmer;
}

/** Whether a pull request is open on theirs: it is one, or its branch is one's head. */
function pullRequestOn(conflict: Conflict, pulls: readonly PullRequestSummary[]): boolean {
  const { ref } = conflict.theirs;
  if (ref.startsWith("refs/pull/")) return true;
  return pulls.some(pull => pull.headRefName === ref || ref.endsWith(`/${pull.headRefName}`));
}

/** Both sides' full changed lines at an overlap's contested lines; null when Git cannot say. */
function changedLines(context: LevelContext, { conflict, files, lines }: StoredConflict): ContestedChange["changed"] {
  const [path] = files;
  if (conflict.kind !== "overlap" || path === undefined || lines.length === 0) return null;
  try {
    return {
      mine: hunkLinesAt(context.root, context.start, null, path, lines).join("\n"),
      theirs: hunkLinesAt(context.root, context.start, conflict.theirs.sha, path, lines).join("\n"),
    };
  } catch {
    return null;
  }
}

/** One open conflict as the dimmer measures it. */
function contested(context: LevelContext, entry: StoredConflict): ContestedChange {
  const { conflict, files } = entry;
  const others = new Set(files.flatMap(file => [...(context.touches.get(file) ?? [])]));
  others.delete(conflict.theirs.ref);
  return {
    kind: conflict.kind,
    files,
    hunks: { mine: conflict.mine.hunk, theirs: conflict.theirs.hunk },
    changed: changedLines(context, entry),
    theirs: { sha: conflict.theirs.sha, own: ownBase(context.root, conflict.theirs.sha, context.base.ref, context.start) },
    pullRequest: pullRequestOn(conflict, context.pulls),
    otherRefs: others.size,
    verdict: context.verdict(conflict.question),
  };
}

/** The snooze after this pass: ended once the level rose past it, a pull request opened on theirs, or the base moved. */
function snoozeAfter(snooze: ConflictSnooze | null, level: ConflictLevel, signals: ConflictSignals, baseTip: string, now: number): ConflictSnooze | null {
  if (snooze === null || snooze.endedAt !== null) return snooze;
  const outrun = rank(level) > rank(snooze.level) || (signals.pullRequest && !snooze.pullRequest) || baseTip !== snooze.base;
  return outrun ? { ...snooze, endedAt: isoSeconds(now) } : snooze;
}

/**
 * Interruptions the person has left for `conflict`: stops in this session; checkpoint prompts today (UTC), a stop
 * counting as one, since it shows the person the same card.
 */
function budgetFor(context: LevelContext, grants: readonly Grant[], conflictId: string, config: DimmerConfig): { stops: number; checkpoints: number } {
  const today = new Date(context.now).toISOString().slice(0, 10);
  const others = grants.filter(grant => grant.conflictId !== conflictId);
  const stops = new Set(others.filter(grant => grant.level === "stop" && grant.sessionId === context.sessionId).map(grant => grant.conflictId)).size;
  const prompts = new Set(others.filter(grant => grant.day === today).map(grant => grant.conflictId)).size;
  return { stops: context.sessionId === null ? 0 : STOPS_PER_SESSION - stops, checkpoints: config.checkpointsPerDay - prompts };
}

export interface LevelledConflicts {
  readonly entries: StoredConflict[];
  /** The person's grants after this pass, for {@link saveBudget} in the transaction that stores the conflicts. */
  readonly grants: Grant[];
}

/**
 * Every open conflict in `entries` measured and levelled, loudest first for the budgets; settled ones unchanged. A
 * conflict the dimmer has not seen before rises to its level at once; one it has, only by clearing the next threshold
 * by 20%, and never drops.
 */
export function levelConflicts(context: LevelContext, entries: readonly StoredConflict[]): LevelledConflicts {
  const open = entries.filter(entry => entry.conflict.status === "open");
  const grants = (readState(context.db, STATE_KEYS.budget(context.person), budgetSchema) ?? []).filter(grant => context.now - grant.at < GRANT_KEPT_MS);
  if (open.length === 0) return { entries: [...entries], grants };
  const config = dimmerConfig(context.root, context.base.ref);
  const baseTip = gitText(context.root, ["rev-parse", "--verify", "--quiet", `${context.base.ref}^{commit}`]) ?? "";
  // Conflicts waiting for a budget have the first claim on it when it frees; then the loudest.
  const waiting = (conflict: Conflict): number => (conflict.queuedFor === null ? -1 : rank(conflict.queuedFor));
  const measured = open.map(entry => ({ entry, signals: measureSignals({ ...context, baseRef: context.base.ref }, contested(context, entry)) }))
    .sort((left, right) => waiting(right.entry.conflict) - waiting(left.entry.conflict) || expectedRework(right.signals) - expectedRework(left.signals));
  const levelled = new Map<string, Conflict>();
  for (const { entry: { conflict }, signals } of measured) {
    // A conflict the dimmer has not measured is new, unless it came back open at a level (reopened, at ambient).
    const previous = conflict.signals === null && conflict.level === "logged" ? null : { level: conflict.level, queuedFor: conflict.queuedFor };
    const result = dimConflict({ signals, previous, budget: budgetFor(context, grants, conflict.id, config), config });
    if ((result.level === "checkpoint" || result.level === "stop")
        && !grants.some(grant => grant.conflictId === conflict.id && grant.level === result.level)) {
      grants.push({ conflictId: conflict.id, level: result.level, sessionId: context.sessionId, day: new Date(context.now).toISOString().slice(0, 10), at: context.now });
    }
    levelled.set(conflict.id, {
      ...conflict, ...result, queued: result.queuedFor !== null, signals, snooze: snoozeAfter(conflict.snooze, result.level, signals, baseTip, context.now),
    });
  }
  return { entries: entries.map(entry => ({ ...entry, conflict: levelled.get(entry.conflict.id) ?? entry.conflict })), grants };
}

/**
 * Store the person's grants, with any another detection stored meanwhile (in another checkout or process): one grant
 * per conflict and level, the first kept.
 */
export function saveBudget(db: StateQueries, person: string, grants: readonly Grant[], now: number): void {
  const key = STATE_KEYS.budget(person);
  const all = [...(readState(db, key, budgetSchema) ?? []), ...grants].filter(grant => now - grant.at < GRANT_KEPT_MS);
  const kept = all.filter((grant, index) => all.findIndex(other => other.conflictId === grant.conflictId && other.level === grant.level) === index);
  writeState(db, key, kept, now);
}

/** `conflict` snoozed by the person ("later"): until its level rises, theirs opens a pull request, or the base moves. */
export function snoozed(conflict: Conflict, root: string, baseRef: string | null, now: number): Conflict {
  const base = baseRef === null ? "" : gitText(root, ["rev-parse", "--verify", "--quiet", `${baseRef}^{commit}`]) ?? "";
  return { ...conflict, snooze: { at: isoSeconds(now), level: conflict.level, pullRequest: conflict.signals?.pullRequest ?? false, base, endedAt: null } };
}

/** Whether `conflict` is snoozed now. */
export function isSnoozed(conflict: Conflict): boolean {
  return conflict.snooze !== null && conflict.snooze.endedAt === null;
}
