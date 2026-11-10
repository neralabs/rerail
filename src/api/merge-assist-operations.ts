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
import type { z } from "zod";
import { internalTesting } from "../engine/internal-testing.js";
import type { ApiCaller } from "../engine/catalog/repository-model.js";
import { DEFAULT_HOOK_TIMEOUT_SECONDS, type MergeAssistStatus } from "../engine/merge-assist/model.js";
import { MergeAssistError, type MergeAssistErrorCode, type MergeAssistService, type MergeAssistSettingsChange } from "../runtime/merge-assist/service.js";
import {
  mergeAssistCandidateBodySchema,
  mergeAssistConflictBodySchema,
  mergeAssistFindingBodySchema,
  mergeAssistSettingsBodySchema,
  mergeAssistSettingsSchema,
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
async function answer<T>(run: () => Promise<T>): Promise<T> {
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

type SettingsBody = z.infer<typeof mergeAssistSettingsBodySchema>;

/** The hook the body names, its timeout defaulting to {@link DEFAULT_HOOK_TIMEOUT_SECONDS}; nothing when it names none. */
function hookChange(hook: SettingsBody["hook"]): Pick<MergeAssistSettingsChange, "hook"> {
  if (hook === undefined) return {};
  return { hook: hook === null ? null : { command: hook.command, timeoutSeconds: hook.timeoutSeconds ?? DEFAULT_HOOK_TIMEOUT_SECONDS } };
}

/** Turn Merge Assist on or off, then change the delivery and the hook, answering the status after the last change. */
async function applySettings(service: MergeAssistService, checkout: string, body: SettingsBody): Promise<MergeAssistStatus> {
  const status = body.enabled === undefined ? null : await service.setEnabled(checkout, body.enabled);
  if (status !== null && body.delivery === undefined && body.hook === undefined) return status;
  return await service.updateSettings(checkout, { ...(body.delivery === undefined ? {} : { delivery: body.delivery }), ...hookChange(body.hook) });
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
    id: "merge_assist_settings_get",
    method: "GET",
    path: "/api/merge-assist/settings",
    summary: internalTesting("Show the person's Merge Assist settings for the checkout's repository: where conflicts reach them (delivery: chat, terminal or both) and the conflict hook command (null for none). Reads only."),
    query: mergeAssistTargetSchema,
    output: mergeAssistSettingsSchema,
    handler: async (stores, input, caller) => {
      const checkout = checkoutOf(stores, input, caller, false);
      return await answer(() => stores.mergeAssist().settings(checkout));
    },
  }),
  defineOperation({
    id: "merge_assist_settings_set",
    method: "PUT",
    path: "/api/merge-assist/settings",
    summary: internalTesting("Change the person's Merge Assist settings for the checkout's repository: turn it on or off (enabled; off removes every decision it applied), where conflicts reach them (delivery: chat, terminal or both), and the conflict hook command (hook: an argv run without a shell on each conflict event, timeoutSeconds 1-60, default 10; null removes it). Name at least one. People only: it lets other branches' decisions steer the person's coding agent, runs their agent CLI and gh, and runs the hook command as them."),
    body: mergeAssistSettingsBodySchema,
    output: mergeAssistStatusSchema,
    handler: async (stores, input, caller) => {
      peopleOnly(caller, "only a person changes Merge Assist's settings, on the dashboard's Merge Assist page or with rerail merge-assist on|off|settings");
      const checkout = checkoutOf(stores, input, caller, true);
      return await answer(() => applySettings(stores.mergeAssist(), checkout, input));
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
    summary: internalTesting("Settle an in-flight conflict with another branch: dismiss it as not a conflict. Experimental, with RERAIL_EXPERIMENTAL_CONFLICT_VERDICTS=1 where Merge Assist runs (refused with invalid-state otherwise): keep this branch's side (mine), take theirs, or write a new decision (new, with text); a verdict writes the decision this branch follows into its working tree, for the person to commit, and it steers every agent from then on. Only a person settles a conflict: a coding agent reads them with merge_assist_get and is refused here (invalid-state)."),
    body: mergeAssistConflictBodySchema,
    output: mergeAssistStatusSchema,
    handler: async (stores, input, caller) => {
      const checkout = checkoutOf(stores, input, caller, true);
      return await answer(() => stores.mergeAssist().resolveConflict(checkout, input.conflictId, input.action, input.text, caller.kind));
    },
  }),
];
