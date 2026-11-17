/**
 * Where Git places a folder that holds a `.git` entry, read from the files Git itself reads to place it: the entry, the
 * Git folder it names, that folder's `HEAD`, `gitdir` and `commondir`, and the common Git folder's `objects`, `refs` and
 * `config`. The daemon places every registered worktree of a repository when the repository starts, and a Git process
 * per worktree there held its one thread for seconds to minutes on a slow Windows machine. A linked worktree is placed
 * from these files once Git has placed a checkout of its repository; any other entry (a clone's own `.git` folder, a
 * submodule's, a Git folder kept apart), and any linked worktree whose files Git might read otherwise, is left to Git,
 * whose answer `repositoryIdentityLocation` keeps while these files are unchanged ({@link PlacementInputs.stamp}).
 */
import { lstatSync, readFileSync, type Stats } from "node:fs";
import { join, resolve } from "node:path";
import { pathsOwnedByCurrentUser } from "../../native/native.js";
import { storedPath } from "../checkout-control.js";

/** What Git reads to place a folder holding a `.git` entry, as it was when read. */
export interface PlacementInputs {
  /** The folder holding the `.git` entry, resolved. */
  readonly folder: string;
  /**
   * One string that changes when any of these inputs changes in a way that can move Git's answer: the entry, the Git
   * folder it names, that folder's `commondir`, and what the common `config` says of the format and the work tree. A
   * Git folder's own times are left out, since every index write moves them.
   */
  readonly stamp: string;
  /** Whether the entry is a `.git` file, as a linked worktree's is. */
  readonly gitFile: boolean;
  /** The Git folder the entry is or names, resolved. */
  readonly gitFolder: string;
  /** The common Git folder its `commondir` names, resolved; undefined when it has none (a clone's own Git folder). */
  readonly linkedCommon: string | undefined;
  readonly config: ConfigFacts;
}

/** Git's answer for a linked worktree: its top level and its common Git folder, as Git prints them. */
export interface LinkedPlacement {
  readonly top: string;
  readonly commonDirectory: string;
}

/**
 * The environment variables that move Git's lookup away from the folder's own `.git` entry, or make Git refuse every
 * folder: with any of them set nothing is read from files or kept, and Git is asked each time.
 */
const LOOKUP_VARIABLES = [
  "GIT_DIR",
  "GIT_WORK_TREE",
  "GIT_COMMON_DIR",
  "GIT_OBJECT_DIRECTORY",
  "GIT_CEILING_DIRECTORIES",
  "GIT_IMPLICIT_WORK_TREE",
  "GIT_TEST_ASSUME_DIFFERENT_OWNER",
];

function environmentMovesLookup(): boolean {
  return LOOKUP_VARIABLES.some(name => process.env[name] !== undefined);
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
  readonly stamp: string;
  readonly text: string;
}

function isText(read: FileText | "none" | undefined): read is FileText {
  return read !== undefined && read !== "none";
}

/** Git refuses a `.git` file larger than this; the other files read here are smaller still. */
const LARGEST_READ = 1 << 20;

/** The small files read here, each read again only once its lstat changed. */
const texts = new Map<string, FileText>();

/** A regular file's text and stamp, `"none"` when nothing is there, undefined when it is something else or unreadable. */
function fileText(path: string): FileText | "none" | undefined {
  const looked = look(path);
  if (!isEntry(looked)) return looked;
  if (!looked.isFile() || looked.size > LARGEST_READ) return undefined;
  const stamp = `${looked.ino}:${looked.size}:${looked.mtimeMs}:${looked.ctimeMs}`;
  const known = texts.get(path);
  return known?.stamp === stamp ? known : readText(path, stamp);
}

function readText(path: string, stamp: string): FileText | undefined {
  try {
    const read = { stamp, text: readFileSync(path, "utf8") };
    texts.set(path, read);
    return read;
  } catch {
    return undefined;
  }
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

const GITDIR = "gitdir: ";

/** The Git folder a `.git` entry is, or the one a `.git` file names, as Git reads the file. */
function gitFolderOf(folder: string, entry: Stats): NamedGitFolder | undefined {
  const at = join(folder, ".git");
  if (entry.isDirectory()) return { gitFolder: at, stamp: `d:${entry.ino}`, file: false };
  const read = entry.isFile() ? fileText(at) : undefined;
  if (!isText(read) || !read.text.startsWith(GITDIR)) return undefined;
  const named = readPath(read.text.slice(GITDIR.length));
  return named === undefined ? undefined : { gitFolder: resolve(folder, named), stamp: `f:${read.stamp}`, file: true };
}

/** {@link gitFolderOf} the `.git` entry in `folder`, when the Git folder is there; its stamp adds that folder's identity. */
function gitFolderAt(folder: string): NamedGitFolder | undefined {
  const entry = look(join(folder, ".git"));
  const named = isEntry(entry) ? gitFolderOf(folder, entry) : undefined;
  const gitFolder = named === undefined ? undefined : look(named.gitFolder);
  if (named === undefined || !isFolder(gitFolder)) return undefined;
  return { ...named, stamp: `${named.stamp}\n${gitFolder.ino}` };
}

/** The common Git folder `gitFolder`'s `commondir` names (undefined `linked` without one); undefined when unreadable. */
function commonOf(gitFolder: string): { readonly linked: string | undefined; readonly stamp: string } | undefined {
  const read = fileText(join(gitFolder, "commondir"));
  if (read === "none") return { linked: undefined, stamp: "-" };
  const named = isText(read) ? readPath(read.text) : undefined;
  return named === undefined || !isText(read) ? undefined : { linked: resolve(gitFolder, named), stamp: read.stamp };
}

/**
 * What Git reads to place `folder`, when it holds a `.git` folder or file whose Git folder is there; undefined when it
 * holds none, when an input cannot be read or is shaped in a way not modelled here, or when the environment moves
 * Git's lookup. Four lstats once the files are known.
 */
export function placementInputs(folder: string): PlacementInputs | undefined {
  const entry = environmentMovesLookup() ? undefined : gitFolderAt(folder);
  return entry === undefined ? undefined : withCommon(folder, entry);
}

function withCommon(folder: string, entry: NamedGitFolder): PlacementInputs | undefined {
  const common = commonOf(entry.gitFolder);
  const config = common === undefined ? undefined : configFactsAt(common.linked ?? entry.gitFolder);
  if (common === undefined || config === undefined) return undefined;
  return {
    folder,
    stamp: [entry.stamp, common.stamp, config.signature].join("\n"),
    gitFile: entry.file,
    gitFolder: entry.gitFolder,
    linkedCommon: common.linked,
    config,
  };
}

/**
 * Git's answer for the linked worktree `inputs` describe, read from its files: its top level is the folder and its
 * common Git folder the one `commondir` names, each spelled as Git prints it (the real path, names as stored). Only for
 * a repository Git already placed a checkout of in this process (`gitPlaced`, given that common folder's real path), so
 * Git has read its format and taken its folders once. Undefined wherever the files do not decide it as Git would, so
 * Git is asked ({@link readAsGitReads}).
 */
export function linkedWorktreeFromFiles(inputs: PlacementInputs, gitPlaced: (common: string) => boolean): LinkedPlacement | undefined {
  const common = inputs.linkedCommon;
  if (common === undefined || !readAsGitReads(inputs, common)) return undefined;
  const placed = realPlacement(inputs.folder, common);
  return placed !== undefined && gitPlaced(placed.commonDirectory) ? placed : undefined;
}

/**
 * Whether the files decide the placement as Git would: the entry is a `.git` file; the Git folder it names has a `HEAD`
 * Git takes and a `gitdir` that names this folder's `.git` back; the common config sets no extension and a format every
 * Git reads; and Git takes the folders for the current user's ({@link ownedAsGitRequires}).
 */
function readAsGitReads(inputs: PlacementInputs, common: string): boolean {
  return inputs.gitFile && inputs.config.plainFormat && validHead(inputs.gitFolder) && namesBack(inputs) && ownedAsGitRequires(inputs, common);
}

function realPlacement(folder: string, common: string): LinkedPlacement | undefined {
  try {
    return { top: storedPath(folder), commonDirectory: storedPath(common) };
  } catch {
    return undefined;
  }
}

/** Whether Git takes `gitFolder/HEAD` for a `HEAD`: a regular file naming a ref under `refs/`, or an object id. */
function validHead(gitFolder: string): boolean {
  const head = fileText(join(gitFolder, "HEAD"));
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
 * Whether Git takes the linked worktree's folders as the current user's ({@link ownedAsGitTakes}), and the common folder
 * holds the `objects` and `refs` Git looks for.
 */
function ownedAsGitRequires(inputs: PlacementInputs, common: string): boolean {
  if (!isFolder(look(join(common, "objects"))) || !isFolder(look(join(common, "refs")))) return false;
  return ownedAsGitTakes(inputs);
}

/**
 * Whether Git, which refuses a repository whose folders another user owns unless `safe.directory` names it, takes the
 * folder, its `.git` entry and the Git folder that entry is or names as the current user's. On POSIX it compares their
 * owner with the effective user, as Git does (root is left to Git, which reads `SUDO_UID`). Windows gives Node no
 * owner: there the engine reads each one's owner as Git for Windows does ({@link pathsOwnedByCurrentUser}), and a
 * network path, or no engine that can tell, leaves the folder to Git. No Git process: a few lstats, and on Windows one
 * engine call.
 */
export function ownedAsGitTakes(inputs: PlacementInputs): boolean {
  const paths = [inputs.folder, join(inputs.folder, ".git"), inputs.gitFolder];
  const owned = paths.map(look);
  if (!owned.every(isEntry)) return false;
  return process.platform === "win32" ? paths.every(plainlyResolved) && pathsOwnedByCurrentUser(paths) === true : ownedByUser(owned);
}

function ownedByUser(owned: readonly Stats[]): boolean {
  const user = process.geteuid?.();
  return user !== undefined && user !== 0 && owned.every(stats => stats.uid === user);
}

/** What a Git config says about where Git places a checkout, read as Git reads its own repository's config. */
export interface ConfigFacts {
  /** The lines that can move Git's answer (`core.bare`, `core.worktree`, the format, any extension, any line Git may refuse), in order. */
  readonly signature: string;
  /** A format every Git release reads alike (none, 0 or 1), no extension, and no line Git may refuse. */
  readonly plainFormat: boolean;
}

const configs = new Map<string, { readonly stamp: string; readonly facts: ConfigFacts }>();

/** {@link ConfigFacts} of `<common>/config`, read again only once it changed; undefined when it cannot be read. */
function configFactsAt(common: string): ConfigFacts | undefined {
  const path = join(common, "config");
  const read = fileText(path);
  if (!isText(read)) return read === "none" ? { signature: "none", plainFormat: true } : undefined;
  const known = configs.get(path);
  if (known?.stamp === read.stamp) return known.facts;
  const facts = configFacts(read.text);
  configs.set(path, { stamp: read.stamp, facts });
  return facts;
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
