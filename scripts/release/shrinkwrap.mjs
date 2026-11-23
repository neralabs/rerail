// The npm-shrinkwrap.json the @neralabs/rerail tarball ships (docs/release.md, "Shrinkwrap"). npm honors a published
// package's shrinkwrap when it installs it, so every install gets the dependency versions and integrity hashes this
// repository's package-lock.json pinned, instead of resolving them fresh. shrinkwrap-pack.mjs writes it at pack time
// and checks the packed one; the checkout keeps package-lock.json as its only lockfile.
//
// The engine packages (engine/packages.mjs) are the one part package-lock.json cannot give: the release that packs core
// builds them, so no lockfile written before it holds their integrity. Their entries come from the manifest of the
// packed tarballs instead. npm installs a shrinkwrapped package's dependencies from its shrinkwrap alone, so an engine
// package the shrinkwrap lacks is never installed, whatever package.json says, and the install still exits with 0.
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { isDeepStrictEqual } from "node:util";
import { absentEnginePackages, ENGINE_PACKAGES, engineLocation, engineLockEntry, enginePins, withoutEnginePins } from "./engine/packages.mjs";

export const SHRINKWRAP_FILE = "npm-shrinkwrap.json";

/** Flags npm computes for the whole workspace tree; the shrinkwrap sets `optional` again for the shipped tree alone. */
const TREE_FLAGS = new Set(["dev", "devOptional", "optional", "peer"]);

/**
 * Set on every dependency's entry. npm runs `node-gyp rebuild` for a package with a `binding.gyp` and no install script
 * unless its package.json says `gypfile: false`, but it reads a shrinkwrapped dependency from its entry here, and the
 * entries npm writes never carry that field. Without this, `npx @neralabs/rerail setup` compiles better-sqlite3, which
 * ships prebuilt binaries and says `gypfile: false` itself, and fails where no compiler is installed (node:24-slim).
 * The managed install runs no install script, so every shipped dependency already works prebuilt and none needs a build.
 */
const NO_SOURCE_BUILD = { gypfile: false };

/** The lockfile location npm resolves `name` to from the package at `from`: its own node_modules, then each parent's. */
function resolveLocation(packages, from, name) {
  let base = from;
  for (;;) {
    const candidate = base === "" ? `node_modules/${name}` : `${base}/node_modules/${name}`;
    if (packages[candidate] !== undefined) return candidate;
    if (base === "") return undefined;
    const parent = base.lastIndexOf("/node_modules/");
    base = parent === -1 ? "" : base.slice(0, parent);
  }
}

/**
 * The dependencies of a lockfile entry as `[name, optional]`. An optional peer is left out: npm installs one only when
 * something else depends on it, and then that edge reaches it.
 */
function edgesOf(entry) {
  const peerMeta = entry.peerDependenciesMeta ?? {};
  return [
    ...Object.keys(entry.dependencies ?? {}).map(name => [name, false]),
    ...Object.keys(entry.optionalDependencies ?? {}).map(name => [name, true]),
    ...Object.keys(entry.peerDependencies ?? {}).filter(name => peerMeta[name]?.optional !== true).map(name => [name, false]),
  ];
}

/** Every location reachable from the root along `follow`'s edges, failing on a required dependency the lock lacks. */
function reachable(packages, root, follow) {
  const seen = new Set();
  const queue = [["", root]];
  while (queue.length > 0) {
    const [from, entry] = queue.shift();
    for (const [name, optional] of edgesOf(entry)) {
      if (!follow(optional)) continue;
      const location = resolveLocation(packages, from, name);
      if (location === undefined) {
        if (optional) continue;
        throw new Error(`package-lock.json resolves no ${name} for ${from === "" ? "the root" : from}: run npm install`);
      }
      if (seen.has(location)) continue;
      const target = packages[location];
      if (target.link === true) throw new Error(`${location} is a workspace link (${target.resolved}); a published dependency must come from a registry`);
      seen.add(location);
      queue.push([location, target]);
    }
  }
  return seen;
}

/** The lockfile's root entry as the package ships it; throws when the lockfile is not `manifest`'s. */
function shippedRoot(manifest, lock) {
  const root = lock.packages?.[""];
  if (!(lock.lockfileVersion >= 2) || root === undefined) throw new Error("package-lock.json must be lockfileVersion 2 or later");
  for (const field of ["dependencies", "optionalDependencies", "peerDependencies"]) {
    if (!isDeepStrictEqual(root[field] ?? {}, manifest[field] ?? {})) {
      throw new Error(`package-lock.json's root ${field} are not package.json's: run npm install`);
    }
  }
  const shipped = { ...root, name: manifest.name, version: manifest.version };
  delete shipped.devDependencies;
  delete shipped.workspaces;
  return shipped;
}

/** The lockfile's entries the root `shipped` reaches, as the shrinkwrap lists them, by location. */
function reachedEntries(lock, shipped) {
  const required = reachable(lock.packages, shipped, optional => !optional);
  const all = reachable(lock.packages, shipped, () => true);
  const entries = {};
  for (const location of Object.keys(lock.packages).filter(candidate => all.has(candidate))) {
    const kept = { ...Object.fromEntries(Object.entries(lock.packages[location]).filter(([key]) => !TREE_FLAGS.has(key))), ...NO_SOURCE_BUILD };
    entries[location] = required.has(location) ? kept : { ...kept, optional: true };
  }
  return entries;
}

/** The root `shipped` with a pin on each engine package of `engines`, and each one's entry, by location. */
function engineEntries(shipped, engines) {
  if (engines.packages.length === 0) return {};
  const entries = { "": { ...shipped, optionalDependencies: { ...shipped.optionalDependencies, ...enginePins(engines) } } };
  for (const entry of engines.packages) entries[engineLocation(entry.name)] = { ...engineLockEntry(entry), ...NO_SOURCE_BUILD };
  return entries;
}

/**
 * The shrinkwrap for `manifest` (package.json, without pins on engine packages) from `lock` (package-lock.json): the
 * root and the packages its dependencies reach, with no development dependency, no workspace pack, and `optional`
 * marking a package only optional dependencies reach. Throws when the lockfile is not package.json's. `engines` (the
 * manifest of the packed engine packages) adds each one it lists: an optional dependency of the root, and its entry.
 */
export function shrinkwrapFrom(manifest, lock, engines = { packages: [] }) {
  const shipped = shippedRoot(manifest, lock);
  const packages = { "": shipped, ...reachedEntries(lock, shipped), ...engineEntries(shipped, engines) };
  return { name: manifest.name, version: manifest.version, lockfileVersion: 3, requires: true, packages };
}

function readJson(path) {
  return JSON.parse(readFileSync(path, "utf8"));
}

/**
 * The shrinkwrap `root`'s package.json and package-lock.json give, as the file's text, with the engine packages
 * `engines` lists (none when undefined). Pins a pack stamped into package.json are left out before the lockfile is
 * compared with it, so a pack that failed before restoring them does not break the next one.
 */
export function shrinkwrapText(root, engines) {
  const shrinkwrap = shrinkwrapFrom(withoutEnginePins(readJson(join(root, "package.json"))), readJson(join(root, "package-lock.json")), engines);
  return `${JSON.stringify(shrinkwrap, null, 2)}\n`;
}

/** The packed shrinkwrap parsed, or why it can't be: `{ shrinkwrap }` or `{ problem }`. */
function parsePacked(packed) {
  if (packed === undefined) return { problem: `it has no ${SHRINKWRAP_FILE}; npm pack writes one as prepack (scripts/release/shrinkwrap-pack.mjs write)` };
  try {
    return { shrinkwrap: JSON.parse(packed) };
  } catch (error) {
    return { problem: `its ${SHRINKWRAP_FILE} is not JSON: ${error.message}` };
  }
}

/**
 * Why `packed` (the tarball's shrinkwrap text, undefined when it has none) is not `expected`, or undefined when it is.
 * Names the package locations that differ, so a stale lockfile points at what changed.
 */
export function shrinkwrapProblem(packed, expected) {
  const { shrinkwrap: actual, problem } = parsePacked(packed);
  if (problem !== undefined) return problem;
  const wanted = JSON.parse(expected);
  if (isDeepStrictEqual(actual, wanted)) return undefined;
  const locations = new Set([...Object.keys(actual.packages ?? {}), ...Object.keys(wanted.packages ?? {})]);
  const differing = [...locations].filter(location => !isDeepStrictEqual(actual.packages?.[location], wanted.packages?.[location]));
  const named = differing.length === 0 ? "its header" : differing.slice(0, 10).map(location => location === "" ? "the root" : location).join(", ");
  return `its ${SHRINKWRAP_FILE} is not what package-lock.json gives (${named}${differing.length > 10 ? `, and ${differing.length - 10} more` : ""})`;
}

/** The name of the package at lockfile location `location`. */
const packageAt = (location) => location.slice(location.lastIndexOf("node_modules/") + "node_modules/".length);

/**
 * Why a packed core (`shrinkwrap` and `packageJson`, parsed) does not pin every engine package as `engines` (the
 * manifest of the packed engine packages) gives them, or undefined. It expects all of `ENGINE_PACKAGES`, whatever the
 * manifest and the writer made of it, so a core short of an engine never passes:
 *
 * - package.json's `optionalDependencies`, and the shrinkwrap root's, are the engine packages at core's version, every
 *   one and nothing else;
 * - each has its entry, exactly: core's version, the manifest's integrity, the `os`, `cpu` and `libc` of its name,
 *   `optional`, and no `resolved` (`engineLockEntry`);
 * - no other entry is `optional`. The managed install passes `--omit=optional` and names its own engine package
 *   (src/setup/updater.ts), so a package core needs must never be one npm leaves out with them.
 */
export function enginePinsProblem(shrinkwrap, packageJson, engines) {
  const { version } = packageJson;
  const all = `the ${ENGINE_PACKAGES.length} engine packages at ${version}`;
  const names = (entries) => entries.map(({ name }) => name).join(", ");
  const absent = absentEnginePackages(engines);
  const pins = enginePins(engines);
  const packages = { "": {}, ...shrinkwrap.packages };
  const wrong = engines.packages.filter((entry) => !isDeepStrictEqual(packages[engineLocation(entry.name)], { ...engineLockEntry(entry), ...NO_SOURCE_BUILD }));
  const pinned = new Set(engines.packages.map(({ name }) => engineLocation(name)));
  const isOther = (location) => packages[location].optional === true || Object.hasOwn(pins, packageAt(location));
  const others = Object.keys(packages).filter((location) => !pinned.has(location) && isOther(location));
  const problems = [
    [engines.version !== version, `it is ${version}, and the engine packages are ${engines.version}`],
    [absent.length > 0, `the engine packages lack ${names(absent)}: a release packs all ${ENGINE_PACKAGES.length}`],
    [!isDeepStrictEqual(packageJson.optionalDependencies, pins), `its package.json's optionalDependencies are not ${all}`],
    [!isDeepStrictEqual(packages[""].optionalDependencies, pins), `its ${SHRINKWRAP_FILE}'s root optionalDependencies are not ${all}`],
    [wrong.length > 0, `its ${SHRINKWRAP_FILE} does not pin ${names(wrong)} as packed: version, integrity, os, cpu, libc, optional, and no resolved`],
    [others.length > 0, `its ${SHRINKWRAP_FILE} has optional packages besides the engine packages (${others.join(", ")}): the managed install omits optional dependencies, so nothing core needs may be one`],
  ];
  return problems.find(([found]) => found)?.[1];
}

/**
 * Why a tarball is not one a release publishes, or undefined: `packed` is its `shrinkwrap` and `packageJson` texts
 * (undefined when it has none), `expected` the shrinkwrap text the checkout gives with `engines`.
 */
export function packedProblem(packed, expected, engines) {
  const { shrinkwrap, problem } = parsePacked(packed.shrinkwrap);
  if (problem !== undefined) return problem;
  if (packed.packageJson === undefined) return "it has no package.json";
  return enginePinsProblem(shrinkwrap, JSON.parse(packed.packageJson), engines) ?? shrinkwrapProblem(packed.shrinkwrap, expected);
}
