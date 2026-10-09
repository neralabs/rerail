import { spawnSync } from "node:child_process";
import { existsSync } from "node:fs";

/** Where a system Git is looked for; PATH is deliberately not consulted. */
const GIT_CANDIDATES: readonly string[] = process.platform === "win32"
  ? [String.raw`C:\Program Files\Git\cmd\git.exe`, String.raw`C:\Program Files (x86)\Git\cmd\git.exe`]
  : ["/usr/bin/git", "/usr/local/bin/git", "/opt/homebrew/bin/git"];

export class GitUnavailableError extends Error {
  constructor() {
    super("Git is not installed in a supported system location");
    this.name = "GitUnavailableError";
  }
}

let probed: { path: string | undefined } | undefined;

/** The system Git, probed once per process; throws {@link GitUnavailableError} when none is installed. */
export function gitExecutable(): string {
  probed ??= { path: GIT_CANDIDATES.find(existsSync) };
  if (probed.path === undefined) throw new GitUnavailableError();
  return probed.path;
}

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
 * `git -C <cwd> <args>` with fsmonitor and hooks disabled, no optional locks, and a 10 s default timeout; throws
 * {@link GitUnavailableError} when Git is missing, never on a non-zero exit.
 */
export function runGit(cwd: string, args: readonly string[], options: RunGitOptions = {}): GitResult {
  const result = spawnSync(gitExecutable(), [...SAFE_CONFIG, "-C", cwd, ...args], {
    encoding: "utf8",
    timeout: DEFAULT_TIMEOUT_MS,
    ...options,
    env: { ...process.env, GIT_OPTIONAL_LOCKS: "0" },
  });
  return { status: result.status, stdout: result.stdout ?? "", stderr: result.stderr ?? "", ...(result.error === undefined ? {} : { error: result.error }) };
}

/** Trimmed stdout of a successful `git -C <cwd> <args>`, or undefined when it fails. */
export function gitText(cwd: string, args: readonly string[]): string | undefined {
  const result = runGit(cwd, args);
  return result.status === 0 ? result.stdout.trim() : undefined;
}
