/**
 * The pinned runtimes the derivation recipes (scripts/pack-data/recipes/) run upstream's tools on, installed from the
 * manifests committed under `runtimes/<runtime>/` beside this module. A manifest pins every direct dependency at an
 * exact version; transitive packages resolve at install time and no lockfile is committed, so only those direct pins
 * are in GitHub's dependency graph (Dependabot's version updates cover the root package alone). When a transitive
 * release changes what upstream answers, the fix is to pin that package too (`overrides`). Every cached install is
 * reused only while it is unmodified, so editing the cache never changes a result.
 */
import { createHash } from "node:crypto";
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { copyFile, mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { parse as parseToml } from "smol-toml";

import { runChecked, type CommandRunner } from "../../conformance/process.js";

/** The committed runtimes, relative to the repository root: one directory per runtime, holding its npm, uv or Composer manifest. */
export const RUNTIMES_DIRECTORY = "scripts/pack-data/runtimes";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");

/** `file` of the committed runtime `runtime` (`runtimes/<runtime>/<file>`), as an absolute path. */
export const runtimeFile = (runtime: string, file: string): string => join(ROOT, RUNTIMES_DIRECTORY, runtime, file);

const sha256 = (value: string | Buffer): string => createHash("sha256").update(value).digest("hex");

/** Every file under a directory, depth first, in name order (code units, the same on every machine). */
function filesUnder(directory: string): string[] {
  if (!existsSync(directory)) return [];
  const found: string[] = [];
  for (const entry of readdirSync(directory, { withFileTypes: true }).sort((a, b) => (a.name < b.name ? -1 : a.name > b.name ? 1 : 0))) {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) found.push(...filesUnder(path));
    else if (entry.isFile()) found.push(path);
  }
  return found;
}

/** A digest of every file under a directory, by relative path and content. */
const treeDigest = (directory: string): string => sha256(filesUnder(directory).map((path) => `${relative(directory, path)}\0${sha256(readFileSync(path))}`).join("\n"));

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
const NPM_PINS = ["package.json"] as const;
const UV_PINS = ["pyproject.toml", ".python-version"] as const;
const COMPOSER_PINS = ["composer.json", ".php-version"] as const;
/**
 * The extensions the pinned PHP loads, one per line as `php -n -m` names them, sorted: the build's profile, which
 * answers depend on (PHP_CodeSniffer's DeprecatedFunctions lists the functions `get_defined_functions()` finds). It is
 * kept once, in runtimes/php (whose build.sh builds that PHP).
 */
const PHP_EXTENSIONS = "extensions.txt";

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
  const source = join(ROOT, RUNTIMES_DIRECTORY, runtime);
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
  const pinned = (await readFile(join(source, ".python-version"), "utf8")).trim();
  if (!/^\d+\.\d+\.\d+$/u.test(pinned)) throw new Error(`runtimes/${runtime}/.python-version must pin an exact interpreter, found ${pinned}`);
  const environment = join(directory, ".venv");
  await cachedInstall(directory, join(environment, "lib"), async () => {
    for (const file of files) await copyFile(join(source, file), join(directory, file));
    // Only uv's own interpreter builds: a system or Homebrew Python of the same minor version is not the pinned one.
    const env: NodeJS.ProcessEnv = { ...process.env, UV_PROJECT_ENVIRONMENT: environment, UV_PYTHON_PREFERENCE: "only-managed" };
    delete env.VIRTUAL_ENV;
    await runChecked(runner, ["uv", "sync", "--no-install-project", "--quiet"], { cwd: directory, env });
  });
  return { directory, python: join(environment, "bin", "python") };
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
 * The php.ini settings every PHP process of a derivation runs with: no php.ini at all (`-n`), so neither the machine's
 * settings nor the extensions it loads reach an answer (the pinned build has every extension compiled in), and on top
 * of PHP's built-in defaults: `short_open_tag` off, since it changes how PHP tokenizes `<?` (php.ini-development and
 * php.ini-production turn it off, PHP's built-in default is on, and upstream's tests skip their short-tag cases when it
 * is off); errors to stderr, so a deprecation notice never lands in a driver's JSON on stdout; memory not limited, as
 * PHP_CodeSniffer's own scripts run it; and UTF-8 as the default charset, which mbstring's functions take as their
 * encoding when they are given none (stillat/blade-parser's offsets are mb_strlen's and mb_substr's).
 */
export const PHP_INI = ["-n", "-d", "short_open_tag=Off", "-d", "display_errors=stderr", "-d", "error_reporting=-1", "-d", "memory_limit=-1", "-d", "default_charset=UTF-8"] as const;

/**
 * The `php` on PATH, which must be exactly the release `runtimes/<runtime>/.php-version` pins (an exact `x.y.z`), with
 * exactly the extensions runtimes/php/extensions.txt lists when it runs with no php.ini; any other throws, so nothing
 * is derived on another PHP or another build of it. Unlike uv, Composer installs no interpreter: runtimes/php/build.sh
 * builds the pinned one, which CI puts on PATH (.github/actions/php) and a maintainer puts first on PATH to derive
 * locally.
 */
export async function pinnedPhp(runtime: string, runner: CommandRunner): Promise<string> {
  const pinned = (await readFile(runtimeFile(runtime, ".php-version"), "utf8")).trim();
  if (!/^\d+\.\d+\.\d+$/u.test(pinned)) throw new Error(`runtimes/${runtime}/.php-version must pin an exact interpreter, found ${pinned}`);
  const [version = "", php = ""] = (await runChecked(runner, ["php", ...PHP_INI, "-r", "echo PHP_VERSION, \"\\n\", PHP_BINARY, \"\\n\";"])).split("\n");
  if (version !== pinned) throw new Error(`runtimes/${runtime}/.php-version pins PHP ${pinned}; the php on PATH (${php || "unknown"}) is ${version || "unknown"}`);
  const profile = extensionList(await readFile(runtimeFile("php", PHP_EXTENSIONS), "utf8"));
  const loaded = extensionList(await runChecked(runner, [php, "-n", "-m"]));
  const missing = profile.filter((extension) => !loaded.includes(extension));
  const extra = loaded.filter((extension) => !profile.includes(extension));
  if (missing.length > 0 || extra.length > 0) {
    throw new Error(`${php} is not the PHP build runtimes/php/${PHP_EXTENSIONS} pins (build it with runtimes/php/build.sh): `
      + [missing.length > 0 ? `missing ${missing.join(", ")}` : "", extra.length > 0 ? `extra ${extra.join(", ")}` : ""].filter(Boolean).join("; "));
  }
  return php;
}

/** The extension names of `php -m` output or of extensions.txt: one per line, section headers (`[PHP Modules]`) left out, sorted and distinct. */
export function extensionList(text: string): string[] {
  return [...new Set(text.split("\n").map((line) => line.trim()).filter((line) => line !== "" && !line.startsWith("[")))].sort((a, b) => (a < b ? -1 : a > b ? 1 : 0));
}

/**
 * A PHP runtime installed by Composer from `runtimes/<runtime>/` (composer.json, every requirement at an exact version,
 * and .php-version, which pins the interpreter exactly): `composer update` resolves it in the cache directory with no
 * plugins and no scripts, and the tree it resolved is in that directory's composer.lock. No lockfile is committed, as for
 * npm and uv: transitive packages resolve at install time. Composer's audit runs with blocking off: an advisory against
 * a resolved package is written to stderr, in the log, instead of failing the install. Returns the cache directory
 * Composer installed into (composer.json, the composer.lock it resolved, and vendor/).
 */
export async function composerRuntime(runtime: string, cacheRoot: string, runner: CommandRunner): Promise<string> {
  const files = COMPOSER_PINS;
  const { source, directory } = await pinnedDirectory(runtime, files, cacheRoot);
  const unpinned = unpinnedComposerDependencies(await readFile(join(source, "composer.json"), "utf8"));
  if (unpinned.length > 0) throw new Error(`runtimes/${runtime}/composer.json must pin every requirement at an exact version: ${unpinned.join(", ")}`);
  const php = await pinnedPhp(runtime, runner);
  const env: NodeJS.ProcessEnv = { ...process.env, COMPOSER_NO_INTERACTION: "1" };
  await cachedInstall(directory, join(directory, "vendor"), async () => {
    for (const file of files) await copyFile(join(source, file), join(directory, file));
    // Composer writes its progress to stderr and the audit's result to stdout, which is all that is kept.
    const audit = await runChecked(runner, [php, ...PHP_INI, await composerPath(runner), "update", "--no-interaction", "--no-progress", "--no-plugins", "--no-scripts", "--prefer-dist", "--no-blocking", "--audit-format=plain"], { cwd: directory, env });
    if (audit.trim() !== "") process.stderr.write(`runtimes/${runtime}: composer audit:\n${audit.trimEnd()}\n`);
  });
  return directory;
}

/** The Composer on PATH, as a file the pinned interpreter runs (a phar, or Composer's bin script). */
async function composerPath(runner: CommandRunner): Promise<string> {
  const path = (await runChecked(runner, ["sh", "-c", "command -v composer"])).trim();
  if (path === "") throw new Error("Composer is not on PATH");
  return path;
}
