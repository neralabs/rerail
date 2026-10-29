import { spawn } from "node:child_process";
import { accessSync, constants, existsSync } from "node:fs";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { delimiter, dirname, isAbsolute, join } from "node:path";

export interface CommandResult {
  stdout: string;
  stderr: string;
}

export interface CommandOptions {
  cwd?: string;
  env?: NodeJS.ProcessEnv;
}

export interface NpmOptions extends CommandOptions {
  /** The Node the result must serve: npm runs under it, so `npm rebuild` compiles for that Node's ABI. */
  node?: string;
}

export function cleanCommandError(text: string): string {
  return text.replace(/(:_(?:authToken|password)=)\S+/gu, "$1[REDACTED]").trim();
}

export async function runCommand(command: string, args: readonly string[], options: CommandOptions = {}): Promise<CommandResult> {
  const { promise, resolve: resolveCommand, reject } = Promise.withResolvers<CommandResult>();
  const child = spawn(command, [...args], {
    stdio: ["ignore", "pipe", "pipe"],
    ...(options.cwd === undefined ? {} : { cwd: options.cwd }),
    ...(options.env === undefined ? {} : { env: options.env }),
  });
  let stdout = "";
  let stderr = "";
  child.stdout.setEncoding("utf8");
  child.stderr.setEncoding("utf8");
  child.stdout.on("data", (chunk) => {
    stdout += chunk;
  });
  child.stderr.on("data", (chunk) => {
    stderr += chunk;
  });
  child.once("error", reject);
  child.once("exit", (code, signal) => {
    if (code === 0) {
      resolveCommand({ stdout, stderr });
      return;
    }
    const detail = cleanCommandError(stderr || stdout);
    const detailSuffix = detail.length === 0 ? "" : `: ${detail}`;
    reject(new Error(`${command} failed (${signal ?? code ?? "unknown"})${detailSuffix}`));
  });
  return await promise;
}

function pathKey(env: NodeJS.ProcessEnv): string {
  return Object.keys(env).find(name => name.toUpperCase() === "PATH") ?? "PATH";
}

/**
 * The absolute path PATH would resolve `command` to, or undefined when nothing on PATH is runnable by that
 * name. An absolute command is returned as is. A compiled-Bun host only knows Node as the bare name `node`;
 * the probe, the rebuild and the daemon spawn must still agree on one binary, so the name is resolved once.
 */
export function resolveExecutable(
  command: string,
  env: NodeJS.ProcessEnv = process.env,
  platform: NodeJS.Platform = process.platform,
): string | undefined {
  if (isAbsolute(command)) return command;
  const windows = platform === "win32";
  const separator = windows ? ";" : delimiter;
  const directories = (env[pathKey(env)] ?? "").split(separator).filter(directory => directory.length > 0);
  const extensions = windows
    ? (env.PATHEXT ?? env.Pathext ?? ".EXE;.CMD;.BAT;.COM").split(";").filter(extension => extension.length > 0)
    : [""];
  for (const directory of directories) {
    for (const extension of extensions) {
      const candidate = join(directory, `${command}${extension}`);
      if (windows ? existsSync(candidate) : runnable(candidate)) return candidate;
    }
  }
  return undefined;
}

function runnable(path: string): boolean {
  try {
    accessSync(path, constants.X_OK);
    return true;
  } catch {
    return false;
  }
}

/** Windows has no `npm` executable a spawned Node can rely on; run the CLI script shipped next to `node`. */
export function windowsNpmCliPath(node: string = process.execPath): string {
  const inherited = process.env.npm_execpath;
  if (inherited !== undefined && /(?:^|[\\/])npm-cli\.js$/iu.test(inherited)) return inherited;
  return join(dirname(resolveExecutable(node) ?? process.execPath), "node_modules", "npm", "bin", "npm-cli.js");
}

/**
 * The environment in which `npm`, a `#!/usr/bin/env node` script, runs under `node` rather than whichever Node
 * PATH reaches first (nvm and Homebrew disagree routinely). A name PATH cannot resolve leaves the environment alone.
 */
export function environmentForNode(node: string, env: NodeJS.ProcessEnv = process.env): NodeJS.ProcessEnv {
  const executable = resolveExecutable(node, env);
  if (executable === undefined) return env;
  const key = pathKey(env);
  const current = env[key];
  const bin = dirname(executable);
  return { ...env, [key]: current === undefined || current.length === 0 ? bin : `${bin}${delimiter}${current}` };
}

export type NpmRunner = (args: readonly string[], options?: NpmOptions) => Promise<CommandResult>;

const NPM_UNAUTHORIZED = /\bE401\b|\b401 Unauthorized\b/u;

/**
 * npm's own error code when it could not reach a registry at all; a failed install script reports its exit code instead.
 * npm 10 and later write `npm error code X`, earlier ones `npm ERR! code X`, and on Windows the line ends in `\r\n`.
 */
const NPM_UNREACHABLE = /\bnpm (?:error|ERR!) code (ECONNREFUSED|ENOTFOUND|EAI_AGAIN|ETIMEDOUT|ECONNRESET|ENETUNREACH|EHOSTUNREACH|ERR_SOCKET_TIMEOUT)\r?$/mu;
/** A registry that accepts the connection and never answers: npm's code is then only `FETCH_ERROR`. */
const NPM_TIMEOUT = /\bnpm (?:error|ERR!) network timeout at: /u;
/** The URL npm's fetch error names: `request to <url> failed, reason: …` or `network timeout at: <url>`. */
const NPM_FETCH_URL = /(?:request to|network timeout at:) (https?:\/\/[^\s,]+)/u;

/**
 * npm waits 10 s, then 60 s, before its two retries of a request, and against a registry that is down it does that for
 * every package an install needs, one after another: many minutes without a word. One quick retry still rides out a
 * dropped connection.
 */
const FETCH_RETRY_OPTIONS = ["--fetch-retries=1", "--fetch-retry-mintimeout=2000", "--fetch-retry-maxtimeout=10000"];

/** npm's 401 detail is a wall of text; the fix is always the same, so lead with it and keep the one line that proves it. */
function unauthorizedError(error: unknown): Error | undefined {
  if (!(error instanceof Error)) return undefined;
  const line = error.message.split("\n").find(candidate => NPM_UNAUTHORIZED.test(candidate));
  if (line === undefined) return undefined;
  return new Error(`npm registry credentials are missing or expired. Run \`npm login\` to refresh them. (${line.trim()})`, { cause: error });
}

/** Why npm could not reach its registry, in one line with what to check; undefined when npm failed for another reason. */
export function unreachableRegistryReason(message: string): string | undefined {
  const code = NPM_UNREACHABLE.exec(message)?.[1] ?? (NPM_TIMEOUT.test(message) ? "timed out" : undefined);
  if (code === undefined) return undefined;
  const url = NPM_FETCH_URL.exec(message)?.[1];
  const registry = url === undefined ? undefined : URL.parse(url)?.origin;
  return `could not reach the npm registry (${registry === undefined ? code : `${registry}, ${code}`}): check your network, proxy, or registry in ~/.npmrc`;
}

function unreachableError(error: unknown): Error | undefined {
  if (!(error instanceof Error)) return undefined;
  const reason = unreachableRegistryReason(error.message);
  return reason === undefined ? undefined : new Error(reason, { cause: error });
}

async function spawnNpm(args: readonly string[], options: NpmOptions): Promise<CommandResult> {
  const { node: requested, ...command } = options;
  const node = requested === undefined ? undefined : resolveExecutable(requested, command.env);
  // Options first: a caller's `--` ends them.
  const npmArgs = [...FETCH_RETRY_OPTIONS, ...args];
  if (process.platform === "win32") {
    return await runCommand(node ?? process.execPath, [windowsNpmCliPath(node ?? process.execPath), ...npmArgs], command);
  }
  return await runCommand("npm", npmArgs, node === undefined ? command : { ...command, env: environmentForNode(node, command.env) });
}

export const runNpm: NpmRunner = async (args, options = {}) => {
  try {
    return await spawnNpm(args, options);
  } catch (error) {
    throw unauthorizedError(error) ?? unreachableError(error) ?? error;
  }
};

/**
 * Fails, as `runNpm` words it, when npm cannot fetch `spec`'s metadata from its registry now. Against a registry that
 * is down npm answers from its cache, however stale, once its retries are spent, so an install waits them out for every
 * package in turn; one request with an empty cache finds out in seconds. npm set to work offline is left to it.
 */
export async function checkRegistry(spec: string): Promise<void> {
  const cache = await mkdtemp(join(tmpdir(), "rerail-npm-"));
  try {
    await runNpm(["view", spec, "version", "--cache", cache, "--fetch-timeout=30000"]);
  } catch (error) {
    if (!(error instanceof Error) || !/\bENOTCACHED\b/u.test(error.message)) throw error;
  } finally {
    await rm(cache, { recursive: true, force: true });
  }
}
