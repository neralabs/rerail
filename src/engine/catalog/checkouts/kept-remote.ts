/**
 * A clone's remote as Git named it, kept while the configuration Git read it from reads the same: the facts it rests on
 * ({@link remoteSource}) are the files and variables Git reads a remote from, read here before Git runs. Where one of
 * them cannot be read or named, no answer is kept and Git is asked each time. A caller that keeps a remote of its own
 * keeps it by what the repository's own config files hold ({@link configFilesText}).
 */
import { existsSync, readdirSync, statSync } from "node:fs";
import { isAbsolute, join, resolve } from "node:path";
import { fileText, placementInputs } from "./git-placement.js";
import { learnGitAccess, standingGitAccess } from "../../native/git-access.js";
import { gitGlobalConfigFiles, type GitReadOptions } from "../../git.js";

export interface RemoteAddress {
  name: string;
  url: string;
}

/** Where a checkout's remotes are read from, the facts Git's answer rests on, and the clone's common Git folder. */
export interface RemoteSource {
  readonly key: string;
  readonly facts: string;
  readonly common: string;
}

/** A remote Git named, "none" included, and the facts it rested on. */
interface KeptRemote {
  readonly facts: string;
  readonly remote: RemoteAddress | undefined;
  /** The clone's common Git folder, real: the answer leaves once the folder it is kept by is gone ({@link forgetGoneRemotes}). */
  readonly common: string;
}

/**
 * Each clone's remote, by where its remotes are read from: the common Git folder, which every worktree of the clone
 * shares, or a worktree's own `config.worktree`, by that file's path, so the main checkout's, which is in the common
 * folder, keeps an answer apart from the one its other worktrees share.
 */
const keptRemotes = new Map<string, KeptRemote>();

/** The answer kept for `source` while the facts it rested on read the same; undefined where Git is to be asked. */
export function keptRemote(source: RemoteSource | undefined): { readonly remote: RemoteAddress | undefined } | undefined {
  const kept = source === undefined ? undefined : keptRemotes.get(source.key);
  return kept !== undefined && kept.facts === source?.facts ? { remote: kept.remote } : undefined;
}

/** Keeps `remote`, "none" included, as Git's answer for `source`: only an answer Git gave to its end is kept. */
export function keepRemote(source: RemoteSource, remote: RemoteAddress | undefined): void {
  keptRemotes.set(source.key, { facts: source.facts, remote, common: source.common });
}

/** Drops the answer kept for each place of the clone whose common Git folder is `common` once that place is gone. */
export function forgetGoneRemotes(common: string): void {
  for (const [key, answer] of keptRemotes) if (answer.common === common && !existsSync(key)) keptRemotes.delete(key);
}

/** How many clones' remotes are kept: for tests, which count what a removed worktree leaves behind. */
export function keptRemoteCount(): number {
  return keptRemotes.size;
}

/**
 * {@link remoteSource}; where it names none for a checkout placed from its files that the user owns, the engine owner of
 * its Git reads, which names the system files, is made first unless it stands, as Git processes no thread waits on
 * ({@link learnGitAccess}), and the source read again.
 */
export async function remoteSourceWithoutBlocking(root: string, options: GitReadOptions): Promise<RemoteSource | undefined> {
  const source = remoteSource(root);
  if (source !== undefined || placementInputs(resolve(root))?.owned !== true) return source;
  await learnGitAccess(options);
  return remoteSource(root);
}

/**
 * Where `root`'s remotes come from, read now ({@link remoteFacts}). Undefined, so Git is asked each time, where Git's
 * placement of `root` is not read from its files ({@link placementInputs}), where another account owns it (Git reads
 * the repository's config only while a `safe.directory` entry trusts it, the system file's too), where a fact cannot
 * be read or named, or where no engine owner of its Git reads stands ({@link systemConfigFiles}).
 */
export function remoteSource(root: string): RemoteSource | undefined {
  const inputs = placementInputs(resolve(root));
  if (inputs === undefined || !inputs.owned) return undefined;
  const own = configText(join(inputs.gitFolder, "config.worktree"));
  const facts = remoteFacts(own, inputs.linkedCommon ?? inputs.gitFolder);
  return facts === undefined ? undefined : { key: own === NO_FILE ? inputs.realCommon : join(inputs.gitFolder, "config.worktree"), facts, common: inputs.realCommon };
}

/**
 * Every fact Git reads a clone's remote from but the worktree's `config.worktree` (`own`, its text): the configuration
 * this process hands Git ({@link environmentConfig}), the text of the repository's `config` and of each file outside
 * it ({@link wideConfigFiles}), where an `insteadOf` also rewrites an address, and the common folder's older remote
 * files ({@link olderRemoteFiles}). Undefined where one cannot be read, includes another file, or cannot be named.
 */
function remoteFacts(own: string | undefined, common: string): string | undefined {
  const environment = environmentConfig();
  const wide = wideConfigFiles();
  if (wide === undefined || INCLUDE.test(environment)) return undefined;
  const files = [join(common, "config"), ...wide];
  const texts = [own, ...files.map(configText), olderRemoteFiles(common)];
  return texts.some(text => text === undefined) ? undefined : [environment, ...files, ...texts].join("\0");
}

/** The variables through which a process hands Git configuration of its own: `-c`'s, and `GIT_CONFIG_COUNT`'s. */
const ENVIRONMENT_CONFIG = /^GIT_CONFIG_(?:PARAMETERS|COUNT|KEY_\d+|VALUE_\d+)$/iu;

/** The configuration this process hands every Git it starts, read now: each such variable and its value, one per line. */
function environmentConfig(): string {
  return Object.entries(process.env)
    .filter(([name]) => ENVIRONMENT_CONFIG.test(name))
    .map(([name, value]) => `${name}=${value ?? ""}`)
    .join("\n");
}

/**
 * The config files outside the repository that Git reads, by path: the system files ({@link systemConfigFiles}) and the
 * person's global ones ({@link gitGlobalConfigFiles}). Undefined where one cannot be named for every Git this process
 * starts: Git named no system file, or a path is relative, which each Git opens from its own folder.
 */
function wideConfigFiles(): string[] | undefined {
  const system = systemConfigFiles();
  if (system === undefined) return undefined;
  const files = [...system, ...gitGlobalConfigFiles()];
  return files.every(file => isAbsolute(file)) ? files : undefined;
}

/** The values Git's boolean reading takes as true, read case-blind; any other value of `GIT_CONFIG_NOSYSTEM` reads the system files too. */
const GIT_TRUE = /^(?:1|true|yes|on)$/iu;

/**
 * The system files Git reads: none under `GIT_CONFIG_NOSYSTEM`; else `GIT_CONFIG_SYSTEM`, or where it is unset the file
 * Rerail's Git was built to read, as the engine's owner of its Git reads names it, with the file Apple Git reads beside
 * it. Undefined where Git named no system file, or no owner stands ({@link standingGitAccess}): this read loads no
 * engine and asks Git nothing to make one.
 */
function systemConfigFiles(): string[] | undefined {
  if (GIT_TRUE.test(process.env["GIT_CONFIG_NOSYSTEM"] ?? "")) return [];
  const access = standingGitAccess();
  if (access === undefined) return undefined;
  const named = process.env["GIT_CONFIG_SYSTEM"] ?? access.systemConfig;
  return named === null ? undefined : [named, access.installationConfig].filter(file => file !== null);
}

/**
 * {@link NO_FILE} while the common Git folder holds neither of the older forms Git still reads a remote from when its
 * configuration gives it no address, `remotes/<name>` and `branches/<name>`; undefined where it holds one or cannot be
 * read, so Git is asked each time.
 */
function olderRemoteFiles(common: string): string | undefined {
  return ["remotes", "branches"].every(folder => entryCount(join(common, folder)) === 0) ? NO_FILE : undefined;
}

/** How many entries the folder at `path` holds: 0 where nothing is there, undefined where it cannot be read. */
function entryCount(path: string): number | undefined {
  try {
    return readdirSync(path).length;
  } catch (error) {
    return typeof error === "object" && error !== null && "code" in error && error.code === "ENOENT" ? 0 : undefined;
  }
}

const NO_FILE = "\0none";
/**
 * Text that may include another file, which this source does not follow: a section opens anywhere on a line
 * (`[core] [include] path = ...`), and the environment names the key itself (`include.path`).
 */
const INCLUDE = /include/iu;
/**
 * A config file's text, read now as Git reads it ({@link fileText}); {@link NO_FILE} when nothing is there; undefined
 * when it is no regular file, cannot be read or includes another.
 */
function configText(path: string): string | undefined {
  const text = textAsGitReads(path);
  return text === undefined || INCLUDE.test(text) ? undefined : text;
}

/** A file's text, read now as Git reads it ({@link fileText}); {@link NO_FILE} when nothing is there; undefined when it is no regular file or cannot be read. */
function textAsGitReads(path: string): string | undefined {
  const read = fileText(path);
  return read === "none" ? NO_FILE : read?.text;
}

/**
 * The repository's own files a checkout's remotes are read from: the common Git folder's `config`, which every
 * worktree of the clone shares, and the worktree's own `config.worktree` (`gitFolder`, the common one for a main
 * checkout, a submodule or a clone whose Git folder is kept apart), which Git reads too where
 * `extensions.worktreeConfig` is on; turning that on writes the first. Without `gitFolder`, the first alone.
 */
export function remoteConfigFiles(common: string, gitFolder: string | undefined): readonly string[] {
  return gitFolder === undefined ? [join(common, "config")] : [join(common, "config"), join(gitFolder, "config.worktree")];
}

/**
 * What `files` hold, as one string: each one's text, read now as Git reads it ({@link textAsGitReads}), so a file that
 * appears or goes changes it, and where its text cannot be read, what the file system says of it ({@link unreadStamp}).
 * An `include` is not followed, so a file it names changed alone leaves this as it was: a caller that must see that
 * asks {@link remoteSource}.
 */
export function configFilesText(files: readonly string[]): string {
  return files.map(file => textAsGitReads(file) ?? unreadStamp(file)).join("\0");
}

/**
 * What the file system says of a file whose text cannot be read (no regular file, too large, no permission), without
 * reading it: its size, modification time in nanoseconds and file id, through a link at its name as the read goes, in
 * `bigint`, which holds an NTFS file id whole; the error's code where it cannot be looked at. A rewrite in place that
 * keeps the size and the time changes none of them.
 */
function unreadStamp(path: string): string {
  try {
    const stats = statSync(path, { bigint: true });
    return `\0stat:${stats.size}:${stats.mtimeNs}:${stats.ino}`;
  } catch (error) {
    return `\0error:${(error as NodeJS.ErrnoException).code ?? "unknown"}`;
  }
}
