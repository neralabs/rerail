/**
 * Which Git ignore rules hold for each checkout now, for every session of the daemon. A native matcher holds its
 * checkout's Git index, so the sessions' routers share one per checkout: each opening its own in every worktree it
 * audited held one index per worktree per session. A router that asks gets the shared matcher while every file that
 * matcher read is as it was when it read it (`IgnoreSources`), and a fresh one otherwise, which the routers that ask
 * after it share. A router keeps the matcher it was given for its life, as it kept the one it opened itself.
 */
import { readFileSync, statSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileStamp } from "../file-stamp.js";
import { defaultExcludesFile, gitHome } from "../git.js";
import { loadNativeBinding, type NativeGitIgnoreMatcher } from "./native.js";

interface Shared {
  readonly root: string;
  readonly matcher: NativeGitIgnoreMatcher | null;
  readonly sources: IgnoreSources;
  readonly users: Set<object>;
}

export class GitIgnoreMatchers {
  readonly #open: (root: string) => NativeGitIgnoreMatcher | null;
  readonly #env: NodeJS.ProcessEnv;
  /** The matcher of each checkout a router that asks now gets, while its sources are unchanged. */
  readonly #current = new Map<string, Shared>();
  /** Every matcher a router holds: the current ones, and those a change left with the routers that took them. */
  readonly #held = new Set<Shared>();

  constructor(
    open: (root: string) => NativeGitIgnoreMatcher | null = root => loadNativeBinding().openGitIgnoreMatcher(root),
    env: NodeJS.ProcessEnv = process.env,
  ) {
    this.#open = open;
    this.#env = env;
  }

  /**
   * `root`'s matcher for `user`: the shared one while the files it read are unchanged, else one opened now; null outside
   * Git. A failed open throws to each user that asks while none is open, as a router's own open did. A checkout whose
   * Git folders cannot be read here (`IgnoreSources.of`) gets a matcher of its own each time, as before sharing.
   */
  matcher(root: string, user: object): NativeGitIgnoreMatcher | null {
    const current = this.#current.get(root);
    if (current !== undefined && current.sources.unchanged()) {
      current.users.add(user);
      leaveFolders(current.matcher);
      return current.matcher;
    }
    const sources = IgnoreSources.of(root, this.#env);
    // Stamped before the open reads them, so a change made meanwhile reads as one at the next ask.
    const matcher = this.#open(root);
    if (sources === undefined) return matcher;
    const shared: Shared = { root, matcher, sources, users: new Set([user]) };
    this.#current.set(root, shared);
    this.#held.add(shared);
    return matcher;
  }

  /** Ends `user`'s use of every matcher; one no user holds is dropped, and its index with it. */
  release(user: object): void {
    for (const shared of [...this.#held]) {
      shared.users.delete(user);
      if (shared.users.size > 0) continue;
      this.#held.delete(shared);
      if (this.#current.get(shared.root) === shared) this.#current.delete(shared.root);
    }
  }

  /** How many matchers routers hold now. */
  get size(): number {
    return this.#held.size;
  }
}

/**
 * Has `matcher` leave the folders below its root. Its exclude stack reads a folder's `.gitignore` as it enters the
 * folder and keeps it while the paths it is asked stay below, so a router taking the shared matcher could be answered by
 * a nested `.gitignore` as an earlier router's question read it. Asked about `.git`, a path at the root that Git never
 * tracks, the stack leaves every folder: its next path below one reads that folder's `.gitignore` again, as a matcher
 * opened now would.
 */
function leaveFolders(matcher: NativeGitIgnoreMatcher | null): void {
  try {
    matcher?.ignored(".git", true);
  } catch {
    // An ignore file that cannot be read is reported by the question that reads it.
  }
}

/**
 * What a matcher read Git's ignore rules from, besides the `.gitignore` of each folder below the root, which it reads as
 * it enters the folder (`leaveFolders`): the root's `.gitignore`, `info/exclude` in the common Git folder, Git's default
 * excludes file and each file the configuration names, each with its stamp; and the configuration files that can name
 * one (`gitConfigFiles` names the same ones), by what they set of the two keys the matcher reads, `core.excludesFile`
 * and `core.ignoreCase`. Git writes those files for much else (`git push -u` sets a branch's upstream there), which
 * leaves the matcher's rules as they were. Read on each router's ask, without Git: about ten `stat` calls, and the
 * configuration files read again once their stamps change.
 */
class IgnoreSources {
  readonly #files: readonly string[];
  readonly #stamps: string;
  readonly #configs: readonly string[];
  #configStamps: string;
  readonly #settings: string;

  private constructor(root: string, folders: { git: string; common: string }, env: NodeJS.ProcessEnv) {
    this.#configs = configFiles(folders, env);
    this.#configStamps = stampsOf(this.#configs);
    const settings = ignoreSettings(this.#configs);
    this.#settings = JSON.stringify(settings);
    const excludes = [defaultExcludesFile(env), ...namedExcludesFiles(settings, root, env)];
    this.#files = [join(root, ".gitignore"), join(folders.common, "info", "exclude"), ...excludes];
    this.#stamps = stampsOf(this.#files);
  }

  /** `root`'s sources as they are now; undefined where `root` holds no `.git` (a folder below a checkout's root, or none). */
  static of(root: string, env: NodeJS.ProcessEnv): IgnoreSources | undefined {
    const folders = gitFolders(root);
    return folders === undefined ? undefined : new IgnoreSources(root, folders, env);
  }

  /** Whether every source is as it was when stamped, the configuration files in what they set of the two keys. */
  unchanged(): boolean {
    return stampsOf(this.#files) === this.#stamps && this.#settingsUnchanged();
  }

  /** Whether the configuration files are as stamped, or set the two keys as they did: then their new stamps are taken. */
  #settingsUnchanged(): boolean {
    const stamps = stampsOf(this.#configs);
    if (stamps === this.#configStamps) return true;
    if (JSON.stringify(ignoreSettings(this.#configs)) !== this.#settings) return false;
    this.#configStamps = stamps;
    return true;
  }
}

/** `root`'s Git folder and the common one every worktree shares, read from its `.git`; undefined without one. */
function gitFolders(root: string): { git: string; common: string } | undefined {
  const dotGit = join(root, ".git");
  const info = statSync(dotGit, { throwIfNoEntry: false });
  if (info === undefined) return undefined;
  if (info.isDirectory()) return { git: dotGit, common: dotGit };
  const named = /^gitdir: *(.+)$/mu.exec(readText(dotGit))?.[1]?.trim();
  if (named === undefined) return undefined;
  const git = resolve(root, named);
  const common = readText(join(git, "commondir")).trim();
  return { git, common: common === "" ? git : resolve(git, common) };
}

/** The configuration files that can name an excludes file: the repository's, the worktree's and the global ones. */
function configFiles(folders: { git: string; common: string }, env: NodeJS.ProcessEnv): string[] {
  const global = env.GIT_CONFIG_GLOBAL;
  const globals = global === undefined || global === "" ? [join(gitHome(env), ".gitconfig"), join(dirname(defaultExcludesFile(env)), "config")] : [global];
  return [join(folders.common, "config"), join(folders.git, "config.worktree"), ...globals];
}

/** `core.excludesFile` and `core.ignoreCase` in a configuration file, in any section: one set elsewhere is read too, never missed. */
const IGNORE_SETTING = /^\s*(excludesfile|ignorecase)\s*=\s*"?([^"\r\n]*?)"?\s*$/gimu;

/** What the configuration files set of the two keys, file by file, in order. */
function ignoreSettings(configs: readonly string[]): Array<{ key: string; value: string }> {
  return configs.flatMap(config => [...readText(config).matchAll(IGNORE_SETTING)].map(match => ({
    key: (match[1] ?? "").toLowerCase(),
    // Git writes a backslash or a quote in a value with a backslash before it (a Windows path: `C:\\Users\\...`).
    value: (match[2] ?? "").replace(/\\(["\\])/gu, "$1"),
  })));
}

function namedExcludesFiles(settings: ReadonlyArray<{ key: string; value: string }>, root: string, env: NodeJS.ProcessEnv): string[] {
  return settings
    .filter(setting => setting.key === "excludesfile" && setting.value !== "")
    .map(({ value }) => (value.startsWith("~/") ? join(gitHome(env), value.slice(2)) : resolve(root, value)));
}

function readText(file: string): string {
  try {
    return readFileSync(file, "utf8");
  } catch {
    return "";
  }
}

/** Each file's stamp, `-` for one absent and `!` for one that cannot be read, in one string. */
function stampsOf(files: readonly string[]): string {
  return files.map(file => {
    try {
      const info = statSync(file, { throwIfNoEntry: false });
      return info === undefined ? "-" : fileStamp(info);
    } catch {
      return "!";
    }
  }).join("|");
}
