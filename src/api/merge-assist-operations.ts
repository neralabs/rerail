/**
 * Merge Assist's operations (docs/merge-assist-v1-build.md): the checkout's state and in-flight conflicts, a scan,
 * the settings, and dismissing a conflict. Each acts on one checkout, which the target names as every repository
 * operation's does. Changing the settings (on or off, the hook command) and dismissing a conflict are
 * people's; a coding agent may read and scan, in its own checkout only. The people-only guarantee holds for this
 * surface; `rerail merge-assist` runs as the person, so an agent with a shell can run it as it can any Rerail command.
 * Internal testing: every operation here.
 */
import type { z } from "zod";
import { internalTesting } from "../engine/internal-testing.js";
import type { ApiCaller } from "../engine/catalog/repository-model.js";
import { DEFAULT_HOOK_TIMEOUT_SECONDS, type MergeAssistStatus } from "../engine/merge-assist/model.js";
import { MergeAssistError, type MergeAssistErrorCode, type MergeAssistService, type MergeAssistSettingsChange } from "../runtime/merge-assist/service.js";
import {
  mergeAssistDismissBodySchema,
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
 * A coding agent's change stays in its own checkout and the checkouts inside it: a scan changes what the sessions there
 * are told.
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

/** Turn Merge Assist on or off, then change the hook, answering the status after the last change. */
async function applySettings(service: MergeAssistService, checkout: string, body: SettingsBody): Promise<MergeAssistStatus> {
  const status = body.enabled === undefined ? null : await service.setEnabled(checkout, body.enabled);
  if (status !== null && body.hook === undefined) return status;
  return await service.updateSettings(checkout, hookChange(body.hook));
}

export const mergeAssistOperations: readonly Operation[] = [
  defineOperation({
    id: "merge_assist_get",
    method: "GET",
    path: "/api/merge-assist",
    summary: internalTesting("Show Merge Assist for one checkout: whether it is on, the gh it uses, the settings, the last scan, and its in-flight conflicts with live branches and open pull requests (each with its summary, and whether it is quiet). Reads only. Without a target an MCP call uses the calling agent's checkout."),
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
    summary: internalTesting("Find the checkout's in-flight conflicts now: overlapping changes with other local and remote-tracking branches and open pull requests. Runs the person's gh (read-only) to list pull requests. A coding agent may scan only its own checkout."),
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
    summary: internalTesting("Show the person's Merge Assist settings for the checkout's repository: the conflict hook command (null for none). Reads only."),
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
    summary: internalTesting("Change the person's Merge Assist settings for the checkout's repository: turn it on or off (enabled; off clears the conflicts it found) and the conflict hook command (hook: an argv run without a shell on each conflict event, timeoutSeconds 1-60, default 10; null removes it). Name at least one. People only: it runs the person's gh, and the hook command as them."),
    body: mergeAssistSettingsBodySchema,
    output: mergeAssistStatusSchema,
    handler: async (stores, input, caller) => {
      peopleOnly(caller, "only a person changes Merge Assist's settings, on the dashboard's Merge Assist page or with rerail merge-assist on|off|settings");
      const checkout = checkoutOf(stores, input, caller, true);
      return await answer(() => applySettings(stores.mergeAssist(), checkout, input));
    },
  }),
  defineOperation({
    id: "merge_assist_conflict_dismiss",
    method: "POST",
    path: "/api/merge-assist/conflicts/dismiss",
    summary: internalTesting("Dismiss an in-flight conflict with another branch as not a conflict: it stays dismissed for its question and that branch, and the conflict hook hears closed. Only a person dismisses a conflict: a coding agent reads them with merge_assist_get and is refused here (invalid-state)."),
    body: mergeAssistDismissBodySchema,
    output: mergeAssistStatusSchema,
    handler: async (stores, input, caller) => {
      const checkout = checkoutOf(stores, input, caller, true);
      return await answer(() => stores.mergeAssist().dismissConflict(checkout, input.conflictId, caller.kind));
    },
  }),
];
