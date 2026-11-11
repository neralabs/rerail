/**
 * The react pack: eslint-plugin-react's and eslint-plugin-react-hooks' answers on the cases of their own rule tests,
 * recorded by packs/react/upstream/<tool>/generate.mjs. Both generators take the pins
 * common.mjs holds (PINS: the npm packages they refuse any other version of; UPSTREAM: the repository and commit whose
 * test files they copy, read here from common.mjs so a pin bump there needs no edit here) from a tools directory laid
 * out as `.rerail/tools/react`: the npm install, and the tool's upstream checkout as `upstream-<name>` beside it.
 */
import { readFileSync } from "node:fs";
import { symlink } from "node:fs/promises";
import { join } from "node:path";

import type { DeriveContext, DeriveRecipe } from "../context.js";

interface Upstream {
  repository: string;
  commit: string;
  /** The directory name the generator looks for under the tools directory. */
  checkout: string;
}

/** common.mjs's `export const NAME = { … };` object literal (strings and nested objects of strings only). */
function commonConstant<T>(root: string, name: string): T {
  const source = readFileSync(join(root, "packs", "react", "upstream", "common.mjs"), "utf8");
  const match = new RegExp(`^export const ${name} = (\\{[\\s\\S]*?\\n\\});$`, "mu").exec(source);
  if (match === null) throw new Error(`packs/react/upstream/common.mjs declares no ${name}`);
  return new Function(`return (${match[1]});`)() as T;
}

/** The upstream paths each generator reads: the copied rule tests and the licence file. */
const SPARSE: Record<string, readonly string[]> = {
  "eslint-plugin-react": ["/tests/lib/rules/", "/LICENSE"],
  "eslint-plugin-react-hooks": ["/packages/eslint-plugin-react-hooks/__tests__/", "/LICENSE"],
};

function recipe(tool: string): DeriveRecipe {
  return {
    pack: "react",
    tool,
    async derive({ root, toolDirectory, exec, checkout, npmTools, scratch }: DeriveContext) {
      const upstream = commonConstant<Record<string, Upstream>>(root, "UPSTREAM")[tool];
      const sparse = SPARSE[tool];
      if (upstream === undefined || sparse === undefined) throw new Error(`packs/react/upstream/common.mjs pins no upstream for ${tool}`);
      const installed = await npmTools("react", { dependencies: commonConstant<Record<string, string>>(root, "PINS") });
      const source = await checkout(upstream.repository, upstream.commit, { sparse });
      const tools = await scratch("tools");
      await symlink(join(installed, "package.json"), join(tools, "package.json"));
      await symlink(join(installed, "node_modules"), join(tools, "node_modules"));
      await symlink(source, join(tools, upstream.checkout));
      await exec(["node", join(toolDirectory, "generate.mjs"), "--tools", tools]);
    },
  };
}

export const recipes: DeriveRecipe[] = [recipe("eslint-plugin-react"), recipe("eslint-plugin-react-hooks")];
