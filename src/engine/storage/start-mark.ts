/**
 * The start mark: a file beside the Rerail database that each change to where Rerail runs renews once it commits, so a
 * one-shot hook can tell that such a change committed after it asked the daemon, even once the database file has been
 * put back as it was before the change.
 *
 * The daemon answers a hook's start check from what its connection read (`startStateReader`). An older copy of the
 * database written over the live file in place (`cp`, `copy /Y`, a file manager's replace), once the daemon has read a
 * later start, leaves the file saying stopped while the daemon still answers started from the pages its connection
 * holds. A hook that repeats the daemon's last answer of stopped (`repeatsStoppedAnswer`) reads the file, which cannot
 * tell it that. So it keeps, with the answer, the mark it found before it asked the daemon, and repeats the answer only
 * while the mark is still that one: a change committed before the hook asked is one the daemon's answer read, and one
 * committed after renews the mark after the hook found it.
 *
 * Each writer that can turn a recorded checkout's answer from stopped renews it ({@link renewStartMark}) once its
 * transaction has committed: a start or a stop, of a checkout or of a repository; set-up's start; a checkout recorded
 * again for another clone; a repository that takes a remote or an alias; and checkouts forgotten. A write inside an
 * outer transaction commits only with it, so the caller that holds that transaction renews the mark after it too. A
 * renewal writes a new file and renames it over the old one, so the mark gets a new file id. Where it cannot at once (a
 * program holding the mark open on Windows, as a virus scan does with a file just written), it writes the stuck mark
 * beside it, and no hook repeats an answer while that is there. It never waits for the mark: a writer is often the
 * daemon, and a wait there would hold every hook asking it. It tries again for 2 s on a timer that keeps no process
 * running, so the daemon, which runs on, renews the mark if the other program lets it go by then; any renewal that
 * succeeds removes the stuck mark. A hook also repeats an answer only where this user may add files to the folder, as a renewal and the
 * stuck mark both must.
 */
import { accessSync, constants, lstatSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { replaceFileSync } from "../file-replace.js";

/** In the Rerail home: the daemon's last answers of stopped that hooks keep, and the start mark beside them. */
export const STOPPED_ANSWERS_DIR = "stopped-answers";

/** The start mark's name in {@link STOPPED_ANSWERS_DIR}, and the name of the file that says a renewal of it failed. */
export const START_MARK = "start-mark";
export const STUCK_MARK = "start-mark-stuck";

/** What {@link startMarkFound} gives where no mark is there yet. */
const NO_MARK = "none";

/**
 * How a renewal another program refused is tried again: every 100 ms for 2 s, the time a rename on Windows waits
 * elsewhere (`renameWithRetry`), each try one rename that waits for nothing.
 */
const RENEW_AGAIN_AFTER_MS = 100;
const RENEWAL_TRIES = 20;

/** The connection a writer holds: the file it opened (`name`), and whether it is a database in memory, with no file. */
interface DatabaseFile {
  readonly $client: { readonly name: string; readonly memory: boolean };
}

/**
 * Renews the start mark beside `database`'s file; a database in memory has none. Nothing is written through a link at
 * the answers folder's name, which can lead anywhere, and no hook repeats an answer kept under one. A failure is not the
 * writer's: the change it marks has committed. One that fails leaves the stuck mark before it returns, and tries again
 * later ({@link renewLater}).
 */
export function renewStartMark(database: DatabaseFile): void {
  const { name, memory } = database.$client;
  if (memory || name === "") return;
  const folder = join(dirname(name), STOPPED_ANSWERS_DIR);
  if (entryAt(folder) === "other") return;
  if (renewedIn(folder)) return;
  markStuck(folder);
  renewLater(folder, RENEWAL_TRIES);
}

/**
 * One renewal of the mark in `folder`, made where it is missing, that waits for nothing: false where any step fails, as
 * where another program holds the mark open on Windows. Each try runs whole in one turn of the event loop, so a renewal
 * refused in this process between this one's rename and its removal of the stuck mark cannot have that mark removed.
 */
function renewedIn(folder: string): boolean {
  try {
    mkdirSync(folder, { recursive: true, mode: 0o700 });
    replaceFileSync(join(folder, START_MARK), `${Date.now()}\n`, { mode: 0o600, retryMs: 0 });
    rmSync(join(folder, STUCK_MARK), { force: true });
    return true;
  } catch {
    return false;
  }
}

/**
 * Tries the renewal refused in `folder` again, `tries` more times, {@link RENEW_AGAIN_AFTER_MS} apart, on a timer that
 * keeps no process running: a one-shot command that ends first leaves the stuck mark, which the next renewal anywhere
 * removes. It stops once the stuck mark is gone, which a renewal since has removed, and once the folder is gone.
 */
function renewLater(folder: string, tries: number): void {
  if (tries === 0) return;
  setTimeout(() => {
    if (entryAt(join(folder, STUCK_MARK)) === "none" || entryAt(folder) !== "folder" || renewedIn(folder)) return;
    renewLater(folder, tries - 1);
  }, RENEW_AGAIN_AFTER_MS).unref();
}

/** Writes the stuck mark in `folder`, created only where nothing is at its name, which hooks read as stuck too. */
function markStuck(folder: string): void {
  try {
    writeFileSync(join(folder, STUCK_MARK), "", { flag: "wx", mode: 0o600 });
  } catch {
    // Something is there already, or the folder takes no new file.
  }
}

/**
 * The start mark beside the database at `databasePath` as a hook keeps it: `none` where nothing is at its name, the
 * device, file id, change time and write time of a plain file there, and undefined for anything else (a link, which is
 * never followed, a folder) or where a look fails. A link at the answers folder's name is not looked through: it can
 * lead to a server that does not answer.
 */
export function startMarkFound(databasePath: string): string | undefined {
  const folder = join(dirname(databasePath), STOPPED_ANSWERS_DIR);
  const kind = entryAt(folder);
  if (kind !== "folder") return kind === "none" ? NO_MARK : undefined;
  return markAt(join(folder, START_MARK));
}

function markAt(path: string): string | undefined {
  try {
    const found = lstatSync(path, { bigint: true, throwIfNoEntry: false });
    if (found === undefined) return NO_MARK;
    return found.isFile() && found.ino !== 0n ? `${found.dev}:${found.ino}:${found.ctimeNs}:${found.mtimeNs}` : undefined;
  } catch {
    return undefined;
  }
}

/**
 * Whether the start mark beside the database at `databasePath` is still `kept`, as {@link startMarkFound} gave it, no
 * renewal of it has failed since one last succeeded, and a renewal could still be recorded there.
 */
export function startMarkHolds(databasePath: string, kept: string): boolean {
  const folder = join(dirname(databasePath), STOPPED_ANSWERS_DIR);
  return startMarkFound(databasePath) === kept && entryAt(join(folder, STUCK_MARK)) === "none" && takesNewFiles(folder);
}

/**
 * Whether this user may add a file to `folder`, as a renewal and the stuck mark both must: where its permissions refuse
 * it (as 0500 does), a renewal and its stuck mark both fail and the mark would stay the one kept. Windows checks only
 * the read-only attribute here, which a folder ignores.
 */
function takesNewFiles(folder: string): boolean {
  try {
    accessSync(folder, constants.W_OK | constants.X_OK);
    return true;
  } catch {
    return false;
  }
}

/** What is at `path`, not followed through a link: nothing, a folder, or anything else, a look that fails included. */
function entryAt(path: string): "none" | "folder" | "other" {
  try {
    const found = lstatSync(path, { throwIfNoEntry: false });
    if (found === undefined) return "none";
    return found.isDirectory() ? "folder" : "other";
  } catch {
    return "other";
  }
}
