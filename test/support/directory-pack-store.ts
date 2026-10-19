/**
 * A `PackStore` over this repository's `packs/`, for set-up tests that must not reach npm: fetching gives each pack's
 * directory as it is, and installing links it into the test's Rerail home, as `rerail packs add <path>` would. The
 * npm store itself is exercised against the registry by `integration/installed-packs.test.ts`.
 */
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { rulePackPackageName } from "../../src/engine/index.js";
import type { PackStore } from "../../src/setup/index.js";
import { installPackDirectory, uninstallPack } from "./installed-packs.js";
import { repositoryPath } from "./oracles.js";

export interface DirectoryPackStore extends PackStore {
  /** The npm specs each `install` call was given, in order. */
  readonly installs: string[][];
}

/** This repository's packs at the versions their `package.json` names: what a confirm answer's `--packs` carries. */
export function repositoryPackVersions(...ids: string[]): { id: string; version: string }[] {
  return ids.map(id => ({ id, version: (JSON.parse(readFileSync(repositoryPath("packs", id, "package.json"), "utf8")) as { version: string }).version }));
}

/**
 * `packsRoot`'s pack directories (this repository's `packs/` by default) as the packs npm would give; with `only`,
 * just those, as if no other pack were published.
 */
export function directoryPackStore(packsRoot: string = repositoryPath("packs"), only?: readonly string[]): DirectoryPackStore {
  const installs: string[][] = [];
  const versionOf = (id: string): string | undefined => {
    const path = join(packsRoot, id, "package.json");
    return existsSync(path) && (only === undefined || only.includes(id)) ? (JSON.parse(readFileSync(path, "utf8")) as { version: string }).version : undefined;
  };
  return {
    installs,
    async fetch(ids) {
      return {
        fetched: ids.filter(id => versionOf(id) !== undefined).map(id => ({ id, directory: join(packsRoot, id) })),
        problems: ids.filter(id => versionOf(id) === undefined && only === undefined).map(id => ({ packageName: rulePackPackageName(id), reason: "npm error 404 Not Found" })),
      };
    },
    async install(specs) {
      installs.push([...specs]);
      for (const spec of specs) {
        const match = /^@neralabs\/rerail-pack-([a-z][a-z0-9-]*)@(.+)$/u.exec(spec);
        if (match === null) throw new Error(`directoryPackStore installs <package>@<version>, not ${spec}`);
        const [, id, version] = match;
        if (versionOf(id!) !== version) throw new Error(`npm error 404 ${spec} is not in the registry`);
        installPackDirectory(join(packsRoot, id!), id!);
      }
    },
    async remove(ids) {
      for (const id of ids) uninstallPack(id);
    },
  };
}
