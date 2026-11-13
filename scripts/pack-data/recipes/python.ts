/**
 * The python pack's derivation recipes: Ruff's and Bandit's generate.py (with packs/python/upstream/common.py), run on
 * the interpreter of scripts/pack-data/runtimes/python (CPython 3.14), with the tool from the runtime its gates.yaml
 * names, over upstream's checkout at the generator's ORIGIN.
 */
import { readFileSync } from "node:fs";
import { join } from "node:path";

import { readGatesManifest } from "../../../conformance/languages/manifest.js";
import { uvRuntime } from "../runtimes.js";
import type { DeriveContext, DeriveRecipe } from "../context.js";

/** The generator's ORIGIN (`<repository>@<commit>`), read from it so a pin bump needs no recipe edit. */
function generatorOrigin(generator: string): { repository: string; commit: string } {
  const origin = /^ORIGIN = "([^"@]+)@([0-9a-f]{40})"$/mu.exec(readFileSync(generator, "utf8"));
  if (origin === null) throw new Error(`${generator}: no ORIGIN = "<repository>@<commit>"`);
  return { repository: origin[1]!, commit: origin[2]! };
}

/** Runs the tool's generate.py in its `fixtures` mode on upstream's `sparse` paths. */
async function derivePython(context: DeriveContext, sparse: readonly string[]): Promise<void> {
  const { root, pack, tool, toolDirectory, runtimeCache, runner } = context;
  const generator = join(toolDirectory, "generate.py");
  const pins = generatorOrigin(generator);
  const interpreter = (await uvRuntime("python", runtimeCache, runner)).python;
  const { upstream } = readGatesManifest(root, pack, tool);
  const executable = join((await uvRuntime(upstream.runtime, runtimeCache, runner)).directory, ".venv", "bin", tool);
  const checkout = await context.checkout(pins.repository, pins.commit, { sparse });
  await context.exec([interpreter, generator, "fixtures", checkout, executable]);
}

export const recipes: DeriveRecipe[] = [
  {
    pack: "python",
    tool: "ruff",
    derive: (context) => derivePython(context, ["/crates/ruff_linter/resources/test/fixtures/"]),
  },
  {
    pack: "python",
    tool: "bandit",
    derive: (context) => derivePython(context, ["/examples/"]),
  },
];
