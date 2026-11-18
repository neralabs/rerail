import { mkdirSync, readdirSync, readFileSync, rmSync, statSync, watch, type FSWatcher } from "node:fs";
import { join } from "node:path";
import { setImmediate as nextTurn } from "node:timers/promises";
import { replaceFile } from "./file-replace.js";

/**
 * The Rerail home's untaken events: events whose hook found the home's daemon running but not reading. The first hook
 * of such a silence waits its whole bounded wait for the daemon, then leaves its event here; while an event left for
 * that daemon is still untaken, the daemon has not run its event loop since, so a later hook leaves its event behind it
 * at once instead of waiting again. Each event is one file, written whole (`replaceFile`), named
 * `<arrival ms>-<daemon pid>-<hook pid>-<count>-<bytes>.json`: the order hooks met their events, whose silence it was,
 * and its size. The daemon takes the folder in that order whenever its event loop runs (a watch of the folder, its
 * start, and before any event sent to it), so no event sent after a silence is checked before one left during it. As it
 * begins to take, it marks the newest event it takes (`TAKEN_MARK`), so a hook counts as untaken only what came after:
 * events the daemon is taking, or a file another program keeps it from removing, never make a hook skip a daemon that
 * reads. Its state file names the folder it watches (`untakenFolderId`), so a hook skips its wait only for events in
 * that very folder. The home is private to its owner, and a process that can write here can read `daemon.json`'s
 * capability and send events itself, so the folder lets in no one the engine socket keeps out.
 */
export const UNTAKEN_EVENTS_DIR = "untaken-events";

/** In the folder: the name of the newest event the daemon has taken, which hooks count nothing up to. */
const TAKEN_MARK = "taken";

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

/** A hook found the folder holding its bound of events (`UNTAKEN_MOST_EVENTS`, `UNTAKEN_MOST_BYTES`): its event is lost. */
export class UntakenFolderFullError extends Error {
  constructor(readonly events: number) {
    super(`${events} events already wait for it, the most Rerail keeps`);
  }
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
  if (held.length >= UNTAKEN_MOST_EVENTS || heldBytes + bytes > UNTAKEN_MOST_BYTES) throw new UntakenFolderFullError(held.length);
  mkdirSync(untakenFolder(home), { recursive: true, mode: 0o700 });
  left += 1;
  const name = `${String(arrivedAt).padStart(13, "0")}-${daemonPid}-${process.pid}-${left}-${bytes}.json`;
  await replaceFile(join(untakenFolder(home), name), body, { mode: 0o600 });
}

/**
 * The identity of the untaken folder at `home` as it stands (its file id), which a daemon names in its state file while it
 * watches that folder; undefined when there is none.
 */
export function untakenFolderId(home: string): string | undefined {
  try {
    return String(statSync(untakenFolder(home), { bigint: true }).ino);
  } catch {
    return undefined;
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
   * Starts a pass whenever the folder changes, until `close`, following it when it is removed or replaced, and returns
   * the id of the folder it watches (`untakenFolderId`); undefined where no watch can start. `watching` gets the id of
   * each folder it follows to, and undefined once it can follow no more (the home is gone, or the watch failed where the
   * folder still is), after which events are taken only at `taken`.
   */
  watch(watching: (folderId: string | undefined) => void): string | undefined;
  close(): void;
}

/** The taker where no hook leaves events (an engine service no daemon's home names): there is nothing to take. */
export const NO_UNTAKEN_EVENTS: UntakenEventTaker = { taken: async () => undefined, watch: () => undefined, close: () => undefined };

/** How many events a pass hands on before it lets the event loop run. */
const PASS_SLICE = 16;

/**
 * The taker of `home`'s untaken events for its daemon, which hands each to `handler`. Each pass first writes the taken
 * mark, so hooks stop counting what it takes, then hands the events on a slice at a time: the daemon answers between
 * slices. A file Windows keeps from removal is removed at a later pass and not handed on twice.
 */
export function untakenEventTaker(home: string, handler: UntakenHandler): UntakenEventTaker {
  const taker = new Taker(home, handler);
  let watcher: FolderWatch | undefined;
  return {
    taken: () => taker.taken(),
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
  /** Events handed on whose file is still there (Windows holds it): removed at a later pass, never handed on again. */
  readonly #handed = new Set<string>();
  #running: Promise<void> | undefined;
  #next: Promise<void> | undefined;

  constructor(readonly home: string, readonly handler: UntakenHandler) {}

  taken(): Promise<void> {
    if (this.#running !== undefined) {
      this.#next ??= this.#running.then(() => {
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
    for (const name of [...this.#handed]) removedName(this.home, name, this.#handed);
    const fresh = untakenNames(this.home).filter(name => !this.#handed.has(name.name));
    if (fresh.length === 0) return;
    await markTaken(this.home, fresh.at(-1)!);
    const checks: Promise<boolean>[] = [];
    for (let start = 0; start < fresh.length && !this.closed; start += PASS_SLICE) {
      // Each check's end is heard at once, so one that fails while later slices are handed on is no unhandled rejection.
      for (const name of fresh.slice(start, start + PASS_SLICE)) checks.push(handOn(this.home, name, this.handler, this.#handed).then(() => true, () => false));
      await nextTurn();
    }
    void reportPass(fresh, checks, this.handler);
  }
}

/**
 * Writes the taken mark: `newest` and every event before it are the daemon's now. Where it cannot be written, a hook
 * still counts each of those events until the pass removes its file.
 */
async function markTaken(home: string, newest: UntakenName): Promise<void> {
  await replaceFile(join(untakenFolder(home), TAKEN_MARK), newest.name, { mode: 0o600 }).catch(() => undefined);
}

/**
 * Hands the event `name` holds to `handler`, removes its file, and returns its check, failed where the file cannot be
 * read (a take another daemon made first) or its check fails.
 */
function handOn(home: string, name: UntakenName, handler: UntakenHandler, handed: Set<string>): Promise<void> {
  const path = join(untakenFolder(home), name.name);
  let body: string;
  try {
    body = readFileSync(path, "utf8");
  } catch (error) {
    return Promise.reject(error instanceof Error ? error : new Error(String(error)));
  }
  // Begun before the file goes, in the folder's order: a daemon checks a session's events in the order it begins them.
  const check = handler.take({ body, arrivedAt: name.order[0]! });
  handed.add(name.name);
  removedName(home, name.name, handed);
  return check;
}

/** Tells `handler` of a pass over `names` once every check it began has ended (each true when it passed). */
async function reportPass(names: readonly UntakenName[], checks: readonly Promise<boolean>[], handler: UntakenHandler): Promise<void> {
  const ended = await Promise.all(checks);
  const failed = ended.filter(passed => !passed).length + names.length - checks.length;
  try {
    handler.passed({ events: names.length, from: names[0]!.order[0]!, to: names.at(-1)!.order[0]!, failed });
  } catch {
    // What the daemon says of a pass is its own; the events are checked whatever it could say.
  }
}

/** Removes the event file `name`, forgetting it was handed on once it is gone; false while Windows holds it open. */
function removedName(home: string, name: string, handed: Set<string>): boolean {
  try {
    rmSync(join(untakenFolder(home), name), { force: true });
    handed.delete(name);
    return true;
  } catch {
    return false;
  }
}

/** A watch of one untaken folder, and the identity of the folder it watches. */
interface Watched {
  watcher: FSWatcher;
  ino: bigint;
}

/** What `watchFolder` gives back: the id of the folder it watches now, and the end of the watch. */
interface FolderWatch {
  folderId(): string | undefined;
  close(): void;
}

/**
 * A watch of `home`'s untaken folder that calls `changed` on each change. A folder removed, moved or made again is no
 * longer the one watched (a watch follows what it opened), so a change that finds another folder there, or none, and an
 * error once the folder went, open a watch on the folder at that path again, making it in a home that still exists, and
 * tell `watching` its id. Once that fails, or the watch errs on the folder it watches, `watching` gets undefined and the
 * watch ends. Undefined where none can start.
 */
function watchFolder(home: string, changed: () => void, watching: (folderId: string | undefined) => void): FolderWatch | undefined {
  let current: Watched | undefined;
  let closed = false;
  const folderId = (): string | undefined => (current === undefined ? undefined : String(current.ino));
  const end = (): void => {
    current?.watcher.close();
    current = undefined;
    if (!closed) watching(undefined);
  };
  const follow = (): void => {
    current?.watcher.close();
    current = closed ? undefined : openWatch(home, onChange, failed);
    if (!closed) watching(folderId());
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
  current = openWatch(home, onChange, failed);
  if (current === undefined) return undefined;
  return {
    folderId,
    close: () => {
      closed = true;
      current?.watcher.close();
    },
  };
}

/** Opens a watch of `home`'s untaken folder, making the folder (not the home) first; undefined when either fails. */
function openWatch(home: string, changed: (event: string, name: string | null) => void, failed: () => void): Watched | undefined {
  try {
    makeFolder(home);
    const { ino } = statSync(untakenFolder(home), { bigint: true });
    const watcher = watch(untakenFolder(home), { persistent: false }, changed);
    watcher.on("error", failed);
    return { watcher, ino };
  } catch {
    return undefined;
  }
}

/** Makes `home`'s untaken folder unless it is there; throws when the home itself is gone. */
function makeFolder(home: string): void {
  try {
    mkdirSync(untakenFolder(home), { mode: 0o700 });
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code !== "EEXIST") throw error;
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
