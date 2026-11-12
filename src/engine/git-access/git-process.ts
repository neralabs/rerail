import type { ChildProcess } from "node:child_process";
import { statSync } from "node:fs";
import { homedir } from "node:os";
import { dirname, join, resolve, win32 } from "node:path";
import { endProcessTree, execFile, spawn, spawnSync } from "../child-process.js";
import { gitExecutable } from "./find-git.js";

export interface GitResult {
  /** Null when Git could not be started or was killed. */
  status: number | null;
  stdout: string;
  stderr: string;
  /** Set when Git could not be started, timed out or overflowed `maxBuffer`. */
  error?: Error;
}

export interface RunGitOptions {
  maxBuffer?: number;
  timeout?: number;
}

/** A repository's own config can't run programs through Rerail's git calls: no fsmonitor, no hooks. */
const SAFE_CONFIG: readonly string[] = ["-c", "core.fsmonitor=false", "-c", `core.hooksPath=${process.platform === "win32" ? "NUL" : "/dev/null"}`];
const DEFAULT_TIMEOUT_MS = 10_000;

/**
 * The environment of every Git process Rerail starts: no optional locks, and no lazy fetch of a partial clone's missing
 * objects. Git honours `GIT_NO_LAZY_FETCH` from 2.45.0 and in the 2024-04-19 security releases 2.39.4, 2.40.2, 2.41.1,
 * 2.42.2, 2.43.4 and 2.44.1; an older Git ignores it and may fetch from the remote.
 */
function gitEnvironment(): NodeJS.ProcessEnv {
  return { ...process.env, GIT_OPTIONAL_LOCKS: "0", GIT_NO_LAZY_FETCH: "1" };
}

/**
 * `git -C <cwd> <args>` with fsmonitor and hooks disabled, in {@link gitEnvironment} (no optional locks, no lazy fetch
 * of a partial clone's missing objects), and a 10 s default timeout; throws {@link GitUnavailableError} when Git is
 * missing, never on a non-zero exit.
 */
export function runGit(cwd: string, args: readonly string[], options: RunGitOptions = {}): GitResult {
  const result = spawnSync(gitExecutable(), [...SAFE_CONFIG, "-C", cwd, ...args], {
    encoding: "utf8",
    timeout: DEFAULT_TIMEOUT_MS,
    ...options,
    env: gitEnvironment(),
  });
  return { status: result.status, stdout: result.stdout ?? "", stderr: result.stderr ?? "", ...(result.error === undefined ? {} : { error: result.error }) };
}

/** Trimmed stdout of a successful `git -C <cwd> <args>`, or undefined when it fails. */
export function gitText(cwd: string, args: readonly string[]): string | undefined {
  const result = runGit(cwd, args);
  return result.status === 0 ? result.stdout.trim() : undefined;
}

/**
 * How a Git read that does not block can be stopped. Every such read settles only once the Git it started has exited:
 * on Windows a running process holds the folder it runs in, so a read that answered before its Git exited would leave
 * the checkout impossible to remove or rename for a moment.
 */
export interface GitReadOptions {
  /**
   * Once aborted, the read ends its Git with every process that Git started, as its timeout does ({@link settleWhenDone}),
   * or starts none, and answers as it does past its timeout.
   */
  signal?: AbortSignal | undefined;
}

/** What a blob read keeps and how long its Git may run. */
export interface BlobLimits {
  /** A blob larger than this is left out, never read into memory. */
  readonly maxBlobBytes: number;
  /** `DEFAULT_TIMEOUT_MS` by default. */
  readonly timeout?: number | undefined;
}

/** Where Git looks for the global configuration files: this process's environment and platform by default. */
export interface GitPlace {
  readonly env?: NodeJS.ProcessEnv;
  readonly platform?: NodeJS.Platform;
}

/** The Git to start a read with; undefined without Git, or once `signal` is aborted. */
function readerGit(signal: AbortSignal | undefined): string | undefined {
  if (signal?.aborted === true) return undefined;
  try {
    return gitExecutable();
  } catch {
    return undefined;
  }
}

/** Resolves once `child` has exited, or at once when it never started. */
async function exited(child: ChildProcess): Promise<void> {
  if (child.exitCode !== null || child.signalCode !== null) return;
  const { promise, resolve: settle } = Promise.withResolvers<void>();
  child.once("exit", () => settle());
  child.once("error", () => {
    if (child.pid === undefined) settle();
  });
  await promise;
}

/**
 * Trimmed stdout of a successful Git command run under `runGit`'s safe configuration without blocking; else undefined.
 * Its timeout and `signal` end it as they end a batch read ({@link settleWhenDone}); it settles once that Git has exited
 * ({@link GitReadOptions}).
 */
async function gitOutput(cwd: string, args: readonly string[], signal: AbortSignal | undefined): Promise<string | undefined> {
  return (await gitStdout(cwd, args, signal))?.trim();
}

/** Stdout, as Git wrote it, of a successful Git command run as `gitOutput` runs one; undefined past `maxBuffer` bytes. */
async function gitStdout(cwd: string, args: readonly string[], signal: AbortSignal | undefined, maxBuffer = 1024 * 1024): Promise<string | undefined> {
  const executable = readerGit(signal);
  if (executable === undefined) return undefined;
  const { promise, resolve: settle } = Promise.withResolvers<string | undefined>();
  const child = execFile(
    executable,
    [...SAFE_CONFIG, "-C", cwd, ...args],
    { encoding: "utf8", maxBuffer, env: { ...process.env, GIT_OPTIONAL_LOCKS: "0", GIT_NO_LAZY_FETCH: "1", GIT_TERMINAL_PROMPT: "0" } },
    (error, stdout) => settle(error === null && signal?.aborted !== true ? stdout : undefined),
  );
  // execFile's own `close` listener comes first and answers with the output; this one answers a Git that was ended and
  // whose output stays open.
  settleWhenDone(child, () => settle(undefined), { timeoutMs: DEFAULT_TIMEOUT_MS, signal });
  const [output] = await Promise.all([promise, exited(child)]);
  return output;
}

/** The commit `HEAD` names in `cwd`; undefined outside a repository, or before its first commit. */
export async function gitHead(cwd: string, options: GitReadOptions = {}): Promise<string | undefined> {
  return await gitOutput(cwd, ["rev-parse", "--verify", "--quiet", "HEAD"], options.signal);
}

/**
 * The exclude files of the Git checkout at `cwd` that no diff shows, absolute: its `info/exclude` (the common Git
 * folder's, which every worktree shares) and the file `core.excludesFile` names, else Git's default, `git/ignore` in
 * `XDG_CONFIG_HOME` or `~/.config`. Empty outside a Git checkout, or without Git.
 */
export async function gitExcludeFiles(cwd: string, options: GitReadOptions = {}): Promise<string[]> {
  // Git prints the path relative to `cwd` unless it is elsewhere; `--path-format` would need Git 2.31.
  const exclude = await gitOutput(cwd, ["rev-parse", "--git-path", "info/exclude"], options.signal);
  if (exclude === undefined || exclude === "") return [];
  const configured = await gitOutput(cwd, ["config", "--path", "--get", "core.excludesFile"], options.signal);
  return [resolve(cwd, exclude), resolve(cwd, configured || defaultExcludesFile())];
}

/**
 * The home folder Git reads `~/.gitconfig` and `~/.config` from: `HOME` when set. Without it Git for Windows takes
 * `HOMEDRIVE` and `HOMEPATH` when that folder exists, which on a machine with a home share is not the profile folder
 * `homedir()` returns, and then the profile folder (`USERPROFILE`). Elsewhere Git takes the account's home.
 */
export function gitHome(
  env: NodeJS.ProcessEnv = process.env,
  platform: NodeJS.Platform = process.platform,
  isFolder: (path: string) => boolean = folderExists,
): string {
  if (env.HOME !== undefined && env.HOME !== "") return env.HOME;
  if (platform === "win32" && env.HOMEDRIVE && env.HOMEPATH) {
    const share = win32.join(env.HOMEDRIVE, env.HOMEPATH);
    if (isFolder(share)) return share;
  }
  return homedir();
}

function folderExists(path: string): boolean {
  try {
    return statSync(path).isDirectory();
  } catch {
    return false;
  }
}

/**
 * The file Git reads as `core.excludesFile` when that names none: `git/ignore` in `XDG_CONFIG_HOME`, else in `.config`
 * in Git's home ({@link gitHome}).
 */
function defaultExcludesFile(env: NodeJS.ProcessEnv = process.env, platform: NodeJS.Platform = process.platform): string {
  const xdg = env.XDG_CONFIG_HOME;
  return join(xdg === undefined || xdg === "" ? join(gitHome(env, platform), ".config") : xdg, "git", "ignore");
}

/**
 * The configuration files that can name the excludes file of the Git checkout at `cwd`, absolute: its repository's
 * `config` and its worktree's `config.worktree`, and the global file, `GIT_CONFIG_GLOBAL` when set, else `.gitconfig`
 * in Git's home ({@link gitHome}, from `env` on `platform`, this process's by default) and `git/config` in
 * `XDG_CONFIG_HOME` or that home's `.config`. The system file, which only an administrator writes, and files
 * `include.path` pulls in are left out. Empty outside a Git checkout, or without Git.
 */
export async function gitConfigFiles(
  cwd: string,
  options: GitReadOptions & { env?: NodeJS.ProcessEnv; platform?: NodeJS.Platform } = {},
): Promise<string[]> {
  const { env = process.env, platform = process.platform } = options;
  const local = await gitOutput(cwd, ["rev-parse", "--git-path", "config", "--git-path", "config.worktree"], options.signal);
  if (local === undefined || local === "") return [];
  const global = env.GIT_CONFIG_GLOBAL;
  const globals = global === undefined || global === ""
    ? [join(gitHome(env, platform), ".gitconfig"), join(dirname(defaultExcludesFile(env, platform)), "config")]
    : [global];
  return [...local.split("\n").map(path => resolve(cwd, path)), ...globals.map(path => resolve(cwd, path))];
}

/** How far back {@link headMovedOnlyByCommits} looks: a window longer than this is no agent's one command. */
const MAX_REFLOG_ENTRIES = 256;
/** The reflog subject of `git commit` in all its forms (`commit (amend): …`, `commit (merge): …`). */
const COMMIT_SUBJECT = /^commit(?: \([^)]*\))?: /u;

/**
 * Whether everything that moved `HEAD` in `cwd` since it named `from` was a commit, which leaves the working tree as it
 * found it, by `HEAD`'s reflog. Anything else there (a checkout, pull, merge, rebase, reset, cherry-pick) wrote files, and
 * so did a move the reflog cannot show (`from` is not in it). With no `from`, `HEAD`'s newest move is all there is to go by.
 */
export async function headMovedOnlyByCommits(cwd: string, from: string | undefined, options: GitReadOptions = {}): Promise<boolean> {
  const reflog = await gitOutput(cwd, ["reflog", "show", "-n", String(MAX_REFLOG_ENTRIES), "--format=%H %gs", "HEAD"], options.signal);
  if (reflog === undefined) return false;
  const entries = reflog.split("\n");
  const moves = from === undefined ? 1 : entries.findIndex(entry => entry.startsWith(`${from} `));
  return moves > 0 && entries.slice(0, moves).every(entry => COMMIT_SUBJECT.test(entry.slice(entry.indexOf(" ") + 1)));
}

/** The distinct commits `cwd`'s branches and remote-tracking branches point at; empty without Git or outside a repository. */
export async function gitBranchTips(cwd: string, options: GitReadOptions = {}): Promise<string[]> {
  const tips = await gitOutput(cwd, ["for-each-ref", "--format=%(objectname)", "refs/heads", "refs/remotes"], options.signal);
  return tips === undefined || tips === "" ? [] : [...new Set(tips.split("\n"))];
}

/** The commits reachable from `to` but not from `from` (`git rev-list from..to`); empty when Git cannot say. */
export async function gitCommitsBetween(cwd: string, from: string, to: string, options: GitReadOptions = {}): Promise<string[]> {
  const commits = await gitOutput(cwd, ["rev-list", `${from}..${to}`], options.signal);
  return commits === undefined || commits === "" ? [] : commits.split("\n");
}

/** A Git boolean that is true. */
const GIT_TRUE = /^(?:true|yes|on|1)$/iu;

/**
 * The rule by which a checkout writes a file's LF line breaks as CRLF, as Git's convert.c applies it (line-endings.ts
 * writes the bytes). `auto` (`core.autocrlf=true`, `text=auto`) writes a blob that already holds a CR, or that Git takes
 * for binary data, as it is. `text` (a `text` attribute, or `eol=crlf` without `text=auto`) writes every LF that follows
 * no CR as CRLF, whatever else the blob holds.
 */
export type CrlfRule = "auto" | "text";

/** The line-ending attributes Git reads for one path, each as `git check-attr` prints it: `set`, `unset`, `unspecified` or a value. */
interface LineEndingAttributes {
  readonly text: string;
  /** The older name of `text`, which Git reads where `text` declares nothing. */
  readonly crlf: string;
  readonly eol: string;
}

/** `core.autocrlf` and `core.eol`, lowercased; undefined where unset. */
interface LineEndingConfig {
  readonly autocrlf: string | undefined;
  readonly coreEol: string | undefined;
}

/** What a `text` or `crlf` attribute declares a file: text, binary, text stored and written with LF, or text if Git guesses so. */
function declaredText(value: string): "text" | "binary" | "input" | "auto" | undefined {
  if (value === "set") return "text";
  if (value === "unset") return "binary";
  return value === "input" || value === "auto" ? value : undefined;
}

/**
 * Whether a text file with no `eol` attribute gets CRLF: under `core.autocrlf=true`, never under `input`, else as
 * `core.eol` says, whose default (`native`) is CRLF on Windows alone.
 */
function textGetsCrlf({ autocrlf, coreEol }: LineEndingConfig, platform: NodeJS.Platform): boolean {
  if (autocrlf === "input") return false;
  if (autocrlf !== undefined && GIT_TRUE.test(autocrlf)) return true;
  return coreEol === "crlf" || (coreEol !== "lf" && platform === "win32");
}

/**
 * The rule by which a checkout writes a file's line breaks as CRLF, or undefined where it writes them as stored, as Git's
 * `convert_attrs` and `output_eol` decide it. The `text` attribute, else `crlf`, declares the file; a binary one is
 * never converted. `eol=crlf` then makes a `text=auto` file `auto` and any other `text`, and `eol=lf` writes LF. With no
 * `eol`, an `input` file gets LF, a `text` or `auto` file gets CRLF where a text file does ({@link textGetsCrlf}), and a
 * file that declares nothing is `auto` under `core.autocrlf=true` alone.
 */
function crlfRule(attributes: LineEndingAttributes, config: LineEndingConfig, platform: NodeJS.Platform): CrlfRule | undefined {
  const declared = declaredText(attributes.text) ?? declaredText(attributes.crlf);
  if (declared === "binary") return undefined;
  if (attributes.eol === "crlf") return declared === "auto" ? "auto" : "text";
  if (attributes.eol === "lf" || declared === "input") return undefined;
  if (declared === undefined) return config.autocrlf !== undefined && GIT_TRUE.test(config.autocrlf) ? "auto" : undefined;
  return textGetsCrlf(config, platform) ? declared : undefined;
}

/**
 * The `text`, `crlf` and `eol` attributes of each of `paths` (relative to `cwd`), by one `git check-attr --stdin`; empty
 * without Git, outside a repository or past the timeout.
 */
async function lineEndingAttributes(cwd: string, paths: readonly string[], signal: AbortSignal | undefined): Promise<Map<string, LineEndingAttributes>> {
  const attributes = new Map<string, { text: string; crlf: string; eol: string }>();
  const chunks: Buffer[] = [];
  await streamGit(cwd, ["check-attr", "-z", "--stdin", "text", "crlf", "eol"], {
    input: paths.map(path => `${path}\0`).join(""), onData: chunk => chunks.push(chunk), signal,
  });
  // `<path> NUL <attribute> NUL <value> NUL`, once per path and attribute.
  const fields = Buffer.concat(chunks).toString("utf8").split("\0");
  for (let at = 0; at + 2 < fields.length; at += 3) {
    const [path, name, value] = [fields[at]!, fields[at + 1]!, fields[at + 2]!];
    const entry = attributes.get(path) ?? { text: "unspecified", crlf: "unspecified", eol: "unspecified" };
    if (name === "text" || name === "crlf" || name === "eol") entry[name] = value;
    attributes.set(path, entry);
  }
  return attributes;
}

/**
 * The paths among `paths` (relative to `cwd`, `/`-separated) that a checkout of `cwd` writes with CRLF line breaks, each
 * with the rule it writes them by ({@link crlfRule}), read from the repository's attributes and configuration. Empty
 * without Git, outside a repository or past the timeout, so a file with CRLF line breaks is then never taken for Git's
 * writing.
 */
export async function gitCrlfPaths(
  cwd: string,
  paths: readonly string[],
  options: GitReadOptions & { platform?: NodeJS.Platform } = {},
): Promise<Map<string, CrlfRule>> {
  const { platform = process.platform, signal } = options;
  const asked = paths.filter(path => !path.includes("\0"));
  const rules = new Map<string, CrlfRule>();
  if (asked.length === 0) return rules;
  const [autocrlf, coreEol, attributes] = await Promise.all([
    gitOutput(cwd, ["config", "--get", "core.autocrlf"], signal),
    gitOutput(cwd, ["config", "--get", "core.eol"], signal),
    lineEndingAttributes(cwd, asked, signal),
  ]);
  const config = { autocrlf: autocrlf?.toLowerCase(), coreEol: coreEol?.toLowerCase() };
  for (const path of asked) {
    const attribute = attributes.get(path);
    const rule = attribute === undefined ? undefined : crlfRule(attribute, config, platform);
    if (rule !== undefined) rules.set(path, rule);
  }
  return rules;
}

/** The most bytes of a listing `gitModifiedOrUntrackedPaths` and `unmergedEntries` read: past it, the first lists none and the second cannot say. */
const MAX_LISTED_PATH_BYTES = 64 * 1024 * 1024;

/**
 * The files of the checkout at `cwd` whose text its index may not hold: modified since staged (or deleted), and untracked
 * but not ignored; relative to `cwd`. Empty without Git, outside a repository, past the timeout, or past
 * `MAX_LISTED_PATH_BYTES` of paths.
 */
export async function gitModifiedOrUntrackedPaths(cwd: string, options: GitReadOptions = {}): Promise<string[]> {
  const listed = await gitStdout(cwd, ["ls-files", "-z", "--modified", "--others", "--exclude-standard"], options.signal, MAX_LISTED_PATH_BYTES);
  return listed === undefined ? [] : listed.split("\0").filter(path => path !== "");
}

/**
 * The object id of each of `revisions` (as for {@link batchBlobs}) that names a blob, by one `git cat-file
 * --batch-check`: no blob is read. A revision Git does not resolve to a blob is absent, and so is every revision
 * without Git, outside a repository, or past the timeout; one containing a newline cannot be asked for and is skipped.
 */
export async function readGitBlobIds(cwd: string, revisions: readonly string[], options: GitReadOptions = {}): Promise<Map<string, string>> {
  const asked = revisions.filter(revision => !/[\r\n]/u.test(revision));
  const ids = new Map<string, string>();
  if (asked.length === 0) return ids;
  const chunks: Buffer[] = [];
  await streamGit(cwd, ["cat-file", "--batch-check=%(objectname) %(objecttype)"], {
    input: asked.map(revision => `${revision}\n`).join(""), onData: chunk => chunks.push(chunk), signal: options.signal,
  });
  // One line per revision, in order: `<oid> <type>`, or `<revision> missing` (or `ambiguous`), which repeats the
  // revision and so may hold any text, spaces included; only a whole line of an object id and `blob` names one.
  const lines = Buffer.concat(chunks).toString("utf8").split("\n");
  asked.forEach((revision, index) => {
    const id = /^([0-9a-f]{40}|[0-9a-f]{64}) blob$/u.exec(lines[index] ?? "")?.[1];
    if (id !== undefined) ids.set(revision, id);
  });
  return ids;
}

/** How long a read waits for Git's output to close once the Git it ended has exited. */
const EXIT_DRAIN_MS = 500;

/** What ends a Git read before its Git is done: its time limit, and the signal that stops the reads it belongs to. */
interface ReadLimits {
  readonly timeoutMs: number;
  readonly signal: AbortSignal | undefined;
}

/**
 * Calls `settle` once `child` fails to start, its output closes, or {@link EXIT_DRAIN_MS} after it exits once ended,
 * whichever comes first. `timeoutMs`, or `signal` once aborted, ends the whole process tree (`endProcessTree`), never
 * the child alone: on Windows the process Rerail starts can still be a launcher that runs the real Git as its own child
 * (a scoop shim, or Git for Windows' `cmd\git.exe` without its own git.exe beside it; {@link gitBehindLauncher}), and
 * killing only the launcher left the real Git running in the repository. Node's own `signal` option kills the child
 * alone, so it is not used. The real Git holds the output too, so the output closes once the whole tree has exited;
 * should it stay open anyway, the answer does not wait for it. A Git that exits by itself is read to the end of its
 * output; one a signal from elsewhere ended is waited for as one this ended.
 */
function settleWhenDone(child: ChildProcess, settle: () => void, { timeoutMs, signal }: ReadLimits): void {
  let ended = false;
  const forget = (): void => {
    clearTimeout(timer);
    signal?.removeEventListener("abort", end);
  };
  const drain = (): void => {
    forget();
    setTimeout(() => {
      child.stdout?.destroy();
      settle();
    }, EXIT_DRAIN_MS).unref();
  };
  const end = (): void => {
    ended = true;
    // A child that has exited has no tree left to reach, and its pid may name another process by now.
    if (child.exitCode === null && child.signalCode === null) endProcessTree(child);
    else drain();
  };
  const timer = setTimeout(end, timeoutMs);
  signal?.addEventListener("abort", end, { once: true });
  const done = (): void => {
    forget();
    settle();
  };
  child.on("error", done);
  child.on("close", done);
  child.on("exit", (_code, killedBy) => {
    if (ended || killedBy !== null) drain();
  });
}

/** What {@link streamGit} writes to Git and does with what Git prints. */
interface GitStream extends GitReadOptions {
  readonly input: string;
  readonly onData: (chunk: Buffer) => void;
  /** `DEFAULT_TIMEOUT_MS` by default. */
  readonly timeout?: number;
}

/**
 * Runs `git -C <cwd> <args>` under `runGit`'s safe configuration without blocking, writes `stream.input` to it and hands
 * each chunk it prints to `stream.onData`. Settles once Git is done ({@link settleWhenDone}) and has exited
 * ({@link GitReadOptions}); without Git, or once `stream.signal` is aborted, it starts nothing.
 */
async function streamGit(cwd: string, args: readonly string[], stream: GitStream): Promise<void> {
  const executable = readerGit(stream.signal);
  if (executable === undefined) return;
  const { promise, resolve: settle } = Promise.withResolvers<void>();
  const child = spawn(executable, [...SAFE_CONFIG, "-C", cwd, ...args], {
    env: { ...gitEnvironment(), GIT_TERMINAL_PROMPT: "0" },
    stdio: ["pipe", "pipe", "ignore"],
  });
  child.stdout.on("data", stream.onData);
  settleWhenDone(child, settle, { timeoutMs: stream.timeout ?? DEFAULT_TIMEOUT_MS, signal: stream.signal });
  // Git exits before reading every line when it cannot start in `cwd`; what it answered before that is kept.
  child.stdin.on("error", () => undefined);
  child.stdin.end(stream.input);
  await Promise.all([promise, exited(child)]);
}

/**
 * The index's unmerged entries under `cwd`, by path relative to it. Each value holds the path's modes, blobs and stages,
 * so a later conflict on the same path reads differently. Undefined when Git cannot say: outside a repository, past the
 * timeout, past `MAX_LISTED_PATH_BYTES` of entries, or once `signal` is aborted.
 */
export async function unmergedEntries(cwd: string, signal: AbortSignal | undefined): Promise<ReadonlyMap<string, string> | undefined> {
  const listed = await gitStdout(cwd, ["ls-files", "--unmerged", "-z"], signal, MAX_LISTED_PATH_BYTES);
  if (listed === undefined) return undefined;
  const entries = new Map<string, string>();
  for (const record of listed.split("\0")) {
    const tab = record.indexOf("\t");
    if (tab < 0) continue;
    const path = record.slice(tab + 1);
    entries.set(path, `${entries.get(path) ?? ""}${record.slice(0, tab)}\n`);
  }
  return entries;
}

/** The file `cwd`'s checkout keeps its index in (a linked worktree's is under the main `.git`); undefined when Git cannot say. */
export async function gitIndexPath(cwd: string, signal: AbortSignal | undefined): Promise<string | undefined> {
  const path = await gitOutput(cwd, ["rev-parse", "--git-path", "index"], signal);
  return path === undefined ? undefined : resolve(cwd, path);
}

/** One object body of `git cat-file --batch` output being read: kept, or skipped as it streams past. */
interface BatchBody {
  readonly revision: string;
  readonly size: number;
  readonly kept: Buffer[] | undefined;
  /** Bytes of the body and its trailing newline still to come. */
  remaining: number;
}

/**
 * The contents of `revisions` (revision expressions such as `:./src/a.ts` or `HEAD:./src/a.ts`, relative to `cwd`),
 * read by one `git cat-file --batch` under `runGit`'s safe configuration, parsed as the output streams. A revision Git
 * does not resolve to a blob, or whose blob is larger than `maxBlobBytes`, is absent; so is every revision without Git,
 * outside a repository, or past the timeout (blobs read before it are kept). A revision containing a newline cannot be
 * asked for and is skipped. Git never fetches a partial clone's missing blob or prompts for credentials here: a blob
 * the clone does not hold is absent. It ends once `options.signal` is aborted ({@link GitReadOptions}).
 */
export async function batchBlobs(cwd: string, revisions: readonly string[], options: BlobLimits & GitReadOptions): Promise<Map<string, Buffer>> {
  const asked = revisions.filter(revision => !/[\r\n]/u.test(revision));
  const blobs = new Map<string, Buffer>();
  if (asked.length === 0) return blobs;
  await streamGit(cwd, ["cat-file", "--batch"], {
    input: asked.map(revision => `${revision}\n`).join(""),
    onData: batchReader(asked, options.maxBlobBytes, blobs),
    ...(options.timeout === undefined ? {} : { timeout: options.timeout }),
    signal: options.signal,
  });
  return blobs;
}

/**
 * Reads `git cat-file --batch` output for `asked`, chunk by chunk, into `blobs`: each blob of at most `maxBlobBytes`,
 * under the revision that asked for it.
 */
function batchReader(asked: readonly string[], maxBlobBytes: number, blobs: Map<string, Buffer>): (chunk: Buffer) => void {
  let next = 0;
  let header: Buffer[] = [];
  let body: BatchBody | undefined;
  return (chunk: Buffer) => {
    let offset = 0;
    while (offset < chunk.length) {
      if (body !== undefined) {
        const take = Math.min(body.remaining, chunk.length - offset);
        body.kept?.push(chunk.subarray(offset, offset + take));
        body.remaining -= take;
        offset += take;
        if (body.remaining === 0) {
          if (body.kept !== undefined) blobs.set(body.revision, Buffer.concat(body.kept).subarray(0, body.size));
          body = undefined;
        }
        continue;
      }
      const end = chunk.indexOf(0x0a, offset);
      if (end === -1) {
        header.push(chunk.subarray(offset));
        break;
      }
      header.push(chunk.subarray(offset, end));
      offset = end + 1;
      // `<oid> <type> <size>` then the object and a newline; anything else (`<revision> missing`) has no body.
      const fields = Buffer.concat(header).toString("utf8").split(" ");
      header = [];
      const revision = asked[next++];
      if (revision === undefined || fields.length !== 3 || !/^[0-9a-f]+$/u.test(fields[0]!)) continue;
      const size = Number(fields[2]);
      const kept = fields[1] === "blob" && size <= maxBlobBytes ? [] : undefined;
      body = { revision, size, kept, remaining: size + 1 };
    }
  };
}
