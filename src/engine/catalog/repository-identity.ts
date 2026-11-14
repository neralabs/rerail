import { randomUUID } from "node:crypto";
import {
  existsSync,
  chmodSync,
  closeSync,
  constants as fsConstants,
  fstatSync,
  linkSync,
  lstatSync,
  mkdirSync,
  readdirSync,
  readFileSync,
  readlinkSync,
  readSync,
  statSync,
  unlinkSync,
  writeFileSync,
  type Stats,
} from "node:fs";
import { basename, dirname, join, resolve } from "node:path";
import { physicalPath, storedPath } from "./checkout-control.js";
import { openNoFollowSync } from "../no-follow.js";
import { GitUnavailableError, gitText, runGit } from "../git.js";
import type { RepositoryKey } from "./repository-model.js";

/** Where a checkout keeps its clone id, and which checkout it is. */
export interface RepositoryIdentityLocation {
  git: boolean;
  /** The checkout's top folder: the main checkout's, or a linked worktree's own. */
  workspaceRoot: string;
  identityPath: string;
  /** False only for a linked Git worktree; a folder outside Git is its own main checkout. */
  main: boolean;
}

function nonGitLocation(requested: string): RepositoryIdentityLocation {
  return { git: false, workspaceRoot: requested, identityPath: join(requested, ".rerail", "repository-id"), main: true };
}

/**
 * Where `workspace`'s clone id lives: `rerail-repository-id` in Git's common metadata directory, shared by every
 * linked worktree, or `.rerail/repository-id` outside Git (or when Git is not installed).
 */
export function repositoryIdentityLocation(workspace: string): RepositoryIdentityLocation {
  const requested = resolve(workspace);
  const result = locateInGit(requested);
  return result === undefined || result.status !== 0 ? nonGitLocation(requested) : gitLocation(requested, result.stdout);
}

/** Git's top level, common directory and Git directory for `requested`; undefined when Git is not installed. */
function locateInGit(requested: string): ReturnType<typeof runGit> | undefined {
  try {
    return runGit(requested, ["rev-parse", "--show-toplevel", "--path-format=absolute", "--git-common-dir", "--git-dir"]);
  } catch (error) {
    if (error instanceof GitUnavailableError) return undefined;
    throw error;
  }
}

function gitLocation(requested: string, stdout: string): RepositoryIdentityLocation {
  const [top, commonDirectory, gitDirectory] = stdout.trim().split("\n").map(line => resolve(line.trim()));
  if (top === undefined || commonDirectory === undefined || gitDirectory === undefined) {
    throw new Error(`could not resolve Git common directory for ${requested}`);
  }
  return checkoutLocation(top, commonDirectory, gitDirectory);
}

function checkoutLocation(top: string, commonDirectory: string, gitDirectory: string): RepositoryIdentityLocation {
  return { git: true, workspaceRoot: top, identityPath: join(commonDirectory, "rerail-repository-id"), main: commonDirectory === gitDirectory };
}

/**
 * Where Git's own search for the checkout of `workspace` can end, read from the filesystem alone: no Git runs and no
 * environment variable is read. Git looks in the folder and in each one above it for a `.git` and stops at the first one
 * it takes, so the search ends at that folder's checkout, or outside Git where something that folder does not hold
 * refuses the checkout (`safe.directory`, a ceiling, a filesystem boundary, `core.bare`); with no `.git` above, it ends
 * outside Git. Each end is the location {@link repositoryIdentityLocation} gives there, for the folder's real path, which
 * is what a hook asks the daemon about. Undefined where the filesystem leaves the end open: a folder with no real path, a
 * `.git` Git would pass over or refuse, a Git folder on the way (inside `.git`, a bare repository), or a `core.worktree`
 * that puts the checkout elsewhere.
 */
export function gitSearchEnds(workspace: string): RepositoryIdentityLocation[] | undefined {
  const requested = realFolder(workspace);
  if (requested === undefined) return undefined;
  const top = nearestGitTop(requested);
  if (top === "open") return undefined;
  return top === undefined ? [nonGitLocation(requested)] : [top, nonGitLocation(requested)];
}

/** `workspace` as a hook asks the daemon about it, its real path spelled as stored; undefined where it has none. */
function realFolder(workspace: string): string | undefined {
  try {
    return existsSync(workspace) ? storedPath(workspace) : undefined;
  } catch {
    return undefined;
  }
}

/** The checkout Git's search from `start` up finds: undefined where it finds none, "open" where the filesystem leaves it open. */
function nearestGitTop(start: string): RepositoryIdentityLocation | "open" | undefined {
  for (let folder = start; ; folder = dirname(folder)) {
    const step = dotGitIn(folder);
    if (step.kind === "taken") return checkoutLocation(folder, step.commonDirectory, step.gitDirectory);
    // A folder that is a Git folder itself (inside `.git`, a bare repository) ends Git's search with no checkout.
    if (step.kind === "open" || existsSync(join(folder, "HEAD"))) return "open";
    if (dirname(folder) === folder) return undefined;
  }
}

/** What a `.git` gives Git's search in one folder: nothing, a Git folder Git takes, or one the filesystem cannot settle. */
type DotGit = { readonly kind: "none" | "open" } | { readonly kind: "taken"; readonly gitDirectory: string; readonly commonDirectory: string };

const NO_DOT_GIT: DotGit = { kind: "none" };
const OPEN_DOT_GIT: DotGit = { kind: "open" };

/** The `.git` in `folder`: the `.git` folder itself, or the folder a `gitdir:` file names, when Git takes it. */
function dotGitIn(folder: string): DotGit {
  const dotGit = join(folder, ".git");
  const kind = entryKind(dotGit);
  if (kind === "none") return NO_DOT_GIT;
  const gitDirectory = kind === "folder" ? dotGit : kind === "file" ? gitfileTarget(dotGit) : undefined;
  return gitDirectory === undefined ? OPEN_DOT_GIT : takenGitDirectory(gitDirectory);
}

/** `path` followed through links, as Git's `stat` follows them: missing, a folder, a file, or anything else. */
function entryKind(path: string): "none" | "folder" | "file" | "other" {
  let stats: Stats | undefined;
  try {
    stats = statSync(path, { throwIfNoEntry: false });
  } catch {
    return "other";
  }
  if (stats === undefined) return "none";
  if (stats.isDirectory()) return "folder";
  return stats.isFile() ? "file" : "other";
}

/** A `gitdir:` file as Git reads it: the whole file, its line ends trimmed. */
const GITFILE = /^gitdir: (.+?)[\r\n]*$/;

/** The folder a `gitdir:` file names, relative to the file's own folder; undefined where Git refuses the file. */
function gitfileTarget(file: string): string | undefined {
  let text: string;
  try {
    text = readFileSync(file, "utf8");
  } catch {
    return undefined;
  }
  const named = GITFILE.exec(text)?.[1];
  return named === undefined ? undefined : resolve(dirname(file), named);
}

/**
 * `gitDirectory` and its common folder where Git takes it as the Git folder of the checkout its `.git` is in: a HEAD Git
 * reads and `objects` and `refs` in the common folder (Git's `is_git_directory`), and no `core.worktree` in the
 * repository's own config. Otherwise open: Git passes over such a `.git` folder and refuses such a file.
 */
function takenGitDirectory(gitDirectory: string): DotGit {
  const commonDirectory = commonDirectoryOf(gitDirectory);
  return commonDirectory !== undefined && isGitDirectory(gitDirectory, commonDirectory) && !setsWorktree(gitDirectory, commonDirectory)
    ? { kind: "taken", gitDirectory, commonDirectory }
    : OPEN_DOT_GIT;
}

/** Git's `is_git_directory`: a HEAD Git reads in `gitDirectory`, and `objects` and `refs` in its common folder. */
function isGitDirectory(gitDirectory: string, commonDirectory: string): boolean {
  return headGitReads(join(gitDirectory, "HEAD")) && isFolder(join(commonDirectory, "objects")) && isFolder(join(commonDirectory, "refs"));
}

/** The common folder Git reads for `gitDirectory`: the one its `commondir` file names, else itself; undefined where Git cannot read that file. */
function commonDirectoryOf(gitDirectory: string): string | undefined {
  const file = join(gitDirectory, "commondir");
  if (!existsSync(file)) return gitDirectory;
  try {
    const named = readFileSync(file, "utf8").replace(/[\r\n]+$/, "");
    return named === "" ? undefined : resolve(gitDirectory, named);
  } catch {
    return undefined;
  }
}

/** A HEAD file Git reads: a ref under `refs/` (spaces as Git's `isspace` allows after `ref:`) or a commit id. */
const HEAD_TEXT = /^(?:ref:[ \t\n\r]*refs\/|[0-9a-fA-F]{40})/;

/** Whether Git reads `head` as a HEAD (`validate_headref`): a link into `refs/`, or a file {@link HEAD_TEXT} matches. */
function headGitReads(head: string): boolean {
  try {
    if (lstatSync(head).isSymbolicLink()) return readlinkSync(head).startsWith("refs/");
    return HEAD_TEXT.test(readFileSync(head, "utf8"));
  } catch {
    return false;
  }
}

function isFolder(path: string): boolean {
  try {
    return statSync(path).isDirectory();
  } catch {
    return false;
  }
}

/** A `worktree` key anywhere in a config file; a commented-out one counts too, which only sends more folders to Git. */
const WORKTREE_KEY = /\bworktree\s*=/i;

/** Whether the repository's own config (`config`, and a linked worktree's `config.worktree`) sets `core.worktree`. */
function setsWorktree(gitDirectory: string, commonDirectory: string): boolean {
  return [join(commonDirectory, "config"), join(gitDirectory, "config.worktree")].some(file => WORKTREE_KEY.test(textOf(file)));
}

function textOf(file: string): string {
  try {
    return readFileSync(file, "utf8");
  } catch {
    return "";
  }
}

/** A recorded checkout's Git registry, as `git worktree list` last gave it, and what the registry then held. */
const worktreeRegistries = new Map<string, WorktreeRegistry & { readonly contents: string }>();

export interface WorktreeRegistry {
  /** The common directory Git reads for the checkout that was asked, physical. */
  readonly commonDirectory: string;
  /** The main checkout and each linked worktree `git worktree add` made, as Git reports them. */
  readonly worktrees: readonly string[];
}

/**
 * Each registered worktree's name and where it says it is: `git worktree add`, `remove`, `move` and `repair` all change
 * it. The `worktrees` folder's own time would miss the last two, which rewrite a file inside an entry.
 */
function registryContents(commonDirectory: string): string {
  const folder = join(commonDirectory, "worktrees");
  let names: string[];
  try {
    names = readdirSync(folder);
  } catch {
    return "";
  }
  return names.toSorted().map(name => {
    try {
      return `${name}\0${readFileSync(join(folder, name, "gitdir"), "utf8")}`;
    } catch {
      return name;
    }
  }).join("\0");
}

/**
 * The registry of `checkout`, a recorded checkout: its own common directory and what Git lists there. Undefined when Git
 * cannot answer in it (it is gone, say). A `.git` file naming that directory is not listed, nor is a worktree moved
 * without `git worktree move` or `repair`. Git's answer is kept until the registry's contents change, so `.git` files a
 * repository plants cost Git one call, not one per file per tool call; only the recorded checkout decides what is kept,
 * never a folder asking to be listed. Line output because `-z` needs Git 2.36, which distributions Rerail runs on do not
 * have yet.
 */
export function worktreeRegistry(checkout: string): WorktreeRegistry | undefined {
  const commonDirectory = registryCommonDirectory(checkout);
  // Read before Git runs: a worktree added meanwhile changes it, and Git is asked again next time.
  const contents = commonDirectory === undefined ? undefined : registryContents(commonDirectory);
  const known = worktreeRegistries.get(checkout);
  if (known !== undefined && known.contents === contents) return known;
  const registry = contents === undefined ? undefined : listWorktrees(checkout, commonDirectory!, contents);
  // A checkout that is gone, or where Git cannot answer, keeps nothing: what is kept is bounded by what answers.
  if (registry === undefined) worktreeRegistries.delete(checkout);
  else worktreeRegistries.set(checkout, registry);
  return registry;
}

/** The physical common directory `checkout` reads, as Git first gave it; undefined when the folder is gone or not in Git. */
function registryCommonDirectory(checkout: string): string | undefined {
  if (!existsSync(checkout)) return undefined;
  const known = worktreeRegistries.get(checkout);
  if (known !== undefined) return known.commonDirectory;
  const location = repositoryIdentityLocation(checkout);
  return location.git ? physicalPath(dirname(location.identityPath)) : undefined;
}

function listWorktrees(checkout: string, commonDirectory: string, contents: string): WorktreeRegistry & { readonly contents: string } | undefined {
  const result = runGit(checkout, ["worktree", "list", "--porcelain"]);
  if (result.status !== 0) return undefined;
  const worktrees = result.stdout.split("\n").filter(line => line.startsWith("worktree ")).map(line => line.slice("worktree ".length));
  return { commonDirectory, worktrees, contents };
}

/** The repository's own folder name: the main checkout's folder for a linked worktree, else the checkout's folder. */
export function repositoryFolderName(location: RepositoryIdentityLocation): string {
  const commonDirectory = dirname(location.identityPath);
  return location.git && basename(commonDirectory) === ".git" ? basename(dirname(commonDirectory)) : basename(location.workspaceRoot);
}

/** {@link repositoryFolderName} of the checkout `workspace` is in. */
export function repositoryName(workspace: string): string {
  return repositoryFolderName(repositoryIdentityLocation(workspace));
}

function errorCode(error: unknown): string | undefined {
  return typeof error === "object" && error !== null && "code" in error && typeof error.code === "string" ? error.code : undefined;
}

/** The clone id `workspace` already has, without creating one; passive readers use it. */
export function readCloneId(workspace: string): string | undefined {
  return readCloneIdAt(repositoryIdentityLocation(workspace).identityPath);
}

/**
 * A clone id: `git:<uuid>` or `<uuid>` today; ids written before (and `rerail.repository-id` config values) are
 * short tokens of letters, digits, `.`, `_`, `:` and `-`. Anything else is refused rather than trusted.
 */
const CLONE_ID = /^[A-Za-z0-9][A-Za-z0-9._:-]{0,99}$/;
const MAX_IDENTITY_BYTES = 128;

/**
 * The identity file's id, read without following a symlink, only from a regular file, and at most
 * {@link MAX_IDENTITY_BYTES}; undefined when it does not exist. Throws on a symlink, a non-file or a malformed id.
 */
function readIdentityFile(identityPath: string): string | undefined {
  let descriptor: number;
  try {
    descriptor = openNoFollowSync(identityPath, fsConstants.O_RDONLY);
  } catch (error) {
    if (errorCode(error) === "ENOENT") return undefined;
    throw new Error(`Rerail repository identity at ${identityPath} is not a regular file`);
  }
  try {
    if (!fstatSync(descriptor).isFile()) throw new Error(`Rerail repository identity at ${identityPath} is not a regular file`);
    const buffer = Buffer.alloc(MAX_IDENTITY_BYTES);
    const length = readSync(descriptor, buffer, 0, MAX_IDENTITY_BYTES, 0);
    const cloneId = buffer.subarray(0, length).toString("utf8").trim();
    if (!CLONE_ID.test(cloneId)) throw new Error(`malformed Rerail repository identity at ${identityPath}`);
    return cloneId;
  } finally {
    closeSync(descriptor);
  }
}

/** The clone id stored at `identityPath`, or undefined before one is created or when the file can't be trusted. */
export function readCloneIdAt(identityPath: string): string | undefined {
  try {
    return readIdentityFile(identityPath);
  } catch {
    return undefined;
  }
}

/** Publish complete identity bytes with an atomic no-clobber link. */
function readOrCreateIdentity(identityPath: string, candidate: string): string {
  const existing = readIdentityFile(identityPath);
  if (existing !== undefined) return existing;
  const temporaryPath = `${identityPath}.${process.pid}.${randomUUID()}.tmp`;
  writeFileSync(temporaryPath, `${candidate}\n`, { encoding: "utf8", flag: "wx", mode: 0o600 });
  try {
    try {
      linkSync(temporaryPath, identityPath);
    } catch (error) {
      if (errorCode(error) !== "EEXIST") throw error;
    }
  } finally {
    unlinkSync(temporaryPath);
  }
  const cloneId = readIdentityFile(identityPath);
  if (cloneId === undefined) throw new Error(`Rerail repository identity at ${identityPath} disappeared`);
  chmodSync(identityPath, 0o600);
  return cloneId;
}

/**
 * `workspace`'s clone id, created on first use and stored atomically in Git's common metadata directory. Linked
 * worktrees share it; clones and forks receive a new one; a legacy `rerail.repository-id` Git config value is adopted
 * when it is a well-formed id. Folders outside Git use the same file contract under `.rerail`, which must not be a
 * symlink.
 */
export function resolveCloneId(workspace: string): { cloneId: string; location: RepositoryIdentityLocation } {
  const location = repositoryIdentityLocation(workspace);
  if (location.git) {
    const legacyId = gitText(location.workspaceRoot, ["config", "--local", "--get", "rerail.repository-id"]) ?? "";
    return { cloneId: readOrCreateIdentity(location.identityPath, CLONE_ID.test(legacyId) ? legacyId : `git:${randomUUID()}`), location };
  }
  const directory = dirname(location.identityPath);
  // Created first, so two processes recording the same folder at once both succeed; whichever made it, it must be a
  // real directory, never a symlink (lstat).
  try {
    mkdirSync(directory, { mode: 0o700 });
  } catch (error) {
    if (errorCode(error) !== "EEXIST") throw error;
    if (!lstatSync(directory).isDirectory()) throw new Error(`${directory} is not a directory Rerail can keep its identity in`);
  }
  return { cloneId: readOrCreateIdentity(location.identityPath, randomUUID()), location };
}

export interface RemoteAddress {
  name: string;
  url: string;
}

/** The checkout's `origin` remote, else its only remote; undefined with no remote or several and no `origin`. */
export function readRemoteAddress(root: string): RemoteAddress | undefined {
  const origin = originUrl(root);
  return origin === undefined ? soleRemote(root) : { name: "origin", url: origin };
}

/** `origin`'s address; `ls-remote --get-url` gives the name back when no remote has it. */
function originUrl(root: string): string | undefined {
  const origin = gitText(root, ["ls-remote", "--get-url", "origin"]);
  return origin === undefined || origin === "" || origin === "origin" ? undefined : origin;
}

function soleRemote(root: string): RemoteAddress | undefined {
  const names = (gitText(root, ["remote"]) ?? "").split("\n").map(name => name.trim()).filter(name => name.length > 0);
  if (names.length !== 1) return undefined;
  const [name] = names as [string];
  const url = gitText(root, ["remote", "get-url", "--", name]);
  return url === undefined || url === "" ? undefined : { name, url };
}

const SCHEME = /^[A-Za-z][A-Za-z0-9+.-]*:\/\//;
const SCP_FORM = /^(?:[^@/]+@)?([^:/]+):(.*)$/;

function splitAddress(address: string): { host: string; path: string } | undefined {
  return SCHEME.test(address) ? splitUrl(address) : splitScpForm(address);
}

/** A `scheme://[user@]host[:port]/path` address; a `file://` one is a local path. */
function splitUrl(address: string): { host: string; path: string } | undefined {
  if (/^file:\/\//i.test(address)) return undefined;
  const rest = address.replace(SCHEME, "");
  const slash = rest.indexOf("/");
  const authority = slash === -1 ? rest : rest.slice(0, slash);
  const host = authority.slice(authority.lastIndexOf("@") + 1).replace(/:\d*$/, "");
  return { host, path: slash === -1 ? "" : rest.slice(slash) };
}

/** A `[user@]host:path` address. */
function splitScpForm(address: string): { host: string; path: string } | undefined {
  const scp = SCP_FORM.exec(address);
  // A single-letter "host" is a Windows drive (`C:/repo`, `C:\repo`), which is a local path.
  if (scp === null || scp[1]!.length === 1) return undefined;
  return { host: scp[1]!, path: scp[2]! };
}

/**
 * `remote:<host>/<path>` for a Git remote address, so the ssh, scp and https forms of one repository agree:
 * scheme, `user@` and port dropped, `//` collapsed, trailing `/`, `.git`, `/` stripped, lowercased. A `file://`
 * or local path gives undefined. An ssh `Host` alias stays its own host.
 */
export function normalizeRemoteAddress(url: string): RepositoryKey | undefined {
  const parts = splitAddress(url.trim());
  if (parts === undefined) return undefined;
  const path = parts.path.replace(/\/{2,}/g, "/").replace(/^\//, "").replace(/\/$/, "").replace(/\.git$/i, "").replace(/\/$/, "");
  if (parts.host === "" || path === "") return undefined;
  return `remote:${parts.host}/${path}`.toLowerCase();
}

/** The repository key: the normalised remote address when there is one, else `clone:<clone id>`. */
export function repositoryKeyOf(remote: RemoteAddress | undefined, cloneId: string): RepositoryKey {
  return (remote === undefined ? undefined : normalizeRemoteAddress(remote.url)) ?? `clone:${cloneId}`;
}
