import { readFile } from "node:fs/promises";
import { basename, isAbsolute, relative, resolve, sep } from "node:path";
import { gitExcludeFiles } from "../git.js";

/**
 * A change to Git's ignore rules in a session that makes them ignore more: `ignoresMore` holds each pattern the file
 * gained and each negation (`!pattern`) it lost. `tool` when a finished tool edit of the session named the file;
 * `observed` when a look after a command or at a turn end found it changed, whoever changed it.
 */
export interface IgnoreRulesChange {
  path: string;
  via: "tool" | "observed";
  ignoresMore: string[];
}

/** What a look compares: the ignore files a finished tool edit named, and the `.gitignore` files a walk found changed. */
export interface IgnoreRuleCandidates {
  /** As a tool names them: relative to the session folder, or absolute; files that are no ignore files are left out. */
  named: readonly string[];
  /** `.gitignore` files relative to the session folder. */
  observed: readonly string[];
}

/**
 * Git's ignore rules as a session can change them: every `.gitignore` a tool edit names or a walk finds, and the
 * exclude files no diff shows (`info/exclude`, `core.excludesFile`) of every checkout whose files the session's checks
 * judge. Each is compared by its patterns, so a comment, a blank line or a duplicate line changes nothing. Files are
 * named relative to the session folder where they lie within it, else absolute.
 */
export class IgnoreRuleFiles {
  readonly #root: string;
  /** The checkouts whose exclude files are watched, physical. */
  readonly #checkouts = new Set<string>();
  /** The watched exclude files, absolute: compared at every look. */
  readonly #excludeFiles = new Set<string>();
  /** Each ignore file known, absolute, with its patterns when last read (none for a file absent or unreadable). */
  readonly #patterns = new Map<string, ReadonlySet<string>>();
  /** The look in progress, or the last one: looks run one at a time, so two never report one change twice. */
  #looking: Promise<unknown> = Promise.resolve();

  private constructor(root: string) {
    this.#root = root;
  }

  /** The ignore rule files of the session folder `root` (physical) and of `checkouts`, as they are now. */
  static async open(root: string, checkouts: readonly string[]): Promise<IgnoreRuleFiles> {
    const files = new IgnoreRuleFiles(root);
    await files.#watch([root, ...checkouts]);
    return files;
  }

  /**
   * Takes what the ignore files among `paths` (as `IgnoreRuleCandidates.named`) hold now as their baseline: before a
   * tool edit writes them, or as a walk first finds them.
   */
  async remember(paths: readonly string[]): Promise<void> {
    await this.#serially(async () => {
      for (const file of this.#ignoreFiles(paths)) this.#patterns.set(file, await patternsOf(file));
    });
  }

  /**
   * What made Git's rules ignore more since the last look, now taken as seen: each ignore file `candidates` names, then
   * each watched exclude file. The exclude files of `checkouts` not watched before are watched from now on, as they are.
   */
  async changes(candidates: IgnoreRuleCandidates, checkouts: readonly string[]): Promise<IgnoreRulesChange[]> {
    return await this.#serially(async () => {
      const named = new Set(this.#ignoreFiles(candidates.named));
      const files = new Set([...named, ...this.#ignoreFiles(candidates.observed), ...this.#excludeFiles]);
      const changes: IgnoreRulesChange[] = [];
      for (const file of files) {
        const ignoresMore = await this.#ignoresMore(file);
        if (ignoresMore.length > 0) changes.push({ path: shownIgnoreFile(this.#root, file), via: named.has(file) ? "tool" : "observed", ignoresMore });
      }
      await this.#watch(checkouts);
      return changes;
    });
  }

  /** Runs `work` after every look and baseline before it, so none sees another half done. */
  async #serially<T>(work: () => Promise<T>): Promise<T> {
    const run = this.#looking.then(work);
    this.#looking = run.catch(() => undefined);
    return await run;
  }

  /** The patterns by which `file` ignores more than at the last look, which it now holds as its baseline. */
  async #ignoresMore(file: string): Promise<string[]> {
    const before = this.#patterns.get(file) ?? NO_PATTERNS;
    const after = await patternsOf(file);
    this.#patterns.set(file, after);
    const gained = [...after].filter(pattern => !pattern.startsWith("!") && !before.has(pattern));
    const lostNegations = [...before].filter(pattern => pattern.startsWith("!") && !after.has(pattern));
    return [...gained, ...lostNegations];
  }

  /** The ignore files among `paths`, absolute: any `.gitignore`, and the watched exclude files. */
  #ignoreFiles(paths: readonly string[]): string[] {
    return paths.map(path => (isAbsolute(path) ? path : resolve(this.#root, path)))
      .filter(absolute => basename(absolute) === ".gitignore" || this.#excludeFiles.has(absolute));
  }

  /** Watches the exclude files of each of `checkouts` not watched yet, from what they hold now. */
  async #watch(checkouts: readonly string[]): Promise<void> {
    for (const checkout of checkouts) {
      if (this.#checkouts.has(checkout)) continue;
      this.#checkouts.add(checkout);
      for (const file of await gitExcludeFiles(checkout)) await this.#watchExcludeFile(file);
    }
  }

  /** Watches the exclude file `file` (absolute) from what it holds now, unless another checkout already shares it. */
  async #watchExcludeFile(file: string): Promise<void> {
    if (this.#excludeFiles.has(file)) return;
    this.#excludeFiles.add(file);
    this.#patterns.set(file, await patternsOf(file));
  }
}

const NO_PATTERNS: ReadonlySet<string> = new Set();

/** An ignore file as a session sees it: relative to its folder `root` where it lies within it, else absolute. */
export function shownIgnoreFile(root: string, absolute: string): string {
  const path = relative(root, absolute);
  return path === "" || path === ".." || path.startsWith(`..${sep}`) || isAbsolute(path) ? absolute : path.split(sep).join("/");
}

/**
 * The patterns of the ignore file `file`, as Git reads them: each line without its unescaped trailing spaces, but for
 * blank lines and comments; none when it is absent or unreadable.
 */
async function patternsOf(file: string): Promise<ReadonlySet<string>> {
  let text: string;
  try {
    text = await readFile(file, "utf8");
  } catch {
    return NO_PATTERNS;
  }
  const patterns = text.split(/\r?\n/u).map(line => line.replace(/(?<!\\) +$/u, ""));
  return new Set(patterns.filter(pattern => pattern !== "" && !pattern.startsWith("#")));
}
