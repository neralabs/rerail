/**
 * The folders Git's search for a repository may read as it walks up from the folder it starts in, bounded as Git's
 * discovery bounds it (`git help git`, `setup.c`): no folder at or above the deepest `GIT_CEILING_DIRECTORIES` entry
 * above the start, and, while `GIT_DISCOVERY_ACROSS_FILESYSTEM` is unset, no folder on another device than the start.
 * They are given one at a time, so a caller stops where Git's search ends, at the first folder that holds a repository,
 * and reads no folder above it: Git never reads one, and an automounter's root, say, looks up each name asked in it.
 * Where Git's rule cannot be matched exactly, more folders are given, never fewer.
 */
import { lstatSync } from "node:fs";
import { delimiter, dirname, isAbsolute } from "node:path";
import { realOf } from "./git-placement.js";

/** `start`, a real path, then each folder above it that Git's search may read, from `start` up, read as they are taken. */
export function* foldersGitSearches(start: string): Generator<string, void, undefined> {
  const ceilings = gitCeilings().filter(ceiling => below(spelled(start), ceiling));
  const onDevice = onStartDevice(start);
  for (let folder = start; ceilings.every(ceiling => below(spelled(folder), ceiling)) && onDevice(folder); folder = dirname(folder)) {
    yield folder;
    if (dirname(folder) === folder) return;
  }
}

/**
 * The `GIT_CEILING_DIRECTORIES` entries as Git compares them with the folder it starts in: an absolute entry through
 * its real path (none when it has none), or as written once an empty entry came before it; a relative entry is dropped.
 */
function gitCeilings(): string[] {
  const entries = (process.env.GIT_CEILING_DIRECTORIES ?? "").split(delimiter);
  const empty = entries.includes("") ? entries.indexOf("") : entries.length;
  return entries.flatMap((entry, index) => ceilingOf(entry, index > empty));
}

function ceilingOf(entry: string, asWritten: boolean): string[] {
  if (!isAbsolute(entry)) return [];
  if (asWritten) return [entry];
  const real = realOf(entry);
  return real === undefined ? [] : [spelled(real)];
}

/** `path` as Git spells the folder it compares with a ceiling: with `/` between names on Windows too. */
function spelled(path: string): string {
  return process.platform === "win32" ? path.replaceAll("\\", "/") : path;
}

/** Whether `folder` is below `ceiling` as Git matches them: `ceiling` less a trailing `/`, then `/` and a name. */
function below(folder: string, ceiling: string): boolean {
  const base = ceiling.endsWith("/") ? ceiling.slice(0, -1) : ceiling;
  return folder.length > base.length + 1 && folder.startsWith(`${base}/`);
}

/**
 * Whether a folder is on the device `start` is on, as Git's search stops at the first that is not. Git for Windows gives
 * every file the device 0, so its search never stops at a change of volume; a value of the variable Git reads as false
 * stops Git there too, and any value walks on here, which reads more than Git, never less.
 */
function onStartDevice(start: string): (folder: string) => boolean {
  if (process.platform === "win32" || process.env.GIT_DISCOVERY_ACROSS_FILESYSTEM !== undefined) return () => true;
  const device = deviceOf(start);
  return folder => deviceOf(folder) === device;
}

/** The device `folder` is on, as an lstat gives it; undefined when it cannot be read. */
function deviceOf(folder: string): number | undefined {
  try {
    return lstatSync(folder).dev;
  } catch {
    return undefined;
  }
}
