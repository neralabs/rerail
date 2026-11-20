/**
 * What a `.rerail` run found, as the audit records and the session is told it: each finding a `CodeRuleViolation` of
 * its rule at the mode its rule's placement gives the finding's path, and each policy the run could not run an
 * `AuditFailure` of its rule.
 */
import { createHash } from "node:crypto";
import type { AuditFailure } from "../audit/audit.js";
import type { CodeRuleViolation } from "../code-rule/code-rule.js";
import type { RerailRuleEntry } from "../code-rule/code-rule-dsl-catalog.js";
import { rrBudgets } from "./rr-budgets.js";
import type { RrFinding, RrRunOutcome } from "./rr-runner.js";
import { modeAt } from "./rr-rule-scope.js";

/** The clause a failure of a whole `.rerail` rule names: the rule did not run, so no clause of it did. */
const RUN_CLAUSE = "run";

/**
 * The finding's id, as the native evaluation makes a violation's (`violation_id`, `eval/output.rs`): from its rule,
 * clause, code, path and span, the empty range at the file's start for a finding with no span.
 */
function violationId(finding: RrFinding): string {
  const source = [finding.policy, finding.clause, finding.code, finding.path, finding.span?.startByte ?? 0, finding.span?.endByte ?? 0].join("\0");
  return `sha256:${createHash("sha256").update(source).digest("hex")}`;
}

/** `finding` as a violation of `rule`; undefined where the rule's placement does not put it on at the finding's path. */
function violationOf(finding: RrFinding, rule: RerailRuleEntry): CodeRuleViolation | undefined {
  const mode = modeAt(rule.placement, finding.path);
  if (mode === undefined) return undefined;
  return {
    id: violationId(finding),
    ruleId: rule.id,
    clauseId: finding.clause,
    code: finding.code,
    mode,
    message: finding.message,
    path: finding.path,
    ...(finding.span === null ? {} : { span: finding.span }),
    captures: {},
    variables: {},
  };
}

/**
 * The violations of `findings` (paths relative to the checkout the run read) by the rules of `rules`, those `keeps`
 * keeps: a finding of a rule the run did not carry, in no file, or where its rule's placement does not put it on is
 * dropped.
 */
export function rrViolations(
  findings: readonly RrFinding[],
  rules: readonly RerailRuleEntry[],
  keeps: (path: string) => boolean,
): CodeRuleViolation[] {
  const byId = new Map(rules.map(rule => [rule.id, rule]));
  return findings.flatMap(finding => {
    const rule = byId.get(finding.policy);
    if (rule === undefined || finding.path === "" || !keeps(finding.path)) return [];
    return violationOf(finding, rule) ?? [];
  });
}

/** A failure of `rule` as a whole: it ran on no file. */
function ruleFailure(rule: RerailRuleEntry | undefined, code: AuditFailure["code"], message: string, stage: AuditFailure["stage"]): AuditFailure {
  return { ...(rule === undefined ? {} : { ruleId: rule.id }), clauseId: RUN_CLAUSE, code, message, stage };
}

/** The failures a completed run reported: each policy it could not run (named by its file), with the engine's reason. */
export function rrRunFailures(failures: readonly { policy: string | null; message: string }[], rules: readonly RerailRuleEntry[]): AuditFailure[] {
  const byPath = new Map(rules.map(rule => [rule.path, rule]));
  return failures.map(failure =>
    ruleFailure(failure.policy === null ? undefined : byPath.get(failure.policy), "adapter-error", `The .rerail engine could not run this rule: ${failure.message}`, "evaluate"));
}

/** Why a run that was killed past a budget gave no findings, and how to raise that budget. */
function killedMessage(reason: "deadline" | "memory", deadlineMs: number): string {
  const budgets = rrBudgets(deadlineMs);
  if (reason === "deadline")
    return `The .rerail run was stopped: it ran past its deadline of ${budgets.deadlineMs} ms. Set RERAIL_RR_DEADLINE_MS (milliseconds) in the daemon's environment to raise it.`;
  return `The .rerail run was stopped: its memory passed ${Math.round(budgets.memoryBytes / 1024 ** 2)} MiB. Set RERAIL_RR_MEMORY_MB in the daemon's environment to raise it.`;
}

/**
 * A failure of each rule of `rules` for a run that gave no findings. A run killed past its deadline (`deadlineMs`, the
 * one it was given) or its memory leaves its rules out of work: a coverage gap of each, which fails no audit by itself,
 * as a native rule out of its own work is. A run that failed fails them. A run stopped on purpose (`aborted`) is no
 * failure of its rules: none.
 */
export function rrStoppedFailures(
  outcome: Exclude<RrRunOutcome, { status: "completed" }>,
  rules: readonly RerailRuleEntry[],
  deadlineMs: number,
): AuditFailure[] {
  if (outcome.status === "failed")
    return rules.map(rule => ruleFailure(rule, "adapter-error", `The .rerail run failed: ${outcome.message}`, "scan"));
  if (outcome.reason === "aborted") return [];
  const message = killedMessage(outcome.reason, deadlineMs);
  return rules.map(rule => ruleFailure(rule, "budget-exceeded", message, "evaluate"));
}
