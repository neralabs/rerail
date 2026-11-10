/**
 * What Merge Assist reports for one checkout: the shapes the runtime service returns and the API, CLI and dashboard
 * show. See docs/merge-assist-v1-build.md §5.
 */
import { z } from "zod";
import { CANDIDATE_GENERALITIES, CANDIDATE_STATES } from "./candidate.js";

/**
 * The agent CLIs Merge Assist can run headless for judgment, in default preference order. Each must be able to run
 * with no tools at all, MCP servers included; OMP and Pi cannot yet (their MCP servers stay connected in print mode).
 */
export const JUDGE_RUNNERS = ["claude", "codex"] as const;
export type JudgeRunnerId = (typeof JUDGE_RUNNERS)[number];

/** Rerail checks a code rule exactly; a note is only ever judged by the person's agent. */
export const CANDIDATE_LEVELS = ["code", "note"] as const;
export type CandidateLevel = (typeof CANDIDATE_LEVELS)[number];

/** absorb: applied automatically; review: waits for the person; talk: contradicts one of their own decisions. */
export const FINDING_TIERS = ["absorb", "review", "talk"] as const;
export type FindingTier = (typeof FINDING_TIERS)[number];

/** open: waiting (review, talk); applied: steering this checkout (absorbed or adopted); dismissed: hidden until it changes. */
export const FINDING_STATUSES = ["open", "applied", "dismissed"] as const;
export type FindingStatus = (typeof FINDING_STATUSES)[number];

export const FINDING_ACTIONS = ["adopt", "dismiss", "undo"] as const;
export type FindingAction = (typeof FINDING_ACTIONS)[number];

export const GITHUB_STATES = ["signed-in", "signed-out", "missing", "not-github"] as const;
export type GithubState = (typeof GITHUB_STATES)[number];

export const judgeReadinessSchema = z.object({
  available: z.boolean(),
  runner: z.enum(JUDGE_RUNNERS).nullable().describe("The agent CLI Rerail runs for judgment, or null when none is found."),
  problem: z.string().nullable().describe("Why judgment is unavailable, in one sentence for a person."),
});
export type JudgeReadiness = z.infer<typeof judgeReadinessSchema>;

export const githubReadinessSchema = z.object({
  state: z.enum(GITHUB_STATES),
  login: z.string().nullable(),
  repository: z.string().nullable().describe("owner/name on GitHub, when the repository's remote is GitHub."),
});
export type GithubReadiness = z.infer<typeof githubReadinessSchema>;

export const ownDecisionSchema = z.object({
  id: z.string(),
  decision: z.string(),
  generality: z.enum(CANDIDATE_GENERALITIES),
  scope: z.array(z.string()),
  level: z.enum(CANDIDATE_LEVELS),
  state: z.enum(CANDIDATE_STATES),
  createdAt: z.string(),
  committed: z.boolean().describe("Whether the candidate file is in the branch's last commit, so pushing shares it."),
  amends: z.object({ id: z.string(), decision: z.string() }).nullable()
    .describe("The decision on this branch this one amends (its supersedes), which it replaces while it is in force."),
  amendedBy: z.string().nullable().describe("The decision on this branch that amends this one, which is then not in force."),
  source: z.object({
    sessionId: z.string().nullable(),
    at: z.string().nullable(),
  }).describe("The session and time of the correction it came from, when Rerail captured it on this machine."),
});
export type OwnDecision = z.infer<typeof ownDecisionSchema>;

export const peerSchema = z.object({
  kind: z.enum(["branch", "pull-request"]),
  ref: z.string().describe("The branch name."),
  commit: z.string(),
  number: z.number().int().nullable(),
  title: z.string().nullable(),
  url: z.string().nullable(),
  author: z.string().nullable().describe("Who GitHub says opened the pull request; null for a local branch. A candidate's own author field is self-declared."),
  trusted: z.boolean().describe("A local branch with no untrusted pull request behind it, or a pull request whose author has write access."),
});
export type Peer = z.infer<typeof peerSchema>;

export const findingSchema = z.object({
  id: z.string(),
  tier: z.enum(FINDING_TIERS),
  status: z.enum(FINDING_STATUSES),
  candidate: z.object({
    id: z.string(),
    decision: z.string(),
    scope: z.array(z.string()),
    level: z.enum(CANDIDATE_LEVELS),
    author: z.string(),
    createdAt: z.string(),
  }),
  peer: peerSchema,
  reason: z.string().describe("Why this tier, in one sentence for a person."),
  evidence: z.object({
    violations: z.array(z.object({ path: z.string(), line: z.number().int(), message: z.string() }))
      .describe("Where this branch's changes break the candidate's code rule."),
    conflicts: z.array(z.string()).describe("Files git merge-tree reports as conflicting between the two branches, within the candidate's scope."),
    mine: z.object({ id: z.string(), decision: z.string() }).nullable().describe("The decision of this branch it contradicts."),
    judgment: z.object({ verdict: z.string(), explanation: z.string() }).nullable(),
  }),
  message: z.string().nullable().describe("For talk: a message the person can send the author."),
  updatedAt: z.string(),
});
export type Finding = z.infer<typeof findingSchema>;

export const lastScanSchema = z.object({
  at: z.string(),
  peers: z.object({ branches: z.number().int(), pullRequests: z.number().int() }),
  problems: z.array(z.string()).describe("What the scan could not read, one sentence each."),
});
export type LastScan = z.infer<typeof lastScanSchema>;

/** overlap: overlapping hunks with a branch in flight; decisions: a peer's decision contradicts one of this branch's. */
export const CONFLICT_KINDS = ["overlap", "decisions"] as const;
export type ConflictKind = (typeof CONFLICT_KINDS)[number];

export const CONFLICT_STATUSES = ["open", "resolved", "dismissed"] as const;
export type ConflictStatus = (typeof CONFLICT_STATUSES)[number];

/** mine: keep this branch's side; theirs: take the other side's; new: a new decision; dismiss: not a conflict. */
export const CONFLICT_ACTIONS = ["mine", "theirs", "new", "dismiss"] as const;
export type ConflictAction = (typeof CONFLICT_ACTIONS)[number];

/**
 * How loud an open conflict is (the dimmer, `dimmer.ts`): logged, on the Conflicts page only; ambient, one line to the
 * agent and the person and a footer segment; checkpoint, a card the agent shows the person at the next boundary;
 * stop, the person is asked before an edit to a contested file.
 */
export const CONFLICT_LEVELS = ["logged", "ambient", "checkpoint", "stop"] as const;
export type ConflictLevel = (typeof CONFLICT_LEVELS)[number];

/**
 * What makes a contested change hard to undo, from the hardest: a persisted schema, migration or file format; a public
 * API (an exported signature in a package entry or index file); a retirement (a deleted file or exported symbol); an
 * exported signature elsewhere; local code.
 */
export const LOCK_IN_CLASSES = ["persisted", "public-api", "retirement", "exported", "local"] as const;
export type LockInClass = (typeof LOCK_IN_CLASSES)[number];

export const conflictSignalsSchema = z.object({
  quiet: z.boolean().describe("Every contested file is a lockfile, a changelog, generated output or under docs/."),
  lockIn: z.union([z.literal(1), z.literal(2), z.literal(4)]),
  lockInClass: z.enum(LOCK_IN_CLASSES),
  blastRadius: z.number().int().nonnegative().describe("callers plus otherRefs."),
  callers: z.number().int().nonnegative().describe("Files in the base referring to the contested symbols, at most 200."),
  otherRefs: z.number().int().nonnegative().describe("Other live refs touching the contested files."),
  growthPerHour: z.number().nonnegative(),
  linesTheirs: z.number().int().nonnegative().describe("Lines theirs added to the contested files since their work forked."),
  linesMine: z.number().int().nonnegative().describe("Lines this branch, committed and working, added to them since it forked."),
  windowHours: z.number().nonnegative().describe("Hours since the two sides' work forked (at least 1): growth's window."),
  pLose: z.number().min(0).max(1),
  hoursToMerge: z.number().nonnegative(),
  pullRequest: z.boolean().describe("A pull request is open on theirs."),
  verdict: z.boolean().describe("This checkout holds a resolution of a conflict on the same question."),
});
export type ConflictSignals = z.infer<typeof conflictSignalsSchema>;

/** "Later": the person put the conflict off until its level rises, theirs opens a pull request, or the base moves. */
export const conflictSnoozeSchema = z.object({
  at: z.string(),
  level: z.enum(CONFLICT_LEVELS).describe("The level when snoozed: a higher one ends the snooze."),
  pullRequest: z.boolean().describe("Whether theirs had a pull request open: one opening ends the snooze."),
  base: z.string().describe("The base branch's tip: a merge that moves it ends the snooze."),
  endedAt: z.string().nullable().describe("When the snooze ended; null while it holds."),
});
export type ConflictSnooze = z.infer<typeof conflictSnoozeSchema>;

export const conflictSideSchema = z.object({
  label: z.string().describe("\"This branch\", \"This session\", \"PR #398\" or a branch name."),
  ref: z.string(),
  sha: z.string(),
  author: z.string().nullable(),
  tip: z.object({ at: z.string(), pushed: z.boolean() }).nullable()
    .describe("When the side's commit was made, and whether it is pushed (on a remote-tracking ref or a pull request) or only committed locally; null for this branch's working state."),
  url: z.string().nullable(),
  summary: z.string().describe("One sentence: the side's design or change."),
  location: z.string().nullable().describe("Where: \"src/x.ts:469\" or \"docs/x.md §9\"."),
  hunk: z.string().nullable().describe("That side's conflicting hunk, at most 40 lines of unified diff."),
  decisionId: z.string().nullable().describe("For kind decisions: the candidate id on that side."),
});
export type ConflictSide = z.infer<typeof conflictSideSchema>;

export const conflictResolutionSchema = z.object({
  action: z.enum(CONFLICT_ACTIONS),
  name: z.string(),
  rule: z.string().nullable(),
  text: z.string().nullable(),
  at: z.string(),
});
export type ConflictResolution = z.infer<typeof conflictResolutionSchema>;

/**
 * What fires the conflict hook, once each per conflict: `detected`, first seen in this checkout; `level-changed`, its
 * level rose; `closed`, dismissed, resolved, or no longer found.
 */
export const CONFLICT_EVENTS = ["detected", "level-changed", "closed"] as const;
export type ConflictEvent = (typeof CONFLICT_EVENTS)[number];

/**
 * Where a conflict reaches the person: `chat`, through the coding agent (its note and checkpoint card); `terminal`, the
 * status line and the person's line only; `both`. A stop before an edit to a contested file goes out in every mode.
 */
export const CONFLICT_DELIVERY_MODES = ["chat", "terminal", "both"] as const;
export type ConflictDelivery = (typeof CONFLICT_DELIVERY_MODES)[number];
export const DEFAULT_CONFLICT_DELIVERY: ConflictDelivery = "both";

/** The conflict hook command's timeout: 10 seconds unless the person sets another, at most 60. */
export const DEFAULT_HOOK_TIMEOUT_SECONDS = 10;
export const MAX_HOOK_TIMEOUT_SECONDS = 60;
/** The longest summary line, in characters. */
export const MAX_CONFLICT_SUMMARY = 90;

export const conflictHookCommandSchema = z.object({
  command: z.array(z.string().min(1).max(4_096)).min(1).max(64)
    .describe("The program and its arguments, run without a shell from the checkout's root; the conflict goes on stdin only."),
  timeoutSeconds: z.number().int().min(1).max(MAX_HOOK_TIMEOUT_SECONDS)
    .describe(`Seconds before it is stopped: ${DEFAULT_HOOK_TIMEOUT_SECONDS} by default, at most ${MAX_HOOK_TIMEOUT_SECONDS}.`),
});
export type ConflictHookCommand = z.infer<typeof conflictHookCommandSchema>;

/** A person's Merge Assist settings for one repository, kept on this machine only. */
export const mergeAssistSettingsSchema = z.object({
  delivery: z.enum(CONFLICT_DELIVERY_MODES).describe("Where conflicts reach the person: chat (through the agent), terminal (status line and the person's line), or both."),
  hook: conflictHookCommandSchema.nullable().describe("The conflict hook command, run on every conflict event; null for none."),
});
export type MergeAssistSettings = z.infer<typeof mergeAssistSettingsSchema>;
export const DEFAULT_MERGE_ASSIST_SETTINGS: MergeAssistSettings = { delivery: DEFAULT_CONFLICT_DELIVERY, hook: null };

/** The conflict's line for the status line: the quick call's summary, or the deterministic fallback until it answers. */
export const conflictSummarySchema = z.object({
  text: z.string(),
  via: z.enum(["judge", "fallback"]),
  at: z.string(),
  reason: z.string().nullable().describe("Why the fallback stands: the budget was used, the judge was unavailable, or it failed; null otherwise."),
});
export type ConflictSummary = z.infer<typeof conflictSummarySchema>;

/** What the conflict hook command answered to the latest event it ran for. */
export const conflictHookOutcomeSchema = z.object({
  event: z.enum(CONFLICT_EVENTS),
  ok: z.boolean(),
  error: z.string().nullable().describe("Why it failed: a non-zero exit, a timeout, or output that is not the hook's JSON."),
  note: z.string().nullable().describe("Added to the agent's note for the conflict when the delivery includes chat."),
  status: z.string().nullable().describe("Replaces the conflict's status line text when the delivery includes terminal."),
  at: z.string(),
});
export type ConflictHookOutcome = z.infer<typeof conflictHookOutcomeSchema>;

export const conflictSchema = z.object({
  id: z.string().describe("Stable: \"conflict:\" + question + \":\" + a short hash of theirs.ref."),
  shortId: z.string().describe("Six hex characters a person can type to name the conflict in chat."),
  kind: z.enum(CONFLICT_KINDS),
  status: z.enum(CONFLICT_STATUSES),
  level: z.enum(CONFLICT_LEVELS).describe("While open, it never drops; resolution resets it."),
  signals: conflictSignalsSchema.nullable().describe("What the dimmer measured at the last detection; null before the first."),
  score: z.number().nonnegative().describe("Expected rework lines if it is left to merge."),
  reasons: z.array(z.string()).describe("One plain sentence per signal, as the card shows them."),
  queued: z.boolean().describe("Over the interruption budget: shown below its level until the budget allows it."),
  queuedFor: z.enum(CONFLICT_LEVELS).nullable().describe("While queued, the level it waits for; it rises to it once the budget frees."),
  snooze: conflictSnoozeSchema.nullable(),
  question: z.string().describe("What the sides disagree on: for an overlap, the file and their changed base lines; for a contradiction, the decisions where each started."),
  title: z.string(),
  mine: conflictSideSchema,
  theirs: conflictSideSchema,
  evidence: z.array(z.string()).describe("Why Rerail thinks it is a conflict, one sentence each."),
  detectedAt: z.string(),
  resolution: conflictResolutionSchema.nullable(),
  summary: conflictSummarySchema.nullable().default(null).describe("The status line's words for it; null only before its first detection is stored."),
  hook: conflictHookOutcomeSchema.nullable().default(null).describe("The conflict hook command's last answer; null while none is set or none ran."),
});
export type Conflict = z.infer<typeof conflictSchema>;

export const mergeAssistStatusSchema = z.object({
  repositoryKey: z.string(),
  checkout: z.string(),
  branch: z.string().nullable(),
  baseBranch: z.string().nullable(),
  enabled: z.boolean(),
  experimental: z.object({
    conflictVerdicts: z.boolean().describe("Experimental conflict verdicts (RERAIL_EXPERIMENTAL_CONFLICT_VERDICTS=1 where Merge Assist runs): resolving a conflict with mine, theirs or new, and answering a card from chat. Off, a conflict can only be dismissed."),
  }),
  settings: mergeAssistSettingsSchema,
  readiness: z.object({
    judge: judgeReadinessSchema,
    github: githubReadinessSchema,
    candidatesIgnored: z.boolean().describe("Whether Git ignores .rerail/candidates/ here, so candidates stay local."),
    rulesActive: z.boolean().describe("Whether Rerail is set up and started in this checkout, so candidate rules can steer. Merge Assist never sets a repository up itself."),
  }),
  budget: z.object({ used: z.number().int(), limit: z.number().int(), windowMinutes: z.number().int() }),
  scanning: z.boolean(),
  lastScan: lastScanSchema.nullable(),
  own: z.array(ownDecisionSchema),
  findings: z.array(findingSchema),
  conflicts: z.array(conflictSchema).describe("Open first, then resolved (latest 20), then dismissed (latest 20)."),
});
export type MergeAssistStatus = z.infer<typeof mergeAssistStatusSchema>;
