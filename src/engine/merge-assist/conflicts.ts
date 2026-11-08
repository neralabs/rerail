/**
 * In-flight conflicts as Merge Assist reports them (CONTRACT: lane 2): their ids, how loud each is (the dimmer), the
 * order the status lists them in, the record the decision log keeps for one, and the log as the person reads it.
 * Pure: the runtime finds the conflicts and stores them.
 */
import { createHash } from "node:crypto";
import type { LoggedDecision } from "./decision-log.js";
import type { Conflict, ConflictKind, ConflictLevel, ConflictSide, DecisionLogItem } from "./model.js";
import { type ConflictRecord, sanitizeIntent, type SideId } from "./conflict-record.js";

/** How many resolved, and how many dismissed, conflicts the status lists; every open one is listed. */
export const SETTLED_CONFLICTS_SHOWN = 20;
/** How many decision log entries the status lists, newest first. */
export const LOG_ITEMS_SHOWN = 50;

/** Files whose overlaps only the Conflicts page shows: lockfiles, changelogs, generated output and docs. */
const QUIET_FILES: readonly RegExp[] = [
  /(?:^|\/)(?:package-lock\.json|npm-shrinkwrap\.json|pnpm-lock\.yaml|go\.sum|bun\.lockb|[^/]+\.lock)$/iu,
  /(?:^|\/)(?:CHANGELOG|CHANGES|HISTORY|NEWS)(?:\.[a-z]+)?$/iu,
  /(?:^|\/)(?:dist|build|generated|__generated__)\/|\.(?:generated|min)\.[a-z]+$|\.pb\.go$|_pb2\.py$/iu,
  /^docs\//u,
];

/** A conflict's stable id: its question and a short hash of their ref, so the same overlap keeps its id. */
export function conflictId(question: string, theirsRef: string): string {
  return `conflict:${question}:${createHash("sha256").update(theirsRef).digest("hex").slice(0, 8)}`;
}

/**
 * The dimmer, v0: an overlap is `logged` (the Conflicts page only) when every file in it is a lockfile, a changelog,
 * generated output or under `docs/`; anything else, and every contradiction between decisions, is `ambient`.
 */
export function conflictLevel(kind: ConflictKind, files: readonly string[]): ConflictLevel {
  if (kind === "decisions" || files.length === 0) return "ambient";
  return files.every(file => QUIET_FILES.some(pattern => pattern.test(file))) ? "logged" : "ambient";
}

/** Open conflicts first (newest first), then the latest resolved, then the latest dismissed. */
export function orderConflicts(conflicts: readonly Conflict[]): Conflict[] {
  const latest = (status: Conflict["status"]): Conflict[] => conflicts
    .filter(conflict => conflict.status === status)
    .sort((left, right) => (right.resolution?.at ?? right.detectedAt).localeCompare(left.resolution?.at ?? left.detectedAt));
  return [...latest("open"), ...latest("resolved").slice(0, SETTLED_CONFLICTS_SHOWN), ...latest("dismissed").slice(0, SETTLED_CONFLICTS_SHOWN)];
}

/** How a person knows a ref: `PR #12`, a branch name, or a short commit. */
export function refLabel(ref: string): string {
  const pull = /^refs\/pull\/(\d+)\/head$/u.exec(ref);
  if (pull !== null) return `PR #${pull[1]}`;
  if (/^[0-9a-f]{40,64}$/u.test(ref)) return ref.slice(0, 12);
  return ref.replace(/^refs\/(?:heads|remotes)\//u, "");
}

/** The decision log's record of a conflict: both sides' refs, commits, sanitized intents and files, on `question`. */
export function conflictRecord(
  input: { repository: string; base: string; question: string },
  sides: Readonly<Record<SideId, { side: ConflictSide; files: readonly string[] }>>,
): ConflictRecord {
  const recordSide = ({ side, files }: { side: ConflictSide; files: readonly string[] }) => ({
    ref: side.ref,
    sha: side.sha,
    intent: sanitizeIntent(side.summary),
    anchors: { files: [...files], symbols: [] },
  });
  return { repository: input.repository, base: input.base, question: input.question, sides: { a: recordSide(sides.a), b: recordSide(sides.b) } };
}

/** What a log entry decided, from this branch's side (`mine`, its ref): Mine, Theirs (who), New decision or Escalated. */
function outcomeOf({ entry }: LoggedDecision, mine: string | null): string {
  if (entry.verdict === "escalated") return "Escalated";
  if (entry.result?.kind === "amendment") return "New decision";
  const winner = entry.conflict.sides[entry.verdict].ref;
  if (winner === mine) return "Mine";
  const sides = [entry.conflict.sides.a.ref, entry.conflict.sides.b.ref];
  return sides.includes(mine ?? "") ? `Theirs (${refLabel(winner)})` : `For ${refLabel(winner)}`;
}

/** A log entry's title: the conflict's own title when this checkout knows it, else the files it was about. */
function titleOf({ entry }: LoggedDecision, titles: ReadonlyMap<string, string>): string {
  const known = titles.get(entry.conflict.question);
  if (known !== undefined) return known;
  const files = [...new Set([...entry.conflict.sides.a.anchors.files, ...entry.conflict.sides.b.anchors.files])];
  return files.length === 0 ? entry.conflict.question : `${files[0]}${files.length > 1 ? ` and ${files.length - 1} more` : ""}`;
}

/** The decision log as the status lists it: newest first, at most {@link LOG_ITEMS_SHOWN}. */
export function decisionLogItems(log: readonly LoggedDecision[], mine: string | null, titles: ReadonlyMap<string, string>): DecisionLogItem[] {
  return log.slice(-LOG_ITEMS_SHOWN).reverse().map(logged => {
    const { entry } = logged;
    const by = entry.decidedBy.kind === "person" ? entry.decidedBy.name : entry.decidedBy.step;
    return {
      id: entry.id,
      recordedAt: entry.recordedAt,
      title: titleOf(logged, titles),
      outcome: outcomeOf(logged, mine),
      by: { kind: entry.decidedBy.kind, name: by },
      rule: entry.result?.rule ?? null,
      text: entry.result?.text ?? null,
    };
  });
}
