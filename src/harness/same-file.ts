import { realpathSync } from "node:fs";

/**
 * Whether two paths name one file once links are followed; false when either is missing. The hook launcher runs a hook
 * in its own process when this finds the node that started it to be the recorded one, and set-up writes `node` as a
 * Windows hook's command only where this finds PATH's node to be the recorded one, so both decide by the same test.
 */
export function sameFile(left: string, right: string): boolean {
  try {
    return realpathSync.native(left) === realpathSync.native(right);
  } catch {
    return false;
  }
}
