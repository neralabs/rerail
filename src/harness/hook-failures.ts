import { randomUUID } from "node:crypto";
import { appendFileSync, closeSync, constants, fstatSync, linkSync, lstatSync, mkdirSync, openSync, readdirSync, readFileSync, readSync, statSync, unlinkSync, type BigIntStats, type Stats } from "node:fs";
import { dirname, join } from "node:path";
import { renameWithRetrySync } from "../engine/file-replace.js";
import { LOCKS_DIR } from "../engine/native/file-lock.js";
import { loadNativeBinding, type NativeExclusiveFileLock } from "../engine/native/native.js";
import { openNoFollowSync } from "../engine/no-follow.js";
import { PACKAGE_ROOT } from "../engine/package-root.js";
import { processState } from "../engine/process-liveness.js";
import { defaultRerailHome } from "../engine/storage/rerail-home.js";
import { makeRerailHome } from "../engine/storage/private-files.js";

/**
 * In the Rerail home: one JSON line per hook event Rerail could not record, so `rerail status` can say that hooks run
 * but nothing is recorded, and why. A hook's own stderr reaches no one: coding agents keep it out of the session.
 */
export const HOOK_FAILURES_FILE = "hook-failures.jsonl";

/** Past this size the file is moved to `<name>.1`, replacing the one before; a failing hook writes on every event. */
const MAX_FAILURE_BYTES = 256 * 1024;

/** Where the full log moves, replacing the one before: the older part of the log. */
const MOVED_ASIDE_FILE = `${HOOK_FAILURES_FILE}.1`;

export interface HookFailure {
  /** Epoch ms. */
  at: number;
  /** The coding agent, as Rerail names its hosts (`claude-code`, `codex`, …). */
  host: string;
  /** The agent's own name for the event, such as `MessageDisplay`. */
  event: string;
  /** The agent and its version as it tells its hooks (Claude Code's `AI_AGENT`), when it does. */
  agent: string | null;
  /** The Rerail version that ran the hook. */
  rerail: string;
  reason: string;
}

function rerailVersion(): string {
  try {
    const manifest = JSON.parse(readFileSync(join(PACKAGE_ROOT, "package.json"), "utf8")) as { version?: unknown };
    return typeof manifest.version === "string" ? manifest.version : "unknown";
  } catch {
    return "unknown";
  }
}

export interface HookFailureContext {
  home?: string;
  /** The failure's time, epoch ms; when not given, the clock read once the log has made room for it. */
  now?: number;
  env?: NodeJS.ProcessEnv;
}

/**
 * The line a hook that moved the full log aside writes into the log after it, with its own failure: every failure
 * stamped from `startedAt` (epoch ms) on is in that part of the log or a later one, so `rerail status` can say from when
 * it counts once the part before is gone. Readers that know only failures skip it, as it has no event.
 */
interface PartStart {
  startedAt: number;
}

/**
 * How far past its own clock a hook that moved the log aside says the next part began ({@link PartStart}): one tick of
 * the system clock and a millisecond. On Windows Node's clock is the system time a process read when it started, which
 * Windows moves on once per timer tick (15.625 ms apart at most), plus a fine counter since then, so two hook processes'
 * clocks can differ by up to one tick; elsewhere they read one clock. One margin on every system keeps one rule that
 * every system's tests run: it moves the time the line names 16 ms later, and the count still holds every failure since.
 */
const PART_START_PAST_CLOCK_MS = 16 + 1;

/** Appends one failure. Never throws: a hook must not break the session over its own bookkeeping. */
export function recordHookFailure(
  failure: Pick<HookFailure, "host" | "event" | "reason">,
  { home = defaultRerailHome(), now, env = process.env }: HookFailureContext = {},
): void {
  try {
    makeRerailHome(home);
    const path = join(home, HOOK_FAILURES_FILE);
    const { moved, into } = placeFor(path, join(home, MOVED_ASIDE_FILE));
    const rerail = rerailVersion();
    // Read after the move. Every failure in the part this hook moved was stamped before the hook took it, by the clock
    // of the hook that wrote it, which can run up to one clock step ahead of this one: past this clock by that and a
    // millisecond is later than all of them. The hook's own failure carries the same time, so it counts in its part.
    const at = now ?? Date.now() + (moved ? PART_START_PAST_CLOCK_MS : 0);
    const line = `${JSON.stringify({ at, ...failure, agent: env.AI_AGENT ?? null, rerail } satisfies HookFailure)}\n`;
    writeInto(into, path, moved ? `${JSON.stringify({ startedAt: at } satisfies PartStart)}\n${line}` : line);
    if (moved) removeStrandedLogs(home, at);
  } catch {
    // Nowhere left to say it. A full log a person linked at the live name that cannot move to `.1` ends here too, with
    // nothing written through the link, as on `main` ({@link moveLink}).
  }
}

function sizeOf(path: string): number {
  try {
    return statSync(path).size;
  } catch {
    return 0;
  }
}

/**
 * Where a hook writes its failure: `into`, the live log it opened without following a link, or {@link BY_NAME}; and
 * whether it moved the log aside first, `moved`.
 */
interface Place {
  moved: boolean;
  into: number | typeof BY_NAME;
}

/** The live log by its name, following a link there, as `main`'s hook appends after its size check and its move. */
const BY_NAME = "by name";

/**
 * Where a hook writes its failure ({@link Place}) once it has moved the log at the live name `path` aside to `movedAside`
 * when its size check, which follows a link as `main`'s does, found it full.
 */
function placeFor(path: string, movedAside: string): Place {
  if (sizeOf(path) <= MAX_FAILURE_BYTES) return { moved: false, into: BY_NAME };
  if (moveAside(path, movedAside)) return { moved: true, into: BY_NAME };
  return placeAfterFull(path, movedAside);
}

/**
 * Where a hook that found the log full and did not move it writes: as when another hook took it first, or `.1` refused
 * it and the hook gave it back. `main`'s hook in that state saw its rename to `.1` fail and writes nothing; this one
 * keeps its failure in the log it opens at the live name `path`, without following a link. A link there by then is one
 * a person made since the size check, or one another hook took and put back ({@link putBackLink}): `main`'s rename,
 * which comes after its size check, moves whatever it finds at the live name, so this hook moves the link to
 * `movedAside` as `main` does ({@link moveLink}) and writes into a new log. It never writes through the link: when that
 * move fails, as `main`'s does, it writes nothing.
 */
function placeAfterFull(path: string, movedAside: string): Place {
  const into = openLive(path);
  if (into !== BY_NAME) return { moved: false, into };
  return { moved: moveLink(path, movedAside), into: BY_NAME };
}

/**
 * The live log at `path`, opened to append without following a link, and begun when there is none; {@link BY_NAME} when
 * the name is a link. On Windows the open looks at the name first and refuses a file other than the one it looked at
 * ({@link openNoFollowSync}), as when another hook moved the log on and began a new one in that moment: the open is
 * then tried again, up to {@link OPEN_TRIES} times in all.
 */
function openLive(path: string, tries = OPEN_TRIES): number | typeof BY_NAME {
  try {
    return openNoFollowSync(path, constants.O_WRONLY | constants.O_APPEND | constants.O_CREAT, 0o600);
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code !== "ELOOP") throw error;
    if (isLink(path)) return BY_NAME;
    if (tries <= 1) throw error;
    return openLive(path, tries - 1);
  }
}

/** How many times a hook opens the live log before it gives up ({@link openLive}). */
const OPEN_TRIES = 4;

/** Appends `text` to the live log at `path` where `into` says ({@link Place}), and closes what the hook opened. */
function writeInto(into: Place["into"], path: string, text: string): void {
  if (into === BY_NAME) {
    appendFileSync(path, text, { mode: 0o600 });
    return;
  }
  try {
    appendFileSync(into, text);
  } finally {
    closeSync(into);
  }
}

/**
 * A log a hook took to move aside keeps the hook's own name, `<name>.<pid>.<uuid>.old`, until the hook moves it on or
 * gives it back. The uninstall knows the name as a move cut short, `readHookFailures` counts what such a file holds,
 * and `removeStrandedLogs` removes one its hook left.
 */
const takenName = (path: string): string => `${path}.${process.pid}.${randomUUID()}.old`;
const TAKEN_LOG = new RegExp(`^${HOOK_FAILURES_FILE.replaceAll(".", "\\.")}\\.(\\d+)\\.[0-9a-f-]+\\.old$`, "u");

/**
 * Moves the full log at `path` to `<name>.1`, and says whether this hook did. Hooks that fail at once each find the log
 * full, and a rename straight to `.1` made the later ones fail on a log already moved, losing their own line, or move
 * the new log another hook had just begun over the full one. So a hook first takes the log under a name of its own, and
 * moves it to `.1` only when what it took is full; a hook that took the new log another hook began gives it back. A
 * hook stopped between those steps, as a coding agent stops a hook past its time limit, leaves the log under its own
 * name, where `readHookFailures` still counts it.
 *
 * On Windows a rename opens the file by name, then renames what it opened: a hook that opened the log before this one
 * took it still moves it on, out from under this hook's name. So each step gives up once the file is gone from that
 * name, leaving it to the hook that holds it last.
 *
 * A live name that is a link, as a person can make it to keep the log elsewhere, is never taken: it moves as `main` moves
 * the log ({@link moveLink}), and throws where `main`'s move fails. A link a person makes there while this hook moves
 * the log it found full and does not move it on is met when the hook opens the live log ({@link placeAfterFull}).
 */
function moveAside(path: string, movedAside: string): boolean {
  return isLink(path) ? moveLink(path, movedAside) : moveFile(path, movedAside);
}

/** Moves the regular file at the live name `path` on to `movedAside` through a name of this hook's own, as {@link moveAside} says. */
function moveFile(path: string, movedAside: string): boolean {
  const taken = takenName(path);
  try {
    renameWithRetrySync(path, taken);
  } catch {
    // Moved first by another hook, or held through the retries: this hook took nothing.
    return false;
  }
  return moveTaken(taken, path, movedAside);
}

/**
 * Moves the link a person made at the live name `path` to `movedAside` as `main` moves the log: the link itself, renamed
 * there right after the size check with the retries of `main`'s one rename. When that rename fails, as when `.1` is a
 * folder or another program holds it past the retries, this throws, so the hook writes nothing and the link stays at
 * the live name, as on `main`. Given back and written through, the link would take the hook's failure, and the coding
 * agent's event with it, to whatever it leads to: a file under a write lease, or a share that stopped answering, holds
 * the hook until the agent stops it. The size check of the live name, as `main`'s, is the one call that follows the
 * link. Hooks that find a linked log full at once fare as `main`'s do: one moves the link, and each other finds no live
 * name and writes nothing, or moves the log the first one began over the link at `.1`, where status counts that log.
 * Status reads no link at `.1` ({@link listLog}), so from the move on the line counts the failures since.
 */
function moveLink(path: string, movedAside: string): true {
  renameWithRetrySync(path, movedAside);
  return true;
}

/** Whether the name `path` itself is a link (a junction, on Windows); false when it is gone. */
function isLink(path: string): boolean {
  try {
    return lstatSync(path).isSymbolicLink();
  } catch {
    // Gone, as when another hook took it: the take finds nothing either.
    return false;
  }
}

/**
 * Moves the log this hook took as `taken` from the live name `path` on to `movedAside` when it is full, and gives it
 * back otherwise. A link there is one a person made at the live name between this hook's look and its take: it moves
 * as `main` moves it ({@link moveTakenLink}), so a give-back holds only a regular file. For a regular file `lstat` gives
 * what `stat` gives, in the one call.
 */
function moveTaken(taken: string, path: string, movedAside: string): boolean {
  let stats: Stats;
  try {
    stats = lstatSync(taken);
  } catch {
    // Moved on by another hook, which moves it aside or gives it back.
    return false;
  }
  if (stats.isSymbolicLink()) return moveTakenLink(taken, path, movedAside);
  try {
    // No wait for a `.1` another program holds: the log goes back at once and a later hook moves it, so it stays under
    // this hook's name for the moment of one rename, not for the 2 s of retries.
    if (stats.size > MAX_FAILURE_BYTES) {
      renameWithRetrySync(taken, movedAside, { retryMs: 0 });
      return true;
    }
  } catch (error) {
    // Moved on by another hook, which moves it aside. Any other failure leaves `.1` held: the log goes back.
    if ((error as NodeJS.ErrnoException).code === "ENOENT") return false;
  }
  giveBack(taken, path);
  return false;
}

/**
 * Moves the link a person made at the live name `path`, which this hook took as `taken`, on to `movedAside` by one
 * rename, as `main`'s rename right after its size check moves it from the live name. A log another hook began at the
 * free live name since stays there, and this hook writes into it. When `.1` refuses that rename (a folder there, or on
 * Windows a `.1` another program holds), the link goes back to the live name ({@link putBackLink}) and moves as `main`
 * moves it, with `main`'s retries ({@link moveLink}): so it waits at the live name, where status reads it, as on `main`.
 * When it cannot go back, this throws: the hook writes nothing, and the link stays under the hook's name, where no hook
 * reads or removes it ({@link removeStrandedLogs}).
 */
function moveTakenLink(taken: string, path: string, movedAside: string): true {
  try {
    renameWithRetrySync(taken, movedAside, { retryMs: 0 });
    return true;
  } catch {
    // `.1` refused it: the link goes back and moves as `main` moves it.
  }
  putBackLink(taken, path);
  return moveLink(path, movedAside);
}

/**
 * Puts the link this hook took as `taken` back at the live name `path`, keeping a log another hook began there while this
 * hook held the link. That hook found no live name and wrote its failure into that log; `main`'s hook finds the link
 * there and, when the file it leads to has room, appends through it, and status counts that failure. The link first
 * gets the live name as a second name, which never replaces a name there ({@link linkedBack}). When a log is there, the
 * log gets a name of this hook's own, where status counts it and a hook that opened it still writes into it, and the
 * link then replaces only the log's live name, by one rename: the live name is never free in between, so no other log
 * begins there. On macOS a second name of a link names the file it leads to, so there the link goes back by that rename
 * alone, and a log begun between the two steps is replaced.
 */
function putBackLink(taken: string, path: string): void {
  if (LINK_NAMES_A_LINK && linkedBack(taken, path)) return;
  keepUnderOwnName(path);
  renameWithRetrySync(taken, path, { retryMs: 0 });
}

/**
 * Whether `fs.linkSync` gives a link a second name of its own: Linux's `link` does not follow a link, nor does Windows'
 * `CreateHardLinkW`; macOS's `link` follows it to the file it leads to.
 */
const LINK_NAMES_A_LINK = process.platform !== "darwin";

/**
 * Gives the link taken as `taken` the live name `path` as a second name, then takes this hook's name off it, and says
 * whether it did. False when a name is at `path`, or the link cannot get a second name there.
 */
function linkedBack(taken: string, path: string): boolean {
  try {
    linkSync(taken, path);
  } catch {
    // A log another hook began is there, or the file system makes no hard links: the link goes back by a rename.
    return false;
  }
  try {
    unlinkSync(taken);
  } catch {
    // The link keeps this hook's name too, where no hook reads or removes it ({@link removeStrandedLogs}).
  }
  return true;
}

/**
 * Gives the log at the live name `path`, which another hook began, a name of this hook's own ({@link takenName}), where
 * status counts it and a later move removes it once it has not changed for a day ({@link removeStrandedLogs}).
 */
function keepUnderOwnName(path: string): void {
  try {
    linkSync(path, takenName(path));
  } catch {
    // No log there; or it cannot get a second name, as when another program holds it on Windows, where the rename back
    // over it fails too.
  }
}

/**
 * Throws {@link LinkInTheWay} when the name `path` is a link: a give-back holds only a regular file a hook took
 * ({@link moveTaken}). A link here would have the give-back read a person's file through it, copy it and remove it. The
 * hook then writes nothing.
 */
function assertNoLink(path: string): void {
  if (isLink(path)) throw new LinkInTheWay(`${path} is a link, which a hook never gives a log back through`);
}

/** A link where a give-back would read or write ({@link assertNoLink}): it ends the hook's record. */
class LinkInTheWay extends Error {}

/**
 * Puts the log taken as `taken` back at the live name `path`. A hook that opened the log to append before this one took
 * it writes into that file whenever it is next scheduled, so the file itself goes back, under a second name and then
 * under that name alone. When another hook has begun a live log since the take, or the file system has no hard links,
 * its lines go back instead.
 */
function giveBack(taken: string, path: string): void {
  assertNoLink(taken);
  try {
    linkSync(taken, path);
  } catch (error) {
    // Gone from its name: another hook moved it on, and gives it back or moves it aside.
    if ((error as NodeJS.ErrnoException).code !== "ENOENT") copyBack(taken, path);
    return;
  }
  try {
    unlinkSync(taken);
  } catch {
    // Moved on by another hook, which moves it aside or, finding it is the live log, removes only its own name. Refused
    // for another reason: status counts the file once under both names, a hook that takes the live name and gives its
    // lines back by copy leaves them here (`heldElsewhere`), and a later move removes this name.
  }
}

/**
 * Appends the lines of the log taken as `taken` to the live log at `path`, and removes it. The live name is a link only
 * when a person made it one while this hook held the log: the lines then stay under the hook's name, where status
 * counts them, and the hook moves the link as `main`'s rename would ({@link placeAfterFull}).
 */
function copyBack(taken: string, path: string): void {
  assertNoLink(taken);
  let descriptor: number;
  try {
    descriptor = openSync(taken, "r");
  } catch {
    return;
  }
  const lock = copyLock(dirname(path));
  try {
    copyLines(descriptor, taken, path);
  } catch {
    // Gone from its name: another hook moved it on, and gives it back or moves it aside. A read or removal refused for
    // any other reason leaves the lines under this hook's name, where status still counts them until a later move
    // removes the file. An append that fails after the removal loses the lines, as the same failure loses this hook's
    // own line, and a second read that fails loses those written since the first.
  } finally {
    if (lock !== null) lock.release();
    closeSync(descriptor);
  }
}

/**
 * The lock a give-back by copy holds from the removal of its own name to the end of the copy: the OS lock on
 * `locks/hook-failures.lock` in the Rerail home, which the system frees when its holder ends, however it ends. On
 * Windows a rename opens the file by name and then renames what it opened, so two hooks can hold one log under their
 * own names at once. Each removes its name and copies only when no other name holds the file; without the lock both
 * can remove theirs before either looks, and both copy. Null when the engine that takes the lock does not load, or
 * another hook holds it past `COPY_LOCK_WAIT_MS`: the copy then goes ahead without it.
 */
function copyLock(home: string): NativeExclusiveFileLock | null {
  try {
    const path = join(home, LOCKS_DIR, COPY_LOCK_FILE);
    mkdirSync(dirname(path), { recursive: true, mode: 0o700 });
    const engine = loadNativeBinding();
    return firstOf(() => engine.tryLockExclusive(path), Date.now() + COPY_LOCK_WAIT_MS);
  } catch {
    // No engine, or no lock file: as when the wait runs out.
    return null;
  }
}

const COPY_LOCK_FILE = "hook-failures.lock";

/** A copy holds its lock for one read and one append; a hook that waits past this copies without it. */
const COPY_LOCK_WAIT_MS = 1_000;

const PAUSE = new Int32Array(new SharedArrayBuffer(4));

/** What `take` gives, tried every 5 ms until it gives one or the clock passes `deadline` (epoch ms); null then. */
function firstOf<T>(take: () => T | null, deadline: number): T | null {
  for (;;) {
    const taken = take();
    if (taken !== null || Date.now() >= deadline) return taken;
    Atomics.wait(PAUSE, 0, 0, 5);
  }
}

/**
 * {@link copyBack}, through `descriptor`, the log taken as `taken` opened for reading, under {@link copyLock}. One
 * window is left: a hook that writes into the log after the second read below loses its line.
 */
function copyLines(descriptor: number, taken: string, path: string): void {
  // Read while the log still has its name: a read that fails leaves the lines there, where status counts them.
  const id = fileId(fstatSync(descriptor, { bigint: true }), taken);
  const lines = readFrom(descriptor, 0);
  // Looked at under the lock, right before the lines leave this hook's name: a person can make the live name a link
  // while this hook waits for the lock or reads. The lines then stay under this hook's name.
  if (isLink(path)) return;
  // Gone from its name before the lines go back, so a hook that moves it on finds nothing to give back twice.
  unlinkSync(taken);
  // Still under another name, as when another hook gave it back while this one moved it on, or a hook's removal of its
  // own name was refused: the lines are there, and status counts them under that name. Copied as well, they would count
  // twice.
  if (heldElsewhere(id, taken, path)) return;
  appendToLive(path, lines);
  // A hook that opened the log to append before this one took it can write after the read above: that goes back too.
  // One that writes later still, after this read, loses its line.
  const later = readFrom(descriptor, lines.length);
  if (later.length > 0) appendToLive(path, later);
}

/**
 * Appends `text` to the live log at `path`, never through a link there: a person can make the live name one after the
 * give-back looked at it, as it removes its own name and lists the others. The open then fails and the lines are lost,
 * as when any append after the removal fails, and none reaches the person's file. Where the system has `O_NOFOLLOW` the
 * kernel refuses the link; on Windows the name is looked at before the open, and the file opened is checked to be the
 * one looked at ({@link openNoFollowSync}).
 */
function appendToLive(path: string, text: Buffer): void {
  const descriptor = openNoFollowSync(path, constants.O_WRONLY | constants.O_APPEND | constants.O_CREAT, 0o600);
  try {
    appendFileSync(descriptor, text);
  } finally {
    closeSync(descriptor);
  }
}

/**
 * Whether a name of the log other than `taken` holds the file whose {@link fileId} is `id`: the live log at `path`, `.1`,
 * or a log another hook took. Another hook can move that name on while this one looks, as it takes the live log: when a
 * name the listing found is gone by the time it is looked at, the files of the log are listed again, as a status read
 * does ({@link readLogParts}). On a file system with no file numbers each name counts as a file of its own, so none does.
 */
function heldElsewhere(id: string, taken: string, path: string): boolean {
  const home = dirname(path);
  for (let listing = 0; listing < HELD_ELSEWHERE_LISTINGS; listing += 1) {
    const names = listLog(home);
    if ([...names].some(([name, other]) => other === id && join(home, name) !== taken)) return true;
    if (![...names.values()].includes(null)) return false;
  }
  return false;
}

/**
 * How many times a hook lists the log's names as it looks for another name of a log it gives back: once, and up to 3
 * more with no pause, as it holds the copy lock and its hook's event.
 */
const HELD_ELSEWHERE_LISTINGS = 4;

/**
 * The {@link fileId} of the name `name` itself, never followed; undefined when it is missing or cannot be read. A link a
 * person made at the live name, which a status read could not open, is not followed a second time: on a network share
 * that stopped answering each call through it waits, where `main` gives up after the open.
 */
function fileIdAt(name: string): string | undefined {
  try {
    return fileId(lstatSync(name, { bigint: true }), name);
  } catch {
    return undefined;
  }
}

/** The bytes of the open file `descriptor` from `start` to its end. */
function readFrom(descriptor: number, start: number): Buffer {
  const bytes = Buffer.alloc(Math.max(0, fstatSync(descriptor).size - start));
  return bytes.length === 0 ? bytes : bytes.subarray(0, readSync(descriptor, bytes, 0, bytes.length, start));
}

/**
 * An id that stays the same under every name a file moves to: its device and file number. A file system with no file
 * numbers gives 0, and the file's name `name` stands in, so each name counts as a file of its own.
 */
function fileId({ dev, ino }: BigIntStats, name: string): string {
  return ino === 0n ? name : `${dev}:${ino}`;
}

/**
 * Removes each log a hook left under its own name that no hook has changed for the period `rerail status` counts, as of
 * `now` (epoch ms): no hook moves such a log on, so without this the Rerail home would keep it for good. A take, a move
 * on, a give-back and a write each set the file's change time, and no hook runs for a day, so the hook that took such a
 * log has ended, whatever process has its pid now. A hook held up since it opened the log to append opened it by the
 * live name, which the file lost at a change, and read its clock before that: a failure it writes into the file after the
 * removal is older than the period status counts, as is every failure the file holds. Only a regular file is removed,
 * the one kind a hook leaves under its name. A link there, as a person's link a hook took in the moment between its
 * look and its take and was stopped holding, stays where it is, as on `main` the link stays at the live name: no hook
 * reads through it or removes it. A hook that moves the log aside calls this, once per 256 KB of failures.
 */
function removeStrandedLogs(home: string, now: number): void {
  for (const name of takenLogs(home)) {
    const path = join(home, name);
    try {
      // Read from the name itself, never through a link: following one to a network share that does not answer waits
      // about 21 s on Windows.
      const stats = lstatSync(path);
      if (stats.isFile() && lastChangeAt(stats) < now - HOOK_FAILURE_WINDOW_MS) unlinkSync(path);
    } catch {
      // Moved on or removed by another hook, or refused: a later move tries again.
    }
  }
}

/**
 * When the file `stats` describes last changed, epoch ms: the later of its change time and its write time, so that a
 * file system that keeps no change time, as FAT does not, still gives the last write.
 */
function lastChangeAt({ ctimeMs, mtimeMs }: Stats): number {
  return Math.max(ctimeMs, mtimeMs);
}

/** How far back `rerail status` looks for hook failures. */
export const HOOK_FAILURE_WINDOW_MS = 24 * 60 * 60 * 1000;

const plural = (count: number, one: string, many: string): string => `${count} ${count === 1 ? one : many}`;

/** What the log holds of the failures since a given time. */
export interface HookFailureLog {
  /** Those failures, oldest first. */
  failures: HookFailure[];
  /**
   * When the log was moved aside and keeps nothing as old as the given time, failures from before the part in `.1`
   * began may be gone: the time it began, epoch ms, from which the failures are counted. Null when the log reaches back
   * to the given time.
   */
  keptSince: number | null;
}

const iso = (at: number): string => new Date(at).toISOString();

/** The period a log's failures cover ({@link HookFailureLog.keptSince}), as the line names it. */
const periodOf = (keptSince: number | null): string =>
  keptSince === null ? "in the last 24 hours" : `since ${iso(keptSince)}, as far back as the log goes`;

/**
 * What `rerail status` says of the log's failures from the last `HOOK_FAILURE_WINDOW_MS`, given when the newest
 * recorded event was observed (`lastRecordedAt`, epoch ms; null when nothing was ever recorded). When hooks failed
 * after the last recorded event, hooks run but nothing is recorded, and the line leads with that.
 */
export function hookFailureLines({ failures, keptSince }: HookFailureLog, lastRecordedAt: number | null): string[] {
  const last = failures.at(-1);
  if (last === undefined) return [];
  const unrecorded = lastRecordedAt === null ? failures : failures.filter(failure => failure.at > lastRecordedAt);
  const from = last.agent ?? last.host;
  const detail = `the last, ${last.event} from ${from} with Rerail ${last.rerail} at ${iso(last.at)}: ${last.reason}`;
  const first = unrecorded[0];
  if (first === undefined) {
    return [`Hooks: ${plural(failures.length, "hook event", "hook events")} could not be recorded ${periodOf(keptSince)}; ${detail}.`];
  }
  return [`${unrecordedSince(first, unrecorded.length, { total: failures.length, keptSince })}; ${detail}.`];
}

/**
 * The lead of the line when `count` hooks failed after the last recorded event, the first of them `first`: nothing was
 * recorded since that one. When failures from before the last recorded event add to them, the count of the whole log
 * follows, `total`, with its period. When every failure the log holds came after it and older ones may be gone, hooks
 * may have failed before `first`, and the lead says so.
 */
function unrecordedSince(first: HookFailure, count: number, { total, keptSince }: { total: number; keptSince: number | null }): string {
  const reach = count === total && keptSince !== null ? ", as far back as the log goes" : "";
  const lead = `Hooks run but nothing was recorded since ${iso(first.at)}${reach}: ${plural(count, "hook event", "hook events")} failed`;
  return count === total ? lead : `${lead}, of ${total} that could not be recorded ${periodOf(keptSince)}`;
}

/**
 * The failures recorded since `since` (epoch ms), in every part of the log: the live part, the full logs hooks took to
 * move aside and hold under their own names, and the part moved aside to `.1`. A file missing or unreadable holds none.
 */
export function readHookFailures(home: string, since: number): HookFailureLog {
  const all = readLogParts(home);
  const keptSince = keptSinceOf(all, since);
  const from = keptSince ?? since;
  // Hooks take the time before they append, and a hook gives back the lines of a log it took after other hooks wrote
  // theirs, so lines from hooks running at once can be a little out of order.
  const failures = all.flatMap(part => part.failures.filter(failure => failure.at >= from)).sort((left, right) => left.at - right.at);
  return { failures, keptSince };
}

/**
 * The pauses, in ms, before each listing a status read makes after its first while files of the log move under it,
 * spent only while hooks keep moving the log: 255 ms in all, and on Windows about 360 ms, where each pause lasts at least
 * one tick of the system timer (15.6 ms by default).
 */
const RELISTING_PAUSES_MS = [1, 2, 4, 8, 16, 32, 64, 128];

/**
 * Every file of the log in the Rerail home `home`, read once each by its {@link fileId}.
 *
 * Hooks move the log by renames, which keep the file: from the live name to a hook's own and on to `.1`, and back to the
 * live name when a hook gives it back. A read takes the live log first, so a log a hook takes from there while the read
 * goes on was met already. Then it lists the files of the log, takes each one's file id and reads each file it has not
 * met. When a file moved under it (a name gone by the time it looked or opened it, another file at the live name or
 * `.1`), it pauses and lists them again, until a listing finds nothing moved. A file it cannot read for another reason,
 * as one another program holds on Windows, counts as holding nothing ({@link readLogFile}). On Windows a rename opens
 * the file by name and then renames what it opened, so every hook that opened the full log by name as it filled moves
 * it on in turn, from one hook's name to the next and from `.1` to a hook's name, until the last of them moves it to
 * `.1`: a burst of hooks keeps the log moving for many milliseconds, faster than a listing and its reads, and the pauses
 * let that end. After the last pause the read stands: a log still moving then is missed by that status, and a status
 * that no move overlaps counts it.
 *
 * A give-back by copy moves lines, not a file. Between the removal of the taken name and the append to the live log the
 * lines are under no name, so a read in that moment misses them; a read that met the taken log, then a live log it had
 * not met that already holds the copy, counts them twice. Either is limited to the lines of the log being given back,
 * for that status: a new log a hook took for full, which grows for as long as that hook is held up between its size
 * check and its take, up to most of a part.
 */
function readLogParts(home: string): LogPart[] {
  const parts = new Map<string, LogPart>();
  readPart(home, HOOK_FAILURES_FILE, parts);
  let moved = readUnmet(home, parts);
  for (const ms of RELISTING_PAUSES_MS) {
    if (!moved) break;
    Atomics.wait(PAUSE, 0, 0, ms);
    moved = readUnmet(home, parts);
  }
  return [...parts.values()];
}

/**
 * A file of the log: the live log's or `.1`'s name when a read last found it there, else the name it was read under,
 * or {@link MOVED_ON} once another file is found under the name it had.
 */
interface LogPart extends LogText {
  name: string;
}

/** The name of a part of the log that another file has since taken. */
const MOVED_ON = "";

/** Reads the file of the log named `name` in `home` into `parts`, under its file id; nothing when it is gone or unreadable. */
function readPart(home: string, name: string, parts: Map<string, LogPart>): void {
  const part = readLogFile(join(home, name));
  // A file met before keeps its name: only a listing moves it.
  if (part !== null) parts.set(part.id, { ...part.log, name: parts.get(part.id)?.name ?? name });
}

/**
 * Lists the files of the log in `home`, reads each one `parts` has not met, and says whether a file moved under it: a
 * name gone by the time it looked or opened it, another file under the live name or `.1` than the one last found there,
 * or no file known as `.1` while a part says a hook moved one there and a hook that still runs holds a log it took a
 * moment ago.
 */
function readUnmet(home: string, parts: Map<string, LogPart>): boolean {
  const names = listLog(home);
  const missed = readNew(home, names, parts);
  const moved = nameParts(parts, names);
  return missed || moved || awaitingMovedAside(home, parts);
}

/** The files of the log a listing found, by name: each one's file id, or null when it was gone before it was looked at. */
type LogNames = Map<string, string | null>;

/**
 * The files of the log in `home`, in the order a hook moves a log through them: the live log, the logs hooks took, `.1`.
 * Only a regular file is listed; any other name is left out, never followed or opened. Rerail makes no other kind: the
 * second name a give-back gives a file is a hard link, which is a regular file. A link (a junction, on Windows) to a
 * network share that does not answer, followed, waits about 21 s on Windows, and a link whose target is gone, taken for
 * a name gone before it was looked at, would make every status read wait out all its pauses for as long as the link
 * stays. A named pipe, opened to read, waits for a program to write to it, and a folder at `.1`, read as a part that
 * holds nothing, would make the line count from the live part's start though no part was dropped. A status read opens
 * the live log before it lists, so a link at the live name is still read through, as `main` reads it, until a hook
 * moves it to `.1` as the link ({@link moveLink}).
 */
function listLog(home: string): LogNames {
  let listed: string[];
  try {
    listed = readdirSync(home);
  } catch {
    return new Map();
  }
  const order = [HOOK_FAILURES_FILE, ...listed.filter(name => TAKEN_LOG.test(name)), MOVED_ASIDE_FILE];
  const names: LogNames = new Map();
  for (const name of order.filter(each => listed.includes(each))) {
    const id = listedId(join(home, name));
    if (id !== undefined) names.set(name, id);
  }
  return names;
}

/**
 * The {@link fileId} of the file a listing found at `path`, read from the name itself: null when the name was gone before
 * it was looked at, and undefined when it is not a regular file (a link, a folder, a named pipe or a device), which is
 * neither followed nor opened. For a regular file `lstat` gives what `stat` gives, in the one call.
 */
function listedId(path: string): string | null | undefined {
  let stats: BigIntStats;
  try {
    stats = lstatSync(path, { bigint: true });
  } catch {
    return null;
  }
  return stats.isFile() ? fileId(stats, path) : undefined;
}

/** Reads each file `names` holds that `parts` has not met, and says whether one of them is still unmet or a name was gone. */
function readNew(home: string, names: LogNames, parts: Map<string, LogPart>): boolean {
  let missed = false;
  for (const [name, id] of names) {
    if (id !== null && !parts.has(id)) readPart(home, name, parts);
    if (id === null || !parts.has(id)) missed = true;
  }
  return missed;
}

/**
 * Gives the live log's name and `.1` to the parts `names` found under them, and says whether either moved. With no file
 * under one of them, the part last found there keeps it: a hook on Windows can hold `.1` under its own name for a moment.
 */
function nameParts(parts: ReadonlyMap<string, LogPart>, names: LogNames): boolean {
  let moved = false;
  for (const name of [HOOK_FAILURES_FILE, MOVED_ASIDE_FILE]) {
    const id = names.get(name);
    if (typeof id === "string" && giveName(parts, name, id)) moved = true;
  }
  return moved;
}

/** Gives `name` to the part with file id `id`, taking it from any other part, and says whether it moved. */
function giveName(parts: ReadonlyMap<string, LogPart>, name: string, id: string): boolean {
  let moved = false;
  for (const [partId, part] of parts) {
    const next = partId === id ? name : part.name === name ? MOVED_ON : part.name;
    if (next !== part.name) moved = true;
    part.name = next;
  }
  return moved;
}

/**
 * Whether no file of the log is known as `.1` though a part holds a start ({@link PartStart}), which a hook writes only
 * once it has moved a full log to `.1`, where a log stays until the next replaces it, and a hook that still runs holds a
 * log in `home` under its own name that it took a moment ago: a hook on Windows moves `.1` on to its own name for a
 * moment, and a read that stopped there would say it counts the last 24 hours though the part before was dropped. A hook
 * that has ended moves nothing, so a log it left is not waited for, nor is a log unchanged for longer than
 * {@link TAKEN_LOG_MOVING_MS}, whose pid another process may have taken since its hook was stopped.
 */
function awaitingMovedAside(home: string, parts: ReadonlyMap<string, LogPart>): boolean {
  const all = [...parts.values()];
  return !all.some(part => part.name === MOVED_ASIDE_FILE) && all.some(part => part.name !== MOVED_ON && part.startedAt !== undefined)
    && all.some(part => heldByRunningHook(home, part.name));
}

/**
 * How long status waits for a log a hook took, past the moment it last changed. A take renames the file, which sets its
 * change time as a write does, and a hook that holds `.1` under its own name moves it on after one stat and one rename,
 * which the pauses wait out. A log unchanged for longer is one a stopped hook left: Windows hands a freed pid to another
 * process soon, and without this bound every status would wait out all the pauses for as long as that process runs.
 */
const TAKEN_LOG_MOVING_MS = 2_000;

/** Whether `name` is a log a hook took ({@link TAKEN_LOG}) in `home`, that hook still runs, and the log changed lately. */
function heldByRunningHook(home: string, name: string): boolean {
  const pid = TAKEN_LOG.exec(name)?.[1];
  return pid !== undefined && processState(Number(pid)) !== "gone" && changedLately(join(home, name));
}

/** Whether the file at `path` changed in the last {@link TAKEN_LOG_MOVING_MS}, or is gone from that name. */
function changedLately(path: string): boolean {
  try {
    return Date.now() - statSync(path).ctimeMs < TAKEN_LOG_MOVING_MS;
  } catch {
    // Gone since the read met it, as a hook moved it on to its own name, or refused: a log that moves.
    return true;
  }
}

/**
 * How far back the log goes ({@link HookFailureLog.keptSince}), given its files and the start of the period, `since`.
 * The part in `.1` began when the hook that moved the part before it aside wrote its start there: every failure from
 * then on is in `.1` or a later part. A `.1` with no start, moved by an earlier build or by a hook stopped before it
 * wrote one, goes back to the oldest failure it or the live part keeps, which hooks failing as that part began can
 * straddle; so does a `.1` whose own start a hook held up between its move and its write, for as long as a part takes
 * to fill, wrote into a later part. A log a hook was stopped holding can be older than `.1`, with dropped logs between
 * them, so it counts from that time on but never sets it.
 */
function keptSinceOf(parts: readonly LogPart[], since: number): number | null {
  const movedAside = parts.find(part => part.name === MOVED_ASIDE_FILE);
  if (movedAside === undefined) return null;
  const start = startOf(firstParts(movedAside, parts));
  return start !== undefined && start >= since ? start : null;
}

/**
 * The parts the period begins with: `.1` and the live part. A `.1` that holds nothing, as one another program keeps from
 * being read, adds nothing to the count, so the period begins with the live part.
 */
function firstParts(movedAside: LogPart, parts: readonly LogPart[]): LogPart[] {
  const live = parts.filter(part => part.name === HOOK_FAILURES_FILE);
  return movedAside.failures.length === 0 && movedAside.startedAt === undefined ? live : [movedAside, ...live];
}

/** When the first of `parts` began: the start written in it, else the oldest failure they hold; undefined with neither. */
function startOf(parts: readonly LogPart[]): number | undefined {
  return parts[0]?.startedAt ?? oldestOf(parts);
}

/** The time of the oldest failure the parts hold, epoch ms; undefined when they hold none. */
function oldestOf(parts: readonly LogPart[]): number | undefined {
  let oldest: number | undefined;
  for (const { failures } of parts) {
    for (const { at } of failures) if (oldest === undefined || at < oldest) oldest = at;
  }
  return oldest;
}

/** The names of the logs hooks took to move aside, in the Rerail home `home`; none when it cannot be listed. */
function takenLogs(home: string): string[] {
  try {
    return readdirSync(home).filter(name => TAKEN_LOG.test(name));
  } catch {
    return [];
  }
}

/**
 * What the file at `path` holds, with its {@link fileId}; null when it is missing. A file there that cannot be read, as
 * one another program holds on Windows, holds nothing: a status read counts without it and does not wait for it to move.
 */
function readLogFile(path: string): { id: string; log: LogText } | null {
  let descriptor: number;
  try {
    descriptor = openSync(path, "r");
  } catch (error) {
    return (error as NodeJS.ErrnoException).code === "ENOENT" ? null : unreadLog(path);
  }
  try {
    return { id: fileId(fstatSync(descriptor, { bigint: true }), path), log: parseLog(readFileSync(descriptor, "utf8")) };
  } catch {
    return unreadLog(path);
  } finally {
    closeSync(descriptor);
  }
}

/** The file at `path` as holding nothing, under its {@link fileId}; null when it is missing. */
function unreadLog(path: string): { id: string; log: LogText } | null {
  const id = fileIdAt(path);
  return id === undefined ? null : { id, log: { failures: [], startedAt: undefined } };
}

/** What a file of the log holds: its failures, and the latest start of a part written in it ({@link PartStart}). */
interface LogText {
  failures: HookFailure[];
  startedAt: number | undefined;
}

/**
 * A log file's failures and part start, from its text, skipping lines that are neither. A file holds more than one
 * start only when a hook that moved the log aside wrote its start after the next part had filled and moved on; the
 * latest is the part's own.
 */
function parseLog(text: string): LogText {
  const log: LogText = { failures: [], startedAt: undefined };
  for (const line of text.split("\n")) {
    const value = parseLine(line);
    if (value === null) continue;
    if ("startedAt" in value) log.startedAt = Math.max(value.startedAt, log.startedAt ?? value.startedAt);
    else log.failures.push(value);
  }
  return log;
}

/** One line of a log file: a failure or a part's start; null when it is neither. */
function parseLine(line: string): HookFailure | PartStart | null {
  if (line.trim() === "") return null;
  try {
    // Parsed from the log's own text; each field is checked before it is used.
    const value = JSON.parse(line) as Partial<Record<keyof (HookFailure & PartStart), unknown>>;
    return typeof value.startedAt === "number" ? { startedAt: value.startedAt } : failureOf(value);
  } catch {
    // A line cut short by a concurrent write.
    return null;
  }
}

/** The failure a parsed line holds, with the optional fields it lacks or holds as other than text filled in; null when it holds none. */
function failureOf(value: Partial<Record<keyof HookFailure, unknown>>): HookFailure | null {
  if (typeof value.at !== "number" || typeof value.event !== "string" || typeof value.reason !== "string") return null;
  return { at: value.at, host: textOr(value.host, "unknown"), event: value.event, agent: textOr(value.agent, null), rerail: textOr(value.rerail, "unknown"), reason: value.reason };
}

/** `value` when it is text, else `otherwise`. */
function textOr<T>(value: unknown, otherwise: T): string | T {
  return typeof value === "string" ? value : otherwise;
}
