/**
 * What Rerail keeps its owner's alone, on any system: a private folder ({@link keepFolderPrivate}) and the check that
 * a secret it reads is private ({@link openToOthers}). Elsewhere than Windows that is mode bits. Windows has none, and
 * ignores `mkdir`'s and `open`'s modes: there it is the folder's access control list (`windows-acl.ts`). Callers make
 * one call for either system and never branch on the platform.
 */
import { chmodSync, lstatSync, mkdirSync, readdirSync } from "node:fs";
import { stat } from "node:fs/promises";
import { homedir } from "node:os";
import { join } from "node:path";
import { MACHINE_ID_FILE, REMOTE_CONFIG_FILE, REMOTE_OUTBOX_FILE, REMOTE_TOKENS_FOLDER } from "./rerail-home.js";
import { insideProfile, WINDOWS_ACL, type WindowsAcl } from "./windows-acl.js";

/** The system a path is kept private on: its platform, Windows' access control and the profile folder; tests pass their own. */
export interface PrivateMachine {
  readonly platform?: NodeJS.Platform;
  readonly acl?: WindowsAcl;
  readonly profile?: string;
}

/**
 * When a call gives a folder its Windows ACL, which starts `icacls`: `made`, the caller just made the folder; `repair`,
 * a rare command (a daemon launch, `rerail remote add`) fixing a folder an earlier release made with its parent's ACL.
 * A call with neither, as one a hook makes on every tool call, starts no program.
 */
export interface KeepFolderPrivate extends PrivateMachine {
  readonly made?: boolean;
  readonly repair?: boolean;
}

/**
 * Keeps `folder` its owner's alone. Elsewhere than Windows, takes group and other access away (`0700`) on every call
 * ({@link keepPrivate}). On Windows, outside the profile folder, gives it an ACL of its own that grants this user,
 * SYSTEM and Administrators alone, which everything in it inherits, files already there included; under the profile
 * the inherited ACL already does that. The ACL goes on only when `made` or `repair` asks. A failure to set it is
 * reported on stderr and never fails the caller: the folder stays as private as its parent left it.
 */
export function keepFolderPrivate(folder: string, options: KeepFolderPrivate = {}): void {
  const { acl = WINDOWS_ACL, platform = process.platform, profile = homedir() } = options;
  if (platform !== "win32") {
    keepPrivate(folder);
    return;
  }
  if ((options.made !== true && options.repair !== true) || insideProfile(folder, profile)) return;
  try {
    acl.restrictToOwner(folder, true);
  } catch (error) {
    process.stderr.write(`rerail: could not keep ${folder} to this user alone: ${error instanceof Error ? error.message : String(error)}\n`);
  }
}

/** What {@link openToOthers} reads a path's access with; tests pass their own. */
export interface AccessReader {
  readonly platform?: NodeJS.Platform;
  readonly acl?: Pick<WindowsAcl, "otherReaders">;
}

/**
 * Who beyond its owner may get at `path`, as the rest of a sentence that starts with the path, or undefined when it is
 * its owner's alone. Elsewhere than Windows, any group or other permission counts, as ssh refuses a private key with
 * one: `is open to other users (mode 0644)`. On Windows, any account besides this user, SYSTEM and Administrators that
 * its ACL lets read it: `can be read by other accounts (BU, AU)`. Throws when it cannot tell (the path is missing, the
 * ACL cannot be read); a caller checking a secret refuses it then too, and never reads it unchecked.
 */
export async function openToOthers(path: string, reader: AccessReader = {}): Promise<string | undefined> {
  const { acl = WINDOWS_ACL, platform = process.platform } = reader;
  if (platform === "win32") {
    const readers = acl.otherReaders(path);
    return readers.length === 0 ? undefined : `can be read by other accounts (${readers.join(", ")})`;
  }
  const mode = (await stat(path)).mode & 0o777;
  return (mode & 0o077) === 0 ? undefined : `is open to other users (mode ${mode.toString(8).padStart(4, "0")})`;
}

/**
 * Takes group and other access away from a folder (to `0700`) or regular file (to `0600`) Rerail keeps private, when
 * any is set and the path belongs to this user. `mkdir` and `open` apply a mode only when they create, so a file an
 * older release wrote, a copy, or a loose umask stays readable by others until this runs. The path is judged with
 * `lstat`, so a symlink is left alone, and changed by path, never through a descriptor of its own: closing any
 * descriptor of a database file drops every POSIX lock this process holds on it, SQLite's included. A path that is
 * missing, a symlink, another user's, or neither a folder nor a regular file is left alone. Windows has no such modes;
 * there the folder's ACL does this ({@link keepFolderPrivate}).
 */
function keepPrivate(path: string): void {
  if (process.platform === "win32") return;
  let stats;
  try {
    stats = lstatSync(path);
  } catch (error) {
    const code = (error as NodeJS.ErrnoException).code;
    if (code === "ENOENT" || code === "EACCES") return;
    throw error;
  }
  if (stats.uid !== process.geteuid!() || (stats.mode & 0o077) === 0) return;
  if (stats.isDirectory()) chmodSync(path, 0o700);
  else if (stats.isFile()) chmodSync(path, 0o600);
}

/**
 * Each file in `folder` whose name starts with one of `prefixes`, kept private ({@link keepPrivate}): a database and
 * its sidecars. On Windows each inherits its folder's ACL, so this does nothing there.
 */
export function keepFilesPrivate(folder: string, prefixes: readonly string[]): void {
  if (process.platform === "win32") return;
  let names: string[];
  try {
    names = readdirSync(folder);
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "ENOENT") return;
    throw error;
  }
  for (const name of names) if (prefixes.some(prefix => name.startsWith(prefix))) keepPrivate(join(folder, name));
}

/**
 * What the Rerail home holds that is the person's alone: the database and its WAL, journal and pre-update copies
 * (`rerail.sqlite*`), the coding-agent recovery database (`harness.sqlite*`), the daemon's state, capability and log, and
 * the remote sink's configuration, tokens, outbox and machine id (`remote.json`, `remote-tokens/`, `remote-outbox.sqlite*`, `machine-id`).
 */
const PRIVATE_HOME_FILES: readonly string[] = ["rerail.sqlite", "harness.sqlite", "daemon.json", "access-token", "daemon.log",
  REMOTE_CONFIG_FILE, REMOTE_TOKENS_FOLDER, REMOTE_OUTBOX_FILE, MACHINE_ID_FILE];

/**
 * Creates the Rerail home (`0700`) when absent, and keeps it and the private files in it private
 * ({@link keepFolderPrivate}, {@link keepFilesPrivate}). On Windows a home it creates gets its ACL, which everything
 * later written in it inherits; the daemon launch repairs one an earlier release made.
 */
export function makeRerailHome(home: string, machine: PrivateMachine = {}): void {
  const created = mkdirSync(home, { recursive: true, mode: 0o700 });
  keepFolderPrivate(home, { ...machine, made: created !== undefined });
  keepFilesPrivate(home, PRIVATE_HOME_FILES);
}
