import type { RuleSourceKind } from "../catalog/repository-model.js";
import { loadRulePacks, RERAIL_PUBLISHER, rulePackIdOf } from "../catalog/rule-packs.js";
import { WORKTREE_PACK_ID } from "../catalog/worktree-pack.js";

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
 * when this process has not loaded that pack) and, for a pack Rerail does not publish or one installed from a folder
 * or tarball, who does or where it came from; the checkout's own rules; or the library it was added from. Without a
 * recorded source the pack is looked up by rule id, as it is for a pack's rule added on its own; undefined when unknown.
 */
export function steerOrigin(ruleId: string, source?: SteerRuleSource): string | undefined {
  const packs = loadRulePacks();
  const packId = source === undefined || source.kind === "rerail_rule" ? rulePackIdOf(ruleId, packs) : source.packId;
  const pack = packs.find(found => found.id === packId);
  if (pack !== undefined && (pack.publisher !== RERAIL_PUBLISHER || pack.localSource !== undefined)) {
    const by = pack.publisher === RERAIL_PUBLISHER ? "" : ` by ${pack.publisher}`;
    const from = pack.localSource === undefined ? "" : `, installed from a ${pack.localSource.kind}`;
    return `From the ${pack.name} rule pack${by}${from}, which Rerail has not reviewed.`;
  }
  // Only the checkout's own catalog holds its rules: an installed pack that took the id would have been found above.
  if (pack === undefined && source?.kind === "pack" && packId === WORKTREE_PACK_ID) return "From this worktree's own rules.";
  if (source?.kind === "library_rule") return "From your rule library.";
  if (source?.kind === "rerail_rule") return "From Rerail's own rules.";
  if (packId !== null) return `From the ${pack?.name ?? packId} rule pack.`;
  return undefined;
}

