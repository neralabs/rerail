import { randomUUID } from "node:crypto";
import { type BigIntStats, constants } from "node:fs";
import { type FileHandle, lstat, mkdir, open, rm } from "node:fs/promises";
import { renameWithRetry } from "../file-replace.js";
import { openNoFollow } from "../no-follow.js";
import { type FoundFolderRepair, keepFolderPrivate, keepFoundFolderPrivate, type PrivateMachine, type RepairEnd, shareLikeParent } from "./private-files.js";
import { dirname, resolve, sep } from "node:path";

/**
 * Session records default to `<checkout>/.rerail/sessions`, a path the repository controls: a committed `.rerail` or
 * `sessions` symlink would send prompts, pending steers and preimages to a folder the repository chose, and a planted
 * file would be read back as Rerail's own state. Below the innermost `.rerail` folder on a path, every folder must be
 * a real one and every file is opened without following a symlink. A root outside any `.rerail` folder
 * (`RERAIL_STORAGE_ROOT`) is the person's own choice and is used as it is.
 */
const LEDGER_FOLDER = ".rerail";

/** A session record path Rerail refuses because a symlink, or something that is not a folder, sits on it. */
class LedgerPathRefusedError extends Error {
  readonly path: string;

  constructor(path: string, what: string) {
    super(`Rerail does not keep session records through ${path}: it is ${what}, which could send them outside the checkout. `
      + "Remove it, and Rerail makes its own.");
    this.name = "LedgerPathRefusedError";
    this.path = path;
  }
}

/** Logs the refusal, which a hook's caller may only see as a skipped hook, and throws it. */
function refuse(path: string, what: string): never {
  const error = new LedgerPathRefusedError(path, what);
  process.stderr.write(`rerail: ${error.message}\n`);
  throw error;
}

function errno(error: unknown): string | undefined {
  return error instanceof Error && "code" in error && typeof error.code === "string" ? error.code : undefined;
}

/**
 * The folders in a session records folder that every account recording there shares: each agent host's locks and
 * held steers (`<host>-locks`, `<host>-pending`, `hook-adapter.ts`). A session's own folder is named by a digest.
 */
const SHARED_FOLDER = /^[a-z][a-z-]*-(?:locks|pending)$/u;

/**
 * Checks the folders from the innermost `.rerail` on `directory` down to it, refusing a symlink or a non-folder among
 * them. With `create`, makes each missing one (`0700`), and the folders above `.rerail` as `mkdir -p` would; without
 * it, stops at the first missing one.
 */
async function ledgerFolder(directory: string, create: boolean, machine: PrivateMachine = {}): Promise<void> {
  const absolute = resolve(directory);
  const segments = absolute.split(sep);
  const start = segments.lastIndexOf(LEDGER_FOLDER);
  if (start <= 0) {
    if (create) await mkdir(absolute, { recursive: true, mode: 0o700 });
    return;
  }
  for (let end = start + 1; end <= segments.length; end += 1) {
    if (!await checkedFolder({ segments, start, end }, create ? machine : undefined)) return;
  }
}

/** A folder on a session record path: the path's segments, where the innermost `.rerail` is, and where the folder ends. */
interface PathFolder {
  readonly segments: readonly string[];
  readonly start: number;
  readonly end: number;
}

/**
 * Checks one folder of {@link ledgerFolder}'s, making it when it is missing and `machine` is given; false when it is
 * missing and not made. Refuses a symlink or a non-folder.
 */
async function checkedFolder(at: PathFolder, machine: PrivateMachine | undefined): Promise<boolean> {
  const folder = at.segments.slice(0, at.end).join(sep);
  let stats = await lstat(folder).catch((error: unknown) => {
    if (errno(error) === "ENOENT") return undefined;
    throw error;
  });
  if (stats === undefined) {
    if (machine === undefined) return false;
    await makeFolder(at, machine);
    stats = await lstat(folder);
  }
  if (stats.isSymbolicLink()) refuse(folder, "a symbolic link");
  if (!stats.isDirectory()) refuse(folder, "not a folder");
  return true;
}

/** Makes one missing folder of {@link ledgerFolder}'s (`0700`), and those above `.rerail` first. */
async function makeFolder(at: PathFolder, machine: PrivateMachine): Promise<void> {
  const { segments, start, end } = at;
  const folder = segments.slice(0, end).join(sep);
  if (end === start + 1) await mkdir(segments.slice(0, start).join(sep) || sep, { recursive: true, mode: 0o700 });
  const made = await mkdir(folder, { mode: 0o700 }).then(() => true, (error: unknown) => {
    if (errno(error) !== "EEXIST") throw error;
    return false;
  });
  if (made) await keepMadeFolder(at, folder, machine);
}

/**
 * Windows ignores `mkdir`'s mode: the folder of session records this call made gets an owner-only ACL, which every
 * record inherits, and a folder in it that every account recording there shares, the entry that lets those accounts
 * add files ({@link shareLikeParent}).
 */
async function keepMadeFolder(at: PathFolder, folder: string, machine: PrivateMachine): Promise<void> {
  if (at.end === at.start + 2) keepFolderPrivate(folder, { ...machine, made: true });
  else if (at.end === at.start + 3 && SHARED_FOLDER.test(at.segments[at.end - 1] ?? "")) await shareLikeParent(folder, dirname(folder), machine);
}

/** What {@link makeLedgerFolder} is given: the system the folder is kept private on, and whether it repairs. */
export interface LedgerFolderOptions extends PrivateMachine {
  /** The daemon opening a session: the session records folder is kept private whoever made it ({@link keepRecordsFolderPrivate}). */
  readonly repair?: boolean;
}

/**
 * A session records folder this process keeps private: what `lstat` found when its repair started, the repair once the
 * folder's ACL is read, and whether the system refused it.
 */
interface KeptRecordsFolder {
  readonly found: BigIntStats;
  readonly repair: Promise<FoundFolderRepair>;
  refused: boolean;
}

/** The session records folders this process keeps private, each until its repair fails in a way that may pass. */
const keptRecordsFolders = new Map<string, KeptRecordsFolder>();

/** The session records folder on `directory`, the one just below the innermost `.rerail`; undefined when there is none. */
function recordsFolderOf(directory: string): string | undefined {
  const segments = resolve(directory).split(sep);
  const start = segments.lastIndexOf(LEDGER_FOLDER);
  return start <= 0 || segments.length < start + 2 ? undefined : segments.slice(0, start + 2).join(sep);
}

/**
 * Keeps the records folder `folder` on `directory` its owner's alone whoever made it ({@link keepFoundFolderPrivate}),
 * once per folder in this process ({@link recordsFolderRepair}). A session that opens there waits for the folder's ACL
 * to be read, not for the repair: unless the repair had ended at `since`, read before the session's own folder was
 * made, that folder gets the owner-only ACL of its own first, so while the repair runs, shared by every session that
 * opens there meanwhile, and once the read or the repair failed. Git makes that folder for a repository that commits a
 * file in it, and a person or a release before 0.1.5 may have made it, each with its parent's ACL on Windows, which
 * every record in it took.
 */
async function keepRecordsFolderPrivate(directory: string, folder: string, found: BigIntStats | undefined, since: number, machine: PrivateMachine): Promise<void> {
  const repair = await recordsFolderRepair(folder, machine, found);
  // A storage root that is `.rerail` itself makes each session's folder the records folder, whose repair is its own.
  await (resolve(directory) === folder ? repair.kept : repair.keepInside(directory, since));
}

/**
 * The repair of the records folder `folder` this process keeps, or a new one when it keeps none, a folder with another
 * file id has taken its place, or its repair was refused and the folder has changed since ({@link changedSince}). One
 * that failed in a way that may pass by itself, as an `icacls` past its time limit, is dropped, so the next session
 * tries again. `found` is the folder as `lstat` found it before this session's own folder was made there.
 */
async function recordsFolderRepair(folder: string, machine: PrivateMachine, found: BigIntStats | undefined): Promise<FoundFolderRepair> {
  const now = found ?? await lstat(folder, { bigint: true });
  const known = keptRecordsFolders.get(folder);
  if (known?.found.ino === now.ino && !(known.refused && changedSince(known.found, now))) return await known.repair;
  const repair = keepFoundFolderPrivate(folder, { ...machine, shared: name => SHARED_FOLDER.test(name) });
  const kept: KeptRecordsFolder = { found: now, repair, refused: false };
  keptRecordsFolders.set(folder, kept);
  void repair.then(started => started.kept, (): RepairEnd => "transient").then(end => {
    kept.refused = end === "refused";
    if (end === "transient" && keptRecordsFolders.get(folder) === kept) keptRecordsFolders.delete(folder);
  });
  return await kept.repair;
}

/**
 * Whether a records folder found as `before` when the system refused its repair may take it now: its owner or mode
 * moved, or the last change to it was to what describes it and came after `before`. A new entry sets a folder's change
 * time and write time to one value, on Windows (NTFS) as on Linux, so the sessions that open there do not count; a
 * change of its access list or owner on Windows, or of its owner, mode or flags elsewhere, moves its change time alone.
 * A refused attempt moves neither.
 */
function changedSince(before: BigIntStats, now: BigIntStats): boolean {
  if (now.uid !== before.uid || now.mode !== before.mode) return true;
  return now.ctimeNs !== now.mtimeNs && now.ctimeNs !== before.ctimeNs;
}

/**
 * Creates the session record folder `directory`, refusing a symlink below `.rerail` (see {@link LEDGER_FOLDER}). On
 * Windows, which ignores `0700`, the folder it makes just below `.rerail` (`sessions`) gets an owner-only ACL when it is
 * outside the profile folder: a checkout under `C:\` would otherwise let every local user read prompts and preimages,
 * and change held steers. With `repair`, as the daemon opens a session, that folder gets it too when another program
 * made it, before anything of the session is written there.
 */
export async function makeLedgerFolder(directory: string, options: LedgerFolderOptions = {}): Promise<void> {
  const { repair = false, ...machine } = options;
  const records = repair ? recordsFolderOf(directory) : undefined;
  // Both read before this session's own folder is made there: the records folder, whose change time that sets as any
  // new entry does, and the time, so a repair that ends after the folder was made, and may have passed over the
  // records folder before it was there, still has it given its owner-only ACL.
  const found = records === undefined ? undefined : await lstat(records, { bigint: true }).catch((error: unknown) => {
    if (errno(error) === "ENOENT") return undefined;
    throw error;
  });
  const since = performance.now();
  await ledgerFolder(directory, true, machine);
  if (records !== undefined) await keepRecordsFolderPrivate(directory, records, found, since, machine);
}

/**
 * Opens a session record file below a checked folder without following a symlink at its name; a symlink there is
 * refused. `flags` are `fs.constants` open flags; a file it creates gets `0600`.
 */
export async function openLedgerFile(path: string, flags: number): Promise<FileHandle> {
  try {
    return await openNoFollow(path, flags, 0o600);
  } catch (error) {
    if (errno(error) === "ELOOP") refuse(path, "a symbolic link");
    throw error;
  }
}

/** A session record file's text, or undefined when it or its folder is missing. Refuses what `makeLedgerFolder` does. */
export async function readLedgerFile(path: string, directory: string): Promise<string | undefined> {
  await ledgerFolder(directory, false);
  let handle: FileHandle;
  try {
    handle = await openLedgerFile(path, constants.O_RDONLY);
  } catch (error) {
    if (errno(error) === "ENOENT") return undefined;
    throw error;
  }
  try {
    return await handle.readFile("utf8");
  } finally {
    await handle.close();
  }
}

/**
 * Replaces a session record file whole: written to a new file beside it, then renamed over it, so a symlink at its
 * name is replaced, never followed, and a reader never sees half of it. Creates its folder as `makeLedgerFolder` does.
 */
export async function writeLedgerFile(path: string, directory: string, text: string): Promise<void> {
  await makeLedgerFolder(directory);
  const temporary = `${path}.${process.pid}.${randomUUID()}.tmp`;
  try {
    const handle = await openLedgerFile(temporary, constants.O_WRONLY | constants.O_CREAT | constants.O_EXCL);
    try {
      await handle.writeFile(text, "utf8");
    } finally {
      await handle.close();
    }
    await renameWithRetry(temporary, path);
  } catch (error) {
    await rm(temporary, { force: true });
    throw error;
  }
}
