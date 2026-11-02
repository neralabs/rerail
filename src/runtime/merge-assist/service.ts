/**
 * Merge Assist's facade: what the runtime, the API and the CLI call for one checkout. See docs/merge-assist-prd.md.
 */
import type { CandidateState } from "../../engine/merge-assist/candidate.js";
import type { FindingAction, MergeAssistStatus } from "../../engine/merge-assist/model.js";

export const MERGE_ASSIST_ERROR_CODES = ["not-a-checkout", "not-found", "invalid-state", "disabled"] as const;
export type MergeAssistErrorCode = (typeof MERGE_ASSIST_ERROR_CODES)[number];

/** A request Merge Assist refuses; `code` says which, the message says why for a person. */
export class MergeAssistError extends Error {
  constructor(readonly code: MergeAssistErrorCode, message: string) {
    super(message);
    this.name = "MergeAssistError";
  }
}

export interface MergeAssistService {
  /** The checkout's state, decisions and findings. Reads only; runs nothing. */
  status(checkoutPath: string): Promise<MergeAssistStatus>;
  /** Turn Merge Assist on or off for the checkout's repository. Off removes every candidate it applied. */
  setEnabled(checkoutPath: string, enabled: boolean): Promise<MergeAssistStatus>;
  /** Read other branches and open pull requests, sort their candidates into tiers, and apply the absorbed ones. */
  scan(checkoutPath: string): Promise<MergeAssistStatus>;
  /** Reject or restore one of this branch's own candidates (rewrites its file's `state`). */
  setCandidateState(checkoutPath: string, candidateId: string, state: CandidateState): Promise<MergeAssistStatus>;
  /** Adopt a review finding, dismiss any finding, or undo an applied one. */
  resolveFinding(checkoutPath: string, findingId: string, action: FindingAction): Promise<MergeAssistStatus>;
}
