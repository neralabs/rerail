import { createHash } from "node:crypto";
import { existsSync, readdirSync, readFileSync, realpathSync } from "node:fs";
import { copyFile, mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { join, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

import { parse as parseToml } from "smol-toml";

import { runChecked, type CommandRunner } from "../../port-readiness/process.js";
import type { ResolvedRuntime } from "../../port-readiness/recording/diff.js";
import { LICENSE_SPARSE_PATTERNS, upstreamCheckoutDirectory } from "../../port-readiness/recorded-adapter.js";
import type { Check } from "../../port-readiness/types.js";

/**
 * Pinned inputs an adapter runs upstream with: a sparse checkout of upstream's repository at the pinned commit, and
 * runtimes installed from the manifests committed under `runtimes/<runtime>/` beside this module. A manifest pins every
 * direct dependency at an exact version; transitive packages resolve at install time and no lockfile is committed, so
 * only those direct pins are in GitHub's dependency graph (Dependabot's version updates cover the root package alone; a
 * security update against a pin here changes it and fails `runtime-pins` until the tool-pack is recorded again). When a
 * transitive release changes what upstream answers, `port-readiness:record --check` (the nightly) fails, and the fix is to pin
 * that package too (`overrides`). Every cached tree is reused only while it is
 * unmodified, so editing the cache never changes a result.
 */

/** The committed runtimes: one directory per runtime, holding its npm, uv or Composer manifest. */
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

/**
 * `text` with this machine's directories taken out of every path under them, so what upstream says about a file (a
 * message naming the scratch copy it analysed, or a PHP error naming a vendor file of the runtime) reads the same on
 * every machine and a recording made on one matches a lock made on another. Each directory is taken as given and
 * resolved through its symlinks (macOS's /var is /private/var), the longest first, so a directory inside another (the
 * scratch directory inside the cache root) leaves no part of itself behind.
 */
export function withoutMachinePaths(text: string, directories: readonly string[]): string {
  const prefixes = [...new Set(directories.flatMap((directory) => {
    const given = resolve(directory);
    return existsSync(given) ? [given, realpathSync(given)] : [given];
  }))].sort((a, b) => b.length - a.length);
  return prefixes.reduce((stripped, prefix) => stripped.replaceAll(`${prefix}/`, ""), text);
}

/**
 * Whether `path` exists under `root` with every component below `root` named exactly as given. macOS's default
 * filesystem finds `grammar/Sequence.py` when the file is `grammar/sequence.py`, Linux's does not, and a module
 * resolved through `existsSync` alone made the recording differ between them.
 */
export function existsExactly(root: string, path: string): boolean {
  if (!existsSync(path)) return false;
  let directory = root;
  for (const part of relative(root, path).split(sep)) {
    if (!readdirSync(directory).includes(part)) return false;
    directory = join(directory, part);
  }
  return true;
}

export interface UpstreamCheckout {
  directory: string;
  checks: Check[];
}

/**
 * Upstream's repository at the pinned commit, sparse and shallow, plus the release tag the runtime was published from.
 * `releasePath` is the source tree the published package is built from: a tool-pack check compares it at the pinned
 * commit and at the release tag, so the runtime that produces expected answers is the code the pack pins. The checkout
 * is named for the repository (upstreamCheckoutDirectory), where the recorder and the gates find it, and holds every
 * licence file at upstream's root besides `paths` (LICENSE_SPARSE_PATTERNS), which upstreamLicenseText reads.
 */
export async function checkoutUpstream(
  cacheRoot: string,
  upstream: { repository: string; commit: string },
  tag: string,
  releasePath: string,
  paths: string[],
  runner: CommandRunner,
): Promise<UpstreamCheckout> {
  const directory = upstreamCheckoutDirectory(cacheRoot, upstream);
  const sparsePaths = [...paths, ...LICENSE_SPARSE_PATTERNS.filter((pattern) => !paths.includes(pattern))];
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

/** The files a runtime of each kind is pinned by: its manifest, and for Python and PHP the exact interpreter. */
export const NPM_PINS = ["package.json"] as const;
export const UV_PINS = ["pyproject.toml", ".python-version"] as const;
export const COMPOSER_PINS = ["composer.json", ".php-version"] as const;
/**
 * The extensions the pinned PHP loads, one per line as `php -n -m` names them, sorted: the build's profile, which
 * answers depend on (PHP_CodeSniffer's DeprecatedFunctions lists the functions `get_defined_functions()` finds). It is
 * kept once, in runtimes/php (whose build.sh builds that PHP), which every PHP tool-pack installs, so its digest is in
 * every PHP recording's lock.
 */
export const PHP_EXTENSIONS = "extensions.txt";
/** A Rust toolchain is pinned by its rustup toolchain file: an exact release channel and its components. */
export const RUSTUP_PINS = ["rust-toolchain.toml"] as const;

/** The sha256 of each pin file of `runtimes/<runtime>/` that the directory has, by file name: what a recording pins. */
export function runtimePinDigests(runtime: string, directory: string = RUNTIMES_DIRECTORY): Record<string, string> {
  const source = join(directory, runtime);
  return Object.fromEntries([...NPM_PINS, ...UV_PINS, ...COMPOSER_PINS, PHP_EXTENSIONS, ...RUSTUP_PINS].filter((file) => existsSync(join(source, file))).sort()
    .map((file) => [file, sha256(readFileSync(join(source, file)))]));
}

/** An exact npm version, or an alias to one (`npm:<name>@<version>`). */
const EXACT_NPM = /^(?:npm:(?:@[a-z0-9._-]+\/)?[a-z0-9._-]+@)?\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/u;

/** npm `overrides` values, nested by dependent, as `path@version`: a `$name` reference stands for a direct dependency. */
function overrideVersions(overrides: Record<string, unknown>, path: string[] = []): [string, string][] {
  return Object.entries(overrides).flatMap(([key, value]): [string, string][] => {
    if (typeof value === "string") return [[[...path, key === "." ? [] : [key]].flat().join(" > "), value]];
    return value !== null && typeof value === "object" ? overrideVersions(value as Record<string, unknown>, [...path, key]) : [];
  });
}

/** The dependencies and `overrides` of a runtime's package.json that are not pinned at an exact version. */
export function unpinnedNpmDependencies(manifest: { dependencies?: Record<string, string>; overrides?: Record<string, unknown> }): string[] {
  return [...Object.entries(manifest.dependencies ?? {}), ...overrideVersions(manifest.overrides ?? {})]
    .filter(([, version]) => !EXACT_NPM.test(version) && !/^\$[A-Za-z0-9@/._-]+$/u.test(version))
    .map(([name, version]) => `${name}@${version}`);
}

/** A requirement pinned with `==` at an exact version, extras allowed (`bandit[toml]==1.9.4`). */
const EXACT_PYTHON = /^[A-Za-z0-9._-]+(?:\[[A-Za-z0-9._-]+(?:,[A-Za-z0-9._-]+)*\])?==\d+(?:\.\d+)*$/u;

/** The `tool.uv` settings the pin check reads; any other (`sources`, `index`) can change what is installed. */
const UV_SETTINGS = new Set(["package", "dev-dependencies", "override-dependencies"]);

/**
 * The requirements of a runtime's pyproject.toml that are not pinned with `==` at an exact version, each after the list
 * it is in (none for `project.dependencies`). Every list `uv sync` can install from is read: the project's dependencies
 * and extras, the dependency groups (it installs `dev` by default), and uv's dev dependencies and overrides. A
 * pyproject.toml that is not TOML, a list that is not one, and a `tool.uv` setting the check does not read are reported
 * too, so nothing it cannot judge passes.
 */
export function unpinnedPythonDependencies(pyproject: string): string[] {
  let manifest: Record<string, unknown>;
  try {
    manifest = parseToml(pyproject);
  } catch (error) {
    return [`(not TOML: ${(error instanceof Error ? error.message : String(error)).split("\n")[0]})`];
  }
  const table = (value: unknown): Record<string, unknown> => (value !== null && typeof value === "object" && !Array.isArray(value) ? value as Record<string, unknown> : {});
  const project = table(manifest.project);
  const uv = table(table(manifest.tool).uv);
  const lists: [string, unknown][] = [
    ["", project.dependencies],
    ...Object.entries(table(project["optional-dependencies"])).map(([extra, list]): [string, unknown] => [`project.optional-dependencies.${extra}: `, list]),
    ...Object.entries(table(manifest["dependency-groups"])).map(([group, list]): [string, unknown] => [`dependency-groups.${group}: `, list]),
    ["tool.uv.dev-dependencies: ", uv["dev-dependencies"]],
    ["tool.uv.override-dependencies: ", uv["override-dependencies"]],
  ];
  const unpinned: string[] = [];
  for (const [label, list] of lists) {
    if (list === undefined) continue;
    if (!Array.isArray(list)) {
      unpinned.push(`${label}(not a list)`);
      continue;
    }
    for (const requirement of list) {
      // A dependency group may include another group, whose own requirements are read as a list of their own.
      const includesGroup = requirement !== null && typeof requirement === "object" && Object.keys(requirement).length === 1 && typeof (requirement as Record<string, unknown>)["include-group"] === "string";
      if (typeof requirement === "string" ? !EXACT_PYTHON.test(requirement) : !includesGroup) unpinned.push(`${label}${typeof requirement === "string" ? requirement : JSON.stringify(requirement)}`);
    }
  }
  for (const setting of Object.keys(uv).filter((key) => !UV_SETTINGS.has(key)).sort()) unpinned.push(`tool.uv.${setting} (not read by the pin check)`);
  return unpinned;
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

/** An exact Composer version: a release, with Composer's optional `v` and a pre-release suffix (`v5.9.0`, `4.0.0-RC1`). */
const EXACT_COMPOSER = /^v?\d+\.\d+\.\d+(?:-[0-9A-Za-z.]+)?$/u;

/**
 * The composer.json keys the pin check reads. Any other can change what is installed or run at install time
 * (`repositories`, `minimum-stability`, `config`'s `platform`, `scripts`, `autoload`), so it is reported rather than
 * read past.
 */
const COMPOSER_KEYS: Record<string, true> = { name: true, description: true, license: true, type: true, require: true };

/**
 * The requirements of a runtime's composer.json that are not pinned at an exact version, as `name@constraint`, and any
 * key the check does not read. Platform requirements (`php`, `ext-*`) are requirements too: the interpreter is pinned
 * by `.php-version`, which the runtime checks, and a range on it here would say less. A composer.json that is not a JSON
 * object is reported as such, so nothing the check cannot judge passes.
 */
export function unpinnedComposerDependencies(composerJson: string): string[] {
  let manifest: unknown;
  try {
    manifest = JSON.parse(composerJson);
  } catch (error) {
    return [`(not JSON: ${(error instanceof Error ? error.message : String(error)).split("\n")[0]})`];
  }
  if (manifest === null || typeof manifest !== "object" || Array.isArray(manifest)) return ["(not a JSON object)"];
  const fields = manifest as Record<string, unknown>;
  const unpinned: string[] = [];
  const require = fields.require;
  if (require !== undefined && (require === null || typeof require !== "object" || Array.isArray(require))) unpinned.push("require (not an object)");
  else {
    for (const [name, constraint] of Object.entries((require ?? {}) as Record<string, unknown>)) {
      if (typeof constraint !== "string" || !EXACT_COMPOSER.test(constraint)) unpinned.push(`${name}@${typeof constraint === "string" ? constraint : JSON.stringify(constraint)}`);
    }
  }
  for (const key of Object.keys(fields).filter((key) => COMPOSER_KEYS[key] !== true).sort()) unpinned.push(`${key} (not read by the pin check)`);
  return unpinned;
}

/**
 * The php.ini settings every PHP process of the gates runs with: no php.ini at all (`-n`), so neither the machine's
 * settings nor the extensions it loads reach an answer (the pinned build has every extension compiled in), and on top
 * of PHP's built-in defaults: `short_open_tag` off, since it changes how PHP tokenizes `<?` (php.ini-development and
 * php.ini-production turn it off, PHP's built-in default is on, and upstream's tests skip their short-tag cases when it
 * is off); errors to stderr, so a deprecation notice never lands in a driver's JSON on stdout; memory not limited, as
 * PHP_CodeSniffer's own scripts run it; and UTF-8 as the default charset, which mbstring's functions take as their
 * encoding when they are given none (stillat/blade-parser's offsets are mb_strlen's and mb_substr's).
 */
export const PHP_INI = ["-n", "-d", "short_open_tag=Off", "-d", "display_errors=stderr", "-d", "error_reporting=-1", "-d", "memory_limit=-1", "-d", "default_charset=UTF-8"] as const;

export interface PhpRuntime {
  /** The cache directory Composer installed into: composer.json, the composer.lock it resolved, and vendor/. */
  directory: string;
  /** The interpreter, `PHP_BINARY` of the `php` on PATH: the one Composer ran on and every driver runs on. */
  php: string;
  /** That interpreter's `PHP_VERSION`, e.g. `8.5.11`: always `pinned`, since the runtime refuses any other. */
  version: string;
  /** The exact interpreter `.php-version` pins. */
  pinned: string;
  /** An executable a package of the runtime installs under vendor/bin (`phpcs`). */
  vendorBin(name: string): string;
  /** A command running a PHP script on the pinned interpreter with PHP_INI: `[php, ...PHP_INI, script, ...args]`. */
  command(script: string, ...args: string[]): string[];
}

/**
 * The `php` on PATH, which must be exactly the release `runtimes/<runtime>/.php-version` pins (an exact `x.y.z`), with
 * exactly the extensions runtimes/php/extensions.txt lists when it runs with no php.ini; any other throws, so nothing
 * is recorded on another PHP or another build of it. Unlike uv, Composer installs no interpreter: runtimes/php/build.sh
 * builds the pinned one, which CI's record and nightly legs put on PATH (.github/actions/php) and a maintainer puts
 * first on PATH to record locally.
 */
export async function pinnedPhp(runtime: string, runner: CommandRunner): Promise<{ php: string; version: string; pinned: string }> {
  const pinned = (await readFile(join(RUNTIMES_DIRECTORY, runtime, ".php-version"), "utf8")).trim();
  if (!/^\d+\.\d+\.\d+$/u.test(pinned)) throw new Error(`runtimes/${runtime}/.php-version must pin an exact interpreter, found ${pinned}`);
  const [version = "", php = ""] = (await runChecked(runner, ["php", ...PHP_INI, "-r", "echo PHP_VERSION, \"\\n\", PHP_BINARY, \"\\n\";"])).split("\n");
  if (version !== pinned) throw new Error(`runtimes/${runtime}/.php-version pins PHP ${pinned}; the php on PATH (${php || "unknown"}) is ${version || "unknown"}`);
  const profile = extensionList(await readFile(join(RUNTIMES_DIRECTORY, "php", PHP_EXTENSIONS), "utf8"));
  const loaded = extensionList(await runChecked(runner, [php, "-n", "-m"]));
  const missing = profile.filter((extension) => !loaded.includes(extension));
  const extra = loaded.filter((extension) => !profile.includes(extension));
  if (missing.length > 0 || extra.length > 0) {
    throw new Error(`${php} is not the PHP build runtimes/php/${PHP_EXTENSIONS} pins (build it with runtimes/php/build.sh): `
      + [missing.length > 0 ? `missing ${missing.join(", ")}` : "", extra.length > 0 ? `extra ${extra.join(", ")}` : ""].filter(Boolean).join("; "));
  }
  return { php, version, pinned };
}

/** The extension names of `php -m` output or of extensions.txt: one per line, section headers (`[PHP Modules]`) left out, sorted and distinct. */
export function extensionList(text: string): string[] {
  return [...new Set(text.split("\n").map((line) => line.trim()).filter((line) => line !== "" && !line.startsWith("[")))].sort((a, b) => (a < b ? -1 : a > b ? 1 : 0));
}

/**
 * A PHP runtime installed by Composer from `runtimes/<runtime>/` (composer.json, every requirement at an exact version,
 * and .php-version, which pins the interpreter exactly): `composer update` resolves it in the cache directory with no
 * plugins and no scripts, and the tree it resolved is in that directory's composer.lock. No lockfile is committed, as for
 * npm and uv: transitive packages resolve at install time, and the nightly's `--check` reports one that moves an answer.
 * What each runtime resolved is kept for the recording summary (resolvedComposerRuntimes), so a transitive release
 * shows there without being recorded, and Composer's audit runs with blocking off: an advisory against a resolved
 * package is written to stderr, in the log, instead of failing the install.
 */
export async function composerRuntime(runtime: string, cacheRoot: string, runner: CommandRunner): Promise<PhpRuntime> {
  const files = COMPOSER_PINS;
  const { source, directory } = await pinnedDirectory(runtime, files, cacheRoot);
  const unpinned = unpinnedComposerDependencies(await readFile(join(source, "composer.json"), "utf8"));
  if (unpinned.length > 0) throw new Error(`runtimes/${runtime}/composer.json must pin every requirement at an exact version: ${unpinned.join(", ")}`);
  const { php, version, pinned } = await pinnedPhp(runtime, runner);
  const env: NodeJS.ProcessEnv = { ...process.env, COMPOSER_NO_INTERACTION: "1" };
  await cachedInstall(directory, join(directory, "vendor"), async () => {
    for (const file of files) await copyFile(join(source, file), join(directory, file));
    // Composer writes its progress to stderr and the audit's result to stdout, which is all that is kept.
    const audit = await runChecked(runner, [php, ...PHP_INI, await composerPath(runner), "update", "--no-interaction", "--no-progress", "--no-plugins", "--no-scripts", "--prefer-dist", "--no-blocking", "--audit-format=plain"], { cwd: directory, env });
    if (audit.trim() !== "") process.stderr.write(`runtimes/${runtime}: composer audit:\n${audit.trimEnd()}\n`);
  });
  RESOLVED_COMPOSER.set(runtime, installedPackages(directory));
  return {
    directory,
    php,
    version,
    pinned,
    vendorBin: (name) => join(directory, "vendor", "bin", name),
    command: (script, ...args) => [php, ...PHP_INI, script, ...args],
  };
}

/** The Composer runtimes this process installed or reused, by runtime, with what each resolved. */
const RESOLVED_COMPOSER = new Map<string, string[]>();

/** The packages Composer installed into a runtime's vendor/ (`vendor/composer/installed.json`), as sorted `name@version`. */
export function installedPackages(directory: string): string[] {
  const installed = JSON.parse(readFileSync(join(directory, "vendor", "composer", "installed.json"), "utf8")) as { packages: { name: string; version: string }[] };
  return installed.packages.map((entry) => `${entry.name}@${entry.version}`).sort((a, b) => (a < b ? -1 : a > b ? 1 : 0));
}

/**
 * Every Composer runtime this process used and the package set it resolved, by runtime name. Nothing locks the
 * transitive packages, so the recording summary lists them: a drift between two recordings is visible there, and is not
 * in the recording archive or the lock, so a transitive release alone never fails a leg.
 */
export function resolvedComposerRuntimes(): ResolvedRuntime[] {
  return [...RESOLVED_COMPOSER].sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0)).map(([runtime, packages]) => ({ runtime, packages }));
}

/** The Composer on PATH, as a file the pinned interpreter runs (a phar, or Composer's bin script). */
async function composerPath(runner: CommandRunner): Promise<string> {
  const path = (await runChecked(runner, ["sh", "-c", "command -v composer"])).trim();
  if (path === "") throw new Error("Composer is not on PATH");
  return path;
}

/** A `rust-toolchain.toml` as rustup reads it, restricted to what a pin may say. */
export interface RustToolchainPin {
  /** An exact release, e.g. `1.99.0`. */
  channel: string;
  components: string[];
  profile: string;
}

/**
 * Parse a pinned `rust-toolchain.toml`: one `[toolchain]` table with `channel` (an exact release: a stable channel
 * name like `stable` or `1.99` moves), `components` and `profile`, and nothing else.
 */
export function parseRustToolchainPin(text: string, label: string): RustToolchainPin {
  const lines = text.split("\n").map((line) => line.replace(/#.*$/u, "").trim()).filter((line) => line !== "");
  if (lines[0] !== "[toolchain]") throw new Error(`${label} must hold one [toolchain] table`);
  const values: Record<string, string> = {};
  for (const line of lines.slice(1)) {
    const match = /^([a-z]+)\s*=\s*(.+)$/u.exec(line);
    if (match === null || match[1]! in values) throw new Error(`${label}: cannot read \`${line}\``);
    values[match[1]!] = match[2]!;
  }
  const extra = Object.keys(values).filter((key) => !["channel", "components", "profile"].includes(key));
  if (extra.length > 0) throw new Error(`${label} may set channel, components and profile only, not ${extra.join(", ")}`);
  const channel = /^"(\d+\.\d+\.\d+)"$/u.exec(values.channel ?? "")?.[1];
  const profile = /^"([a-z]+)"$/u.exec(values.profile ?? "")?.[1];
  const list = /^\[\s*((?:"[^"\\]*"\s*,\s*)*(?:"[^"\\]*")?)\s*,?\s*\]$/u.exec(values.components ?? "")?.[1];
  if (channel === undefined) throw new Error(`${label} must pin channel to an exact release, found ${values.channel ?? "none"}`);
  if (profile === undefined || list === undefined) throw new Error(`${label} must set components and profile`);
  return { channel, components: [...list.matchAll(/"([^"\\]*)"/gu)].map((match) => match[1]!), profile };
}

export interface RustupRuntime {
  pin: RustToolchainPin;
  /** `rustc +<channel> --version`, e.g. `rustc 1.99.0 (b940084d7 2026-09-28)`. */
  rustc: string;
  /** `clippy-driver --version` of the toolchain, e.g. `clippy 0.1.99 (b940084d7e 2026-09-28)`. */
  clippy: string;
  /** The version Clippy reports, e.g. `0.1.99`. */
  clippyVersion: string;
  /** The toolchain's clippy-driver. */
  driver: string;
  /** rustc's host target triple. */
  host: string;
  /** The process environment the toolchain runs in: no RUSTUP_TOOLCHAIN, RUSTFLAGS or RUSTC_WRAPPER of the caller's. */
  env: NodeJS.ProcessEnv;
}

/**
 * A Rust toolchain installed by rustup from `runtimes/<runtime>/rust-toolchain.toml` (an exact release and its
 * components), installed when missing and then checked: rustc must report that release, and the toolchain's
 * clippy-driver and `cargo clippy` the same build (rustc's commit and date). Anything else is refused, as
 * packs/rust/upstream/common.py's check_toolchain() refuses it. Every command names the toolchain (`+<channel>`).
 */
export async function rustupRuntime(runtime: string, runner: CommandRunner): Promise<RustupRuntime> {
  const source = join(RUNTIMES_DIRECTORY, runtime);
  const label = `runtimes/${runtime}/rust-toolchain.toml`;
  const pin = parseRustToolchainPin(await readFile(join(source, "rust-toolchain.toml"), "utf8"), label);
  const env: NodeJS.ProcessEnv = { ...process.env };
  for (const key of ["RUSTUP_TOOLCHAIN", "RUSTFLAGS", "RUSTC_WRAPPER", "RUSTC", "CARGO_BUILD_RUSTFLAGS"]) delete env[key];
  const toolchain = `+${pin.channel}`;
  const sysroot = async () => (await runner.run(["rustc", toolchain, "--print", "sysroot"], { env })).stdout.trim();
  const installedRoot = await sysroot().catch(() => "");
  if (installedRoot === "" || (pin.components.includes("clippy") && !existsSync(join(installedRoot, "bin", "clippy-driver")))) {
    await runChecked(runner, ["rustup", "toolchain", "install", pin.channel, "--profile", pin.profile, "--no-self-update", ...pin.components.flatMap((component) => ["--component", component])], { env });
  }
  const verbose = await runChecked(runner, ["rustc", toolchain, "-vV"], { env });
  const field = (name: string) => new RegExp(`^${name}: (.+)$`, "mu").exec(verbose)?.[1]?.trim() ?? "";
  const release = field("release");
  const commit = field("commit-hash");
  const date = field("commit-date");
  const host = field("host");
  const rustc = (await runChecked(runner, ["rustc", toolchain, "--version"], { env })).trim();
  if (release !== pin.channel || rustc !== `rustc ${pin.channel} (${commit.slice(0, 9)} ${date})`) {
    throw new Error(`${label} pins Rust ${pin.channel}; rustc ${toolchain} is \`${rustc}\` (release ${release})`);
  }
  const driver = join(await sysroot(), "bin", "clippy-driver");
  const clippy = (await runChecked(runner, [driver, "--version"], { env })).trim();
  const built = /^clippy (\d+\.\d+\.\d+) \(([0-9a-f]+) (\d{4}-\d{2}-\d{2})\)$/u.exec(clippy);
  if (built === null || !commit.startsWith(built[2]!) || built[3] !== date) {
    throw new Error(`${driver} is \`${clippy}\`, not the clippy of \`${rustc}\` (${commit} ${date})`);
  }
  const cargoClippy = (await runChecked(runner, ["cargo", toolchain, "clippy", "--version"], { env })).trim();
  if (cargoClippy !== clippy) throw new Error(`cargo ${toolchain} clippy is \`${cargoClippy}\`, its clippy-driver \`${clippy}\``);
  return { pin, rustc, clippy, clippyVersion: built[1]!, driver, host, env };
}
