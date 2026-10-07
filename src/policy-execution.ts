import type { ExecutablePolicyFinding } from "./executable-policy.js";

export const POLICY_SCOPES = ["repository", "touched-files", "ai-output"] as const;
export const POLICY_MODES = ["steer", "log"] as const;
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
  origin: "user" | "rerail";
}

export interface PolicyExecutionFailure {
  ruleId?: string;
  clauseId: string;
  code: "budget-exceeded" | "missing-capability" | "source-incomplete" | "source-partial" | "adapter-error";
  message: string;
  path?: string | undefined;
  stage?: "read" | "parse" | "evaluate" | "scan" | "scope" | "persistence" | undefined;
}

export const POLICY_COVERAGE_STATUSES = ["complete", "partial", "failed"] as const;

/** One failure code against a clause: how many failures, and at most three example paths, the first in path order. */
export interface PolicyCoverageReason {
  code: PolicyExecutionFailure["code"];
  count: number;
  paths: string[];
}

/**
 * A clause's coverage, which the native evaluation (`eval/output.rs` `coverage`) derives from its failures alone:
 * `complete` without any, `partial` where each is `source-partial` or `source-incomplete`, else `failed`. A failure
 * naming no evaluated clause (the `runtime` budget) counts against every clause of its rule.
 */
export interface PolicyCheckCoverage {
  clauseId: string;
  ruleId?: string;
  status: (typeof POLICY_COVERAGE_STATUSES)[number];
  reasons: PolicyCoverageReason[];
}

export interface PolicyRunInput {
  sessionId: string;
  workspaceRoot: string;
  scope: PolicyScope;
  mode: PolicyMode;
  revision: number;
  /** Digest of the compiled policy catalog, null when an older runtime did not record it. */
  catalogRevision: string | null;
  /** Exact compiled rule revisions; an empty array is honest legacy provenance. */
  ruleRevisions: Array<{ ruleId: string; revision: number }>;
  paths: string[];
  ruleProvenance?: RuleEvaluationProvenance[];
}

export interface PolicyRunResult {
  status: Exclude<PolicyRunStatus, "running">;
  violations?: ExecutablePolicyFinding[];
  failures?: PolicyExecutionFailure[];
}

export interface PolicyRunRecord extends Omit<PolicyRunInput, "ruleProvenance"> {
  ruleProvenance: RuleEvaluationProvenance[];
  auditId: string;
  startedAt: number;
  finishedAt: number | null;
  status: PolicyRunStatus;
  violations: ExecutablePolicyFinding[];
  failures: PolicyExecutionFailure[];
}

/** Bounded session status, independent of the size of stored violation payloads. */
export interface PolicyRunSummary {
  workspaceRoot: string;
  scope: PolicyScope;
  mode: PolicyMode;
  status: PolicyRunStatus;
  violationCount: number;
  failureCount: number;
}

/** Execution bookkeeping is separate from policy violations. */
export interface PolicyExecutionRecorder {
  readonly supportsResultPages?: boolean;
  start(input: PolicyRunInput): string;
  finish(runId: string, result: PolicyRunResult): void;
  progress(runId: string, result: Pick<PolicyRunResult, "violations" | "failures">): void;
  appendResultPage?(runId: string, pageIndex: number, result: Pick<PolicyRunResult, "violations" | "failures">): void;
  finishResultPages?(runId: string, status: PolicyRunStatus): void;
  interrupt(sessionId: string): void;
}
