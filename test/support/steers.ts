import { steerBlock, type SteerRule } from "../../src/engine/index.js";

/** An edit's steer for one finding of `ruleId`, as the code-rule enforcer renders it. */
export function editSteer(ruleId: string, message: string, location?: string): string {
  return steerBlock({ kind: "edit" }, [{ ruleId, mode: "steer", message, location }]);
}

/** The rules a delivery names for `ruleIds`, each steered, and linked to its page on `dashboard` when one is given. */
export function steeredRules(ruleIds: readonly string[], dashboard?: string): SteerRule[] {
  return ruleIds.map(id => ({
    id,
    state: "steered",
    ...(dashboard === undefined ? {} : { url: `${dashboard}#rule/${encodeURIComponent(id)}` }),
  }));
}
