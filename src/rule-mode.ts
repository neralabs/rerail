/**
 * What Rerail does when a rule fires, for every rule kind: `log` records it for review only,
 * `steer` sends the rule's guidance to the agent, and `block` stops the action. Until block
 * enforcement lands, the runtime delivers a `block` rule exactly like a `steer` rule.
 */
export const RULE_MODES = ["log", "steer", "block"] as const;
export type RuleMode = (typeof RULE_MODES)[number];

/** Whether a rule in this mode sends guidance to the agent. */
export function steers(mode: RuleMode): boolean {
  return mode !== "log";
}
