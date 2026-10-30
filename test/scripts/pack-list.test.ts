// scripts/generate/packs/pack-list.ts: the pack list set-up fetches from npm names every published pack and leaves out
// a private one, which npm never publishes.

import { mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { readPackList } from "../../scripts/generate/packs/pack-list.js";

describe("readPackList", () => {
  it("lists published packs by id and leaves out a private one", () => {
    const directory = mkdtempSync(join(tmpdir(), "pack-list-"));
    for (const [id, extra] of [["open", {}], ["closed", { private: true }]] as const) {
      mkdirSync(join(directory, id));
      writeFileSync(join(directory, id, "package.json"), JSON.stringify({ name: `@neralabs/rerail-pack-${id}`, version: "0.1.0", ...extra }));
    }

    expect(readPackList(directory)).toEqual([{ id: "open", packageName: "@neralabs/rerail-pack-open" }]);
  });
});
