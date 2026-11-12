import { randomUUID } from "node:crypto";
import { appendFileSync, closeSync, fstatSync, openSync, readdirSync, readFileSync, statSync, unlinkSync } from "node:fs";
import { join } from "node:path";
import { renameWithRetrySync } from "../engine/file-replace.js";
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
  now?: number;
  env?: NodeJS.ProcessEnv;
}

/** Appends one failure. Never throws: a hook must not break the session over its own bookkeeping. */
export function recordHookFailure(
  failure: Pick<HookFailure, "host" | "event" | "reason">,
  { home = defaultRerailHome(), now = Date.now(), env = process.env }: HookFailureContext = {},
): void {
  try {
    makeRerailHome(home);
    const path = join(home, HOOK_FAILURES_FILE);
    if (sizeOf(path) > MAX_FAILURE_BYTES) moveAside(path, join(home, MOVED_ASIDE_FILE));
    const line: HookFailure = { at: now, ...failure, agent: env.AI_AGENT ?? null, rerail: rerailVersion() };
    appendFileSync(path, `${JSON.stringify(line)}\n`, { mode: 0o600 });
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
 * gives it back. The uninstall knows the name as a move cut short, and `readHookFailures` counts what such a file holds.
 */
const takenName = (path: string): string => `${path}.${process.pid}.${randomUUID()}.old`;
const isTakenName = (name: string): boolean => name.startsWith(`${HOOK_FAILURES_FILE}.`) && name.endsWith(".old");

/**
 * Moves the full log at `path` to `<name>.1`. Hooks that fail at once each find the log full, and a rename straight to
 * `.1` made the later ones fail on a log already moved, losing their own line, or move the new log another hook had
 * just begun over the full one. So a hook first takes the log under a name of its own, and moves it to `.1` only when
 * what it took is full; a hook that took the new log another hook began gives its lines back. A hook stopped between
 * those steps, as a coding agent stops a hook past its time limit, leaves the log under its own name, where
 * `readHookFailures` still counts it.
 *
 * On Windows a rename opens the file by name, then renames what it opened: a hook that opened the log before this one
 * took it still moves it on, out from under this hook's name. So each step gives up once the file is gone from that
 * name, leaving it to the hook that holds it last.
 */
function moveAside(path: string, movedAside: string): void {
  const taken = takenName(path);
  try {
    renameWithRetrySync(path, taken);
  } catch {
    // Moved first by another hook, or held through the retries: this hook took nothing.
    return;
  }
  try {
    // No wait for a `.1` another program holds: the log goes back at once and a later hook moves it, so it stays under
    // this hook's name for the moment of one rename, not for the 2 s of retries.
    if (sizeOf(taken) > MAX_FAILURE_BYTES) {
      renameWithRetrySync(taken, movedAside, { retryMs: 0 });
      return;
    }
  } catch (error) {
    // Moved on by another hook. Any other failure leaves `.1` held: the log goes back.
    if ((error as NodeJS.ErrnoException).code === "ENOENT") return;
  }
  try {
    const lines = readFileSync(taken);
    // Gone from its name before the lines go back, so a hook that moves it on finds nothing to give back twice.
    unlinkSync(taken);
    appendFileSync(path, lines, { mode: 0o600 });
  } catch {
    // Gone from its name: another hook moved it on, and gives it back or moves it aside. A read or removal refused for
    // any other reason leaves the lines under this hook's name, where status still counts them; nothing removes that
    // file, and the uninstall lists it among Rerail's own. An append that fails after the removal loses the lines, as
    // the same failure loses this hook's own line.
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
   * When the log was moved aside and keeps nothing as old as the given time, failures from before about its oldest may
   * be gone (hooks failing as the log moves can land on either side of it): that oldest failure's time, epoch ms, from
   * which the failures are counted. Null when the log reaches back to the given time.
   */
  keptSince: number | null;
}

/**
 * What `rerail status` says of the log's failures from the last `HOOK_FAILURE_WINDOW_MS`, given when the newest
 * recorded event was observed (`lastRecordedAt`, epoch ms; null when nothing was ever recorded). When every failure is
 * newer than the last recorded event, hooks run but nothing is recorded, and the line leads with that.
 */
export function hookFailureLines({ failures, keptSince }: HookFailureLog, lastRecordedAt: number | null): string[] {
  const last = failures.at(-1);
  if (last === undefined) return [];
  const since = lastRecordedAt === null ? failures : failures.filter(failure => failure.at > lastRecordedAt);
  const from = last.agent ?? last.host;
  const detail = `the last, ${last.event} from ${from} with Rerail ${last.rerail} at ${new Date(last.at).toISOString()}: ${last.reason}`;
  if (since.length === failures.length) {
    return [`Hooks run but nothing was recorded since ${new Date(failures[0]!.at).toISOString()}: ${plural(failures.length, "hook event", "hook events")} failed; ${detail}.`];
  }
  const period = keptSince === null ? "in the last 24 hours" : `since ${new Date(keptSince).toISOString()}, as far back as the log goes`;
  return [`Hooks: ${plural(failures.length, "hook event", "hook events")} could not be recorded ${period}; ${detail}.`];
}

/**
 * The failures recorded since `since` (epoch ms), in every part of the log: the live part, the full logs hooks took to
 * move aside and hold under their own names, and the part moved aside to `.1`. A file missing or unreadable holds none.
 */
export function readHookFailures(home: string, since: number): HookFailureLog {
  // A hook moves the log from the live name to its own and on to `.1` by renames, which keep the file. Read in that
  // order, a log moved during this read is met again under a later name: the read under that name stands, so its
  // failures count once, as the part it became. A give-back copies a taken log's lines to the live one, against that
  // order: a read it overlaps can miss them, never count them twice.
  const parts = new Map<string, LogPart>();
  const read = (name: string): void => {
    const part = readLogFile(join(home, name));
    if (part !== null) parts.set(part.id, { name, failures: part.failures });
  };
  read(HOOK_FAILURES_FILE);
  for (const name of takenLogs(home)) read(name);
  read(MOVED_ASIDE_FILE);
  const all = [...parts.values()];
  // How far back the log goes is the oldest failure of `.1` and the live part. A log a hook was stopped holding can be
  // older than `.1`, with dropped logs between them, so it counts from that time on but never sets it.
  const oldest = oldestOf(all.filter(part => part.name === MOVED_ASIDE_FILE || part.name === HOOK_FAILURES_FILE));
  const keptSince = all.some(part => part.name === MOVED_ASIDE_FILE) && oldest !== undefined && oldest >= since ? oldest : null;
  const from = keptSince ?? since;
  // Hooks take the time before they append, and a hook gives back the lines of a log it took after other hooks wrote
  // theirs, so lines from hooks running at once can be a little out of order.
  const failures = all.flatMap(part => part.failures.filter(failure => failure.at >= from)).sort((left, right) => left.at - right.at);
  return { failures, keptSince };
}

/** A file of the log, by the last name a read met it under. */
interface LogPart {
  name: string;
  failures: HookFailure[];
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
    return readdirSync(home).filter(isTakenName);
  } catch {
    return [];
  }
}

/**
 * The failures the file at `path` holds, with an id that stays the same under every name the file moves to (its device
 * and file number); null when it is missing or unreadable. A file system with no file numbers gives 0, and each name
 * then counts as a file of its own.
 */
function readLogFile(path: string): { id: string; failures: HookFailure[] } | null {
  let descriptor: number;
  try {
    descriptor = openSync(path, "r");
  } catch {
    return null;
  }
  try {
    const { dev, ino } = fstatSync(descriptor, { bigint: true });
    return { id: ino === 0n ? path : `${dev}:${ino}`, failures: parseFailures(readFileSync(descriptor, "utf8")) };
  } catch {
    return null;
  } finally {
    closeSync(descriptor);
  }
}

/** The failures in a log file's text, skipping lines that are not one. */
function parseFailures(text: string): HookFailure[] {
  const failures: HookFailure[] = [];
  for (const line of text.split("\n")) {
    if (line.trim() === "") continue;
    try {
      const value = JSON.parse(line) as Partial<HookFailure>;
      if (typeof value.at === "number" && typeof value.event === "string" && typeof value.reason === "string") {
        failures.push({ at: value.at, host: value.host ?? "unknown", event: value.event, agent: value.agent ?? null, rerail: value.rerail ?? "unknown", reason: value.reason });
      }
    } catch {
      // A line cut short by a concurrent write.
    }
  }
  return failures;
}
