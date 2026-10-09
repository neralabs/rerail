import type { SubCheckId } from "../checks.js";

/** One sub-check's verdict from a gate: the id it is filed under (conformance/checks.ts), and why. */
export interface GateCheck {
  id: SubCheckId;
  ok: boolean;
  detail: string;
}
