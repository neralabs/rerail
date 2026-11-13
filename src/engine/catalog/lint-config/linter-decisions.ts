/**
 * One linter's decisions for the several upstream names a port honours (two ESLint rules, a Ruff code and its alias) as
 * the one decision that linter's config makes for the port, settling in the order verdicts across linters settle
 * (`settleVerdicts`): an explicit off, an explicit on, an implicit on, unknown, an implicit off.
 */
import type { PortDecision } from "./linter-config.js";

type Decided = Extract<PortDecision, { on: boolean }>;

/** Whether `decision` is in tier `tier`. */
const inTier = (decision: PortDecision, tier: (typeof TIERS)[number]): boolean =>
  tier === "unknown" ? decision.on === "unknown" : decision.on === tier.on && (decision as Decided).explicit === tier.explicit;

const TIERS = [
  { on: false, explicit: true },
  { on: true, explicit: true },
  { on: true, explicit: false },
  "unknown",
  { on: false, explicit: false },
] as const;

/** The decision `decisions` come to, their reasons joined; undefined when there are none. */
export function combineDecisions(decisions: readonly PortDecision[]): PortDecision | undefined {
  for (const tier of TIERS) {
    const found = decisions.filter(decision => inTier(decision, tier));
    if (found.length === 0) continue;
    const because = [...new Set(found.map(decision => decision.because))].join("; ");
    if (tier === "unknown") return { on: "unknown", because };
    const config = (found as Decided[]).find(decision => decision.config !== undefined)?.config;
    return { on: tier.on, explicit: tier.explicit, because, ...(config === undefined ? {} : { config }) };
  }
  return undefined;
}
