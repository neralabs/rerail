/**
 * Installs packs into the test file's Rerail home the way npm does: it links the pack's directory into the prefix's
 * `node_modules` and records a spec in the prefix's `package.json`. `installPack` stands for an install from the npm
 * registry (the pack's version is recorded, so Nera shipped its rules and they run); `installPackDirectory` stands for
 * `rerail packs add <path>` by default (recorded as `file:<path>`, so its rules wait for a person's approval).
 * Core ships no packs, so a test that needs one installs it.
 */
import { existsSync, mkdirSync, readFileSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { defaultRerailHome, installedPacksPrefix, rulePackPackageName } from "../../src/engine/index.js";
import { repositoryPath } from "./oracles.js";

/** Sets or, with `spec` undefined, removes `id`'s entry in the prefix's `package.json` dependencies. */
function record(home: string, id: string, spec: string | undefined): void {
  const path = join(installedPacksPrefix(home), "package.json");
  const manifest = existsSync(path) ? JSON.parse(readFileSync(path, "utf8")) as { dependencies?: Record<string, string> } : {};
  const dependencies = { ...manifest.dependencies };
  if (spec === undefined) delete dependencies[rulePackPackageName(id)];
  else dependencies[rulePackPackageName(id)] = spec;
  writeFileSync(path, JSON.stringify({ ...manifest, dependencies }, null, 2));
}

/**
 * Links `directory` into `home`'s packs as `@neralabs/rerail-pack-<name>` and records `spec` for it: `file:<directory>`
 * by default, as `rerail packs add <directory>` does; a version stands for an install from the registry. Returns the
 * installed path.
 */
export function installPackDirectory(directory: string, name: string, home: string = defaultRerailHome(), spec: string = `file:${directory}`): string {
  const scope = join(installedPacksPrefix(home), "node_modules", "@neralabs");
  mkdirSync(scope, { recursive: true });
  const installed = join(scope, `rerail-pack-${name}`);
  rmSync(installed, { recursive: true, force: true });
  symlinkSync(directory, installed, process.platform === "win32" ? "junction" : "dir");
  record(home, name, spec);
  return installed;
}

/** The version `directory`'s `package.json` names. */
export function packVersion(directory: string): string {
  const manifest: { version: string } = JSON.parse(readFileSync(join(directory, "package.json"), "utf8"));
  return manifest.version;
}

/**
 * Installs `packs/<id>` from this repository into `home` (the test file's Rerail home by default) as an install from
 * the npm registry: it records the pack's version, so its rules run as Nera shipped them.
 */
export function installPack(id: string, home: string = defaultRerailHome()): string {
  const directory = repositoryPath("packs", id);
  return installPackDirectory(directory, id, home, packVersion(directory));
}

/** Removes an installed pack, as `rerail packs remove <id>` does. */
export function uninstallPack(id: string, home: string = defaultRerailHome()): void {
  rmSync(join(installedPacksPrefix(home), "node_modules", "@neralabs", `rerail-pack-${id}`), { recursive: true, force: true });
  if (existsSync(join(installedPacksPrefix(home), "package.json"))) record(home, id, undefined);
}

/** Removes every pack installed in `home`: a test that installs packs leaves the next one a home without them. */
export function removeInstalledPacks(home: string = defaultRerailHome()): void {
  rmSync(installedPacksPrefix(home), { recursive: true, force: true });
}

/** The directory a `@neralabs/rerail-pack-<id>@file:<directory>` spec names, or undefined for a spec of a version. */
export function fileSpecDirectory(spec: string): string | undefined {
  const at = spec.indexOf("@file:");
  return at === -1 ? undefined : spec.slice(at + "@file:".length);
}
