import type { CodeRuleViolation } from "../code-rule/code-rule.js";
import { folderContains, type Decider, type RepositoryKey, type RuleSourceKind } from "../catalog/repository-model.js";
import type { RuleMode } from "../catalog/rule-mode.js";

export const AUDIT_SCOPES = ["repository", "touched-files", "ai-output"] as const;
export const AUDIT_MODES = ["steer", "log"] as const;
export const AUDIT_STATUSES = ["running", "completed", "failed", "interrupted", "superseded"] as const;
export type AuditScope = (typeof AUDIT_SCOPES)[number];
export type AuditMode = (typeof AUDIT_MODES)[number];
export type AuditStatus = (typeof AUDIT_STATUSES)[number];

/** Audits recorded before repository-owned rules: the rule's project-catalog applicability. */
export interface LegacyAuditRuleProvenance {
  ruleId: string;
  revision: number;
  payloadHash: string;
  applicability: "global" | "project";
  projectId?: string;
  origin: "user" | "rerail";
}

/** How a rule ran in one folder of its checkout, and which owner decided `on` and `mode` there. */
export interface RulePlacementDecision {
  folder: string;
  on: boolean;
  mode: RuleMode;
  onFrom: Decider;
  modeFrom: Decider;
}

/** A rule a repository's configuration made present in the checkout the audit evaluated. */
export interface RepositoryAuditRuleProvenance {
  ruleId: string;
  revision: number;
  payloadHash: string;
  source: { kind: RuleSourceKind; id: string; packId: string | null };
  repositoryKey: RepositoryKey;
  checkoutPath: string;
  /** Every folder the rule is present in, sorted; the deepest folder containing a path decides there. */
  placement: RulePlacementDecision[];
}

export type AuditRuleProvenance = LegacyAuditRuleProvenance | RepositoryAuditRuleProvenance;

/**
 * What decided a violation's rule at its path (relative to the audit's checkout): the deepest placement folder that
 * contains it. Undefined for legacy provenance, a rule the audit did not record, or a violation without a path.
 */
export function decisionFor(
  audit: { ruleProvenance: readonly AuditRuleProvenance[] },
  violation: Pick<CodeRuleViolation, "ruleId" | "path">,
): RulePlacementDecision | undefined {
  const path = violation.path;
  if (path === undefined) return undefined;
  const provenance = audit.ruleProvenance.find(entry => entry.ruleId === violation.ruleId && "placement" in entry);
  if (provenance === undefined || !("placement" in provenance)) return undefined;
  let decision: RulePlacementDecision | undefined;
  for (const entry of provenance.placement) {
    if (folderContains(entry.folder, path) && (decision === undefined || entry.folder.length > decision.folder.length)) decision = entry;
  }
  return decision;
}

export interface AuditFailure {
  ruleId?: string;
  clauseId: string;
  code: "budget-exceeded" | "missing-capability" | "source-incomplete" | "source-partial" | "adapter-error";
  message: string;
  path?: string | undefined; // NOSONAR - exactOptionalPropertyTypes: zod-parsed native failures carry `path: undefined`
  stage?: "read" | "parse" | "evaluate" | "scan" | "scope" | "persistence" | undefined; // NOSONAR - as `path`
}

/**
 * The head of the message every AST clause fails with (`source-incomplete`) on a file the parser did not read: its
 * syntax-error regions cover half of its bytes or more. The full message follows it with `: error regions cover N% of
 * it, first at line L, column C` (`native/src/code_rule_dsl/soundness.rs` `UNREAD`).
 */
export const UNREAD_FAILURE_HEAD = "file did not parse";

/** Whether `failure` reports a file the parser did not read (`UNREAD_FAILURE_HEAD`), on any evaluation path. */
export function isUnreadFailure(failure: { code: string; message: string }): boolean {
  return failure.code === "source-incomplete" && failure.message.startsWith(`${UNREAD_FAILURE_HEAD}: `);
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
