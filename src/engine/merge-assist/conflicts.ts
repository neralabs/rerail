/**
 * In-flight conflicts as Merge Assist reports them (CONTRACT: lane 2): their ids, the files the dimmer keeps quiet, the
 * order the status lists them in, and the record kept for one.
 * Pure: the runtime finds the conflicts and stores them.
 */
import { createHash } from "node:crypto";
import { CONFLICT_LEVELS, type Conflict, type ConflictSide } from "./model.js";
import { type ConflictRecord, sanitizeIntent, type SideId } from "./conflict-record.js";

/** How many resolved, and how many dismissed, conflicts the status lists; every open one is listed. */
export const SETTLED_CONFLICTS_SHOWN = 20;

/** Files whose overlaps the dimmer keeps on the Conflicts page only: lockfiles, changelogs, generated output and docs. */
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

/** The six hex characters a person types to name a conflict in chat (`#a1b2c3`). */
export function shortConflictId(id: string): string {
  return createHash("sha256").update(id).digest("hex").slice(0, 6);
}

/** Whether every one of `files` is a lockfile, a changelog, generated output or under `docs/`: the dimmer logs it only. */
export function quietFiles(files: readonly string[]): boolean {
  return files.length > 0 && files.every(file => QUIET_FILES.some(pattern => pattern.test(file)));
}

/** A newly found conflict's dimmer fields before the dimmer has seen it: logged, unmeasured, not queued or snoozed. */
export function undimmed(id: string): Pick<Conflict, "shortId" | "level" | "signals" | "score" | "reasons" | "queued" | "queuedFor" | "snooze"> {
  return { shortId: shortConflictId(id), level: "logged", signals: null, score: 0, reasons: [], queued: false, queuedFor: null, snooze: null };
}

/** Open conflicts first (loudest first, then newest), then the latest resolved, then the latest dismissed. */
export function orderConflicts(conflicts: readonly Conflict[]): Conflict[] {
  const latest = (status: Conflict["status"]): Conflict[] => conflicts
    .filter(conflict => conflict.status === status)
    .sort((left, right) => (status === "open" ? CONFLICT_LEVELS.indexOf(right.level) - CONFLICT_LEVELS.indexOf(left.level) : 0)
      || (right.resolution?.at ?? right.detectedAt).localeCompare(left.resolution?.at ?? left.detectedAt));
  return [...latest("open"), ...latest("resolved").slice(0, SETTLED_CONFLICTS_SHOWN), ...latest("dismissed").slice(0, SETTLED_CONFLICTS_SHOWN)];
}

/** How a person knows a ref: `PR #12`, a branch name, or a short commit. */
export function refLabel(ref: string): string {
  const pull = /^refs\/pull\/(\d+)\/head$/u.exec(ref);
  if (pull !== null) return `PR #${pull[1]}`;
  if (/^[0-9a-f]{40,64}$/u.test(ref)) return ref.slice(0, 12);
  return ref.replace(/^refs\/(?:heads|remotes)\//u, "");
}

/** A conflict's record, as the resolver reads one: both sides' refs, commits, sanitized intents and files, on `question`. */
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
