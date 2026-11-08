/**
 * The replay's `git-conflict` arms: what they tell the agent about the textual conflicts its work would have with
 * other refs in flight. See benchmark/merge-assist-replay.ts. The conflicts themselves come from the engine's overlap
 * identifier (src/engine/merge-assist/overlap.ts): read-only, no merge, hunks as base lines, renames ignored.
 */
import { spawnSync } from "node:child_process";
import { clipLines, type GitConflict, HUNK_LINES, type RefHunk } from "../src/engine/index.js";
import { fenceData, newJudgeNonce } from "../src/runtime/index.js";

/** The most commit subjects a ref's stated intent quotes when it has no pull request title. */
const INTENT_SUBJECTS = 3;

/** Another ref as visible when the work started, with its changes against the base. */
export interface RefSide {
  /** How the person knows it: `PR #12` or `branch a/b`. */
  readonly label: string;
  readonly ref: string;
  readonly sha: string;
  readonly author: string;
  /** When its first own commit became public. */
  readonly since: string;
  /** Its pull request title and the first paragraph of its body, or its latest commit subjects. */
  readonly intent: string;
  /** Its hunks per file, in base lines. */
  readonly files: Readonly<Record<string, readonly RefHunk[]>>;
}

const ENV: NodeJS.ProcessEnv = { ...process.env, GIT_CONFIG_NOSYSTEM: "1", GIT_CONFIG_GLOBAL: "/dev/null", GIT_OPTIONAL_LOCKS: "0", GIT_TERMINAL_PROMPT: "0" };

/** A read-only git command's output; null when it fails. */
export function gitRead(cwd: string, args: readonly string[]): string | null {
  const result = spawnSync("git", ["--no-optional-locks", "-c", "core.quotePath=false", ...args], { cwd, env: ENV, encoding: "utf8", maxBuffer: 512 * 1024 * 1024 });
  return result.status === 0 ? result.stdout : null;
}

/** A ref's stated intent: its pull request title and first body paragraph, else its latest own commit subjects (after `own`). */
export function statedIntent(repo: string, own: string, sha: string, pr: { title: string; body?: string } | null): string {
  if (pr !== null) {
    const paragraph = (pr.body ?? "").trim().split(/\n\s*\n/u)[0]?.trim() ?? "";
    return paragraph === "" ? pr.title : `${pr.title}\n${paragraph}`;
  }
  return (gitRead(repo, ["log", "--no-merges", `-${INTENT_SUBJECTS}`, "--format=%s", `${own}..${sha}`]) ?? "").trim();
}

/** Their overlapping hunks as one change, at most {@link HUNK_LINES} lines. */
function theirChange(conflict: GitConflict): string {
  return clipLines(conflict.theirs.flatMap(hunk => hunk.text.split("\n")), HUNK_LINES);
}

function sideOf(conflict: GitConflict, refs: readonly RefSide[]): RefSide {
  const side = refs.find(candidate => candidate.ref === conflict.ref);
  if (side === undefined) throw new Error(`no ref ${conflict.ref}`);
  return side;
}

/** `git-conflict`'s text for one conflict: who, where, their change and stated intent fenced as data, then what to do. */
export function pushText(conflict: GitConflict, refs: readonly RefSide[], nonce: string = newJudgeNonce()): string {
  const side = sideOf(conflict, refs);
  return [
    `Git conflict in flight: ${side.label} by ${side.author} (public since ${side.since}) also changes ${conflict.file}; your changes would conflict with theirs. Their change:`,
    fenceData("their change", theirChange(conflict), nonce),
    "Their stated intent:",
    fenceData("their stated intent", side.intent, nonce),
    "Make your code compatible with theirs, unless that is contrary to the approach your task asks for; if it is, keep yours and tell the person in one line which branch you diverge from and why.",
  ].join("\n");
}

/** `git-conflict-pull`'s one line, sent when the set of conflicts changes. */
export function pullLine(conflicts: readonly GitConflict[]): string {
  if (conflicts.length === 0) return "In flight: your changes no longer conflict with other branches (`git conflicts` lists none).";
  const refs = new Set(conflicts.map(conflict => conflict.ref)).size;
  const files = [...new Set(conflicts.map(conflict => conflict.file))];
  return `In flight: your changes would conflict with ${refs} other branch${refs === 1 ? "" : "es"} (${files.join(", ")}). See them with \`git conflicts\`. Decide when to deal with it; before you finish, make your code compatible with theirs unless that is contrary to the approach your task asks for, and tell the person which you diverged from.`;
}

/** What `git conflicts` prints under `git-conflict-pull`: one item per (ref, file), data fenced. */
export function pullFile(conflicts: readonly GitConflict[], refs: readonly RefSide[], nonce: string = newJudgeNonce()): string {
  if (conflicts.length === 0) return "No in-flight conflicts: none of your changes overlap another branch's.\n";
  const items = conflicts.map((conflict, index) => {
    const side = sideOf(conflict, refs);
    return [
      `## ${index + 1}. ${side.label} by ${side.author} (public since ${side.since}): ${conflict.file}`,
      "",
      "Their change:",
      fenceData("their change", theirChange(conflict), nonce),
      "",
      "Their stated intent:",
      fenceData("their stated intent", side.intent, nonce),
    ].join("\n");
  });
  return [
    "# In-flight conflicts",
    "",
    `Your changes would conflict with ${conflicts.length === 1 ? "this change" : `these ${conflicts.length} changes`} on other branches. Before you finish, make your code compatible with theirs unless that is contrary to the approach your task asks for, and tell the person which you diverged from.`,
    "",
    items.join("\n\n"),
    "",
  ].join("\n");
}
