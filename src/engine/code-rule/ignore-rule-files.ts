import { createHash } from "node:crypto";
import { readFile, stat } from "node:fs/promises";
import { basename, isAbsolute, relative, resolve, sep } from "node:path";
import { gitExcludeFiles } from "../git.js";

/** Past this size an exclude file is compared by its stamp, never read whole: it is no source, so no source limit applies. */
const MAX_DIGEST_BYTES = 1_000_000;

/**
 * A change to Git's ignore rules in a session: `tool` when a finished tool edit of the session named the ignore file,
 * `observed` when the exclude file's contents were found changed after a command or at a turn end. Every session that
 * looks finds an observed change, whoever made it: nothing tells which wrote it.
 */
export interface IgnoreRulesChange {
  path: string;
  via: "tool" | "observed";
}

/**
 * Git's ignore rules as a session can change them: each `.gitignore` a tool edit names, and the exclude files no diff
 * shows (`info/exclude`, `core.excludesFile`) of every checkout whose files the session's checks judge, whose contents
 * it compares at each look, whatever wrote them. Files are named relative to the session folder where they lie within
 * it, else absolute.
 */
export class IgnoreRuleFiles {
  readonly #root: string;
  /** The checkouts whose exclude files are watched, physical. */
  readonly #checkouts = new Set<string>();
  /** Each watched exclude file, absolute, with what it held at the last look (`null`: absent). */
  readonly #digests = new Map<string, string | null>();
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
   * What changed since the last look, now taken as seen: each ignore file among `paths` (a finished tool edit's, as the
   * tool names them: relative to the session folder, or absolute), then each other exclude file whose contents changed.
   * The exclude files of `checkouts` not watched before are watched from now on, as they are now.
   */
  async changes(paths: readonly string[], checkouts: readonly string[]): Promise<IgnoreRulesChange[]> {
    const look = this.#looking.then(async () => {
      const named = this.#named(paths);
      const observed = (await this.#look()).filter(path => !named.includes(path));
      await this.#watch(checkouts);
      return [...named.map(path => ({ path, via: "tool" as const })), ...observed.map(path => ({ path, via: "observed" as const }))];
    });
    this.#looking = look.catch(() => undefined);
    return await look;
  }

  /** The ignore files among `paths`, shown: any `.gitignore`, and the watched exclude files. */
  #named(paths: readonly string[]): string[] {
    const named = paths.map(path => (isAbsolute(path) ? path : resolve(this.#root, path)))
      .filter(absolute => basename(absolute) === ".gitignore" || this.#digests.has(absolute));
    return [...new Set(named.map(absolute => shownIgnoreFile(this.#root, absolute)))];
  }

  /** The watched exclude files whose contents changed since the last look, shown; each now holds its new digest. */
  async #look(): Promise<string[]> {
    const changed: string[] = [];
    for (const [file, before] of this.#digests) {
      const after = await digestOf(file);
      if (after === before) continue;
      this.#digests.set(file, after);
      changed.push(shownIgnoreFile(this.#root, file));
    }
    return changed;
  }

  /** Watches the exclude files of each of `checkouts` not watched yet, from what they hold now. */
  async #watch(checkouts: readonly string[]): Promise<void> {
    for (const checkout of checkouts) {
      if (this.#checkouts.has(checkout)) continue;
      this.#checkouts.add(checkout);
      for (const file of await gitExcludeFiles(checkout)) if (!this.#digests.has(file)) this.#digests.set(file, await digestOf(file));
    }
  }
}

/** An ignore file as a session sees it: relative to its folder `root` where it lies within it, else absolute. */
export function shownIgnoreFile(root: string, absolute: string): string {
  const path = relative(root, absolute);
  return path === "" || path === ".." || path.startsWith(`..${sep}`) || isAbsolute(path) ? absolute : path.split(sep).join("/");
}

/** What `file` holds: its digest, its stamp past `MAX_DIGEST_BYTES`, or null when it is absent or unreadable. */
async function digestOf(file: string): Promise<string | null> {
  try {
    const info = await stat(file);
    if (!info.isFile()) return null;
    if (info.size > MAX_DIGEST_BYTES) return `stamp:${info.size}:${info.mtimeMs}:${info.ino}`;
    return createHash("sha256").update(await readFile(file)).digest("hex");
  } catch {
    return null;
  }
}
