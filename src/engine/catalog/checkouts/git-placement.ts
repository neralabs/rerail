/**
 * Where Git places a folder that holds a `.git` entry, read from the files Git itself reads to place it: the entry, the
 * Git folder it names, that folder's `HEAD`, `gitdir`, `commondir` and `config.worktree`, and the common Git folder's
 * `objects`, `refs` and `config`. The daemon places every registered worktree of a repository when the repository
 * starts, and a Git process per worktree there held its one thread for seconds to minutes on a slow Windows machine. A
 * linked worktree is placed from these files once Git has placed a checkout of its repository; any other entry (a
 * clone's own `.git` folder, a submodule's, a Git folder kept apart), and any linked worktree whose files Git might read
 * otherwise, is left to Git, whose answer `repositoryIdentityLocation` keeps while these files and the real paths Git
 * prints are unchanged and Git takes the folders for the user's ({@link PlacementInputs.stamp}). Each file, and each
 * owner, is read again at every use, as Git reads it at every start: no file time stands in for its text, since a
 * rewrite within one tick of the file system's clock leaves every time as it was.
 */
import {
  accessSync,
  closeSync,
  constants as fsConstants,
  fstatSync,
  lstatSync,
  openSync,
  readFileSync,
  readlinkSync,
  statSync,
  type Stats,
} from "node:fs";
import { basename, dirname, isAbsolute, join, resolve, sep } from "node:path";
import { pathsOwnedByCurrentUser } from "../../native/native.js";
import { checkoutRealpath, realpathNamesInTurn, storedForm, storedPath } from "../checkout-control.js";

/** What Git reads to place a folder holding a `.git` entry, as it was when read. */
export interface PlacementInputs {
  /** The folder holding the `.git` entry, resolved. */
  readonly folder: string;
  /** The folder's real path, spelled as Git prints a top level ({@link storedPath}). */
  readonly realFolder: string;
  /**
   * One string that changes when any of these inputs changes in a way that can move Git's answer: the entry, the Git
   * folder it names, whether Git takes their owners for the user's, that folder's `commondir`, whether Git takes it for
   * a Git folder, what the common `config` (and the Git folder's `config.worktree`, when the common one has Git read it)
   * says of the format and the work tree, and the real paths of the folder and of the common Git folder, which a link
   * retargeted at a renamed folder moves while every file stays as it was. A Git folder's own times are left out, since
   * every index write moves them.
   */
  readonly stamp: string;
  /** Whether the entry is a `.git` file, as a linked worktree's is. */
  readonly gitFile: boolean;
  /** The Git folder the entry is or names, resolved. */
  readonly gitFolder: string;
  /** The common Git folder its `commondir` names, resolved; undefined when it has none (a clone's own Git folder). */
  readonly linkedCommon: string | undefined;
  /** The real path of the common Git folder: the one `commondir` names, else the Git folder itself. */
  readonly realCommon: string;
  /** Whether Git takes the Git folder for one: a `HEAD` Git takes, and `objects` and `refs` in the common folder. */
  readonly takenByGit: boolean;
  /**
   * Whether Git takes the folder, its `.git` entry and the Git folder that entry is or names for the current user's, read
   * now ({@link ownedAsGitTakes}). Git reads `safe.directory` only for folders it does not, from files outside these
   * inputs, so each placement of such a folder asks Git.
   */
  readonly owned: boolean;
  readonly config: ConfigFacts;
}

/** Git's answer for a linked worktree: its top level and its common Git folder, as Git prints them. */
export interface LinkedPlacement {
  readonly top: string;
  readonly commonDirectory: string;
}

/**
 * The environment variables that move Git's lookup away from the folder's own `.git` entry, or make Git refuse every
 * folder: with any of them set nothing is read from files or kept, and Git is asked each time. `GIT_CEILING_DIRECTORIES`
 * is not one: it stops Git's search above a folder, and Git reads a folder's own `.git` entry before it looks above.
 */
const LOOKUP_VARIABLES = [
  "GIT_DIR",
  "GIT_WORK_TREE",
  "GIT_COMMON_DIR",
  "GIT_OBJECT_DIRECTORY",
  "GIT_IMPLICIT_WORK_TREE",
  "GIT_TEST_ASSUME_DIFFERENT_OWNER",
];

function environmentMovesLookup(): boolean {
  return LOOKUP_VARIABLES.some(name => process.env[name] !== undefined);
}

/**
 * The lookup variables set in this process's environment with their values, one per line; undefined when none is set.
 * A process's environment does not change once it runs, so while it holds one, Git's answer for a folder moves only
 * with that folder's own inputs.
 */
export function lookupEnvironment(): string | undefined {
  const set = LOOKUP_VARIABLES.filter(name => process.env[name] !== undefined);
  return set.length === 0 ? undefined : set.map(name => `${name}=${process.env[name]}`).join("\n");
}

type Looked = Stats | "none" | undefined;

/** An lstat that answers `"none"` for a missing path and undefined for one it cannot read. */
function look(path: string): Looked {
  try {
    return lstatSync(path, { throwIfNoEntry: false }) ?? "none";
  } catch {
    return undefined;
  }
}

function isEntry(looked: Looked): looked is Stats {
  return looked !== undefined && looked !== "none";
}

function isFolder(looked: Looked): looked is Stats {
  return isEntry(looked) && looked.isDirectory();
}

interface FileText {
  readonly text: string;
}

function isText(read: FileText | "none" | undefined): read is FileText {
  return read !== undefined && read !== "none";
}

/** Git refuses a `.git` file larger than this; the other files read here are smaller still. */
const LARGEST_READ = 1 << 20;

/** Opens a file to read it; on POSIX a FIFO planted at its name opens at once, and is then refused as no regular file. */
const READ_FLAGS = fsConstants.O_RDONLY | (fsConstants.O_NONBLOCK ?? 0);

/**
 * A file's text, read now, as Git reads it (through a link at its name); `"none"` when nothing is there, undefined when
 * it is no regular file, is too large or cannot be read.
 */
function fileText(path: string): FileText | "none" | undefined {
  let descriptor: number;
  try {
    descriptor = openSync(path, READ_FLAGS);
  } catch (error) {
    return missing(error) ? "none" : undefined;
  }
  try {
    return regularText(descriptor);
  } catch {
    return undefined;
  } finally {
    closeSync(descriptor);
  }
}

function regularText(descriptor: number): FileText | undefined {
  const stats = fstatSync(descriptor);
  return stats.isFile() && stats.size <= LARGEST_READ ? { text: readFileSync(descriptor, "utf8") } : undefined;
}

function missing(error: unknown): boolean {
  const code = typeof error === "object" && error !== null && "code" in error ? error.code : undefined;
  return code === "ENOENT" || code === "ENOTDIR";
}

/**
 * A path as Git reads it from one of these files: trailing line ends removed; undefined when what is left is empty,
 * spans lines, or is one Git and Node may resolve differently ({@link plainlyResolved}).
 */
function readPath(text: string): string | undefined {
  const named = text.replace(/[\r\n]+$/u, "");
  return /[\r\n]/u.test(named) || named === "" || !plainlyResolved(named) ? undefined : named;
}

/**
 * Whether `path` is one Git and Node resolve alike: on Windows no network path, and no drive letter other than an
 * absolute path's (Git reads `C:name` as a name inside the folder, Node as a path on that drive).
 */
function plainlyResolved(path: string): boolean {
  if (process.platform !== "win32") return true;
  if (/^[\\/]{2}/u.test(path)) return false;
  return !path.slice(2).includes(":") && (!path.includes(":") || /^[A-Za-z]:[\\/]/u.test(path));
}

interface NamedGitFolder {
  readonly gitFolder: string;
  readonly stamp: string;
  readonly file: boolean;
}

interface FoundGitFolder extends NamedGitFolder {
  /** The lstats of the `.git` entry and of the Git folder it is or names, whose owners Git reads. */
  readonly entries: readonly Stats[];
}

const GITDIR = "gitdir: ";

/**
 * The Git folder a `.git` entry is, or the one a `.git` file names, as Git reads the file and reaches that folder
 * ({@link gitFolderReached}); undefined when the file names none, or the folder cannot be reached.
 */
function gitFolderOf(folder: string, entry: Stats): NamedGitFolder | undefined {
  const at = join(folder, ".git");
  if (entry.isDirectory()) return { gitFolder: at, stamp: `d:${entry.ino}`, file: false };
  const named = gitFileNames(at, entry);
  const gitFolder = named === undefined ? undefined : gitFolderReached(folder, named.written);
  return named === undefined || gitFolder === undefined ? undefined : { gitFolder, stamp: `f:${named.text}`, file: true };
}

/** The `.git` file `entry` at `at`, read as Git reads it: its text, and the path it names its Git folder by, as written. */
function gitFileNames(at: string, entry: Stats): { readonly text: string; readonly written: string } | undefined {
  const read = entry.isFile() ? fileText(at) : undefined;
  if (!isText(read) || !read.text.startsWith(GITDIR)) return undefined;
  const written = readPath(read.text.slice(GITDIR.length));
  return written === undefined ? undefined : { text: read.text, written };
}

/**
 * {@link gitFolderOf} the `.git` entry in `folder`, when the Git folder is there, with the lstats of both, read now; its
 * stamp adds that folder's identity.
 */
function gitFolderAt(folder: string): FoundGitFolder | undefined {
  const entry = look(join(folder, ".git"));
  if (!isEntry(entry)) return undefined;
  const named = gitFolderOf(folder, entry);
  const gitFolder = named === undefined ? undefined : look(named.gitFolder);
  if (named === undefined || !isFolder(gitFolder)) return undefined;
  return { ...named, entries: [entry, gitFolder], stamp: [named.stamp, gitFolder.ino].join("\n") };
}

interface CommonFolder {
  readonly linked: string | undefined;
  /** The name `commondir` holds, as written; `-` without one. */
  readonly stamp: string;
}

/**
 * The common Git folder `gitFolder`'s `commondir` names (undefined `linked` without one), as Git resolves it
 * ({@link commonFolderOf}); undefined when it cannot be read or resolved.
 */
function commonOf(gitFolder: string): CommonFolder | undefined {
  const read = fileText(join(gitFolder, "commondir"));
  if (read === "none") return { linked: undefined, stamp: "-" };
  const named = isText(read) ? readPath(read.text) : undefined;
  const linked = named === undefined ? undefined : commonFolderOf(gitFolder, named);
  return named === undefined || linked === undefined ? undefined : { linked, stamp: named };
}

/**
 * The folder a `commondir` holding `named` leads to from the Git folder `gitFolder`, as Git resolves it
 * (`get_common_dir_noenv` takes the real path of the two joined). Git on Linux and macOS resolves the names in turn,
 * each link before the `..` after it, so the joined text is never cleaned first: `x/link/../y` leaves the folder the
 * link reaches, where cleaning the text would cancel `link` and `..`. Undefined when that real path cannot be read,
 * where Git finds no `objects` or stops. Git for Windows opens the joined path, and Windows takes a `..` off the path
 * as written before it follows a link.
 */
function commonFolderOf(gitFolder: string, named: string): string | undefined {
  if (process.platform === "win32") return resolve(gitFolder, named);
  try {
    return realpathNamesInTurn(isAbsolute(named) ? named : `${gitFolder}/${named}`);
  } catch {
    return undefined;
  }
}

/**
 * What Git reads to place `folder`, when it holds a `.git` folder or file whose Git folder is there; undefined when it
 * holds none, when an input cannot be read or is shaped in a way not modelled here, or when the environment moves
 * Git's lookup. A few lstats and reads of small files, and no Git process. Real paths: two, the folder's and the common
 * Git folder's; on Linux and macOS one more for a `commondir`; and the one {@link gitFolderReached} reads for a `.git`
 * file whose path holds a `..` (on Windows only a relative one).
 */
export function placementInputs(folder: string): PlacementInputs | undefined {
  return environmentMovesLookup() ? undefined : filePlacementInputs(folder);
}

/** {@link placementInputs} read from the files whatever the environment says, for an answer Git still gives. */
export function filePlacementInputs(folder: string): PlacementInputs | undefined {
  const entry = gitFolderAt(folder);
  const common = entry === undefined ? undefined : commonOf(entry.gitFolder);
  return entry === undefined || common === undefined ? undefined : withCommon(folder, entry, common);
}

/**
 * Whether `folder` is a linked worktree whose Git folder is gone: its `.git` file, read as Git reads it, names a folder
 * in a Git folder's `worktrees` that is not there where Git reaches it ({@link gitFolderReached}). Its main checkout was
 * deleted or moved away, or Git pruned the worktree's entry, and the clone's id file, kept in the common Git folder, may
 * have gone too; Git, asked about the folder, reads the `.git` file first and refuses it. A `.git` that is no regular
 * file is never opened. False while the environment moves Git's lookup, and wherever the disk cannot tell. Two lstats
 * and one read of a small file, and the real path {@link gitFolderReached} reads for a path with a `..`.
 */
export function worktreeGitFolderGone(folder: string): boolean {
  if (environmentMovesLookup()) return false;
  const at = join(folder, ".git");
  const entry = look(at);
  const written = isEntry(entry) ? gitFileNames(at, entry)?.written : undefined;
  const reached = written === undefined ? undefined : gitFolderReached(folder, written);
  return reached !== undefined && basename(dirname(reached)) === "worktrees" && look(reached) === "none";
}

/** What splits a path a `.git` file holds into names: on Windows a backslash as well as a slash. */
const NAME_SEPARATOR = process.platform === "win32" ? /[\\/]/u : "/";

/**
 * The Git folder Git reaches for the path `written` that the `.git` file in `folder` holds, asked from `folder` as
 * spelled, in a spelling with no `.` or `..` left: an lstat of it, a name joined to it as text and its own names are
 * those of the folder Git reaches. Every reader of such a path takes it: the placement ({@link gitFolderOf}), the gone
 * check and its `worktrees` name ({@link worktreeGitFolderGone}), and the facts an acceptance is kept under ({@link
 * namedBehind}). Git joins a relative path to the folder it changed into, by its real path, and opens the joined text.
 * With no `..`, every spelling of that text reaches one folder, so it is joined and cleaned as text and no real path is
 * read: a path as `git worktree add` writes it is one. On Linux and macOS the system follows each link before the `..`
 * after it, so the text up to its last `..` is read by its real path, one read ({@link climbedInTurn}); on Windows,
 * which takes each `..` off the text before it follows a junction or a link, an absolute path needs no read and a
 * relative one one ({@link climbedOnWindows}). Undefined where that real path cannot be read: Git is then asked.
 */
export function gitFolderReached(folder: string, written: string): string | undefined {
  const names = written.split(NAME_SEPARATOR);
  const climb = names.lastIndexOf("..");
  if (climb === -1) return resolve(folder, written);
  return process.platform === "win32" ? climbedOnWindows(folder, written) : climbedInTurn(folder, names, climb);
}

/**
 * {@link gitFolderReached} on Linux and macOS of `names`, whose last `..` is at `climb`: the text up to it, joined to
 * `folder` when relative, read by its native real path, as the system resolves it, and the names after it joined to
 * that as text. `resolve` would cancel a link and the `..` after it as text, where Git leaves the folder the link leads
 * to. Undefined where that real path cannot be read: then the system, too, reaches no folder by that text.
 */
function climbedInTurn(folder: string, names: readonly string[], climb: number): string | undefined {
  const upTo = names.slice(0, climb + 1).join("/");
  try {
    return resolve(realpathNamesInTurn(isAbsolute(upTo) ? upTo : `${folder}/${upTo}`), ...names.slice(climb + 1));
  } catch {
    return undefined;
  }
}

/**
 * {@link gitFolderReached} on Windows of a path with a `..`: Windows takes it off the text before it follows a link, so
 * an absolute path is cleaned as text, with no real path read, and a relative one is joined to `folder`'s real path,
 * where Git for Windows changed into. Undefined where that real path cannot be read.
 */
function climbedOnWindows(folder: string, written: string): string | undefined {
  if (isAbsolute(written)) return resolve(written);
  const real = realOf(folder);
  return real === undefined ? undefined : resolve(real, written);
}

function withCommon(folder: string, entry: FoundGitFolder, common: CommonFolder): PlacementInputs | undefined {
  const facts = gitFolderFacts(folder, entry.gitFolder, common.linked ?? entry.gitFolder);
  if (facts === undefined) return undefined;
  const { realFolder, realCommon, takenByGit, config } = facts;
  const owned = ownedAsGitTakes(realFolder, entry.gitFolder, entry.entries);
  return {
    folder,
    stamp: [realFolder, realCommon, entry.stamp, `owned:${owned}`, common.stamp, `taken:${takenByGit}`, config.signature].join("\n"),
    gitFile: entry.file,
    gitFolder: entry.gitFolder,
    linkedCommon: common.linked,
    owned,
    ...facts,
  };
}

type GitFolderFacts = Pick<PlacementInputs, "realFolder" | "realCommon" | "takenByGit" | "config">;

/** What the Git folder and the common one tell of `folder`, read now; undefined when one cannot be read. */
function gitFolderFacts(folder: string, gitFolder: string, common: string): GitFolderFacts | undefined {
  const config = configOf(gitFolder, common);
  const real = config === undefined ? undefined : realPaths(folder, common);
  if (config === undefined || real === undefined) return undefined;
  const takenByGit = gitFolderTaken(gitFolder, common);
  return takenByGit === undefined ? undefined : { ...real, takenByGit, config };
}

/** The real paths of `folder` and of `common`, spelled as Git prints them; undefined when one cannot be read. */
function realPaths(folder: string, common: string): { readonly realFolder: string; readonly realCommon: string } | undefined {
  try {
    return { realFolder: storedPath(folder), realCommon: storedPath(common) };
  } catch {
    return undefined;
  }
}

/**
 * Whether Git takes `gitFolder` for a Git folder, as it does before it places a checkout there: a `HEAD` it takes
 * ({@link validHead}), and `objects` and `refs` in the common Git folder that Git can reach ({@link reachable}).
 * Undefined where `HEAD` is a link, which Git reads by its target's name and which is left to Git.
 */
function gitFolderTaken(gitFolder: string, common: string): boolean | undefined {
  const head = validHead(gitFolder);
  if (head === undefined) return undefined;
  return head && reachable(join(common, "objects")) && reachable(join(common, "refs"));
}

/**
 * Whether Git's `access(path, X_OK)` passes, as Git checks `objects` and `refs`: through a link, as Git follows one, so
 * a link whose folder goes is told from one whose folder is there. On Windows it asks only whether the path is there,
 * as Git for Windows does.
 */
function reachable(path: string): boolean {
  try {
    accessSync(path, fsConstants.X_OK);
    return true;
  } catch {
    return false;
  }
}

/**
 * Git's answer for the linked worktree `inputs` describe, read from its files: its top level is the folder and its
 * common Git folder the one `commondir` names, each spelled as Git prints it (the real path, names as stored). Only for
 * a repository Git already placed a checkout of in this process (`gitPlaced`, given that common folder's real path), so
 * Git has read its format and taken its folders once. Undefined wherever the files do not decide it as Git would, so
 * Git is asked ({@link readAsGitReads}).
 */
export function linkedWorktreeFromFiles(inputs: PlacementInputs, gitPlaced: (common: string) => boolean): LinkedPlacement | undefined {
  if (inputs.linkedCommon === undefined || !readAsGitReads(inputs)) return undefined;
  return gitPlaced(inputs.realCommon) ? { top: inputs.realFolder, commonDirectory: inputs.realCommon } : undefined;
}

/**
 * Whether the files decide the placement as Git would: the entry is a `.git` file; Git takes the Git folder it names
 * ({@link PlacementInputs.takenByGit}), whose `gitdir` names this folder's `.git` back; the common config sets no
 * extension and a format every Git reads; and Git takes the folders for the current user's ({@link ownedAsGitTakes}).
 */
function readAsGitReads(inputs: PlacementInputs): boolean {
  return inputs.gitFile && inputs.config.plainFormat && inputs.takenByGit && inputs.owned && namesBack(inputs);
}

/**
 * Whether Git takes `gitFolder/HEAD` for a `HEAD`: a regular file whose first bytes, as many as Git reads, name a ref
 * under `refs/` or an object id ({@link headTaken}). Undefined for a link, which Git takes by the name it holds.
 */
function validHead(gitFolder: string): boolean | undefined {
  const path = join(gitFolder, "HEAD");
  const looked = look(path);
  if (looked === undefined || (isEntry(looked) && looked.isSymbolicLink())) return undefined;
  return looked !== "none" && headTaken(fileText(path));
}

/**
 * How much of a `HEAD` Git reads to take it (`validate_headref`): a ref named past it is not taken. The text Git can
 * take is ASCII up to the end of `refs/` or of the object id, so these characters are the bytes Git reads.
 */
const HEAD_READ = 255;

/**
 * Whether Git takes this `HEAD` text: `ref:`, then only a space, a tab or line ends (Git's `isspace`, not a vertical tab,
 * a form feed or a Unicode space), then `refs/`; or an object id. Read in its first {@link HEAD_READ} characters.
 */
function headTaken(head: FileText | "none" | undefined): boolean {
  const text = isText(head) ? head.text.slice(0, HEAD_READ) : undefined;
  return text !== undefined && (/^ref:[\t\n\r ]*refs\//u.test(text) || /^(?:[0-9a-f]{64}|[0-9a-f]{40})/iu.test(text));
}

/**
 * Whether the Git folder's `gitdir` names this folder's `.git`, as `git worktree add`, `move` and `repair` leave it.
 * Git places the folder without reading it; a worktree moved by hand, or a `.git` file written to name another
 * worktree's Git folder, disagrees here and is left to Git.
 */
function namesBack(inputs: PlacementInputs): boolean {
  const read = fileText(join(inputs.gitFolder, "gitdir"));
  const named = isText(read) ? readPath(read.text) : undefined;
  if (named === undefined) return false;
  try {
    return storedPath(resolve(inputs.gitFolder, named)) === storedPath(join(inputs.folder, ".git"));
  } catch {
    return false;
  }
}

/**
 * Whether Git, which refuses a repository whose folders another user owns unless `safe.directory` names it, takes the
 * folder, its `.git` entry and the Git folder that entry is or names (`entries`: the lstats of the last two) as the
 * current user's. Git reads the folder it changed into by its real path, so a link's own owner never stands in for the
 * folder's: `realFolder` is that real path. On POSIX it compares their owner with the effective user as Git does
 * ({@link ownedByUser}). Windows gives Node no owner: there the engine reads each one's owner as Git for Windows does
 * ({@link ownersTaken}), and a network path, or no engine that can tell, counts as another's. No Git process: one
 * lstat, and on Windows one engine call.
 */
function ownedAsGitTakes(realFolder: string, gitFolder: string, entries: readonly Stats[]): boolean {
  const top = look(realFolder);
  if (!isEntry(top)) return false;
  if (process.platform !== "win32") return [top, ...entries].every(stats => ownedByUser(stats.uid));
  return ownersTaken([realFolder, join(realFolder, ".git"), gitFolder]) === true;
}

/**
 * Whether Git for Windows takes every one of `paths` as the user's, read by the engine ({@link pathsOwnedByCurrentUser}):
 * false for a network path, undefined when no engine can tell.
 */
function ownersTaken(paths: readonly string[]): boolean | undefined {
  return paths.every(plainlyResolved) ? pathsOwnedByCurrentUser(paths) : false;
}

/**
 * The facts Git reads to place `folder`, raw, for a folder {@link filePlacementInputs} cannot describe (a `.git` link, a
 * Git folder reached through a link, a `HEAD` link, a file that cannot be read): one string that changes whenever one of
 * them does, under which the worktree registry keeps Git's own acceptance of the folder. It holds the folder's real
 * path; whether Git takes the folder, the `.git` entry and the Git folder for the user's; the entry's own type and
 * identity, and its link text; what the entry resolves to through links, and for a `.git` file its text and the real
 * path of the Git folder it names ({@link gitFolderBehind}); and that Git folder's facts ({@link gitFolderRaw}). Each
 * is read now; a part that cannot be read is held as such. Undefined when the folder's real path, or on Windows the
 * owners, cannot be read.
 */
export function gitEntryFacts(folder: string): string | undefined {
  const realFolder = realOf(folder);
  if (realFolder === undefined) return undefined;
  const dotGit = join(realFolder, ".git");
  const behind = gitFolderBehind(realFolder, dotGit);
  const owned = ownersAsGitReads(realFolder, dotGit, behind.gitFolder ?? dotGit);
  return owned === undefined ? undefined : [realFolder, `owned:${owned}`, behind.facts, gitFolderRaw(behind.gitFolder)].join("\n");
}

/** `path`'s real path, through every link; undefined when it cannot be read (a link whose target is gone, say). */
export function realOf(path: string): string | undefined {
  try {
    return checkoutRealpath(path);
  } catch {
    return undefined;
  }
}

/** An entry's type, permissions and identity on its volume, as an lstat gives them; `none` or `?` when it gives none. */
function described(looked: Looked): string {
  if (looked === undefined) return "?";
  return looked === "none" ? "none" : `${looked.mode}:${looked.dev}:${looked.ino}`;
}

/** {@link described} of the entry at `path`, read now: one lstat. */
function entryFacts(path: string): string {
  return described(look(path));
}

/** What a folder holds where Git's search for a repository looks: its `.git` entry and its own `HEAD`, as lstats gave them. */
interface FolderLook {
  readonly dotGit: string;
  readonly entry: Looked;
  readonly head: Looked;
}

function lookIn(folder: string): FolderLook {
  const dotGit = join(folder, ".git");
  return { dotGit, entry: look(dotGit), head: look(join(folder, "HEAD")) };
}

/**
 * What Git's search for a repository reads in `folder`, read now: the `.git` entry's facts ({@link described}) and what
 * it holds ({@link heldIn}), and the facts of the folder's own `HEAD`, which a bare repository has. Two lstats where
 * nothing is there, a third in a `.git` folder, a readlink more for a `.git` link, and no Git process.
 */
export function searchedIn(folder: string): string {
  return entriesOf(lookIn(folder));
}

/** {@link searchedIn} of a folder, and whether Git's search for a repository ends there ({@link searchEnds}). */
export interface SearchStep {
  readonly entries: string;
  readonly ends: boolean;
}

/** {@link SearchStep} of `folder`, both read from the same lstats. */
export function searchStep(folder: string): SearchStep {
  const looked = lookIn(folder);
  return { entries: entriesOf(looked), ends: searchEnds(folder, looked) };
}

function entriesOf({ dotGit, entry, head }: FolderLook): string {
  return `${described(entry)}:${heldIn(dotGit, entry)} ${described(head)}`;
}

/**
 * Whether Git's search for a repository ends in `folder`, read as Git's `setup.c` decides it from `looked`: its `.git`,
 * reached through any link, is a file (Git reads it, or stops at it with an error) or a folder Git takes for a Git
 * folder ({@link takenAsGitFolder}); or the folder holds a `HEAD` and is itself one, a bare repository. Never sooner
 * than Git: where the files cannot tell what Git takes (a `HEAD` link, a `commondir` that cannot be read), it answers
 * no, and the read goes on to the folder above. A lookup variable needs no exception: under `GIT_DIR` Git searches no
 * folder, and under `GIT_OBJECT_DIRECTORY` or `GIT_COMMON_DIR` Git reads `objects` or `refs` there for every folder, so
 * a folder these files take and Git does not means Git takes none, and its answer is not kept.
 */
function searchEnds(folder: string, looked: FolderLook): boolean {
  if (gitEntryEnds(looked)) return true;
  return isEntry(looked.head) && takenAsGitFolder(folder);
}

function gitEntryEnds({ dotGit, entry }: FolderLook): boolean {
  const reached = isEntry(entry) && entry.isSymbolicLink() ? reach(dotGit) : entry;
  if (isEntry(reached) && reached.isFile()) return true;
  return isFolder(reached) && takenAsGitFolder(dotGit);
}

/**
 * Whether Git's search takes `path` for a Git folder (`is_git_directory`): a `HEAD` it takes ({@link validHead}), and
 * `objects` and `refs` it can reach in the common Git folder `commondir` names ({@link commonOf}), else in `path`
 * itself. On Linux and macOS a `..` in `commondir` leaves the folder a `.git` link reaches, as Git resolves it.
 */
function takenAsGitFolder(path: string): boolean {
  const common = validHead(path) === true ? commonOf(path) : undefined;
  const at = common === undefined ? undefined : (common.linked ?? path);
  return at !== undefined && reachable(join(at, "objects")) && reachable(join(at, "refs"));
}

/**
 * What the `.git` entry `dotGit`, whose lstat is `looked`, holds where Git's search reads it: the `objects` folder in a
 * folder, or the text of a file (rewritten in place to name another Git folder); for a link (or junction), the link's
 * own text, which a link pointed at another Git folder moves even where the new link gets the freed file id of the one
 * it replaced. `git init` makes `objects` as it fills an empty `.git` folder where it is; a branch switch, a commit, a
 * rebase, a fetch or a gc in that repository leaves its facts as they were, where each writes `HEAD` anew. Nothing is
 * read through a link: a kept ask reads this at every tool call, and a link can lead to a share on another machine
 * that stops answering, where a read would hold the daemon's thread until the share answers.
 */
function heldIn(dotGit: string, looked: Looked): string {
  if (isEntry(looked) && looked.isSymbolicLink()) return `link:${linkText(dotGit)}`;
  if (isFolder(looked)) return entryFacts(join(dotGit, "objects"));
  return isEntry(looked) && looked.isFile() ? textFact(fileText(dotGit)) : "-";
}

/** A stat of `path` through every link, as Git's search reads a `.git` entry: `"none"` where nothing is reached. */
function reach(path: string): Looked {
  try {
    return statSync(path, { throwIfNoEntry: false }) ?? "none";
  } catch {
    return undefined;
  }
}

function linkText(path: string): string {
  try {
    return readlinkSync(path);
  } catch {
    return "?";
  }
}

function textFact(read: FileText | "none" | undefined): string {
  return isText(read) ? `text:${read.text}` : (read ?? "?");
}

/** What a `.git` entry is and the real path of the Git folder Git reaches through it. */
interface Behind {
  readonly facts: string;
  readonly gitFolder: string | undefined;
}

/**
 * The `.git` entry in `realFolder`, as Git reads it: its own facts and link text, the real path it resolves to and that
 * entry's facts; for a folder there, that folder is the Git folder ({@link namedBehind} for a file).
 */
function gitFolderBehind(realFolder: string, dotGit: string): Behind {
  const entry = look(dotGit);
  const target = realOf(dotGit);
  const resolved = target === undefined ? undefined : look(target);
  const facts = [described(entry), isEntry(entry) && entry.isSymbolicLink() ? linkText(dotGit) : "-", target ?? "?", described(resolved)].join("\n");
  return isFolder(resolved) ? { facts, gitFolder: target } : namedBehind(realFolder, dotGit, facts, resolved);
}

/**
 * A `.git` entry that resolves to a file: its text, and the real path of the Git folder it names, which Git reaches
 * from the folder holding the `.git` entry, not from where a link took it ({@link gitFolderReached}).
 */
function namedBehind(realFolder: string, dotGit: string, facts: string, resolved: Looked): Behind {
  const read = isEntry(resolved) && resolved.isFile() ? fileText(dotGit) : undefined;
  const named = isText(read) && read.text.startsWith(GITDIR) ? readPath(read.text.slice(GITDIR.length)) : undefined;
  return { facts: `${facts}\n${textFact(read)}`, gitFolder: named === undefined ? undefined : realGitFolder(realFolder, named) };
}

/** The real path of the Git folder {@link gitFolderReached} gives; undefined where either cannot be read. */
function realGitFolder(folder: string, written: string): string | undefined {
  const reached = gitFolderReached(folder, written);
  return reached === undefined ? undefined : realOf(reached);
}

/**
 * A Git folder's facts as Git reads them to place a checkout: its identity, what Git takes of its `HEAD` ({@link
 * headRaw}), `commondir`, the lines of `config.worktree` that place a checkout ({@link configRaw}), and the common Git
 * folder's ({@link commonRaw}).
 */
function gitFolderRaw(gitFolder: string | undefined): string {
  if (gitFolder === undefined) return "-";
  const commondir = fileText(join(gitFolder, "commondir"));
  return [gitFolder, described(look(gitFolder)), headRaw(join(gitFolder, "HEAD")), textFact(commondir), configRaw(join(gitFolder, "config.worktree")), commonRaw(gitFolder, commondir)].join("\n");
}

/**
 * The lines of a config file that place a checkout, as a checkout its files describe is kept under ({@link
 * ConfigFacts.signature}); `?` when it cannot be read. A remote, an upstream or a user name written there moves nothing.
 */
function configRaw(path: string): string {
  return configFactsAt(path)?.signature ?? "?";
}

/**
 * What Git reads of a `HEAD` to take its folder for a Git folder: for a link, whether its text names a path under
 * `refs/`; else whether it is a file Git takes ({@link headTaken}). The commit or branch it names is left out, since Git
 * places the folder alike whichever it is: a commit at a detached `HEAD`, or a switch to another branch, moves nothing.
 */
function headRaw(head: string): string {
  const looked = look(head);
  if (isEntry(looked) && looked.isSymbolicLink()) return `link:${linkText(head).startsWith("refs/")}`;
  return `taken:${headTaken(fileText(head))}`;
}

/**
 * The common Git folder `commondir` names (else the Git folder): its real path, identity, `objects`, `refs` and the
 * lines of `config` that place a checkout ({@link configRaw}).
 */
function commonRaw(gitFolder: string, commondir: FileText | "none" | undefined): string {
  const named = isText(commondir) ? readPath(commondir.text) : undefined;
  const linked = named === undefined ? gitFolder : commonFolderOf(gitFolder, named);
  const common = linked === undefined ? undefined : realOf(linked);
  if (common === undefined) return "?";
  const reached = `objects:${reachable(join(common, "objects"))} refs:${reachable(join(common, "refs"))}`;
  return [common, described(look(common)), reached, configRaw(join(common, "config"))].join("\n");
}

/**
 * Whether Git takes the real folder, its `.git` entry and the Git folder for the user's, read now: on POSIX each one's
 * lstat owner ({@link ownedByUser}), on Windows the engine's read ({@link ownersTaken}); undefined when no engine can tell.
 */
function ownersAsGitReads(realFolder: string, dotGit: string, gitFolder: string): boolean | undefined {
  const paths = [realFolder, dotGit, gitFolder];
  if (process.platform === "win32") return ownersTaken(paths);
  return paths.every(path => {
    const looked = look(path);
    return isEntry(looked) && ownedByUser(looked.uid);
  });
}

/**
 * Whether Git takes an entry its lstat gives `owner` for the effective user's: the same id, or for root an entry root
 * owns or one owned by the id `SUDO_UID` holds.
 */
function ownedByUser(owner: number): boolean {
  const user = process.geteuid?.();
  return owner === user || (user === 0 && owner === sudoUser());
}

/** The id `SUDO_UID` holds, when it is one Git reads as a number. */
function sudoUser(): number | undefined {
  const value = process.env["SUDO_UID"];
  return value !== undefined && /^\d{1,9}$/u.test(value) ? Number(value) : undefined;
}

/** What a Git config says about where Git places a checkout, read as Git reads its own repository's config. */
export interface ConfigFacts {
  /** The lines that can move Git's answer (`core.bare`, `core.worktree`, the format, any extension, any line Git may refuse), in order. */
  readonly signature: string;
  /** A format every Git release reads alike (none, 0 or 1), no extension, and no line Git may refuse. */
  readonly plainFormat: boolean;
}

/**
 * Each config file's facts with the text they were read from, so an unchanged text is not parsed again, and the folder
 * holding the file in its {@link storedForm}, read with a new text: a `.git` file naming its Git folder by a relative
 * path spells the file's path from the folder as it was asked, through a link or in another letter case. A file's
 * entry leaves once the file is found gone ({@link forgetGoneConfigs}).
 */
const configs = new Map<string, { readonly text: string; readonly facts: ConfigFacts; readonly folder: string }>();

/**
 * Drops the facts kept for each config file in `common`, a common Git folder in its {@link storedForm}, or below it,
 * that is gone, such as the `config.worktree` of a worktree Git removed from it, however the file's path was spelt.
 * One lstat per file kept there, and none for another folder's.
 */
export function forgetGoneConfigs(common: string): void {
  for (const [path, { folder }] of configs) if (`${folder}${sep}`.startsWith(`${common}${sep}`) && look(path) === "none") configs.delete(path);
}

/** How many config files' facts are kept: for tests, which count what a removed worktree leaves behind. */
export function keptConfigCount(): number {
  return configs.size;
}

/** {@link ConfigFacts} of the config file at `path`, read now; undefined when it cannot be read. */
function configFactsAt(path: string): ConfigFacts | undefined {
  const read = fileText(path);
  if (!isText(read)) return read === "none" ? { signature: "none", plainFormat: true } : undefined;
  const known = configs.get(path);
  if (known?.text === read.text) return known.facts;
  const facts = configFacts(read.text);
  configs.set(path, { text: read.text, facts, folder: storedForm(dirname(path)) });
  return facts;
}

/**
 * The config Git reads to place a checkout: the common folder's `config`, and with `extensions.worktreeConfig` set
 * there (by `git sparse-checkout`, say) the Git folder's own `config.worktree` too, whose `core.bare` and
 * `core.worktree` Git takes over the common ones. Git reads no included file to place a checkout. Undefined when one
 * cannot be read.
 */
function configOf(gitFolder: string, common: string): ConfigFacts | undefined {
  const shared = configFactsAt(join(common, "config"));
  if (shared === undefined || !shared.signature.split("\n").some(line => line.startsWith("extensions.worktreeconfig="))) return shared;
  const own = configFactsAt(join(gitFolder, "config.worktree"));
  return own === undefined ? undefined : { signature: `${shared.signature}\n[config.worktree]\n${own.signature}`, plainFormat: false };
}

/** A section header: a name, then a quoted subsection; a comment may follow. */
const SECTION = /^\[([A-Za-z0-9.-]+)(\s+"(?:[^"\\]|\\.)*")?\]\s*(?:[#;].*)?$/u;
/** A key, then `=` and its value, or nothing (a boolean that is true). */
const KEY = /^([A-Za-z][A-Za-z0-9-]*)\s*(?:=(.*))?$/u;
/** The keys of `core` that move a checkout's top level, or how Git reads the repository. */
const PLACING_CORE_KEYS = new Set(["bare", "worktree", "repositoryformatversion"]);

/** {@link ConfigFacts} of a config file's text. */
export function configFacts(text: string): ConfigFacts {
  let section = "";
  const placing: string[] = [];
  for (const line of text.split("\n").map(raw => raw.trim()).filter(meaningful)) {
    const header = SECTION.exec(line);
    if (header === null) placing.push(...placingLine(section, line));
    else section = `${header[1]!.toLowerCase()}${header[2] === undefined ? "" : " sub"}`;
  }
  return { signature: placing.join("\n"), plainFormat: placing.every(plainLine) };
}

function meaningful(line: string): boolean {
  return line !== "" && !line.startsWith("#") && !line.startsWith(";");
}

/** The signature line a key line adds in `section`: none for a key that does not place; `?` for a line Git may refuse. */
function placingLine(section: string, line: string): string[] {
  const key = KEY.exec(line);
  const value = key === null ? undefined : configValue(key[2] ?? "");
  return key === null || value === undefined ? [`? ${line}`] : placingKey(section, key[1]!.toLowerCase(), value);
}

function placingKey(section: string, name: string, value: string): string[] {
  if (section.startsWith("extensions")) return [`extensions.${name}=${value}`];
  return section === "core" && PLACING_CORE_KEYS.has(name) ? [`core.${name}=${value}`] : [];
}

/** Whether a signature line leaves the format plain: `core.bare`, `core.worktree`, or a format version of 0 or 1. */
function plainLine(line: string): boolean {
  if (line.startsWith("core.repositoryformatversion=")) return /^core\.repositoryformatversion=[01]$/u.test(line);
  return line.startsWith("core.");
}

/** The escapes Git reads in a config value. */
const ESCAPES: Readonly<Record<string, string>> = { "\\": "\\", "\"": "\"", n: "\n", t: "\t", b: "\b" };

/**
 * A config value as Git reads it (quotes removed, escapes applied, a comment cut, the ends trimmed); undefined where
 * Git refuses it (an escape it does not know, a quote left open) or where it goes on to the next line.
 */
function configValue(text: string): string | undefined {
  let value = "";
  let quoted = false;
  for (let index = 0; index < text.length; index += 1) {
    const step = valueStep(text, index, quoted);
    if (step === undefined) return undefined;
    if (step === "end") break;
    value += step.add;
    quoted = step.quoted;
    index += step.skip;
  }
  return quoted ? undefined : value.trim();
}

/** One character of a config value: what it adds, whether a quote is open after it, and how many more it takes. */
function valueStep(text: string, index: number, quoted: boolean): { add: string; quoted: boolean; skip: number } | "end" | undefined {
  const char = text[index]!;
  if (char === "\\") {
    const escaped = ESCAPES[text[index + 1] ?? ""];
    return escaped === undefined ? undefined : { add: escaped, quoted, skip: 1 };
  }
  if (char === "\"") return { add: "", quoted: !quoted, skip: 0 };
  if (!quoted && "#;".includes(char)) return "end";
  return { add: char, quoted, skip: 0 };
}
