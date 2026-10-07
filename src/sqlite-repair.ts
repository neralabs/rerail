import { realpathSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname } from "node:path";
import { pathToFileURL } from "node:url";
import { type NpmRunner, runCommand, runNpm } from "./npm.js";

export const SQLITE_PACKAGE_NAME = "better-sqlite3";

/** Loads the native binding the way the daemon will; a binary built for another Node ABI fails here, not at import. */
export const SQLITE_PROBE_SOURCE = `new(require('${SQLITE_PACKAGE_NAME}'))(':memory:').close()`;

export interface SqliteRepairOptions {
  /** Node the daemon will run under; the probe and rebuild must see that Node's ABI. Defaults to this process. */
  executable?: string;
  npm?: NpmRunner;
  log?: (line: string) => void;
}

export interface SqliteRepairResult {
  /** The npm prefix whose `node_modules` holds the binding. */
  prefix: string;
  rebuilt: boolean;
}

export type SqliteProbe = { ok: true } | { ok: false; error: string };

interface NodeAbi {
  node: string;
  modules: string;
}

/**
 * The install prefix whose `node_modules` resolves `better-sqlite3` for `packageRoot`.
 *
 * A checkout resolves to itself; a managed install below `~/.rerail/versions/<v>/node_modules/@neralabs/rerail`
 * resolves to `~/.rerail/versions/<v>`, where npm hoisted the binding and where `npm rebuild` must run.
 * A symlinked root such as `~/.rerail/current` is resolved first: `createRequire` walks the parents of the path it
 * is given, and the link's parents hold no `node_modules`.
 */
export function sqliteInstallPrefix(packageRoot: string): string | undefined {
  let root: string;
  try {
    root = realpathSync(packageRoot);
  } catch {
    return undefined;
  }
  const require = createRequire(pathToFileURL(`${root}/`));
  let manifest: string;
  try {
    manifest = require.resolve(`${SQLITE_PACKAGE_NAME}/package.json`);
  } catch {
    return undefined;
  }
  return dirname(dirname(dirname(manifest)));
}

export async function probeSqlite(executable: string, prefix: string): Promise<SqliteProbe> {
  try {
    await runCommand(executable, ["-e", SQLITE_PROBE_SOURCE], { cwd: prefix });
    return { ok: true };
  } catch (error) {
    return { ok: false, error: (error as Error).message };
  }
}

async function nodeAbi(executable: string): Promise<NodeAbi> {
  if (executable === process.execPath) return { node: process.versions.node, modules: process.versions.modules };
  try {
    const { stdout } = await runCommand(executable, ["-p", "process.versions.node + ' ' + process.versions.modules"]);
    const [node = "unknown", modules = "unknown"] = stdout.trim().split(" ");
    return { node, modules };
  } catch {
    return { node: "unknown", modules: "unknown" };
  }
}

function unrepairable(prefix: string, abi: NodeAbi, detail: string): Error {
  return new Error(
    `${SQLITE_PACKAGE_NAME} in ${prefix} cannot load under Node ${abi.node} (ABI ${abi.modules}) and npm rebuild did not fix it.\n` +
    `${detail}\n` +
    `Check that the rebuild can download or compile the binding for this Node, then run rerail daemon start again; ` +
    `or reinstall Rerail with npx @neralabs/rerail setup.`,
  );
}

/**
 * Probe the SQLite binding the daemon will load, rebuild it for the running Node when it fails, and re-probe.
 *
 * Runs in the launcher, before the daemon process exists: `rerail daemon start`, and the autostart it spawns, can then
 * report an unrepairable binding to whoever ran them instead of leaving the daemon to fail in `daemon.log`.
 */
export async function ensureSqlite(packageRoot: string, options: SqliteRepairOptions = {}): Promise<SqliteRepairResult> {
  const executable = options.executable ?? process.execPath;
  const npm = options.npm ?? runNpm;
  const log = options.log ?? ((): void => {});
  const prefix = sqliteInstallPrefix(packageRoot);
  if (prefix === undefined) {
    throw new Error(
      `${SQLITE_PACKAGE_NAME} is not installed below ${packageRoot}; reinstall Rerail with npx @neralabs/rerail setup.`,
    );
  }
  const first = await probeSqlite(executable, prefix);
  if (first.ok) return { prefix, rebuilt: false };
  const abi = await nodeAbi(executable);
  log(`Rebuilding ${SQLITE_PACKAGE_NAME} in ${prefix} for Node ${abi.node} (ABI ${abi.modules})`);
  try {
    await npm(["rebuild", SQLITE_PACKAGE_NAME], { cwd: prefix, node: executable });
  } catch (error) {
    throw unrepairable(prefix, abi, `Rebuild failed: ${(error as Error).message}`);
  }
  const second = await probeSqlite(executable, prefix);
  if (!second.ok) throw unrepairable(prefix, abi, `Probe still fails: ${second.error}`);
  return { prefix, rebuilt: true };
}
