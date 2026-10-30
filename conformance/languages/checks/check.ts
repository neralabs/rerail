/** One deterministic check. `detail` says what was compared, so a failure explains itself. */
export interface Check {
  id: string;
  ok: boolean;
  detail: string;
}
