import { readFile, stat } from "node:fs/promises";
import { basename, isAbsolute, relative, resolve, sep } from "node:path";
import { gitConfigFiles, gitExcludeFiles } from "../git.js";

/**
 * A change to Git's ignore rules in a session that can make them ignore more. `ignoresMore` holds each pattern the file
 * gained and each negation (`!pattern`) it lost; `reordered` is set when the file holds a negation and its patterns
 * changed otherwise (an order, a duplicate, a removal), since Git's last matching pattern wins. `tool` when a finished
 * tool edit of the session named the file; `observed` when a look after a command or at a turn end found it changed,
 * whoever changed it.
 */
export interface IgnoreRulesChange {
  path: string;
  via: "tool" | "observed";
  ignoresMore: string[];
  reordered?: true;
}

/** What a look compares: the ignore files a finished tool edit named, and the `.gitignore` files a walk found changed. */
export interface IgnoreRuleCandidates {
  /** As a tool names them: relative to the session folder, or absolute; files that are no ignore files are left out. */
  named: readonly string[];
  /** `.gitignore` files relative to the session folder. */
  observed: readonly string[];
}

/** The patterns of an ignore file, in order (`patternsOf`). */
type Patterns = readonly string[];

/**
 * Git's ignore rules as a session can change them: every `.gitignore` a tool edit names or a walk finds, and the
 * exclude files no diff shows (`info/exclude`, `core.excludesFile`) of every checkout whose files the session's checks
 * judge. Each is compared by its patterns, so a comment, a blank line or a duplicate line changes nothing. The Git
 * configuration files that name `core.excludesFile` are watched too: once one changes, an excludes file it newly names
 * is compared from empty. Files are named relative to the session folder where they lie within it, else absolute.
 */
export class IgnoreRuleFiles {
  readonly #root: string;
  /** The checkouts whose exclude files are watched, physical. */
  readonly #checkouts = new Set<string>();
  /** The watched exclude files, absolute: compared at every look. */
  readonly #excludeFiles = new Set<string>();
  /** Each watched Git configuration file, absolute, with its stamp at the last look (`null`: absent). */
  readonly #configStamps = new Map<string, string | null>();
  /** Each ignore file known, absolute, with its patterns when last read (none for a file absent or unreadable). */
  readonly #patterns = new Map<string, Patterns>();
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
   * What can have made Git's rules ignore more since the last look, now taken as seen: each ignore file `candidates`
   * names, then each watched exclude file, including one a changed configuration file now names. The exclude files of
   * `checkouts` not watched before are watched from now on, as they are.
   */
  async changes(candidates: IgnoreRuleCandidates, checkouts: readonly string[]): Promise<IgnoreRulesChange[]> {
    return await this.#serially(async () => {
      await this.#followConfiguration();
      const named = new Set(this.#ignoreFiles(candidates.named));
      const files = new Set([...named, ...this.#ignoreFiles(candidates.observed), ...this.#excludeFiles]);
      const changes: IgnoreRulesChange[] = [];
      for (const file of files) {
        const change = await this.#change(file, named.has(file) ? "tool" : "observed");
        if (change !== undefined) changes.push(change);
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

  /** How `file` can ignore more than at the last look, which it now holds as its baseline; undefined when it cannot. */
  async #change(file: string, via: IgnoreRulesChange["via"]): Promise<IgnoreRulesChange | undefined> {
    const before = this.#patterns.get(file) ?? [];
    const after = await patternsOf(file);
    this.#patterns.set(file, after);
    const { ignoresMore, reordered } = widening(before, after);
    if (ignoresMore.length === 0 && !reordered) return undefined;
    return { path: shownIgnoreFile(this.#root, file), via, ignoresMore, ...(reordered ? { reordered: true as const } : {}) };
  }

  /** The ignore files among `paths`, absolute: any `.gitignore`, and the watched exclude files. */
  #ignoreFiles(paths: readonly string[]): string[] {
    return paths.map(path => (isAbsolute(path) ? path : resolve(this.#root, path)))
      .filter(absolute => basename(absolute) === ".gitignore" || this.#excludeFiles.has(absolute));
  }

  /**
   * Once a watched configuration file changed, asks Git again which exclude files each watched checkout reads, and
   * watches one it newly names from empty: its patterns are all new to the session's rules.
   */
  async #followConfiguration(): Promise<void> {
    if (!(await this.#configurationChanged())) return;
    for (const checkout of this.#checkouts) {
      for (const file of await gitExcludeFiles(checkout)) this.#excludeFiles.add(file);
    }
  }

  /** Whether a watched configuration file changed since the last look; each now holds its new stamp. */
  async #configurationChanged(): Promise<boolean> {
    let changed = false;
    for (const [file, before] of this.#configStamps) {
      const after = await stampOf(file);
      if (after === before) continue;
      this.#configStamps.set(file, after);
      changed = true;
    }
    return changed;
  }

  /** Watches the exclude and configuration files of each of `checkouts` not watched yet, from what they hold now. */
  async #watch(checkouts: readonly string[]): Promise<void> {
    for (const checkout of checkouts) {
      if (this.#checkouts.has(checkout)) continue;
      this.#checkouts.add(checkout);
      for (const file of await gitExcludeFiles(checkout)) await this.#watchExcludeFile(file);
      for (const file of await gitConfigFiles(checkout)) if (!this.#configStamps.has(file)) this.#configStamps.set(file, await stampOf(file));
    }
  }

  /** Watches the exclude file `file` (absolute) from what it holds now, unless another checkout already shares it. */
  async #watchExcludeFile(file: string): Promise<void> {
    if (this.#excludeFiles.has(file)) return;
    this.#excludeFiles.add(file);
    this.#patterns.set(file, await patternsOf(file));
  }
}

/**
 * How the patterns `after` of an ignore file can ignore more than `before`: each pattern gained and each negation
 * lost, and, where either holds a negation, whether their order changed, since Git's last matching pattern wins (a
 * pattern added again after a negation re-ignores what it re-included). Without a negation, order and duplicates
 * change nothing.
 */
export function widening(before: Patterns, after: Patterns): { ignoresMore: string[]; reordered: boolean } {
  const had = new Set(before);
  const has = new Set(after);
  const ignoresMore = [
    ...[...has].filter(pattern => !pattern.startsWith("!") && !had.has(pattern)),
    ...[...had].filter(pattern => pattern.startsWith("!") && !has.has(pattern)),
  ];
  const negated = [...before, ...after].some(pattern => pattern.startsWith("!"));
  const reordered = negated && ignoresMore.length === 0 && before.join("\n") !== after.join("\n");
  return { ignoresMore, reordered };
}

/** An ignore file as a session sees it: relative to its folder `root` where it lies within it, else absolute. */
export function shownIgnoreFile(root: string, absolute: string): string {
  const path = relative(root, absolute);
  return path === "" || path === ".." || path.startsWith(`..${sep}`) || isAbsolute(path) ? absolute : path.split(sep).join("/");
}

/**
 * The patterns of the ignore file `file`, in order, as Git reads them: each line without its unescaped trailing spaces,
 * but for blank lines and comments; none when it is absent or unreadable.
 */
async function patternsOf(file: string): Promise<Patterns> {
  let text: string;
  try {
    text = await readFile(file, "utf8");
  } catch {
    return [];
  }
  return text.split(/\r?\n/u).map(line => line.replace(/(?<!\\) +$/u, "")).filter(pattern => pattern !== "" && !pattern.startsWith("#"));
}

/** A configuration file's stamp (size, modification time, inode), or null when it is absent. */
async function stampOf(file: string): Promise<string | null> {
  try {
    const info = await stat(file);
    return `${info.size}:${info.mtimeMs}:${info.ino}`;
  } catch {
    return null;
  }
}
