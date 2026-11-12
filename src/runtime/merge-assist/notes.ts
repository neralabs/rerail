/**
 * The Merge Assist notes a session's coding agent gets (docs/in-session-surfaces.md, "Merge Assist notes"): an in-flight
 * conflict at its level (ambient: information; checkpoint: a card to show the person; stop: at an edit of a contested
 * file, see {@link stopNotes}), and what the person's conflict hook command added. A conflict goes to a session once per
 * level it reached: what a session was handed is kept under `delivered:<session>`, and only once a delivery has taken
 * it. A note carries the person's line for its conflict (`conflict`). The person's `conflictDelivery` decides which of a
 * conflict's surfaces a session gets (`conflict-delivery.ts`): in `terminal` mode a conflict's note carries only the
 * person's line, with no text for the agent; a stop goes out in every mode.
 */
import { like } from "drizzle-orm";
import { isAbsolute, relative, sep } from "node:path";
import { z } from "zod";
import type { ConflictNotice } from "../../engine/adapter-contract.js";
import { checkpointNote, conflictNote, hookFollowUpNote, hookNote, sideName, stopNote, type ConflictCardInput } from "../../engine/code-rule/rerail-steer.js";
import type { Conflict, ConflictDelivery } from "../../engine/merge-assist/model.js";
import type { RerailDatabase } from "../../engine/storage/database.js";
import { mergeAssistState } from "../../engine/storage/schema.js";
import { settingsAt, statusText, viaChat, viaTerminal } from "./conflict-delivery.js";
import { readConflicts, type StoredConflict } from "./conflict-store.js";
import { STATE_KEYS, readState, writeState, type StateQueries } from "./state.js";

const deliveredSchema = z.array(z.string());

/** A note not yet handed to the session, the marks (`<conflict id>:<surface>`) of what it speaks for, and the conflict whose line the person sees with it. */
export interface PendingNote {
  readonly content: string;
  readonly marks: readonly string[];
  readonly conflict?: ConflictNotice;
}

/** The prefix every mark of a conflict starts with. */
const conflictMark = (conflict: Conflict): string => `${conflict.id}:`;

/** How a session takes a checkout's conflict notes: the person's settings, the conflict hook's state, and its host. */
export interface ConflictNoteOptions {
  readonly delivery: ConflictDelivery;
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
  const text = conflict.level === "checkpoint" || conflict.level === "stop"
    ? `decide: ${conflict.title} (${conflict.theirs.label})`
    : `your edit overlaps ${sideName(conflict.theirs)}: ${conflict.title}`;
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
function cardOf(conflict: Conflict): ConflictCardInput {
  return {
    shortId: conflict.shortId, title: conflict.title, mine: conflict.mine.summary, theirs: conflict.theirs.summary,
    author: conflict.theirs.author, peer: conflict.theirs.label, url: conflict.theirs.url, reasons: conflict.reasons,
  };
}

/** What the person's conflict hook command added for the agent, when it answered. */
const hookNoteOf = (conflict: Conflict): string | null => (conflict.hook?.ok === true && conflict.hook.note?.trim() ? conflict.hook.note : null);

/** The mark of the hook's note in its latest answer, when it gave one: given once per conflict, event and session. */
const hookMark = (conflict: Conflict): string[] =>
  (hookNoteOf(conflict) === null ? [] : [`${conflictMark(conflict)}hook:${conflict.hook!.event}@${conflict.hook!.at}`]);

/** A checkpoint's mark: once per conflict. */
const checkpointMark = (conflict: Conflict): string => `${conflictMark(conflict)}checkpoint`;

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
  return conflicts.filter(({ conflict }) => conflict.level === "ambient" && !waits(conflict, `${conflictMark(conflict)}open`, taken, options))
    .flatMap(entry => surfaceNote(entry, () => {
      const { conflict, files } = entry;
      return conflictNote({ file: files[0] ?? conflict.title, peer: conflict.theirs.label, author: conflict.theirs.author, summary: conflict.theirs.summary, pullRequest: conflict.theirs.label.startsWith("PR #") });
    }, `${conflictMark(entry.conflict)}open`, options));
}

/**
 * One card per checkpoint or stop conflict the session was not handed: through the agent in chat modes; in terminal
 * mode the person's line carries it, with the status line.
 */
function checkpointNotes(conflicts: readonly StoredConflict[], taken: (mark: string) => boolean, options: PendingNoteOptions): PendingNote[] {
  return conflicts.filter(({ conflict }) => (conflict.level === "checkpoint" || conflict.level === "stop") && !waits(conflict, checkpointMark(conflict), taken, options))
    .flatMap(entry => surfaceNote(entry, () => checkpointNote(cardOf(entry.conflict), entry.files), checkpointMark(entry.conflict), options));
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
 * The notes for `sessionId` in the checkout that it has not been handed, in the order ambient conflicts, checkpoints,
 * the hook's late notes; notes whose marks are in `inFlight` (queued for a delivery that has not taken them yet) are
 * left out. Reads only: {@link markNotesDelivered} records a note once it is handed over.
 */
export function pendingNotes(db: StateQueries, sessionId: string, checkoutPath: string, inFlight: ReadonlySet<string>, options: PendingNoteOptions): PendingNote[] {
  const given = new Set(readState(db, STATE_KEYS.delivered(sessionId), deliveredSchema) ?? []);
  const taken = (mark: string): boolean => given.has(mark) || inFlight.has(mark);
  const conflicts = surfacedConflicts(db, checkoutPath);
  return [
    ...ambientNotes(conflicts, taken, options),
    ...checkpointNotes(conflicts, taken, options),
    ...hookFollowUps(conflicts, taken, options),
  ];
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
function stopAsk({ conflict, files }: StoredConflict): string {
  const contested = files.length === 1 ? `${files[0]} is` : `${files.join(", ")} are`;
  return `Rerail: ${contested} contested with ${sideName(conflict.theirs)}: ${conflict.title}. Allow this edit, or deny it and talk to ${conflict.theirs.author ?? `the author of ${conflict.theirs.label}`} first.`;
}

/**
 * The stop notes for a tool call about to edit contested files: one per open stop conflict whose files it edits, once
 * per session, in every delivery mode. Where the host can ask the person (`asks`), the first stop the
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
    .filter(entry => entry.conflict.status === "open" && entry.conflict.level === "stop"
      && !taken(`${conflictMark(entry.conflict)}stop`) && paths.some(path => entry.files.includes(path)))
    .map(entry => {
      const { conflict, files } = entry;
      const ask = asks && !asked;
      asked ||= ask;
      const notice = noticeOf(entry, options);
      const card = stopNote(cardOf(conflict), files, ask);
      return {
        content: viaChat(options.delivery) ? hookNote(card, hookNoteOf(conflict)) : card,
        marks: [`${conflictMark(conflict)}stop`, checkpointMark(conflict), ...(viaChat(options.delivery) ? hookMark(conflict) : [])],
        conflict: ask ? { ...notice, ask: stopAsk(entry) } : notice,
      };
    });
}

/** What the footer shows of the conflicts in a session's checkout (docs/in-session-surfaces.md, "What the footer means"). */
export interface ConflictFooter {
  /** Open conflicts above logged: the newest one's line, and `(+N)` for the rest. */
  readonly conflicts: number;
  /** The loudest open checkpoint or stop conflict: `◉ decide: <title>`. */
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
  const loudest = open.map(entry => entry.conflict).filter(conflict => conflict.level === "checkpoint" || conflict.level === "stop")
    .sort((left, right) => Number(right.level === "stop") - Number(left.level === "stop") || left.detectedAt.localeCompare(right.detectedAt))[0];
  return {
    conflicts: open.length,
    decide: loudest === undefined ? null : { id: loudest.id, title: loudest.title },
    summary: { id: newest.conflict.id, text: statusText(newest, now) },
  };
}

/**
 * Record that `marks` were handed to the session. Only marks of conflicts still in the checkout are kept: a conflict
 * that is found again after it went away is news again.
 */
export function markNotesDelivered(db: RerailDatabase, sessionId: string, checkoutPath: string, marks: readonly string[], now: number): void {
  if (marks.length === 0) return;
  db.transaction(tx => {
    const key = STATE_KEYS.delivered(sessionId);
    const given = readState(tx, key, deliveredSchema) ?? [];
    const current = readConflicts(tx, checkoutPath).map(entry => conflictMark(entry.conflict));
    writeState(tx, key, [...new Set([...given, ...marks])].filter(entry => current.some(version => entry.startsWith(version))), now);
  }, { behavior: "immediate" });
}
