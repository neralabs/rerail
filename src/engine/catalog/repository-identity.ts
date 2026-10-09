import { randomUUID } from "node:crypto";
import {
  chmodSync,
  closeSync,
  constants as fsConstants,
  fstatSync,
  linkSync,
  lstatSync,
  mkdirSync,
  openSync,
  readSync,
  unlinkSync,
  writeFileSync,
} from "node:fs";
import { basename, dirname, join, resolve } from "node:path";
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
  let result;
  try {
    result = runGit(requested, ["rev-parse", "--show-toplevel", "--path-format=absolute", "--git-common-dir", "--git-dir"]);
  } catch (error) {
    if (error instanceof GitUnavailableError) return nonGitLocation(requested);
    throw error;
  }
  if (result.status !== 0) return nonGitLocation(requested);
  const [top, commonDirectory, gitDirectory] = result.stdout.trim().split("\n").map(line => resolve(line.trim()));
  if (top === undefined || commonDirectory === undefined || gitDirectory === undefined) {
    throw new Error(`could not resolve Git common directory for ${requested}`);
  }
  return { git: true, workspaceRoot: top, identityPath: join(commonDirectory, "rerail-repository-id"), main: commonDirectory === gitDirectory };
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
  const origin = gitText(root, ["ls-remote", "--get-url", "origin"]);
  if (origin !== undefined && origin !== "" && origin !== "origin") return { name: "origin", url: origin };
  const names = (gitText(root, ["remote"]) ?? "").split("\n").map(name => name.trim()).filter(name => name.length > 0);
  if (names.length !== 1) return undefined;
  const [name] = names as [string];
  const url = gitText(root, ["remote", "get-url", "--", name]);
  return url === undefined || url === "" ? undefined : { name, url };
}

const SCHEME = /^[A-Za-z][A-Za-z0-9+.-]*:\/\//;
const SCP_FORM = /^(?:[^@/]+@)?([^:/]+):(.*)$/;

function splitAddress(address: string): { host: string; path: string } | undefined {
  if (SCHEME.test(address)) {
    if (/^file:\/\//i.test(address)) return undefined;
    const rest = address.replace(SCHEME, "");
    const slash = rest.indexOf("/");
    const authority = slash === -1 ? rest : rest.slice(0, slash);
    const host = authority.slice(authority.lastIndexOf("@") + 1).replace(/:\d*$/, "");
    return { host, path: slash === -1 ? "" : rest.slice(slash) };
  }
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
