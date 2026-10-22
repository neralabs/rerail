/** The complexity pack: lizard's scores on Rerail's own corpus (fixtures/, authored), recorded by generate.mjs. */
import type { DeriveRecipe } from "../context.js";

export const recipes: DeriveRecipe[] = [
  {
    pack: "complexity",
    tool: "lizard",
    inputs: [],
    runtimes: ["uv"],
    async derive({ exec, toolDirectory }) {
      // generate.mjs pins lizard==1.24.0 and runs it with uvx.
      await exec(["node", `${toolDirectory}/generate.mjs`]);
    },
  },
];
