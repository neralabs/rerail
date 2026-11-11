/**
 * Signed customer profiles, activation, and the device secret store.
 *
 * The public surface of this layer: what code outside `src/` imports from it (`scripts/check/check-boundaries.mjs`).
 */
export {
  ACTIVE_PROFILE_FILE,
  type ActivationTrustRoots,
  type CustomerProfile,
  type FlattenedJws,
  PROFILES_DIR,
  type SignedActivationRequest,
} from "./customer-profile.js";
export { DEVICE_SECRETS_DIR } from "./device-keystore.js";
