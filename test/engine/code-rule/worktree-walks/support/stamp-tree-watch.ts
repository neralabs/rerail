import { closeSync, fstatSync, lstatSync, openSync, readdirSync } from "node:fs";
import { join } from "node:path";
import type { TreeNotice, TreeWatch } from "../../../../../src/engine/code-rule/tree-watch.js";

/** Each entry under a root, by its `/`-path relative to the root, with what a write moves of it. */
function entriesOf(root: string): Map<string, string> {
  const entries = new Map<string, string>();
  const pending = [""];
  for (let folder = pending.pop(); folder !== undefined; folder = pending.pop()) {
    let names: string[];
    try {
      names = readdirSync(folder === "" ? root : join(root, folder));
    } catch {
      continue;
    }
    for (const name of names) {
      // A `.git` entry's own notices never count, nor anything under it.
      if (name === ".git") continue;
      const path = folder === "" ? name : `${folder}/${name}`;
      const info = lstatSync(join(root, path), { bigint: true });
      entries.set(path, `${info.ino}:${info.size}:${info.mtimeNs}:${info.ctimeNs}`);
      if (info.isDirectory()) pending.push(path);
    }
  }
  return entries;
}

/**
 * A `TreeWatch` that sees every write, as the engine's watch is meant to: each `take` compares each watched root's
 * entries with those of the last take, through `node:fs` calls a test's counted `node:fs/promises` never sees. A test
 * makes it miss writes (`silence`), hand notices late (`hold`), lose notices (`lose`), end a watch (`end`), refuse
 * roots (`unwatchable`) or tell of a folder moved away as the system does (`movedAway`), and reads which roots it
 * watches and with which folders. `byFolder` makes it watch as Linux's
 * watch does: only the entries of the folders a root was watched with, a folder made in one as an entry with nothing
 * under it, a folder it takes up from the moment it is given, not before, and a folder deleted or put back as another
 * let go of, with the folders under it, until a `watch` names it again.
 */
export class StampTreeWatch implements TreeWatch {
  readonly #entries = new Map<string, Map<string, string>>();
  /** The folders each watched root was last watched with. */
  readonly folders = new Map<string, readonly string[]>();
  /** How many times each root was asked to be watched, refused or not. */
  readonly asked = new Map<string, number>();
  readonly #byFolder: boolean;
  /** Every root `unwatch` let go of, in order. */
  readonly unwatched: string[] = [];
  readonly #unwatchable = new Set<string>();
  readonly #lost = new Set<string>();
  readonly #ended = new Set<string>();
  readonly #held = new Set<string>();
  readonly #movedAway = new Set<string>();
  /** By root, a handle on each folder a Linux watch follows: the watch stands for that folder, not for its path. */
  readonly #handles = new Map<string, Map<string, number>>();
  closed = false;

  constructor(options: { byFolder?: boolean } = {}) {
    this.#byFolder = options.byFolder === true;
  }

  watch(root: string, folders: readonly string[]): boolean {
    this.asked.set(root, (this.asked.get(root) ?? 0) + 1);
    if (this.#unwatchable.has(root)) return false;
    const before = this.#entries.get(root);
    if (before === undefined) this.#entries.set(root, entriesOf(root));
    else if (this.#byFolder) this.#takeUp(root, before, folders);
    this.folders.set(root, folders);
    if (this.#byFolder) this.#hold(root, folders);
    return true;
  }

  /** Holds a handle on each of `folders` not held yet, and lets go of each held one no longer followed. */
  #hold(root: string, folders: readonly string[]): void {
    const handles = this.#handles.get(root) ?? new Map<string, number>();
    this.#handles.set(root, handles);
    for (const [folder, handle] of handles) if (!folders.includes(folder)) letGoOf(handles, folder, handle);
    for (const folder of folders) if (!handles.has(folder)) handles.set(folder, openSync(join(root, folder), "r"));
  }

  /** Takes in, with no notice, what the folders `folders` adds to the watch of `root` hold now. */
  #takeUp(root: string, before: Map<string, string>, folders: readonly string[]): void {
    const watched = new Set(this.folders.get(root));
    const added = new Set(folders.filter(folder => !watched.has(folder)));
    for (const [path, stamp] of entriesOf(root)) if (added.has(parentOf(path))) before.set(path, stamp);
  }

  unwatch(root: string): void {
    this.#entries.delete(root);
    this.folders.delete(root);
    this.#hold(root, []);
    this.unwatched.push(root);
  }

  take(): TreeNotice[] {
    const notices: TreeNotice[] = [];
    for (const [root, before] of this.#entries) {
      // Seen at a later take, as a notice the system hands over late.
      const notice = this.#held.has(root) ? undefined : this.#takeRoot(root, before);
      if (notice !== undefined) notices.push(notice);
    }
    return notices;
  }

  /** What a take tells of `root`, whose entries were `before` at the last take; undefined when it tells nothing. */
  #takeRoot(root: string, before: ReadonlyMap<string, string>): TreeNotice | undefined {
    const after = entriesOf(root);
    this.#entries.set(root, after);
    const notice = this.#watched(root, this.#named(root, before, after));
    if (notice.ended) this.unwatch(root);
    const gone = this.#byFolder && !notice.ended ? this.#letGo(root) : [];
    const told = gone.length === 0 ? notice : { ...notice, changed: notice.changed.filter(path => !gone.includes(path)), renamed: [...new Set([...notice.renamed, ...gone])] };
    return tells(told) ? told : undefined;
  }

  /** What the system names of `root` from `before` to `after` (`noticeOf`), each folder moved away as itself alone (`movedAway`). */
  #named(root: string, before: ReadonlyMap<string, string>, after: ReadonlyMap<string, string>): TreeNotice {
    const named = noticeOf(root, before, after, this.#lost.delete(root), this.#ended.delete(root));
    return this.#movedAway.delete(root) ? withoutWhatWentWith(named, before) : named;
  }

  /**
   * As Linux's watch lets go of a folder deleted or moved away, with the folders under it, and tells of it as an entry of
   * its parent that came or went: one put back at the same path is another folder, and tells nothing until a `watch`
   * names it again. The folders let go of.
   */
  #letGo(root: string): string[] {
    const handles = this.#handles.get(root) ?? new Map<string, number>();
    const gone = [...handles].filter(([folder, handle]) => folder !== "" && !stillAt(handle, join(root, folder))).map(([folder]) => folder);
    if (gone.length === 0) return gone;
    const under = (folder: string): boolean => gone.some(path => folder === path || folder.startsWith(`${path}/`));
    this.folders.set(root, (this.folders.get(root) ?? []).filter(folder => !under(folder)));
    for (const [folder, handle] of handles) if (under(folder)) letGoOf(handles, folder, handle);
    return gone;
  }

  /** `notice` as the watch of `root` tells it: with `byFolder`, only the entries of the folders it was watched with. */
  #watched(root: string, notice: TreeNotice): TreeNotice {
    if (!this.#byFolder) return notice;
    const folders = new Set(this.folders.get(root));
    const told = (path: string): boolean => folders.has(parentOf(path));
    return { ...notice, changed: notice.changed.filter(told), renamed: notice.renamed.filter(told) };
  }

  close(): void {
    this.closed = true;
    this.#entries.clear();
    this.folders.clear();
    for (const root of this.#handles.keys()) this.#hold(root, []);
  }

  /** Takes in, with no notice, every write made in `root` so far, as a write through a hard link or a memory map. */
  silence(root: string): void {
    if (this.#entries.has(root)) this.#entries.set(root, entriesOf(root));
  }

  /** Makes the next take of `root` say notices were lost, with none named. */
  lose(root: string): void {
    this.silence(root);
    this.#lost.add(root);
  }

  /** Holds back the notices of `root` until `release`: each take until then hands none, as though none had come yet. */
  hold(root: string): void {
    this.#held.add(root);
  }

  release(root: string): void {
    this.#held.delete(root);
  }

  /** Makes the next take of `root` say its watch ended. */
  end(root: string): void {
    this.silence(root);
    this.#ended.add(root);
  }

  /**
   * Makes the next take of `root` tell of each folder that went as that folder alone, not of what it held, as the
   * system tells of a folder moved away: the folder's parent hears that its entry went, and nothing watches its files.
   */
  movedAway(root: string): void {
    this.#movedAway.add(root);
  }

  /** Refuses to watch `root` from now on. */
  unwatchable(root: string): void {
    this.#unwatchable.add(root);
  }
}

function tells(notice: TreeNotice): boolean {
  return notice.lost || notice.ended || notice.changed.length > 0 || notice.renamed.length > 0;
}

/** `notice` without the paths under each folder it says went (`before` held entries under it). */
function withoutWhatWentWith(notice: TreeNotice, before: ReadonlyMap<string, string>): TreeNotice {
  const went = notice.renamed.filter(path => [...before.keys()].some(entry => entry.startsWith(`${path}/`)));
  const kept = (path: string): boolean => !went.some(folder => path.startsWith(`${folder}/`));
  return { ...notice, changed: notice.changed.filter(kept), renamed: notice.renamed.filter(kept) };
}

function parentOf(path: string): string {
  const slash = path.lastIndexOf("/");
  return slash < 0 ? "" : path.slice(0, slash);
}

/**
 * Whether the folder `handle` was opened on is still the one at `path`: a folder deleted has no link left, and one moved
 * away or put back is another inode there, even where the file system gave the new folder the old one's number.
 */
function stillAt(handle: number, path: string): boolean {
  const held = fstatSync(handle);
  const now = lstatSync(path, { throwIfNoEntry: false });
  return held.nlink > 0 && now !== undefined && now.ino === held.ino && now.dev === held.dev;
}

function letGoOf(handles: Map<string, number>, folder: string, handle: number): void {
  closeSync(handle);
  handles.delete(folder);
}

function noticeOf(root: string, before: ReadonlyMap<string, string>, after: ReadonlyMap<string, string>, lost: boolean, ended: boolean): TreeNotice {
  if (lost || ended) return { root, changed: [], renamed: [], lost, ended };
  const renamed = [...after.keys()].filter(path => !before.has(path));
  renamed.push(...[...before.keys()].filter(path => !after.has(path)));
  const changed = [...after].filter(([path, stamp]) => before.has(path) && before.get(path) !== stamp).map(([path]) => path);
  return { root, changed, renamed, lost: false, ended: false };
}
