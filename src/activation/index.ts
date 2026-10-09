/**
 * Signed customer profiles, activation, and the device secret store.
 *
 * The public surface of this layer: what code outside `src/` imports from it (`scripts/check/check-boundaries.mjs`).
 */
export {
  type ActivationTrustRoots,
  type CustomerProfile,
  type FlattenedJws,
  type SignedActivationRequest,
} from "./customer-profile.js";
