import { tmpdir } from "node:os";
import { GitUnavailableError, runGit, runGitAsync, sharedRead, type GitReadOptions, type GitResult, type RunGitOptions, type SharedRead } from "../git.js";
import { loadNativeBinding, type NativeBinding, type NativeGitAccess, type NativeGitInstallation } from "./native.js";

const NULL_DEVICE = process.platform === "win32" ? "nul" : "/dev/null";

/**
 * How the asks run: outside any repository, and with the variables that move the system file removed, so Git names the
 * file it was built to read; the engine applies `GIT_CONFIG_SYSTEM` and `GIT_CONFIG_NOSYSTEM` itself. The global file
 * is the null device, as the listing is read for the system and installation files only.
 */
const ASK: RunGitOptions = { env: { GIT_CONFIG_SYSTEM: undefined, GIT_CONFIG_NOSYSTEM: undefined, GIT_CONFIG_GLOBAL: NULL_DEVICE } };
const OUTSIDE_ANY_REPOSITORY = [`--git-dir=${NULL_DEVICE}`];
const SYSTEM_ASK: readonly string[] = [...OUTSIDE_ANY_REPOSITORY, "var", "GIT_CONFIG_SYSTEM"];
const LISTING_ASK: readonly string[] = [...OUTSIDE_ANY_REPOSITORY, "config", "--list", "--show-origin", "--show-scope", "-z", "--name-only", "--no-includes"];
/** How long a process takes the fallback after an ask that Git did not answer (a timeout, a failed start). */
export const GIT_ASK_AGAIN_MS = 60_000;
/**
 * How long a reader waits after an open that outran its time limit before it opens again: each open on a disk that
 * answers only slowly holds the JS thread for the whole limit, so a walk must not pay that at every path.
 */
export const GIT_OPEN_AGAIN_MS = 60_000;

/** What Rerail's Git reported, and whether it was an answer worth keeping for the life of the process. */
export interface GitInstallationReport {
  installation: NativeGitInstallation;
  /** False where an ask did not finish (it timed out or could not start): ask again later. */
  settled: boolean;
  /** Why no system file was named, where none was. */
  problem?: string;
}

/**
 * What Rerail's Git reads outside any repository, in two asks through Rerail's Git starter: `git var GIT_CONFIG_SYSTEM`,
 * which a Git before 2.42 refuses, and its config listing with each entry's scope and file. Where neither names a system
 * file (no Git, a Git before 2.26 that rejects `--show-scope`, a failed ask) it reports none, and the engine then reads
 * gix's `/etc/gitconfig` on Linux and macOS, and no system file on Windows, where finding one would start a Git.
 */
export function readGitInstallation(run: typeof runGit = runGit): GitInstallationReport {
  const folder = tmpdir();
  let system: GitResult;
  try {
    system = run(folder, SYSTEM_ASK, ASK);
  } catch (error) {
    return withoutGit(error);
  }
  return installationReport(system, run(folder, LISTING_ASK, ASK));
}

/**
 * {@link readGitInstallation} with its two asks run as Git processes no thread waits on, one after the other; undefined
 * once `options.signal` is aborted, whose asks did not run to their end. Rejects where Git is missing.
 */
async function readGitInstallationWithoutBlocking(options: GitReadOptions): Promise<GitInstallationReport | undefined> {
  const folder = tmpdir();
  const system = await runGitAsync(folder, SYSTEM_ASK, { ...ASK, ...options });
  const listing = await runGitAsync(folder, LISTING_ASK, { ...ASK, ...options });
  return options.signal?.aborted === true ? undefined : installationReport(system, listing);
}

/** The report where no Git is installed: settled, with none named, and why; any other failure is thrown again. */
function withoutGit(error: unknown): GitInstallationReport {
  if (!(error instanceof GitUnavailableError)) throw error;
  return { installation: {}, settled: true, problem: error.message };
}

/** What Git's answers to the two asks report ({@link readGitInstallation}). */
function installationReport(system: GitResult, listing: GitResult): GitInstallationReport {
  // An absent answer is left out, not null: the addon takes a missing field as none and refuses a null.
  const systemConfig = system.status === 0 ? system.stdout.trim() : "";
  const installation = {
    ...(systemConfig === "" ? {} : { systemConfig }),
    ...(listing.status === 0 ? { configOrigins: listing.stdout } : {}),
  };
  const settled = [system, listing].every(result => result.status !== null && result.error === undefined);
  const problem = systemConfig === "" ? listingProblem(listing) : undefined;
  return { installation, settled, ...(problem === undefined ? {} : { problem }) };
}

/** Why the listing named no file: undefined where it answered. */
function listingProblem(listing: GitResult): string | undefined {
  return listing.status === 0 ? undefined : (listing.error?.message ?? listing.stderr.trim()) || `exit ${listing.status}`;
}

/** One owner of the engine's Git reads and the answer it was made from, kept as {@link gitAccessOwner} says. */
interface GitAccessKeeper {
  /** The owner while its answer stands: undefined before the first answer, and once an ask that did not finish is due again. */
  standing(): NativeGitAccess | undefined;
  /** The owner for `report`: the one kept while the answer is the same, else a new one. */
  take(report: GitInstallationReport): NativeGitAccess;
}

function gitAccessKeeper(binding: NativeBinding, now: () => number): GitAccessKeeper {
  let owner: NativeGitAccess | undefined;
  let answer = "";
  let askAgainAt = Number.POSITIVE_INFINITY;
  const tell = fallbackNotice();
  return {
    standing: () => (owner !== undefined && now() < askAgainAt ? owner : undefined),
    take: ({ installation, settled, problem }) => {
      const asked = JSON.stringify([installation.systemConfig ?? null, installation.configOrigins ?? null]);
      if (owner === undefined || asked !== answer) {
        owner = new binding.GitAccess(installation);
        answer = asked;
      }
      askAgainAt = settled ? Number.POSITIVE_INFINITY : now() + GIT_ASK_AGAIN_MS;
      tell(problem);
      return owner;
    },
  };
}

/** Says once on stderr why Git named no system file, and what the engine's Git reads take instead. */
function fallbackNotice(): (problem: string | undefined) => void {
  let reported = false;
  return problem => {
    if (problem === undefined || reported) return;
    reported = true;
    const fallback = process.platform === "win32" ? "no system config file" : "/etc/gitconfig";
    process.stderr.write(`rerail: Git did not say which system config file it reads (${problem}); the engine's Git reads take ${fallback}\n`);
  };
}

/**
 * The owner of a process's in-process Git reads, made from {@link readGitInstallation} at the first need. An answer Git
 * gave is kept for the life of the process; after an ask that did not finish, the owner takes the fallback and asks
 * again once {@link GIT_ASK_AGAIN_MS} has passed. A new owner is made only when the answer changed, since readers open
 * their matchers again under a new one. Where Git named no system file, stderr says so once.
 */
export function gitAccessOwner(binding: NativeBinding, run: typeof runGit = runGit, now: () => number = Date.now): () => NativeGitAccess {
  const keeper = gitAccessKeeper(binding, now);
  return () => keeper.standing() ?? keeper.take(readGitInstallation(run));
}

let processOwner: GitAccessKeeper | undefined;

/**
 * The process's one owner of the engine's in-process Git reads (the ignore matcher, the workspace's Git discovery), made
 * at the first need ({@link gitAccessOwner}): every read in the process then takes the config files Rerail's Git reads,
 * and the engine starts no Git of its own to find them.
 */
export function nativeGitAccess(binding: NativeBinding = loadNativeBinding()): NativeGitAccess {
  processOwner ??= gitAccessKeeper(binding, Date.now);
  return processOwner.standing() ?? processOwner.take(readGitInstallation());
}

/**
 * The process's owner ({@link nativeGitAccess}) while its answer stands, made by no call here: it loads no addon and
 * starts no Git. Undefined before an owner is made, and once an ask that did not finish is due again.
 */
export function standingGitAccess(): NativeGitAccess | undefined {
  return processOwner?.standing();
}

/** The ask of Git's installation under way without blocking: callers meanwhile share its two Git processes ({@link sharedRead}). */
const learning = new Map<"installation", SharedRead<void>>();

/**
 * Makes the process's owner ({@link nativeGitAccess}) unless its answer stands, from asks run as Git processes no thread
 * waits on, so {@link standingGitAccess} gives it after. Callers meanwhile share one ask; where another caller's signal
 * stopped it, a caller whose own signal is live asks again, and a caller whose own signal is aborted returns without
 * waiting for it. It never fails: where no engine loads, Git is missing or `options.signal` is aborted, it makes nothing.
 */
export async function learnGitAccess(options: GitReadOptions = {}): Promise<void> {
  if (standingGitAccess() !== undefined) return;
  await sharedRead(learning, "installation", options, async () => await learnedGitAccess(options).catch(() => undefined));
}

/**
 * Makes the owner from asks run without blocking; nothing where `options.signal` stopped them, or where an owner stands
 * already, which a blocking ask may have made while a stopped ask ran and before this one began.
 */
async function learnedGitAccess(options: GitReadOptions): Promise<void> {
  if (standingGitAccess() !== undefined) return;
  const binding = loadNativeBinding();
  const report = await readGitInstallationWithoutBlocking(options);
  if (report === undefined) return;
  processOwner ??= gitAccessKeeper(binding, Date.now);
  // A blocking ask that answered meanwhile stands: its owner stays, and the readers that opened under it.
  if (processOwner.standing() === undefined) processOwner.take(report);
}

/**
 * Whether `error`, from opening an engine Git read, is an open that outran its time limit, or one refused because an
 * earlier open of the same root has not ended: a later open may answer, unlike a repository Git refuses.
 */
export function gitReadTimedOut(error: unknown): boolean {
  return error instanceof Error && "code" in error && error.code === "GitReadTimedOut";
}
