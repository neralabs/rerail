import type { ExecutablePolicyFinding } from "./executable-policy.js";

export const POLICY_SCOPES = ["repository", "touched-files", "ai-output"] as const;
export const POLICY_MODES = ["resteer", "log"] as const;
export const POLICY_RUN_STATUSES = ["running", "completed", "failed", "interrupted", "superseded"] as const;
export type PolicyScope = (typeof POLICY_SCOPES)[number];
export type PolicyMode = (typeof POLICY_MODES)[number];
export type PolicyRunStatus = (typeof POLICY_RUN_STATUSES)[number];

export interface RuleEvaluationProvenance {
  ruleId: string;
  revision: number;
  payloadHash: string;
  applicability: "global" | "project";
  projectId?: string;
  origin: "local" | "bundled" | "nera";
  syncState: "local-only" | "synced" | "pending";
  upstreamRuleId?: string;
}

export interface PolicyExecutionFailure {
  policyId?: string | undefined;
  checkId: string;
  code: "budget-exceeded" | "missing-capability" | "source-incomplete" | "source-partial" | "adapter-error";
  message: string;
  path?: string | undefined;
  stage?: "read" | "parse" | "evaluate" | "scan" | "scope" | "persistence" | undefined;
}

export const POLICY_COVERAGE_STATUSES = ["complete", "partial", "failed"] as const;

/** One failure code against a check: how many failures, and at most three example paths, the first in path order. */
export interface PolicyCoverageReason {
  code: PolicyExecutionFailure["code"];
  count: number;
  paths: string[];
}

/**
 * A check's coverage, which the native evaluation (`eval/output.rs` `coverage`) derives from its failures alone:
 * `complete` without any, `partial` where each is `source-partial` or `source-incomplete`, else `failed`. A failure
 * naming no evaluated check (the `runtime` budget) counts against every check of its policy.
 */
export interface PolicyCheckCoverage {
  checkId: string;
  policyId?: string | undefined;
  status: (typeof POLICY_COVERAGE_STATUSES)[number];
  reasons: PolicyCoverageReason[];
}

export interface PolicyRunInput {
  sessionId: string;
  workspaceRoot: string;
  scope: PolicyScope;
  mode: PolicyMode;
  revision: number;
  /** Digest of the compiled policy catalog; null when the run never reached evaluation. */
  catalogRevision: string | null;
  /** Exact compiled rule revisions; empty when the run never reached evaluation. */
  ruleRevisions: Array<{ ruleId: string; revision: number }>;
  paths: string[];
  ruleProvenance?: RuleEvaluationProvenance[];
}

export interface PolicyRunResult {
  status: Exclude<PolicyRunStatus, "running">;
  findings?: ExecutablePolicyFinding[];
  failures?: PolicyExecutionFailure[];
}

export interface PolicyRunRecord extends Omit<PolicyRunInput, "ruleProvenance"> {
  ruleProvenance: RuleEvaluationProvenance[];
  runId: string;
  startedAt: number;
  finishedAt: number | null;
  status: PolicyRunStatus;
  findings: ExecutablePolicyFinding[];
  failures: PolicyExecutionFailure[];
}

/** Bounded session status, independent of the size of stored finding payloads. */
export interface PolicyRunSummary {
  workspaceRoot: string;
  scope: PolicyScope;
  mode: PolicyMode;
  status: PolicyRunStatus;
  findingCount: number;
  failureCount: number;
}

/** Execution bookkeeping is separate from policy findings. */
export interface PolicyExecutionRecorder {
  readonly supportsResultPages?: boolean;
  start(input: PolicyRunInput): string;
  finish(runId: string, result: PolicyRunResult): void;
  progress(runId: string, result: Pick<PolicyRunResult, "findings" | "failures">): void;
  appendResultPage?(runId: string, pageIndex: number, result: Pick<PolicyRunResult, "findings" | "failures">): void;
  finishResultPages?(runId: string, status: PolicyRunStatus): void;
  interrupt(sessionId: string): void;
}
