/**
 * The folders Git's search for a repository reads as it walks up from the folder it starts in, bounded as Git's
 * discovery bounds it (`git help git`, `setup.c`): no folder at or above the deepest `GIT_CEILING_DIRECTORIES` entry
 * above the start, and, while `GIT_DISCOVERY_ACROSS_FILESYSTEM` is unset, no folder on another device than the start.
 * Reading only these keeps every read out of folders Git never reads, such as an automounter's root, which looks up
 * each name asked in it. Where Git's rule cannot be matched exactly, more folders are read, never fewer.
 */
import { lstatSync } from "node:fs";
import { delimiter, dirname, isAbsolute } from "node:path";
import { realOf } from "./git-placement.js";

/** `start`, a real path, and each folder above it that Git's search reads, from `start` up. */
export function foldersGitSearches(start: string): string[] {
  const ceilings = gitCeilings().filter(ceiling => below(spelled(start), ceiling));
  const searched = foldersUp(start).filter(folder => ceilings.every(ceiling => below(spelled(folder), ceiling)));
  // Git for Windows gives every file the device 0, so its search never stops at a change of volume. A value of the
  // variable Git reads as false stops Git there too; any value walks on here, which reads more than Git, never less.
  return process.platform === "win32" || process.env.GIT_DISCOVERY_ACROSS_FILESYSTEM !== undefined ? searched : onStartDevice(searched);
}

/** `folder` and every folder above it, up to its root. */
function foldersUp(folder: string): string[] {
  const parent = dirname(folder);
  return parent === folder ? [folder] : [folder, ...foldersUp(parent)];
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

/** The folders of `folders`, from the first up, until one is on another device than the first, as Git stops there. */
function onStartDevice(folders: readonly string[]): string[] {
  const [start, ...above] = folders;
  const device = start === undefined ? undefined : deviceOf(start);
  const end = above.findIndex(folder => deviceOf(folder) !== device);
  return end === -1 ? [...folders] : folders.slice(0, end + 1);
}

/** The device `folder` is on, as an lstat gives it; undefined when it cannot be read. */
function deviceOf(folder: string): number | undefined {
  try {
    return lstatSync(folder).dev;
  } catch {
    return undefined;
  }
}
