/**
 * In-flight conflicts as Merge Assist reports them (docs/merge-assist-v1-build.md): their questions and ids, which
 * ones are quiet, and the order the status lists them in.
 * Pure: the runtime finds the conflicts and stores them.
 */
import { createHash } from "node:crypto";
import type { Conflict } from "./model.js";

/** How many dismissed conflicts the status lists; every open one is listed. */
export const DISMISSED_CONFLICTS_SHOWN = 20;

/** Files whose overlaps are quiet, listed on the dashboard and in the CLI only: lockfiles, changelogs, generated output and docs. */
const QUIET_FILES: readonly RegExp[] = [
  /(?:^|\/)(?:package-lock\.json|npm-shrinkwrap\.json|pnpm-lock\.yaml|go\.sum|bun\.lockb|[^/]+\.lock)$/iu,
  /(?:^|\/)(?:CHANGELOG|CHANGES|HISTORY|NEWS)(?:\.[a-z]+)?$/iu,
  /(?:^|\/)(?:dist|build|generated|__generated__)\/|\.(?:generated|min)\.[a-z]+$|\.pb\.go$|_pb2\.py$/iu,
  /^docs\//u,
];

/**
 * An overlap's question: its file and the content anchor of the base lines their overlapping change covers
 * (`contestedAnchor`). Another overlap in the same file, or another change of theirs, is another question, so a
 * dismissal of one never settles the other; lines added above it, or a base that moved, keep it.
 */
export function overlapQuestion(file: string, anchor: string): string {
  const digest = createHash("sha256").update(JSON.stringify({ files: [file], symbols: [anchor] })).digest("hex");
  return `overlap:${digest.slice(0, 24)}`;
}

/** A conflict's stable id: its question and a short hash of their ref, so the same overlap keeps its id. */
export function conflictId(question: string, theirsRef: string): string {
  return `conflict:${question}:${createHash("sha256").update(theirsRef).digest("hex").slice(0, 8)}`;
}

/** The six hex characters a conflict's card names it by (`#a1b2c3`). */
export function shortConflictId(id: string): string {
  return createHash("sha256").update(id).digest("hex").slice(0, 6);
}

/** Whether every one of `files` is a lockfile, a changelog, generated output or under `docs/`: a quiet conflict's files. */
export function quietFiles(files: readonly string[]): boolean {
  return files.length > 0 && files.every(file => QUIET_FILES.some(pattern => pattern.test(file)));
}

/** Open conflicts first (the ones not quiet first, each newest first), then the latest dismissed. */
export function orderConflicts(conflicts: readonly Conflict[]): Conflict[] {
  const latest = (status: Conflict["status"]): Conflict[] => conflicts
    .filter(conflict => conflict.status === status)
    .sort((left, right) => (status === "open" ? Number(left.quiet) - Number(right.quiet) : 0)
      || (right.dismissal?.at ?? right.detectedAt).localeCompare(left.dismissal?.at ?? left.detectedAt));
  return [...latest("open"), ...latest("dismissed").slice(0, DISMISSED_CONFLICTS_SHOWN)];
}
