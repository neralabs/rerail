/**
 * Deriving pack test data and bumping upstream pins are paused (docs/upstream.md, "Derive and bumps are paused"): each
 * UPSTREAM.md records its repository per repo now, while the generators still write per-file rows. `fixtures:derive`
 * and `upstream:bump` refuse with this message until the generators write per-repo provenance.
 */
export const DERIVE_PAUSED =
  "derive and bumps are paused until the generators write per-repo provenance; see docs/upstream.md#derive-and-bumps-are-paused";

/** A refusal while derive and bumps are paused: its message is the whole report, with no stack. */
export class DerivePausedError extends Error {
  constructor(label: string) {
    super(`${label}: ${DERIVE_PAUSED}`);
    this.name = "DerivePausedError";
  }
}
