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
 * makes it miss a change (`unnoticed`), hand notices late (`hold`), lose notices (`lose`), end a watch (`end`) or refuse
 * roots (`unwatchable`), and reads which roots it watches and with which folders.
 */
export class StampTreeWatch implements TreeWatch {
  readonly #entries = new Map<string, Map<string, string>>();
  /** The folders each watched root was last watched with. */
  readonly folders = new Map<string, readonly string[]>();
  /** Every root `unwatch` let go of, in order. */
  readonly unwatched: string[] = [];
  readonly #unwatchable = new Set<string>();
  readonly #lost = new Set<string>();
  readonly #ended = new Set<string>();
  readonly #held = new Set<string>();
  /** What `until` waits for, each checked again after a watch. */
  readonly #waiting: Array<{ readonly met: () => boolean; readonly resolve: () => void }> = [];
  closed = false;

  watch(root: string, folders: readonly string[]): boolean {
    if (this.#unwatchable.has(root)) return false;
    if (!this.#entries.has(root)) this.#entries.set(root, entriesOf(root));
    this.folders.set(root, folders);
    this.#settle();
    return true;
  }

  /** Resolves once `met` holds, read now and after each watch: a test waits for that fact, not for a time. */
  until(met: () => boolean): Promise<void> {
    if (met()) return Promise.resolve();
    const { promise, resolve } = Promise.withResolvers<void>();
    this.#waiting.push({ met, resolve });
    return promise;
  }

  #settle(): void {
    for (const waiter of this.#waiting.filter(waiter => waiter.met())) {
      this.#waiting.splice(this.#waiting.indexOf(waiter), 1);
      waiter.resolve();
    }
  }

  unwatch(root: string): void {
    this.#entries.delete(root);
    this.folders.delete(root);
    // As the engine does, the notices queued for a root it lets go of go with it.
    this.#lost.delete(root);
    this.#ended.delete(root);
    this.unwatched.push(root);
  }

  take(): TreeNotice[] {
    const notices: TreeNotice[] = [];
    for (const [root, before] of this.#entries) {
      // Seen at a later take, as a notice the system hands over late.
      if (this.#held.has(root)) continue;
      const after = entriesOf(root);
      this.#entries.set(root, after);
      const notice = noticeOf(root, before, after, this.#lost.delete(root), this.#ended.delete(root));
      if (notice.ended) this.unwatch(root);
      if (notice.lost || notice.ended || notice.changed.length > 0 || notice.renamed.length > 0) notices.push(notice);
    }
    return notices;
  }

  close(): void {
    this.closed = true;
    this.#entries.clear();
    this.folders.clear();
  }

  /**
   * Makes `change` in `root` with no notice, as a write through a hard link or a memory map does: a take while it runs
   * hands none for `root`, so the walks' notice pump asks for no walk on its account, and every write it made is then
   * taken in silently. A hold the test already had on `root` stays until its own `release`, and the writes made under
   * that hold before the change are taken in silently with it: their notices never come.
   */
  async unnoticed(root: string, change: () => Promise<unknown>): Promise<void> {
    const heldBefore = this.#held.has(root);
    this.hold(root);
    try {
      await change();
      this.#silence(root);
    } finally {
      if (!heldBefore) this.release(root);
    }
  }

  /** Takes in, with no notice, every write made in `root` so far. */
  #silence(root: string): void {
    if (this.#entries.has(root)) this.#entries.set(root, entriesOf(root));
  }

  /** Makes the next take of `root` say notices were lost, with none named. */
  lose(root: string): void {
    this.#silence(root);
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
    this.#silence(root);
    this.#ended.add(root);
  }

  /** Refuses to watch `root` from now on. */
  unwatchable(root: string): void {
    this.#unwatchable.add(root);
  }
}

function noticeOf(root: string, before: ReadonlyMap<string, string>, after: ReadonlyMap<string, string>, lost: boolean, ended: boolean): TreeNotice {
  if (lost || ended) return { root, changed: [], renamed: [], lost, ended };
  const renamed = [...after.keys()].filter(path => !before.has(path));
  renamed.push(...[...before.keys()].filter(path => !after.has(path)));
  const changed = [...after].filter(([path, stamp]) => before.has(path) && before.get(path) !== stamp).map(([path]) => path);
  return { root, changed, renamed, lost: false, ended: false };
}
