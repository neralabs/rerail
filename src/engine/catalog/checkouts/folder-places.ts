/**
 * Where a checkout's top folder is on disk: the answer `physicalPath`, `storedPath` and `existsSync` give for it, kept
 * while the engine's tree watch shows that nothing moved it, so a repository's worktrees are not looked up on disk at
 * every walk of every session. Each look-up on Windows opens the folder to read its final path; at a few hundred
 * worktrees, listed again at each walk, those look-ups were a quarter of the daemon's busy time.
 *
 * An answer is kept only for a folder the walk of the other worktrees reads, given here by its path as Git lists it,
 * and only when it is exact to keep: on Linux and Windows, where a move of a watched folder or of a folder above it is
 * a notice before the next look-up (`TreeWatch`); for a folder that exists; and when no link is on its way (its
 * physical path is its path, letter case aside on Windows), since a link, or a junction, swapped or retargeted above a
 * folder moves nothing the watch follows; and on Linux when the engine can watch every folder above it for its going,
 * which it can only for a folder it may read. A notice that the folder's watch ended or lost its notices forgets the
 * answer, and the next look-up asks the disk again. Every other path, and every path where no watch can be had, is
 * asked of the disk at each look-up, as before.
 */
import { accessSync, constants, existsSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { physicalPath, storedPath } from "../checkout-control.js";

/** What the places need of the engine's tree watch (`TreeWatch`): watching a folder, and its notices. */
export interface PlaceWatch {
  watch(root: string, folders: readonly string[]): boolean;
  unwatch(root: string): void;
  take(): ReadonlyArray<{ readonly root: string; readonly lost: boolean; readonly ended: boolean }>;
}

export interface FolderPlacesOptions {
  /** A watch of the places' own; without one every answer is the disk's. */
  readonly watch?: PlaceWatch | undefined;
  readonly platform?: NodeJS.Platform;
}

export class FolderPlaces {
  readonly #watch: PlaceWatch | undefined;
  readonly #platform: NodeJS.Platform;
  /** By each path a look-up was given (resolved), the stored path of the watched folder it names. */
  readonly #byPath = new Map<string, string>();
  /** By each watched folder (stored path), the paths kept for it. */
  readonly #paths = new Map<string, Set<string>>();
  /** Whether this stretch of synchronous work took the watch's notices already. */
  #taken = false;

  constructor(options: FolderPlacesOptions = {}) {
    this.#platform = options.platform ?? process.platform;
    this.#watch = this.#platform === "linux" || this.#platform === "win32" ? options.watch : undefined;
  }

  /**
   * `storedPath(path)` for the top folder of a worktree the walk reads, kept as said above. The first look-up of a
   * folder watches it, and the answer is kept once the watch is up and the disk still gives it.
   */
  worktreeRoot(path: string): string {
    const lexical = resolve(path);
    const kept = this.#kept(lexical);
    if (kept !== undefined) return kept;
    const stored = storedPath(lexical);
    if (this.#watch !== undefined) this.#keep(this.#watch, lexical, stored);
    return stored;
  }

  /** `physicalPath(path)`: a kept folder's own, else the disk's. */
  physical(path: string): string {
    const lexical = resolve(path);
    return this.#kept(lexical) ?? physicalPath(lexical);
  }

  /** `existsSync(path)`: true for a kept folder, else the disk's. */
  exists(path: string): boolean {
    return this.#kept(resolve(path)) !== undefined || existsSync(path);
  }

  /** The kept answer for `lexical`, after the notices that came since this stretch of work began. */
  #kept(lexical: string): string | undefined {
    if (this.#watch === undefined) return undefined;
    this.#collect();
    return this.#byPath.get(lexical);
  }

  /**
   * Takes the watch's notices once per stretch of synchronous work, and forgets each folder whose watch ended or lost
   * its notices: one that moved, went or was renamed, it or a folder above it, or whose notices overflowed.
   */
  #collect(): void {
    if (this.#taken) return;
    this.#taken = true;
    queueMicrotask(() => {
      this.#taken = false;
    });
    for (const notice of this.#watch!.take()) if (notice.ended || notice.lost) this.#forget(notice.root);
  }

  /** Keeps `stored` for `lexical` when it is exact to: see the module's comment. */
  #keep(watch: PlaceWatch, lexical: string, stored: string): void {
    if (!this.#linkFree(lexical, stored)) return;
    const known = this.#paths.get(stored);
    if (known !== undefined) {
      known.add(lexical);
      this.#byPath.set(lexical, stored);
      return;
    }
    if (!this.#watched(watch, stored)) return;
    // The watch is up: a move from here on is a notice. One between the look-up and the watch shows on the disk now.
    if (storedPath(lexical) !== stored) {
      watch.unwatch(stored);
      return;
    }
    this.#paths.set(stored, new Set([lexical, stored]));
    this.#byPath.set(lexical, stored);
    this.#byPath.set(stored, stored);
  }

  /**
   * Whether `stored` exists and is watched now, so that a move of it or of a folder above it is a notice by the next
   * look-up. The engine's Linux watch follows each folder above a root for its going, and can follow only one it may
   * read: a root below a folder it may enter but not list is watched without it, and that folder's rename goes
   * unreported.
   */
  #watched(watch: PlaceWatch, stored: string): boolean {
    return existsSync(stored) && this.#aboveFollowed(stored) && watch.watch(stored, [""]);
  }

  /** Whether the engine follows every folder above `stored` up to the file system's root (`#watched`). */
  #aboveFollowed(stored: string): boolean {
    if (this.#platform !== "linux") return true;
    for (let above = dirname(stored); above !== dirname(above); above = dirname(above)) {
      if (!readable(above)) return false;
    }
    return true;
  }

  /** Whether no link lies on the way to `lexical`: it resolves to itself, letter case aside on Windows. */
  #linkFree(lexical: string, stored: string): boolean {
    if (this.#platform !== "win32") return lexical === stored;
    return lexical.length === stored.length && lexical.toUpperCase() === stored.toUpperCase();
  }

  #forget(root: string): void {
    const paths = this.#paths.get(root);
    if (paths === undefined) return;
    this.#paths.delete(root);
    for (const path of paths) this.#byPath.delete(path);
    this.#watch!.unwatch(root);
  }
}

/** Whether this process may list `folder`. */
function readable(folder: string): boolean {
  try {
    accessSync(folder, constants.R_OK);
    return true;
  } catch {
    return false;
  }
}
