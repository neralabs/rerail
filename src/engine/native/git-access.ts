import { tmpdir } from "node:os";
import { GitUnavailableError, runGit, type GitResult, type RunGitOptions } from "../git.js";
import { loadNativeBinding, type NativeBinding, type NativeGitAccess, type NativeGitInstallation } from "./native.js";

/** A Git run whose stdout is text: {@link runGit}'s, or a test's stand-in. */
type GitRun = (cwd: string, args: readonly string[], options?: RunGitOptions) => GitResult;

const NULL_DEVICE = process.platform === "win32" ? "nul" : "/dev/null";

/**
 * How the asks run: outside any repository, and with the variables that move the system file removed, so Git names the
 * file it was built to read; the engine applies `GIT_CONFIG_SYSTEM` and `GIT_CONFIG_NOSYSTEM` itself. The global file
 * is the null device, as the listing is read for the system and installation files only.
 */
const ASK: RunGitOptions = { env: { GIT_CONFIG_SYSTEM: undefined, GIT_CONFIG_NOSYSTEM: undefined, GIT_CONFIG_GLOBAL: NULL_DEVICE } };
const OUTSIDE_ANY_REPOSITORY = [`--git-dir=${NULL_DEVICE}`];
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
export function readGitInstallation(run: GitRun = runGit): GitInstallationReport {
  const folder = tmpdir();
  let system: GitResult;
  try {
    system = run(folder, [...OUTSIDE_ANY_REPOSITORY, "var", "GIT_CONFIG_SYSTEM"], ASK);
  } catch (error) {
    if (!(error instanceof GitUnavailableError)) throw error;
    return { installation: {}, settled: true, problem: error.message };
  }
  const listing = run(folder, [...OUTSIDE_ANY_REPOSITORY, "config", "--list", "--show-origin", "--show-scope", "-z", "--name-only", "--no-includes"], ASK);
  // An absent answer is left out, not null: the addon takes a missing field as none and refuses a null.
  const systemConfig = system.status === 0 ? system.stdout.trim() : "";
  const installation = {
    ...(systemConfig === "" ? {} : { systemConfig }),
    ...(listing.status === 0 ? { configOrigins: listing.stdout } : {}),
  };
  const settled = [system, listing].every(result => result.status !== null && result.error === undefined);
  const problem = systemConfig === "" && listing.status !== 0 ? (listing.error?.message ?? listing.stderr.trim()) || `exit ${listing.status}` : undefined;
  return { installation, settled, ...(problem === undefined ? {} : { problem }) };
}

/**
 * The owner of a process's in-process Git reads, made from {@link readGitInstallation} at the first need. An answer Git
 * gave is kept for the life of the process; after an ask that did not finish, the owner takes the fallback and asks
 * again once {@link GIT_ASK_AGAIN_MS} has passed. A new owner is made only when the answer changed, since readers open
 * their matchers again under a new one. Where Git named no system file, stderr says so once.
 */
export function gitAccessOwner(binding: NativeBinding, run: GitRun = runGit, now: () => number = Date.now): () => NativeGitAccess {
  let owner: NativeGitAccess | undefined;
  let answer = "";
  let askAgainAt = Number.POSITIVE_INFINITY;
  let reported = false;
  return () => {
    if (owner !== undefined && now() < askAgainAt) return owner;
    const { installation, settled, problem } = readGitInstallation(run);
    const asked = JSON.stringify([installation.systemConfig ?? null, installation.configOrigins ?? null]);
    if (owner === undefined || asked !== answer) {
      owner = new binding.GitAccess(installation);
      answer = asked;
    }
    askAgainAt = settled ? Number.POSITIVE_INFINITY : now() + GIT_ASK_AGAIN_MS;
    if (problem !== undefined && !reported) {
      reported = true;
      const fallback = process.platform === "win32" ? "no system config file" : "/etc/gitconfig";
      process.stderr.write(`rerail: Git did not say which system config file it reads (${problem}); the engine's Git reads take ${fallback}\n`);
    }
    return owner;
  };
}

let processOwner: (() => NativeGitAccess) | undefined;

/**
 * The process's one owner of the engine's in-process Git reads (the ignore matcher, the workspace's Git discovery), made
 * at the first need ({@link gitAccessOwner}): every read in the process then takes the config files Rerail's Git reads,
 * and the engine starts no Git of its own to find them.
 */
export function nativeGitAccess(binding: NativeBinding = loadNativeBinding()): NativeGitAccess {
  processOwner ??= gitAccessOwner(binding);
  return processOwner();
}

/**
 * Whether `error`, from opening an engine Git read, is an open that outran its time limit, or one refused because an
 * earlier open of the same root has not ended: a later open may answer, unlike a repository Git refuses.
 */
export function gitReadTimedOut(error: unknown): boolean {
  return error instanceof Error && "code" in error && error.code === "GitReadTimedOut";
}
