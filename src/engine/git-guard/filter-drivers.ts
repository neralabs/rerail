/**
 * The filter drivers a Git call could start, and the config that keeps a repository's own drivers from starting.
 *
 * Git converts a worktree file to its stored form with the `filter.<name>.clean` or `filter.<name>.process` program a
 * config file names, for each file whose `.gitattributes` names that driver, whenever it compares the file's bytes
 * with the index: `git diff` against the worktree, `git ls-files --modified`, `git status`, and the `git status` Git
 * runs in each submodule to see whether it is dirty. It compares bytes for a tracked file whose size still matches its
 * index entry while its other stat data does not, which touching the file or copying the checkout is enough for.
 *
 * Repository content is untrusted (docs/threat-model.md, B3), and a repository's own config (`.git/config`, its
 * worktree config and the files they include) is repository content. So each call that reads a worktree file gets every
 * driver a repository's config defines set as the person's own config defines it (the system, global and command-line
 * scopes), or set to no program where theirs defines none ({@link driverOverrides}). A driver the person installed,
 * such as Git LFS from its global config, still runs, and the call answers as their Git does. A file under a driver
 * only the repository defines is compared as it is on disk: it is listed when its bytes differ from the index's, which
 * a changed file's do, and also when only its stat changed and the repository's program would have mapped its bytes to
 * the index's; it is then read and checked like any changed file.
 */

/** What a Git call reads from the worktree: nothing, its files, or its files and each submodule's through `git status`. */
export type WorktreeRead = "none" | "files" | "files-and-submodules";

/** Git commands that never convert a worktree file, whatever their options. */
const NO_CONVERSION = new Set([
  "check-attr", "check-ignore", "config", "for-each-ref", "ls-remote", "ls-tree", "merge-base", "remote", "rev-list", "rev-parse",
  "reflog", "show-ref", "symbolic-ref", "var", "version",
]);

/** How the options of a command that can convert a worktree file decide what it reads. */
const READ_BY_OPTIONS: Readonly<Record<string, (options: readonly string[]) => WorktreeRead>> = {
  "ls-files": options => (options.some(option => option === "--modified" || /^-[^-]*m/u.test(option)) ? "files" : "none"),
  "cat-file": options => (options.some(option => option === "--filters" || option === "--textconv") ? "files" : "none"),
  worktree: options => (options[0] === "list" ? "none" : "files"),
  status: options => (ignoresSubmoduleWorktrees(options) ? "files" : "files-and-submodules"),
  diff: diffRead,
};

/** Git's options before a command that take their value as the next argument. */
const VALUED_GIT_OPTIONS = new Set(["-c", "-C"]);

/** Where the command stands in `args`, the arguments after `git -C <folder>`: past every option and its value. */
function commandAt(args: readonly string[]): number {
  let at = 0;
  while (at < args.length && args[at]!.startsWith("-")) at += VALUED_GIT_OPTIONS.has(args[at]!) ? 2 : 1;
  return at;
}

/**
 * What the Git call `args` (the arguments after `git -C <folder>`) reads from the worktree. A command this does not
 * know is taken to read files, so a new call is guarded until it is shown to need nothing.
 */
export function worktreeRead(args: readonly string[]): WorktreeRead {
  const at = commandAt(args);
  const command = args[at];
  if (command === undefined || NO_CONVERSION.has(command)) return "none";
  const byOptions = READ_BY_OPTIONS[command];
  return byOptions === undefined ? "files" : byOptions(args.slice(at + 1));
}

/** Whether `flags` keep Git out of every submodule's worktree: `--ignore-submodules`, `=all` or `=dirty`. */
function ignoresSubmoduleWorktrees(flags: readonly string[]): boolean {
  return flags.some(flag => /^--ignore-submodules(?:=(?:all|dirty))?$/u.test(flag));
}

/** A `git diff`: the index against a commit reads no file; the worktree is read, and its submodules unless ignored. */
function diffRead(options: readonly string[]): WorktreeRead {
  const end = options.indexOf("--");
  const flags = end < 0 ? options : options.slice(0, end);
  if (flags.some(flag => flag === "--cached" || flag === "--staged")) return "none";
  return ignoresSubmoduleWorktrees(flags) ? "files" : "files-and-submodules";
}

/** The arguments that list a repository's whole config, each entry with its scope: `<scope> NUL <key>[ LF <value>] NUL`. */
export const CONFIG_LISTING = ["config", "-z", "--show-scope", "--list"] as const;

/** One entry of a {@link CONFIG_LISTING} output. */
export interface ListedEntry {
  readonly scope: string;
  /** `section.name` or `section.subsection.name`: section and name lower-cased, the subsection as written. */
  readonly key: string;
  /** Undefined for a key with no `=`, which Git reads as true. */
  readonly value: string | undefined;
}

/** The entries of a {@link CONFIG_LISTING} output, in the order Git read them. */
export function listedEntries(listing: string): ListedEntry[] {
  const fields = listing.split("\0");
  const entries: ListedEntry[] = [];
  for (let at = 0; at + 1 < fields.length; at += 2) entries.push(listedEntry(fields[at]!, fields[at + 1]!));
  return entries;
}

function listedEntry(scope: string, entry: string): ListedEntry {
  const newline = entry.indexOf("\n");
  return newline < 0 ? { scope, key: entry, value: undefined } : { scope, key: entry.slice(0, newline), value: entry.slice(newline + 1) };
}

/** The value Git reads last for `key` in `entries`, undefined when none sets it; a key with no `=` reads as `true`. */
export function lastValue(entries: readonly ListedEntry[], key: string): string | undefined {
  const found = entries.findLast(entry => entry.key === key);
  return found === undefined ? undefined : found.value ?? "true";
}

/** The scopes the person writes: Git's build (`system`), their account (`global`) and the command line. */
const PERSON_SCOPES = new Set(["system", "global", "command"]);

/** The keys of a filter driver that start a program; `required` makes a failed one fail the call. */
const PROGRAM_KEYS = ["clean", "smudge", "process"] as const;
const DRIVER_KEYS = new Set<string>([...PROGRAM_KEYS, "required"]);

/** The filter drivers some config listings define: the person's values, and which keys repositories set. */
export interface DriverConfig {
  /** Each `filter.<name>.<key>` the person's scopes set, with the value Git reads last. */
  readonly person: ReadonlyMap<string, string>;
  /** Each driver name a repository's config sets, with the keys it sets. */
  readonly repository: ReadonlyMap<string, ReadonlySet<string>>;
}

/** The filter drivers the listed config `entries` define, read in order. */
export function readDriverConfig(entries: readonly ListedEntry[]): DriverConfig {
  const config = { person: new Map<string, string>(), repository: new Map<string, Set<string>>() };
  for (const entry of entries) addEntry(config, entry);
  return config;
}

/** Adds the listed `entry` to `config` when it is a filter driver's; a key with no `=` is true to Git. */
function addEntry(config: { person: Map<string, string>; repository: Map<string, Set<string>> }, entry: ListedEntry): void {
  const driver = driverKey(entry.key);
  if (driver === undefined) return;
  if (PERSON_SCOPES.has(entry.scope)) config.person.set(entry.key, entry.value ?? (driver.variable === "required" ? "true" : ""));
  else config.repository.set(driver.name, (config.repository.get(driver.name) ?? new Set()).add(driver.variable));
}

/** The driver name and variable of a `filter.<name>.<variable>` key, whose name may hold dots, `=` or nothing at all. */
function driverKey(key: string): { name: string; variable: string } | undefined {
  const dot = key.lastIndexOf(".");
  if (!key.startsWith("filter.") || dot < "filter.".length) return undefined;
  const variable = key.slice(dot + 1);
  return DRIVER_KEYS.has(variable) ? { name: key.slice("filter.".length, dot), variable } : undefined;
}

/**
 * The config entries that leave each driver a repository defines as the person's config defines it: each program key
 * either of them sets takes the person's value, or the empty value, which Git starts no program for. Git cannot be
 * told to unset a key, so where a repository adds a `process` the person's driver lacks, that empty `process` turns
 * the person's `clean` and `smudge` off as well. `required` is then false, since a required driver that starts nothing
 * fails the call; otherwise it is the person's.
 */
export function driverOverrides({ person, repository }: DriverConfig): Array<readonly [string, string]> {
  return [...repository].flatMap(([name, keys]) => {
    const own = (variable: string): string | undefined => person.get(`filter.${name}.${variable}`);
    const set = PROGRAM_KEYS.filter(variable => keys.has(variable) || own(variable) !== undefined);
    const intact = !keys.has("process") || own("process") !== undefined;
    return [
      ...set.map(variable => [`filter.${name}.${variable}`, own(variable) ?? ""] as const),
      [`filter.${name}.required`, intact ? own("required") ?? "false" : "false"] as const,
    ];
  });
}

/**
 * The variables that add `entries` as command-line config through `GIT_CONFIG_COUNT` to an environment `env`, after
 * any entries it holds. Unlike `-c`, which splits at the first `=`, these keep a driver name that holds `=`; Git reads
 * them after every config file.
 */
export function configEntryVariables(env: Readonly<Record<string, string | undefined>>, entries: ReadonlyArray<readonly [string, string]>): Record<string, string> {
  if (entries.length === 0) return {};
  const first = Number.parseInt(env.GIT_CONFIG_COUNT ?? "0", 10) || 0;
  const added: Record<string, string> = { GIT_CONFIG_COUNT: String(first + entries.length) };
  entries.forEach(([key, value], index) => {
    added[`GIT_CONFIG_KEY_${first + index}`] = key;
    added[`GIT_CONFIG_VALUE_${first + index}`] = value;
  });
  return added;
}

/**
 * The first Git the guard works with: it lists the config with each entry's scope (`--show-scope`, Git 2.26) and
 * passes its entries through `GIT_CONFIG_COUNT` (Git 2.31).
 */
export const GUARD_GIT = { major: 2, minor: 31 } as const;

/**
 * What Rerail says, once in each process that makes a Git call the guard would cover, when the `git version` answer
 * `version` names a Git older than {@link GUARD_GIT}: every call then runs as it ran before the guard. Undefined for
 * that Git or a newer one, and for an answer it cannot read.
 */
export function oldGitNotice(version: string): string | undefined {
  const match = /^git version (\d+)\.(\d+)(\S*)/mu.exec(version);
  if (match === null) return undefined;
  const [major, minor] = [Number(match[1]), Number(match[2])];
  if (major > GUARD_GIT.major || (major === GUARD_GIT.major && minor >= GUARD_GIT.minor)) return undefined;
  const needed = `${GUARD_GIT.major}.${GUARD_GIT.minor}`;
  return `Git ${match[1]}.${match[2]}${match[3]} is older than ${needed}, so Rerail's Git calls run the filter programs a repository's own config names, as that Git runs them. Update Git to ${needed} or later.`;
}
