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
  openSync,
  readdirSync,
  readFileSync,
  readSync,
  unlinkSync,
  writeFileSync,
  type Stats,
} from "node:fs";
import { basename, dirname, join, resolve } from "node:path";
import { physicalPath, storedPath } from "./checkout-control.js";
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

/** A checkout's three folders as Git names them: its top level, its Git folder and the common Git folder. */
interface GitFolders {
  readonly top: string;
  readonly gitDirectory: string;
  readonly commonDirectory: string;
}

/**
 * Git's answer for each folder {@link repositoryIdentityLocation} was asked about, with the key of the files Git read
 * to give it ({@link discoveryOf}). One map for the process: the daemon's runtimes, its owner lookups and its checkout
 * records all ask about the same few hundred folders, and each folder then costs one Git process until a file Git
 * read for it changes, not one per call. Past {@link MAX_KEPT_LOCATIONS} folders the one kept longest goes first.
 */
const keptLocations = new Map<string, { readonly key: string; readonly location: RepositoryIdentityLocation }>();
const MAX_KEPT_LOCATIONS = 2_048;

/**
 * Where `workspace`'s clone id lives: `rerail-repository-id` in Git's common metadata directory, shared by every
 * linked worktree, or `.rerail/repository-id` outside Git (or when Git is not installed). Git answers; its answer is
 * kept and given again while the key of the files it read is unchanged. Only an answer whose three folders are those
 * the files lead to is kept, so a folder outside Git, a failed or timed-out Git, and a top level Git found by another
 * way (`core.worktree`, `GIT_DIR`, a link on the way) ask Git on every call.
 */
export function repositoryIdentityLocation(workspace: string): RepositoryIdentityLocation {
  const requested = resolve(workspace);
  // Read before Git runs: a file that changes while Git runs gives the next call another key, which asks Git again.
  const discovered = discoveryOf(requested);
  const kept = keptLocations.get(requested);
  if (kept !== undefined && kept.key === discovered?.key) return { ...kept.location };
  keptLocations.delete(requested);
  const result = locateInGit(requested);
  if (result === undefined || result.status !== 0) return nonGitLocation(requested);
  const folders = gitFolders(requested, result.stdout);
  const location = gitLocation(folders);
  if (discovered !== undefined && sameFolders(discovered, folders)) keepLocation(requested, discovered.key, location);
  return location;
}

function keepLocation(requested: string, key: string, location: RepositoryIdentityLocation): void {
  keptLocations.set(requested, { key, location: { ...location } });
  if (keptLocations.size <= MAX_KEPT_LOCATIONS) return;
  for (const oldest of keptLocations.keys()) {
    keptLocations.delete(oldest);
    return;
  }
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

function gitFolders(requested: string, stdout: string): GitFolders {
  const [top, commonDirectory, gitDirectory] = stdout.trim().split("\n").map(line => resolve(line.trim()));
  if (top === undefined || commonDirectory === undefined || gitDirectory === undefined) {
    throw new Error(`could not resolve Git common directory for ${requested}`);
  }
  return { top, gitDirectory, commonDirectory };
}

function gitLocation({ top, gitDirectory, commonDirectory }: GitFolders): RepositoryIdentityLocation {
  return { git: true, workspaceRoot: top, identityPath: join(commonDirectory, "rerail-repository-id"), main: commonDirectory === gitDirectory };
}

/** Whether Git found the folders the files lead to, compared as the filesystem stores them (links resolved, case as on disk). */
function sameFolders(discovered: GitFolders, answered: GitFolders): boolean {
  return storedPath(discovered.top) === storedPath(answered.top)
    && storedPath(discovered.gitDirectory) === storedPath(answered.gitDirectory)
    && storedPath(discovered.commonDirectory) === storedPath(answered.commonDirectory);
}

/** The most bytes of a `.git` file Rerail reads to key it; Git reads a larger one, but its answer is not kept. */
const MAX_GIT_FILE_BYTES = 4_096;

/**
 * The folders the files at and above `folder` lead to, followed as Git follows them, and the key of every file Git
 * reads on the way. The nearest folder at or above `folder` with a `.git` entry is the top level. The `.git` folder,
 * or the folder a `.git` file names (`gitdir: <path>`, relative to the top level), is the Git folder; the folder its
 * `commondir` file names (relative to it) is the common one, else the Git folder itself. The key holds the top level,
 * the `.git` entry (a folder, or the file's text), `commondir`'s text, whether the Git folder holds a `HEAD` Git
 * accepts and the common one `objects` and `refs` (Git refuses a folder without them), and the `core` and `extensions`
 * sections of the common `config` and the Git folder's `config.worktree` ({@link formatSections}): `core.worktree`
 * moves the top level, `core.bare` takes it away, and an extension this Git does not know makes it refuse the
 * repository. A remote, a branch or an upstream written to `config` leaves the key as it was. Each file is read by one
 * call that closes it before returning. Undefined where Git would read what the key leaves out: no `.git` entry up to
 * the root, a `.git` that is a link or a file naming no folder, a `config` that includes another file, or a file that
 * could not be read.
 */
function discoveryOf(folder: string): (GitFolders & { readonly key: string }) | undefined {
  try {
    const nearest = nearestGitEntry(folder);
    const entry = nearest === undefined ? undefined : gitEntry(nearest.top, nearest.entry);
    if (nearest === undefined || entry === undefined) return undefined;
    const { top } = nearest;
    const gitDirectory = resolve(top, entry.gitDirectory);
    const common = textOrAbsent(join(gitDirectory, "commondir"));
    const commonDirectory = common === undefined ? gitDirectory : resolve(gitDirectory, gitPathLine(common));
    const config = formatSections(join(commonDirectory, "config"));
    const worktreeConfig = formatSections(join(gitDirectory, "config.worktree"));
    if (config === undefined || worktreeConfig === undefined) return undefined;
    const holds = [acceptedHead(join(gitDirectory, "HEAD")), present(join(commonDirectory, "objects")), present(join(commonDirectory, "refs"))];
    const key = [top, entry.text, common ?? "!ENOENT", holds.join(","), config, worktreeConfig].join("\0");
    return { top, gitDirectory, commonDirectory, key };
  } catch {
    return undefined;
  }
}

/** The nearest folder at or above `folder` holding a `.git` entry of any kind, and that entry; undefined when none does up to the root. */
function nearestGitEntry(folder: string): { readonly top: string; readonly entry: Stats } | undefined {
  for (let directory = folder; ; directory = dirname(directory)) {
    const entry = lstatSync(join(directory, ".git"), { throwIfNoEntry: false });
    if (entry !== undefined) return { top: directory, entry };
    if (dirname(directory) === directory) return undefined;
  }
}

/** The `.git` entry of `top`: a folder, or a file naming the Git folder; undefined for a link, or a file naming none. */
function gitEntry(top: string, entry: Stats): { readonly gitDirectory: string; readonly text: string } | undefined {
  const path = join(top, ".git");
  if (entry.isDirectory()) return { gitDirectory: path, text: "!EISDIR" };
  if (!entry.isFile()) return undefined;
  const text = textOrAbsent(path, MAX_GIT_FILE_BYTES) ?? "";
  const named = text.startsWith("gitdir: ") ? gitPathLine(text.slice("gitdir: ".length)) : "";
  return named === "" ? undefined : { gitDirectory: named, text };
}

/** A path Git keeps in a file of its own (`.git`, `commondir`): the text with its line ends trimmed, as Git trims them. */
function gitPathLine(text: string): string {
  return text.replace(/[\r\n]+$/u, "");
}

/** The most bytes of a `commondir` or config file Rerail reads to key it; past it, Git is asked on every call. */
const MAX_KEYED_FILE_BYTES = 1024 * 1024;

/**
 * The text of the regular file at `path`, undefined when there is none. Throws for anything else: a FIFO or a device
 * (a `.git` file can name any folder, and reading a FIFO there would block the daemon), a file past `maxBytes`, or a
 * read that failed. Opened without blocking and closed before it returns.
 */
function textOrAbsent(path: string, maxBytes = MAX_KEYED_FILE_BYTES): string | undefined {
  let descriptor: number;
  try {
    descriptor = openSync(path, fsConstants.O_RDONLY | (fsConstants.O_NONBLOCK ?? 0));
  } catch (error) {
    if (errorCode(error) === "ENOENT") return undefined;
    throw error;
  }
  try {
    const info = fstatSync(descriptor);
    if (!info.isFile() || info.size > maxBytes) throw new Error(`${path} is not a file Rerail keys a checkout's location by`);
    return readFileSync(descriptor, "utf8");
  } finally {
    closeSync(descriptor);
  }
}

function present(path: string): boolean {
  return lstatSync(path, { throwIfNoEntry: false }) !== undefined;
}

/** A `HEAD` Git accepts (`validate_headref` in Git's setup.c): a symbolic ref under `refs/`, or an object name. */
const GIT_HEAD = /^(?:ref:\s*refs\/|[0-9a-f]{40})/iu;

/**
 * Whether the `HEAD` file at `path` is one Git accepts, without which Git does not take its folder for a Git folder. Only
 * whether: the branch it names moves with every switch, which changes nothing Git answers here.
 */
function acceptedHead(path: string): boolean {
  const text = textOrAbsent(path, MAX_GIT_FILE_BYTES);
  return text !== undefined && GIT_HEAD.test(text);
}

/**
 * The `core` and `extensions` sections of the Git config file at `path`, one trimmed line each, `!ENOENT` without the
 * file; undefined when it includes another file (`include` or `includeIf`), whose lines Rerail does not follow.
 */
function formatSections(path: string): string | undefined {
  const text = textOrAbsent(path);
  if (text === undefined) return "!ENOENT";
  let kept = false;
  const lines: string[] = [];
  for (const line of text.split("\n")) {
    // A section header: `[core]`, `[extensions]`, `[branch "main"]`, or the older `[branch.main]`.
    const section = /^\s*\[([^\s\]".]*)/u.exec(line)?.[1]?.toLowerCase();
    if (section === "include" || section === "includeif") return undefined;
    if (section !== undefined) kept = section === "core" || section === "extensions";
    if (kept && line.trim() !== "") lines.push(line.trim());
  }
  return lines.join("\n");
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
    descriptor = openSync(identityPath, fsConstants.O_RDONLY | (fsConstants.O_NOFOLLOW ?? 0));
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

/** Publish complete identity bytes with an atomic no-clobber link; `candidate` is asked only when there is no identity yet. */
function readOrCreateIdentity(identityPath: string, candidate: () => string): string {
  const existing = readIdentityFile(identityPath);
  if (existing !== undefined) return existing;
  const temporaryPath = `${identityPath}.${process.pid}.${randomUUID()}.tmp`;
  writeFileSync(temporaryPath, `${candidate()}\n`, { encoding: "utf8", flag: "wx", mode: 0o600 });
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
    // Git's config is read only for a checkout with no identity file yet: once the file exists, it is the id.
    const candidate = (): string => {
      const legacyId = gitText(location.workspaceRoot, ["config", "--local", "--get", "rerail.repository-id"]) ?? "";
      return CLONE_ID.test(legacyId) ? legacyId : `git:${randomUUID()}`;
    };
    return { cloneId: readOrCreateIdentity(location.identityPath, candidate), location };
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
  return { cloneId: readOrCreateIdentity(location.identityPath, () => randomUUID()), location };
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
