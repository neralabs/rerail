import { mkdirSync, mkdtempSync, realpathSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { parsePacksFile, readPacksFile, repositoryPackDirectory } from "../../../src/engine/catalog/packs-file/packs-file.js";
import { linkFolder, symlinkOrSkip } from "../../support/platform/links.js";
import { removeFolderSync } from "../../support/platform/remove-folder.js";

const roots: string[] = [];
afterEach(() => {
  for (const root of roots.splice(0)) removeFolderSync(root);
});

function checkout(): string {
  const root = realpathSync(mkdtempSync(join(tmpdir(), "rerail-packs-file-")));
  roots.push(root);
  return root;
}

const file = (packs: unknown, extra: Record<string, unknown> = {}): string => JSON.stringify({ format: "rerail.packs/v1", packs, ...extra });

describe(".rerail/packs.json", () => {
  it("reads a Rerail pack by id, any pack by package name, and a folder in the repository, in the file's order", () => {
    expect(parsePacksFile(file([{ id: "complexity", version: "0.1.8" }, { path: "packs/acme" }, { id: "@acme/rerail-pack-sql", version: "1.0.0-next.2" }]))).toEqual([
      { kind: "published", packageName: "@neralabs/rerail-pack-complexity", id: "complexity", version: "0.1.8" },
      { kind: "repository", path: "packs/acme" },
      { kind: "published", packageName: "@acme/rerail-pack-sql", id: "sql", version: "1.0.0-next.2" },
    ]);
  });

  it.each([
    [file([{ id: "complexity", version: "0.1.8" }, { id: "@neralabs/rerail-pack-complexity", version: "0.1.9" }]), "lists @neralabs/rerail-pack-complexity twice"],
    [file([{ id: "complexity", version: "latest" }]), "packs[0].version must be one exact version"],
    [file([{ id: "complexity" }]), "packs[0] must have \"id\" and \"version\" for a published pack, or only \"path\""],
    [file([{ path: "packs/acme", version: "1.0.0" }]), "packs[0] must have \"id\" and \"version\""],
    [file([{ id: "Not A Pack", version: "1.0.0" }]), "packs[0].id \"Not A Pack\" is neither a pack id"],
    [file([{ path: "/etc" }]), "packs[0].path \"/etc\" is absolute"],
    [file([{ path: "packs/./acme" }]), "must name a folder inside the repository"],
    [file([{ path: "packs\\acme" }]), "uses \\"],
    [file([], { rules: [] }), "is not a rerail.packs/v1 document"],
    ["{\"format\": \"rerail.packs/v1\", \"format\": \"rerail.packs/v1\", \"packs\": []}", "format"],
  ])("refuses %s", (text, message) => {
    expect(() => parsePacksFile(text)).toThrow(message);
  });

  it("is absent without the file, and never read through a symbolic link", context => {
    const root = checkout();
    expect(readPacksFile(root)).toBeUndefined();
    const elsewhere = checkout();
    writeFileSync(join(elsewhere, "packs.json"), file([]));
    mkdirSync(join(root, ".rerail"));
    symlinkOrSkip(context, join(elsewhere, "packs.json"), join(root, ".rerail", "packs.json"), "file");
    expect(() => readPacksFile(root)).toThrow(".rerail/packs.json is not a regular file");
  });

  it("finds a pack folder inside the checkout, and refuses one reached through a symbolic link or missing", () => {
    const root = checkout();
    mkdirSync(join(root, "packs", "acme"), { recursive: true });
    expect(repositoryPackDirectory(root, { kind: "repository", path: "packs/acme" })).toBe(join(root, "packs", "acme"));
    const outside = checkout();
    linkFolder(outside, join(root, "linked"));
    expect(() => repositoryPackDirectory(root, { kind: "repository", path: "linked" })).toThrow("whose path goes through a symbolic link");
    expect(() => repositoryPackDirectory(root, { kind: "repository", path: "packs/gone" })).toThrow("which is not in the repository");
  });

  it("refuses a pack folder whose rules are reached through a symbolic link, as a scan at a ref would not read them", () => {
    const root = checkout();
    mkdirSync(join(root, "packs", "acme"), { recursive: true });
    const outside = checkout();
    linkFolder(outside, join(root, "packs", "acme", "rules"));
    expect(() => repositoryPackDirectory(root, { kind: "repository", path: "packs/acme" })).toThrow("packs/acme, which holds a symbolic link at rules");
  });
});
