/**
 * Merge Assist's operations (docs/merge-assist-v1-build.md §5.6): the checkout's state, decisions and findings, a scan of
 * other branches, and the changes a person makes on the dashboard's Merge Assist page. Each acts on one checkout, which
 * the target names as every repository operation's does, and answers with that checkout's status. Turning Merge Assist
 * on or off, adopting, undoing and restoring are people's: each widens what steers a coding agent. A coding agent may
 * scan, dismiss a finding that is still open (dismissing an applied one would undo it) and reject one of its branch's
 * decisions, in its own checkout only. The people-only guarantee holds for this surface; `rerail merge-assist` runs as
 * the person, so an agent with a shell can run it as it can any Rerail command.
 * Internal testing: every operation here.
 */
import { internalTesting } from "../engine/internal-testing.js";
import type { ApiCaller } from "../engine/catalog/repository-model.js";
import type { MergeAssistStatus } from "../engine/merge-assist/model.js";
import { MergeAssistError, type MergeAssistErrorCode } from "../runtime/merge-assist/service.js";
import {
  mergeAssistCandidateBodySchema,
  mergeAssistConflictBodySchema,
  mergeAssistFindingBodySchema,
  mergeAssistSettingsBodySchema,
  mergeAssistStatusSchema,
  mergeAssistTargetSchema,
} from "./merge-assist-contracts.js";
import { ApiError, defineOperation, type Operation, type RerailApiStores } from "./operation.js";
import { checkoutOnDisk, resolveTarget, within } from "./repository-operations.js";

const ERROR_STATUS: Record<MergeAssistErrorCode, number> = {
  "not-a-checkout": 404,
  "not-found": 404,
  "invalid-state": 409,
  disabled: 409,
};

/** The service's refusals as API errors, under the service's own code. */
async function answer(run: () => Promise<MergeAssistStatus>): Promise<MergeAssistStatus> {
  try {
    return await run();
  } catch (error) {
    if (error instanceof MergeAssistError) throw new ApiError(ERROR_STATUS[error.code], error.code, error.message);
    throw error;
  }
}

/**
 * The checkout on disk the target names: `checkout`, else the repository's main checkout, else the calling agent's.
 * A coding agent's change stays in its own checkout and the checkouts inside it: a scan or a dismissal changes what
 * steers the sessions there.
 */
function checkoutOf(stores: RerailApiStores, input: { repository?: string | undefined; checkout?: string | undefined }, caller: ApiCaller, write: boolean): string {
  const checkout = checkoutOnDisk(resolveTarget(stores, input, caller, write));
  if (write && caller.kind === "agent" && (caller.checkout === undefined || within(caller.checkout, checkout) === undefined)) {
    throw new ApiError(403, "agent-not-permitted", "a coding agent changes Merge Assist only in its own checkout and the checkouts inside it");
  }
  return checkout;
}

function peopleOnly(caller: ApiCaller, message: string): void {
  if (caller.kind === "agent") throw new ApiError(403, "agent-not-permitted", message);
}

export const mergeAssistOperations: readonly Operation[] = [
  defineOperation({
    id: "merge_assist_get",
    method: "GET",
    path: "/api/merge-assist",
    summary: internalTesting("Show Merge Assist for one checkout: whether it is on, the agent CLI and gh it uses, this branch's own decisions, and what the last scan found on other branches (decided differently, needs review, added). Reads only. Without a target an MCP call uses the calling agent's checkout."),
    query: mergeAssistTargetSchema,
    output: mergeAssistStatusSchema,
    handler: async (stores, input, caller) => {
      const checkout = checkoutOf(stores, input, caller, false);
      return await answer(() => stores.mergeAssist().status(checkout));
    },
  }),
  defineOperation({
    id: "merge_assist_scan",
    method: "POST",
    path: "/api/merge-assist/scan",
    summary: internalTesting("Read the decisions on other local branches and open pull requests, sort them into decided differently, needs review and added, and apply the added ones to this checkout. Runs the person's gh (read-only) and agent CLI. A coding agent may scan only its own checkout."),
    body: mergeAssistTargetSchema,
    output: mergeAssistStatusSchema,
    handler: async (stores, input, caller) => {
      const checkout = checkoutOf(stores, input, caller, true);
      return await answer(() => stores.mergeAssist().scan(checkout));
    },
  }),
  defineOperation({
    id: "merge_assist_settings_set",
    method: "PUT",
    path: "/api/merge-assist/settings",
    summary: internalTesting("Turn Merge Assist on or off for the checkout's repository. Off removes every decision it applied. People only: it lets other branches' decisions steer the person's coding agent, and runs their agent CLI and gh."),
    body: mergeAssistSettingsBodySchema,
    output: mergeAssistStatusSchema,
    handler: async (stores, input, caller) => {
      peopleOnly(caller, "only a person turns Merge Assist on or off, on the dashboard's Merge Assist page or with rerail merge-assist on|off");
      const checkout = checkoutOf(stores, input, caller, true);
      return await answer(() => stores.mergeAssist().setEnabled(checkout, input.enabled));
    },
  }),
  defineOperation({
    id: "merge_assist_candidate_set",
    method: "PUT",
    path: "/api/merge-assist/candidates",
    summary: internalTesting("Reject one of this branch's own decisions (Not a rule), or restore it. The decision's file keeps the new state, so a teammate who absorbed it sees the change. A coding agent may reject one in its own checkout when the person asks; only a person restores one."),
    body: mergeAssistCandidateBodySchema,
    output: mergeAssistStatusSchema,
    handler: async (stores, input, caller) => {
      if (input.state === "active") peopleOnly(caller, "only a person restores a decision, on the dashboard's Merge Assist page; a coding agent may only reject one");
      const checkout = checkoutOf(stores, input, caller, true);
      return await answer(() => stores.mergeAssist().setCandidateState(checkout, input.candidateId, input.state));
    },
  }),
  defineOperation({
    id: "merge_assist_finding_set",
    method: "PUT",
    path: "/api/merge-assist/findings",
    summary: internalTesting("Adopt a decision from another branch that needs review, dismiss a finding until its decision changes, or undo one that was added or adopted. A coding agent may only dismiss a finding that is still open, in its own checkout; adopting and undoing are the person's, and dismissing an applied finding would undo it."),
    body: mergeAssistFindingBodySchema,
    output: mergeAssistStatusSchema,
    handler: async (stores, input, caller) => {
      if (input.action !== "dismiss") {
        peopleOnly(caller, `only a person ${input.action === "adopt" ? "adopts" : "undoes"} a decision from another branch, on the dashboard's Merge Assist page or with rerail merge-assist ${input.action}; a coding agent may only dismiss a finding`);
      }
      const checkout = checkoutOf(stores, input, caller, true);
      return await answer(() => stores.mergeAssist().resolveFinding(checkout, input.findingId, input.action, caller.kind));
    },
  }),
  defineOperation({
    id: "merge_assist_conflict_set",
    method: "PUT",
    path: "/api/merge-assist/conflicts",
    summary: internalTesting("Resolve an in-flight conflict with another branch: keep this branch's side (mine), take theirs, write a new decision (new, with text), or dismiss it as not a conflict. A verdict appends to the decision log (refs/rerail/decisions) and writes a decision on this branch that steers every agent from then on. Only a person resolves a conflict: a coding agent reads them with merge_assist_get and is refused here (invalid-state)."),
    body: mergeAssistConflictBodySchema,
    output: mergeAssistStatusSchema,
    handler: async (stores, input, caller) => {
      const checkout = checkoutOf(stores, input, caller, true);
      return await answer(() => stores.mergeAssist().resolveConflict(checkout, input.conflictId, input.action, input.text, caller.kind));
    },
  }),
];
