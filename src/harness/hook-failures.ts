import { appendFileSync, closeSync, fstatSync, mkdirSync, openSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import { renameWithRetrySync } from "../engine/file-replace.js";
import { LOCKS_DIR } from "../engine/native/file-lock.js";
import { loadNativeBinding, type NativeExclusiveFileLock } from "../engine/native/native.js";
import { PACKAGE_ROOT } from "../engine/package-root.js";
import { defaultRerailHome } from "../engine/storage/rerail-home.js";
import { makeRerailHome } from "../engine/storage/private-files.js";

/**
 * In the Rerail home: one JSON line per hook event Rerail could not record, so `rerail status` can say that hooks run
 * but nothing is recorded, and why. A hook's own stderr reaches no one: coding agents keep it out of the session.
 */
export const HOOK_FAILURES_FILE = "hook-failures.jsonl";

/** Past this size the file is moved to `<name>.1`, replacing the one before; a failing hook writes on every event. */
const MAX_FAILURE_BYTES = 256 * 1024;

/**
 * Past this size the live log takes no more failures: the most it holds while it cannot be moved aside, as when another
 * program holds `.1`, or a hook is stopped holding the move's lock until the system frees it.
 */
const CAP_FAILURE_BYTES = 4 * MAX_FAILURE_BYTES;

/** Where the full log moves, replacing the one before: the older part of the log. */
const MOVED_ASIDE_FILE = `${HOOK_FAILURES_FILE}.1`;

/** In the Rerail home's `locks/`: the OS lock a hook holds while it moves the full log aside. */
const MOVE_LOCK_FILE = "hook-failures.lock";

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
 * The line a hook that moved the full log aside writes into the new log: every failure stamped from `startedAt` (epoch
 * ms) on is in this part of the log or a later one, so `rerail status` can say from when it counts once the part before
 * is gone. Readers that know only failures skip it, as it has no event.
 */
interface PartStart {
  startedAt: number;
}

/**
 * How far past its own clock a hook that moved the log aside says the next part began: one tick of the system clock and
 * a millisecond. On Windows two processes' clocks can differ by up to one tick (15.625 ms at most), so a failure another
 * hook stamped just before the move can read a little later than the mover's clock.
 */
const PART_START_PAST_CLOCK_MS = 16 + 1;

/**
 * Appends one failure. Never throws: a hook must not break the session over its own bookkeeping. A log that could not
 * be moved aside and has reached `CAP_FAILURE_BYTES` keeps what it holds, and the failure is not written.
 */
export function recordHookFailure(
  failure: Pick<HookFailure, "host" | "event" | "reason">,
  { home = defaultRerailHome(), now, env = process.env }: HookFailureContext = {},
): void {
  try {
    makeRerailHome(home);
    const path = join(home, HOOK_FAILURES_FILE);
    const size = sizeOf(path);
    const moved = size > MAX_FAILURE_BYTES && movedAside(home, path);
    // At the cap by the size read before the lock, the log is read again: another hook may have moved it since.
    if (!moved && size >= CAP_FAILURE_BYTES && sizeOf(path) >= CAP_FAILURE_BYTES) return;
    // Read after the move, past it by a clock step when this hook moved the log: the hook's own failure carries the
    // time the new part starts, so it counts in that part.
    const at = now ?? Date.now() + (moved ? PART_START_PAST_CLOCK_MS : 0);
    const line = `${JSON.stringify({ at, ...failure, agent: env.AI_AGENT ?? null, rerail: rerailVersion() } satisfies HookFailure)}\n`;
    appendFileSync(path, moved ? `${JSON.stringify({ startedAt: at } satisfies PartStart)}\n${line}` : line, { mode: 0o600 });
  } catch {
    // Nowhere left to say it.
  }
}

/** The log's size: 0 when there is none, and past every limit when it cannot be told, so nothing is written unchecked. */
function sizeOf(path: string): number {
  try {
    return statSync(path).size;
  } catch (error) {
    return (error as NodeJS.ErrnoException).code === "ENOENT" ? 0 : Number.POSITIVE_INFINITY;
  }
}

/**
 * Moves the full log at `path` to `.1`, replacing the one before; false when this hook moved nothing. One hook at a
 * time moves it, under an OS lock it tries once and never waits for: a hook that finds the lock held leaves the move to
 * its holder and appends to whichever log is live, which status reads either way. Under the lock the size is read
 * again, so a hook that found the log full just before another moved it does not move the new log over the full one in
 * `.1`. Without the engine there is no lock, and the hook moves the log unlocked, as before the lock: two hooks moving
 * at once may then replace a full `.1`, but the log stays bounded.
 */
function movedAside(home: string, path: string): boolean {
  const lock = moveLock(home);
  if (lock === "busy") return false;
  try {
    // A size that cannot be told now (a stat that failed for a moment) moves nothing: that log may be the small one.
    const size = sizeOf(path);
    if (!Number.isFinite(size) || size <= MAX_FAILURE_BYTES) return false;
    renameWithRetrySync(path, join(home, MOVED_ASIDE_FILE));
    return true;
  } catch {
    // Another program holds `.1` (Windows), or something other than a file is there: the failure goes into the live
    // log, up to its cap, and a later hook moves it.
    return false;
  } finally {
    lock?.release();
  }
}

/**
 * The move's lock; "busy" when another hook holds it; undefined when the engine does not load or the lock file cannot
 * be made, where the hook moves the log unlocked.
 */
function moveLock(home: string): NativeExclusiveFileLock | "busy" | undefined {
  try {
    const locks = join(home, LOCKS_DIR);
    mkdirSync(locks, { recursive: true, mode: 0o700 });
    return loadNativeBinding().tryLockExclusive(join(locks, MOVE_LOCK_FILE)) ?? "busy";
  } catch {
    return undefined;
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
 * The failures recorded since `since` (epoch ms), in the live log and the part moved aside to `.1`. When the log has
 * been moved aside, failures from before `.1` began may be gone, so it counts from then (`keptSince`) where that is
 * later than `since`. A `.1` that cannot be read holds nothing, and the count starts where the live log began.
 */
export function readHookFailures(home: string, since: number): HookFailureLog {
  const livePath = join(home, HOOK_FAILURES_FILE);
  let live = readPart(livePath);
  const movedAside = readPart(join(home, MOVED_ASIDE_FILE));
  // A hook moved the live log to `.1` between the two reads, so `.1` is the file just read as live: read the live name
  // again, which now holds the next part, rather than count the moved one twice.
  if (live !== undefined && live.id !== undefined && live.id === movedAside?.id) live = readPart(livePath);
  const start = movedAside === undefined ? live?.startedAt : movedAside.startedAt ?? oldestOf(movedAside) ?? live?.startedAt;
  const keptSince = start !== undefined && start > since ? start : null;
  const from = keptSince ?? since;
  // Hooks read the clock before they append, so lines from hooks running at once can be a little out of order.
  const failures = [movedAside, live].flatMap(part => part?.failures.filter(failure => failure.at >= from) ?? []).sort((left, right) => left.at - right.at);
  return { failures, keptSince };
}

/** What a part of the log holds: its failures, and when it began where a hook that moved the log aside wrote that. */
interface LogPart {
  /** The file's volume and inode, which a rename keeps; undefined where the file system gives no inode. */
  id: string | undefined;
  failures: HookFailure[];
  startedAt: number | undefined;
}

/** The part of the log at `path`; undefined when it is missing or cannot be read. */
function readPart(path: string): LogPart | undefined {
  let descriptor: number;
  try {
    descriptor = openSync(path, "r");
  } catch {
    return undefined;
  }
  let id: string | undefined;
  let text: string;
  try {
    const { dev, ino } = fstatSync(descriptor, { bigint: true });
    id = ino === 0n ? undefined : `${dev}:${ino}`;
    text = readFileSync(descriptor, "utf8");
  } catch {
    return undefined;
  } finally {
    closeSync(descriptor);
  }
  const part: LogPart = { id, failures: [], startedAt: undefined };
  for (const line of text.split("\n")) {
    const value = parseLine(line);
    if (value === null) continue;
    // A part holds one start; a second only where the mover wrote it after the part filled and moved on: the latest is its own.
    if ("startedAt" in value) part.startedAt = Math.max(value.startedAt, part.startedAt ?? value.startedAt);
    else part.failures.push(value);
  }
  return part;
}

/** The time of the oldest failure in `part`, epoch ms; undefined when it holds none. */
function oldestOf(part: LogPart): number | undefined {
  let oldest: number | undefined;
  for (const { at } of part.failures) if (oldest === undefined || at < oldest) oldest = at;
  return oldest;
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
