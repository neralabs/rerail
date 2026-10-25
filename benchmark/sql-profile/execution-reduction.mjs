// Historical delivery deltas remain diagnostic; they do not establish performance acceptance.
import { executionBudget } from './execution-budget.mjs';

export function executionReduction(report, { baseline, removal }) {
  if (!Number.isSafeInteger(baseline) || !Number.isSafeInteger(removal) || removal <= 0 || baseline < removal)
    throw new Error('A delivery requires a positive, explicit complete-job removal target.');
  const budget = executionBudget(report);
  const removed = baseline - budget.observed;
  return { baseline, requiredRemoval: removal, removed, observed: budget.observed,
    ceiling: baseline - removal, accepted: budget.problems.length === 0 && removed >= removal,
    problems: budget.problems, finalLimit: budget.limit, withinReference: budget.observed <= budget.limit,
    diagnosticOnly: true, performanceAccepted: null };
}
