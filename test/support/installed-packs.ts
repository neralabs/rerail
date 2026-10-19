/**
 * Installs this repository's packs into the test file's Rerail home the way `rerail packs add <path>` does: npm links a
 * local directory into the prefix's `node_modules`. Core ships no packs, so a test that needs one installs it.
 */
import { mkdirSync, rmSync, symlinkSync } from "node:fs";
import { join } from "node:path";
import { defaultRerailHome, installedPacksPrefix } from "../../src/engine/index.js";
import { repositoryPath } from "./oracles.js";

/** Links `directory` into `home`'s packs as `@neralabs/rerail-pack-<name>`; returns the installed path. */
export function installPackDirectory(directory: string, name: string, home: string = defaultRerailHome()): string {
  const scope = join(installedPacksPrefix(home), "node_modules", "@neralabs");
  mkdirSync(scope, { recursive: true });
  const installed = join(scope, `rerail-pack-${name}`);
  rmSync(installed, { recursive: true, force: true });
  symlinkSync(directory, installed, process.platform === "win32" ? "junction" : "dir");
  return installed;
}

/** Installs `packs/<id>` from this repository into `home` (the test file's Rerail home by default). */
export function installPack(id: string, home: string = defaultRerailHome()): string {
  return installPackDirectory(repositoryPath("packs", id), id, home);
}

/** Removes an installed pack, as `rerail packs remove <id>` does. */
export function uninstallPack(id: string, home: string = defaultRerailHome()): void {
  rmSync(join(installedPacksPrefix(home), "node_modules", "@neralabs", `rerail-pack-${id}`), { recursive: true, force: true });
}

/** Removes every pack installed in `home`: a test that installs packs leaves the next one a home without them. */
export function removeInstalledPacks(home: string = defaultRerailHome()): void {
  rmSync(installedPacksPrefix(home), { recursive: true, force: true });
}
