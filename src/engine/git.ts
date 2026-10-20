import { spawn, spawnSync } from "node:child_process";
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

/**
 * The contents of `revisions` (revision expressions such as `:./src/a.ts` or `HEAD:./src/a.ts`, relative to `cwd`),
 * read by one `git cat-file --batch` under `runGit`'s safe configuration. A revision Git does not resolve to a blob is
 * absent; without Git, outside a repository, past `maxBytes` of output or past the timeout the map is empty. A
 * revision containing a newline cannot be asked for and is skipped.
 */
export async function readGitBlobs(
  cwd: string,
  revisions: readonly string[],
  options: { maxBytes: number; timeout?: number },
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
  const { promise, resolve: settle } = Promise.withResolvers<Buffer | undefined>();
  const child = spawn(executable, [...SAFE_CONFIG, "-C", cwd, "cat-file", "--batch"], {
    env: { ...process.env, GIT_OPTIONAL_LOCKS: "0" },
    stdio: ["pipe", "pipe", "ignore"],
    timeout: options.timeout ?? DEFAULT_TIMEOUT_MS,
  });
  const chunks: Buffer[] = [];
  let bytes = 0;
  let overflowed = false;
  child.stdout.on("data", (chunk: Buffer) => {
    bytes += chunk.length;
    if (bytes > options.maxBytes) {
      overflowed = true;
      child.kill();
      return;
    }
    chunks.push(chunk);
  });
  child.on("error", () => settle(undefined));
  child.on("close", status => settle(status === 0 && !overflowed ? Buffer.concat(chunks) : undefined));
  // Git exits before reading every line when it cannot start in `cwd`; the close status says so.
  child.stdin.on("error", () => undefined);
  child.stdin.end(asked.map(revision => `${revision}\n`).join(""));
  const output = await promise;
  if (output === undefined) return blobs;
  let offset = 0;
  for (const revision of asked) {
    const end = output.indexOf(0x0a, offset);
    if (end === -1) break;
    const header = output.toString("utf8", offset, end).split(" ");
    offset = end + 1;
    // `<oid> <type> <size>` then the object and a newline; anything else (`<revision> missing`) has no body.
    if (header.length !== 3 || !/^[0-9a-f]+$/u.test(header[0]!)) continue;
    const size = Number(header[2]);
    if (header[1] === "blob") blobs.set(revision, output.subarray(offset, offset + size));
    offset += size + 1;
  }
  return blobs;
}
