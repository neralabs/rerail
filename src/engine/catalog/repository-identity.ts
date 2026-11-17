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
  readSync,
  unlinkSync,
  writeFileSync,
} from "node:fs";
import { basename, dirname, join, resolve } from "node:path";
import { physicalPath, storedPath } from "./checkout-control.js";
import { linkedWorktreeFromFiles, ownedAsGitTakes, placementInputs, type PlacementInputs } from "./checkouts/git-placement.js";
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

const IDENTITY_FILE = "rerail-repository-id";

/** A kept answer, and the generation of {@link checkOwnersAgain} its folders' owners were last read at. */
interface HeldTopLevel {
  readonly stamp: string;
  readonly location: RepositoryIdentityLocation;
  ownersRead: number;
}

/**
 * Git's answers for the folders it took for their own checkout's top level, each kept while what Git read to give it
 * is unchanged ({@link PlacementInputs.stamp}) and Git would still take its folders as the user's: a daemon asks after
 * every registered worktree each time a repository starts, in every session's runtime, and once a checkout is recorded
 * every runtime asks again.
 */
const heldTopLevels = new Map<string, HeldTopLevel>();

/** The recorded checkouts' version the current owner generation started at, and that generation. */
let ownersVersion: string | undefined;
let ownersGeneration = 0;

/**
 * Has each kept answer's owners read again at its next use, once per change of the recorded checkouts (`version`,
 * the same string for every runtime of a daemon): Git refuses a checkout whose folders another account came to own, or
 * that a `safe.directory` entry no longer names, and before answers were kept each such change asked Git again. The
 * stamp holds no owner, so a change of one alone shows only here. Costs a few lstats per kept answer, and on Windows one
 * engine call; Git is asked only for a folder whose owners Git would not take.
 */
export function checkOwnersAgain(version: string): void {
  if (version === ownersVersion) return;
  ownersVersion = version;
  ownersGeneration += 1;
}

/** Whether `held` still answers for `inputs`: the same stamp, and owners read in this generation or read again now. */
function stillAnswers(held: HeldTopLevel, inputs: PlacementInputs): boolean {
  if (held.stamp !== inputs.stamp) return false;
  if (held.ownersRead !== ownersGeneration && !ownedAsGitTakes(inputs)) return false;
  held.ownersRead = ownersGeneration;
  return true;
}

/** The common Git folders of the checkouts Git placed in this process, as real paths: Git read each one's format and took its folders. */
const gitPlacedCommons = new Set<string>();

/**
 * Where `workspace`'s clone id lives: `rerail-repository-id` in Git's common metadata directory, shared by every
 * linked worktree, or `.rerail/repository-id` outside Git (or when Git is not installed). A folder that holds a `.git`
 * entry is answered as Git last answered while what Git reads there is unchanged, and a linked worktree of a repository
 * Git placed a checkout of is answered from its files ({@link linkedWorktreeFromFiles}); every other folder, and every
 * case those files do not decide, is asked of Git.
 */
export function repositoryIdentityLocation(workspace: string): RepositoryIdentityLocation {
  const requested = resolve(workspace);
  const inputs = placementInputs(requested);
  if (inputs === undefined) {
    heldTopLevels.delete(requested);
    return locatedByGit(requested).location;
  }
  const held = heldTopLevels.get(requested);
  if (held !== undefined && stillAnswers(held, inputs)) return { ...held.location };
  const fromFiles = linkedLocation(inputs);
  const located = fromFiles === undefined ? locatedByGit(requested) : { location: fromFiles, byGit: false };
  holdTopLevel(requested, inputs.stamp, located);
  return { ...located.location };
}

function locatedByGit(requested: string): { location: RepositoryIdentityLocation; byGit: boolean } {
  const result = locateInGit(requested);
  const location = result === undefined || result.status !== 0 ? nonGitLocation(requested) : gitLocation(requested, result.stdout);
  return { location, byGit: true };
}

function linkedLocation(inputs: PlacementInputs): RepositoryIdentityLocation | undefined {
  const placed = linkedWorktreeFromFiles(inputs, common => gitPlacedCommons.has(common));
  if (placed === undefined) return undefined;
  return { git: true, workspaceRoot: placed.top, identityPath: join(placed.commonDirectory, IDENTITY_FILE), main: false };
}

/**
 * Keeps `located` for `folder` when it names the folder as its own top level: what makes that so is read at the folder
 * and the Git folders it names, all in its stamp. Any other answer depends on the folders above too, and is not kept.
 */
function holdTopLevel(folder: string, stamp: string, located: { location: RepositoryIdentityLocation; byGit: boolean }): void {
  heldTopLevels.delete(folder);
  const { location } = located;
  try {
    if (!location.git || physicalPath(location.workspaceRoot) !== physicalPath(folder)) return;
    const common = located.byGit ? storedPath(dirname(location.identityPath)) : undefined;
    heldTopLevels.set(folder, { stamp, location, ownersRead: ownersGeneration });
    if (common !== undefined) gitPlacedCommons.add(common);
  } catch {
    // A folder whose real path cannot be read now is asked of Git again next time.
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

function gitLocation(requested: string, stdout: string): RepositoryIdentityLocation {
  const [top, commonDirectory, gitDirectory] = stdout.trim().split("\n").map(line => resolve(line.trim()));
  if (top === undefined || commonDirectory === undefined || gitDirectory === undefined) {
    throw new Error(`could not resolve Git common directory for ${requested}`);
  }
  return { git: true, workspaceRoot: top, identityPath: join(commonDirectory, IDENTITY_FILE), main: commonDirectory === gitDirectory };
}

/**
 * Each common directory's Git registry, as `git worktree list` last gave it, and what the registry then held. Every
 * checkout of a repository reads the same registry, so a start that lists it from each session's checkout lists it once.
 */
const worktreeRegistries = new Map<string, WorktreeRegistry & { readonly contents: string }>();

/** The common directory each recorded checkout reads, physical, as Git first gave it. */
const registryFolders = new Map<string, string>();

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
  if (commonDirectory === undefined) return undefined;
  // Read before Git runs: a worktree added meanwhile changes it, and Git is asked again next time.
  const contents = registryContents(commonDirectory);
  const known = worktreeRegistries.get(commonDirectory);
  if (known !== undefined && known.contents === contents) return known;
  const registry = listWorktrees(checkout, commonDirectory, contents);
  // A checkout where Git cannot answer keeps nothing: what is kept is bounded by what answers.
  if (registry === undefined) {
    worktreeRegistries.delete(commonDirectory);
    registryFolders.delete(checkout);
  } else {
    worktreeRegistries.set(commonDirectory, registry);
  }
  return registry;
}

/** The physical common directory `checkout` reads, as Git first gave it; undefined when the folder is gone or not in Git. */
function registryCommonDirectory(checkout: string): string | undefined {
  if (!existsSync(checkout)) {
    registryFolders.delete(checkout);
    return undefined;
  }
  const known = registryFolders.get(checkout);
  if (known !== undefined) return known;
  const location = repositoryIdentityLocation(checkout);
  if (!location.git) return undefined;
  const commonDirectory = physicalPath(dirname(location.identityPath));
  registryFolders.set(checkout, commonDirectory);
  return commonDirectory;
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
 * The clone id of a checkout in Git: the identity file's, else a legacy `rerail.repository-id` config value when it is
 * well formed, else a new one. Git's config is read only while no identity file is there, since the file wins once it
 * is: so a repository's worktrees, which share one, start no Git process to be recorded.
 */
function gitCloneId(location: RepositoryIdentityLocation): string {
  const existing = readIdentityFile(location.identityPath);
  if (existing !== undefined) return existing;
  const legacyId = gitText(location.workspaceRoot, ["config", "--local", "--get", "rerail.repository-id"]) ?? "";
  return readOrCreateIdentity(location.identityPath, CLONE_ID.test(legacyId) ? legacyId : `git:${randomUUID()}`);
}

/**
 * `workspace`'s clone id, created on first use and stored atomically in Git's common metadata directory. Linked
 * worktrees share it; clones and forks receive a new one; a legacy `rerail.repository-id` Git config value is adopted
 * when it is a well-formed id. Folders outside Git use the same file contract under `.rerail`, which must not be a
 * symlink.
 */
export function resolveCloneId(workspace: string): { cloneId: string; location: RepositoryIdentityLocation } {
  const location = repositoryIdentityLocation(workspace);
  if (location.git) return { cloneId: gitCloneId(location), location };
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
