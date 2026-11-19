/**
 * Which submodules a `git diff --name-only` lists for a changed worktree, found without letting Git look into one.
 *
 * To tell whether a checked-out submodule's worktree changed, Git's diff starts `git status --porcelain=2` inside it.
 * That status reads the submodule's own config with nothing of Rerail's guard on it (`git-filter-drivers.ts`), so it
 * runs the submodule's filter drivers, and where a submodule is registered but not checked out, a writer of the
 * worktree alone can put a Git folder there for Git to look into. So Rerail runs such a diff with
 * `--ignore-submodules=dirty`, which still lists a submodule whose commit moved, and asks each checked-out submodule the
 * same question itself, through the guard: its config listed, its drivers set, then its status. This module holds Git's
 * rules (2.31 to 2.53) for which submodules a diff looks into (`set_diffopt_flags_from_submodule_config` and
 * `repo_diff_setup`; for the submodules inside a submodule, `wt_status_collect_changes_worktree`) and for what makes a
 * submodule changed (`is_submodule_modified`): any entry its status lists.
 */
import { accessSync, closeSync, constants, existsSync, lstatSync, openSync, readFileSync, readlinkSync, readSync, statSync, type Stats } from "node:fs";
import { join, resolve } from "node:path";
import { lastValue, type ListedEntry } from "./git-filter-drivers.js";

/** How a diff treats a submodule's worktree: Git's `--ignore-submodules` values. */
export type SubmoduleIgnore = "none" | "untracked" | "dirty" | "all";
const IGNORE_VALUES: ReadonlySet<string> = new Set(["none", "untracked", "dirty", "all"]);

/** A Git question that failed, as `runGit` reports it. */
export interface GitFailure {
  readonly status: number | null;
  readonly stdout: string;
  readonly stderr: string;
  readonly error?: Error;
}

/** Where a repository's worktree is, as a question asked in it sees it. */
export interface Places {
  /** The worktree's top folder. */
  readonly top: string;
  /** The folder the question runs in, relative to the top: empty at the top, else ending in `/`. */
  readonly prefix: string;
}

/** A repository asked through the guard on Rerail's Git calls: each question runs with the drivers its config defines set. */
export interface GuardedRepository {
  /** Stdout of `git <args>` run in the repository, or how it failed. */
  readonly ask: (args: readonly string[]) => string | GitFailure;
  /** The repository's config, every scope, as the guard listed it. */
  readonly entries: readonly ListedEntry[];
  readonly places: Places;
}

/** The repository a diff runs in: its places are asked only once its index is seen to record a submodule. */
export interface DiffRepository extends Omit<GuardedRepository, "places"> {
  readonly place: () => Places | GitFailure;
}

/**
 * What an opener answers for a submodule the guard cannot ask ({@link askableWithoutConfig}): a diff lists it as
 * changed, and no Git starts in it.
 */
export const UNASKED = "unasked";

/**
 * Opens the submodule checked out at `folder`, whose Git folder is `gitFolder` ({@link submoduleGitFolder}), through the
 * guard, as Git opens one for its status; how that failed instead, or {@link UNASKED}.
 */
export type SubmoduleOpener = (folder: string, gitFolder: string) => GuardedRepository | GitFailure | typeof UNASKED;

/** A `git diff --name-only -z` this module answers for submodules. */
export interface NameDiff {
  /** Paths relative to the folder Git runs in, and only those below it (`--relative`). */
  readonly relative: boolean;
  /** The `--ignore-submodules` its options set, which overrides every config; undefined when they set none. */
  readonly ignore: SubmoduleIgnore | undefined;
  /** Whether its `--diff-filter` lets a changed entry (`M`) through. */
  readonly listsModified: boolean;
}

/** The options of a diff: its arguments after `diff` and before `--`. */
function diffOptions(args: readonly string[]): readonly string[] {
  const end = args.indexOf("--");
  return args.slice(1, end < 0 ? undefined : end);
}

/**
 * `args` (from `diff` on) as a {@link NameDiff}; undefined for any other form: another output, a pathspec, or
 * `--relative=<folder>`, which this module does not take apart.
 */
export function nameDiff(args: readonly string[]): NameDiff | undefined {
  if (!namesOnly(args)) return undefined;
  const options = diffOptions(args);
  return { relative: options.includes("--relative"), ignore: ignoreOption(options), listsModified: listsModified(options) };
}

/** Whether `args` are a `git diff --name-only -z` with no pathspec and no `--relative=<folder>`. */
function namesOnly(args: readonly string[]): boolean {
  const options = diffOptions(args);
  const end = args.indexOf("--");
  const noPathspec = end < 0 || end === args.length - 1;
  return args[0] === "diff" && noPathspec && ["--name-only", "-z"].every(option => options.includes(option))
    && !options.some(option => option.startsWith("--relative="));
}

/** The `--ignore-submodules` `options` set last (a bare one is `all`); undefined without one or with a value Git refuses. */
function ignoreOption(options: readonly string[]): SubmoduleIgnore | undefined {
  const last = options.findLast(option => option === "--ignore-submodules" || option.startsWith("--ignore-submodules="));
  const value = last === "--ignore-submodules" ? "all" : last?.slice("--ignore-submodules=".length);
  return value !== undefined && IGNORE_VALUES.has(value) ? value as SubmoduleIgnore : undefined;
}

/** Whether the last `--diff-filter` of `options` lets `M` through: upper case letters choose, lower case ones leave out. */
function listsModified(options: readonly string[]): boolean {
  const filter = options.findLast(option => option.startsWith("--diff-filter="))?.slice("--diff-filter=".length);
  if (filter === undefined) return true;
  return !filter.includes("m") && (filter.includes("M") || !/[A-Z]/u.test(filter));
}

/** `args` with `--ignore-submodules=dirty` as their last option, so Git looks into no submodule's worktree. */
export function outsideSubmodules(args: readonly string[]): string[] {
  const end = args.indexOf("--");
  return end < 0 ? [...args, "--ignore-submodules=dirty"] : [...args.slice(0, end), "--ignore-submodules=dirty", ...args.slice(end)];
}

/** The arguments that list a repository's whole index, each path from its top: `<mode> <object> <stage> TAB <path> NUL`. */
export const GITLINK_LISTING = ["ls-files", "-z", "--stage", "--full-name", "--", ":/"] as const;

/** The submodules a repository's index records, each path from its top, and whether its `.gitmodules` is in conflict. */
export interface Gitlinks {
  readonly paths: readonly string[];
  readonly gitmodulesUnmerged: boolean;
}

/** The {@link Gitlinks} of a {@link GITLINK_LISTING} output. */
export function readGitlinks(listing: string): Gitlinks {
  const entries = listing.split("\0").filter(entry => entry !== "").map(entry => {
    const tab = entry.indexOf("\t");
    const [mode, , stage] = entry.slice(0, tab).split(" ");
    return { mode, stage, path: entry.slice(tab + 1) };
  });
  return {
    paths: entries.filter(entry => entry.mode === "160000" && entry.stage === "0").map(entry => entry.path),
    gitmodulesUnmerged: entries.some(entry => entry.path === ".gitmodules" && entry.stage !== "0"),
  };
}

/** What a repository's `.gitmodules` says of its submodules: each one's name by path, and the `ignore` each sets. */
export interface Gitmodules {
  readonly names: ReadonlyMap<string, string>;
  readonly ignores: ReadonlyMap<string, SubmoduleIgnore>;
}

/**
 * The {@link Gitmodules} of a `git config -z --list` output of `.gitmodules`. A name's last `path` holds, and an `ignore`
 * Git does not take is left out, as Git's submodule config leaves it.
 */
export function readGitmodules(listing: string): Gitmodules {
  const names = new Map<string, string>();
  const ignores = new Map<string, SubmoduleIgnore>();
  for (const entry of listing.split("\0")) {
    const match = /^submodule\.(.+)\.(path|ignore)\n(.*)$/su.exec(entry);
    if (match === null) continue;
    const [, name, key, value] = match as unknown as [string, string, string, string];
    if (key === "path") names.set(value, name);
    else if (IGNORE_VALUES.has(value)) ignores.set(name, value as SubmoduleIgnore);
  }
  return { names, ignores };
}

/** What decides how a diff in one repository treats each of its submodules. */
interface IgnoreRules {
  readonly entries: readonly ListedEntry[];
  readonly gitmodules: Gitmodules;
  readonly unmerged: boolean;
  /** How it treats a submodule its `.gitmodules` does not name. */
  readonly base: SubmoduleIgnore;
}

/**
 * How the diff treats the submodule at `path`: `submodule.<name>.ignore` from the config, else from `.gitmodules`, else
 * `all` while `.gitmodules` is in conflict, else the base; undefined for a configured value Git refuses, which fails it.
 */
function submoduleIgnore(rules: IgnoreRules, path: string): SubmoduleIgnore | undefined {
  const name = rules.gitmodules.names.get(path);
  if (name === undefined) return rules.base;
  const configured = lastValue(rules.entries, `submodule.${name}.ignore`);
  if (configured !== undefined) return IGNORE_VALUES.has(configured) ? configured as SubmoduleIgnore : undefined;
  return rules.gitmodules.ignores.get(name) ?? (rules.unmerged ? "all" : rules.base);
}

/** `diff.ignoreSubmodules` as `entries` set it; null when unset, undefined for a value Git refuses. */
function configuredDiffIgnore(entries: readonly ListedEntry[]): SubmoduleIgnore | null | undefined {
  const value = lastValue(entries, "diff.ignoresubmodules");
  if (value === undefined) return null;
  return IGNORE_VALUES.has(value) ? value as SubmoduleIgnore : undefined;
}

/** How a `git diff` with no `--ignore-submodules` treats a submodule `.gitmodules` does not name: untracked files left out. */
function diffBase(entries: readonly ListedEntry[]): SubmoduleIgnore | undefined {
  const configured = configuredDiffIgnore(entries);
  return configured === null ? "untracked" : configured;
}

/**
 * How the `git status` Git runs in a submodule treats the submodules inside it, for one that lists untracked files
 * (`untrackedShown`) or not.
 */
function statusBase(entries: readonly ListedEntry[], untrackedShown: boolean): SubmoduleIgnore | undefined {
  const configured = configuredDiffIgnore(entries);
  if (configured === undefined) return undefined;
  if (untrackedShown) return configured ?? "none";
  const base = configured ?? "untracked";
  return base === "none" ? "untracked" : base;
}

/** Whether a `git status` without `-uno` lists untracked files: `status.showUntrackedFiles` is not `no` or false. */
function showsUntracked(entries: readonly ListedEntry[]): boolean {
  const value = lastValue(entries, "status.showuntrackedfiles")?.toLowerCase();
  return value === undefined || !["no", "false", "off", "0", ""].includes(value);
}

/** What a `git status --porcelain=2 -z --ignore-submodules=dirty` lists of its own files, and of each submodule. */
interface StatusRead {
  /** Any entry that is not a submodule's. */
  own: boolean;
  /** Each submodule entry by path: whether its index differs from `HEAD`, and its worktree from the index. */
  readonly submodules: Map<string, { readonly staged: boolean; readonly moved: boolean }>;
}

/** The {@link StatusRead} of a status output. */
export function readStatus(output: string): StatusRead {
  const read: StatusRead = { own: false, submodules: new Map() };
  const tokens = output.split("\0");
  for (let at = 0; at < tokens.length; at += 1) {
    const token = tokens[at]!;
    // A rename's entry is followed by the path it had.
    if (token.startsWith("2 ")) at += 1;
    readEntry(read, token);
  }
  return read;
}

function readEntry(read: StatusRead, entry: string): void {
  if (entry === "") return;
  if (!entry.startsWith("1 ") || entry[5] !== "S") {
    read.own = true;
    return;
  }
  const fields = entry.split(" ");
  const changes = fields[1]!;
  read.submodules.set(fields.slice(8).join(" "), { staged: changes[0] !== ".", moved: changes[1] !== "." });
}

/** How far submodules inside submodules are followed; past it the call fails. */
const MAX_DEPTH = 16;

/** One submodule of a diff: its path from the repository's top, and as the diff names it. */
interface Shown {
  readonly path: string;
  readonly name: string;
}

/** The submodules of `paths` the diff names, each as it names it: below its folder and from there with `--relative`. */
function shownSubmodules(paths: readonly string[], prefix: string, relative: boolean): Shown[] {
  if (!relative) return paths.map(path => ({ path, name: path }));
  return paths.filter(path => path.startsWith(prefix)).map(path => ({ path, name: path.slice(prefix.length) }));
}

/** `.gitmodules` as Git reads it for submodule config: the worktree's file, else the index's, else `HEAD`'s. */
function gitmodulesOf(repository: GuardedRepository): Gitmodules | GitFailure {
  const file = join(repository.places.top, ".gitmodules");
  if (existsSync(file)) {
    const listed = repository.ask(["config", "-z", "--list", "--file", file]);
    return typeof listed === "string" ? readGitmodules(listed) : listed;
  }
  const blob = [":.gitmodules", "HEAD:.gitmodules"].map(source => repository.ask(["config", "-z", "--list", "--blob", source])).find(listed => typeof listed === "string");
  return readGitmodules(typeof blob === "string" ? blob : "");
}

const NO_GITMODULES: Gitmodules = { names: new Map(), ignores: new Map() };

/** How a diff in `repository` treats each submodule, given its base; how it fails for a base Git refuses. */
function ignoreRules(repository: GuardedRepository, gitlinks: Gitlinks, base: SubmoduleIgnore | undefined): IgnoreRules | GitFailure {
  if (base === undefined) return refused("diff.ignoreSubmodules");
  const gitmodules = gitlinks.paths.length === 0 ? NO_GITMODULES : gitmodulesOf(repository);
  return "names" in gitmodules ? { entries: repository.entries, gitmodules, unmerged: gitlinks.gitmodulesUnmerged, base } : gitmodules;
}

/** The rules of a diff whose `--ignore-submodules` option overrides every config: `ignore` for each submodule. */
function overriding(repository: GuardedRepository, ignore: SubmoduleIgnore): IgnoreRules {
  return { entries: repository.entries, gitmodules: NO_GITMODULES, unmerged: false, base: ignore };
}

function refused(what: string): GitFailure {
  return { status: 128, stdout: "", stderr: `fatal: bad --ignore-submodules argument in ${what}\n` };
}

function failed(error: Error): GitFailure {
  return { status: null, stdout: "", stderr: "", error };
}

/**
 * Git's `diff` stdout for `diff`, from `stdout`, its stdout with {@link outsideSubmodules} in `top`: each submodule the
 * diff looks into whose worktree changed added, and with no `--ignore-submodules` option, each the config tells it to
 * ignore whole left out. The names keep Git's order (byte order of the paths). How it failed instead when a submodule
 * cannot be asked, as Git's diff fails then. An index that records no submodule leaves `stdout` as it is.
 */
export function withChangedSubmodules(stdout: string, diff: NameDiff, top: DiffRepository, open: SubmoduleOpener): string | GitFailure {
  const listing = top.ask(GITLINK_LISTING);
  if (typeof listing !== "string") return listing;
  const gitlinks = readGitlinks(listing);
  if (gitlinks.paths.length === 0) return stdout;
  const places = top.place();
  return "top" in places ? changedSubmodules(stdout, { diff, gitlinks, top: { ...top, places }, open }) : places;
}

/** What {@link changedSubmodules} decides from: the diff, the submodules its index records, its repository and the opener. */
interface DiffSubmodules {
  readonly diff: NameDiff;
  readonly gitlinks: Gitlinks;
  readonly top: GuardedRepository;
  readonly open: SubmoduleOpener;
}

/** {@link withChangedSubmodules} once the index is seen to record submodules. */
function changedSubmodules(stdout: string, { diff, gitlinks, top, open }: DiffSubmodules): string | GitFailure {
  const rules = topRules(top, gitlinks, diff);
  if (!("base" in rules)) return rules;
  const state: DiffState = { named: new Set(stdout.split("\0").filter(name => name !== "")), changed: false, rules, diff, top, open };
  for (const shown of shownSubmodules(gitlinks.paths, top.places.prefix, diff.relative)) {
    const failure = submoduleChange(shown, state);
    if (failure !== undefined) return failure;
  }
  return state.changed ? namesText(state.named) : stdout;
}

/** How the diff treats each submodule: as its option says, else as the config and `.gitmodules` of `top` say. */
function topRules(top: GuardedRepository, gitlinks: Gitlinks, diff: NameDiff): IgnoreRules | GitFailure {
  return diff.ignore === undefined ? ignoreRules(top, gitlinks, diffBase(top.entries)) : overriding(top, diff.ignore);
}

/** `names` as `git diff --name-only -z` prints them: in byte order, each ended by NUL. */
function namesText(names: ReadonlySet<string>): string {
  return [...names].sort(byteOrder).map(name => `${name}\0`).join("");
}

/** What deciding one submodule of a diff reads and changes. */
interface DiffState {
  readonly named: Set<string>;
  /** Whether a name was added or left out. */
  changed: boolean;
  readonly rules: IgnoreRules;
  readonly diff: NameDiff;
  readonly top: GuardedRepository;
  readonly open: SubmoduleOpener;
}

/** Names or leaves out `shown` in `state.named` as Git's diff would; how it failed instead, else undefined. */
function submoduleChange(shown: Shown, state: DiffState): GitFailure | undefined {
  const ignore = submoduleIgnore(state.rules, shown.path);
  if (ignore === undefined) return refused(`submodule.${shown.path}.ignore`);
  if (ignore === "all") leaveOut(state, shown.name);
  if (!askedOf(state, shown.name, ignore)) return undefined;
  const changed = submoduleChanged(join(state.top.places.top, shown.path), ignore === "untracked", state.open, MAX_DEPTH);
  if (typeof changed !== "boolean") return changed;
  if (changed) addName(state, shown.name);
  return undefined;
}

/** Whether the diff asks the submodule it names `name` about its worktree: not named already, looked into, and listable. */
function askedOf(state: DiffState, name: string, ignore: SubmoduleIgnore): boolean {
  return !state.named.has(name) && lookedInto(ignore) && state.diff.listsModified;
}

function leaveOut(state: DiffState, name: string): void {
  if (state.named.delete(name)) state.changed = true;
}

function addName(state: DiffState, name: string): void {
  state.named.add(name);
  state.changed = true;
}

function lookedInto(ignore: SubmoduleIgnore): boolean {
  return ignore === "none" || ignore === "untracked";
}

/** Orders names as Git's index does: by the bytes of their UTF-8 form. */
function byteOrder(left: string, right: string): number {
  return Buffer.compare(Buffer.from(left), Buffer.from(right));
}

/**
 * Whether the submodule at `folder` has a changed worktree as `is_submodule_modified` finds it: its `git status` lists
 * any entry, without untracked files when `untrackedLeftOut`. False when it is not checked out (no `.git`); how it
 * failed when its `.git` names no repository, as Git's diff fails then; true, with no Git started in it, when the guard
 * cannot ask it ({@link UNASKED}).
 */
export function submoduleChanged(folder: string, untrackedLeftOut: boolean, open: SubmoduleOpener, depth: number): boolean | GitFailure {
  const gitFolder = submoduleGitFolder(folder);
  if (typeof gitFolder !== "string") return gitFolder ?? false;
  if (depth === 0) return failed(new Error(`Rerail follows submodules ${MAX_DEPTH} levels deep, and ${folder} is deeper`));
  const repository = open(folder, gitFolder);
  if (repository === UNASKED) return true;
  return "ask" in repository ? changedThroughGuard(repository, untrackedLeftOut, { open, depth }) : repository;
}

/** The most bytes of a `.git` file Rerail reads for the Git folder it names. */
const MAX_GITFILE_BYTES = 64 * 1024;

/**
 * The Git folder of the submodule checked out at `folder`, as `is_submodule_modified` finds it: `.git` when it is a
 * folder, else the folder a `.git` file's `gitdir: <path>` names, relative to `folder`. Undefined when there is no
 * `.git`, or it went before it could be read: the submodule is not checked out. How it failed for a `.git` file Git
 * cannot read as one, or a Git folder that holds no repository ({@link holdsRepository}), each of which fails Git's
 * diff. Read once for each question and never handed to Git as a file, so the guard's listing and the status after it
 * open this one folder whatever `.git` names by then.
 */
export function submoduleGitFolder(folder: string): string | GitFailure | undefined {
  const dotGit = join(folder, ".git");
  const stats = statOf(dotGit);
  if (stats === undefined) return undefined;
  const gitFolder = stats.isDirectory() ? dotGit : namedGitFolder(folder, dotGit, stats);
  return typeof gitFolder === "string" && !holdsRepository(gitFolder) ? notARepository(gitFolder) : gitFolder;
}

/**
 * Whether `gitFolder` holds a repository as Git's `is_git_directory` decides it, read in process: a `HEAD` Git takes
 * ({@link validHead}), and an `objects` and a `refs` it can enter in the repository's common directory.
 */
function holdsRepository(gitFolder: string): boolean {
  const common = validHead(join(gitFolder, "HEAD")) ? commonDirectory(gitFolder) : undefined;
  return common !== undefined && enterable(join(common, "objects")) && enterable(join(common, "refs"));
}

/**
 * Whether Rerail asks the submodule whose Git folder is `gitFolder`, once the guard's listing of its config showed none
 * of the repository's own: read again in process, the folder holds a repository whose config file holds no entry, so
 * it defines no driver. A listing shows the same for a Git folder a writer took away while Git read it; one put back
 * by now, or a config that gained an entry, is not asked, so a status never opens a config the listing did not read.
 */
export function askableWithoutConfig(gitFolder: string): boolean {
  const common = holdsRepository(gitFolder) ? commonDirectory(gitFolder) : undefined;
  return common !== undefined && holdsNoEntry(join(common, "config"));
}

/**
 * The common directory of the repository in `gitFolder`, as Git's `get_common_dir` finds it: the folder its `commondir`
 * file names, relative to it, else `gitFolder`; undefined when that file cannot be read as one.
 */
function commonDirectory(gitFolder: string): string | undefined {
  const file = join(gitFolder, "commondir");
  if (!existsSync(file)) return gitFolder;
  const named = textOf(file)?.replace(/[\r\n]+$/u, "");
  return named === undefined ? undefined : resolve(gitFolder, named);
}

/** Whether Git takes the `HEAD` at `path` (`validate_headref`): a link into `refs/`, `ref: refs/…`, or an object id first. */
function validHead(path: string): boolean {
  const stats = lstatOf(path);
  if (stats?.isSymbolicLink() === true) return linkOf(path)?.startsWith("refs/") === true;
  const text = headText(path);
  return text !== undefined && (/^ref:[ \t\n\r]*refs\//u.test(text) || /^[0-9a-fA-F]{40}/u.test(text));
}

/** The first bytes of `path`, as many as Git reads of a `HEAD` (255); undefined when it cannot be read. */
function headText(path: string): string | undefined {
  try {
    const file = openSync(path, "r");
    try {
      const head = Buffer.alloc(255);
      return head.toString("latin1", 0, readSync(file, head));
    } finally {
      closeSync(file);
    }
  } catch {
    return undefined;
  }
}

/** Whether the config file at `path` is absent or holds only blank lines, comments and section headers. */
function holdsNoEntry(path: string): boolean {
  if (!existsSync(path)) return true;
  const lines = textOf(path)?.split("\n");
  return lines !== undefined && lines.every(line => /^\s*(?:[#;].*|\[[^\]]*\]\s*(?:[#;].*)?)?$/u.test(line));
}

/** Whether Git could enter `path` (`access(path, X_OK)`); on Windows, whether it is there. */
function enterable(path: string): boolean {
  try {
    accessSync(path, constants.X_OK);
    return true;
  } catch {
    return false;
  }
}

function lstatOf(path: string): Stats | undefined {
  try {
    return lstatSync(path);
  } catch {
    return undefined;
  }
}

function linkOf(path: string): string | undefined {
  try {
    return readlinkSync(path);
  } catch {
    return undefined;
  }
}

/**
 * The Git folder the `.git` file `dotGit` (`stats`) names, relative to `folder`; undefined when it went before it could
 * be read, and how it failed for one Git cannot read as a `.git` file (not a regular file, too large, no `gitdir:`).
 */
function namedGitFolder(folder: string, dotGit: string, stats: Stats): string | GitFailure | undefined {
  const text = stats.isFile() && stats.size <= MAX_GITFILE_BYTES ? textOf(dotGit) : "";
  if (text === undefined) return undefined;
  return text.startsWith("gitdir: ") ? resolve(folder, text.slice("gitdir: ".length).replace(/[\r\n]+$/u, "")) : notARepository(dotGit);
}

function statOf(path: string): Stats | undefined {
  try {
    return statSync(path);
  } catch {
    return undefined;
  }
}

function textOf(path: string): string | undefined {
  try {
    return readFileSync(path, "utf8");
  } catch {
    return undefined;
  }
}

/** How a question fails in `gitFolder`, which holds no repository, as Git fails it. */
export function notARepository(gitFolder: string): GitFailure {
  return { status: 128, stdout: "", stderr: `fatal: not a git repository: ${gitFolder}\n` };
}

/**
 * {@link submoduleChanged} for a submodule opened through the guard: its status, then the submodules inside it. The
 * status finds no renames: a partial clone fails rename detection on a blob it does not hold, as Rerail fetches none,
 * and a rename makes the submodule changed as surely as the deletion and addition it is listed as instead.
 */
function changedThroughGuard(repository: GuardedRepository, untrackedLeftOut: boolean, deeper: { open: SubmoduleOpener; depth: number }): boolean | GitFailure {
  const status = repository.ask(["status", "--porcelain=2", "-z", "--no-renames", "--ignore-submodules=dirty", ...(untrackedLeftOut ? ["-uno"] : [])]);
  if (typeof status !== "string") return status;
  const read = readStatus(status);
  // A submodule inside whose index differs from HEAD is listed whatever its rules: Git compares the index with HEAD so.
  return read.own || [...read.submodules.values()].some(entry => entry.staged) || nestedChanged(repository, read, untrackedLeftOut, deeper);
}

/**
 * Whether a submodule inside `repository` makes it changed, as the `git status` Git runs in `repository` finds them: by
 * its own `ignore` rules, its base set by `diff.ignoreSubmodules` and whether that status lists untracked files.
 */
function nestedChanged(
  repository: GuardedRepository, read: StatusRead, untrackedLeftOut: boolean, deeper: { open: SubmoduleOpener; depth: number },
): boolean | GitFailure {
  const listing = repository.ask(GITLINK_LISTING);
  if (typeof listing !== "string") return listing;
  const gitlinks = readGitlinks(listing);
  if (gitlinks.paths.length === 0) return false;
  const rules = ignoreRules(repository, gitlinks, statusBase(repository.entries, !untrackedLeftOut && showsUntracked(repository.entries)));
  if (!("base" in rules)) return rules;
  return firstChanged(gitlinks.paths, { repository, read, rules, ...deeper });
}

/** Whether any submodule at `paths` inside `nested.repository` makes it changed, asked in order; how asking one failed instead. */
function firstChanged(paths: readonly string[], nested: Nested): boolean | GitFailure {
  for (const path of paths) {
    const changed = nestedSubmoduleChanged(path, nested);
    if (changed !== false) return changed;
  }
  return false;
}

/** One submodule inside another, and what decides whether it changed. */
interface Nested {
  readonly repository: GuardedRepository;
  readonly read: StatusRead;
  readonly rules: IgnoreRules;
  readonly open: SubmoduleOpener;
  readonly depth: number;
}

/** Whether the submodule at `path` in `nested.repository` makes it changed; how asking it failed instead. */
function nestedSubmoduleChanged(path: string, nested: Nested): boolean | GitFailure {
  const ignore = submoduleIgnore(nested.rules, path);
  if (ignore === undefined) return refused(`submodule.${path}.ignore`);
  if (ignore === "all") return false;
  if (nested.read.submodules.get(path)?.moved === true) return true;
  if (!lookedInto(ignore)) return false;
  return submoduleChanged(join(nested.repository.places.top, path), ignore === "untracked", nested.open, nested.depth - 1);
}
