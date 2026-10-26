/**
 * The python pack's derivation recipes: Ruff's and Bandit's generate.py (with packs/python/upstream/common.py), run on
 * the interpreter of the recorder's runtimes/python (CPython 3.14), with the tool from the recorder's
 * runtimes/<tool>-<VERSION>, over upstream's checkout at the generator's ORIGIN.
 */
import { readFileSync } from "node:fs";
import { join } from "node:path";

import { uvRuntime } from "../../../conformance/recorders/port-readiness/runtimes.js";
import type { DeriveContext, DeriveRecipe } from "../context.js";

/** The generator's VERSION and ORIGIN (`<repository>@<commit>`), read from it so a pin bump needs no recipe edit. */
function generatorPins(generator: string): { version: string; repository: string; commit: string } {
  const text = readFileSync(generator, "utf8");
  const version = /^VERSION = "([^"]+)"$/mu.exec(text)?.[1];
  const origin = /^ORIGIN = "([^"@]+)@([0-9a-f]{40})"$/mu.exec(text);
  if (version === undefined || origin === null) throw new Error(`${generator}: no VERSION and ORIGIN = "<repository>@<commit>"`);
  return { version, repository: origin[1]!, commit: origin[2]! };
}

/** Runs the tool's generate.py in its `fixtures` mode on upstream's `sparse` paths. */
async function derivePython(context: DeriveContext, sparse: readonly string[]): Promise<void> {
  const { tool, toolDirectory, recorderCache, runner } = context;
  const generator = join(toolDirectory, "generate.py");
  const pins = generatorPins(generator);
  const interpreter = (await uvRuntime("python", recorderCache, runner)).python;
  const executable = join((await uvRuntime(`${tool}-${pins.version}`, recorderCache, runner)).directory, ".venv", "bin", tool);
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
