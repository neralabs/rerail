/**
 * What Merge Assist reports for one checkout: the shapes the runtime service returns and the API, CLI and dashboard
 * show. See docs/merge-assist-v1-build.md.
 */
import { z } from "zod";

/**
 * `unchecked`: Merge Assist is off for the repository, so Rerail ran no `gh`; `unavailable`: `gh` is installed but its
 * sign-in could not be read this time (it timed out, crashed or printed something unexpected).
 */
export const GITHUB_STATES = ["signed-in", "signed-out", "missing", "not-github", "unchecked", "unavailable"] as const;
export type GithubState = (typeof GITHUB_STATES)[number];

export const githubReadinessSchema = z.object({
  state: z.enum(GITHUB_STATES),
  login: z.string().nullable(),
  repository: z.string().nullable().describe("owner/name on GitHub, when the repository's remote is GitHub."),
});
export type GithubReadiness = z.infer<typeof githubReadinessSchema>;

export const lastScanSchema = z.object({
  at: z.string(),
  peers: z.object({ branches: z.number().int(), pullRequests: z.number().int() })
    .describe("The live branches and open pull requests the scan compared this checkout's work with."),
  problems: z.array(z.string()).describe("What the scan could not read, one sentence each."),
});
export type LastScan = z.infer<typeof lastScanSchema>;

/** overlap: overlapping hunks with a branch or pull request in flight. */
export const CONFLICT_KINDS = ["overlap"] as const;
export type ConflictKind = (typeof CONFLICT_KINDS)[number];

/** open: found at the last detection; dismissed: the person said it is not a conflict, which holds for its id. */
export const CONFLICT_STATUSES = ["open", "dismissed"] as const;
export type ConflictStatus = (typeof CONFLICT_STATUSES)[number];

export const conflictSideSchema = z.object({
  label: z.string().describe("\"This branch\" for this checkout's side; \"PR #398\" or a branch name for theirs."),
  ref: z.string(),
  sha: z.string(),
  author: z.string().nullable(),
  tip: z.object({ at: z.string(), pushed: z.boolean() }).nullable()
    .describe("When the side's commit was made, and whether it is pushed (on a remote-tracking ref or a pull request) or only committed locally; null for this branch's working state."),
  url: z.string().nullable(),
  summary: z.string().describe("One line: the subject of the commit that changed the side's contested lines, else theirs' pull request title, else \"edits <file>\"."),
  location: z.string().nullable().describe("Where the side's overlapping hunk starts: \"src/x.ts:469\"."),
  hunk: z.string().nullable().describe("That side's conflicting hunk, at most 40 lines of unified diff."),
});
export type ConflictSide = z.infer<typeof conflictSideSchema>;

/** Who dismissed a conflict as not a conflict, and when. */
export const conflictDismissalSchema = z.object({
  name: z.string(),
  at: z.string(),
});
export type ConflictDismissal = z.infer<typeof conflictDismissalSchema>;

/** What fires the conflict hook, once each per conflict: `detected`, first seen in this checkout; `closed`, dismissed or no longer found. */
export const CONFLICT_EVENTS = ["detected", "closed"] as const;
export type ConflictEvent = (typeof CONFLICT_EVENTS)[number];

/** The conflict hook command's timeout: 10 seconds unless the person sets another, at most 60. */
export const DEFAULT_HOOK_TIMEOUT_SECONDS = 10;
export const MAX_HOOK_TIMEOUT_SECONDS = 60;

export const conflictHookCommandSchema = z.object({
  command: z.array(z.string().min(1).max(4_096)).min(1).max(64)
    .describe("The program and its arguments, run without a shell from the checkout's root; the conflict goes on stdin only."),
  timeoutSeconds: z.number().int().min(1).max(MAX_HOOK_TIMEOUT_SECONDS)
    .describe(`Seconds before it is stopped: ${DEFAULT_HOOK_TIMEOUT_SECONDS} by default, at most ${MAX_HOOK_TIMEOUT_SECONDS}.`),
});
export type ConflictHookCommand = z.infer<typeof conflictHookCommandSchema>;

/** A person's Merge Assist settings for one repository, kept on this machine only. */
export const mergeAssistSettingsSchema = z.object({
  hook: conflictHookCommandSchema.nullable().describe("The conflict hook command, run on every conflict event; null for none."),
});
export type MergeAssistSettings = z.infer<typeof mergeAssistSettingsSchema>;
export const DEFAULT_MERGE_ASSIST_SETTINGS: Readonly<MergeAssistSettings> = Object.freeze({ hook: null });

/** What the conflict hook command answered to the latest event it ran for. */
export const conflictHookOutcomeSchema = z.object({
  event: z.enum(CONFLICT_EVENTS),
  ok: z.boolean(),
  error: z.string().nullable().describe("Why it failed: a non-zero exit, a timeout, or output that is not the hook's JSON."),
  note: z.string().nullable().describe("Added to the agent's note for the conflict."),
  status: z.string().nullable().describe("Replaces the conflict's summary wherever its line shows."),
  at: z.string(),
});
export type ConflictHookOutcome = z.infer<typeof conflictHookOutcomeSchema>;

export const conflictSchema = z.object({
  id: z.string().describe("Stable: \"conflict:\" + question + \":\" + a short hash of theirs.ref."),
  shortId: z.string().describe("Six hex characters its card names it by."),
  kind: z.enum(CONFLICT_KINDS),
  status: z.enum(CONFLICT_STATUSES),
  quiet: z.boolean().describe("Every contested file is a lockfile, a changelog, generated output or under docs/: listed on the dashboard and in the CLI only, never told to the agent, the person's line or the status line."),
  question: z.string().describe("What the sides collide on: the file and the base lines their overlapping change covers."),
  title: z.string(),
  mine: conflictSideSchema,
  theirs: conflictSideSchema,
  evidence: z.array(z.string()).describe("Why Rerail thinks it is a conflict, one sentence each."),
  detectedAt: z.string(),
  dismissal: conflictDismissalSchema.nullable().describe("Who dismissed it and when; null while it is open."),
  summary: z.string().describe("Its line: \"<their label> (<author>) is changing <symbol or file>\"; the hook's status replaces it where the line shows."),
  hook: conflictHookOutcomeSchema.nullable().default(null).describe("The conflict hook command's last answer; null while none is set or none ran."),
});
export type Conflict = z.infer<typeof conflictSchema>;

export const mergeAssistStatusSchema = z.object({
  repositoryKey: z.string(),
  checkout: z.string(),
  branch: z.string().nullable(),
  baseBranch: z.string().nullable(),
  enabled: z.boolean(),
  settings: mergeAssistSettingsSchema,
  readiness: z.object({
    github: githubReadinessSchema,
  }),
  scanning: z.boolean(),
  lastScan: lastScanSchema.nullable(),
  conflicts: z.array(conflictSchema).describe("Open first (the ones not quiet first, then newest first), then dismissed (latest 20)."),
});
export type MergeAssistStatus = z.infer<typeof mergeAssistStatusSchema>;
