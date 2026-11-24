#!/usr/bin/env node
// The npm packages that carry Rerail's native engine, one per operating system and processor. Core (@neralabs/rerail)
// ships no engine itself: npm's registry refuses a tarball that holds all six. Core names the six as exact-version
// optional dependencies instead, and each is published at core's version, before core.
//
// `pack` builds each package from a built engine (native/rerail_engine.<suffix>.node) and writes its tarball, and a
// manifest of what it packed (ENGINE_PACKAGES_FILE), into one folder. Packing core against that folder
// (RERAIL_ENGINE_PACKAGES, read by shrinkwrap-pack.mjs) pins each tarball's version and integrity in core's
// package.json and npm-shrinkwrap.json. Nothing is committed: a package's folder exists only while it is packed.
//
// The release packs all six (`--all` fails when an engine is absent). A checkout usually has only its own machine's
// engine: without `--all`, `pack` packs the engines it finds and names the ones it does not. A core packed against
// that folder installs with an engine on those platforms only, and the release's check refuses it (shrinkwrap-pack.mjs
// check).
//
// Nothing here needs node_modules.
//
//   node scripts/release/engine/packages.mjs pack --out <folder> [--native <folder>] [--all]
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { copyFileSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { parseArgs } from "node:util";
import { npmCommand } from "../../build/ensure-sqlite.mjs";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..", "..");

/** The manifest `pack` writes beside the tarballs: `{ version, packages: [{ suffix, name, version, file, integrity }] }`. */
export const ENGINE_PACKAGES_FILE = "engine-packages.json";

/** Names the folder `pack` wrote, for packing core against it and for the release's check (shrinkwrap-pack.mjs). */
export const ENGINE_PACKAGES_ENV = "RERAIL_ENGINE_PACKAGES";

/**
 * The engine packages: the one table of them, and the only place a script makes a package name. `suffix` is what the
 * runtime's `nativePlatformSuffix` (src/engine/native/native.ts) gives for `os` and `cpu` (Node's `process.platform`
 * and `process.arch`). The runtime loads the package of its own suffix and cannot import this file (src/ imports
 * nothing outside it), so test/scripts/release-engine-packages.test.mjs fails when the two disagree. `libc` keeps npm
 * from installing a glibc engine on a musl Linux. `target` is the Rust target the engine is built for (package.json's
 * `napi.targets`), and `engine` the file napi writes for it in native/.
 */
export const ENGINE_PACKAGES = [
  { suffix: "darwin-arm64", os: "darwin", cpu: "arm64", target: "aarch64-apple-darwin" },
  { suffix: "darwin-x64", os: "darwin", cpu: "x64", target: "x86_64-apple-darwin" },
  { suffix: "linux-arm64-gnu", os: "linux", cpu: "arm64", libc: "glibc", target: "aarch64-unknown-linux-gnu" },
  { suffix: "linux-x64-gnu", os: "linux", cpu: "x64", libc: "glibc", target: "x86_64-unknown-linux-gnu" },
  { suffix: "win32-arm64-msvc", os: "win32", cpu: "arm64", target: "aarch64-pc-windows-msvc" },
  { suffix: "win32-x64-msvc", os: "win32", cpu: "x64", target: "x86_64-pc-windows-msvc" },
].map((row) => Object.freeze({ ...row, name: `@neralabs/rerail-engine-${row.suffix}`, engine: `rerail_engine.${row.suffix}.node` }));

const BY_NAME = new Map(ENGINE_PACKAGES.map((enginePackage) => [enginePackage.name, enginePackage]));
const SYSTEMS = { darwin: "macOS", linux: "Linux with glibc", win32: "Windows" };
const INTEGRITY = /^sha512-[A-Za-z0-9+/]{86}==$/u;
/** A bare file name: a manifest names its tarballs beside itself, never by a path. */
const FILE_NAME = /^[^\\/]+$/u;

/** Files of the repository every engine package carries: the engine links the third-party code the notices cover. */
const NOTICES = ["LICENSE", "NOTICE", "THIRD_PARTY_NOTICES"];

function readJson(path) {
  return JSON.parse(readFileSync(path, "utf8"));
}

/** The lockfile location npm installs `name` at as a dependency of core. */
export const engineLocation = (name) => `node_modules/${name}`;

/**
 * An engine package's package.json at core's version (`core` is core's package.json). `os`, `cpu` and `libc` let npm
 * skip the package on another platform; `repository` must be this repository's, or npm refuses the provenance. It has
 * no scripts, dependencies or `exports`: the runtime loads the engine by its file name.
 */
export function enginePackageJson(enginePackage, core) {
  return {
    name: enginePackage.name,
    version: core.version,
    license: core.license,
    repository: core.repository,
    os: [enginePackage.os],
    cpu: [enginePackage.cpu],
    ...(enginePackage.libc === undefined ? {} : { libc: [enginePackage.libc] }),
    engines: { node: core.engines.node },
    publishConfig: { access: "public" },
  };
}

/** An engine package's README.md: what it is, and that core is the package to install. */
export function enginePackageReadme(enginePackage, core) {
  return [
    `# ${enginePackage.name}`,
    "",
    `The native engine of [Rerail](${core.homepage}) for ${SYSTEMS[enginePackage.os]} on ${enginePackage.cpu}.`,
    "",
    `[\`${core.name}\`](https://www.npmjs.com/package/${core.name}) depends on this package at its own version and loads the engine from it. Install \`${core.name}\`, not this package: it holds the engine and nothing to import.`,
    "",
  ].join("\n");
}

/** A file's integrity as npm's registry and lockfiles give it: `sha512-` and the digest in base64. */
export function fileIntegrity(path) {
  return `sha512-${createHash("sha512").update(readFileSync(path)).digest("base64")}`;
}

/** Packs `enginePackage` from `native`'s engine into `out`, through a folder it removes; returns its manifest entry. */
function packEnginePackage(enginePackage, { root, core, native, out }) {
  const folder = mkdtempSync(join(tmpdir(), "rerail-engine-package-"));
  try {
    copyFileSync(join(native, enginePackage.engine), join(folder, enginePackage.engine));
    writeFileSync(join(folder, "package.json"), `${JSON.stringify(enginePackageJson(enginePackage, core), null, 2)}\n`);
    writeFileSync(join(folder, "README.md"), enginePackageReadme(enginePackage, core));
    for (const notice of NOTICES) copyFileSync(join(root, notice), join(folder, notice));
    const [command, ...leading] = npmCommand();
    const pack = [...leading, "pack", "--json", "--ignore-scripts", "--pack-destination", out];
    const [{ filename }] = JSON.parse(execFileSync(command, pack, { cwd: folder, encoding: "utf8", stdio: ["ignore", "pipe", "inherit"] }));
    if (!existsSync(join(out, filename))) throw new Error(`npm pack named ${filename} for ${enginePackage.name}, and ${out} has no such file`);
    return { suffix: enginePackage.suffix, name: enginePackage.name, version: core.version, file: filename, integrity: fileIntegrity(join(out, filename)) };
  } finally {
    rmSync(folder, { recursive: true, force: true });
  }
}

/**
 * Packs an engine package for every engine `native` holds into `out`, and writes the manifest there. Returns the
 * manifest and the packages whose engine is absent (`absent`). Throws when `native` holds no engine, and, with `all`,
 * when it lacks any: a release packs every one.
 */
export function packEnginePackages({ root = ROOT, native = join(root, "native"), out, all = false }) {
  const absent = ENGINE_PACKAGES.filter((enginePackage) => !existsSync(join(native, enginePackage.engine)));
  if (absent.length === ENGINE_PACKAGES.length) throw new Error(`${native} holds no engine: npm run build:native builds this machine's, and npm run engine:pull fetches a released one`);
  if (all && absent.length > 0) throw new Error(`${native} lacks ${absent.map(({ engine }) => engine).join(", ")}: a release packs all ${ENGINE_PACKAGES.length} engines`);
  const core = readJson(join(root, "package.json"));
  mkdirSync(out, { recursive: true });
  // A manifest from an earlier run describes tarballs this run replaces.
  rmSync(join(out, ENGINE_PACKAGES_FILE), { force: true });
  const packages = ENGINE_PACKAGES.filter((enginePackage) => !absent.includes(enginePackage)).map((enginePackage) => packEnginePackage(enginePackage, { root, core, native, out }));
  const manifest = { version: core.version, packages };
  writeFileSync(join(out, ENGINE_PACKAGES_FILE), `${JSON.stringify(manifest, null, 2)}\n`);
  return { manifest, absent };
}

const isText = (value, pattern) => typeof value === "string" && pattern.test(value);

/** Why `entry` is not the entry `pack` writes for an engine package at `version`, or undefined. */
function manifestEntryProblem(entry, version) {
  const enginePackage = BY_NAME.get(entry?.name);
  if (enginePackage === undefined) return `${entry?.name} is not an engine package`;
  const problems = [
    [entry.suffix !== enginePackage.suffix, `${entry.name}'s suffix is ${entry.suffix}, not ${enginePackage.suffix}`],
    [entry.version !== version, `${entry.name} is ${entry.version}, not ${version}`],
    [!isText(entry.file, FILE_NAME), `${entry.name} names no file in the folder`],
    [!isText(entry.integrity, INTEGRITY), `${entry.name} has no sha512 integrity`],
  ];
  return problems.find(([refused]) => refused)?.[1];
}

/** Why `manifest` is not one `pack` writes, or undefined: each entry an engine package's, once, at the manifest's version. */
function engineManifestProblem(manifest) {
  if (typeof manifest?.version !== "string" || !Array.isArray(manifest.packages)) return "it has no version or no packages";
  const names = manifest.packages.map((entry) => entry?.name);
  const twice = names.find((name, index) => names.indexOf(name) !== index);
  if (twice !== undefined) return `it lists ${twice} twice`;
  return manifest.packages.map((entry) => manifestEntryProblem(entry, manifest.version)).find((problem) => problem !== undefined);
}

/** The manifest `pack` wrote in `folder`. Throws when the folder has none, or one `pack` does not write. */
export function readEngineManifest(folder) {
  const path = join(folder, ENGINE_PACKAGES_FILE);
  if (!existsSync(path)) throw new Error(`${folder} has no ${ENGINE_PACKAGES_FILE}: node scripts/release/engine/packages.mjs pack --out ${folder} writes it`);
  const manifest = readJson(path);
  const problem = engineManifestProblem(manifest);
  if (problem !== undefined) throw new Error(`${path}: ${problem}`);
  return manifest;
}

/** The engine packages `manifest` lacks. A release's lacks none. */
export const absentEnginePackages = (manifest) => ENGINE_PACKAGES.filter(({ name }) => !manifest.packages.some((entry) => entry.name === name));

/**
 * Why a tarball `manifest` lists is not in `folder` as packed, or undefined. The release publishes these files and pins
 * the manifest's integrity in core, so a file that is absent or hashes to another value would publish an engine core
 * does not name, and npm skips an optional dependency whose integrity differs, with exit code 0.
 */
export function engineTarballsProblem(folder, manifest) {
  for (const { name, file, integrity } of manifest.packages) {
    const path = join(folder, file);
    if (!existsSync(path)) return `${folder} has no ${file}, which its ${ENGINE_PACKAGES_FILE} lists for ${name}`;
    const actual = fileIntegrity(path);
    if (actual !== integrity) return `${path} is not the tarball ${ENGINE_PACKAGES_FILE} lists for ${name} (${actual}, not ${integrity})`;
  }
  return undefined;
}

/** Core's `optionalDependencies` on the engine packages `manifest` lists: each at its exact version, no range. */
export function enginePins(manifest) {
  return Object.fromEntries(manifest.packages.map(({ name, version }) => [name, version]));
}

/**
 * The shrinkwrap entry of the engine package `entry` (a manifest entry). It has no `resolved`: with one, npm asks that
 * URL's registry for the tarball instead of the registry the `@neralabs` scope is mapped to, and a mirror or a local
 * registry then installs no engine. `os`, `cpu` and `libc` are what npm filters on; it does not read them from the
 * package's own package.json for a shrinkwrapped dependency.
 */
export function engineLockEntry({ name, version, integrity }) {
  const { os, cpu, libc } = BY_NAME.get(name);
  return { version, integrity, cpu: [cpu], ...(libc === undefined ? {} : { libc: [libc] }), optional: true, os: [os] };
}

/** `packageJson` without its pins on engine packages, as the repository keeps it. The same object when it has none. */
export function withoutEnginePins(packageJson) {
  const pinned = Object.keys(packageJson.optionalDependencies ?? {}).filter((name) => BY_NAME.has(name));
  if (pinned.length === 0) return packageJson;
  const { optionalDependencies, ...rest } = packageJson;
  const others = Object.entries(optionalDependencies).filter(([name]) => !BY_NAME.has(name));
  if (others.length === 0) return rest;
  return Object.fromEntries(Object.entries(packageJson).map(([key, value]) => [key, key === "optionalDependencies" ? Object.fromEntries(others) : value]));
}

/**
 * `packageJson` pinning the engine packages `manifest` lists as `optionalDependencies`, placed after `dependencies`.
 * The repository's package.json never has them: `npm ci` in a checkout would ask the registry for packages the next
 * release has yet to publish. So packing stamps them in and takes them out again (shrinkwrap-pack.mjs).
 */
export function withEnginePins(packageJson, manifest) {
  const unpinned = withoutEnginePins(packageJson);
  if (manifest.packages.length === 0) return unpinned;
  const optionalDependencies = { ...unpinned.optionalDependencies, ...enginePins(manifest) };
  // Where package.json already has the key it stays, so taking the pins out gives the file back as it was.
  if (Object.hasOwn(unpinned, "optionalDependencies") || !Object.hasOwn(unpinned, "dependencies")) return { ...unpinned, optionalDependencies };
  return Object.fromEntries(Object.entries(unpinned).flatMap(([key, value]) => (key === "dependencies" ? [[key, value], ["optionalDependencies", optionalDependencies]] : [[key, value]])));
}

const USAGE = "usage: engine/packages.mjs pack --out <folder> [--native <folder>] [--all]";

function parseArguments(argv) {
  const { values, positionals } = parseArgs({ args: argv, allowPositionals: true, options: { out: { type: "string" }, native: { type: "string" }, all: { type: "boolean", default: false } } });
  if (positionals.join(" ") !== "pack" || values.out === undefined) throw new Error("it takes pack and --out <folder>");
  return { all: values.all, out: resolve(values.out), ...(values.native === undefined ? {} : { native: resolve(values.native) }) };
}

if (process.argv[1] !== undefined && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  let options;
  try {
    options = parseArguments(process.argv.slice(2));
  } catch (error) {
    process.stderr.write(`${USAGE}\n${error.message}\n`);
    process.exit(2);
  }
  const { manifest, absent } = packEnginePackages(options);
  for (const { name, version, file } of manifest.packages) console.log(`${name}@${version}: ${join(options.out, file)}`);
  console.log(`${join(options.out, ENGINE_PACKAGES_FILE)} lists ${manifest.packages.length} of ${ENGINE_PACKAGES.length} engine packages.`);
  if (absent.length > 0) {
    console.warn(`Not packed, for want of an engine in ${options.native ?? join(ROOT, "native")}: ${absent.map(({ name }) => name).join(", ")}. A core packed against ${options.out} installs with an engine on ${manifest.packages.map(({ suffix }) => suffix).join(", ")} only, and the release's check refuses it.`);
  }
}
