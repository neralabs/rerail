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
 * renewal writes a new file and renames it over the old one, so the mark gets a new file id. Where it cannot (a program
 * holding the mark open on Windows), it writes the stuck mark beside it, and no hook repeats an answer while that is
 * there; the next renewal that succeeds removes it.
 */
import { lstatSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { replaceFileSync } from "../file-replace.js";

/** In the Rerail home: the daemon's last answers of stopped that hooks keep, and the start mark beside them. */
export const STOPPED_ANSWERS_DIR = "stopped-answers";

/** The start mark's name in {@link STOPPED_ANSWERS_DIR}, and the name of the file that says a renewal of it failed. */
export const START_MARK = "start-mark";
export const STUCK_MARK = "start-mark-stuck";

/** What {@link startMarkFound} gives where no mark is there yet. */
const NO_MARK = "none";

/** The connection a writer holds: the file it opened (`name`), and whether it is a database in memory, with no file. */
interface DatabaseFile {
  readonly $client: { readonly name: string; readonly memory: boolean };
}

/**
 * Renews the start mark beside `database`'s file; a database in memory has none. Nothing is written through a link at
 * the answers folder's name, which can lead anywhere, and no hook repeats an answer kept under one. A failure is not the
 * writer's: the change it marks has committed.
 */
export function renewStartMark(database: DatabaseFile): void {
  const { name, memory } = database.$client;
  if (memory || name === "") return;
  const folder = join(dirname(name), STOPPED_ANSWERS_DIR);
  if (entryAt(folder) === "other") return;
  try {
    mkdirSync(folder, { recursive: true, mode: 0o700 });
    replaceFileSync(join(folder, START_MARK), `${Date.now()}\n`, { mode: 0o600 });
    rmSync(join(folder, STUCK_MARK), { force: true });
  } catch {
    markStuck(folder);
  }
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
 * Whether the start mark beside the database at `databasePath` is still `kept`, as {@link startMarkFound} gave it, and
 * no renewal of it has failed since one last succeeded.
 */
export function startMarkHolds(databasePath: string, kept: string): boolean {
  return startMarkFound(databasePath) === kept && entryAt(join(dirname(databasePath), STOPPED_ANSWERS_DIR, STUCK_MARK)) === "none";
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
