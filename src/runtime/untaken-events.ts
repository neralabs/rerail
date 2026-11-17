import { mkdirSync, readdirSync, readFileSync, rmSync, watch, type FSWatcher } from "node:fs";
import { join } from "node:path";
import { replaceFile } from "../engine/file-replace.js";

/**
 * The Rerail home's untaken events: events whose hook found the home's daemon running but not reading. The first hook
 * of such a silence waits its whole bounded wait for the daemon, then leaves its event here; while an event left for
 * that daemon is still here, the daemon has not run its event loop since, so a later hook leaves its event behind it at
 * once instead of waiting again. Each event is one file, written whole (`replaceFile`), named
 * `<arrival ms>-<daemon pid>-<hook pid>-<count>.json`: the order hooks met their events, and whose silence it was.
 * The daemon takes the folder in that order whenever its event loop runs (a watch of the folder, its start, and before
 * any event sent to it), so no event sent after a silence is recorded before one left during it. The home is private
 * to its owner, and a process that can write here can read `daemon.json`'s capability and send events itself, so the
 * folder lets in no one the engine socket keeps out.
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

/** Whether `home` holds an event a hook left for the daemon `daemonPid` that no daemon has taken. */
export function untakenFor(home: string, daemonPid: number): boolean {
  return untakenNames(home).some(entry => entry.order[1] === daemonPid);
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
  /** Takes the folder whenever it changes, until `close`; false where no watch can start. */
  watch(): boolean;
  close(): void;
}

/**
 * The taker of `home`'s untaken events for its daemon: `record` stores one, synchronously, and reports its own
 * failures. A take runs to its end on the daemon's one thread, so an event sent to the daemon while it takes waits
 * behind every event left before it. A file Windows keeps from removal is removed at a later take and not recorded
 * twice.
 */
export function untakenEventTaker(home: string, record: (event: UntakenEvent) => void): UntakenEventTaker {
  const recorded = new Set<string>();
  let watcher: FSWatcher | undefined;
  const take = (): void => {
    for (const { name, order } of untakenNames(home)) {
      const path = join(untakenFolder(home), name);
      if (!recorded.has(name)) recordFile(path, order[0]!, record);
      recorded.add(name);
      if (removed(path)) recorded.delete(name);
    }
  };
  return {
    take,
    watch: () => {
      watcher = watchFolder(home, take);
      return watcher !== undefined;
    },
    close: () => watcher?.close(),
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

/** A watch of `home`'s untaken folder that calls `changed` on each change; undefined where none can start. */
function watchFolder(home: string, changed: () => void): FSWatcher | undefined {
  try {
    mkdirSync(untakenFolder(home), { recursive: true, mode: 0o700 });
    const watcher = watch(untakenFolder(home), { persistent: false }, changed);
    watcher.on("error", () => watcher.close());
    return watcher;
  } catch {
    return undefined;
  }
}
