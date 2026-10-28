import { chmodSync, mkdirSync, readFileSync, renameSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";

/** In the Rerail home: the person's answers about what Rerail may do with what it records. */
const CONSENT_FILE = "consent.json";

/** The person's standing answer about sending feedback notes; with none, Rerail asks each time. */
export const FEEDBACK_SHARING_ANSWERS = ["always", "never"] as const;
export type FeedbackSharingAnswer = (typeof FEEDBACK_SHARING_ANSWERS)[number];

export interface Consent {
  /** That Rerail may read their coding agent sessions (T2b). */
  sessions?: { approvedAt: string };
  /** Whether Rerail sends the feedback notes coding agents record (`rerail feedback`), without asking each time. */
  feedback?: { sharing: FeedbackSharingAnswer; answeredAt: string };
}

/** The answers recorded in `home`; a missing or unreadable file, or an answer in no known shape, is no answer. */
export function readConsent(home: string): Consent {
  let raw: unknown;
  try {
    raw = JSON.parse(readFileSync(join(home, CONSENT_FILE), "utf8"));
  } catch {
    return {};
  }
  if (typeof raw !== "object" || raw === null) return {};
  const { sessions, feedback } = raw as { sessions?: { approvedAt?: unknown }; feedback?: { sharing?: unknown; answeredAt?: unknown } };
  const consent: Consent = {};
  if (typeof sessions?.approvedAt === "string") consent.sessions = { approvedAt: sessions.approvedAt };
  if (FEEDBACK_SHARING_ANSWERS.includes(feedback?.sharing as FeedbackSharingAnswer) && typeof feedback?.answeredAt === "string") {
    consent.feedback = { sharing: feedback.sharing as FeedbackSharingAnswer, answeredAt: feedback.answeredAt };
  }
  return consent;
}

/**
 * Record `change` of the answers in `home`, keeping the others; with no answer left, the file goes. The file is
 * written whole then renamed over the old one, so a crash leaves the old answers, never half a file, and it is
 * readable by its owner only whatever mode an earlier copy had.
 */
export function updateConsent(home: string, change: (consent: Consent) => Consent): void {
  const path = join(home, CONSENT_FILE);
  const next = change(readConsent(home));
  if (next.sessions === undefined && next.feedback === undefined) {
    rmSync(path, { force: true });
    return;
  }
  mkdirSync(home, { recursive: true, mode: 0o700 });
  const staging = `${path}.${process.pid}.tmp`;
  writeFileSync(staging, `${JSON.stringify(next, null, 2)}\n`, { mode: 0o600 });
  chmodSync(staging, 0o600);
  renameSync(staging, path);
}
