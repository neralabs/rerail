import { createHash } from "node:crypto";
import { readFile, stat } from "node:fs/promises";
import { basename, isAbsolute, relative, resolve, sep } from "node:path";
import { gitExcludeFiles } from "../git.js";

/** Past this size an exclude file is compared by its stamp, never read whole. */
const MAX_DIGEST_BYTES = 1_000_000;

/**
 * Git's ignore rules as a session can change them: each `.gitignore` a tool edit names, and the exclude files no diff
 * shows (`info/exclude`, `core.excludesFile`), whose contents it compares at each look, whatever wrote them. Files are
 * named relative to the session folder where they lie within it, else absolute.
 */
export class IgnoreRuleFiles {
  readonly #root: string;
  /** Each exclude file, absolute, with what it held at the last look (`null`: absent). */
  readonly #digests: Map<string, string | null>;
  #looking: Promise<unknown> = Promise.resolve();

  private constructor(root: string, digests: Map<string, string | null>) {
    this.#root = root;
    this.#digests = digests;
  }

  /** The ignore rule files of the Git checkout at `root` (the session folder, physical), as they are now. */
  static async open(root: string): Promise<IgnoreRuleFiles> {
    const files = await gitExcludeFiles(root);
    return new IgnoreRuleFiles(root, new Map(await Promise.all(files.map(async file => [file, await digestOf(file)] as const))));
  }

  /** The ignore rule files among `paths` (as an edit tool names them: relative to the session folder, or absolute). */
  named(paths: readonly string[]): string[] {
    const named = paths.map(path => (isAbsolute(path) ? path : resolve(this.#root, path)))
      .filter(absolute => basename(absolute) === ".gitignore" || this.#digests.has(absolute));
    return [...new Set(named.map(absolute => shownIgnoreFile(this.#root, absolute)))];
  }

  /** The exclude files whose contents changed since the last look, now taken as seen; looks run one at a time. */
  async changed(): Promise<string[]> {
    const look = this.#looking.then(async () => await this.#look());
    this.#looking = look.catch(() => undefined);
    return await look;
  }

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
