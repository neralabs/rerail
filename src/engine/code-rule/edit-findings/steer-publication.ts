/**
 * How a checkout's findings go to the session as a steer: the steerable ones, with their paths as the session sees
 * them, in a background steer the runtime delivers at the session's next boundary. The native evaluation's findings and
 * the `.rerail` runs' go out the same way.
 */
import type { AuditRuleProvenance } from "../../audit/audit.js";
import { steers } from "../../catalog/rule-mode.js";
import { checkoutPathBoundary, sensitiveByName } from "../../path-boundary.js";
import { compareDeterministicStrings } from "../../stable-json.js";
import { buildSteer, type CodeRuleViolation, type RuleSteerDelivery, type SteerSourceOf } from "../code-rule.js";
import type { SteerShell, SteerTrigger } from "../rerail-steer.js";

/** Where a checkout's steers go: the session, how its paths are shown there, and the shell its commands are written for. */
export interface SteerTarget {
  readonly sessionId: string;
  readonly delivery: RuleSteerDelivery;
  /** The checkout the findings' paths are relative to. */
  readonly workspaceRoot: string;
  readonly presentPath: (path: string) => string;
  readonly steerShell: SteerShell | undefined;
}

/** One steer's findings and what they speak for. */
export interface SteerRequest {
  readonly revision: number;
  /** The sources (relative to the checkout) whose newer revisions make the steer stale; none for a repository audit's. */
  readonly sourceScope: readonly string[];
  readonly violations: readonly CodeRuleViolation[];
  readonly trigger: SteerTrigger;
  readonly sourceOf: SteerSourceOf;
  /** Whether the findings are of the session's generated edits (`SteerMetadata.generatedEdit`). */
  readonly generatedEdit: boolean;
}

/**
 * Where each rule of `provenance` was added from, for a steer to name, with the title `titleOf` gives it (its document's
 * own; none, and `steerTitle` takes its pack's).
 */
export function steerSourceOf(
  provenance: readonly AuditRuleProvenance[],
  titleOf: (ruleId: string) => string | undefined = () => undefined,
): SteerSourceOf {
  return ruleId => {
    const recorded = provenance.find(entry => entry.ruleId === ruleId);
    if (recorded === undefined || !("source" in recorded)) return undefined;
    return { ...recorded.source, title: titleOf(ruleId) };
  };
}

/** The paths, as the session sees them, of the steering findings in files that hold secrets or link to one. */
function secretPaths(target: SteerTarget, violations: readonly CodeRuleViolation[]): Set<string> {
  // Paths are judged in the workspace they are relative to, so a file that links to a secret is judged as one.
  const boundary = checkoutPathBoundary(target.workspaceRoot);
  return new Set(violations.flatMap(({ path, mode }) =>
    path !== undefined && steers(mode) && boundary.sensitive(path) ? [target.presentPath(path)] : []));
}

/** Sends `request`'s steering findings to the session as one background steer; nothing when none steers. */
export async function deliverSteer(target: SteerTarget, request: SteerRequest): Promise<void> {
  const steerable = request.violations
    .filter(violation => steers(violation.mode))
    .map(violation => (violation.path === undefined ? violation : { ...violation, path: target.presentPath(violation.path) }));
  if (steerable.length === 0) return;
  const scope = request.sourceScope.map(path => target.presentPath(path)).sort(compareDeterministicStrings);
  const secret = secretPaths(target, request.violations);
  const steer = buildSteer(steerable, request.trigger, request.sourceOf, path => secret.has(path) || sensitiveByName(path), target.steerShell);
  await target.delivery.requestSteer(target.sessionId, "steer", steer.content, {
    revision: request.revision,
    scope,
    ruleIds: steer.ruleIds,
    rules: steer.rules,
    background: true,
    ...(request.generatedEdit ? { generatedEdit: { violations: steerable, sourceOf: request.sourceOf } } : {}),
  });
}
