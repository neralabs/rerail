/**
 * Installs this repository's packs into the test file's Rerail home the way `rerail packs add <path>` does: npm links a
 * local directory into the prefix's `node_modules` and records it in the prefix's `package.json` as `file:<path>`.
 * Core ships no packs, so a test that needs one installs it.
 */
import { cpSync, existsSync, mkdirSync, readFileSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { defaultRerailHome, installedPacksPrefix, rulePackPackageOf } from "../../src/engine/index.js";
import type { PackStore } from "../../src/setup/index.js";
import { fixturesPath, repositoryPath } from "./oracles.js";

/** The package `name` names, as `rerail packs add` reads it: a pack id is Rerail's own package, a package name itself. */
function packageNamed(name: string): string {
  const packageName = rulePackPackageOf(name);
  if (packageName === undefined) throw new Error(`${name} is neither a pack id nor a pack's package name`);
  return packageName;
}

/** Sets or, with `spec` undefined, removes `packageName`'s entry in the prefix's `package.json` dependencies. */
function record(home: string, packageName: string, spec: string | undefined): void {
  const path = join(installedPacksPrefix(home), "package.json");
  const manifest = existsSync(path) ? JSON.parse(readFileSync(path, "utf8")) as { dependencies?: Record<string, string> } : {};
  const dependencies = { ...manifest.dependencies };
  if (spec === undefined) delete dependencies[packageName];
  else dependencies[packageName] = spec;
  writeFileSync(path, JSON.stringify({ ...manifest, dependencies }, null, 2));
}

/**
 * Links `directory` into `home`'s packs as package `name`: a pack id is Rerail's own `@neralabs/rerail-pack-<id>`, and a
 * package name (`rerail-pack-<id>`, `@<scope>/rerail-pack-<id>`) is itself. Returns the installed path.
 */
export function installPackDirectory(directory: string, name: string, home: string = defaultRerailHome()): string {
  const packageName = packageNamed(name);
  const installed = join(installedPacksPrefix(home), "node_modules", packageName);
  mkdirSync(dirname(installed), { recursive: true });
  rmSync(installed, { recursive: true, force: true });
  symlinkSync(directory, installed, process.platform === "win32" ? "junction" : "dir");
  record(home, packageName, `file:${directory}`);
  return installed;
}

/** Installs `packs/<id>` from this repository into `home` (the test file's Rerail home by default). */
export function installPack(id: string, home: string = defaultRerailHome()): string {
  return installPackDirectory(repositoryPath("packs", id), id, home);
}

/** Removes an installed pack, a pack id naming Rerail's own package, as `rerail packs remove <package>` does. */
export function uninstallPack(name: string, home: string = defaultRerailHome()): void {
  const packageName = packageNamed(name);
  rmSync(join(installedPacksPrefix(home), "node_modules", packageName), { recursive: true, force: true });
  if (existsSync(join(installedPacksPrefix(home), "package.json"))) record(home, packageName, undefined);
}

/** Removes every pack installed in `home`: a test that installs packs leaves the next one a home without them. */
export function removeInstalledPacks(home: string = defaultRerailHome()): void {
  rmSync(installedPacksPrefix(home), { recursive: true, force: true });
}

/** The directory a `<package>@file:<directory>` spec names, or undefined for a spec of a version. */
export function fileSpecDirectory(spec: string): string | undefined {
  const at = spec.indexOf("@file:");
  return at === -1 ? undefined : spec.slice(at + "@file:".length);
}

/**
 * A copy, under `root`, of the valid fixture pack as a third party publishes it: package `packageName`, pack id `id`,
 * each rule id `<id>-<rule>` (so it can clash with another pack only by its pack id), at `version` and, with `range`,
 * for those versions of core. Returns its folder.
 */
export function thirdPartyPack(root: string, packageName: string, id: string, version = "1.0.0", range?: string): string {
  const copy = join(root, packageName, version);
  cpSync(fixturesPath("packs", "valid"), copy, { recursive: true, filter: source => !source.includes("upstream") });
  const pack = JSON.parse(readFileSync(join(copy, "pack.json"), "utf8")) as { rules: { id: string }[] };
  const rules = pack.rules.map(rule => {
    const document = JSON.parse(readFileSync(join(copy, "rules", `${rule.id}.json`), "utf8")) as Record<string, unknown>;
    rmSync(join(copy, "rules", `${rule.id}.json`));
    writeFileSync(join(copy, "rules", `${id}-${rule.id}.json`), JSON.stringify({ ...document, id: `${id}-${rule.id}` }));
    return { ...rule, id: `${id}-${rule.id}`, document: `rules/${id}-${rule.id}.json` };
  });
  writeFileSync(join(copy, "pack.json"), JSON.stringify({ ...pack, id, name: `${packageName}'s ${id}`, rules }));
  const manifest = JSON.parse(readFileSync(join(copy, "package.json"), "utf8")) as Record<string, unknown>;
  const peerDependencies = range === undefined ? {} : { peerDependencies: { "@neralabs/rerail": range } };
  writeFileSync(join(copy, "package.json"), JSON.stringify({ ...manifest, name: packageName, version, repository: undefined, ...peerDependencies }));
  return copy;
}

/**
 * A `PackStore` on `home` that installs `<package>@<version>` by linking `folders[package]`, and a recorded
 * `<package>@file:<folder>` spec from that folder, as npm does; it records each install's and removal's arguments.
 */
export function linkingPackStore(home: string, folders: Readonly<Record<string, string>>): PackStore & { installs: string[][]; removes: string[][] } {
  const installs: string[][] = [];
  const removes: string[][] = [];
  return {
    home,
    installs,
    removes,
    fetch: async () => ({ fetched: [], problems: [], release: async () => {} }),
    async install(specs) {
      installs.push([...specs]);
      for (const spec of specs) {
        const name = spec.slice(0, spec.indexOf("@", 1));
        const folder = fileSpecDirectory(spec) ?? folders[name];
        if (folder === undefined) throw new Error(`npm error code E404\nnpm error 404 Not Found - ${name}`);
        installPackDirectory(folder, name, home);
      }
    },
    async remove(packageNames) {
      removes.push([...packageNames]);
      for (const packageName of packageNames) uninstallPack(packageName, home);
    },
  };
}
