import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

import { gatedToolPacks } from "../../../conformance/languages/manifest.js";
import { needsPhp } from "../../../scripts/upstream/tool-pack-needs.js";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..", "..");

describe("upstream bump: a tool-pack's needs", () => {
  it("sets PHP up for exactly the php and blade tool-packs, from their runtimes", () => {
    const needing = gatedToolPacks(ROOT).filter(({ pack, tool }) => needsPhp(ROOT, pack, tool)).map(({ pack }) => pack);
    expect(new Set(needing)).toEqual(new Set(["php", "blade"]));
    expect(needing.length).toBeGreaterThanOrEqual(6);
  });
});
