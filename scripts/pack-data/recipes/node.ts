/**
 * The node pack: eslint-plugin-n's answers on the cases of its own rule tests, recorded by
 * packs/node/upstream/eslint-plugin-n/generate.mjs with the npm packages its PINS name and eslint-community/eslint-plugin-n
 * at its UPSTREAM_COMMIT (sparse: the tests it copies and the fixtures they read, and the licence file), both read from
 * the generator, which refuses any other version or commit. It takes them from a tools directory laid out as
 * `.rerail/tools/node`: the npm install, and the checkout as `upstream-eslint-plugin-n` beside it.
 */
import { readFileSync } from "node:fs";
import { symlink } from "node:fs/promises";
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
    pack: "node",
    tool: "eslint-plugin-n",
    async derive({ toolDirectory, exec, checkout, npmTools, scratch }) {
      const generator = join(toolDirectory, "generate.mjs");
      const source = readFileSync(generator, "utf8");
      const { UPSTREAM_REPOSITORY, UPSTREAM_COMMIT } = stringConstants(source);
      if (UPSTREAM_REPOSITORY === undefined || UPSTREAM_COMMIT === undefined) throw new Error("generate.mjs declares no UPSTREAM_REPOSITORY or UPSTREAM_COMMIT");
      const installed = await npmTools("node", { dependencies: pins(source) });
      const upstream = await checkout(UPSTREAM_REPOSITORY, UPSTREAM_COMMIT, { sparse: ["/tests/", "/LICENSE"] });
      const tools = await scratch("tools");
      await symlink(join(installed, "package.json"), join(tools, "package.json"));
      await symlink(join(installed, "node_modules"), join(tools, "node_modules"));
      await symlink(upstream, join(tools, "upstream-eslint-plugin-n"));
      await exec(["node", generator, "--tools", tools]);
    },
  },
];
