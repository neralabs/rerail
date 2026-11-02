import { chmodSync, lstatSync, mkdirSync, readdirSync } from "node:fs";
import { join } from "node:path";

/**
 * Takes group and other access away from a folder (to `0700`) or regular file (to `0600`) Rerail keeps private, when
 * any is set and the path belongs to this user. `mkdir` and `open` apply a mode only when they create, so a file an
 * older release wrote, a copy, or a loose umask stays readable by others until this runs. The path is judged with
 * `lstat`, so a symlink is left alone, and changed by path, never through a descriptor of its own: closing any
 * descriptor of a database file drops every POSIX lock this process holds on it, SQLite's included. A path that is
 * missing, a symlink, another user's, or neither a folder nor a regular file is left alone. Windows has no such modes.
 */
export function keepPrivate(path: string): void {
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
 * its sidecars.
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
 * (`rerail.sqlite*`), the coding-agent recovery database (`harness.sqlite*`), the daemon's state, capability and log.
 */
const PRIVATE_HOME_FILES: readonly string[] = ["rerail.sqlite", "harness.sqlite", "daemon.json", "access-token", "daemon.log"];

/** Creates the Rerail home (`0700`) when absent, and keeps it and the private files in it private ({@link keepPrivate}). */
export function makeRerailHome(home: string): void {
  mkdirSync(home, { recursive: true, mode: 0o700 });
  keepPrivate(home);
  keepFilesPrivate(home, PRIVATE_HOME_FILES);
}
