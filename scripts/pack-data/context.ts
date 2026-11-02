/**
 * What a derivation recipe (scripts/pack-data/recipes/) is handed: the tool-pack it derives, and the pinned inputs it
 * provisions before running the tool-pack's generator. Every input is named by a pin the repository holds (a commit, an
 * exact package version, an interpreter version), installed under the derive cache (`.rerail/pack-data/` unless
 * `--cache` says otherwise) and reused only while it still matches that pin, so a second derivation needs no network.
 */
import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, readFileSync } from "node:fs";
import { mkdir, rm, writeFile } from "node:fs/promises";
import { join, resolve } from "node:path";

import type { CommandRunner } from "../../conformance/process.js";

export interface RunOptions {
  cwd?: string;
  /** The child's whole environment; the derivation's own when absent. */
  env?: NodeJS.ProcessEnv;
  /** Text for the child's stdin. */
  input?: string;
}

export interface CheckoutOptions {
  /** Where the checkout goes; `<cache>/upstream/<repository name>@<commit>` when absent. */
  directory?: string;
  /** Sparse paths (git's non-cone patterns); the whole tree when absent. */
  sparse?: readonly string[];
  /** Tags to fetch beside the commit (`git describe`, a generator that checks a release tag). */
  tags?: readonly string[];
}

export interface DeriveContext {
  /** The repository root. */
  root: string;
  pack: string;
  tool: string;
  /** packs/<pack>/upstream/<tool>, absolute. */
  toolDirectory: string;
  /** Where the recipe's installs, checkouts and scratch files go (gitignored). */
  cacheRoot: string;
  /** Where the pinned runtimes (runtimes.ts's npmRuntime, uvRuntime, composerRuntime) install (gitignored). */
  runtimeCache: string;
  /** The runner the runtime helpers take. */
  runner: CommandRunner;
  /** Runs a command with its output shown, failing when it exits non-zero. */
  exec(argv: readonly string[], options?: RunOptions): Promise<void>;
  /** Runs a command and returns its stdout (its stderr is shown), failing when it exits non-zero. */
  capture(argv: readonly string[], options?: RunOptions): Promise<string>;
  /** Upstream's repository at `commit`, shallow; reused while it is still at that commit with no tracked file modified. */
  checkout(repository: string, commit: string, options?: CheckoutOptions): Promise<string>;
  /** A `node_modules` tree installed from `manifest` (a package.json object, every dependency at an exact version); returns its directory. */
  npmTools(name: string, manifest: Record<string, unknown>, options?: { scripts?: boolean }): Promise<string>;
  /**
   * uv's managed CPython at exactly `version` (`3.14.0`, or a minor version uv resolves); returns the interpreter's path.
   * `downloads` (a uv download list, UV_PYTHON_DOWNLOADS_JSON_URL's format, every entry one build of `version`) pins the
   * build too, installed apart from uv's own interpreters, since uv keeps one build of a version.
   */
  python(version: string, options?: { downloads?: string }): Promise<string>;
  /** A virtual environment on uv's managed CPython `python` with `requirements` (each `name==version`); returns its directory. */
  pythonTools(name: string, python: string, requirements: readonly string[]): Promise<string>;
  /** A Composer project from `manifest` (a composer.json object) installed with the `php` and `composer` on PATH; returns its directory. */
  composerTools(name: string, manifest: Record<string, unknown>): Promise<string>;
  /** A scratch directory of the recipe's own under the cache, emptied first. */
  scratch(name: string): Promise<string>;
  log(message: string): void;
}

const sha256 = (value: string | Buffer): string => createHash("sha256").update(value).digest("hex");

/**
 * The process environment without what would make one machine's run differ from another's (npm's and the agents'), and
 * with Python writing no bytecode beside the generators (packs/<id>/upstream/__pycache__/ would be an untracked file).
 */
export function derivationEnvironment(base: NodeJS.ProcessEnv = process.env): NodeJS.ProcessEnv {
  const env: NodeJS.ProcessEnv = { PYTHONDONTWRITEBYTECODE: "1" };
  for (const [key, value] of Object.entries(base)) {
    if (value === undefined) continue;
    if (/^npm_/iu.test(key) || ["AGENT", "CLAUDECODE", "OMPCODE", "ORCA_OPENCODE_AGENT", "VIRTUAL_ENV", "NODE_OPTIONS", "INIT_CWD"].includes(key)) continue;
    env[key] = value;
  }
  return env;
}

/**
 * How one tool-pack's derived pack test data is made from its pins: the generator(s) under packs/<pack>/upstream/,
 * run on inputs the recipe provisions. derive.ts runs it on a tool directory holding only the authored data (every
 * derived file removed), then its checks.
 */
export interface DeriveRecipe {
  pack: string;
  tool: string;
  derive(context: DeriveContext): Promise<void>;
}

function spawnChecked(argv: readonly string[], options: RunOptions, capture: boolean): Promise<string> {
  if (argv.length === 0) return Promise.reject(new Error("cannot run an empty command"));
  const { promise, resolve: done, reject } = Promise.withResolvers<string>();
  const child = spawn(argv[0]!, argv.slice(1), {
    cwd: options.cwd,
    env: options.env ?? derivationEnvironment(),
    stdio: [options.input === undefined ? "ignore" : "pipe", capture ? "pipe" : "inherit", "inherit"],
  });
  const stdout: Buffer[] = [];
  child.stdout?.on("data", (chunk: Buffer) => stdout.push(chunk));
  child.on("error", reject);
  child.on("close", (code, signal) => {
    if (code === 0) done(Buffer.concat(stdout).toString("utf8"));
    else reject(new Error(`${argv.slice(0, 4).join(" ")}${argv.length > 4 ? " …" : ""} ${signal === null ? `exited ${code}` : `was killed by ${signal}`}${options.cwd === undefined ? "" : ` (in ${options.cwd})`}`));
  });
  if (options.input !== undefined) {
    child.stdin!.once("error", reject);
    child.stdin!.end(options.input);
  }
  return promise;
}

/** A directory installed by `install`, reused while its marker still records `pin`. */
async function pinnedInstall(directory: string, pin: string, install: () => Promise<void>): Promise<string> {
  const marker = join(directory, ".rerail-derive-pin");
  if (existsSync(marker) && readFileSync(marker, "utf8") === pin) return directory;
  await rm(directory, { recursive: true, force: true });
  await mkdir(directory, { recursive: true });
  await install();
  await writeFile(marker, pin);
  return directory;
}

export interface ContextOptions {
  root: string;
  pack: string;
  tool: string;
  cacheRoot: string;
  runtimeCache: string;
  runner: CommandRunner;
  log: (message: string) => void;
}

export function createContext(options: ContextOptions): DeriveContext {
  const { root, pack, tool, cacheRoot, runtimeCache, runner, log } = options;
  const exec = async (argv: readonly string[], run: RunOptions = {}) => {
    await spawnChecked(argv, run, false);
  };
  const capture = (argv: readonly string[], run: RunOptions = {}) => spawnChecked(argv, run, true);
  const git = (directory: string, ...args: string[]) => capture(["git", "-C", directory, ...args]).then((out) => out.trim());

  const checkout = async (repository: string, commit: string, checkoutOptions: CheckoutOptions = {}): Promise<string> => {
    if (!/^[0-9a-f]{40}$/u.test(commit)) throw new Error(`${repository}: pin a full commit, not ${commit}`);
    const name = repository.replace(/\.git$/u, "").split("/").pop()!;
    const directory = resolve(checkoutOptions.directory ?? join(cacheRoot, "upstream", `${name}@${commit.slice(0, 12)}`));
    const sparse = [...(checkoutOptions.sparse ?? [])];
    const intact = existsSync(join(directory, ".git"))
      && (await git(directory, "rev-parse", "HEAD").catch(() => "")) === commit
      && (await git(directory, "status", "--porcelain", "--untracked-files=no").catch(() => "?")) === ""
      && (sparse.length === 0
        ? (await git(directory, "config", "--get", "core.sparseCheckout").catch(() => "")) !== "true"
        : (await git(directory, "sparse-checkout", "list").catch(() => "")).split("\n").join("\0") === sparse.join("\0"));
    if (!intact) {
      log(`checking out ${repository} at ${commit}${sparse.length > 0 ? ` (${sparse.length} sparse path(s))` : ""}`);
      await rm(directory, { recursive: true, force: true });
      await mkdir(directory, { recursive: true });
      await git(directory, "init", "-q");
      await git(directory, "remote", "add", "origin", repository.endsWith(".git") ? repository : `${repository}.git`);
      if (sparse.length > 0) await git(directory, "sparse-checkout", "set", "--no-cone", ...sparse);
      await git(directory, "fetch", "-q", "--depth", "1", ...(sparse.length > 0 ? ["--filter=blob:none"] : []), "origin", commit);
      await git(directory, "-c", "advice.detachedHead=false", "checkout", "-q", "--force", "FETCH_HEAD");
    }
    for (const tag of checkoutOptions.tags ?? []) {
      if ((await git(directory, "rev-parse", "-q", "--verify", `refs/tags/${tag}`).catch(() => "")) !== "") continue;
      await git(directory, "fetch", "-q", "--depth", "1", "--filter=blob:none", "origin", `refs/tags/${tag}:refs/tags/${tag}`);
    }
    return directory;
  };

  const npmTools = async (name: string, manifest: Record<string, unknown>, npmOptions: { scripts?: boolean } = {}): Promise<string> => {
    const text = `${JSON.stringify({ name: `rerail-derive-${name}`, private: true, ...manifest }, null, 2)}\n`;
    const pin = sha256(`${text}\0scripts=${npmOptions.scripts === true}`);
    return pinnedInstall(join(cacheRoot, "tools", `${name}-${pin.slice(0, 16)}`), pin, async () => {
      const directory = join(cacheRoot, "tools", `${name}-${pin.slice(0, 16)}`);
      await writeFile(join(directory, "package.json"), text);
      await exec(["npm", "install", "--no-audit", "--no-fund", "--loglevel=error", ...(npmOptions.scripts === true ? [] : ["--ignore-scripts"])], { cwd: directory });
    });
  };

  const python = async (version: string, options: { downloads?: string } = {}): Promise<string> => {
    if (options.downloads !== undefined) return pinnedBuild(version, options.downloads);
    const env = { ...derivationEnvironment(), UV_PYTHON_PREFERENCE: "only-managed" };
    // An interpreter uv already manages is used as it is: an older uv may no longer list its download.
    const found = await capture(["uv", "python", "find", version], { env }).catch(() => "");
    if (found.trim() !== "") return found.trim();
    await exec(["uv", "python", "install", "--quiet", version], { env });
    return (await capture(["uv", "python", "find", version], { env })).trim();
  };

  // The build a download list pins, in a directory of its own: uv keys installs by version, so an install directory
  // holding another build of the version (a newer uv's) would be used as it is.
  const pinnedBuild = async (version: string, downloads: string): Promise<string> => {
    const entries = Object.values(JSON.parse(readFileSync(downloads, "utf8")) as Record<string, { build?: unknown; major?: unknown; minor?: unknown; patch?: unknown }>);
    const builds = [...new Set(entries.map((entry) => entry.build))];
    const versions = [...new Set(entries.map((entry) => `${String(entry.major)}.${String(entry.minor)}.${String(entry.patch)}`))];
    const build = builds[0];
    if (builds.length !== 1 || typeof build !== "string" || versions.length !== 1 || versions[0] !== version) {
      throw new Error(`${downloads} must list one build of CPython ${version}; it lists ${versions.join(", ")} at ${builds.map(String).join(", ")}`);
    }
    const env = {
      ...derivationEnvironment(),
      UV_PYTHON_PREFERENCE: "only-managed",
      UV_PYTHON_INSTALL_DIR: join(cacheRoot, "python", build),
      UV_PYTHON_CPYTHON_BUILD: build,
      UV_PYTHON_DOWNLOADS_JSON_URL: resolve(downloads),
    };
    let interpreter = (await capture(["uv", "python", "find", version], { env }).catch(() => "")).trim();
    if (interpreter === "") {
      await exec(["uv", "python", "install", "--quiet", version], { env });
      interpreter = (await capture(["uv", "python", "find", version], { env })).trim();
    }
    const installed = (await capture([interpreter, "-c", "import pathlib, sys; print(pathlib.Path(sys.base_prefix, 'BUILD').read_text())"])).trim();
    if (installed !== build) throw new Error(`${interpreter} is CPython build ${installed}, not the ${build} ${downloads} pins`);
    return interpreter;
  };

  const pythonTools = async (name: string, interpreter: string, requirements: readonly string[]): Promise<string> => {
    const unpinned = requirements.filter((requirement) => !/^[A-Za-z0-9._-]+(?:\[[A-Za-z0-9._,-]+\])?==[\w.]+$/u.test(requirement));
    if (unpinned.length > 0) throw new Error(`${name}: pin every requirement with == at an exact version: ${unpinned.join(", ")}`);
    const version = (await capture([interpreter, "-c", "import sys; print(sys.version)"])).trim();
    const pin = sha256(`${resolve(interpreter)}\0${version}\0${[...requirements].sort().join("\n")}`);
    const directory = join(cacheRoot, "tools", `${name}-${pin.slice(0, 16)}`);
    return pinnedInstall(directory, pin, async () => {
      // The interpreter's own venv module: `uv venv` would put a uv-managed interpreter's minor-version link, in
      // whichever install directory the environment names, in its place, and that link may be another build.
      await exec([interpreter, "-m", "venv", "--without-pip", join(directory, "venv")]);
      const env = { ...derivationEnvironment(), UV_PYTHON_PREFERENCE: "only-managed" };
      if (requirements.length > 0) await exec(["uv", "pip", "install", "--quiet", "--python", join(directory, "venv", "bin", "python"), ...requirements], { env });
    });
  };

  const composerTools = async (name: string, manifest: Record<string, unknown>): Promise<string> => {
    const text = `${JSON.stringify(manifest, null, 4)}\n`;
    const phpVersion = (await capture(["php", "-n", "-r", "echo PHP_VERSION;"])).trim();
    const pin = sha256(`${text}\0php=${phpVersion}`);
    const directory = join(cacheRoot, "tools", `${name}-${pin.slice(0, 16)}`);
    return pinnedInstall(directory, pin, async () => {
      await writeFile(join(directory, "composer.json"), text);
      await exec(["composer", "update", "--no-interaction", "--no-progress", "--quiet"], { cwd: directory, env: { ...derivationEnvironment(), COMPOSER_NO_INTERACTION: "1" } });
    });
  };

  return {
    root,
    pack,
    tool,
    toolDirectory: join(root, "packs", pack, "upstream", tool),
    cacheRoot,
    runtimeCache,
    runner,
    exec,
    capture,
    checkout,
    npmTools,
    python,
    pythonTools,
    composerTools,
    async scratch(name: string) {
      const directory = join(cacheRoot, "scratch", `${pack}--${tool}`, name);
      await rm(directory, { recursive: true, force: true });
      await mkdir(directory, { recursive: true });
      return directory;
    },
    log,
  };
}
