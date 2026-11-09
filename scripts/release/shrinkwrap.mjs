// The npm-shrinkwrap.json the @neralabs/rerail tarball ships (docs/release.md, "Shrinkwrap"). npm honors a published
// package's shrinkwrap when it installs it, so every install gets the dependency versions and integrity hashes this
// repository's package-lock.json pinned, instead of resolving them fresh. shrinkwrap-pack.mjs writes it at pack time
// and checks the packed one; the checkout keeps package-lock.json as its only lockfile.
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { isDeepStrictEqual } from "node:util";

export const SHRINKWRAP_FILE = "npm-shrinkwrap.json";

/** Flags npm computes for the whole workspace tree; the shrinkwrap sets `optional` again for the shipped tree alone. */
const TREE_FLAGS = new Set(["dev", "devOptional", "optional", "peer"]);

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

/**
 * The shrinkwrap for `manifest` (package.json) from `lock` (package-lock.json): the root and the packages its
 * dependencies reach, with no development dependency, no workspace pack, and `optional` marking a package only
 * optional dependencies reach. Throws when the lockfile is not package.json's.
 */
export function shrinkwrapFrom(manifest, lock) {
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
  const required = reachable(lock.packages, shipped, optional => !optional);
  const all = reachable(lock.packages, shipped, () => true);
  const packages = { "": shipped };
  for (const [location, entry] of Object.entries(lock.packages)) {
    if (!all.has(location)) continue;
    const kept = Object.fromEntries(Object.entries(entry).filter(([key]) => !TREE_FLAGS.has(key)));
    packages[location] = required.has(location) ? kept : { ...kept, optional: true };
  }
  return { name: manifest.name, version: manifest.version, lockfileVersion: 3, requires: true, packages };
}

function readJson(path) {
  return JSON.parse(readFileSync(path, "utf8"));
}

/** The shrinkwrap `root`'s package.json and package-lock.json give, as the file's text. */
export function shrinkwrapText(root) {
  const shrinkwrap = shrinkwrapFrom(readJson(join(root, "package.json")), readJson(join(root, "package-lock.json")));
  return `${JSON.stringify(shrinkwrap, null, 2)}\n`;
}

/**
 * Why `packed` (the tarball's shrinkwrap text, undefined when it has none) is not `expected`, or undefined when it is.
 * Names the package locations that differ, so a stale lockfile points at what changed.
 */
export function shrinkwrapProblem(packed, expected) {
  if (packed === undefined) return `it has no ${SHRINKWRAP_FILE}; npm pack writes one as prepack (scripts/release/shrinkwrap-pack.mjs write)`;
  let actual;
  try {
    actual = JSON.parse(packed);
  } catch (error) {
    return `its ${SHRINKWRAP_FILE} is not JSON: ${error.message}`;
  }
  const wanted = JSON.parse(expected);
  if (isDeepStrictEqual(actual, wanted)) return undefined;
  const locations = new Set([...Object.keys(actual.packages ?? {}), ...Object.keys(wanted.packages ?? {})]);
  const differing = [...locations].filter(location => !isDeepStrictEqual(actual.packages?.[location], wanted.packages?.[location]));
  const named = differing.length === 0 ? "its header" : differing.slice(0, 10).map(location => location === "" ? "the root" : location).join(", ");
  return `its ${SHRINKWRAP_FILE} is not what package-lock.json gives (${named}${differing.length > 10 ? `, and ${differing.length - 10} more` : ""})`;
}
