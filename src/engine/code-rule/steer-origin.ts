import type { RuleSourceKind } from "../catalog/repository-model.js";
import { loadRulePacks, RERAIL_PUBLISHER, rulePackIdOf } from "../catalog/rule-packs.js";

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
 * when this process has not loaded that pack) and, for a pack Rerail does not publish, who does; or the library it
 * was added from. Without a recorded source the pack is looked up by rule id, as it is for a pack's rule added on its
 * own; undefined when unknown.
 */
export function steerOrigin(ruleId: string, source?: SteerRuleSource): string | undefined {
  const packs = loadRulePacks();
  const packId = source === undefined || source.kind === "rerail_rule" ? rulePackIdOf(ruleId, packs) : source.packId;
  const pack = packs.find(found => found.id === packId);
  if (pack !== undefined && pack.publisher !== RERAIL_PUBLISHER) return `From the ${pack.name} rule pack by ${pack.publisher}, which Rerail has not reviewed.`;
  if (source?.kind === "library_rule") return "From your rule library.";
  if (source?.kind === "rerail_rule") return "From Rerail's own rules.";
  if (packId !== null) return `From the ${pack?.name ?? packId} rule pack.`;
  return undefined;
}

