import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, realpathSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { basename, dirname, join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { packAddSpec, runPacksCommand } from "../../../src/cli/packs-cli.js";
import { listRepositories, openRerailDatabase, recordCheckout, setUpRepository } from "../../../src/engine/index.js";
import { setRepositoryStarted } from "../../../src/runtime/index.js";
import { installedPackagesOf, type PackStore } from "../../../src/setup/index.js";
import { attachedPacks } from "../../support/attached-packs.js";
import { directoryPackStore } from "../../support/directory-pack-store.js";
import { installPackDirectory, linkingPackStore, thirdPartyPack, withOlderTokenRule } from "../../support/installed-packs.js";
import { fixturesPath, repositoryPath } from "../../support/oracles.js";
import { removeFolderSync } from "../../support/platform/remove-folder.js";

const homes: string[] = [];
afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllEnvs();
  for (const home of homes.splice(0)) removeFolderSync(home);
});

/** A fresh Rerail home, removed after the test. */
function temporaryHome(): string {
  const home = mkdtempSync(join(tmpdir(), "rerail-packs-cli-"));
  homes.push(home);
  return home;
}

/** What `rerail packs <args>` prints on `store`. */
async function packsOutput(args: readonly string[], store: PackStore): Promise<string> {
  const output: string[] = [];
  vi.spyOn(process.stdout, "write").mockImplementation(chunk => {
    output.push(String(chunk));
    return true;
  });
  await runPacksCommand(args, () => "usage", store);
  vi.restoreAllMocks();
  return output.join("");
}

describe("rerail packs add", () => {
  it("installs a pack by id, at a version, range or tag, or by its package name", () => {
    expect(packAddSpec("typescript")).toBe("@neralabs/rerail-pack-typescript");
    expect(packAddSpec("typescript@0.1.1")).toBe("@neralabs/rerail-pack-typescript@0.1.1");
    expect(packAddSpec("typescript@^0.1.0")).toBe("@neralabs/rerail-pack-typescript@^0.1.0");
    expect(packAddSpec("typescript@next")).toBe("@neralabs/rerail-pack-typescript@next");
    expect(packAddSpec("@neralabs/rerail-pack-node@0.1.2")).toBe("@neralabs/rerail-pack-node@0.1.2");
  });

  it("installs a third party's pack by its package name, scoped or not, at a version, range or tag", () => {
    expect(packAddSpec("@acme/rerail-pack-sql")).toBe("@acme/rerail-pack-sql");
    expect(packAddSpec("@acme/rerail-pack-sql@1.0.0")).toBe("@acme/rerail-pack-sql@1.0.0");
    expect(packAddSpec("@acme.dev/rerail-pack-sql@^1.0.0")).toBe("@acme.dev/rerail-pack-sql@^1.0.0");
    // A pack's package name, not the id rerail-pack-lint of a pack of Rerail's.
    expect(packAddSpec("rerail-pack-lint")).toBe("rerail-pack-lint");
    expect(packAddSpec("rerail-pack-lint@next")).toBe("rerail-pack-lint@next");
  });

  it("installs a folder or tarball that exists, by its absolute path", () => {
    const folder = mkdtempSync(join(tmpdir(), "rerail-pack-folder-"));
    try {
      expect(packAddSpec(folder)).toBe(folder);
    } finally {
      removeFolderSync(folder);
    }
  });

  it.each([
    ["an npm option, which could undo --ignore-scripts", "--ignore-scripts=false"],
    ["a short npm option", "-g"],
    ["another scope's package that is not a pack", "@someone/package"],
    ["a package whose name does not end in a pack id", "@acme/rerail-pack-"],
    ["a package with capitals in its pack id", "rerail-pack-Lint"],
    ["a path that does not exist", "./no/such/pack"],
    ["an npm alias under a pack's name", "typescript@npm:left-pad@1.3.0"],
    ["a git spec under a pack's name", "typescript@github:someone/pack"],
    ["a URL under a pack's package name", "@neralabs/rerail-pack-typescript@https://example.invalid/pack.tgz"],
    ["a file spec under a pack's package name", "@neralabs/rerail-pack-typescript@file:../pack"],
    ["a git spec under a third party's package name", "@acme/rerail-pack-sql@github:acme/sql"],
    ["a URL under an unscoped package name", "rerail-pack-lint@https://example.invalid/pack.tgz"],
    ["a file spec under an unscoped package name", "rerail-pack-lint@file:../pack"],
    ["an npm alias under a third party's package name", "@acme/rerail-pack-sql@npm:left-pad@1.3.0"],
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

describe("rerail packs add in a checkout", () => {
  /** A Git checkout Rerail has recorded in `home`, set up (with no packs) unless `setUp` is false. */
  function recordedCheckout(home: string, setUp = true): { checkout: string; repositoryKey: string } {
    const checkout = realpathSync(mkdtempSync(join(tmpdir(), "rerail-packs-cli-checkout-")));
    homes.push(checkout);
    execFileSync("git", ["init", "-q"], { cwd: checkout });
    const database = openRerailDatabase(join(home, "rerail.sqlite"));
    try {
      const { repositoryKey } = recordCheckout(database, checkout);
      if (setUp) setUpRepository(database, repositoryKey, { packs: [] });
      return { checkout, repositoryKey };
    } finally {
      database.$client.close();
    }
  }

  /** The packs added to the repository of `checkout` (`attachedPacks`), and whether it is set up. */
  function addedPacks(home: string, checkout: string, repositoryKey: string): { packs: string[]; setUp: boolean } {
    const databasePath = join(home, "rerail.sqlite");
    const database = openRerailDatabase(databasePath, { readonly: true });
    try {
      const listed = listRepositories(database).find(repository => repository.repositoryKey === repositoryKey);
      return { packs: attachedPacks(checkout, databasePath), setUp: listed !== undefined && listed.configuredAt !== null };
    } finally {
      database.$client.close();
    }
  }

  /** What `rerail packs add <args>` prints run in `checkout`. */
  async function addIn(checkout: string, args: readonly string[], store: PackStore): Promise<string> {
    vi.spyOn(process, "cwd").mockReturnValue(checkout);
    return await packsOutput(["add", ...args], store);
  }

  it("adds a pack installed from a folder to the repository of the checkout it is run in, for the whole repository", async () => {
    const home = temporaryHome();
    const { checkout, repositoryKey } = recordedCheckout(home);
    const folder = thirdPartyPack(join(home, "copies"), "rerail-pack-conventions", "conventions");

    const text = await addIn(checkout, [folder], linkingPackStore(home, {}));

    expect(addedPacks(home, checkout, repositoryKey)).toEqual({ packs: ["conventions"], setUp: true });
    expect(text).toContain(`Added to ${basename(checkout)}, for the whole repository: conventions`);
  });

  it("adds a pack named again that is installed already, and changes nothing for one added already", async () => {
    const home = temporaryHome();
    const { checkout, repositoryKey } = recordedCheckout(home);
    const folder = thirdPartyPack(join(home, "copies"), "rerail-pack-conventions", "conventions");
    const store = linkingPackStore(home, { "@neralabs/rerail-pack-valid": fixturesPath("packs", "valid") });
    // Installed before `packs add` added packs anywhere: in the home, added to no repository.
    installPackDirectory(folder, "rerail-pack-conventions", home);

    await addIn(checkout, [folder], store);
    expect(addedPacks(home, checkout, repositoryKey).packs).toEqual(["conventions"]);

    const text = await addIn(checkout, [folder, "valid@0.1.0"], store);
    expect(addedPacks(home, checkout, repositoryKey).packs).toEqual(["conventions", "valid"]);
    expect(text).toContain(`Added to ${basename(checkout)}, for the whole repository: valid`);
    expect(text).toContain(`Added to ${basename(checkout)} already: conventions`);
  });

  it("sets a recorded repository that is not set up up with the pack, without starting Rerail there", async () => {
    const home = temporaryHome();
    const { checkout, repositoryKey } = recordedCheckout(home, false);
    const store = linkingPackStore(home, { "@neralabs/rerail-pack-valid": fixturesPath("packs", "valid") });

    const text = await addIn(checkout, ["valid@0.1.0"], store);

    expect(addedPacks(home, checkout, repositoryKey)).toEqual({ packs: ["valid"], setUp: true });
    expect(text).toContain(`Added to ${basename(checkout)}, for the whole repository: valid. ${basename(checkout)} is set up with it now.`);
    expect(text).toContain("  Not running: Rerail is not on in this checkout. `rerail start` turns it on.");
  });

  it("adds a pack kept in the repository, named by a path relative to where it is run, from any folder of the checkout", async () => {
    const home = temporaryHome();
    const { checkout, repositoryKey } = recordedCheckout(home);
    // The repository keeps one project's rules as a pack in that project's .rerail folder.
    const project = join(checkout, "web-client");
    const folder = thirdPartyPack(project, "rerail-pack-web-client", "web-client");
    const pack = join(project, ".rerail");
    execFileSync("mv", [folder, pack]);
    const store = linkingPackStore(home, {});

    const text = await addIn(checkout, ["./web-client/.rerail"], store);
    expect(addedPacks(home, checkout, repositoryKey).packs).toEqual(["web-client"]);
    expect(text).toContain(`Added to ${basename(checkout)}, for the whole repository: web-client`);
    // Rerail is not on in the checkout; said of the checkout, not of the folder it was run in.
    expect(text).toContain("  Not running: Rerail is not on in this checkout. `rerail start` turns it on.");

    expect(await addIn(project, ["./.rerail"], store)).toContain(`Added to ${basename(checkout)} already: web-client`);
  });

  it("only installs outside a checkout Rerail knows, and says where the pack is added", async () => {
    const home = temporaryHome();
    const elsewhere = realpathSync(mkdtempSync(join(tmpdir(), "rerail-packs-cli-elsewhere-")));
    homes.push(elsewhere);
    const store = linkingPackStore(home, { "@neralabs/rerail-pack-valid": fixturesPath("packs", "valid") });

    const text = await addIn(elsewhere, ["valid@0.1.0"], store);

    expect(store.installs).toEqual([["@neralabs/rerail-pack-valid@0.1.0"]]);
    expect(text).toContain(`Not added to a repository: Rerail does not know ${elsewhere}.`);
    // No database is made for it, as on a CI machine that installs packs for `rerail scan --pack`.
    expect(existsSync(join(home, "rerail.sqlite"))).toBe(false);
  });

  it("is not added to a set-up repository from a coding agent's session, as the API refuses it", async () => {
    const home = temporaryHome();
    const { checkout, repositoryKey } = recordedCheckout(home);
    vi.stubEnv("CLAUDECODE", "1");

    const text = await addIn(checkout, ["valid@0.1.0"], linkingPackStore(home, { "@neralabs/rerail-pack-valid": fixturesPath("packs", "valid") }));

    expect(addedPacks(home, checkout, repositoryKey).packs).toEqual([]);
    expect(text).toContain(`Not added to ${basename(checkout)}: it is set up, so a coding agent can't add a pack to the whole repository (Claude Code runs this command, CLAUDECODE is set). `
      + "Ask the person to add it on the dashboard's Rule packs page, or to run rerail packs add valid@0.1.0 in their own terminal.");
  });
});

describe("a third party's pack", () => {
  it("is added under its package name, listed by it, and warned about once: Rerail has not reviewed it", async () => {
    const home = temporaryHome();
    installPackDirectory(fixturesPath("packs", "valid"), "valid", home);
    const store = linkingPackStore(home, {
      "@acme/rerail-pack-acme-valid": thirdPartyPack(join(home, "copies"), "@acme/rerail-pack-acme-valid", "acme-valid"),
      "rerail-pack-lint": thirdPartyPack(join(home, "copies"), "rerail-pack-lint", "lint"),
    });

    const text = await packsOutput(["add", "@acme/rerail-pack-acme-valid@1.0.0", "rerail-pack-lint@1.0.0"], store);

    expect(store.installs).toEqual([["@acme/rerail-pack-acme-valid@1.0.0", "rerail-pack-lint@1.0.0"]]);
    expect(text).toMatch(/^ {2}@acme\/rerail-pack-acme-valid@1\.0\.0 {2}@acme\/rerail-pack-acme-valid's acme-valid: /mu);
    expect(text).toMatch(/^ {2}rerail-pack-lint@1\.0\.0 {2}/mu);
    expect(text).toMatch(/^ {2}valid@0\.1\.0 {2}Valid: /mu);
    for (const name of ["@acme/rerail-pack-acme-valid@1.0.0", "rerail-pack-lint@1.0.0"]) {
      expect(text.split(`${name} is not one of Rerail's packs, and Rerail has not reviewed it.`)).toHaveLength(2);
    }
    expect(text).toContain("Its rules run in Log until a person reads them and approves this version (rerail rules approve rerail-pack-lint);");
    expect(text).not.toContain("valid@0.1.0 is not one of Rerail's packs");
  });

  it("is not warned about when the pack added is Rerail's own", async () => {
    const home = temporaryHome();
    const store = linkingPackStore(home, { "@neralabs/rerail-pack-valid": fixturesPath("packs", "valid") });

    expect(await packsOutput(["add", "valid@0.1.0"], store)).not.toContain("Rerail has not reviewed");
  });

  it("is said to come from a folder when added from one, and warned about even when its package.json names Rerail's scope", async () => {
    const home = temporaryHome();
    const folder = thirdPartyPack(join(home, "copies"), "@neralabs/rerail-pack-local", "local");

    const text = await packsOutput(["add", folder], linkingPackStore(home, {}));

    expect(text).toMatch(new RegExp(`^ {2}local@1\\.0\\.0 {2}.*rules start on {2}\\(from the folder ${folder.replace(/[.*+?^${}()|[\]\\]/gu, "\\$&")}\\)$`, "mu"));
    expect(text).toContain(`@neralabs/rerail-pack-local@1.0.0 was installed from the folder ${folder}, not from npm, and Rerail has not reviewed it.`);
  });

  it("is removed by its package name alone, or with every other package holding its pack id by that id", async () => {
    const home = temporaryHome();
    const copies = join(home, "copies");
    installPackDirectory(fixturesPath("packs", "valid"), "valid", home);
    installPackDirectory(thirdPartyPack(copies, "@acme/rerail-pack-valid", "valid"), "@acme/rerail-pack-valid", home);
    installPackDirectory(thirdPartyPack(copies, "rerail-pack-valid", "valid"), "rerail-pack-valid", home);
    installPackDirectory(thirdPartyPack(copies, "@acme/rerail-pack-other", "other"), "@acme/rerail-pack-other", home);
    const store = linkingPackStore(home, {});

    await packsOutput(["remove", "@acme/rerail-pack-other"], store);
    expect(store.removes).toEqual([["@acme/rerail-pack-other"]]);

    const text = await packsOutput(["remove", "valid"], store);
    expect(store.removes.at(-1)).toEqual(["@acme/rerail-pack-valid", "@neralabs/rerail-pack-valid", "rerail-pack-valid"]);
    expect(installedPackagesOf("valid", home)).toEqual([]);
    expect(text).toContain("No rule packs are installed.");
  });

  it("is not left to take a pack id by name: removing the package that holds it, while another holding it stays, is refused", async () => {
    const home = temporaryHome();
    const copies = join(home, "copies");
    installPackDirectory(fixturesPath("packs", "valid"), "valid", home);
    installPackDirectory(thirdPartyPack(copies, "@acme/rerail-pack-valid", "valid"), "@acme/rerail-pack-valid", home);
    const store = linkingPackStore(home, {});

    await expect(packsOutput(["remove", "@neralabs/rerail-pack-valid"], store)).rejects.toThrow(
      "Removing @neralabs/rerail-pack-valid would let @acme/rerail-pack-valid run as pack valid in every repository that adds valid. "
      + "Remove them all with `rerail packs remove valid`, or remove @acme/rerail-pack-valid first.",
    );
    expect(store.removes).toEqual([]);
    // The package left out can go on its own.
    await packsOutput(["remove", "@acme/rerail-pack-valid"], store);
    expect(store.removes).toEqual([["@acme/rerail-pack-valid"]]);
  });

  it("is refused for removal under a name that is neither a pack id nor a pack's package", async () => {
    const store = linkingPackStore(temporaryHome(), {});
    await expect(runPacksCommand(["remove", "@acme/sql"], () => "usage", store)).rejects.toThrow("Name the rule packs to remove by id or package");
    expect(store.removes).toEqual([]);
  });
});

describe("rerail packs recommend", () => {
  it("says why no rule pack fits, then lists each pack once, with what it looks for", { timeout: 30_000 }, async () => {
    const root = realpathSync(temporaryHome());
    for (const path of ["tests/test_a.py", "tests/test_b.py", "scripts/release.py", "README.md"]) {
      mkdirSync(dirname(join(root, path)), { recursive: true });
      writeFileSync(join(root, path), "x = 1\n");
    }
    execFileSync("git", ["-C", root, "init", "-q"]);

    const text = await packsOutput(["recommend", "--repo", root], directoryPackStore(repositoryPath("packs"), ["python"], temporaryHome()));

    expect(text).toMatch(new RegExp([
      `No rule pack matches what ${root.replace(/[.*+?^${}()|[\]\\]/gu, "\\$&")} is built with\\.`,
      "  Found           \\.py 3 · \\.md 1",
      "                  from Git’s list of files, which leaves out what Git ignores",
      "  Closest         Python excludes the 3 files it looks for: tests/ \\(2\\),",
      "                  scripts/ \\(1\\)",
      "Other rule packs:",
      "  @neralabs/rerail-pack-python@\\S+ \\(\\d+ rules\\): looks for \\*\\.py, \\*\\.pyi\n",
    ].join("\n"), "u"));
    expect(text).not.toContain("Packs look for");
  });
});

describe("rerail packs list", () => {
  it("names a pack left out because Rerail's own pack holds its pack id, with the reason, wherever it was found", async () => {
    const home = temporaryHome();
    installPackDirectory(fixturesPath("packs", "valid"), "valid", home);
    // npm installed it without the check `rerail packs add` runs; its scope sorts before @neralabs.
    installPackDirectory(thirdPartyPack(join(home, "copies"), "@aaa/rerail-pack-valid", "valid"), "@aaa/rerail-pack-valid", home);

    const text = await packsOutput(["list"], linkingPackStore(home, {}));

    expect(text).toMatch(/^ {2}valid@0\.1\.0 {2}Valid: /mu);
    expect(text).toContain("  left out: @aaa/rerail-pack-valid@1.0.0: pack id valid is taken by @neralabs/rerail-pack-valid");
  });

  it("leaves a rule this Rerail no longer runs out of what starts on, and names it with what runs it again", async () => {
    const home = temporaryHome();
    installPackDirectory(withOlderTokenRule(fixturesPath("packs", "valid"), join(home, "copy"), "pass-statement"), "valid", home);

    const text = await packsOutput(["list"], linkingPackStore(home, {}));

    expect(text).toMatch(/^ {2}valid@0\.1\.0 {2}Valid: 1 of 2 rules start on/mu);
    expect(text).toContain("    1 rule does not run on this Rerail: it tests a token parsed files now leave out, and the pack's next version runs it again: pass-statement\n");
  });

  it("lists the worktree's own rules there, saying when they do not run until Rerail is on and the repo set up", async () => {
    const home = temporaryHome();
    const worktree = realpathSync(mkdtempSync(join(tmpdir(), "rerail-packs-cli-worktree-")));
    homes.push(worktree);
    execFileSync("git", ["init", "-q"], { cwd: worktree });
    mkdirSync(join(worktree, ".rerail", "rules"), { recursive: true });
    writeFileSync(join(worktree, ".rerail", "rules", "no-todo.json"), JSON.stringify({
      format: "rerail.policy/v1", id: "local:no-todo", revision: 1, mode: "steer",
      clauses: [{ id: "todo", violation: { code: "TODO", anchor: "witness.span", message: "Resolve it." }, forbid: { regex: { pattern: "TODO", source: "file.content" } } }],
    }));
    // `rerail packs list` reads the checkout it is run in; `packsOutput` restores every mock after it runs.
    const list = async (): Promise<string> => {
      vi.spyOn(process, "cwd").mockReturnValue(worktree);
      return await packsOutput(["list"], linkingPackStore(home, {}));
    };

    expect(await list()).toContain("  Not running: Rerail is not on in this checkout. `rerail start` turns it on.");

    const database = openRerailDatabase(join(home, "rerail.sqlite"));
    const { repositoryKey } = recordCheckout(database, worktree);
    setRepositoryStarted(database, repositoryKey, true);
    const started = await list();
    expect(started).toContain("  Not running: this repo is not set up yet, so no rule runs here. `rerail start` sets it up.");
    expect(started).toMatch(/^ {2}local:no-todo \(code, steer\)$/mu);

    setUpRepository(database, repositoryKey, { packs: [] });
    database.$client.close();
    expect(await list()).not.toContain("Not running");
  });
});
