/**
 * The next pack: @next/eslint-plugin-next's answers on the cases of its own rule tests, recorded by
 * packs/next/upstream/eslint-plugin-next/generate.mjs (every mode: tests and fixtures, then the provenance) with
 * the npm packages its PINS name and vercel/next.js at its UPSTREAM_COMMIT, sparse: the rule tests it copies and the
 * licence file. Both are read from the generator, which refuses any other version or commit.
 */
import { readFileSync } from "node:fs";
import { join } from "node:path";

import type { DeriveRecipe } from "../context.js";

/** The generator's string constants (`const NAME = "value";`), by name. */
function stringConstants(source: string): Record<string, string> {
  return Object.fromEntries([...source.matchAll(/^const (\w+) = ("[^"\n]*");$/gmu)].map(([, name, value]) => [name!, JSON.parse(value!) as string]));
}

/** The generator's `const PINS = { … };`, its references to string constants resolved. */
function pins(source: string): Record<string, string> {
  const match = /^const PINS = (\{[^}]*\});$/mu.exec(source);
  if (match === null) throw new Error("generate.mjs declares no PINS");
  const constants = stringConstants(source);
  return new Function(...Object.keys(constants), `return (${match[1]});`)(...Object.values(constants)) as Record<string, string>;
}

export const recipes: DeriveRecipe[] = [
  {
    pack: "next",
    tool: "eslint-plugin-next",
    async derive({ toolDirectory, exec, checkout, npmTools }) {
      const generator = join(toolDirectory, "generate.mjs");
      const source = readFileSync(generator, "utf8");
      const { UPSTREAM_REPOSITORY, UPSTREAM_COMMIT, UPSTREAM_TESTS } = stringConstants(source);
      if (UPSTREAM_REPOSITORY === undefined || UPSTREAM_COMMIT === undefined || UPSTREAM_TESTS === undefined) throw new Error("generate.mjs declares no UPSTREAM_REPOSITORY, UPSTREAM_COMMIT or UPSTREAM_TESTS");
      const tools = await npmTools("next", { dependencies: pins(source) });
      const upstream = await checkout(UPSTREAM_REPOSITORY, UPSTREAM_COMMIT, { sparse: [`/${UPSTREAM_TESTS}/`, "/license.md"] });
      await exec(["node", generator, "--tools", tools, "--upstream", upstream]);
    },
  },
];
