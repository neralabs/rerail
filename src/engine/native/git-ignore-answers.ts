/**
 * Git's ignore answers learnt through one checkout's shared matcher (`GitIgnoreMatchers`), for every session's walks.
 * A walk asks Git's ignore rules about every folder and file it finds, a file twice (whether Git ignores it, then
 * whether a lane reads it), and every session walks its checkout again at each turn's end: one answer now serves every
 * question of every walk of every session, where each question asked the engine again.
 *
 * An answer holds until a fact ends it, each read at most once per walk (`GitIgnoreLook`), none by a timer:
 * - the checkout's Git index changed (Git's checksum at the index file's end, or the digest of its bytes where Git
 *   writes none): a path the index tracks is never ignored, and the matcher reads its rules again with the index;
 *   every answer of the checkout ends, and answers are kept again. An index that cannot be read keeps none;
 * - a `.gitignore` on the way to the path changed, appeared or went (its text, folder by folder from the checkout's
 *   root): the answers of that folder and the folders below it end, and the matcher is told to read the folders'
 *   files again as questions enter them (`sourcesUnchanged`), as it does once a question leaves a folder; where it says
 *   the rules it read changed (the configuration, the excludes file, the root `.gitignore`, `info/exclude`), which it
 *   reads again only with a new index, no answer is kept until the index changes;
 * - the matcher: a router that asks for the checkout's matcher once those rules changed gets a new one, with answers of
 *   its own, and answers live only as long as their matcher (`GitIgnoreMatchers`);
 * - the path's kind: a folder's answer and a file's at one path are kept apart.
 */
import { createHash } from "node:crypto";
import { closeSync, fstatSync, lstatSync, openSync, readFileSync, readSync } from "node:fs";
import { join, resolve } from "node:path";
import type { NativeGitIgnoreMatcher } from "./native.js";

/** The bytes read at an index file's end for Git's checksum: a SHA-256 one's length, which holds a SHA-1 one. */
const CHECKSUM_BYTES = 32;
/** A SHA-1 checksum's length: where Git writes no checksum (`index.skipHash`), the index ends with at least this many zero bytes. */
const SHA1_BYTES = 20;

/** The answers of one folder's entries, with its `.gitignore` as they were learnt (`rulesOf`). */
export interface FolderAnswers {
  readonly rules: string;
  /** Git's answer for each entry, by its path; a folder's path ends with `/`. */
  readonly answers: Map<string, boolean>;
}

/**
 * What a walk has checked of one checkout's answers (`GitIgnoreLook`): the folders whose `.gitignore` it read, and
 * those it has learnt an answer in since it read them (`#learnt`).
 */
export interface CheckedInWalk {
  readonly folders: Map<string, FolderAnswers | undefined>;
  readonly learning: Set<string>;
}

/**
 * The `.gitignore` at `path` as its rules come from it: its text, "none" where there is no such file, or the failure of
 * a read that fails, so the answers learnt while it fails end once it no longer does. The engine reads a rules file by
 * its bytes too: a rewrite at the same size within one tick of the file clock changes it, and a touch does not.
 */
function rulesOf(path: string): string {
  try {
    return lstatSync(path, { throwIfNoEntry: false }) === undefined ? "none" : `text ${readFileSync(path, "latin1")}`;
  } catch (error) {
    return `failed ${(error as NodeJS.ErrnoException).code ?? "?"}`;
  }
}

/**
 * The Git index file of the checkout at `root`: the `index` of its `.git` folder, or of the folder its `.git` file names
 * (a linked worktree, a submodule); undefined where neither can be read or there is no `.git`, so no answer is kept.
 */
function indexFileOf(root: string): string | undefined {
  const entry = join(root, ".git");
  try {
    const stats = lstatSync(entry, { throwIfNoEntry: false });
    if (stats === undefined) return undefined;
    if (stats.isDirectory()) return join(entry, "index");
    const named = /^gitdir: (.+?)\r?\n?$/u.exec(readFileSync(entry, "utf8"))?.[1];
    return named === undefined ? undefined : join(resolve(root, named), "index");
  } catch {
    return undefined;
  }
}

/**
 * What tells the index file at `path` from every other: Git's checksum at its end, which Git computes over every byte
 * before it, or the SHA-256 of its bytes where Git wrote zero bytes in its place (`index.skipHash`, which
 * `feature.manyFiles` turns on); "none" where there is no file; undefined where it cannot be read. A stamp of the file's
 * identity, size and times is not enough: ext4 gives a freed inode number to the next new file at once, so two writes
 * of one size within one tick of the file clock can leave all four as they were. Both reads go through one open, so
 * they read one file while Git renames a new index over it.
 */
function indexStamp(path: string): string | undefined {
  let descriptor: number;
  try {
    descriptor = openSync(path, "r");
  } catch (error) {
    return (error as NodeJS.ErrnoException).code === "ENOENT" ? "none" : undefined;
  }
  try {
    return openStamp(descriptor);
  } catch {
    return undefined;
  } finally {
    closeSync(descriptor);
  }
}

/** `indexStamp` of the open index file `descriptor`. */
function openStamp(descriptor: number): string {
  const { size } = fstatSync(descriptor);
  const end = Buffer.alloc(Math.min(size, CHECKSUM_BYTES));
  readSync(descriptor, end, 0, end.length, size - end.length);
  if (end.subarray(-SHA1_BYTES).some(byte => byte !== 0)) return `checksum ${size} ${end.toString("hex")}`;
  return `digest ${createHash("sha256").update(readFileSync(descriptor)).digest("hex")}`;
}

/** The folder holding `path` (`/`-separated, relative to the checkout), "" for an entry of the checkout's root. */
function folderOf(path: string): string {
  const slash = path.lastIndexOf("/");
  return slash < 0 ? "" : path.slice(0, slash);
}

/** The answers learnt through one checkout's matcher, by folder, with the facts that end them (this file's head). */
export class GitIgnoreAnswers {
  readonly #root: string;
  readonly #matcher: NativeGitIgnoreMatcher;
  /** The index's stamp the answers were learnt under (`indexStamp`). */
  #index: string | undefined;
  readonly #folders = new Map<string, FolderAnswers>();
  /** False from a change of the rules the matcher reads only with a new index, until the index changes. */
  #keeps = true;

  constructor(root: string, matcher: NativeGitIgnoreMatcher) {
    this.#root = root;
    this.#matcher = matcher;
  }

  /** Whether Git ignores `path` (relative to the checkout, `/`-separated): the answer kept, else the matcher's, kept. */
  ignored(path: string, directory: boolean, checked: CheckedInWalk | undefined): boolean {
    if (checked === undefined) return this.#matcher.ignored(path, directory);
    const folder = this.#folder(folderOf(path), checked);
    if (folder === undefined) return this.#matcher.ignored(path, directory);
    const key = directory ? `${path}/` : path;
    const known = folder.answers.get(key);
    if (known !== undefined) return known;
    const answer = this.#learnt(path, directory, checked);
    folder.answers.set(key, answer);
    return answer;
  }

  /**
   * The matcher's answer for `path`, to keep. The engine reads a folder's `.gitignore` as a question enters the folder
   * and not while questions stay in it, so it could answer by a text it read before the walk read the folder's: a
   * question outside the walks (a tool call's file) can leave it in a folder, before the walk or during it. Before
   * the walk's first answer to keep in each folder, the matcher is asked about `.git`, a folder at the root Git never
   * tracks, which leaves every folder below the root, as the engine leaves them itself (`leave_folders` in
   * `native/src/git_ignore.rs`): the folders on the way are read again after the walk read them.
   */
  #learnt(path: string, directory: boolean, checked: CheckedInWalk): boolean {
    const folder = folderOf(path);
    if (!checked.learning.has(folder)) {
      this.#matcher.ignored(".git", true);
      checked.learning.add(folder);
    }
    return this.#matcher.ignored(path, directory);
  }

  /**
   * What a walk checks first of these answers: the index's stamp, once per walk, before any question the walk asks, so
   * an index Git writes after it reads as a change at the next walk. Undefined where no answer may be kept for this
   * walk: an index that cannot be read, or rules the matcher reads only with a new index that changed.
   */
  check(): CheckedInWalk | undefined {
    const file = indexFileOf(this.#root);
    const index = file === undefined ? undefined : indexStamp(file);
    if (index !== this.#index) {
      this.#folders.clear();
      this.#index = index;
      this.#keeps = true;
    }
    return index === undefined || !this.#keeps ? undefined : { folders: new Map(), learning: new Set() };
  }

  /**
   * The answers of `folder`, checked once per walk: the `.gitignore` of each folder above it first, then its own.
   * Undefined once a change there leaves no answer to keep (`#rulesChanged`).
   */
  #folder(folder: string, checked: CheckedInWalk): FolderAnswers | undefined {
    if (checked.folders.has(folder)) return checked.folders.get(folder);
    const keptAbove = folder === "" || this.#folder(folderOf(folder), checked) !== undefined;
    const answers = keptAbove ? this.#stamped(folder) : undefined;
    checked.folders.set(folder, answers);
    return answers;
  }

  /** The answers of `folder` learnt while its `.gitignore` was as it is now: those kept, else none yet. */
  #stamped(folder: string): FolderAnswers | undefined {
    const rules = rulesOf(join(this.#root, folder, ".gitignore"));
    const known = this.#folders.get(folder);
    if (known?.rules === rules) return known;
    if (known !== undefined) this.#rulesChanged(folder);
    if (!this.#keeps) return undefined;
    const answers: FolderAnswers = { rules, answers: new Map() };
    this.#folders.set(folder, answers);
    return answers;
  }

  /**
   * Ends the answers of `folder` and of every folder below it, whose `.gitignore` changed, and has the matcher read
   * each folder's file again as the next question enters it. A matcher that says the rules it reads only with a new
   * index changed, or cannot say, keeps answering as it does, so none of its answers is kept until the index changes.
   */
  #rulesChanged(folder: string): void {
    this.#forgetFrom(folder);
    if (this.#matcherSourcesUnchanged()) return;
    this.#keeps = false;
    this.#folders.clear();
  }

  /** Ends the answers of `folder` and of every folder below it. */
  #forgetFrom(folder: string): void {
    for (const known of [...this.#folders.keys()]) {
      if (folder === "" || known === folder || known.startsWith(`${folder}/`)) this.#folders.delete(known);
    }
  }

  /** Whether the matcher says the rules it reads only with a new index are as it read them (`sourcesUnchanged`). */
  #matcherSourcesUnchanged(): boolean {
    try {
      return this.#matcher.sourcesUnchanged();
    } catch {
      // A check that fails reads as a change, as `GitIgnoreMatchers` reads it.
      return false;
    }
  }
}

/**
 * One walk's questions of the checkouts' answers (`GitIgnoreAnswers`): each checkout's index and each folder's
 * `.gitignore` are read once for the walk, as it first asks there. A matcher with no answers kept for it (one a
 * router opened for itself) answers each question itself.
 */
export class GitIgnoreLook {
  readonly #answersOf: (matcher: NativeGitIgnoreMatcher) => GitIgnoreAnswers | undefined;
  readonly #checked = new Map<GitIgnoreAnswers, CheckedInWalk | undefined>();

  constructor(answersOf: (matcher: NativeGitIgnoreMatcher) => GitIgnoreAnswers | undefined) {
    this.#answersOf = answersOf;
  }

  /** Whether Git ignores `path` (relative to `matcher`'s checkout), by the answers kept for `matcher`. */
  ignored(matcher: NativeGitIgnoreMatcher, path: string, directory: boolean): boolean {
    const answers = this.#answersOf(matcher);
    if (answers === undefined) return matcher.ignored(path, directory);
    if (!this.#checked.has(answers)) this.#checked.set(answers, answers.check());
    return answers.ignored(path, directory, this.#checked.get(answers));
  }
}
