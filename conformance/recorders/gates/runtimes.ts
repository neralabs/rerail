import { createHash } from "node:crypto";
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { copyFile, mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { runChecked, type CommandRunner } from "../../gates/process.js";
import type { Check } from "../../gates/types.js";

/**
 * Pinned inputs an adapter runs upstream with: a sparse checkout of upstream's repository at the pinned commit, and
 * runtimes installed from the manifests committed under `runtimes/<runtime>/` beside this module. A manifest pins every
 * direct dependency at an exact version; transitive packages resolve at install time and no lockfile is committed, so
 * nothing here feeds Dependabot. When a transitive release changes what upstream answers, `gates:record --check` (the
 * nightly) fails, and the fix is to pin that package too (`overrides`). Every cached tree is reused only while it is
 * unmodified, so editing the cache never changes a result.
 */

/** The committed runtimes: one directory per runtime, holding its npm or uv manifest. */
export const RUNTIMES_DIRECTORY = fileURLToPath(new URL("./runtimes/", import.meta.url));

const sha256 = (value: string | Buffer): string => createHash("sha256").update(value).digest("hex");

/** Every file under a directory whose path satisfies `predicate`, depth first, in name order (code units, the same on every machine). */
export function filesUnder(directory: string, predicate: (path: string) => boolean): string[] {
  if (!existsSync(directory)) return [];
  const found: string[] = [];
  for (const entry of readdirSync(directory, { withFileTypes: true }).sort((a, b) => (a.name < b.name ? -1 : a.name > b.name ? 1 : 0))) {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) found.push(...filesUnder(path, predicate));
    else if (entry.isFile() && predicate(path)) found.push(path);
  }
  return found;
}

/** A digest of every file under a directory (that `keep` accepts), by relative path and content. */
export function treeDigest(directory: string, keep: (path: string) => boolean = () => true): string {
  return sha256(filesUnder(directory, keep).map((path) => `${relative(directory, path)}\0${sha256(readFileSync(path))}`).join("\n"));
}

export interface UpstreamCheckout {
  directory: string;
  checks: Check[];
}

/**
 * Upstream's repository at the pinned commit, sparse and shallow, plus the release tag the runtime was published from.
 * `releasePath` is the source tree the published package is built from: a tool-pack check compares it at the pinned
 * commit and at the release tag, so the runtime that produces expected answers is the code the pack pins.
 */
export async function checkoutUpstream(
  cacheRoot: string,
  name: string,
  upstream: { repository: string; commit: string },
  tag: string,
  releasePath: string,
  sparsePaths: string[],
  runner: CommandRunner,
): Promise<UpstreamCheckout> {
  const directory = resolve(cacheRoot, "upstream", name, upstream.commit);
  const git = (...args: string[]) => runChecked(runner, ["git", "-C", directory, ...args]).then((out) => out.trim());
  const pristine = async () => existsSync(join(directory, ".git"))
    && (await git("rev-parse", "HEAD").catch(() => "")) === upstream.commit
    && (await git("status", "--porcelain").catch(() => "?")) === "";
  if (!(await pristine())) {
    await rm(directory, { recursive: true, force: true });
    await mkdir(directory, { recursive: true });
    await git("init", "-q");
    await git("remote", "add", "origin", `${upstream.repository}.git`);
    await git("sparse-checkout", "set", "--no-cone", ...sparsePaths);
    await git("fetch", "-q", "--depth", "1", "--filter=blob:none", "origin", upstream.commit);
    await git("checkout", "-q", "--force", "FETCH_HEAD");
  } else {
    // The gates fetch upstream's test files into the same directory with a narrower sparse set: widen it, never narrow.
    const sparse = (await git("sparse-checkout", "list").catch(() => "")).split("\n");
    const missing = sparsePaths.filter((path) => !sparse.includes(path));
    if (missing.length > 0) await git("sparse-checkout", "add", ...missing);
  }
  if ((await git("rev-parse", "-q", "--verify", `refs/tags/${tag}^{commit}`).catch(() => "")) === "") {
    await git("fetch", "-q", "--depth", "1", "--filter=blob:none", "origin", `refs/tags/${tag}:refs/tags/${tag}`);
  }
  const head = await git("rev-parse", "HEAD");
  const clean = await pristine();
  const commitTree = await git("rev-parse", `${upstream.commit}:${releasePath}`);
  const releaseTree = await git("rev-parse", `refs/tags/${tag}^{commit}:${releasePath}`);
  return {
    directory,
    checks: [
      {
        id: "upstream-commit",
        ok: head === upstream.commit && clean,
        detail: `upstream checkout is at ${head} with ${clean ? "an unmodified" : "a modified"} working tree; the pack pins ${upstream.commit}`,
      },
      {
        id: "runtime-matches-commit",
        ok: commitTree === releaseTree,
        detail: `${releasePath} at the pinned commit is tree ${commitTree}; at release tag ${tag} it is tree ${releaseTree}`,
      },
    ],
  };
}

/** Reuse `directory` only while its recorded digest of `tree` holds; otherwise rebuild it with `install`. */
async function cachedInstall(directory: string, tree: string, install: () => Promise<void>): Promise<void> {
  const record = `${directory}.tree.sha256`;
  const intact = existsSync(tree) && existsSync(record) && (await readFile(record, "utf8")).trim() === treeDigest(tree);
  if (intact) return;
  await rm(directory, { recursive: true, force: true });
  await mkdir(directory, { recursive: true });
  await install();
  await writeFile(record, `${treeDigest(tree)}\n`);
}

/** The files a runtime of each kind is pinned by: its manifest, and for Python the exact interpreter. */
export const NPM_PINS = ["package.json"] as const;
export const UV_PINS = ["pyproject.toml", ".python-version"] as const;

/** The sha256 of each pin file of `runtimes/<runtime>/` that the directory has, by file name: what a recording pins. */
export function runtimePinDigests(runtime: string, directory: string = RUNTIMES_DIRECTORY): Record<string, string> {
  const source = join(directory, runtime);
  return Object.fromEntries([...NPM_PINS, ...UV_PINS].filter((file) => existsSync(join(source, file))).sort()
    .map((file) => [file, sha256(readFileSync(join(source, file)))]));
}

/** An exact npm version, or an alias to one (`npm:<name>@<version>`). */
const EXACT_NPM = /^(?:npm:(?:@[a-z0-9._-]+\/)?[a-z0-9._-]+@)?\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/u;

/** The direct dependencies of a runtime's package.json that are not pinned at an exact version. */
export function unpinnedNpmDependencies(manifest: { dependencies?: Record<string, string> }): string[] {
  return Object.entries(manifest.dependencies ?? {}).filter(([, version]) => !EXACT_NPM.test(version)).map(([name, version]) => `${name}@${version}`);
}

/** The dependencies of a runtime's pyproject.toml that are not pinned with `==` at an exact version. */
export function unpinnedPythonDependencies(pyproject: string): string[] {
  const list = /^dependencies\s*=\s*\[([^\]]*)\]/mu.exec(pyproject)?.[1] ?? "";
  return [...list.matchAll(/"([^"]+)"/gu)].map((match) => match[1]!).filter((requirement) => !/^[A-Za-z0-9._-]+==\d+(?:\.\d+)*$/u.test(requirement));
}

/** The pin files of `runtimes/<runtime>/`, and a cache directory keyed by their digest. */
async function pinnedDirectory(runtime: string, files: readonly string[], cacheRoot: string): Promise<{ source: string; directory: string }> {
  const source = join(RUNTIMES_DIRECTORY, runtime);
  const digest = sha256((await Promise.all(files.map(async (file) => `${file}\0${sha256(await readFile(join(source, file)))}`))).join("\n"));
  return { source, directory: resolve(cacheRoot, "runtimes", runtime, digest.slice(0, 16)) };
}

/**
 * A Node runtime installed with `npm install --ignore-scripts` from `runtimes/<runtime>/package.json`, every direct
 * dependency at an exact version. The tree it resolved is in the cache directory's package-lock.json. Returns its directory.
 */
export async function npmRuntime(runtime: string, cacheRoot: string, runner: CommandRunner): Promise<string> {
  const files = NPM_PINS;
  const { source, directory } = await pinnedDirectory(runtime, files, cacheRoot);
  const unpinned = unpinnedNpmDependencies(JSON.parse(await readFile(join(source, "package.json"), "utf8")) as { dependencies?: Record<string, string> });
  if (unpinned.length > 0) throw new Error(`runtimes/${runtime}/package.json must pin every dependency at an exact version: ${unpinned.join(", ")}`);
  await cachedInstall(directory, join(directory, "node_modules"), async () => {
    for (const file of files) await copyFile(join(source, file), join(directory, file));
    // No install scripts: transitive packages are not locked, and none of the runtimes needs one (esbuild and
    // unrs-resolver ship their binaries as platform packages).
    await runChecked(runner, ["npm", "install", "--no-audit", "--no-fund", "--ignore-scripts"], { cwd: directory });
  });
  return directory;
}

export interface PythonRuntime {
  directory: string;
  /** The environment's interpreter. */
  python: string;
  /** `python --version` of that interpreter, for the report. */
  version: string;
  /** The exact interpreter `.python-version` pins, e.g. `3.13.12`; a pack check compares it with `version`. */
  pinned: string;
}

/**
 * A Python environment installed with `uv sync` from `runtimes/<runtime>/` (pyproject.toml, every dependency `==` an
 * exact version, and .python-version, which pins the interpreter). uv fetches the interpreter on first use. The tree it
 * resolved is in the cache directory's uv.lock.
 */
export async function uvRuntime(runtime: string, cacheRoot: string, runner: CommandRunner): Promise<PythonRuntime> {
  const files = UV_PINS;
  const { source, directory } = await pinnedDirectory(runtime, files, cacheRoot);
  const unpinned = unpinnedPythonDependencies(await readFile(join(source, "pyproject.toml"), "utf8"));
  if (unpinned.length > 0) throw new Error(`runtimes/${runtime}/pyproject.toml must pin every dependency with == at an exact version: ${unpinned.join(", ")}`);
  const environment = join(directory, ".venv");
  await cachedInstall(directory, join(environment, "lib"), async () => {
    for (const file of files) await copyFile(join(source, file), join(directory, file));
    // Only uv's own interpreter builds: a system or Homebrew Python of the same minor version is not the pinned one.
    const env: NodeJS.ProcessEnv = { ...process.env, UV_PROJECT_ENVIRONMENT: environment, UV_PYTHON_PREFERENCE: "only-managed" };
    delete env.VIRTUAL_ENV;
    await runChecked(runner, ["uv", "sync", "--no-install-project", "--quiet"], { cwd: directory, env });
  });
  const python = join(environment, "bin", "python");
  const version = (await runChecked(runner, [python, "--version"])).trim();
  const pinned = (await readFile(join(source, ".python-version"), "utf8")).trim();
  if (!/^\d+\.\d+\.\d+$/u.test(pinned)) throw new Error(`runtimes/${runtime}/.python-version must pin an exact interpreter, found ${pinned}`);
  return { directory, python, version, pinned };
}
