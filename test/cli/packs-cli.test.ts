import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { packAddSpec, runPacksCommand } from "../../src/cli/packs-cli.js";
import type { PackStore } from "../../src/setup/index.js";

describe("rerail packs add", () => {
  it("installs a pack by id, at a version, range or tag, or by its package name", () => {
    expect(packAddSpec("typescript")).toBe("@neralabs/rerail-pack-typescript");
    expect(packAddSpec("typescript@0.1.1")).toBe("@neralabs/rerail-pack-typescript@0.1.1");
    expect(packAddSpec("typescript@^0.1.0")).toBe("@neralabs/rerail-pack-typescript@^0.1.0");
    expect(packAddSpec("typescript@next")).toBe("@neralabs/rerail-pack-typescript@next");
    expect(packAddSpec("@neralabs/rerail-pack-node@0.1.2")).toBe("@neralabs/rerail-pack-node@0.1.2");
  });

  it("installs a folder or tarball that exists, by its absolute path", () => {
    const folder = mkdtempSync(join(tmpdir(), "rerail-pack-folder-"));
    try {
      expect(packAddSpec(folder)).toBe(folder);
    } finally {
      rmSync(folder, { recursive: true, force: true });
    }
  });

  it.each([
    ["an npm option, which could undo --ignore-scripts", "--ignore-scripts=false"],
    ["a short npm option", "-g"],
    ["another scope's package", "@someone/package"],
    ["a path that does not exist", "./no/such/pack"],
    ["an npm alias under a pack's name", "typescript@npm:left-pad@1.3.0"],
    ["a git spec under a pack's name", "typescript@github:someone/pack"],
    ["a URL under a pack's package name", "@neralabs/rerail-pack-typescript@https://example.invalid/pack.tgz"],
    ["a file spec under a pack's package name", "@neralabs/rerail-pack-typescript@file:../pack"],
  ])("refuses %s", (_case, argument) => {
    expect(() => packAddSpec(argument)).toThrow();
  });

  it("installs nothing when any argument is refused, the packs before it included", async () => {
    const installs: string[][] = [];
    const store: PackStore = {
      home: join(tmpdir(), "rerail-packs-cli-unused"),
      fetch: async () => ({ fetched: [], problems: [], release: async () => {} }),
      install: async specs => { installs.push([...specs]); },
      remove: async () => {},
    };
    await expect(runPacksCommand(["add", "typescript", "--ignore-scripts=false"], () => "usage", store)).rejects.toThrow("not npm options");
    expect(installs).toEqual([]);
  });
});
