/**
 * `HEAD` as `git rev-parse --verify --quiet HEAD` names it (`gitHead`), for callers that ask on every walk and pass: the
 * shared walk asks for each watched worktree every 5 s, and each session's walk for its checkouts on every pass. A Git
 * process per question blocks the daemon's thread while it starts, the longer the more memory the daemon holds.
 *
 * Git is asked at a checkout's first read, and again whenever one of the inputs that decide whether Git takes the
 * checkout changed: its `.git` entry, Git folder and common folder, the owners Git checks (`safe.directory`), the
 * presence of `objects` and `refs`, and every configuration file Git reads (the repository's `config`, the worktree's
 * `config.worktree`, the global and system files `git var` names, and each file an include names). While those inputs
 * hold, Git's last answer vouches that Git takes the checkout, and the commit is read from Git's own files as Git reads
 * it: `HEAD`, then the branch it names as a loose file in the common folder, else its line in `packed-refs`.
 *
 * Git is asked again, on every read, wherever those files do not decide alone: a symbolic ref that names another, a
 * branch name Windows would read as another file, a ref file that is not one object id, an object id of another length
 * than Git named, a file that is a link, a `GIT_DIR`, `GIT_WORK_TREE`, `GIT_COMMON_DIR` or `GIT_OBJECT_DIRECTORY` in the
 * environment, a folder that is not its checkout's top, and an input written less than `settleMs` ago (a write in the
 * same clock tick could leave its stamp unchanged). Before Git 2.42 `git var` names neither the global nor the system
 * file, and every read asks Git; a `git var` that failed is asked again at the next read, and an include spelled so that
 * only Git can name its file (`configuredPath`) makes every read ask Git. Any file of these that cannot be read makes the
 * read ask Git, never answer from what it read before. On Windows `stat` names no owner, so there each folder's stamp stands for its owner,
 * and any change to the checkout's folder, its `.git` entry or its Git folder asks Git again.
 */
import { constants } from "node:fs";
import { type FileHandle, lstat, open, realpath, stat } from "node:fs/promises";
import { dirname, isAbsolute, join, resolve } from "node:path";
import { fileStamp } from "./file-stamp.js";
import { gitHead, gitHome, type GitReadOptions, type GitResult, runGitAsync } from "./git.js";
import { openNoFollow } from "./no-follow.js";

/** Where a checkout's Git files are, as its `.git` entry and its Git folder's `commondir` name them. */
export interface Layout {
  readonly gitDir: string;
  readonly commonDir: string;
}

/** The commit `HEAD` names by Git's files, or `unborn` where it names a branch with no commit yet. */
type FilesHead = { readonly kind: "commit"; readonly commit: string } | { readonly kind: "unborn" };

/** The inputs under which Git's answer vouches for a checkout's files, as one string, and whether each has settled. */
export interface Vouch {
  readonly key: string;
  readonly settled: boolean;
}

/** What one read found in a checkout's files before asking anyone. */
interface FilesRead {
  readonly layout: Layout;
  readonly head: FilesHead | undefined;
  readonly vouch: Vouch | undefined;
}

/** What Git said of a checkout: its Git folders where the folder asked is its top, and the commit `HEAD` names. */
interface Asked {
  readonly layout: Layout | undefined;
  readonly head: string | undefined;
}

/** A file's stamp (`absent` where nothing is there) and whether it was last changed at least the settle time ago. */
interface Stamped {
  readonly stamp: string;
  readonly settled: boolean;
}

/** A configuration file Git reads, stamped. */
interface StampedFile extends Stamped {
  readonly path: string;
}

/** The owners Git checks (`safe.directory`) of the checkout's folder, `.git` entry and Git folder, as one string. */
export type OwnersRead = (paths: readonly string[]) => Promise<string | undefined>;

export interface GitHeadReaderOptions {
  /** How long an input must be unchanged before its stamp vouches for it; 2 s by default. */
  readonly settleMs?: number;
  /** Reads the owners: by default the user id on POSIX and, on Windows, where `stat` names no owner, each path's stamp. */
  readonly owners?: OwnersRead;
}

export type GitHeadReader = (cwd: string, options?: GitReadOptions) => Promise<string | undefined>;

const SETTLE_MS = 2_000;
const MAX_SMALL_FILE_BYTES = 64 * 1024;
const MAX_PACKED_REFS_BYTES = 64 * 1024 * 1024;
const MAX_CONFIG_BYTES = 1024 * 1024;
const OBJECT_ID = /^(?:[0-9a-f]{40}|[0-9a-f]{64})$/u;
/** Variables that make Git read another folder than the files at the checkout: then only Git answers. */
const MOVING_VARIABLES = ["GIT_DIR", "GIT_WORK_TREE", "GIT_COMMON_DIR", "GIT_OBJECT_DIRECTORY"];
/**
 * A part of a branch name that Git's `check-ref-format` refuses (empty, starting with `.`, ending with `.lock`, holding
 * `..`, `@{`, a control character, a space or one of `~^:?*[`), or that Windows would read as another file: one holding
 * a backslash, a folder separator there, or ending with `.`, which Windows drops. Such a name is never joined into a path.
 */
const REFUSED_REF_PART = /^$|^\.|\.$|\.lock$|\.\.|@\{|[\p{Cc} ~^:?*[\\]/u;
/** An include section of a configuration file, in any letter case: `[include]` or `[includeIf "..."]`. */
const INCLUDE_SECTION = /^\s*\[\s*include/imu;
/**
 * A configured path only Git can name the file of: empty, another user's home (`~user/`), Git's own prefix (`%(prefix)/`)
 * or a path with a magic prefix such as `:(optional)`. Git expands these itself (`interpolate_path`), by rules a reader
 * of its files does not keep, so where a file Git reads is spelled so, Git is asked.
 */
const GIT_ONLY_SPELLING = /^(?:$|%\(|:\(|~(?!\/))/u;
/** `git var`'s exit when Git knows no such variable, as before Git 2.42 for the configuration and attribute files. */
const UNKNOWN_VARIABLE = 129;

/** `absent` for an error that says nothing is there (or a folder is, as Windows refuses to open one); else undefined. */
function absence(error: unknown): "absent" | undefined {
  const code = error instanceof Error && "code" in error ? error.code : undefined;
  return code === "ENOENT" || code === "EISDIR" || code === "ENOTDIR" ? "absent" : undefined;
}

/** How a file is opened: `openNoFollow` refuses a link, `open` follows one. */
type Opener = (path: string, flags: number) => Promise<FileHandle>;

/**
 * A small regular file's text, never through a link unless `opener` follows one; `absent` where nothing or a folder is
 * there; else undefined.
 */
async function fileText(path: string, maxBytes = MAX_SMALL_FILE_BYTES, opener: Opener = openNoFollow): Promise<string | undefined> {
  let handle;
  try {
    handle = await opener(path, constants.O_RDONLY | (constants.O_NONBLOCK ?? 0));
  } catch (error) {
    return absence(error);
  }
  try {
    const info = await handle.stat();
    if (info.isDirectory()) return "absent";
    return info.isFile() && info.size <= maxBytes ? (await handle.readFile()).toString("utf8") : undefined;
  } catch {
    return undefined;
  } finally {
    await handle.close();
  }
}

async function folderAt(path: string): Promise<string | undefined> {
  try {
    // Git follows a link to its folders, as `stat` does.
    return (await stat(path)).isDirectory() ? path : undefined;
  } catch {
    return undefined;
  }
}

/** The checkout's Git folders by its `.git` entry: the folder itself, or the one a `gitdir:` file names. */
async function layoutOf(cwd: string): Promise<Layout | undefined> {
  const dotGit = join(cwd, ".git");
  const text = await fileText(dotGit);
  const gitDir = text === "absent" ? await folderAt(dotGit) : await namedGitDir(cwd, text);
  if (gitDir === undefined) return undefined;
  const commonDir = await commonDirOf(gitDir);
  return commonDir === undefined ? undefined : { gitDir, commonDir };
}

/** The Git folder a `.git` file's `gitdir:` line names, from the checkout's folder that holds the file (`asGitResolves`). */
async function namedGitDir(cwd: string, text: string | undefined): Promise<string | undefined> {
  const named = /^gitdir: (.+?)\r?\n?$/u.exec(text ?? "")?.[1];
  return named === undefined ? undefined : await asGitResolves(cwd, named);
}

/** The common folder `gitDir`'s `commondir` names from it (`asGitResolves`), `gitDir` itself without one. */
async function commonDirOf(gitDir: string): Promise<string | undefined> {
  const common = await fileText(join(gitDir, "commondir"));
  if (common === "absent") return gitDir;
  const named = common?.trimEnd() ?? "";
  return named === "" ? undefined : await asGitResolves(gitDir, named);
}

/**
 * The folder a `gitdir:` line or a `commondir` names from `base`, as Git finds it. On Linux and macOS Git takes the real
 * path of the joined text, so a link in it is followed before the `..` after it, where `resolve` cancels `link/..` as
 * text and names another folder; nothing is taken off the text before the system resolves it. Git for Windows opens
 * the joined path, which Windows cleans as text. Undefined, so Git answers, where the system cannot resolve it.
 */
async function asGitResolves(base: string, named: string): Promise<string | undefined> {
  if (process.platform === "win32") return resolve(base, named);
  try {
    return await realpath(isAbsolute(named) ? named : `${base}/${named}`);
  } catch {
    return undefined;
  }
}

/** The branch `HEAD` names (`refs/heads/...`), the commit it holds detached, or undefined where only Git can say. */
function headTarget(text: string | undefined): { readonly branch: string } | FilesHead | undefined {
  const detached = text?.replace(/\n$/u, "") ?? "";
  if (OBJECT_ID.test(detached)) return { kind: "commit", commit: detached };
  const branch = /^ref: (refs\/heads\/[^\n]+)\n?$/u.exec(text ?? "")?.[1];
  return branch === undefined || branch.split("/").some(part => REFUSED_REF_PART.test(part)) ? undefined : { branch };
}

/** The commit `HEAD` names by Git's files, where they decide it alone; undefined where only Git can say. */
async function headFromFiles(layout: Layout, packed: PackedRefs): Promise<FilesHead | undefined> {
  const target = headTarget(await fileText(join(layout.gitDir, "HEAD")));
  if (target === undefined || !("branch" in target)) return target;
  return await branchFromFiles(layout.commonDir, target.branch, packed);
}

/** A branch's commit: its loose file in the common folder, else its line in `packed-refs`, else none (unborn). */
async function branchFromFiles(commonDir: string, branch: string, packed: PackedRefs): Promise<FilesHead | undefined> {
  const loose = await fileText(join(commonDir, ...branch.split("/")));
  if (loose !== "absent") {
    const commit = loose?.replace(/\n$/u, "") ?? "";
    return OBJECT_ID.test(commit) ? { kind: "commit", commit } : undefined;
  }
  const branches = await packed(join(commonDir, "packed-refs"));
  const commit = branches?.get(branch);
  if (branches === undefined) return undefined;
  return commit === undefined ? { kind: "unborn" } : { kind: "commit", commit };
}

/** The branches of a `packed-refs` file by name, none where it is absent; undefined when it cannot be read whole. */
type PackedRefs = (path: string) => Promise<ReadonlyMap<string, string> | undefined>;

/** Reads `packed-refs` again only when its stamp moved or had not settled: every checkout of a repository shares it. */
function packedRefsReader(settleMs: number): PackedRefs {
  const kept = new Map<string, { readonly stamp: string; readonly branches: ReadonlyMap<string, string> }>();
  return async path => {
    const stamped = await stampOf(path, settleMs);
    // A file that cannot be stamped says nothing of what it holds now: Git answers, never the branches read before.
    if (stamped === undefined) return undefined;
    const last = kept.get(path);
    if (last?.stamp === stamped.stamp) return last.branches;
    kept.delete(path);
    const branches = packedBranches(await fileText(path, MAX_PACKED_REFS_BYTES));
    if (branches !== undefined && stamped.settled) kept.set(path, { stamp: stamped.stamp, branches });
    return branches;
  };
}

/** `<id> <ref>` lines that name a branch; the header and the peeled `^<id>` lines name none. */
function packedBranches(text: string | undefined): Map<string, string> | undefined {
  if (text === undefined) return undefined;
  const branches = new Map<string, string>();
  for (const line of text === "absent" ? [] : text.split("\n")) {
    const match = /^([0-9a-f]{40}|[0-9a-f]{64}) (refs\/heads\/\S+)\r?$/u.exec(line);
    if (match !== null) branches.set(match[2]!, match[1]!);
  }
  return branches;
}

/**
 * `path`'s stamp, or `absent`; undefined when it cannot be read. Git reads a configuration file or `packed-refs` through a
 * link, so the stamp is the file's the link names (`stat`): a write to that file moves it.
 */
async function stampOf(path: string, settleMs: number): Promise<Stamped | undefined> {
  try {
    const info = await stat(path);
    return { stamp: fileStamp(info), settled: settleMs <= 0 || Date.now() - Math.floor(info.ctimeMs) >= settleMs };
  } catch (error) {
    return absence(error) === undefined ? undefined : { stamp: "absent", settled: true };
  }
}

/** Each of `paths` stamped; undefined when one cannot be read. */
async function stampedFiles(paths: readonly string[], settleMs: number): Promise<StampedFile[] | undefined> {
  const stamps = await Promise.all(paths.map(async path => await stampOf(path, settleMs)));
  return stamps.every(stamped => stamped !== undefined) ? stamps.map((stamped, at) => ({ path: paths[at]!, ...stamped })) : undefined;
}

/** `files`' paths and stamps as one string. */
function filesKey(files: readonly StampedFile[]): string {
  return files.map(file => `${file.path}\0${file.stamp}`).join("\0");
}

/** Whether every one of `files` was last changed at least the settle time ago. */
function allSettled(files: readonly StampedFile[]): boolean {
  return files.every(file => file.settled);
}

/**
 * POSIX: each path's user id, which Git compares with its own. Windows: each path's stamp, which an owner change moves.
 * The checkout's folder (the first path) is the physical folder Git checks, as `stat` reads it; its `.git` entry and Git
 * folder are read as they are (`lstat`), as Git reads them.
 */
async function defaultOwners(paths: readonly string[]): Promise<string | undefined> {
  try {
    const infos = await Promise.all(paths.map(async (path, at) => (at === 0 ? await stat(path) : await lstat(path))));
    return infos.map(info => (process.platform === "win32" ? fileStamp(info) : String(info.uid))).join("\0");
  } catch {
    return undefined;
  }
}

/**
 * What a `git var` said of the files it names: their paths; `unknown` where they cannot be named once for every
 * checkout, an answer that holds while the daemon runs: Git knows no such variable (before Git 2.42), or names a
 * relative path (a relative `GIT_CONFIG_GLOBAL`), which each checkout's Git opens in its own folder; undefined where Git
 * could not answer, which the next question asks again.
 */
export type NamedFiles = string[] | "unknown" | undefined;

/** The global and system configuration files `git var` names for this process, as {@link NamedFiles}. */
async function askWideFiles(cwd: string): Promise<NamedFiles> {
  const named: string[] = [];
  for (const variable of ["GIT_CONFIG_SYSTEM", "GIT_CONFIG_GLOBAL"]) {
    const lines = namedFiles(await gitRun(cwd, ["var", variable]));
    if (!Array.isArray(lines)) return lines;
    named.push(...lines);
  }
  // Git for Windows reads its program data file before the system one.
  const programData = process.platform === "win32" ? process.env.PROGRAMDATA : undefined;
  return programData ? [...named, join(programData, "Git", "config")] : named;
}

/**
 * The files a `git var` names, absolute: none where it exits 1 printing nothing, as for `GIT_CONFIG_SYSTEM` under
 * `GIT_CONFIG_NOSYSTEM`; `unknown` where Git knows no such variable (its usage, exit 129); undefined for any other end:
 * Git did not start, or could not run in the folder or read its configuration.
 */
export function namedFiles(printed: GitResult | undefined): NamedFiles {
  if (printed?.status === UNKNOWN_VARIABLE) return "unknown";
  if (printed?.status === 1 && printed.stdout.trim() === "") return [];
  if (printed?.status !== 0) return undefined;
  const lines = printed.stdout.split(/\r?\n/u).filter(line => line !== "");
  return lines.every(line => isAbsolute(line)) ? lines.map(line => resolve(line)) : "unknown";
}

/**
 * `ask`, answered once and kept, but for an answer of undefined or a throw, a failure: no failure is kept, so the next
 * question asks again, where its own folder or a Git that starts now can answer. Callers that ask at once share one
 * question.
 */
export function keptUnlessFailed<T>(ask: (cwd: string) => Promise<T | undefined>): (cwd: string) => Promise<T | undefined> {
  let asking: Promise<T | undefined> | undefined;
  return async cwd => {
    const question = (asking ??= ask(cwd));
    let answer: T | undefined;
    try {
      answer = await question;
    } finally {
      if (answer === undefined && asking === question) asking = undefined;
    }
    return answer;
  };
}

/**
 * The file a configured path names, as Git finds it: `~/` in Git's home, a relative path against `base`; undefined for
 * a spelling only Git can name the file of ({@link GIT_ONLY_SPELLING}).
 */
export function configuredPath(value: string, base: string): string | undefined {
  if (GIT_ONLY_SPELLING.test(value)) return undefined;
  return value.startsWith("~/") ? join(gitHome(), value.slice(2)) : resolve(base, value);
}

/** The files the includes of `cwd`'s configuration name, and whether one depends on the branch; undefined when unknown. */
interface Included {
  readonly paths: string[];
  readonly byBranch: boolean;
}

async function askIncludes(cwd: string, signal: AbortSignal | undefined): Promise<Included | undefined> {
  const printed = await gitRun(cwd, ["config", "--show-origin", "-z", "--get-regexp", String.raw`^include(if\..+)?\.path$`], signal);
  // Exit 1 with nothing printed: no include is set.
  if (printed?.status === 1 && printed.stdout === "") return { paths: [], byBranch: false };
  return printed?.status === 0 ? includedFiles(cwd, printed.stdout.split("\0")) : undefined;
}

/** `file:<path>`, then `<key>\n<value>`, for each include; undefined for one Git does not name by a file's path. */
function includedFiles(cwd: string, fields: readonly string[]): Included | undefined {
  const paths: string[] = [];
  let byBranch = false;
  for (let at = 0; at + 1 < fields.length; at += 2) {
    const [key = "", value = ""] = fields[at + 1]!.split("\n");
    const path = includedPath(cwd, fields[at]!, value);
    if (path === undefined) return undefined;
    paths.push(path);
    byBranch = byBranch || key.startsWith("includeif.onbranch:");
  }
  return { paths, byBranch };
}

/**
 * An include's file as Git finds it: a relative path in the folder of the file holding the include. Git prints that
 * file's path as it opened it, relative to the checkout's top it ran in (`file:.git/config`) for the repository's own.
 */
function includedPath(cwd: string, origin: string, value: string): string | undefined {
  if (!origin.startsWith("file:")) return undefined;
  return configuredPath(value, resolve(cwd, dirname(origin.slice("file:".length))));
}

/** Per checkout, the inputs under which Git last named a commit, and that commit's length. */
interface Held {
  readonly vouch: string;
  readonly idLength: number;
}

/** The answer a read gives from the files, where Git's last answer still vouches for them; else undefined. */
function keptAnswer(held: Held | undefined, read: FilesRead | undefined): { readonly head: string | undefined } | undefined {
  const head = read?.head;
  if (held === undefined || head === undefined || read?.vouch?.key !== held.vouch) return undefined;
  if (head.kind === "unborn") return { head: undefined };
  return head.commit.length === held.idLength ? { head: head.commit } : undefined;
}

/** What Git's answer vouches for: the read's inputs, when they settled and its files named the commit Git named. */
function vouchedBy(read: FilesRead | undefined, asked: Asked): Held | undefined {
  const named = read?.head?.kind === "commit" ? read.head.commit : undefined;
  if (read?.vouch?.settled !== true || named === undefined || named !== asked.head || !sameLayout(read.layout, asked.layout)) return undefined;
  return { vouch: read.vouch.key, idLength: named.length };
}

/**
 * A reader keeping the contract of this module's comment. One per daemon is enough ({@link readGitHead}): every walk and
 * session of it reads the same files, so a checkout's first read asks Git once for all of them.
 */
export function gitHeadReader(options: GitHeadReaderOptions = {}): GitHeadReader {
  const settleMs = options.settleMs ?? SETTLE_MS;
  const owners = options.owners ?? defaultOwners;
  const packed = packedRefsReader(settleMs);
  const held = new Map<string, Held>();
  // The daemon's readers share one: the global and system files are named once, and each file read once per stamp.
  const configs = options.settleMs === undefined ? sharedConfigs() : configReader(settleMs);

  /** Every input under which Git's answer for `cwd` vouches for its files; undefined when one cannot be read. */
  const vouchOf = async (cwd: string, layout: Layout, signal: AbortSignal | undefined): Promise<Vouch | undefined> => {
    const [owned, objects, refs, config] = await Promise.all([
      owners([cwd, join(cwd, ".git"), layout.gitDir]),
      folderAt(join(layout.commonDir, "objects")),
      folderAt(join(layout.commonDir, "refs")),
      configs(cwd, layout, signal),
    ]);
    if ([owned, objects, refs, config].includes(undefined)) return undefined;
    return { key: [layout.gitDir, layout.commonDir, owned, config!.key].join("\0"), settled: config!.settled };
  };

  const readFiles = async (cwd: string, signal: AbortSignal | undefined): Promise<FilesRead | undefined> => {
    const layout = MOVING_VARIABLES.some(name => process.env[name]) ? undefined : await layoutOf(cwd);
    if (layout === undefined) return undefined;
    const [head, vouch] = await Promise.all([headFromFiles(layout, packed), vouchOf(cwd, layout, signal)]);
    return { layout, head, vouch };
  };

  return async (cwd, readOptions = {}) => {
    const read = await readFiles(cwd, readOptions.signal);
    const kept = keptAnswer(held.get(cwd), read);
    if (kept !== undefined) return kept.head;
    held.delete(cwd);
    const asked = await askHead(cwd, readOptions.signal);
    const vouched = vouchedBy(read, asked);
    if (vouched !== undefined) held.set(cwd, vouched);
    return asked.head;
  };
}

/** The files a checkout's includes name, stamped, and `HEAD`'s text where an `includeIf "onbranch:"` is set, else empty. */
interface IncludedRead {
  readonly files: readonly StampedFile[];
  readonly branch: string;
}

/**
 * What Git named of a checkout's includes, and every input that answer depends on as it was before Git was asked: the
 * repository's, worktree's, global and system files (`base`, paths and stamps), each included file's stamp, and, where
 * an `includeIf "onbranch:"` is set, `HEAD`'s text. An included file can hold includes of its own, and an `onbranch`
 * include is read only on its branch, so each of those moves the files Git reads.
 */
interface KeptIncludes {
  readonly base: string;
  readonly included: Included;
  readonly stamps: readonly string[];
  readonly branch: string;
}

/** `HEAD`'s text in `gitDir`, which an `includeIf "onbranch:"` depends on; undefined when it cannot be read. */
async function headText(gitDir: string): Promise<string | undefined> {
  return await fileText(join(gitDir, "HEAD"));
}

/** Whether `files` hold, in order, the stamps `stamps` holds. */
function sameStamps(files: readonly StampedFile[] | undefined, stamps: readonly string[]): files is readonly StampedFile[] {
  return files !== undefined && files.length === stamps.length && files.every((file, at) => file.stamp === stamps[at]);
}

/**
 * The configuration files Git reads for a checkout, stamped, as one string. The global and system files are named once
 * per process (`git var`), asked again after a failure; the repository's files are read for an include section only when
 * their stamp moved, and the files includes name are asked of Git only when one holds one, again once any input of that
 * answer moved ({@link KeptIncludes}). What is kept by a stamp is kept only once the stamp has settled: a write in the
 * same clock tick can leave a stamp as it was.
 */
function configReader(settleMs: number): (cwd: string, layout: Layout, signal: AbortSignal | undefined) => Promise<Vouch | undefined> {
  const wideNamed = keptUnlessFailed(askWideFiles);
  const sections = new Map<string, { readonly stamp: string; readonly include: boolean }>();
  const includes = new Map<string, KeptIncludes>();

  const wideFiles = async (cwd: string): Promise<string[] | undefined> => {
    const named = await wideNamed(cwd);
    return Array.isArray(named) ? named : undefined;
  };

  /** Whether `file` holds an include section; its text is read again only when its stamp moved or had not settled. */
  const holdsInclude = async ({ path, stamp, settled }: StampedFile): Promise<boolean | undefined> => {
    const known = sections.get(path);
    if (known?.stamp === stamp) return known.include;
    // Git reads a configuration file through a link, as `stampOf` stamps it: a global file a dotfiles tool links in.
    const text = await fileText(path, MAX_CONFIG_BYTES, open);
    if (text === undefined) return undefined;
    const include = text !== "absent" && INCLUDE_SECTION.test(text);
    if (settled) sections.set(path, { stamp, include });
    return include;
  };

  /** The includes kept for `cwd`, where every input of Git's answer is as it was; else undefined. */
  const keptIncludes = async (cwd: string, gitDir: string, base: readonly StampedFile[]): Promise<IncludedRead | undefined> => {
    const known = includes.get(cwd);
    if (known?.base !== filesKey(base)) return undefined;
    const branch = known.included.byBranch ? await headText(gitDir) : "";
    const files = await stampedFiles(known.included.paths, settleMs);
    return branch === known.branch && sameStamps(files, known.stamps) ? { files, branch } : undefined;
  };

  /**
   * Git's includes for `cwd`, kept once every file they depend on has settled; `HEAD` is read first, as Git reads it
   * after, and needed only where an `includeIf "onbranch:"` is set.
   */
  const askedIncludes = async (cwd: string, gitDir: string, base: readonly StampedFile[], signal: AbortSignal | undefined) => {
    const head = await headText(gitDir);
    const asked = await askIncludes(cwd, signal);
    const included = asked?.byBranch === true && head === undefined ? undefined : asked;
    const files = included === undefined ? undefined : await stampedFiles(included.paths, settleMs);
    if (files === undefined) return undefined;
    const branch = included!.byBranch ? head! : "";
    if (allSettled([...base, ...files])) includes.set(cwd, { base: filesKey(base), included: included!, stamps: files.map(file => file.stamp), branch });
    return { files, branch };
  };

  const includedFor = async (cwd: string, gitDir: string, base: readonly StampedFile[], signal: AbortSignal | undefined): Promise<IncludedRead | undefined> => {
    const found = await Promise.all(base.map(holdsInclude));
    if (found.includes(undefined)) return undefined;
    if (!found.includes(true)) return { files: [], branch: "" };
    return (await keptIncludes(cwd, gitDir, base)) ?? (await askedIncludes(cwd, gitDir, base, signal));
  };

  return async (cwd, layout, signal) => {
    const named = await wideFiles(cwd);
    const base = named === undefined ? undefined : await stampedFiles([join(layout.commonDir, "config"), join(layout.gitDir, "config.worktree"), ...named], settleMs);
    const included = base === undefined ? undefined : await includedFor(cwd, layout.gitDir, base, signal);
    if (included === undefined) return undefined;
    const files = [...base!, ...included.files];
    return { key: `${filesKey(files)}\0branch\0${included.branch}`, settled: allSettled(files) };
  };
}

/** Git's own answer: the checkout's Git folders where `cwd` is its top, and the commit `HEAD` names. */
async function askHead(cwd: string, signal: AbortSignal | undefined): Promise<Asked> {
  const printed = await gitRun(cwd, ["rev-parse", "--show-cdup", "--git-dir", "--git-common-dir", "--verify", "--quiet", "HEAD"], signal);
  const lines = printedLines(printed);
  if (printed?.status === 0 && lines.length === 4) return askedOf(cwd, lines);
  if (namesNoCommit(printed, lines)) return { layout: undefined, head: undefined };
  // Any other end is asked as `gitHead` asks it.
  return { layout: undefined, head: await gitHead(cwd, { signal }) };
}

/**
 * Whether Git's end names no commit: Git named the folders and no commit, or found no repository. Git looks for the
 * repository before it answers the first option and, finding none, stops there (128) having printed nothing, where
 * `gitHead`'s `rev-parse` stops too.
 */
function namesNoCommit(printed: GitResult | undefined, lines: readonly string[]): boolean {
  if (printed?.status === 128 && lines.length === 0) return true;
  return printed?.status === 1 && lines.length === 3;
}

/** The lines Git printed, without the empty one after the last newline. */
function printedLines(printed: GitResult | undefined): string[] {
  const lines = printed?.stdout.split(/\r?\n/u) ?? [];
  if (lines.at(-1) === "") lines.pop();
  return lines;
}

/** `rev-parse`'s four lines: the way up to the top (empty at the top), the Git folder, the common folder, the commit. */
function askedOf(cwd: string, [cdup, gitDir = "", commonDir = "", head]: readonly string[]): Asked {
  return { layout: cdup === "" ? { gitDir: resolve(cwd, gitDir), commonDir: resolve(cwd, commonDir) } : undefined, head };
}

/** `runGitAsync`, or undefined where no Git can be started. */
async function gitRun(cwd: string, args: readonly string[], signal?: AbortSignal): Promise<GitResult | undefined> {
  try {
    return await runGitAsync(cwd, args, { signal });
  } catch {
    return undefined;
  }
}

function sameLayout(files: Layout, asked: Layout | undefined): boolean {
  const same = (one: string, other: string): boolean => (process.platform === "win32" ? one.toLowerCase() === other.toLowerCase() : one === other);
  return asked !== undefined && same(files.gitDir, asked.gitDir) && same(files.commonDir, asked.commonDir);
}

type ConfigReader = ReturnType<typeof configReader>;
let shared: ConfigReader | undefined;

function sharedConfigs(): ConfigReader {
  shared ??= configReader(SETTLE_MS);
  return shared;
}

/** A checkout's Git folders and the stamps of every configuration file Git reads for it, as one key. */
export interface CheckoutConfiguration extends Vouch {
  readonly layout: Layout;
}

/**
 * {@link CheckoutConfiguration} of the checkout at `cwd` (its top); undefined where the files cannot name it: a `.git`
 * entry or a configuration file that cannot be read, Git before 2.42, or a variable that moves where Git looks. A
 * reader that keeps an answer Git gave by this key asks Git again once a configuration file changed.
 */
export async function checkoutConfiguration(cwd: string, signal?: AbortSignal): Promise<CheckoutConfiguration | undefined> {
  const layout = MOVING_VARIABLES.some(name => process.env[name]) ? undefined : await layoutOf(cwd);
  const config = layout === undefined ? undefined : await sharedConfigs()(cwd, layout, signal);
  return config === undefined ? undefined : { layout: layout!, ...config };
}

/** The daemon's one reader: the walks and every session's walk share what Git answered and what its files hold. */
export const readGitHead: GitHeadReader = gitHeadReader();
