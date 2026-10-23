/**
 * The vue pack: eslint-plugin-vue's findings on its own RuleTester cases (`generate.mjs fixtures`, which copies the
 * cases from the plugin's checkout) and on the pack's corpora (`generate.mjs corpus`), with the tools
 * fixtures/vue/tools.mjs pins.
 */
import { readFileSync } from "node:fs";
import { join } from "node:path";

import { TOOLS } from "../../../fixtures/vue/tools.mjs";
import type { DeriveRecipe } from "../context.js";

/** A string constant of the generator (`const NAME = "value";`), so a pin bump there needs no edit here. */
function constant(source: string, name: string): string {
  const match = new RegExp(`^const ${name} = "([^"]+)";$`, "mu").exec(source);
  if (match === null) throw new Error(`generate.mjs declares no ${name}`);
  return match[1]!;
}

export const recipes: DeriveRecipe[] = [
  {
    pack: "vue",
    tool: "eslint-plugin-vue",
    async derive({ toolDirectory, exec, checkout, npmTools, corpora }) {
      const generator = join(toolDirectory, "generate.mjs");
      const commit = constant(readFileSync(generator, "utf8"), "UPSTREAM_COMMIT");
      const tools = await npmTools("vue", { dependencies: TOOLS });
      const plugin = await checkout("https://github.com/vuejs/eslint-plugin-vue", commit);
      await exec(["node", generator, "fixtures", plugin, "--tools", tools]);
      await exec(["node", generator, "corpus", "--tools", tools, "--corpora", await corpora()]);
    },
  },
];
