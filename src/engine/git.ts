import { spawn, spawnSync } from "node:child_process";
import { existsSync } from "node:fs";
import { homedir, userInfo } from "node:os";
import { posix, win32 } from "node:path";

/**
 * Where a system Git is looked for, in order. PATH is deliberately not consulted: Rerail runs Git from hooks in
 * whatever environment the coding agent started in, inside repositories it does not trust, and a PATH entry a
 * repository controls (a relative entry, `node_modules/.bin`, direnv's `PATH_add`) could put its own `git` first;
 * the same reason the calls below switch off fsmonitor and hooks. The fixed system locations come first, then the
 * per-user installs whose path only the person's account decides: Git for Windows installed for one user and scoop's
 * Git, Linuxbrew's, and a Nix profile's.
 */
export function gitCandidates(
  platform: NodeJS.Platform = process.platform,
  home: string = accountHome(),
  env: NodeJS.ProcessEnv = process.env,
): string[] {
  if (platform === "win32") {
    const localAppData = env.LOCALAPPDATA || win32.join(home, "AppData", "Local");
    return [
      String.raw`C:\Program Files\Git\cmd\git.exe`,
      String.raw`C:\Program Files (x86)\Git\cmd\git.exe`,
      win32.join(localAppData, "Programs", "Git", "cmd", "git.exe"),
      win32.join(home, "scoop", "apps", "git", "current", "cmd", "git.exe"),
    ];
  }
  return [
    "/usr/bin/git",
    "/usr/local/bin/git",
    "/opt/homebrew/bin/git",
    "/home/linuxbrew/.linuxbrew/bin/git",
    "/run/current-system/sw/bin/git",
    posix.join(home, ".nix-profile", "bin", "git"),
  ];
}

/**
 * The account's own home, not `HOME`: the install's health check runs Rerail with a scratch `HOME`, and it must find
 * the same Git as the person's sessions do. An account without a user database entry (a container's arbitrary uid)
 * falls back to `HOME`.
 */
function accountHome(): string {
  try {
    return userInfo().homedir;
  } catch {
    return homedir();
  }
}

export class GitUnavailableError extends Error {
  /** Every path Rerail looked for Git at. */
  readonly looked: readonly string[];

  constructor(looked: readonly string[]) {
    super(`Git is not installed where Rerail looks for it: ${looked.join(", ")}`);
    this.name = "GitUnavailableError";
    this.looked = looked;
  }
}

let probed: { path: string | undefined; looked: readonly string[] } | undefined;

/** The system Git, probed once per process; throws {@link GitUnavailableError} when none is installed. */
export function gitExecutable(): string {
  if (probed === undefined) {
    const looked = gitCandidates();
    probed = { path: looked.find(existsSync), looked };
  }
  if (probed.path === undefined) throw new GitUnavailableError(probed.looked);
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
 * the clone does not hold is absent.
 */
export async function readGitBlobs(
  cwd: string,
  revisions: readonly string[],
  options: { maxBlobBytes: number; timeout?: number },
): Promise<Map<string, Buffer>> {
  const asked = revisions.filter(revision => !/[\r\n]/u.test(revision));
  const blobs = new Map<string, Buffer>();
  if (asked.length === 0) return blobs;
  let executable: string;
  try {
    executable = gitExecutable();
  } catch {
    return blobs;
  }
  const { promise, resolve: settle } = Promise.withResolvers<void>();
  const child = spawn(executable, [...SAFE_CONFIG, "-C", cwd, "cat-file", "--batch"], {
    env: { ...process.env, GIT_OPTIONAL_LOCKS: "0", GIT_NO_LAZY_FETCH: "1", GIT_TERMINAL_PROMPT: "0" },
    stdio: ["pipe", "pipe", "ignore"],
    timeout: options.timeout ?? DEFAULT_TIMEOUT_MS,
  });
  let next = 0;
  let header: Buffer[] = [];
  let body: BatchBody | undefined;
  child.stdout.on("data", (chunk: Buffer) => {
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
      const kept = fields[1] === "blob" && size <= options.maxBlobBytes ? [] : undefined;
      body = { revision, size, kept, remaining: size + 1 };
    }
  });
  child.on("error", () => settle());
  child.on("close", () => settle());
  // Git exits before reading every line when it cannot start in `cwd`; what it answered before that is kept.
  child.stdin.on("error", () => undefined);
  child.stdin.end(asked.map(revision => `${revision}\n`).join(""));
  await promise;
  return blobs;
}
