import { lstatSync, readdirSync } from "node:fs";
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
 * makes it miss writes (`silence`), hand notices late (`hold`), lose notices (`lose`), end a watch (`end`) or refuse
 * roots (`unwatchable`), and reads which roots it watches and with which folders. `byFolder` makes it watch as Linux's
 * watch does: only the entries of the folders a root was watched with, a folder made in one as an entry with nothing
 * under it, and a folder it takes up from the moment it is given, not before.
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
    return true;
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
    const notice = this.#watched(root, noticeOf(root, before, after, this.#lost.delete(root), this.#ended.delete(root)));
    if (notice.ended) this.unwatch(root);
    return tells(notice) ? notice : undefined;
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

  /** Refuses to watch `root` from now on. */
  unwatchable(root: string): void {
    this.#unwatchable.add(root);
  }
}

function tells(notice: TreeNotice): boolean {
  return notice.lost || notice.ended || notice.changed.length > 0 || notice.renamed.length > 0;
}

function parentOf(path: string): string {
  const slash = path.lastIndexOf("/");
  return slash < 0 ? "" : path.slice(0, slash);
}

function noticeOf(root: string, before: ReadonlyMap<string, string>, after: ReadonlyMap<string, string>, lost: boolean, ended: boolean): TreeNotice {
  if (lost || ended) return { root, changed: [], renamed: [], lost, ended };
  const renamed = [...after.keys()].filter(path => !before.has(path));
  renamed.push(...[...before.keys()].filter(path => !after.has(path)));
  const changed = [...after].filter(([path, stamp]) => before.has(path) && before.get(path) !== stamp).map(([path]) => path);
  return { root, changed, renamed, lost: false, ended: false };
}
