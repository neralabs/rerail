/**
 * The complexity pack: lizard's scores on Rerail's own corpus (fixtures/, authored), recorded by generate.mjs with the
 * lizard of the runtime its gates.yaml names under scripts/pack-data/runtimes/.
 */
import { join } from "node:path";

import { readGatesManifest } from "../../../conformance/languages/manifest.js";
import { uvRuntime } from "../runtimes.js";
import type { DeriveRecipe } from "../context.js";

export const recipes: DeriveRecipe[] = [
  {
    pack: "complexity",
    tool: "lizard",
    async derive({ root, pack, tool, exec, toolDirectory, runtimeCache, runner }) {
      const generator = join(toolDirectory, "generate.mjs");
      const { upstream } = readGatesManifest(root, pack, tool);
      const lizard = join((await uvRuntime(upstream.runtime, runtimeCache, runner)).directory, ".venv", "bin", "lizard");
      await exec(["node", generator, lizard]);
    },
  },
];
