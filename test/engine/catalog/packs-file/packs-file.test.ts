import { mkdirSync, mkdtempSync, realpathSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { checkPackFolders, parsePacksFile, readPacksFile, repositoryPackDirectory } from "../../../../src/engine/catalog/packs-file/packs-file.js";
import { linkFolder, symlinkOrSkip } from "../../../support/platform/links.js";
import { removeFolderSync } from "../../../support/platform/remove-folder.js";
import { repositoryPath } from "../../../support/oracles.js";

const roots: string[] = [];
afterEach(() => {
  for (const root of roots.splice(0)) removeFolderSync(root);
});

function checkout(): string {
  const root = realpathSync(mkdtempSync(join(tmpdir(), "rerail-packs-file-")));
  roots.push(root);
  return root;
}

const file = (packs: unknown, extra: Record<string, unknown> = {}): string => JSON.stringify({ schema: "rerail.packs/1", packs, ...extra });

describe(".rerail/packs.json", () => {
  it("reads a Rerail pack by id, any pack by package name, and a folder in the repository, in the file's order", () => {
    expect(parsePacksFile(file([{ id: "complexity", version: "0.1.8" }, { path: "packs/acme" }, { id: "@acme/rerail-pack-sql", version: "1.0.0-next.2" }]))).toEqual([
      { kind: "published", packageName: "@neralabs/rerail-pack-complexity", id: "complexity", version: "0.1.8" },
      { kind: "repository", path: "packs/acme" },
      { kind: "published", packageName: "@acme/rerail-pack-sql", id: "sql", version: "1.0.0-next.2" },
    ]);
  });

  it("reads this repository's own .rerail/packs.json, as main commits it, as packs for the whole repository", () => {
    // The file already in use in this and other repositories; a change that refuses it breaks them.
    expect(readPacksFile(repositoryPath())).toEqual([
      { kind: "published", packageName: "@neralabs/rerail-pack-complexity", id: "complexity", version: "0.1.8" },
      { kind: "repository", path: "packs/rerail" },
    ]);
  });

  it("keeps a pack to the folders it names, on a published pack or a pack in the repository", () => {
    expect(parsePacksFile(file([{ id: "typescript", version: "0.1.8", folders: ["web", "admin/app"] }, { path: "packs/acme", folders: ["api"] }]))).toEqual([
      { kind: "published", packageName: "@neralabs/rerail-pack-typescript", id: "typescript", version: "0.1.8", folders: ["web", "admin/app"] },
      { kind: "repository", path: "packs/acme", folders: ["api"] },
    ]);
  });

  it("reads the option values an entry sets for its pack's rules, beside its folders", () => {
    expect(parsePacksFile(file([{ id: "complexity", version: "0.1.9", config: { ceiling: 10 } }, { path: "packs/acme", folders: ["api"], config: { "max-depth": 3 } }]))).toEqual([
      { kind: "published", packageName: "@neralabs/rerail-pack-complexity", id: "complexity", version: "0.1.9", config: { ceiling: 10 } },
      { kind: "repository", path: "packs/acme", folders: ["api"], config: { "max-depth": 3 } },
    ]);
  });

  it.each([
    [file([{ id: "complexity", version: "0.1.8" }, { id: "@neralabs/rerail-pack-complexity", version: "0.1.9" }]), "lists @neralabs/rerail-pack-complexity twice"],
    [file([{ id: "complexity", version: "latest" }]), "packs[0].version must be one exact version"],
    [file([{ id: "complexity" }]), "packs[0] must have \"id\" and \"version\" for a published pack, or \"path\""],
    [file([{ path: "packs/acme", version: "1.0.0" }]), "packs[0] must have \"id\" and \"version\""],
    [file([{ id: "Not A Pack", version: "1.0.0" }]), "packs[0].id \"Not A Pack\" is neither a pack id"],
    [file([{ path: "/etc" }]), "packs[0].path \"/etc\" is absolute"],
    [file([{ path: "packs/./acme" }]), "must name a folder inside the repository"],
    [file([{ path: "packs\\acme" }]), "uses \\"],
    [file([{ path: "packs/acme", folders: [] }]), "packs[0].folders must be a list of at least one folder"],
    [file([{ path: "packs/acme", folders: "web" }]), "packs[0].folders must be a list of at least one folder"],
    [file([{ id: "complexity", version: "0.1.8", folders: ["web/../legacy"] }]), "packs[0].folders[0] \"web/../legacy\" must name a folder inside the repository"],
    [file([{ id: "complexity", version: "0.1.8", folders: ["/srv"] }]), "packs[0].folders[0] \"/srv\" is absolute"],
    [file([{ id: "complexity", version: "0.1.8", folders: [3] }]), "packs[0].folders[0] is not a string"],
    [file([{ id: "complexity", version: "0.1.8", folders: ["web", "web"] }]), "packs[0].folders lists web twice"],
    [file([{ id: "complexity", version: "0.1.8", config: 10 }]), "packs[0].config must be an object of option names and their values"],
    [file([{ id: "complexity", version: "0.1.8", config: [10] }]), "packs[0].config must be an object of option names and their values"],
    [file([{ id: "complexity", version: "0.1.8", config: {} }]), "packs[0].config must be an object of option names and their values"],
    [file([], { rules: [] }), "is not a rerail.packs/1 document"],
    [JSON.stringify({ format: "rerail.packs/v1", packs: [] }), "is not a rerail.packs/1 document"],
    ["{\"schema\": \"rerail.packs/1\", \"schema\": \"rerail.packs/1\", \"packs\": []}", "schema"],
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

  it("checks a pack's folders as it checks a pack's path: in the checkout, a folder, reached through no symbolic link", () => {
    const root = checkout();
    mkdirSync(join(root, "web"));
    writeFileSync(join(root, "README.md"), "");
    linkFolder(checkout(), join(root, "linked"));
    const pack = (folders: string[]) => ({ kind: "published" as const, packageName: "@neralabs/rerail-pack-typescript", id: "typescript", version: "0.1.8", folders });
    expect(() => checkPackFolders(root, pack(["web"]))).not.toThrow();
    expect(() => checkPackFolders(root, pack(["web", "admin"]))).toThrow("the folder admin for @neralabs/rerail-pack-typescript@0.1.8, which is not in the repository");
    expect(() => checkPackFolders(root, pack(["README.md"]))).toThrow("which is not a folder");
    expect(() => checkPackFolders(root, pack(["linked"]))).toThrow("whose path goes through a symbolic link");
  });
});
