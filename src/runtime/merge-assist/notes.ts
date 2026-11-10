/**
 * The Merge Assist notes a session's coding agent gets (docs/in-session-surfaces.md, "Merge Assist notes"): the
 * decision it saved from the person's correction in that session; a decision the person settled a conflict with; a
 * teammate's decision that is a note, once absorbed or adopted; a trusted teammate's decision that contradicts this
 * branch's (Talk); an in-flight conflict at its level (ambient: information; checkpoint: a card to show the person;
 * stop: at an edit of a contested file, see {@link stopNotes}); and how many wait for the person's review. Each goes to
 * a session at most once per version of its candidate (a conflict once per level it reached, and again after a snooze
 * ends): what a session was handed is kept under `delivered:<session>`, and only once a delivery has taken it. A note
 * about a conflict carries the person's line for it (`conflict`).
 */
import { eq, like } from "drizzle-orm";
import { isAbsolute, relative, sep } from "node:path";
import { z } from "zod";
import type { ConflictNotice } from "../../engine/adapter-contract.js";
import { checkpointNote, conflictNote, decidedNote, mergeAssistNote, stopNote, verdictsNote, type ConflictCardInput } from "../../engine/code-rule/rerail-steer.js";
import { globPattern } from "../../engine/glob-pattern.js";
import type { Conflict } from "../../engine/merge-assist/model.js";
import type { RerailDatabase } from "../../engine/storage/database.js";
import { mergeAssistItems, mergeAssistState, type MergeAssistItemRow } from "../../engine/storage/schema.js";
import { activationFailures, itemRuleId } from "./activation.js";
import { isSnoozed } from "./conflict-level.js";
import { followedMark, provisionalResults, type ProvisionalResult } from "./ledger-store.js";
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

function evidenceOf(item: MergeAssistItemRow): { mine?: { decision?: unknown }; ruleProblem?: unknown; branch?: unknown; resolves?: unknown; peer?: unknown; followed?: unknown } {
  return item.evidence as { mine?: { decision?: unknown }; ruleProblem?: unknown; branch?: unknown; resolves?: unknown; peer?: unknown; followed?: unknown };
}

/**
 * The decision an own item steers by: its candidate's text, or for one that follows another branch's decision by
 * reference, that decision's latest text as the last detection read it (`refreshFollowed`).
 */
function decisionText(item: MergeAssistItemRow): string {
  const followed = evidenceOf(item).followed;
  return item.candidate.follows !== undefined && typeof followed === "string" ? followed : item.candidate.decision;
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

/**
 * What an item can be news as for a session: its saved decision, a settled conflict, an applied note, another branch's
 * verdict that has not merged (information only), a contradiction, or a review.
 */
type News = "saved" | "decided" | "applied" | "provisional" | "talk" | "review";

/** What one of this checkout's own decisions is news as: saved in this session, or settling a conflict (for every session). */
function ownNews(item: MergeAssistItemRow, sessionId: string): News | null {
  if (item.status !== "applied") return null;
  if (item.sessionId === sessionId) return "saved";
  return item.sessionId === null && typeof evidenceOf(item).resolves === "string" ? "decided" : null;
}

/**
 * What an item is news as for `sessionId`, or null when it is no news for the agent. A decision that is the result of
 * a verdict not in force (`verdicts`) is no guidance while a `verdicts` conflict on its question is open, whichever side
 * it is; another branch's otherwise reaches the agent only as information.
 */
function newsOf(item: MergeAssistItemRow, sessionId: string, failed: Readonly<Record<string, string>>, verdicts: ReadonlyMap<string, ProvisionalResult>): News | null {
  const verdict = verdicts.get(item.candidateId);
  if (verdict?.contested === true) return null;
  if (item.origin === "own") return ownNews(item, sessionId);
  if (verdict !== undefined && verdict.place !== "branch") return item.status === "applied" ? "provisional" : null;
  if (item.status === "applied" && (item.tier === "absorb" || item.tier === "review")) {
    // Absorbed by the scan, or adopted from review by the person. A decision with a working code rule steers like any
    // rule; only a note reaches the agent as text.
    const note = item.candidate.rule === undefined || typeof evidenceOf(item).ruleProblem === "string" || failed[itemRuleId(item)] !== undefined;
    return note ? "applied" : null;
  }
  if (item.status !== "open") return null;
  // Only a trusted teammate's text is quoted to the agent; an untrusted one waits in review, counted.
  if (item.tier === "talk") return item.trusted && typeof evidenceOf(item).mine?.decision === "string" ? "talk" : null;
  return item.tier === "review" ? "review" : null;
}

/** The prefix every mark of an item's version starts with. */
const versionOf = (item: MergeAssistItemRow): string => `${item.id}:${item.candidateHash}:`;
/** The prefix every mark of a conflict starts with. */
const conflictMark = (conflict: Conflict): string => `${conflict.id}:`;

const who = (side: Conflict["theirs"]): string => (side.author === null ? side.label : `${side.label} (${side.author})`);

/** The person's line for an open conflict: what it is at ambient; what to decide at a checkpoint or a stop. */
function noticeOf(conflict: Conflict): ConflictNotice {
  if (conflict.level === "checkpoint" || conflict.level === "stop") return { id: conflict.id, text: `decide: ${conflict.title} (${conflict.theirs.label})` };
  const texts: Readonly<Record<Conflict["kind"], string>> = {
    overlap: `your edit overlaps ${who(conflict.theirs)}: ${conflict.title}`,
    decisions: `${who(conflict.theirs)} decided differently: ${conflict.title}`,
    verdicts: `${who(conflict.theirs)} recorded a different verdict on "${conflict.title}"`,
  };
  const text = texts[conflict.kind];
  return { id: conflict.id, text };
}

/** The open conflicts the session and the person hear about: every level but logged (those stay on the page). */
function surfacedConflicts(db: StateQueries, checkoutPath: string): StoredConflict[] {
  return readConflicts(db, checkoutPath).filter(({ conflict }) => conflict.status === "open" && conflict.level !== "logged");
}

/** The card the person decides on, from the conflict as stored. */
function cardOf(conflict: Conflict): ConflictCardInput {
  return {
    shortId: conflict.shortId, title: conflict.title, mine: conflict.mine.summary, theirs: conflict.theirs.summary,
    author: conflict.theirs.author, peer: conflict.theirs.label, reasons: conflict.reasons,
  };
}

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

/** One note per ambient overlap or verdict clash the session was not handed: who, what they did, information only. */
function ambientNotes(conflicts: readonly StoredConflict[], taken: (mark: string) => boolean): PendingNote[] {
  return conflicts.filter(({ conflict }) => conflict.level === "ambient" && conflict.kind !== "decisions" && !taken(`${conflictMark(conflict)}open`))
    .map(({ conflict, files }) => ({
      content: conflict.kind === "overlap"
        ? conflictNote({
          file: files[0] ?? conflict.title,
          peer: conflict.theirs.label,
          author: conflict.theirs.author,
          summary: conflict.theirs.summary,
          pullRequest: conflict.theirs.label.startsWith("PR #"),
        })
        : verdictsNote({ title: conflict.title, peer: conflict.theirs.label, author: conflict.theirs.author }),
      marks: [`${conflictMark(conflict)}open`],
      conflict: noticeOf(conflict),
    }));
}

/** One card per checkpoint or stop conflict the session was not handed and the person has not put off ("later"). */
function checkpointNotes(conflicts: readonly StoredConflict[], taken: (mark: string) => boolean): PendingNote[] {
  return conflicts.filter(({ conflict }) => (conflict.level === "checkpoint" || conflict.level === "stop") && !isSnoozed(conflict) && !taken(checkpointMark(conflict)))
    .map(({ conflict, files }) => ({ content: checkpointNote(cardOf(conflict), files), marks: [checkpointMark(conflict)], conflict: noticeOf(conflict) }));
}

/**
 * The notes for `sessionId` in the checkout that it has not been handed, in the order saved, decided, applied, talk,
 * ambient conflicts, checkpoints, review; items whose marks are in `inFlight` (queued for a delivery that has not taken
 * them yet) are left out. An item's mark is `<item id>:<candidate hash>:<news>`, so an item is news again when it moves,
 * as from review to adopted. Reads only: {@link markNotesDelivered} records a note once it is handed over.
 */
export function pendingNotes(db: StateQueries, sessionId: string, checkoutPath: string, inFlight: ReadonlySet<string>, decisionLedger: boolean): PendingNote[] {
  const given = new Set(readState(db, STATE_KEYS.delivered(sessionId), deliveredSchema) ?? []);
  const taken = (mark: string): boolean => given.has(mark) || inFlight.has(mark);
  const items = db.select().from(mergeAssistItems).where(eq(mergeAssistItems.checkoutPath, checkoutPath)).all();
  const failed = activationFailures(db, checkoutPath);
  const fresh: Record<News, Array<{ item: MergeAssistItemRow; mark: string }>> = { saved: [], decided: [], applied: [], provisional: [], talk: [], review: [] };
  const verdicts = provisionalResults(db, checkoutPath, decisionLedger);
  for (const item of items) {
    const news = newsOf(item, sessionId, failed, verdicts);
    // A decision that follows another is news again when the followed text changes.
    const mark = `${versionOf(item)}${news}${news === "decided" && item.candidate.follows !== undefined ? `:${followedMark(decisionText(item))}` : ""}`;
    if (news !== null && !taken(mark)) fresh[news].push({ item, mark });
  }
  const conflicts = surfacedConflicts(db, checkoutPath);
  return [
    ...itemNotes(fresh, conflicts.map(entry => entry.conflict)),
    ...ambientNotes(conflicts, taken),
    ...checkpointNotes(conflicts, taken),
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

/**
 * The stop notes for a tool call about to edit contested files: one per open stop conflict whose files it edits, not
 * snoozed, once per session. Where the host can ask the person (`asks`), the first stop the session gets asks them
 * before the edit (`conflict.ask`); every other one tells the agent not to edit those files and to ask the person now.
 * Each carries the person's line and counts as the conflict's checkpoint card too. Reads only.
 */
export function stopNotes(db: StateQueries, sessionId: string, checkoutPath: string, call: EditCall, asks: boolean, inFlight: ReadonlySet<string>): PendingNote[] {
  const paths = editedPaths(checkoutPath, call);
  if (paths.length === 0) return [];
  const given = new Set(readState(db, STATE_KEYS.delivered(sessionId), deliveredSchema) ?? []);
  const taken = (mark: string): boolean => given.has(mark) || inFlight.has(mark);
  let asked = [...given, ...inFlight].some(mark => mark.endsWith(":stop"));
  return readConflicts(db, checkoutPath)
    .filter(entry => entry.conflict.status === "open" && entry.conflict.level === "stop" && !isSnoozed(entry.conflict)
      && !taken(`${conflictMark(entry.conflict)}stop`) && paths.some(path => contests(entry, path)))
    .map(({ conflict, files }) => {
      const ask = asks && !asked;
      asked ||= ask;
      const notice = noticeOf(conflict);
      const contested = files.length === 1 ? `${files[0]} is` : `${files.join(", ")} are`;
      return {
        content: stopNote(cardOf(conflict), files, ask),
        marks: [`${conflictMark(conflict)}stop`, checkpointMark(conflict)],
        conflict: ask
          ? { ...notice, ask: `Rerail: ${contested} contested with ${who(conflict.theirs)}: ${conflict.title}. Allow this edit, or deny it and decide first (keep mine · take theirs · new decision · later).` }
          : notice,
      };
    });
}

/** What the footer shows of the conflicts in a session's checkout (docs/in-session-surfaces.md, "What the footer means"). */
export interface ConflictFooter {
  /** Open conflicts above logged, of every kind: `◐ N conflicts`. */
  readonly conflicts: number;
  /** The loudest open checkpoint or stop conflict the person has not put off: `◉ decide: <title>`. */
  readonly decide: { readonly id: string; readonly title: string } | null;
}

/**
 * The footer's Merge Assist segment for a session working in `cwd`: from the conflicts of the checkout that holds it
 * (the longest stored checkout path `cwd` is in); undefined when none is open above logged. Reads only.
 */
export function conflictFooter(db: StateQueries, cwd: string): ConflictFooter | undefined {
  const prefix = STATE_KEYS.conflicts("");
  const checkout = db.select({ key: mergeAssistState.key }).from(mergeAssistState).where(like(mergeAssistState.key, `${prefix}%`)).all()
    .map(row => row.key.slice(prefix.length))
    .filter(path => cwd === path || cwd.startsWith(`${path}${sep}`))
    .sort((left, right) => right.length - left.length)[0];
  if (checkout === undefined) return undefined;
  const open = readConflicts(db, checkout).map(entry => entry.conflict).filter(conflict => conflict.status === "open" && conflict.level !== "logged");
  if (open.length === 0) return undefined;
  const loudest = open.filter(conflict => (conflict.level === "checkpoint" || conflict.level === "stop") && !isSnoozed(conflict))
    .sort((left, right) => Number(right.level === "stop") - Number(left.level === "stop") || left.detectedAt.localeCompare(right.detectedAt))[0];
  return { conflicts: open.length, decide: loudest === undefined ? null : { id: loudest.id, title: loudest.title } };
}

/** The notes for fresh items, but the review count: each of saved, decided, applied, provisional and talk on its own. */
function itemNotes(fresh: Record<News, Array<{ item: MergeAssistItemRow; mark: string }>>, conflicts: readonly Conflict[]): PendingNote[] {
  return [
    ...fresh.saved.map(({ item, mark }) => ({ content: mergeAssistNote({ kind: "saved", decision: item.candidate.decision, candidateId: item.candidateId }), marks: [mark] })),
    ...fresh.decided.map(({ item, mark }) => ({ content: decidedNote(decisionText(item), String(evidenceOf(item).peer ?? "another branch")), marks: [mark] })),
    ...fresh.applied.map(({ item, mark }) => ({ content: mergeAssistNote({ kind: "absorbed", decision: item.candidate.decision, author: authorOf(item), peer: peerLabel(item) }), marks: [mark] })),
    ...fresh.provisional.map(({ item, mark }) => ({ content: mergeAssistNote({ kind: "provisional", decision: item.candidate.decision, author: authorOf(item), peer: peerLabel(item) }), marks: [mark] })),
    ...fresh.talk.map(({ item, mark }) => talkNote(item, mark, conflicts)),
  ];
}

/** A Talk note, with the person's line for the contradiction it is when that conflict is ambient (a card carries its own). */
function talkNote(item: MergeAssistItemRow, mark: string, conflicts: readonly Conflict[]): PendingNote {
  const mine = String(evidenceOf(item).mine?.decision);
  const content = mergeAssistNote({ kind: "talk", author: authorOf(item), peer: peerLabel(item), theirs: item.candidate.decision, mine });
  const conflict = conflicts.find(entry => entry.kind === "decisions" && entry.level === "ambient" && entry.theirs.decisionId === item.candidateId);
  return conflict === undefined ? { content, marks: [mark] } : { content, marks: [mark], conflict: noticeOf(conflict) };
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
