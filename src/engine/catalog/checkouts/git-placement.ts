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
  type Stats,
} from "node:fs";
import { join, resolve } from "node:path";
import { pathsOwnedByCurrentUser } from "../../native/native.js";
import { checkoutRealpath, storedPath } from "../checkout-control.js";

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

/** The Git folder a `.git` entry is, or the one a `.git` file names, as Git reads the file. */
function gitFolderOf(folder: string, entry: Stats): NamedGitFolder | undefined {
  const at = join(folder, ".git");
  if (entry.isDirectory()) return { gitFolder: at, stamp: `d:${entry.ino}`, file: false };
  const read = entry.isFile() ? fileText(at) : undefined;
  if (!isText(read) || !read.text.startsWith(GITDIR)) return undefined;
  const named = readPath(read.text.slice(GITDIR.length));
  return named === undefined ? undefined : { gitFolder: resolve(folder, named), stamp: `f:${read.text}`, file: true };
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
  readonly stamp: string;
}

/** The common Git folder `gitFolder`'s `commondir` names (undefined `linked` without one); undefined when unreadable. */
function commonOf(gitFolder: string): CommonFolder | undefined {
  const read = fileText(join(gitFolder, "commondir"));
  if (read === "none") return { linked: undefined, stamp: "-" };
  const named = isText(read) ? readPath(read.text) : undefined;
  return named === undefined ? undefined : { linked: resolve(gitFolder, named), stamp: named };
}

/**
 * What Git reads to place `folder`, when it holds a `.git` folder or file whose Git folder is there; undefined when it
 * holds none, when an input cannot be read or is shaped in a way not modelled here, or when the environment moves
 * Git's lookup. A few lstats, reads of small files and two real paths, and no Git process.
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
 * Whether Git takes `gitFolder/HEAD` for a `HEAD`: a regular file naming a ref under `refs/`, or an object id.
 * Undefined for a link, which Git takes by the name it holds.
 */
function validHead(gitFolder: string): boolean | undefined {
  const path = join(gitFolder, "HEAD");
  const looked = look(path);
  if (looked === undefined || (isEntry(looked) && looked.isSymbolicLink())) return undefined;
  return looked !== "none" && headTaken(fileText(path));
}

function headTaken(head: FileText | "none" | undefined): boolean {
  return isText(head) && (/^ref:\s*refs\//u.test(head.text) || /^(?:[0-9a-f]{64}|[0-9a-f]{40})/iu.test(head.text));
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
export function entryFacts(path: string): string {
  return described(look(path));
}

/**
 * What Git's search for a repository reads at the `.git` entry `dotGit`, read now: the entry's facts ({@link
 * described}), and the `objects` folder in it when it is a folder, or its text when it is a file (rewritten in place to
 * name another Git folder). `git init` makes `objects` as it fills an empty `.git` folder where it is; a branch switch,
 * a commit, a rebase, a fetch or a gc in that repository leaves its facts as they were, where each writes `HEAD` anew.
 * One lstat where nothing is there.
 */
export function searchedEntryFacts(dotGit: string): string {
  const looked = look(dotGit);
  return `${described(looked)}:${heldIn(dotGit, looked)}`;
}

function heldIn(dotGit: string, looked: Looked): string {
  if (isFolder(looked)) return entryFacts(join(dotGit, "objects"));
  return isEntry(looked) && looked.isFile() ? textFact(fileText(dotGit)) : "-";
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
 * A `.git` entry that resolves to a file: its text, and the real path of the Git folder it names, which Git resolves
 * from the folder holding the `.git` entry, not from where a link took it.
 */
function namedBehind(realFolder: string, dotGit: string, facts: string, resolved: Looked): Behind {
  const read = isEntry(resolved) && resolved.isFile() ? fileText(dotGit) : undefined;
  const named = isText(read) && read.text.startsWith(GITDIR) ? readPath(read.text.slice(GITDIR.length)) : undefined;
  return { facts: `${facts}\n${textFact(read)}`, gitFolder: named === undefined ? undefined : realOf(resolve(realFolder, named)) };
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
  const common = named === undefined ? gitFolder : realOf(resolve(gitFolder, named));
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

/** Each config file's facts with the text they were read from, so an unchanged text is not parsed again. */
const configs = new Map<string, { readonly text: string; readonly facts: ConfigFacts }>();

/** {@link ConfigFacts} of the config file at `path`, read now; undefined when it cannot be read. */
function configFactsAt(path: string): ConfigFacts | undefined {
  const read = fileText(path);
  if (!isText(read)) return read === "none" ? { signature: "none", plainFormat: true } : undefined;
  const known = configs.get(path);
  if (known?.text === read.text) return known.facts;
  const facts = configFacts(read.text);
  configs.set(path, { text: read.text, facts });
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
