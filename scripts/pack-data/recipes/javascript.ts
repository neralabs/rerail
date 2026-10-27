/**
 * The javascript pack: ESLint core's and eslint-plugin-unicorn's answers on the cases of their own rule tests, recorded by
 * packs/javascript/upstream/generate.mjs (ESLint) and eslint-plugin-unicorn/generate.mjs. Each runs on the port readiness
 * recorder's npm runtime its gates.yaml names (runtimes/eslint-10.11.0, runtimes/eslint-plugin-unicorn-76.0.0: the
 * versions the generator's VERSIONS/PINS check) and on upstream's repository at the gates.yaml commit, whose test files
 * it reads; both generators refuse any other version or commit.
 */
import { join } from "node:path";

import { readGatesManifest } from "../../../conformance/languages/manifest.js";
import { npmRuntime } from "../../../conformance/recorders/port-readiness/runtimes.js";
import type { DeriveContext, DeriveRecipe } from "../context.js";

/** The tool-pack's recorder runtime and its upstream checkout, as its gates.yaml pins them. */
async function pinnedInputs({ root, pack, tool, recorderCache, runner, checkout }: DeriveContext): Promise<{ tools: string; upstream: string }> {
  const { upstream } = readGatesManifest(root, pack, tool);
  const tools = await npmRuntime(upstream.runtime, recorderCache, runner);
  return { tools, upstream: await checkout(upstream.repository, upstream.commit) };
}

export const recipes: DeriveRecipe[] = [
  {
    pack: "javascript",
    tool: "eslint",
    async derive(context) {
      const { tools, upstream } = await pinnedInputs(context);
      await context.exec(["node", join(context.root, "packs", "javascript", "upstream", "generate.mjs"), "--tools", tools, "--eslint", upstream]);
    },
  },
  {
    pack: "javascript",
    tool: "eslint-plugin-unicorn",
    async derive(context) {
      const { tools, upstream } = await pinnedInputs(context);
      await context.exec(["node", join(context.toolDirectory, "generate.mjs"), "--tools", tools, "--unicorn", upstream]);
    },
  },
];
