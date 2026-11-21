import { randomUUID } from "node:crypto";
import { closeSync, constants, fstatSync, lstatSync, mkdirSync, readdirSync, readFileSync, rmSync, statSync, watch, writeFileSync, type FSWatcher, type Stats } from "node:fs";
import { join } from "node:path";
import { setImmediate as nextTurn } from "node:timers/promises";
import { replaceFile } from "../file-replace.js";
import { openNoFollowSync } from "../no-follow.js";

/**
 * The Rerail home's untaken events: events whose hook found the home's daemon running but not reading. The first hook
 * of such a silence waits its whole bounded wait for the daemon, then leaves its event here; while an event left for
 * that daemon is still untaken, the daemon has not run its event loop since, so a later hook leaves its event behind it
 * at once instead of waiting again. Each event is one file, written whole (`replaceFile`), named
 * `<arrival ms>-<daemon pid>-<hook pid>-<count>-<bytes>.json`: the order hooks met their events, whose silence it was,
 * and its size. The daemon takes the folder in that order whenever its event loop runs (a watch of the folder, its
 * start, and before any event sent to it), so no event sent after a silence is checked before one left during it. As it
 * begins to take, it marks the newest event it takes (`TAKEN_MARK`), so a hook counts as untaken only what came after:
 * events the daemon is taking, a file another program keeps it from removing, or one it cannot read, never make a hook
 * skip a daemon that reads. Its state file names the folder it watches (`untakenFolderId`), so a hook skips its wait only for events in
 * that very folder. The home is private to its owner, and a process that can write here can read `daemon.json`'s
 * capability and send events itself, so the folder lets in no one the engine socket keeps out.
 */
export const UNTAKEN_EVENTS_DIR = "untaken-events";

/** In the folder: the name of the newest event the daemon has taken, which hooks count nothing up to. */
const TAKEN_MARK = "taken";

/** In the folder: the mark a daemon writes as its watch opens on it, which is the folder's id (`untakenFolderId`). */
export const UNTAKEN_WATCH_MARK = "watched";

/**
 * The most events, and the most bytes of them, the folder holds: a hook that finds it full loses its event, as a hook
 * that finds no daemon does. A daemon that reads again checks each left event as it checks one sent to it, so the bound
 * keeps the work a resume brings to what the daemon checks in a few seconds, and a resume never turns into a second
 * silence. An event is at most an engine request's frame.
 */
export const UNTAKEN_MOST_EVENTS = 256;
export const UNTAKEN_MOST_BYTES = 16 * 1024 * 1024;

const UNTAKEN = /^(\d{13})-(\d+)-(\d+)-(\d+)-(\d+)\.json$/u;

/** How many events this process has left, so two left in one millisecond get two names. */
let left = 0;

function untakenFolder(home: string): string {
  return join(home, UNTAKEN_EVENTS_DIR);
}

/** An untaken event's name, read back: when its hook met it, the daemon it was left for, the hook, its count there. */
interface UntakenName {
  name: string;
  order: readonly number[];
  bytes: number;
}

function parsedName(name: string): UntakenName | undefined {
  const match = UNTAKEN.exec(name);
  return match === null ? undefined : { name, order: match.slice(1, 5).map(Number), bytes: Number(match[5]) };
}

/** First by arrival, then by daemon, hook and count: the order the hooks met their events. */
function earlier(a: Pick<UntakenName, "order">, b: Pick<UntakenName, "order">): number {
  const differs = a.order.findIndex((value, index) => value !== b.order[index]);
  return differs === -1 ? 0 : a.order[differs]! - b.order[differs]!;
}

/** The untaken events in `home`, oldest first; none when the folder is missing or cannot be read. */
function untakenNames(home: string): UntakenName[] {
  let names: string[];
  try {
    names = readdirSync(untakenFolder(home));
  } catch {
    return [];
  }
  return names.map(parsedName).filter(name => name !== undefined).sort(earlier);
}

/** The newest event the daemon has taken from `home`'s folder; undefined when it has taken none there. */
function takenMark(home: string): UntakenName | undefined {
  try {
    return parsedName(readFileSync(join(untakenFolder(home), TAKEN_MARK), "utf8").trim());
  } catch {
    return undefined;
  }
}

/** The events of `home` no daemon has begun to take: those after the taken mark. */
function waitingNames(home: string): UntakenName[] {
  const names = untakenNames(home);
  const mark = names.length === 0 ? undefined : takenMark(home);
  return mark === undefined ? names : names.filter(name => earlier(name, mark) > 0);
}

/**
 * A hook found the folder holding its bound of events (`UNTAKEN_MOST_EVENTS`, `UNTAKEN_MOST_BYTES`): its event is lost.
 * The message names the bound it met.
 */
export class UntakenFolderFullError extends Error {}

const MIB = 1024 * 1024;
const size = (bytes: number): string => (bytes < MIB ? `${Math.ceil(bytes / 1024)} KiB` : `${(bytes / MIB).toFixed(1)} MiB`);

/** Why the folder refuses an event of `bytes` while it holds `held` events of `heldBytes`; undefined while it takes one. */
function folderRefusal(held: number, heldBytes: number, bytes: number): string | undefined {
  if (held >= UNTAKEN_MOST_EVENTS) return `${held} events already wait for it, the most Rerail keeps`;
  if (heldBytes + bytes <= UNTAKEN_MOST_BYTES) return undefined;
  const waiting = held === 1 ? "the 1 event that already waits for it holds" : `the ${held} events that already wait for it hold`;
  return `${waiting} ${size(heldBytes)}, and this one's ${size(bytes)} would pass the ${UNTAKEN_MOST_BYTES / MIB} MiB Rerail keeps`;
}

/**
 * Leaves `body` (an engine request, `engineRequestBody`) in `home` for the daemon `daemonPid`, which runs but did not
 * read, under the time its hook met the event (`arrivedAt`, epoch ms). Throws when it cannot be written, or when the
 * folder holds its bound already (`UntakenFolderFullError`), so the caller counts the event lost.
 */
export async function leaveUntaken(home: string, daemonPid: number, body: string, arrivedAt: number): Promise<void> {
  const bytes = Buffer.byteLength(body);
  const held = untakenNames(home);
  const heldBytes = held.reduce((sum, name) => sum + name.bytes, 0);
  const refusal = folderRefusal(held.length, heldBytes, bytes);
  if (refusal !== undefined) throw new UntakenFolderFullError(refusal);
  mkdirSync(untakenFolder(home), { recursive: true, mode: 0o700 });
  left += 1;
  const name = `${String(arrivedAt).padStart(13, "0")}-${daemonPid}-${process.pid}-${left}-${bytes}.json`;
  await replaceFile(join(untakenFolder(home), name), body, { mode: 0o600 });
}

/**
 * The identity of the untaken folder at `home` as it stands, which a daemon names in its state file while it watches
 * that folder: the mark the daemon wrote in it as its watch opened (`UNTAKEN_WATCH_MARK`); undefined when there is no
 * folder, or no daemon marked the one there. A daemon whose watch saw its folder go writes nothing to say so
 * (`watchFolder`), and a folder made again there, by a hook or anyone, holds no mark until a daemon watches it, so no
 * hook takes it for the one that went. Linux gives such a folder the removed one's file id at once, and its birth time
 * within the same clock tick, so neither can tell them apart.
 */
export function untakenFolderId(home: string): string | undefined {
  try {
    return readFileSync(join(untakenFolder(home), UNTAKEN_WATCH_MARK), "utf8") || undefined;
  } catch {
    return undefined;
  }
}

/** Whether a folder is at `home`'s untaken path, watched or not. */
function folderThere(home: string): boolean {
  try {
    return statSync(untakenFolder(home)).isDirectory();
  } catch {
    return false;
  }
}

/**
 * Whether `home` holds an event a hook left for the daemon `daemonPid` that no daemon has begun to take, in the folder
 * `folderId` names: the one that daemon watches. A folder removed, moved or made again since is another folder.
 */
export function untakenFor(home: string, daemonPid: number, folderId: string): boolean {
  return untakenFolderId(home) === folderId && waitingNames(home).some(entry => entry.order[1] === daemonPid);
}

/** What waits in `home` for the daemon `daemonPid`: how many events, and when the first of them came (epoch ms). */
export interface UntakenWaiting {
  events: number;
  since: number;
}

/** The events left in `home` for the daemon `daemonPid` that it has not begun to take; undefined when there are none. */
export function untakenWaiting(home: string, daemonPid: number): UntakenWaiting | undefined {
  const waiting = waitingNames(home).filter(entry => entry.order[1] === daemonPid);
  const first = waiting[0];
  return first === undefined ? undefined : { events: waiting.length, since: first.order[0]! };
}

/** One event a hook left: its engine request (`engineRequestBody`), and when its hook met it (epoch ms). */
export interface UntakenEvent {
  body: string;
  arrivedAt: number;
}

/** One pass of a taker over the folder: how many events it handed on, the first and last arrival, and how many failed. */
export interface UntakenPass {
  events: number;
  from: number;
  to: number;
  failed: number;
}

/** Where a taker hands the events it takes: `take` begins one's check and resolves when it ends; `passed` hears of each pass. */
export interface UntakenHandler {
  take(event: UntakenEvent): Promise<void>;
  passed(pass: UntakenPass): void;
}

/** A daemon's taker of its home's untaken events. */
export interface UntakenEventTaker {
  /**
   * Hands every event the folder holds to `take`, oldest first, a slice per turn of the event loop, each removed once
   * handed on, and resolves once every event the folder held at the call is handed on. One pass runs at a time: a call
   * during one waits for it and for the pass after it, which takes what came since. Takes nothing after `close`.
   */
  taken(): Promise<void>;
  /**
   * Makes the folder, starts a pass whenever it changes, until `close`, and returns the id of the folder it watches
   * (`untakenFolderId`); undefined where no watch can start. A folder removed, moved or replaced is followed to the
   * folder at its path only when one is there, never one the watch makes, since a watch cannot tell a folder removed
   * alone from a home being removed: when none is there, the watch ends and writes nothing, and `taken` follows the
   * folder the next hook makes. `watching` gets the id of each folder it follows to, and undefined once the watch failed
   * where the folder still is; events are taken at `taken` either way.
   */
  watch(watching: (folderId: string | undefined) => void): string | undefined;
  close(): void;
}

/** The taker where no hook leaves events (an engine service no daemon's home names): there is nothing to take. */
export const NO_UNTAKEN_EVENTS: UntakenEventTaker = { taken: async () => undefined, watch: () => undefined, close: () => undefined };

/** How many events a pass hands on before it lets the event loop run. */
const PASS_SLICE = 16;

/**
 * How many passes may fail to read an event's file (another program holds it, no descriptor is free) before it is lost.
 */
const READ_TRIES = 3;

/**
 * The taker of `home`'s untaken events for its daemon, which hands each to `handler`. Each pass first moves the taken
 * mark on, so hooks stop counting what it takes, then hands the events on a slice at a time: the daemon answers between
 * slices. Each name is handed on once: a file Windows keeps from removal is removed at a later pass and not handed on
 * twice. A file the daemon cannot read now is read again at the next pass, until `READ_TRIES` passes could not; one
 * that is no event a hook wrote (a folder, a link, a pipe, a size other than its name's) fails in the pass that met it.
 * Each failure goes to stderr (daemon.log) with the event's name. A pass writes into the folder only for names newer
 * than its taken mark, so a pass its own writes start, or one that reads such a file again, writes nothing.
 */
export function untakenEventTaker(home: string, handler: UntakenHandler): UntakenEventTaker {
  const taker = new Taker(home, handler);
  let watcher: FolderWatch | undefined;
  return {
    taken: () => {
      watcher?.refollow();
      return taker.taken();
    },
    watch: watching => {
      watcher = watchFolder(home, () => void taker.taken(), watching);
      return watcher?.folderId();
    },
    close: () => {
      taker.closed = true;
      watcher?.close();
    },
  };
}

/** The passes of one taker: at most one runs, and calls meanwhile share the one after it. */
class Taker {
  closed = false;
  /**
   * Events this taker is done with, handed on or failed, whose file is still there (Windows holds it, or it is no file):
   * removed at a later pass where it can be, never handed on or failed again.
   */
  readonly #done = new Set<string>();
  /** By name, how many passes could not read an event's file, which stays for the next. */
  readonly #unread = new Map<string, number>();
  #running: Promise<void> | undefined;
  #next: Promise<void> | undefined;

  constructor(readonly home: string, readonly handler: UntakenHandler) {}

  taken(): Promise<void> {
    if (this.#running !== undefined) {
      // After the running pass however it ends: one that failed tells its own caller, and the next takes what came since.
      this.#next ??= this.#running.catch(() => undefined).then(() => {
        this.#next = undefined;
        return this.taken();
      });
      return this.#next;
    }
    this.#running = this.#pass().finally(() => {
      this.#running = undefined;
    });
    return this.#running;
  }

  async #pass(): Promise<void> {
    if (this.closed) return;
    for (const name of [...this.#done]) removedName(this.home, name, this.#done);
    const fresh = untakenNames(this.home).filter(name => !this.#done.has(name.name));
    if (fresh.length === 0) return;
    await markTaken(this.home, fresh.at(-1)!);
    void reportPass(await this.#handOnInSlices(fresh), this.handler);
  }

  /** Hands `names` on a slice per turn of the event loop, and returns those handed on, each with its check. */
  async #handOnInSlices(names: readonly UntakenName[]): Promise<HandedOn[]> {
    const handed: HandedOn[] = [];
    for (let start = 0; start < names.length && !this.closed; start += PASS_SLICE) {
      for (const name of names.slice(start, start + PASS_SLICE)) {
        const check = this.#handOn(name);
        // Each check's end is heard at once, so one that fails while later slices are handed on is no unhandled
        // rejection.
        if (check !== undefined) handed.push({ name, passed: check.then(() => true, (error: unknown) => toldFailed(name, error)) });
      }
      await nextTurn();
    }
    return handed;
  }

  /**
   * Hands the event `name` holds to the handler, removes its file, and returns its check, failed where the file is no
   * event or could not be read `READ_TRIES` times; undefined, the file left for the next pass, while fewer passes could
   * not read it. The taker is done with a name it hands on (`#done`), so it is met once.
   */
  #handOn(name: UntakenName): Promise<void> | undefined {
    const read = this.#read(name);
    if (read === undefined) return undefined;
    // Begun before its file goes, in the folder's order: a daemon checks a session's events in the order it begins
    // them.
    const check = read instanceof Error ? Promise.reject(read) : this.handler.take({ body: read, arrivedAt: name.order[0]! });
    this.#unread.delete(name.name);
    this.#done.add(name.name);
    removedName(this.home, name.name, this.#done);
    return check;
  }

  /** What `name`'s file holds, or why it fails; undefined, said on stderr, when the next pass reads it again. */
  #read(name: UntakenName): string | Error | undefined {
    try {
      return eventText(join(untakenFolder(this.home), name.name), name.bytes);
    } catch (thrown) {
      const error = thrown instanceof Error ? thrown : new Error(String(thrown));
      const tries = (this.#unread.get(name.name) ?? 0) + 1;
      if (error instanceof NotAnEventError || tries >= READ_TRIES) return error;
      this.#unread.set(name.name, tries);
      process.stderr.write(`rerail: the event a hook left in ${name.name} could not be read, and is read again at the next pass: ${error.message}\n`);
      return undefined;
    }
  }
}

/** An event a pass handed on, with its check: true once it passed. */
interface HandedOn {
  readonly name: UntakenName;
  readonly passed: Promise<boolean>;
}

/** Says on stderr which event's check failed, and why; false, for the pass's count. */
function toldFailed(name: UntakenName, error: unknown): false {
  process.stderr.write(`rerail: the event a hook left in ${name.name} could not be checked: ${error instanceof Error ? error.message : String(error)}\n`);
  return false;
}

/**
 * Moves the taken mark on to `newest`: it and every event before it are the daemon's now. A mark already at or past it
 * stays, since a file that stays in the folder under it (one Windows holds, one the daemon cannot read) must not count
 * for hooks again once an event that came before it is taken. Where it cannot be written, a hook still counts each of
 * those events until the pass removes its file.
 */
async function markTaken(home: string, newest: UntakenName): Promise<void> {
  const mark = takenMark(home);
  if (mark !== undefined && earlier(newest, mark) <= 0) return;
  await replaceFile(join(untakenFolder(home), TAKEN_MARK), newest.name, { mode: 0o600 }).catch(() => undefined);
}

/** What a hook never leaves under an event's name: a folder, a link, a pipe, a file of another size than its name's. */
class NotAnEventError extends Error {}

/**
 * The text of the event file at `path`, which a hook wrote whole: a regular file of the `bytes` its name says, within
 * the folder's bound (`eventShape`). It is looked at before it is opened, so a folder, a link or a pipe under its name
 * is never opened; the open follows no link (`openNoFollowSync`) and, where the system has the flag (not Windows), waits
 * for no pipe's writer; and what was opened is looked at again, so what is read is such a file, whatever was put under
 * the name since.
 */
function eventText(path: string, bytes: number): string {
  eventShape(lstatSync(path), bytes);
  const fd = openNoFollowSync(path, constants.O_RDONLY | constants.O_NONBLOCK);
  try {
    eventShape(fstatSync(fd), bytes);
    return readFileSync(fd, "utf8");
  } finally {
    closeSync(fd);
  }
}

/** Fails (`NotAnEventError`) unless `info` is of a regular file of the `bytes` an event's name says, within the bound. */
function eventShape(info: Stats, bytes: number): void {
  if (!info.isFile()) throw new NotAnEventError("it is not a file");
  if (info.size !== bytes || bytes > UNTAKEN_MOST_BYTES) throw new NotAnEventError(`it holds ${info.size} bytes, where its name says ${bytes}`);
}

/**
 * Tells `handler` of a pass once every check it began has ended, over the events it handed on. A close cuts a pass
 * short, and the names it did not hand on stay in the folder for the next daemon's taker, as does a file to be read
 * again; a pass that handed on none tells nothing.
 */
async function reportPass(handed: readonly HandedOn[], handler: UntakenHandler): Promise<void> {
  if (handed.length === 0) return;
  const failed = (await Promise.all(handed.map(event => event.passed))).filter(passed => !passed).length;
  try {
    handler.passed({ events: handed.length, from: handed[0]!.name.order[0]!, to: handed.at(-1)!.name.order[0]!, failed });
  } catch {
    // What the daemon says of a pass is its own; the events are checked whatever it could say.
  }
}

/**
 * Removes the event file `name`, forgetting the taker met it once it is gone; false while Windows holds it open, or
 * where it is a folder.
 */
function removedName(home: string, name: string, done: Set<string>): boolean {
  try {
    rmSync(join(untakenFolder(home), name), { force: true });
    done.delete(name);
    return true;
  } catch {
    return false;
  }
}

/**
 * A watch of one untaken folder: the folder's file id, which tells whether the folder at the path is still the one
 * watched, and the id the watch marked it with (`untakenFolderId`), which tells hooks. The mark goes when `rm -r`
 * empties the folder, before the folder itself, so it cannot tell the watch whether to follow: a follow then would mark
 * the folder again and keep it from going.
 */
interface Watched {
  watcher: FSWatcher;
  ino: bigint;
  id: string;
}

/**
 * What `watchFolder` gives back: the id of the folder it watches now, a follow of the folder at the path when it is not
 * the one watched (`refollow`), and the end of the watch.
 */
interface FolderWatch {
  folderId(): string | undefined;
  refollow(): void;
  close(): void;
}

/**
 * A watch of `home`'s untaken folder that calls `changed` on each change. A folder removed, moved or made again is no
 * longer the one watched (a watch follows what it opened), so a change that finds another folder there, or none, and an
 * error once the folder went, follow the folder at that path (`follow`). Once the watch errs on the folder it watches,
 * or cannot open on the one at the path, `watching` gets undefined and the watch ends. Undefined where none can start.
 */
function watchFolder(home: string, changed: () => void, watching: (folderId: string | undefined) => void): FolderWatch | undefined {
  let current: Watched | undefined;
  let closed = false;
  const folderId = (): string | undefined => current?.id;
  const end = (): void => {
    current?.watcher.close();
    current = undefined;
    if (!closed) watching(undefined);
  };
  /**
   * Opens a watch on the folder at the path and tells `watching` its id. With no folder there it makes none and tells
   * nothing: `rm -r` of the home removes the folder before the home, and a folder made, or a state file written, in
   * between keeps the home from going. The state file still names the folder that went, which no hook finds again.
   */
  const follow = (): void => {
    current?.watcher.close();
    current = undefined;
    if (closed) return;
    current = openWatch(home, onChange, failed);
    if (current !== undefined || folderThere(home)) watching(folderId());
  };
  // An error on the folder still at the path cannot be followed (a watch that fails where it is would fail again).
  const failed = (): void => {
    if (watchesFolder(home, current)) end();
    else follow();
  };
  // A change named for the folder itself (its removal or move; the name may be reused at once) or a folder that is not
  // the one watched any more: watch the folder at the path again.
  const onChange = (_: string, name: string | null): void => {
    if (name === null || name === UNTAKEN_EVENTS_DIR || !watchesFolder(home, current)) follow();
    changed();
  };
  makeFolder(home);
  current = openWatch(home, onChange, failed);
  if (current === undefined) return undefined;
  return {
    folderId,
    refollow: () => {
      if (!closed && !watchesFolder(home, current) && folderThere(home)) follow();
    },
    close: () => {
      closed = true;
      current?.watcher.close();
    },
  };
}

/**
 * Opens a watch of the untaken folder at `home`'s path, which it never makes, and marks the folder with a new id first,
 * so its own mark wakes no pass; undefined when there is none or either fails.
 */
function openWatch(home: string, changed: (event: string, name: string | null) => void, failed: () => void): Watched | undefined {
  try {
    if (!folderThere(home)) return undefined;
    const id = `${process.pid}-${randomUUID()}`;
    writeFileSync(join(untakenFolder(home), UNTAKEN_WATCH_MARK), id, { mode: 0o600 });
    const { ino } = statSync(untakenFolder(home), { bigint: true });
    const watcher = watch(untakenFolder(home), { persistent: false }, changed);
    watcher.on("error", failed);
    return { watcher, ino, id };
  } catch {
    return undefined;
  }
}

/**
 * Makes `home`'s untaken folder unless it is there, as the daemon's watch starts in the home it holds; neither the home
 * nor a missing folder is an error here: the watch then fails to open, and events are taken at `taken`.
 */
function makeFolder(home: string): void {
  try {
    mkdirSync(untakenFolder(home), { mode: 0o700 });
  } catch {
    // There already, or the home is gone.
  }
}

/** Whether the folder at `home`'s untaken path is still the one `watched` watches. */
function watchesFolder(home: string, watched: Watched | undefined): boolean {
  try {
    return watched !== undefined && statSync(untakenFolder(home), { bigint: true }).ino === watched.ino;
  } catch {
    return false;
  }
}
