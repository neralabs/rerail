import { closeSync, constants, fstatSync, lstatSync, mkdirSync, openSync, statSync, writeSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { makeRerailHome } from "../storage/private-files.js";
import { defaultRerailHome } from "../storage/rerail-home.js";

/**
 * The file a clock writes one byte to for each sample, in the Rerail home or, when the clock is read elsewhere, in this
 * user's folder in the temporary folder (`chooseClockProbe`). Each write stamps it with the clock an edit stamps a
 * source with, so a sample makes and removes no folder, and a process killed mid-sample leaves nothing new behind.
 */
export const CLOCK_PROBE_FILE = "clock-probe";
const CLOCK_PROBE_BYTE = Buffer.from(".");

/** Where a clock's samples are taken: the probe file, and how to make the folder that holds it when that is missing. */
interface ClockProbe {
  readonly path: string;
  makeFolder(): void;
}

/**
 * The clock the files under `checkout` are stamped with, read as the stamp a write leaves on a probe file. `Date.now()`
 * is not that clock: on Linux file stamps advance per scheduler tick and lag it, so a write made right after session
 * start carries an earlier mtime, while on macOS stamps carry sub-millisecond precision that `Date.now()` truncates away.
 * Nor is every file system's clock the checkout's: a FAT drive keeps 2 s stamps, and a network drive takes its
 * server's clock.
 *
 * The probe is chosen once, at the first sample. Each sample's steps are synchronous calls, so a sample is taken whole
 * when it is asked for: through `fs/promises` each step would wait behind the walks' file reads in the thread pool, then
 * for its turn of the event loop, which on a busy machine put tens of milliseconds into the start and the end of every
 * tool call. Taken at once, a sample holds the event loop for a fraction of a millisecond.
 */
export function filesystemClock(checkout: string): () => number {
  let probe: ClockProbe | undefined;
  return () => {
    probe ??= chooseClockProbe(checkout);
    return sample(probe);
  };
}

/**
 * The probe for the files under `checkout`, by the file systems the paths are on, as their devices say. A write is the
 * session's when its stamp falls between the samples taken as one of its tool calls starts and ends, so the samples have
 * to agree with the checkout's file system, and the Rerail home's probe does when the home is on it. No probe goes in
 * the checkout itself, a folder the repository controls, editors watch and Git lists. With the home on another file
 * system, the temporary folder's is the better guess: it is on the local disk on nearly every machine, as checkouts are,
 * while a home with a file system of its own is most often a network one (a home folder on a file server, stamped by
 * the server's clock) or a removable one (`RERAIL_HOME` on a FAT drive). So the probe moves to the temporary folder,
 * unless the home shares the temporary folder's file system and so reads the same clock.
 */
function chooseClockProbe(checkout: string): ClockProbe {
  const home = defaultRerailHome();
  const device = deviceOf(home);
  if (device !== undefined && (device === deviceOf(checkout) || device === deviceOf(tmpdir()))) return homeProbe(home);
  return temporaryProbe() ?? homeProbe(home);
}

/** The device of the file system `path` is on, or that a folder made at `path` would be on; undefined when unreadable. */
function deviceOf(path: string): number | undefined {
  try {
    for (let folder = path; ; folder = dirname(folder)) {
      const stats = statSync(folder, { throwIfNoEntry: false });
      if (stats !== undefined) return stats.dev;
      if (dirname(folder) === folder) return undefined;
    }
  } catch {
    return undefined;
  }
}

/** The probe in the Rerail home, which a sample makes through `makeRerailHome` when it is missing, as Rerail's other writers make it. */
function homeProbe(home: string): ClockProbe {
  return { path: join(home, CLOCK_PROBE_FILE), makeFolder: () => makeRerailHome(home) };
}

/**
 * The probe in this user's own folder in the temporary folder: `rerail-<uid>`, where the daemon keeps its socket on
 * Linux and macOS, or `rerail` on Windows, whose temporary folder is the person's own. Undefined when that folder cannot
 * be made or is not this user's alone: a fixed name in a shared `/tmp` is one another user could make first.
 */
function temporaryProbe(): ClockProbe | undefined {
  const uid = process.getuid?.();
  const folder = join(tmpdir(), uid === undefined ? "rerail" : `rerail-${uid}`);
  const probe: ClockProbe = { path: join(folder, CLOCK_PROBE_FILE), makeFolder: () => makePrivateFolder(folder, uid) };
  try {
    probe.makeFolder();
    return probe;
  } catch {
    return undefined;
  }
}

/**
 * Makes `folder` (`0700`) when it is missing, and refuses it unless it is this user's alone, as the daemon checks the
 * folder of its socket: a real folder, not a link, and, where files have owners and modes (not on Windows), owned by
 * `uid` with no group or other access.
 */
function makePrivateFolder(folder: string, uid: number | undefined): void {
  mkdirSync(folder, { recursive: true, mode: 0o700 });
  const stats = lstatSync(folder);
  const own = stats.isDirectory() && (uid === undefined || (stats.uid === uid && (stats.mode & 0o077) === 0));
  if (!own) throw new Error(`${folder} is not this user's own folder`);
}

/**
 * One reading of the clock: the probe's byte rewritten, then its stamp read from the open file. Any failure falls back to
 * `Date.now()`, but a failed close keeps the reading: the sample was taken.
 */
function sample(probe: ClockProbe): number {
  try {
    const descriptor = openProbe(probe);
    try {
      writeSync(descriptor, CLOCK_PROBE_BYTE, 0, CLOCK_PROBE_BYTE.length, 0);
      const info = fstatSync(descriptor);
      return Math.min(info.mtimeMs, info.ctimeMs);
    } finally {
      closeQuietly(descriptor);
    }
  } catch {
    return Date.now();
  }
}

/**
 * Opens the probe to rewrite its byte, making it the first time (`0600`, private as Rerail's other files are) and the
 * folder that holds it when that is missing. The open never empties the file, as `w` would: a sample needs only its one
 * byte rewritten. It refuses a link at the name, as the bridge's other opens do, so no sample writes into the file a
 * link names; the sample falls back to `Date.now()` while the link stays. Windows has no `O_NOFOLLOW`, so there this
 * open follows one as those do.
 */
function openProbe(probe: ClockProbe): number {
  const flags = constants.O_RDWR | constants.O_CREAT | constants.O_NOFOLLOW;
  try {
    return openSync(probe.path, flags, 0o600);
  } catch (error) {
    if (!isMissing(error)) throw error;
    probe.makeFolder();
    return openSync(probe.path, flags, 0o600);
  }
}

/** Closes `descriptor`; a failed close must not throw a sample away for `Date.now()`, the clock this module avoids. */
function closeQuietly(descriptor: number): void {
  try {
    closeSync(descriptor);
  } catch {
    // The sample was taken; there is nothing left to undo.
  }
}

function isMissing(error: unknown): boolean {
  return error instanceof Error && "code" in error && error.code === "ENOENT";
}
