import { mkdirSync, readdirSync, readFileSync, rmSync, statSync, watch, type FSWatcher } from "node:fs";
import { join } from "node:path";
import { setImmediate as nextTurn } from "node:timers/promises";
import { replaceFile } from "./file-replace.js";

/**
 * The Rerail home's untaken events: events whose hook found the home's daemon running but not reading. The first hook
 * of such a silence waits its whole bounded wait for the daemon, then leaves its event here; while an event left for
 * that daemon is still here, the daemon has not run its event loop since, so a later hook leaves its event behind it at
 * once instead of waiting again. Each event is one file, written whole (`replaceFile`), named
 * `<arrival ms>-<daemon pid>-<hook pid>-<count>.json`: the order hooks met their events, and whose silence it was.
 * The daemon takes the folder in that order whenever its event loop runs (a watch of the folder, its start, and before
 * any event sent to it), so no event sent after a silence is recorded before one left during it. Its state file names
 * the folder it watches (`untakenFolderId`), so a hook skips its wait only for events in that very folder. The home is
 * private to its owner, and a process that can write here can read `daemon.json`'s capability and send events itself,
 * so the folder lets in no one the engine socket keeps out.
 */
export const UNTAKEN_EVENTS_DIR = "untaken-events";

const UNTAKEN = /^(\d{13})-(\d+)-(\d+)-(\d+)\.json$/u;

/** How many events this process has left, so two left in one millisecond get two names. */
let left = 0;

function untakenFolder(home: string): string {
  return join(home, UNTAKEN_EVENTS_DIR);
}

/** An untaken event's name, read back: when its hook met it, the daemon it was left for, the hook, its count there. */
interface UntakenName {
  name: string;
  order: readonly number[];
}

function parsedName(name: string): UntakenName | undefined {
  const match = UNTAKEN.exec(name);
  return match === null ? undefined : { name, order: match.slice(1).map(Number) };
}

/** First by arrival, then by daemon, hook and count: the order the hooks met their events. */
function earlier(a: UntakenName, b: UntakenName): number {
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

/**
 * Leaves `body` (an engine request, `engineRequestBody`) in `home` for the daemon `daemonPid`, which runs but did not
 * read, under the time its hook met the event (`arrivedAt`, epoch ms). Throws when it cannot be written, so the caller
 * counts the event lost.
 */
export async function leaveUntaken(home: string, daemonPid: number, body: string, arrivedAt: number): Promise<void> {
  mkdirSync(untakenFolder(home), { recursive: true, mode: 0o700 });
  left += 1;
  const name = `${String(arrivedAt).padStart(13, "0")}-${daemonPid}-${process.pid}-${left}.json`;
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
 * Whether `home` holds an event a hook left for the daemon `daemonPid` that no daemon has taken, in the folder `folderId`
 * names: the one that daemon watches. A folder removed, moved or made again since is another folder.
 */
export function untakenFor(home: string, daemonPid: number, folderId: string): boolean {
  return untakenFolderId(home) === folderId && untakenNames(home).some(entry => entry.order[1] === daemonPid);
}

/** One event a hook left: its engine request (`engineRequestBody`), and when its hook met it (epoch ms). */
export interface UntakenEvent {
  body: string;
  arrivedAt: number;
}

/** A daemon's taker of its home's untaken events. */
export interface UntakenEventTaker {
  /** Records every event the folder holds, oldest first, removing each once it is recorded. */
  take(): void;
  /**
   * Takes every event the folder holds as `take` does, a slice per turn of the event loop, so a long backlog keeps the
   * daemon answering while it is taken; a `take` meanwhile takes the rest at once, in the same order. Ends at `close`.
   */
  drain(): Promise<void>;
  /**
   * Takes the folder whenever it changes, until `close`, following it when it is removed or replaced, and returns the id
   * of the folder it watches (`untakenFolderId`); undefined where no watch can start. `watching` gets the id of each
   * folder it follows to, and undefined once it can follow no more (the home is gone, or the watch failed where the
   * folder still is), after which events are taken only at `take` and `drain`.
   */
  watch(watching: (folderId: string | undefined) => void): string | undefined;
  close(): void;
}

/** How many events a slice of `drain` takes before it lets the event loop run. */
const DRAIN_SLICE = 64;

/**
 * The taker of `home`'s untaken events for its daemon: `record` stores one, synchronously, and reports its own
 * failures. A take runs to its end on the daemon's one thread, so an event sent to the daemon while it takes waits
 * behind every event left before it. A file Windows keeps from removal is removed at a later take and not recorded
 * twice.
 */
export function untakenEventTaker(home: string, record: (event: UntakenEvent) => void): UntakenEventTaker {
  const recorded = new Set<string>();
  let watcher: FolderWatch | undefined;
  let closed = false;
  const takeSome = (limit: number): boolean => {
    const names = untakenNames(home);
    for (const { name, order } of names.slice(0, limit)) {
      const path = join(untakenFolder(home), name);
      if (!recorded.has(name)) recordFile(path, order[0]!, record);
      recorded.add(name);
      if (removed(path)) recorded.delete(name);
    }
    return names.length > limit;
  };
  const take = (): void => void takeSome(Number.POSITIVE_INFINITY);
  return {
    take,
    drain: async () => {
      while (!closed && takeSome(DRAIN_SLICE)) await nextTurn();
    },
    watch: watching => {
      watcher = watchFolder(home, take, watching);
      return watcher?.folderId();
    },
    close: () => {
      closed = true;
      watcher?.close();
    },
  };
}

/** Hands the event in `path` to `record`; a file that is gone or cannot be read is a take another daemon made first. */
function recordFile(path: string, arrivedAt: number, record: (event: UntakenEvent) => void): void {
  let body: string;
  try {
    body = readFileSync(path, "utf8");
  } catch {
    return;
  }
  record({ body, arrivedAt });
}

/** Removes `path`; false while Windows holds it open. */
function removed(path: string): boolean {
  try {
    rmSync(path, { force: true });
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
