import { tmpdir } from "node:os";
import { GitUnavailableError, runGit, type GitResult, type RunGitOptions } from "../git.js";
import { loadNativeBinding, type NativeBinding, type NativeGitAccess, type NativeGitInstallation } from "./native.js";

/** The first Git with `git var GIT_CONFIG_SYSTEM`. */
const GIT_VAR_SYSTEM_CONFIG: readonly [number, number] = [2, 42];

const NULL_DEVICE = process.platform === "win32" ? "nul" : "/dev/null";

/**
 * How the asks run: outside any repository, and with the variables that move the system file removed, so Git names the
 * file it was built to read; the engine applies `GIT_CONFIG_SYSTEM` and `GIT_CONFIG_NOSYSTEM` itself. The global file
 * is the null device, as the listing is read for the system and installation files only.
 */
const ASK: RunGitOptions = { env: { GIT_CONFIG_SYSTEM: undefined, GIT_CONFIG_NOSYSTEM: undefined, GIT_CONFIG_GLOBAL: NULL_DEVICE } };
const OUTSIDE_ANY_REPOSITORY = [`--git-dir=${NULL_DEVICE}`];

/** Whether `version` (`git version 2.53.0`, `git version 2.56.0.windows.1`) is `minimum` or later. */
function atLeast(version: string, minimum: readonly [number, number]): boolean {
  const found = /(\d+)\.(\d+)/u.exec(version);
  if (found === null) return false;
  const [major, minor] = [Number(found[1]), Number(found[2])];
  return major > minimum[0] || (major === minimum[0] && minor >= minimum[1]);
}

/**
 * What Rerail's Git reads outside any repository, in three asks through Rerail's Git starter (two on a Git before 2.42):
 * its version, `git var GIT_CONFIG_SYSTEM` and its config listing with each entry's scope and file. Without Git it
 * reports nothing, and the engine then reads no system file on Windows, where finding one would start a Git.
 */
export function readGitInstallation(run: typeof runGit = runGit): NativeGitInstallation {
  const folder = tmpdir();
  let version: GitResult;
  try {
    version = run(folder, ["version"], ASK);
  } catch (error) {
    if (error instanceof GitUnavailableError) return { version: "" };
    throw error;
  }
  const text = version.status === 0 ? version.stdout.trim() : "";
  const system = atLeast(text, GIT_VAR_SYSTEM_CONFIG) ? run(folder, [...OUTSIDE_ANY_REPOSITORY, "var", "GIT_CONFIG_SYSTEM"], ASK) : undefined;
  const listing = run(folder, [...OUTSIDE_ANY_REPOSITORY, "config", "--list", "--show-origin", "--show-scope", "-z", "--name-only", "--no-includes"], ASK);
  return {
    version: text,
    systemConfig: system?.status === 0 ? system.stdout.trim() || null : null,
    configOrigins: listing.status === 0 ? listing.stdout : null,
  };
}

let owner: NativeGitAccess | undefined;

/**
 * The process's one owner of the engine's in-process Git reads (the ignore matcher, the workspace's Git discovery), made
 * at the first need from {@link readGitInstallation}: every read in the process then takes the config files Rerail's
 * Git reads, and the engine starts no Git of its own to find them.
 */
export function nativeGitAccess(binding: NativeBinding = loadNativeBinding()): NativeGitAccess {
  owner ??= new binding.GitAccess(readGitInstallation());
  return owner;
}
