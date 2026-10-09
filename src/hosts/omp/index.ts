/**
 * The OMP extension.
 *
 * The public surface of this layer: what code outside `src/` imports from it (`scripts/check/check-boundaries.mjs`).
 */
export {
  createConversationRulesExtension,
  MATCH_ENTRY_TYPE,
  MUTATION_ENTRY_TYPE,
  TURN_ENTRY_TYPE,
} from "./extension.js";
