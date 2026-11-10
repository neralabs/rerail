/**
 * A `PackStore` over this repository's `packs/`, for set-up tests that must not reach npm: fetching gives each pack's
 * directory as it is, and installing links it into the test's Rerail home, as `rerail packs add <path>` would. The
 * npm store itself is exercised against the registry by `integration/installed-packs.test.ts`.
 */
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { defaultRerailHome, rulePackPackageName } from "../../src/engine/index.js";
import type { PackStore, PackVersion } from "../../src/setup/index.js";
import { fileSpecDirectory, installPackDirectory, uninstallPack } from "./installed-packs.js";
import { repositoryPath } from "./oracles.js";

export interface DirectoryPackStore extends PackStore {
  /** The npm specs each `install` call was given, in order. */
  readonly installs: string[][];
}

/** This repository's packs at the versions their `package.json` names: what a confirm answer's `--packs` carries. */
export function repositoryPackVersions(...ids: string[]): PackVersion[] {
  return ids.map(id => ({
    packageName: rulePackPackageName(id),
    id,
    version: (JSON.parse(readFileSync(repositoryPath("packs", id, "package.json"), "utf8")) as { version: string }).version,
  }));
}

/**
 * `packsRoot`'s pack directories (this repository's `packs/` by default) as the packs npm would give; with `only`,
 * just those, as if no other pack were published. Installs into `home`: the test's Rerail home when it is made.
 */
export function directoryPackStore(packsRoot: string = repositoryPath("packs"), only?: readonly string[], home: string = defaultRerailHome()): DirectoryPackStore {
  const installs: string[][] = [];
  const versionOf = (id: string): string | undefined => {
    const path = join(packsRoot, id, "package.json");
    return existsSync(path) && (only === undefined || only.includes(id)) ? (JSON.parse(readFileSync(path, "utf8")) as { version: string }).version : undefined;
  };
  return {
    home,
    installs,
    async fetch(ids) {
      return {
        fetched: ids.filter(id => versionOf(id) !== undefined).map(id => ({ id, directory: join(packsRoot, id) })),
        problems: ids.filter(id => versionOf(id) === undefined && only === undefined).map(id => ({ packageName: rulePackPackageName(id), reason: "npm error 404 Not Found" })),
        // The packs are this repository's own directories: nothing to remove.
        release: async () => {},
      };
    },
    async install(specs) {
      installs.push([...specs]);
      for (const spec of specs) {
        const match = /^((?:@[a-z0-9][a-z0-9._-]*\/)?rerail-pack-([a-z][a-z0-9-]*))@(.+)$/u.exec(spec);
        if (match === null) throw new Error(`directoryPackStore installs <package>@<version>, not ${spec}`);
        const [, packageName, id, version] = match as unknown as [string, string, string, string];
        // What npm recorded for a pack added from a folder, whoever publishes it: put back as it was.
        const folder = fileSpecDirectory(spec);
        if (folder !== undefined) {
          installPackDirectory(folder, packageName, home);
          continue;
        }
        // `packsRoot` holds Rerail's own packs: npm has no version of another package there.
        if (packageName !== rulePackPackageName(id) || versionOf(id) !== version) throw new Error(`npm error code ETARGET\nnpm error notarget No matching version found for ${spec}.`);
        installPackDirectory(join(packsRoot, id), packageName, home);
      }
    },
    async remove(packageNames) {
      for (const packageName of packageNames) uninstallPack(packageName, home);
    },
  };
}
