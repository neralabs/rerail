import type { RuleSourceKind } from "../catalog/repository-model.js";
import { loadRulePacks, rulePackIdOf } from "../catalog/rule-packs.js";

/*
 * Kept apart from `rerail-steer.ts`: the rule pack catalog it reads pulls in modules the bundled, passive
 * `session-presentation.js` reader must not carry.
 */

/** Where a rule was added from, as a rule's provenance records it. */
export interface SteerRuleSource {
  readonly kind: RuleSourceKind;
  readonly packId: string | null;
}

/**
 * Where the rule comes from, in the words the coding agent can repeat when asked why: its rule pack's name (its id
 * when this process has not loaded that pack), or the library it was added from. Without a recorded source the pack
 * is looked up by rule id; undefined when unknown.
 */
export function steerOrigin(ruleId: string, source?: SteerRuleSource): string | undefined {
  const packs = loadRulePacks();
  const packId = source === undefined ? rulePackIdOf(ruleId, packs) : source.packId;
  if (packId !== null) return `From the ${packs.find(pack => pack.id === packId)?.name ?? packId} rule pack.`;
  if (source?.kind === "library_rule") return "From your rule library.";
  if (source?.kind === "rerail_rule") return "From Rerail's own rules.";
  return undefined;
}

