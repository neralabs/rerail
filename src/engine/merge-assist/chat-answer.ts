/**
 * A person's answer to a checkpoint or stop card, read from their own prompt (CONTRACT: Answers from chat). The answer
 * is the whole message: `keep mine`, `take theirs` or `later` (trailing punctuation allowed), or a message that starts
 * with `new decision: <text>`. The conflict's name, `#a1b2c3` (its short id) or its title, may lead either, or follow
 * one of the first three (`take theirs #a1b2c3`, `keep mine (#a1b2c3)`). It
 * answers the one conflict the message names, else the one open that is not put off. Deterministic, case-insensitive
 * phrase matching, no model: prose that merely mentions an option records nothing, and so does an ambiguous message.
 */

/** What the person can answer from chat: the card's four options. */
export const CHAT_OPTIONS = ["mine", "theirs", "new", "later"] as const;
export type ChatOption = (typeof CHAT_OPTIONS)[number];

/** The longest new decision a chat answer records, as the dashboard allows. */
export const MAX_CHAT_DECISION = 600;

/** A conflict whose card the person was shown, and whether they put it off ("later"). */
export interface ChatConflict {
  readonly id: string;
  readonly shortId: string;
  readonly title: string;
  readonly snoozed?: boolean;
}

export type ChatAnswer =
  | { readonly kind: "none" }
  | { readonly kind: "ambiguous"; readonly reason: string }
  | { readonly kind: "answer"; readonly conflictId: string; readonly option: ChatOption; readonly text: string | null };

const WHOLE: Readonly<Record<string, Exclude<ChatOption, "new">>> = { "keep mine": "mine", "take theirs": "theirs", later: "later" };
const NEW_DECISION = /^new decision\s*:\s*/iu;
/** What may follow a whole-message option: punctuation and spaces. */
const TRAILING = /[\s.!]*$/u;
/** The card's own placeholder, which is never a decision. */
const PLACEHOLDER = /^<\s*text\s*>[\s.!?`'"]*$/iu;

/** The ways a message names `conflict`: `#a1b2c3`, `a1b2c3`, or its title (four characters or more), lower-cased. */
const namesOf = (conflict: ChatConflict): string[] =>
  [`#${conflict.shortId}`, conflict.shortId, ...(conflict.title.trim().length >= 4 ? [conflict.title.trim()] : [])].map(name => name.toLowerCase());

/** `message` without a leading name of one of `open` and the separator after it. */
function leadingName(message: string, open: readonly ChatConflict[]): { conflict: ChatConflict | null; rest: string } {
  const text = message.trimStart();
  const lower = text.toLowerCase();
  for (const conflict of open) {
    for (const name of namesOf(conflict)) {
      const after = lower.slice(name.length);
      if (lower.startsWith(name) && (after === "" || /^[\s:,.\-–—]/u.test(after))) {
        return { conflict, rest: text.slice(name.length).replace(/^[\s:,.\-–—]+/u, "") };
      }
    }
  }
  return { conflict: null, rest: text };
}

/**
 * `message` without a trailing name of one of `open` (`take theirs #a1b2c3`, `keep mine (#a1b2c3)`, `later a1b2c3.`):
 * the name last, in parentheses or not, after a space or separator, then only punctuation.
 */
function trailingName(message: string, open: readonly ChatConflict[]): { conflict: ChatConflict | null; rest: string } {
  const text = message.replace(TRAILING, "");
  const lower = text.toLowerCase();
  for (const conflict of open) {
    for (const name of namesOf(conflict)) {
      for (const form of [`(${name})`, name]) {
        const before = lower.slice(0, lower.length - form.length);
        if (lower.endsWith(form) && /[\s:,\-–—]$/u.test(before)) return { conflict, rest: text.slice(0, before.length).replace(/[\s:,\-–—]+$/u, "") };
      }
    }
  }
  return { conflict: null, rest: message };
}

/** The option `rest` is, with a new decision's text; null when it is not plainly one. */
function optionOf(rest: string): { option: ChatOption; text: string | null } | null {
  const decision = NEW_DECISION.exec(rest);
  if (decision !== null) return { option: "new", text: rest.slice(decision[0].length).trim() };
  const option = WHOLE[rest.replace(TRAILING, "").trim().toLowerCase()];
  return option === undefined ? null : { option, text: null };
}

/**
 * The option `message` is and the conflict it names: the name before the option, or (for keep mine, take theirs and
 * later) after it; a new decision's name comes first, since its text runs to the end.
 */
function answerIn(message: string, open: readonly ChatConflict[]): { named: ChatConflict | null; found: { option: ChatOption; text: string | null } } | null {
  const leading = leadingName(message, open);
  const found = optionOf(leading.rest);
  if (found !== null) return { named: leading.conflict, found };
  if (leading.conflict !== null) return null;
  const trailing = trailingName(message, open);
  const after = trailing.conflict === null ? null : optionOf(trailing.rest.trim());
  return after === null || after.option === "new" ? null : { named: trailing.conflict, found: after };
}

/** The person's answer in `message` to one of `open` (the checkpoint and stop cards they were shown), if it is plainly one. */
export function parseChatAnswer(message: string, open: readonly ChatConflict[]): ChatAnswer {
  if (open.length === 0) return { kind: "none" };
  const read = answerIn(message, open);
  if (read === null) return { kind: "none" };
  const { named, found } = read;
  if (found.option === "new") {
    if (found.text === null || found.text === "" || PLACEHOLDER.test(found.text)) return { kind: "ambiguous", reason: "a new decision without its text" };
    if (found.text.length > MAX_CHAT_DECISION) return { kind: "ambiguous", reason: `a new decision over ${MAX_CHAT_DECISION} characters` };
  }
  const awake = open.filter(conflict => conflict.snoozed !== true);
  const candidates = awake.length > 0 ? awake : open;
  const conflict = named ?? (candidates.length === 1 ? candidates[0] : undefined);
  if (conflict === undefined) return { kind: "ambiguous", reason: "several conflicts are open and the message names none" };
  return { kind: "answer", conflictId: conflict.id, option: found.option, text: found.text };
}
