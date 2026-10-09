import { renderNodeTooOld, type SetupLane } from "./setup-screens.js";

/** The oldest Node major Rerail runs on; matches `engines.node` in package.json. */
export const REQUIRED_NODE_MAJOR = 24;

export type NodeVersionCheck =
  | { readonly ok: true; readonly major: number }
  | { readonly ok: false; readonly found: string; readonly requiredMajor: number; readonly message: string };

/** `24`, `v24.1.0`, `24.0.0-nightly20250101abc`; the major is the leading integer. */
const NODE_VERSION = /^v?(\d+)(?:\.\d+){0,2}(?:[-+][0-9A-Za-z.-]*)?$/;

/**
 * The explicit check npm's `engines` only warns about. Pass
 * `process.versions.node` (or `process.version`). A version that cannot be
 * read fails the check: set-up stops rather than guessing.
 */
export function checkNodeVersion(found: string, lane: SetupLane, requiredMajor: number = REQUIRED_NODE_MAJOR): NodeVersionCheck {
  const version = found.trim();
  const majorText = NODE_VERSION.exec(version)?.[1];
  const major = majorText === undefined ? Number.NaN : Number.parseInt(majorText, 10);
  if (major >= requiredMajor) return { ok: true, major };
  const shown = version.replace(/^v/, "") || "unknown";
  return { ok: false, found: shown, requiredMajor, message: renderNodeTooOld({ found: shown, requiredMajor }, lane) };
}
