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

/** ambient: one line to the agent and the person; logged: on the Conflicts page only. */
export const CONFLICT_LEVELS = ["ambient", "logged"] as const;
export type ConflictLevel = (typeof CONFLICT_LEVELS)[number];

export const conflictSideSchema = z.object({
  label: z.string().describe("\"This branch\", \"This session\", \"PR #398\" or a branch name."),
  ref: z.string(),
  sha: z.string(),
  author: z.string().nullable(),
  pushedAt: z.string().nullable(),
  url: z.string().nullable(),
  summary: z.string().describe("One sentence: the side's design or change."),
  location: z.string().nullable().describe("Where: \"src/x.ts:469\" or \"docs/x.md §9\"."),
  hunk: z.string().nullable().describe("That side's conflicting hunk, at most 40 lines of unified diff."),
  decisionId: z.string().nullable().describe("For kind decisions: the candidate id on that side."),
});
export type ConflictSide = z.infer<typeof conflictSideSchema>;

export const conflictResolutionSchema = z.object({
  action: z.enum(CONFLICT_ACTIONS),
  by: z.enum(["person", "precedent"]),
  name: z.string(),
  logId: z.string().nullable(),
  rule: z.string().nullable(),
  text: z.string().nullable(),
  at: z.string(),
});
export type ConflictResolution = z.infer<typeof conflictResolutionSchema>;

export const conflictSchema = z.object({
  id: z.string().describe("Stable: \"conflict:\" + question + \":\" + a short hash of theirs.ref."),
  kind: z.enum(CONFLICT_KINDS),
  status: z.enum(CONFLICT_STATUSES),
  level: z.enum(CONFLICT_LEVELS),
  question: z.string().describe("The question key shared with the decision log."),
  title: z.string(),
  mine: conflictSideSchema,
  theirs: conflictSideSchema,
  evidence: z.array(z.string()).describe("Why Rerail thinks it is a conflict, one sentence each."),
  detectedAt: z.string(),
  resolution: conflictResolutionSchema.nullable(),
});
export type Conflict = z.infer<typeof conflictSchema>;

export const decisionLogItemSchema = z.object({
  id: z.string(),
  recordedAt: z.string(),
  title: z.string(),
  outcome: z.string().describe("\"Theirs (PR #398)\", \"Mine\", \"New decision\" or \"Escalated\"."),
  by: z.object({ kind: z.enum(["person", "policy"]), name: z.string() }),
  rule: z.string().nullable(),
  text: z.string().nullable(),
});
export type DecisionLogItem = z.infer<typeof decisionLogItemSchema>;

export const mergeAssistStatusSchema = z.object({
  repositoryKey: z.string(),
  checkout: z.string(),
  branch: z.string().nullable(),
  baseBranch: z.string().nullable(),
  enabled: z.boolean(),
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
  decisionLog: z.object({ entries: z.array(decisionLogItemSchema), problems: z.array(z.string()) }).describe("Newest first, at most 50."),
});
export type MergeAssistStatus = z.infer<typeof mergeAssistStatusSchema>;
