/**
 * `npm run fixtures:derive` while derive and bumps are paused (docs/upstream.md, "Derive and bumps are paused"): every
 * tool-pack refuses before anything is removed or any generator runs.
 */
import assert from "node:assert/strict";
import { test, vi } from "vitest";

import type { DeriveRecipe } from "../../../scripts/pack-data/context.js";
import { deriveToolPack, type DeriveOptions } from "../../../scripts/pack-data/derive.js";
import { DERIVE_PAUSED, DerivePausedError } from "../../../scripts/pack-data/paused.js";
import { RECIPES } from "../../../scripts/pack-data/recipes/index.js";

test("the pause message points at the documented pause", () => {
  assert.match(DERIVE_PAUSED, /^derive and bumps are paused until the generators write per-repo provenance; see docs\/upstream\.md#derive-and-bumps-are-paused$/u);
});

test("every tool-pack's derive refuses with the pause message, removing nothing and running no generator", async () => {
  assert.ok(RECIPES.length > 0);
  for (const { pack, tool } of RECIPES) {
    const generator = vi.fn<DeriveRecipe["derive"]>(async () => {});
    const log = vi.fn<DeriveOptions["log"]>();
    const options: DeriveOptions = { cacheRoot: "/nonexistent/cache", runtimeCache: "/nonexistent/runtimes", update: true, log };
    const refusal = await deriveToolPack({ pack, tool, derive: generator }, options).then(
      () => assert.fail(`${pack}/${tool} derived while paused`),
      (error: unknown) => error,
    );
    assert.ok(refusal instanceof DerivePausedError, `${pack}/${tool}: ${String(refusal)}`);
    assert.equal(refusal.message, `packs/${pack}/upstream/${tool}: ${DERIVE_PAUSED}`);
    assert.equal(generator.mock.calls.length, 0);
    // Removing the stale derived files is the first thing a derivation logs.
    assert.equal(log.mock.calls.length, 0);
  }
});
