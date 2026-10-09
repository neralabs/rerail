import type { CodeRuleViolation } from "./code-rule.js";

export const AUDIT_SCOPES = ["repository", "touched-files", "ai-output"] as const;
export const AUDIT_MODES = ["steer", "log"] as const;
export const AUDIT_STATUSES = ["running", "completed", "failed", "interrupted", "superseded"] as const;
export type AuditScope = (typeof AUDIT_SCOPES)[number];
export type AuditMode = (typeof AUDIT_MODES)[number];
export type AuditStatus = (typeof AUDIT_STATUSES)[number];

export interface AuditRuleProvenance {
  ruleId: string;
  revision: number;
  payloadHash: string;
  applicability: "global" | "project";
  projectId?: string;
  origin: "user" | "rerail";
}

export interface AuditFailure {
  ruleId?: string;
  clauseId: string;
  code: "budget-exceeded" | "missing-capability" | "source-incomplete" | "source-partial" | "adapter-error";
  message: string;
  path?: string | undefined; // NOSONAR - exactOptionalPropertyTypes: zod-parsed native failures carry `path: undefined`
  stage?: "read" | "parse" | "evaluate" | "scan" | "scope" | "persistence" | undefined; // NOSONAR - as `path`
}

export const AUDIT_COVERAGE_STATUSES = ["complete", "partial", "failed"] as const;

/** One failure code against a clause: how many failures, and at most three example paths, the first in path order. */
export interface AuditCoverageReason {
  code: AuditFailure["code"];
  count: number;
  paths: string[];
}

/**
 * A clause's coverage, which the native evaluation (`eval/output.rs` `coverage`) derives from its failures alone:
 * `complete` without any, `partial` where each is `source-partial` or `source-incomplete`, else `failed`. A failure
 * naming no evaluated clause (the `runtime` budget) counts against every clause of its rule.
 */
export interface ClauseCoverage {
  clauseId: string;
  ruleId?: string;
  status: (typeof AUDIT_COVERAGE_STATUSES)[number];
  reasons: AuditCoverageReason[];
}

export interface AuditInput {
  sessionId: string;
  workspaceRoot: string;
  scope: AuditScope;
  mode: AuditMode;
  revision: number;
  /** Digest of the compiled code-rule catalog; null when the audit never reached evaluation. */
  catalogRevision: string | null;
  /** Exact compiled rule revisions; empty when the audit never reached evaluation. */
  ruleRevisions: Array<{ ruleId: string; revision: number }>;
  paths: string[];
  ruleProvenance?: AuditRuleProvenance[];
}

export interface AuditResult {
  status: Exclude<AuditStatus, "running">;
  violations?: CodeRuleViolation[];
  failures?: AuditFailure[];
}

export interface AuditRecord extends Omit<AuditInput, "ruleProvenance"> {
  ruleProvenance: AuditRuleProvenance[];
  auditId: string;
  startedAt: number;
  finishedAt: number | null;
  status: AuditStatus;
  violations: CodeRuleViolation[];
  failures: AuditFailure[];
}

/** Bounded session status, independent of the size of stored violation payloads. */
export interface AuditSummary {
  workspaceRoot: string;
  scope: AuditScope;
  mode: AuditMode;
  status: AuditStatus;
  violationCount: number;
  failureCount: number;
}

/** Audit bookkeeping is separate from code-rule violations. */
export interface AuditRecorder {
  readonly supportsResultPages?: boolean;
  start(input: AuditInput): string;
  finish(auditId: string, result: AuditResult): void;
  progress(auditId: string, result: Pick<AuditResult, "violations" | "failures">): void;
  appendResultPage?(auditId: string, pageIndex: number, result: Pick<AuditResult, "violations" | "failures">): void;
  finishResultPages?(auditId: string, status: AuditStatus): void;
  interrupt(sessionId: string): void;
}
