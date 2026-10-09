import { spawn } from "node:child_process";
import { accessSync, constants, existsSync } from "node:fs";
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

/** npm's 401 detail is a wall of text; the fix is always the same, so lead with it and keep the one line that proves it. */
function unauthorizedError(error: unknown): Error | undefined {
  if (!(error instanceof Error)) return undefined;
  const line = error.message.split("\n").find(candidate => NPM_UNAUTHORIZED.test(candidate));
  if (line === undefined) return undefined;
  return new Error(`npm registry credentials are missing or expired. Run \`npm login\` to refresh them. (${line.trim()})`, { cause: error });
}

async function spawnNpm(args: readonly string[], options: NpmOptions): Promise<CommandResult> {
  const { node: requested, ...command } = options;
  const node = requested === undefined ? undefined : resolveExecutable(requested, command.env);
  if (process.platform === "win32") {
    return await runCommand(node ?? process.execPath, [windowsNpmCliPath(node ?? process.execPath), ...args], command);
  }
  return await runCommand("npm", args, node === undefined ? command : { ...command, env: environmentForNode(node, command.env) });
}

export const runNpm: NpmRunner = async (args, options = {}) => {
  try {
    return await spawnNpm(args, options);
  } catch (error) {
    throw unauthorizedError(error) ?? error;
  }
};
