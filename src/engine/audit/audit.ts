import type { CodeRuleViolation } from "../code-rule/code-rule.js";
import { folderContains, type Decider, type RepositoryKey, type RuleSourceKind, type RuleSuppressions } from "../catalog/repository-model.js";
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

/**
 * How a rule ran in one folder of its checkout, and which owner decided `on`, `mode` and `suppressions` there.
 * `suppressions` is absent from audits recorded before it existed (they honoured suppression comments).
 */
export interface RulePlacementDecision {
  folder: string;
  on: boolean;
  mode: RuleMode;
  onFrom: Decider;
  modeFrom: Decider;
  suppressions?: RuleSuppressions;
  suppressionsFrom?: Decider;
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

export const AUDIT_BLIND_REASONS = ["budget", "syntax"] as const;
/** Why no clause read a file: its work ran out (`budget`), or its syntax errors kept every clause out (`syntax`). */
export type AuditBlindReason = (typeof AUDIT_BLIND_REASONS)[number];
/**
 * What a failure is. `budget-exceeded`: a clause ran out of its own work (or a scan or file guardrail stopped it).
 * `file-budget-exceeded`: the file's work stopped a clause, not the clause's own: its shared work (parse, tape, anchor
 * index) ran out, or the rules before it spent the file's work ceiling. `witness-limit`: a clause found more than
 * `maxWitnesses` lets it report; it read the file, so this is no coverage gap and it fails the audit. `input-limit`:
 * the file is past the rule's input limits (`maxSourceBytes`, `maxFiles`), so the rule never ran on it; it fails the
 * audit.
 */
export const AUDIT_FAILURE_CODES = [
  "budget-exceeded", "file-budget-exceeded", "witness-limit", "input-limit", "missing-capability", "source-incomplete",
  "source-partial", "adapter-error",
] as const;

export interface AuditFailure {
  ruleId?: string;
  clauseId: string;
  code: (typeof AUDIT_FAILURE_CODES)[number];
  message: string;
  path?: string | undefined; // NOSONAR - exactOptionalPropertyTypes: zod-parsed native failures carry `path: undefined`
  stage?: "read" | "parse" | "evaluate" | "scan" | "scope" | "persistence" | undefined; // NOSONAR - as `path`
  /**
   * Set on a blind file's one failure, which stands for every clause that failed on it: no clause read the file. A file
   * blind for its work (`budget`) is a coverage gap: it never fails the audit. One blind for its syntax fails it, as any
   * rule kept out by the file's syntax does. Neither is cut from the list.
   */
  blind?: AuditBlindReason | undefined; // NOSONAR - as `path`
}

/** The paths of the blind files among `failures` (`AuditFailure.blind`), in order. */
export function blindFilePaths(failures: readonly AuditFailure[]): string[] {
  return failures.flatMap((failure) => (failure.blind !== undefined && failure.path !== undefined ? [failure.path] : []));
}

/** Whether `failure` is a rule running out of its own work on a file while evaluating it (at no read or scan stage). */
export function isOutOfWork(failure: AuditFailure): boolean {
  return failure.code === "budget-exceeded" && failure.blind === undefined && failure.ruleId !== undefined
    && failure.stage !== "read" && failure.stage !== "scan";
}
/** Whether `failure` is a rule the file's work stopped on a file (`file-budget-exceeded`, at no read or scan stage). */
export function isFileWorkStopped(failure: AuditFailure): boolean {
  return failure.code === "file-budget-exceeded" && failure.blind === undefined && failure.ruleId !== undefined
    && failure.stage !== "read" && failure.stage !== "scan";
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

/** Why a file with a NUL byte was not evaluated; the native index reports the same text (`workspace/index.rs`). */
export const BINARY_SOURCE_MESSAGE = "Binary source cannot be evaluated";

/**
 * Whether a failure only says one file was read in part: its rules ran around its syntax errors (`source-partial`),
 * a rule that needs the whole file skipped it for them, the parser did not read it at all (`isUnreadFailure`), it is
 * binary, or its syntax kept every rule out (a blind file for its syntax). A repository audit lists these and still
 * completes: a large checkout with a few files Rerail cannot fully parse still gets its baseline.
 */
export function isSourceGap(failure: AuditFailure): boolean {
  if (failure.path === undefined) return false;
  if (failure.blind === "syntax" || failure.code === "source-partial") return true;
  return failure.code === "source-incomplete"
    && (failure.message === BINARY_SOURCE_MESSAGE || failure.message.startsWith("file has syntax errors") || isUnreadFailure(failure));
}

/**
 * The failure an audit recorded, before audits kept the count in `unlistedFailureCount`, after the failures it kept
 * to say how many more it dropped at its cap. Audits a database already holds may end with one; none is written now.
 */
export function omittedFailuresSummary(count: number): AuditFailure {
  return {
    clauseId: "scan", code: "budget-exceeded", stage: "scan",
    message: `${count} further ${count === 1 ? "failure" : "failures"} not recorded`,
  };
}

export function isOmittedFailuresSummary(failure: Pick<AuditFailure, "stage" | "code" | "message">): boolean {
  return failure.stage === "scan" && failure.code === "budget-exceeded" && /^\d+ further failures? not recorded$/u.test(failure.message);
}

/**
 * Whether a failure is a coverage gap rather than a failure of the audit: a file blind for its work, a rule that ran
 * out of its own work on a file (`isOutOfWork`), or a rule the file's work stopped (`isFileWorkStopped`). The audit
 * reports gaps, with what would close them, and still completes; a file past the byte budget, a scan guardrail, an
 * engine error and an output cap fail it, and a file's syntax keeping rules out (`isSourceGap`) fails any audit but a
 * repository one. `latestAuditSummary` counts gaps by the same test, in SQL.
 */
export function isCoverageGap(failure: AuditFailure): boolean {
  if (failure.blind !== undefined) return failure.blind === "budget";
  return isOutOfWork(failure) || isFileWorkStopped(failure);
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
  /** How many failures the audit had beyond those `failures` lists (the list is capped); 0 when omitted. */
  unlistedFailureCount?: number;
}

export interface AuditRecord extends Omit<AuditInput, "ruleProvenance"> {
  ruleProvenance: AuditRuleProvenance[];
  auditId: string;
  startedAt: number;
  finishedAt: number | null;
  status: AuditStatus;
  violations: CodeRuleViolation[];
  failures: AuditFailure[];
  /** Every failure the audit had: those `failures` lists, and `unlistedFailureCount` it had beyond them. */
  failureCount: number;
  unlistedFailureCount: number;
  /** The files no clause read (`blindFilePaths` of `failures`), and how many. */
  blindFiles: string[];
  blindFileCount: number;
}

/** Bounded session status, independent of the size of stored violation payloads. */
export interface AuditSummary {
  workspaceRoot: string;
  scope: AuditScope;
  mode: AuditMode;
  status: AuditStatus;
  violationCount: number;
  /** Every failure the audit had, listed or not (`AuditRecord.failureCount`). */
  failureCount: number;
  /** Of `failureCount`, those the capped list leaves out. */
  unlistedFailureCount: number;
  /** Of the listed failures, the blind files': coverage gaps of a completed audit. Blind files are always listed. */
  blindFileCount: number;
  /** Of the listed failures, every coverage gap's (`isCoverageGap`): blind files and rules that ran out of work. */
  gapCount: number;
  /** The rules that ran out of work on some file without leaving it blind; each keeps a listed failure. */
  outOfWorkRuleCount: number;
  /** The rules the file's work stopped (`file-budget-exceeded`) on some file without leaving it blind. */
  fileWorkStoppedRuleCount: number;
}

/** Audit bookkeeping is separate from code-rule violations. */
export interface AuditRecorder {
  readonly supportsResultPages?: boolean;
  start(input: AuditInput): string;
  finish(auditId: string, result: AuditResult): void;
  progress(auditId: string, result: Pick<AuditResult, "violations" | "failures" | "unlistedFailureCount">): void;
  appendResultPage?(auditId: string, pageIndex: number, result: Pick<AuditResult, "violations" | "failures">): void;
  finishResultPages?(auditId: string, status: AuditStatus, unlistedFailureCount?: number): void;
  interrupt(sessionId: string): void;
}
