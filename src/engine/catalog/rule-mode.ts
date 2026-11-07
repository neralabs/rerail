/**
 * What Rerail does when a rule fires, for every rule kind: `log` records it for review only,
 * `steer` sends the rule's guidance to the agent, and `block` stops the action. Until block
 * enforcement lands, the runtime delivers a `block` rule exactly like a `steer` rule.
 * Internal testing: `block`. Rerail 0.1.0 releases `log` and `steer`.
 */
export const RULE_MODES = ["log", "steer", "block"] as const;
export type RuleMode = (typeof RULE_MODES)[number];

/** Whether a rule in this mode sends guidance to the agent. */
export function steers(mode: RuleMode): boolean {
  return mode !== "log";
}

/**
 * What Rerail says wherever a person or a coding agent puts a rule on `block` (a CLI or API write, an approval, a
 * listing), until block enforcement lands: the rule is stored as `block` and steers.
 */
export const BLOCK_STEERS_NOTE = "Block does not block in this release: a rule on Block steers instead, exactly like a rule on Steer.";
