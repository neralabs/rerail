/**
 * What the walk of a repository's other worktrees looks at in each watched worktree on every walk, to tell a change no
 * change notice tells of (`WorktreeWalks`): the worktree folder's identity (a folder put in its place), the unmerged
 * entries of each checkout nested in it that its last read passed (their index moved), the text of the files that tell
 * Git what to ignore beyond `.gitignore` (the repository's `info/exclude`, the file `core.excludesFile` names, and the
 * configuration files that name it), and on Linux each filesystem mounted at or in it. A file one of these stops
 * ignoring, or a filesystem mounted over a folder, changes what a read finds with no notice in the worktree.
 *
 * Every file is read whole and closed at once (`readFileSync`): Git for Windows renames its lock files over these,
 * which a handle held across turns of the event loop makes fail. Git names the ignore files once per worktree, and
 * again only when their text changed.
 */
import { lstatSync, readFileSync } from "node:fs";
import { gitConfigFiles, gitExcludeFiles, type GitReads, gitUnmergedReader } from "../git.js";

/** One walk's look at one worktree. `HEAD` and the worktree's own index are read for its findings (`WorktreeFindings`). */
export interface WorktreeLook {
  /** The folder's device and file number; `""` once it is gone. */
  readonly folder: string;
  /** By each checkout nested in the worktree, its unmerged entries (`gitUnmergedReader`: the same map while its index holds). */
  readonly nested: ReadonlyMap<string, ReadonlyMap<string, string> | undefined>;
  /** The ignore files' text, each after its path. */
  readonly ignores: string;
  /** The mount table's lines for filesystems mounted at or in the worktree (Linux); `""` elsewhere. */
  readonly mounts: string;
}

/** Whether anything `before` saw of a worktree moved by `now`. */
export function lookMoved(before: WorktreeLook, now: WorktreeLook): boolean {
  if (before.folder !== now.folder || before.ignores !== now.ignores || before.mounts !== now.mounts) return true;
  if (before.nested.size !== now.nested.size) return true;
  return [...now.nested].some(([checkout, entries]) => before.nested.get(checkout) !== entries);
}

/** `/proc/self/mountinfo`, or undefined where there is none. */
function procMountInfo(): string | undefined {
  try {
    return readFileSync("/proc/self/mountinfo", "utf8");
  } catch {
    return undefined;
  }
}

/** A mount point as the mount table writes it, with a space, tab, newline or backslash escaped as `\` and three octal digits. */
function unescapedMountPoint(written: string): string {
  return written.replace(/\\([0-7]{3})/gu, (_escape, octal: string) => String.fromCharCode(Number.parseInt(octal, 8)));
}

/** Each mount table line by the mount point it names (its fifth field). */
function mountLines(table: string | undefined): ReadonlyArray<readonly [string, string]> {
  if (table === undefined) return [];
  return table.split("\n").filter(line => line !== "").map(line => [unescapedMountPoint(line.split(" ")[4] ?? ""), line] as const);
}

/** The text of `path`, or `""` when it cannot be read: a missing file is one Git reads as empty. */
function fileText(path: string): string {
  try {
    return readFileSync(path, "utf8");
  } catch {
    return "";
  }
}

/** The device and file number of the folder at `root`; `""` when it is gone. */
function folderIdentity(root: string): string {
  try {
    const info = lstatSync(root, { bigint: true });
    return `${info.dev}:${info.ino}`;
  } catch {
    return "";
  }
}

/** What a walk reads once for every worktree it looks at: each ignore file's text and the mount table. */
interface WalkFiles {
  readonly texts: Map<string, string>;
  readonly mounts: ReadonlyArray<readonly [string, string]>;
}

/**
 * The looks of one repository's walks (`WorktreeWalks`): the ignore files Git names for each worktree and each
 * checkout's index reader, kept while a session watches the worktree.
 */
export class WorktreeLooks {
  readonly #readUnmerged = gitUnmergedReader();
  readonly #mountInfo: () => string | undefined;
  /** By worktree, the ignore files Git named for it, and their text then. */
  readonly #ignoreFiles = new Map<string, { readonly paths: readonly string[]; readonly text: string }>();

  /** `mountInfo`: the mount table, `/proc/self/mountinfo` by default on Linux; none elsewhere. */
  constructor(options: { mountInfo?: () => string | undefined } = {}) {
    this.#mountInfo = options.mountInfo ?? (process.platform === "linux" ? procMountInfo : () => undefined);
  }

  /** What one walk reads once for all its looks. */
  begin(): WalkFiles {
    return { texts: new Map(), mounts: mountLines(this.#mountInfo()) };
  }

  /** The look at `root` (physical) now, with the nested `checkouts` its last read passed. */
  async look(git: GitReads, files: WalkFiles, root: string, checkouts: Iterable<string>): Promise<WorktreeLook> {
    const nested = new Map<string, ReadonlyMap<string, string> | undefined>();
    for (const checkout of checkouts) {
      if (checkout !== root) nested.set(checkout, await git.run(signal => this.#readUnmerged(checkout, { signal })));
    }
    return { folder: folderIdentity(root), nested, ignores: await this.#ignores(git, files, root), mounts: this.#mountsIn(files, root) };
  }

  /** Forgets each worktree not in `roots`. */
  retain(roots: ReadonlySet<string>): void {
    for (const root of this.#ignoreFiles.keys()) if (!roots.has(root)) this.#ignoreFiles.delete(root);
  }

  /**
   * The ignore files' text. Git names them the first time, and again when their text changed since: a configuration
   * file's change may name another excludes file, which the next look reads.
   */
  async #ignores(git: GitReads, files: WalkFiles, root: string): Promise<string> {
    const named = this.#ignoreFiles.get(root);
    if (named !== undefined) {
      const text = textOf(files, named.paths);
      if (text === named.text) return text;
    }
    const paths = [
      ...await git.run(signal => gitExcludeFiles(root, { signal })),
      ...await git.run(signal => gitConfigFiles(root, { signal })),
    ];
    const text = textOf(files, paths);
    this.#ignoreFiles.set(root, { paths, text });
    return text;
  }

  #mountsIn(files: WalkFiles, root: string): string {
    const inside = `${root}/`;
    return files.mounts.filter(([point]) => point === root || point.startsWith(inside)).map(([, line]) => line).join("\n");
  }
}

/** The text of each of `paths`, after its path, read once per walk. */
function textOf(files: WalkFiles, paths: readonly string[]): string {
  return paths.map(path => {
    let text = files.texts.get(path);
    if (text === undefined) {
      text = fileText(path);
      files.texts.set(path, text);
    }
    return `${path}\0${text}`;
  }).join("\0");
}
