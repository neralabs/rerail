/**
 * The typescript pack's derivation recipes: typescript-eslint's and @vitest/eslint-plugin's own RuleTester cases and
 * their recorded findings, each recorded by its generator.
 */
import { readFileSync } from "node:fs";
import { join } from "node:path";

import { npmRuntime, RUNTIMES_DIRECTORY } from "../../../conformance/recorders/port-readiness/runtimes.js";
import type { DeriveRecipe } from "../context.js";

/** The object literal `const <name> = { … };` in `source`, as its `"key": "string"` pairs (generator pins). */
function constantPins(source: string, name: string): Record<string, string> {
  const literal = new RegExp(`^const ${name} = \\{([^}]*)\\};`, "mu").exec(source)?.[1];
  if (literal === undefined) throw new Error(`the generator declares no const ${name}`);
  return Object.fromEntries([...literal.matchAll(/"?([@\w./-]+)"?\s*:\s*"([^"]+)"/gu)].map((match) => [match[1]!, match[2]!]));
}

/** The `<key>: { repository: "…", commit: "…" }` (or `const <key> = { … }`) checkout pin in `source`. */
function checkoutPin(source: string, key: string): { repository: string; commit: string } {
  const pin = new RegExp(`${key}\\s*[:=]\\s*\\{\\s*repository:\\s*"([^"]+)",\\s*commit:\\s*"([0-9a-f]{40})"\\s*\\}`, "u").exec(source);
  if (pin === null) throw new Error(`the generator pins no ${key} checkout`);
  return { repository: pin[1]!, commit: pin[2]! };
}

export const recipes: DeriveRecipe[] = [
  {
    pack: "typescript",
    tool: "typescript-eslint",
    async derive({ root, exec, checkout, npmTools }) {
      // packs/typescript/upstream/generate.mjs records every typescript-eslint tool of the pack; it pins eslint,
      // @typescript-eslint/parser and eslint-plugin, and typescript (VERSIONS), and the checkout (CHECKOUTS). It needs
      // tsx beside them at no version of its own: the recorder's typescript-eslint 8.70.1 runtime's tsx loads the same
      // test files there.
      const generator = join(root, "packs", "typescript", "upstream", "generate.mjs");
      const source = readFileSync(generator, "utf8");
      const runtime = JSON.parse(readFileSync(join(RUNTIMES_DIRECTORY, "typescript-eslint-8.70.1", "package.json"), "utf8")) as { dependencies: Record<string, string> };
      const tools = await npmTools("typescript-eslint", { dependencies: { ...constantPins(source, "VERSIONS"), tsx: runtime.dependencies.tsx } });
      const upstream = checkoutPin(source, '"typescript-eslint"');
      // The rules' test files, and the rule sources they import (stubbed or swapped for the published build's).
      const typescriptEslint = await checkout(upstream.repository, upstream.commit, { sparse: ["/packages/eslint-plugin/"] });
      await exec(["node", generator, "--tools", tools, "--typescript-eslint", typescriptEslint]);
    },
  },
  {
    pack: "typescript",
    tool: "eslint-plugin-vitest",
    async derive({ toolDirectory, exec, checkout, recorderCache, runner }) {
      // generate.mjs's PINS are the recorder's vitest-eslint-plugin-1.6.27 runtime (it says so, and pinnedRequire
      // checks every version), so that runtime is the one source of truth. The checkout is whole: a test imports its
      // rule's source for the rule's name, and the stub the generator falls back to when the source is missing exports it
      // under `default`, which the test's default import of a CommonJS module without `__esModule` does not unwrap.
      const generator = join(toolDirectory, "generate.mjs");
      const tools = await npmRuntime("vitest-eslint-plugin-1.6.27", recorderCache, runner);
      const upstream = checkoutPin(readFileSync(generator, "utf8"), "CHECKOUT");
      const vitest = await checkout(upstream.repository, upstream.commit);
      await exec(["node", generator, "--tools", tools, "--vitest", vitest]);
    },
  },
];
