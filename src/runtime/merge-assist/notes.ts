/**
 * The Merge Assist notes a session's coding agent gets (docs/in-session-surfaces.md, "Merge Assist notes"): the
 * decision it saved from the person's correction in that session; a decision the person settled a conflict with; a
 * teammate's decision that is a note, once absorbed or adopted; a trusted teammate's decision that contradicts this
 * branch's (Talk); an in-flight conflict at its level (ambient: information; checkpoint: a card to show the person;
 * stop: at an edit of a contested file, see {@link stopNotes}); and how many wait for the person's review. Each goes to
 * a session at most once per version of its candidate (a conflict once per level it reached, and again after a snooze
 * ends): what a session was handed is kept under `delivered:<session>`, and only once a delivery has taken it. A note
 * about a conflict carries the person's line for it (`conflict`). The person's `conflictDelivery` decides which of a
 * conflict's surfaces a session gets (`conflict-delivery.ts`): in `terminal` mode a conflict's note carries only the
 * person's line, with no text for the agent; a stop goes out in every mode.
 */
import { eq, like } from "drizzle-orm";
import { isAbsolute, relative, sep } from "node:path";
import { z } from "zod";
import type { ConflictNotice } from "../../engine/adapter-contract.js";
import { checkpointNote, conflictNote, decidedNote, hookFollowUpNote, hookNote, mergeAssistNote, stopNote, type ConflictCardInput } from "../../engine/code-rule/rerail-steer.js";
import { globPattern } from "../../engine/glob-pattern.js";
import type { Conflict, ConflictDelivery } from "../../engine/merge-assist/model.js";
import type { RerailDatabase } from "../../engine/storage/database.js";
import { mergeAssistItems, mergeAssistState, type MergeAssistItemRow } from "../../engine/storage/schema.js";
import { activationFailures, itemRuleId } from "./activation.js";
import { settingsAt, statusText, viaChat, viaTerminal } from "./conflict-delivery.js";
import { isSnoozed } from "./conflict-level.js";
import { readConflicts, type StoredConflict } from "./conflict-store.js";
import { STATE_KEYS, readState, writeState, type StateQueries } from "./state.js";

const deliveredSchema = z.array(z.string());

/**
 * A note not yet handed to the session, the marks (`<item id>:<candidate hash>:<news>`, or `<conflict id>:<surface>`)
 * of what it speaks for, and the conflict whose line the person sees with it.
 */
export interface PendingNote {
  readonly content: string;
  readonly marks: readonly string[];
  readonly conflict?: ConflictNotice;
}

function evidenceOf(item: MergeAssistItemRow): { mine?: { decision?: unknown }; ruleProblem?: unknown; branch?: unknown; resolves?: unknown; peer?: unknown } {
  return item.evidence as { mine?: { decision?: unknown }; ruleProblem?: unknown; branch?: unknown; resolves?: unknown; peer?: unknown };
}

/** `<branch>` or `PR #<n>`, as a note names where a decision came from. */
function peerLabel(item: MergeAssistItemRow): string {
  if (item.peerKind === "pull-request") return `PR #${item.peerNumber}`;
  const branch = evidenceOf(item).branch;
  return typeof branch === "string" ? branch : item.peerRef;
}

/** Who wrote a teammate's decision: GitHub's author of the pull request, else the name the candidate gives itself. */
function authorOf(item: MergeAssistItemRow): string {
  return item.peerKind === "pull-request" && item.peerAuthor !== null ? item.peerAuthor : item.candidate.author;
}

/** What an item can be news as for a session: its saved decision, a settled conflict, an applied note, a contradiction, or a review. */
type News = "saved" | "decided" | "applied" | "talk" | "review";

/** What one of this checkout's own decisions is news as: saved in this session, or settling a conflict (for every session). */
function ownNews(item: MergeAssistItemRow, sessionId: string): News | null {
  if (item.status !== "applied") return null;
  if (item.sessionId === sessionId) return "saved";
  return item.sessionId === null && typeof evidenceOf(item).resolves === "string" ? "decided" : null;
}

/**
 * What another branch's applied decision is news as: absorbed by the scan, or adopted from review by the person. A
 * decision with a working code rule steers like any rule; only a note reaches the agent as text.
 */
function appliedNews(item: MergeAssistItemRow, failed: Readonly<Record<string, string>>): News | null {
  if (item.tier !== "absorb" && item.tier !== "review") return null;
  const note = item.candidate.rule === undefined || typeof evidenceOf(item).ruleProblem === "string" || failed[itemRuleId(item)] !== undefined;
  return note ? "applied" : null;
}

/** What another branch's open finding is news as: only a trusted teammate's text is quoted; an untrusted one waits in review, counted. */
function openNews(item: MergeAssistItemRow): News | null {
  if (item.tier === "talk") return item.trusted && typeof evidenceOf(item).mine?.decision === "string" ? "talk" : null;
  return item.tier === "review" ? "review" : null;
}

/** What an item is news as for `sessionId`, or null when it is no news for the agent. */
function newsOf(item: MergeAssistItemRow, sessionId: string, failed: Readonly<Record<string, string>>): News | null {
  if (item.origin === "own") return ownNews(item, sessionId);
  if (item.status === "applied") return appliedNews(item, failed);
  return item.status === "open" ? openNews(item) : null;
}

/** The prefix every mark of an item's version starts with. */
const versionOf = (item: MergeAssistItemRow): string => `${item.id}:${item.candidateHash}:`;
/** The prefix every mark of a conflict starts with. */
const conflictMark = (conflict: Conflict): string => `${conflict.id}:`;

const who = (side: Conflict["theirs"]): string => (side.author === null ? side.label : `${side.label} (${side.author})`);

/** How a session takes a checkout's conflict notes: the person's settings, the conflict hook's state, and its host. */
export interface ConflictNoteOptions {
  readonly delivery: ConflictDelivery;
  /** The card offers the experimental verdicts instead of talking to the other side's author. */
  readonly verdictCards: boolean;
  /** What the session's host draws: the person's line (`systemMessage`), and a status line or widget. */
  readonly surfaces: { readonly personLine: boolean; readonly statusLine: boolean };
  readonly now: number;
}

export interface PendingNoteOptions extends ConflictNoteOptions {
  /** Conflicts whose agent note waits for a conflict hook handler (the person's command) to answer. */
  readonly awaiting: ReadonlySet<string>;
}

/**
 * The person's line for an open conflict: what it is at ambient; what to decide at a checkpoint or a stop. When the
 * delivery includes terminal on a host with no status line (Codex), the line carries the conflict's status line text too.
 */
function noticeOf(entry: StoredConflict, options: ConflictNoteOptions): ConflictNotice {
  const { conflict } = entry;
  const texts: Readonly<Record<Conflict["kind"], string>> = {
    overlap: `your edit overlaps ${who(conflict.theirs)}: ${conflict.title}`,
    decisions: `${who(conflict.theirs)} decided differently: ${conflict.title}`,
  };
  const text = conflict.level === "checkpoint" || conflict.level === "stop" ? `decide: ${conflict.title} (${conflict.theirs.label})` : texts[conflict.kind];
  const summary = viaTerminal(options.delivery) && !options.surfaces.statusLine ? ` — ${statusText(entry, options.now)}` : "";
  return { id: conflict.id, text: `${text}${summary}` };
}

/**
 * `notice` as the person's line reads at delivery time: from the conflict as stored now, so a status line text the
 * conflict hook or the quick call gave after the note was queued is the one it carries. Unchanged when the conflict is
 * no longer open here; a stop's question to the person stays.
 */
export function currentNotice(db: StateQueries, checkoutPath: string, notice: ConflictNotice, options: ConflictNoteOptions): ConflictNotice {
  const entry = readConflicts(db, checkoutPath).find(({ conflict }) => conflict.id === notice.id && conflict.status === "open");
  if (entry === undefined) return notice;
  const current = noticeOf(entry, options);
  return notice.ask === undefined ? current : { ...current, ask: notice.ask };
}

/** The open conflicts the session and the person hear about: every level but logged (those stay on the page). */
function surfacedConflicts(db: StateQueries, checkoutPath: string): StoredConflict[] {
  return readConflicts(db, checkoutPath).filter(({ conflict }) => conflict.status === "open" && conflict.level !== "logged");
}

/** The card the person decides on, from the conflict as stored. */
function cardOf(conflict: Conflict, verdictCards: boolean): ConflictCardInput {
  return {
    shortId: conflict.shortId, title: conflict.title, mine: conflict.mine.summary, theirs: conflict.theirs.summary,
    author: conflict.theirs.author, peer: conflict.theirs.label, url: conflict.theirs.url, reasons: conflict.reasons,
    answers: verdictCards ? "verdicts" : "talk",
  };
}

/** What the person's conflict hook command added for the agent, when it answered. */
const hookNoteOf = (conflict: Conflict): string | null => (conflict.hook?.ok === true && conflict.hook.note?.trim() ? conflict.hook.note : null);

/** The mark of the hook's note in its latest answer, when it gave one: given once per conflict, event and session. */
const hookMark = (conflict: Conflict): string[] =>
  (hookNoteOf(conflict) === null ? [] : [`${conflictMark(conflict)}hook:${conflict.hook!.event}@${conflict.hook!.at}`]);

/** A checkpoint's mark: once per conflict, and once more after each snooze ends, so "later" comes back. */
const checkpointMark = (conflict: Conflict): string => {
  const ended = conflict.snooze?.endedAt ?? null;
  return `${conflictMark(conflict)}checkpoint${ended === null ? "" : `@${ended}`}`;
};

/**
 * Whether the session was handed a conflict's card: its checkpoint note (any round) or its stop, recorded as given.
 * Only that session's person can answer the card from chat.
 */
export function cardsShown(db: StateQueries, sessionId: string): (conflict: Conflict) => boolean {
  const given = readState(db, STATE_KEYS.delivered(sessionId), deliveredSchema) ?? [];
  return conflict => given.some(mark => mark === `${conflictMark(conflict)}stop` || mark.startsWith(`${conflictMark(conflict)}checkpoint`));
}

/**
 * A conflict's note for one surface: through the agent (`content`, with the hook's note) in chat modes; in terminal
 * mode, only the person's line, where the host draws one; nothing otherwise.
 */
function surfaceNote(entry: StoredConflict, content: () => string, mark: string, options: ConflictNoteOptions): PendingNote[] {
  const notice = noticeOf(entry, options);
  if (viaChat(options.delivery)) return [{ content: hookNote(content(), hookNoteOf(entry.conflict)), marks: [mark, ...hookMark(entry.conflict)], conflict: notice }];
  return options.surfaces.personLine ? [{ content: "", marks: [mark], conflict: notice }] : [];
}

/** Whether a conflict's note waits: its mark is taken, or (through the agent) a hook handler has not answered yet. */
function waits(conflict: Conflict, mark: string, taken: (mark: string) => boolean, options: PendingNoteOptions): boolean {
  return taken(mark) || (viaChat(options.delivery) && options.awaiting.has(conflict.id));
}

/** One note per ambient overlap the session was not handed: who, what they did, information only. */
function ambientNotes(conflicts: readonly StoredConflict[], taken: (mark: string) => boolean, options: PendingNoteOptions): PendingNote[] {
  return conflicts.filter(({ conflict }) => conflict.level === "ambient" && conflict.kind === "overlap" && !waits(conflict, `${conflictMark(conflict)}open`, taken, options))
    .flatMap(entry => surfaceNote(entry, () => {
      const { conflict, files } = entry;
      return conflictNote({ file: files[0] ?? conflict.title, peer: conflict.theirs.label, author: conflict.theirs.author, summary: conflict.theirs.summary, pullRequest: conflict.theirs.label.startsWith("PR #") });
    }, `${conflictMark(entry.conflict)}open`, options));
}

/**
 * One card per checkpoint or stop conflict the session was not handed and the person has not put off ("later"):
 * through the agent in chat modes; in terminal mode the person's line carries it, with the status line.
 */
function checkpointNotes(conflicts: readonly StoredConflict[], taken: (mark: string) => boolean, options: PendingNoteOptions): PendingNote[] {
  return conflicts.filter(({ conflict }) => (conflict.level === "checkpoint" || conflict.level === "stop") && !isSnoozed(conflict) && !waits(conflict, checkpointMark(conflict), taken, options))
    .flatMap(entry => surfaceNote(entry, () => checkpointNote(cardOf(entry.conflict, options.verdictCards), entry.files), checkpointMark(entry.conflict), options));
}

/** Whether the session was handed (or is about to be handed) a conflict's own note: ambient, checkpoint or stop. */
const noteGiven = (conflict: Conflict, taken: (mark: string) => boolean): boolean =>
  taken(`${conflictMark(conflict)}open`) || taken(`${conflictMark(conflict)}stop`) || taken(checkpointMark(conflict));

/**
 * The hook's note on its own, through the agent, for each conflict whose note went out before the person's command
 * answered (Rerail's note waits for it a second at most): once per conflict, event and session, attributed to the hook.
 */
function hookFollowUps(conflicts: readonly StoredConflict[], taken: (mark: string) => boolean, options: PendingNoteOptions): PendingNote[] {
  if (!viaChat(options.delivery)) return [];
  return conflicts.flatMap(({ conflict }) => {
    const marks = hookMark(conflict);
    const note = hookNoteOf(conflict);
    if (note === null || marks.some(taken) || !noteGiven(conflict, taken)) return [];
    return [{ content: hookFollowUpNote({ title: conflict.title, peer: conflict.theirs.label }, note), marks }];
  });
}

/**
 * The notes for `sessionId` in the checkout that it has not been handed, in the order saved, decided, applied, talk,
 * ambient conflicts, checkpoints, review; items whose marks are in `inFlight` (queued for a delivery that has not taken
 * them yet) are left out. An item's mark is `<item id>:<candidate hash>:<news>`, so an item is news again when it moves,
 * as from review to adopted. Reads only: {@link markNotesDelivered} records a note once it is handed over.
 */
export function pendingNotes(db: StateQueries, sessionId: string, checkoutPath: string, inFlight: ReadonlySet<string>, options: PendingNoteOptions): PendingNote[] {
  const given = new Set(readState(db, STATE_KEYS.delivered(sessionId), deliveredSchema) ?? []);
  const taken = (mark: string): boolean => given.has(mark) || inFlight.has(mark);
  const items = db.select().from(mergeAssistItems).where(eq(mergeAssistItems.checkoutPath, checkoutPath)).all();
  const failed = activationFailures(db, checkoutPath);
  const fresh: Record<News, Array<{ item: MergeAssistItemRow; mark: string }>> = { saved: [], decided: [], applied: [], talk: [], review: [] };
  for (const item of items) {
    const news = newsOf(item, sessionId, failed);
    const mark = `${versionOf(item)}${news}`;
    if (news !== null && !taken(mark)) fresh[news].push({ item, mark });
  }
  const conflicts = surfacedConflicts(db, checkoutPath);
  return [
    ...itemNotes(fresh, conflicts, options),
    ...ambientNotes(conflicts, taken, options),
    ...checkpointNotes(conflicts, taken, options),
    ...hookFollowUps(conflicts, taken, options),
    ...reviewNote(fresh.review),
  ];
}

/** Whether a stop conflict's contested files include `path` (repository-relative): an overlap's file, or a decision's scope. */
function contests({ conflict, files }: StoredConflict, path: string): boolean {
  if (conflict.kind === "overlap") return files.includes(path);
  try {
    return globPattern(files).test(path);
  } catch {
    return false;
  }
}

/** A tool call as the stop check reads it: its tool's name and arguments. */
export interface EditCall {
  readonly toolName: string;
  readonly input: unknown;
}

/** Tools that edit files, across coding agents. */
const EDIT_TOOLS = /^(?:write|edit|multiedit|notebookedit|ast_edit|apply_patch)$/iu;
/** Tools that run shell commands (Codex edits through them), as `isCommandTool` names them. */
const COMMAND_TOOLS = /bash|shell|exec|terminal|command|cmd/iu;
/** A file a patch (`apply_patch`) adds, updates or deletes, or a unified diff's new file. */
const PATCH_FILE = /^(?:\*\*\* (?:Update|Add|Delete|Move to) File: |\+\+\+ b\/)(.+)$/gmu;
/** The target of a `>` or `>>` redirection (not `2>&1` or `<>`). */
const REDIRECT = /(?:^|[^<>&0-9])>>?\s*(["']?)([^\s"'<>|;&()]+)\1/gu;

const unquote = (token: string): string => token.replace(/^(["'])(.*)\1$/u, "$2");

/**
 * The files one shell command segment writes, read deterministically: redirection targets; `tee`'s files; the files
 * `sed -i` and `perl -i` edit in place; `mv`, `cp` and `install`'s destination; `truncate` and `rm`'s operands.
 */
function segmentWrites(segment: string): string[] {
  const written = [...segment.matchAll(REDIRECT)].map(match => match[2]!);
  const tokens = segment.trim().split(/\s+/u).map(unquote).filter(token => token !== "");
  const [command, ...args] = tokens;
  const operands = args.filter(token => !token.startsWith("-"));
  if (command === "tee" || command === "truncate" || command === "rm") written.push(...operands);
  if ((command === "sed" || command === "perl") && args.some(token => /^-[a-zA-Z]*i/u.test(token))) written.push(...operands.slice(1));
  if ((command === "mv" || command === "cp" || command === "install") && operands.length >= 2) written.push(operands.at(-1)!);
  return written;
}

/** The files a shell command writes: each segment's (`;`, `&&`, `||`, `|`, a new line), and any patch it applies. */
function shellWrites(command: string): string[] {
  return [
    ...[...command.matchAll(PATCH_FILE)].map(match => match[1]!.trim()),
    ...command.split(/\r?\n|;|&&|\|\||\|/u).flatMap(segmentWrites),
  ];
}

/**
 * The repository-relative paths an edit names: an edit tool's one-line string arguments and the files a patch touches,
 * or the files a shell command writes (see {@link shellWrites}).
 */
function editedPaths(root: string, call: EditCall): string[] {
  const edit = EDIT_TOOLS.test(call.toolName);
  if (!edit && !COMMAND_TOOLS.test(call.toolName)) return [];
  const texts: string[] = [];
  const walk = (value: unknown, depth: number): void => {
    if (typeof value === "string") texts.push(value);
    else if (depth < 4 && value !== null && typeof value === "object") for (const child of Object.values(value)) walk(child, depth + 1);
  };
  walk(call.input, 0);
  const named = edit
    ? texts.flatMap(text => [
      ...[...text.matchAll(PATCH_FILE)].map(match => match[1]!.trim()),
      ...(/[\r\n]/u.test(text) || text.length > 1_024 ? [] : [text.trim()]),
    ])
    : texts.flatMap(shellWrites);
  const relativeOf = (path: string): string => (isAbsolute(path) ? relative(root, path) : path).split(sep).join("/").replace(/^\.\//u, "");
  return [...new Set(named.map(relativeOf).filter(path => path !== "" && !path.startsWith("../") && path !== ".."))];
}

/** The question a host that can ask the person puts before an edit to a contested file. */
function stopAsk({ conflict, files }: StoredConflict, verdictCards: boolean): string {
  const contested = files.length === 1 ? `${files[0]} is` : `${files.join(", ")} are`;
  const then = verdictCards ? "decide first (keep mine · take theirs · new decision · later)" : `talk to ${conflict.theirs.author ?? `the author of ${conflict.theirs.label}`} first`;
  return `Rerail: ${contested} contested with ${who(conflict.theirs)}: ${conflict.title}. Allow this edit, or deny it and ${then}.`;
}

/**
 * The stop notes for a tool call about to edit contested files: one per open stop conflict whose files it edits, not
 * snoozed, once per session, in every delivery mode. Where the host can ask the person (`asks`), the first stop the
 * session gets asks them before the edit (`conflict.ask`); every other one tells the agent not to edit those files and
 * to ask the person now. Each carries the person's line and counts as the conflict's checkpoint card too. Reads only.
 */
export function stopNotes(db: StateQueries, sessionId: string, checkoutPath: string, call: EditCall, asks: boolean, inFlight: ReadonlySet<string>, options: ConflictNoteOptions): PendingNote[] {
  const paths = editedPaths(checkoutPath, call);
  if (paths.length === 0) return [];
  const given = new Set(readState(db, STATE_KEYS.delivered(sessionId), deliveredSchema) ?? []);
  const taken = (mark: string): boolean => given.has(mark) || inFlight.has(mark);
  let asked = [...given, ...inFlight].some(mark => mark.endsWith(":stop"));
  return readConflicts(db, checkoutPath)
    .filter(entry => entry.conflict.status === "open" && entry.conflict.level === "stop" && !isSnoozed(entry.conflict)
      && !taken(`${conflictMark(entry.conflict)}stop`) && paths.some(path => contests(entry, path)))
    .map(entry => {
      const { conflict, files } = entry;
      const ask = asks && !asked;
      asked ||= ask;
      const notice = noticeOf(entry, options);
      const card = stopNote(cardOf(conflict, options.verdictCards), files, ask);
      return {
        content: viaChat(options.delivery) ? hookNote(card, hookNoteOf(conflict)) : card,
        marks: [`${conflictMark(conflict)}stop`, checkpointMark(conflict), ...(viaChat(options.delivery) ? hookMark(conflict) : [])],
        conflict: ask ? { ...notice, ask: stopAsk(entry, options.verdictCards) } : notice,
      };
    });
}

/** What the footer shows of the conflicts in a session's checkout (docs/in-session-surfaces.md, "What the footer means"). */
export interface ConflictFooter {
  /** Open conflicts above logged, of every kind: the newest one's line, and `(+N)` for the rest. */
  readonly conflicts: number;
  /** The loudest open checkpoint or stop conflict the person has not put off: `◉ decide: <title>`. */
  readonly decide: { readonly id: string; readonly title: string } | null;
  /** The newest open conflict above logged: `◐ <summary>`, the hook's status line when it gave one. */
  readonly summary: { readonly id: string; readonly text: string };
}

/**
 * The footer's Merge Assist segment for a session working in `cwd`: from the conflicts of the checkout that holds it
 * (the longest stored checkout path `cwd` is in); undefined when none is open above logged, or when the person's
 * `conflictDelivery` leaves the status line out (chat). Reads only.
 */
export function conflictFooter(db: StateQueries, cwd: string, now: number): ConflictFooter | undefined {
  const prefix = STATE_KEYS.conflicts("");
  const checkout = db.select({ key: mergeAssistState.key }).from(mergeAssistState).where(like(mergeAssistState.key, `${prefix}%`)).all()
    .map(row => row.key.slice(prefix.length))
    .filter(path => cwd === path || cwd.startsWith(`${path}${sep}`))
    .sort((left, right) => right.length - left.length)[0];
  if (checkout === undefined || !viaTerminal(settingsAt(db, checkout).delivery)) return undefined;
  const open = surfacedConflicts(db, checkout);
  const newest = open.toSorted((left, right) => right.conflict.detectedAt.localeCompare(left.conflict.detectedAt))[0];
  if (newest === undefined) return undefined;
  const loudest = open.map(entry => entry.conflict).filter(conflict => (conflict.level === "checkpoint" || conflict.level === "stop") && !isSnoozed(conflict))
    .sort((left, right) => Number(right.level === "stop") - Number(left.level === "stop") || left.detectedAt.localeCompare(right.detectedAt))[0];
  return {
    conflicts: open.length,
    decide: loudest === undefined ? null : { id: loudest.id, title: loudest.title },
    summary: { id: newest.conflict.id, text: statusText(newest, now) },
  };
}

/** The notes for fresh items, but the review count: each of saved, decided, applied and talk on its own. */
function itemNotes(fresh: Record<News, Array<{ item: MergeAssistItemRow; mark: string }>>, conflicts: readonly StoredConflict[], options: ConflictNoteOptions): PendingNote[] {
  return [
    ...fresh.saved.map(({ item, mark }) => ({ content: mergeAssistNote({ kind: "saved", decision: item.candidate.decision, candidateId: item.candidateId }), marks: [mark] })),
    ...fresh.decided.map(({ item, mark }) => ({ content: decidedNote(item.candidate.decision, String(evidenceOf(item).peer ?? "another branch")), marks: [mark] })),
    ...fresh.applied.map(({ item, mark }) => ({ content: mergeAssistNote({ kind: "absorbed", decision: item.candidate.decision, author: authorOf(item), peer: peerLabel(item) }), marks: [mark] })),
    ...fresh.talk.map(({ item, mark }) => talkNote(item, mark, conflicts, options)),
  ];
}

/** A Talk note, with the person's line for the contradiction it is when that conflict is ambient (a card carries its own). */
function talkNote(item: MergeAssistItemRow, mark: string, conflicts: readonly StoredConflict[], options: ConflictNoteOptions): PendingNote {
  const mine = String(evidenceOf(item).mine?.decision);
  const content = mergeAssistNote({ kind: "talk", author: authorOf(item), peer: peerLabel(item), theirs: item.candidate.decision, mine });
  const conflict = conflicts.find(({ conflict: entry }) => entry.kind === "decisions" && entry.level === "ambient" && entry.theirs.decisionId === item.candidateId);
  return conflict === undefined ? { content, marks: [mark] } : { content, marks: [mark], conflict: noticeOf(conflict, options) };
}

function reviewNote(review: ReadonlyArray<{ mark: string }>): PendingNote[] {
  if (review.length === 0) return [];
  return [{ content: mergeAssistNote({ kind: "review", count: review.length }), marks: review.map(({ mark }) => mark) }];
}

/**
 * Record that `marks` were handed to the session. Only marks of items and conflicts still in the checkout are kept: a
 * withdrawn decision that comes back is news again.
 */
export function markNotesDelivered(db: RerailDatabase, sessionId: string, checkoutPath: string, marks: readonly string[], now: number): void {
  if (marks.length === 0) return;
  db.transaction(tx => {
    const key = STATE_KEYS.delivered(sessionId);
    const given = readState(tx, key, deliveredSchema) ?? [];
    const current = [
      ...tx.select().from(mergeAssistItems).where(eq(mergeAssistItems.checkoutPath, checkoutPath)).all().map(versionOf),
      ...readConflicts(tx, checkoutPath).map(entry => conflictMark(entry.conflict)),
    ];
    writeState(tx, key, [...new Set([...given, ...marks])].filter(entry => current.some(version => entry.startsWith(version))), now);
  }, { behavior: "immediate" });
}
