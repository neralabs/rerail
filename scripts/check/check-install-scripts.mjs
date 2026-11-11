#!/usr/bin/env node
// Every dependency that ships its addon prebuilt has its install scripts denied in package.json's allowScripts, keyed
// to the version installed. Such a package (better-sqlite3 13) has a binding.gyp, a prebuilds folder its loader reads
// first, and "gypfile": false, which tells npm not to build it. package-lock.json drops gypfile and npm installs from
// the lock entry, so `npm ci` runs `node-gyp rebuild` for the package anyway: without Python and C++ build tools the
// install fails, and with them it compiles an addon nothing loads. npm 11.16 and later skip the scripts allowScripts
// denies (`"name@version": false`). A deny keyed to the version stops matching at the next bump, so the bump fails
// here until someone confirms the new version still ships its prebuilds and moves the key.
//
//   node scripts/check/check-install-scripts.mjs   (npm run check:install-scripts; part of npm run check)
import { existsSync, readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");

/** What npm installed at a lockfile location under `root`: its package.json, binding.gyp and prebuilds folder. */
export function installedOnDisk(root) {
  return location => {
    const manifest = join(root, location, "package.json");
    if (!existsSync(manifest)) return undefined;
    return {
      manifest: JSON.parse(readFileSync(manifest, "utf8")),
      bindingGyp: existsSync(join(root, location, "binding.gyp")),
      prebuilds: existsSync(join(root, location, "prebuilds")),
    };
  };
}

const NODE_MODULES = "node_modules/";

/** Whether a lockfile entry is a package npm installs in a node_modules folder, not a link to a workspace. */
const installedPackage = ([location, entry]) => location.startsWith(NODE_MODULES) && entry.link !== true;

/** Whether what npm installed (`installedOnDisk`'s answer) ships its addon prebuilt and tells npm not to build it. */
const shipsPrebuilt = own => own !== undefined && own.manifest.gypfile === false && own.bindingGyp && own.prebuilds;

/** Each installed package, name and version once, that ships its addon prebuilt and tells npm not to build it. */
export function installedAddons(lock, installed = installedOnDisk(ROOT)) {
  const addons = new Map();
  for (const [location, entry] of Object.entries(lock.packages ?? {}).filter(installedPackage)) {
    if (!shipsPrebuilt(installed(location))) continue;
    const name = location.slice(location.lastIndexOf(NODE_MODULES) + NODE_MODULES.length);
    addons.set(`${name}@${entry.version}`, { name, version: entry.version });
  }
  return [...addons.values()];
}

/** Why `allowScripts` (package.json's) lets npm build a prebuilt addon, a line per package; empty when it does not. */
export function installScriptProblems(allowScripts, lock, installed = installedOnDisk(ROOT)) {
  return installedAddons(lock, installed).flatMap(({ name, version }) => {
    const key = `${name}@${version}`;
    const deny = `"${key}": false`;
    if (allowScripts[name] === true || allowScripts[key] === true) {
      return [`allowScripts approves ${key}'s install scripts, so npm runs node-gyp rebuild for its binding.gyp though it ships its addons prebuilt: replace the approval with ${deny}`];
    }
    if (allowScripts[key] !== false) {
      return [`allowScripts does not deny ${key}'s install scripts, so npm 11 runs node-gyp rebuild for its binding.gyp though it ships its addons prebuilt: check that this version still ships prebuilds/, then add ${deny}`];
    }
    return [];
  });
}

if (process.argv[1] !== undefined && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  const manifest = JSON.parse(readFileSync(join(ROOT, "package.json"), "utf8"));
  const lock = JSON.parse(readFileSync(join(ROOT, "package-lock.json"), "utf8"));
  const problems = installScriptProblems(manifest.allowScripts ?? {}, lock);
  for (const problem of problems) process.stderr.write(`package.json: ${problem}\n`);
  if (problems.length > 0) process.exit(1);
  const denied = installedAddons(lock).map(({ name, version }) => `${name}@${version}`);
  console.log(`allowScripts denies the install scripts of every prebuilt addon installed: ${denied.join(", ") || "none"}`);
}
