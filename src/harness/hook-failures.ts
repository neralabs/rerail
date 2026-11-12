import { randomUUID } from "node:crypto";
import { appendFileSync, closeSync, fstatSync, linkSync, openSync, readdirSync, readFileSync, readSync, statSync, unlinkSync, type BigIntStats } from "node:fs";
import { join } from "node:path";
import { renameWithRetrySync } from "../engine/file-replace.js";
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

/** Appends one failure. Never throws: a hook must not break the session over its own bookkeeping. */
export function recordHookFailure(
  failure: Pick<HookFailure, "host" | "event" | "reason">,
  { home = defaultRerailHome(), now, env = process.env }: HookFailureContext = {},
): void {
  try {
    makeRerailHome(home);
    const path = join(home, HOOK_FAILURES_FILE);
    const moved = sizeOf(path) > MAX_FAILURE_BYTES && moveAside(path, join(home, MOVED_ASIDE_FILE));
    const rerail = rerailVersion();
    // Read after the move. Every failure in the part this hook moved was stamped before the hook took it, so a
    // millisecond past the clock is later than all of them, even one stamped in the millisecond of the move.
    const at = now ?? Date.now() + (moved ? 1 : 0);
    const line = `${JSON.stringify({ at, ...failure, agent: env.AI_AGENT ?? null, rerail } satisfies HookFailure)}\n`;
    if (!moved) {
      appendFileSync(path, line, { mode: 0o600 });
      return;
    }
    appendFileSync(path, `${JSON.stringify({ startedAt: at } satisfies PartStart)}\n${line}`, { mode: 0o600 });
    removeStrandedLogs(home, at);
  } catch {
    // Nowhere left to say it.
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
 */
function moveAside(path: string, movedAside: string): boolean {
  const taken = takenName(path);
  try {
    renameWithRetrySync(path, taken);
  } catch {
    // Moved first by another hook, or held through the retries: this hook took nothing.
    return false;
  }
  try {
    // No wait for a `.1` another program holds: the log goes back at once and a later hook moves it, so it stays under
    // this hook's name for the moment of one rename, not for the 2 s of retries.
    if (sizeOf(taken) > MAX_FAILURE_BYTES) {
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
 * Puts the log taken as `taken` back at the live name `path`. A hook that opened the log to append before this one took
 * it writes into that file whenever it is next scheduled, so the file itself goes back, under a second name and then
 * under that name alone. When another hook has begun a live log since the take, or the file system has no hard links,
 * its lines go back instead.
 */
function giveBack(taken: string, path: string): void {
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
    // for another reason: status counts the file once under both names, and a later move removes this one.
  }
}

/** Appends the lines of the log taken as `taken` to the live log at `path`, and removes it. */
function copyBack(taken: string, path: string): void {
  let descriptor: number;
  try {
    descriptor = openSync(taken, "r");
  } catch {
    return;
  }
  try {
    copyLines(descriptor, taken, path);
  } catch {
    // Gone from its name: another hook moved it on, and gives it back or moves it aside. A read or removal refused for
    // any other reason leaves the lines under this hook's name, where status still counts them until a later move
    // removes the file. An append that fails after the removal loses the lines, as the same failure loses this hook's
    // own line.
  } finally {
    closeSync(descriptor);
  }
}

/** {@link copyBack}, through `descriptor`, the log taken as `taken` opened for reading. */
function copyLines(descriptor: number, taken: string, path: string): void {
  // Already the live log under a second name, as when another hook gave it back while this one moved it on: only this
  // name goes, as the lines are there.
  if (isLiveLog(descriptor, taken, path)) {
    unlinkSync(taken);
    return;
  }
  const lines = readFrom(descriptor, 0);
  // Gone from its name before the lines go back, so a hook that moves it on finds nothing to give back twice.
  unlinkSync(taken);
  appendFileSync(path, lines, { mode: 0o600 });
  // A hook that opened the log to append before this one took it can write after the read above: that goes back too.
  // One that writes later still, after this read, loses its line.
  const later = readFrom(descriptor, lines.length);
  if (later.length > 0) appendFileSync(path, later, { mode: 0o600 });
}

/** Whether the live log at `path` is the file open as `descriptor`, the log taken as `taken`. */
function isLiveLog(descriptor: number, taken: string, path: string): boolean {
  try {
    return fileId(statSync(path, { bigint: true }), path) === fileId(fstatSync(descriptor, { bigint: true }), taken);
  } catch {
    return false;
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
 * Removes each log a hook left under its own name, once that hook has ended and the file was last written before the
 * period `rerail status` counts, as of `now` (epoch ms): no hook moves such a log on, so without this the Rerail home
 * would keep it for good. A file stays while the hook named in it runs, since that hook may still move it on, and while
 * it may hold failures status counts. A hook that moves the log aside calls this, so it runs once per 256 KB of failures.
 */
function removeStrandedLogs(home: string, now: number): void {
  for (const { name, pid } of takenLogs(home)) {
    const path = join(home, name);
    try {
      if (pid === process.pid || statSync(path).mtimeMs >= now - HOOK_FAILURE_WINDOW_MS) continue;
      if (processState(pid) === "gone") unlinkSync(path);
    } catch {
      // Moved on or removed by another hook, or refused: a later move tries again.
    }
  }
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
  // A hook moves the log from the live name to its own and on to `.1` by renames, which keep the file. Read in that
  // order, a log moved during this read is met again under a later name: the read under that name stands, so its
  // failures count once, as the part it became. A give-back puts a taken log back at the live name, or copies its lines
  // to the live log, against that order: a read it overlaps can miss them, or, when it copied them and that live log
  // also moves to `.1` before the read gets there, count them twice.
  const parts = new Map<string, LogPart>();
  const read = (name: string): void => {
    const part = readLogFile(join(home, name));
    if (part !== null) parts.set(part.id, { name, ...part.log });
  };
  read(HOOK_FAILURES_FILE);
  for (const { name } of takenLogs(home)) read(name);
  read(MOVED_ASIDE_FILE);
  const all = [...parts.values()];
  const keptSince = keptSinceOf(all, since);
  const from = keptSince ?? since;
  // Hooks take the time before they append, and a hook gives back the lines of a log it took after other hooks wrote
  // theirs, so lines from hooks running at once can be a little out of order.
  const failures = all.flatMap(part => part.failures.filter(failure => failure.at >= from)).sort((left, right) => left.at - right.at);
  return { failures, keptSince };
}

/** A file of the log, by the last name a read met it under. */
interface LogPart extends LogText {
  name: string;
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
  const start = movedAside.startedAt ?? oldestOf(parts.filter(part => part === movedAside || part.name === HOOK_FAILURES_FILE));
  return start !== undefined && start >= since ? start : null;
}

/** The time of the oldest failure the parts hold, epoch ms; undefined when they hold none. */
function oldestOf(parts: readonly LogPart[]): number | undefined {
  let oldest: number | undefined;
  for (const { failures } of parts) {
    for (const { at } of failures) if (oldest === undefined || at < oldest) oldest = at;
  }
  return oldest;
}

/** A log a hook took to move aside, by its name, and the pid of that hook. */
interface TakenLog {
  name: string;
  pid: number;
}

/** The logs hooks took to move aside, in the Rerail home `home`; none when it cannot be listed. */
function takenLogs(home: string): TakenLog[] {
  let names: string[];
  try {
    names = readdirSync(home);
  } catch {
    return [];
  }
  return names.flatMap(name => {
    const pid = TAKEN_LOG.exec(name)?.[1];
    return pid === undefined ? [] : [{ name, pid: Number(pid) }];
  });
}

/** What the file at `path` holds, with its {@link fileId}; null when it is missing or unreadable. */
function readLogFile(path: string): { id: string; log: LogText } | null {
  let descriptor: number;
  try {
    descriptor = openSync(path, "r");
  } catch {
    return null;
  }
  try {
    return { id: fileId(fstatSync(descriptor, { bigint: true }), path), log: parseLog(readFileSync(descriptor, "utf8")) };
  } catch {
    return null;
  } finally {
    closeSync(descriptor);
  }
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
    const value = JSON.parse(line) as Partial<HookFailure & PartStart>;
    return typeof value.startedAt === "number" ? { startedAt: value.startedAt } : failureOf(value);
  } catch {
    // A line cut short by a concurrent write.
    return null;
  }
}

/** The failure a parsed line holds, with the optional fields it lacks filled in; null when it holds none. */
function failureOf(value: Partial<HookFailure>): HookFailure | null {
  if (typeof value.at !== "number" || typeof value.event !== "string" || typeof value.reason !== "string") return null;
  return { at: value.at, host: value.host ?? "unknown", event: value.event, agent: value.agent ?? null, rerail: value.rerail ?? "unknown", reason: value.reason };
}
