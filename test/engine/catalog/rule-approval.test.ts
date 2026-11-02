import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { approve, isApproved, packSubject, readRuleCatalog } from "../../../src/engine/index.js";
import { thirdPartyPack } from "../../support/installed-packs.js";
import { temporaryDatabase, temporaryDirectory } from "../../support/rule-configuration-builders.js";

describe("a pack's approval", () => {
  it("waits again when what the pack says of itself changes, and holds again once it is back as approved", async () => {
    const db = await temporaryDatabase();
    const folder = thirdPartyPack(await temporaryDirectory(), "@acme/rerail-pack-sql", "sql");
    const packJson = join(folder, "pack.json");
    const original = readFileSync(packJson, "utf8");
    const subject = () => {
      const catalog = readRuleCatalog([folder]);
      return packSubject(catalog, catalog.packs.get("sql")!)!;
    };
    approve(db, subject());

    // The name a steer's note and the dashboard show, and the lines under it, are part of what a person approved.
    for (const field of ["name", "covers", "reason"] as const) {
      writeFileSync(packJson, JSON.stringify({ ...(JSON.parse(original) as Record<string, unknown>), [field]: "Rerail's own, reviewed" }));
      expect(isApproved(db, subject()), field).toBe(false);
    }
    writeFileSync(packJson, original);
    expect(isApproved(db, subject())).toBe(true);
  });
});
