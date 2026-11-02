/**
 * What Merge Assist reports for one checkout: the shapes the runtime service returns and the API, CLI and dashboard
 * show. See docs/merge-assist-prd.md §5.
 */
import { z } from "zod";
import { CANDIDATE_GENERALITIES, CANDIDATE_STATES } from "./candidate.js";

/** The agent CLIs Merge Assist can run headless for judgment, in default preference order. */
export const JUDGE_RUNNERS = ["claude", "codex", "omp"] as const;
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
  trusted: z.boolean().describe("A local branch on this machine, or a pull request whose author has write access."),
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
  }),
  budget: z.object({ used: z.number().int(), limit: z.number().int(), windowMinutes: z.number().int() }),
  scanning: z.boolean(),
  lastScan: lastScanSchema.nullable(),
  own: z.array(ownDecisionSchema),
  findings: z.array(findingSchema),
});
export type MergeAssistStatus = z.infer<typeof mergeAssistStatusSchema>;
