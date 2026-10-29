/**
 * The complexity pack: lizard's scores on Rerail's own corpus (fixtures/, authored), recorded by generate.mjs with the
 * lizard of scripts/pack-data/runtimes/lizard-<VERSION>, the release the generator's VERSION names.
 */
import { readFileSync } from "node:fs";
import { join } from "node:path";

import { uvRuntime } from "../runtimes.js";
import type { DeriveRecipe } from "../context.js";

export const recipes: DeriveRecipe[] = [
  {
    pack: "complexity",
    tool: "lizard",
    async derive({ exec, toolDirectory, runtimeCache, runner }) {
      const generator = join(toolDirectory, "generate.mjs");
      const version = /^const VERSION = "([^"]+)";$/mu.exec(readFileSync(generator, "utf8"))?.[1];
      if (version === undefined) throw new Error(`${generator}: no const VERSION = "<release>";`);
      const lizard = join((await uvRuntime(`lizard-${version}`, runtimeCache, runner)).directory, ".venv", "bin", "lizard");
      await exec(["node", generator, lizard]);
    },
  },
];
