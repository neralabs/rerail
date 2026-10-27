/** One deterministic check. `detail` says what was compared, so a failure explains itself. */
export interface Check {
  id: string;
  ok: boolean;
  detail: string;
  /**
   * The check passed with no evidence behind it: it is reported (the summary's and the comment's "Passing without
   * evidence"), never failed.
   */
  vacuous?: true;
}
