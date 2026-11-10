/**
 * Merge Assist's experimental conflict handlers (docs/merge-assist-v1-build.md §12), registered on the conflict hook only
 * while `RERAIL_EXPERIMENTAL_CONFLICT_VERDICTS=1` in the process that runs Merge Assist (`experimental-features.ts`):
 *
 * - **verdicts**: a person resolves a conflict with keep mine, take theirs or a new decision, and the cards offer those
 *   answers;
 * - **chat answers**: the person's prompt answers a card the session was handed, "later" included.
 *
 * Neither does anything on the hook's events: each takes part through its {@link ConflictExperiment} stages.
 */
import { baseBranch } from "../../engine/merge-assist/branch-git.js";
import { parseChatAnswer, type ChatAnswer } from "../../engine/merge-assist/chat-answer.js";
import type { ChatInput, ConflictHandler } from "./conflict-hook.js";
import { isSnoozed, snoozed } from "./conflict-level.js";
import { decideConflict } from "./conflict-resolve.js";
import { readConflicts, replaceConflict } from "./conflict-store.js";
import { cardsShown } from "./notes.js";

/** Verdicts: keep mine, take theirs and new decision settle a conflict; the cards offer them. */
const verdictsHandler: ConflictHandler = {
  name: "verdicts",
  on: async () => undefined,
  experiment: {
    verdictCards: true,
    resolve: async ({ database, checkout, now, stored, action, text, name }) => await decideConflict({ database, checkout, now }, stored, { action, text, name }),
  },
};

/**
 * The person's prompt, when it plainly answers a checkpoint or stop card that session was handed (`parseChatAnswer`):
 * a verdict, recorded as theirs by the same path as the dashboard, or "later", which snoozes the conflict.
 */
async function answerFromChat({ database, checkout, now, prompt, sessionId, resolve }: ChatInput): Promise<ChatAnswer> {
  const root = checkout.checkoutPath;
  // Only the session the card was handed to answers it: a prompt in any other (a headless `claude -p` an agent runs
  // from its shell, say) is not the person answering.
  const shown = cardsShown(database, sessionId);
  const open = readConflicts(database, root).map(entry => entry.conflict)
    .filter(conflict => conflict.status === "open" && (conflict.level === "checkpoint" || conflict.level === "stop") && shown(conflict))
    .map(conflict => ({ id: conflict.id, shortId: conflict.shortId, title: conflict.title, snoozed: isSnoozed(conflict) }));
  const answer = parseChatAnswer(prompt, open);
  if (answer.kind !== "answer") return answer;
  if (answer.option !== "later") {
    await resolve(answer.conflictId, answer.option, answer.text ?? undefined);
    return answer;
  }
  const base = baseBranch(root)?.ref ?? null;
  database.transaction(tx => {
    const stored = readConflicts(tx, root).find(entry => entry.conflict.id === answer.conflictId);
    if (stored === undefined || stored.conflict.status !== "open" || isSnoozed(stored.conflict)) return;
    replaceConflict(tx, root, { ...stored, conflict: snoozed(stored.conflict, root, base, now()) }, now());
  }, { behavior: "immediate" });
  return answer;
}

/** Chat answers: the person answers a card in the session it was handed to. */
const chatAnswersHandler: ConflictHandler = {
  name: "chat-answers",
  on: async () => undefined,
  experiment: { answer: answerFromChat },
};

/** The experimental handlers for the flag, in the order they hear events: verdicts, then chat answers. */
export function experimentalHandlers(conflictVerdicts: boolean): ConflictHandler[] {
  return conflictVerdicts ? [verdictsHandler, chatAnswersHandler] : [];
}
