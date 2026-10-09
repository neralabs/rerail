import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, realpathSync, rmSync, writeFileSync } from "node:fs";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { openRerailDatabase, type RerailDatabase } from "../../../src/engine/storage/database.js";
import {
  adoptRemote,
  linkAlias,
  listRepositories,
  lookupCheckout,
  parentCheckouts,
  pruneCheckouts,
  recordCheckout,
} from "../../../src/engine/catalog/repository-registry.js";
import { repositoryIdentityLocation } from "../../../src/engine/catalog/repository-identity.js";
import { setUpRepository } from "../../../src/engine/catalog/rule-configuration.js";

const temporaryDirectories: string[] = [];
const openDatabases: RerailDatabase[] = [];
afterEach(async () => {
  for (const database of openDatabases.splice(0)) database.$client.close();
  await Promise.all(temporaryDirectories.splice(0).map(path => rm(path, { recursive: true, force: true })));
});

async function temporary(prefix: string): Promise<string> {
  const path = realpathSync(await mkdtemp(join(tmpdir(), prefix)));
  temporaryDirectories.push(path);
  return path;
}

function git(cwd: string, ...args: string[]): string {
  return execFileSync("git", ["-c", "protocol.file.allow=always", ...args], { cwd, encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] });
}

function gitRepository(path: string, origin?: string): string {
  mkdirSync(path, { recursive: true });
  git(path, "init", "-q", "-b", "main");
  git(path, "-c", "user.name=Rerail", "-c", "user.email=rerail@example.invalid", "commit", "-q", "--allow-empty", "-m", "base");
  if (origin !== undefined) git(path, "remote", "add", "origin", origin);
  return path;
}

function open(path: string): RerailDatabase {
  const database = openRerailDatabase(path);
  openDatabases.push(database);
  return database;
}

function checkoutSetting(database: RerailDatabase, checkoutPath: string): void {
  database.$client.prepare(
    "INSERT INTO rule_settings (owner_kind, owner_id, folder, target_kind, target_id, \"on\", mode, updated_at) VALUES ('checkout', ?, '', 'rule', 'r', 0, NULL, 1)",
  ).run(checkoutPath);
}

function settingOwners(database: RerailDatabase): unknown[] {
  return database.$client.prepare("SELECT owner_kind, owner_id FROM rule_settings ORDER BY owner_kind, owner_id").all();
}

describe("the repository registry", () => {
  it("puts clones that share a remote and worktrees that share a clone in one repository", async () => {
    const root = await temporary("rerail-registry-join-");
    const database = open(join(root, "rerail.sqlite"));
    const first = gitRepository(join(root, "first"), "ssh://git@GitHub.com:22/acme/web.git/");
    const second = gitRepository(join(root, "second"), "git@github.com:acme/web.git");
    const worktree = join(root, "first-feature");
    git(first, "worktree", "add", "-q", "-b", "feature", worktree);

    const recorded = [first, second, worktree].map(path => recordCheckout(database, path));
    expect(recorded.map(checkout => checkout.repositoryKey)).toEqual(Array(3).fill("remote:github.com/acme/web"));
    expect(recorded[2]).toMatchObject({ checkoutPath: worktree, cloneId: recorded[0]!.cloneId, main: false });
    // A remote-keyed repository is named after its remote, not the folder it was first recorded in.
    expect(listRepositories(database)).toMatchObject([{ repositoryKey: "remote:github.com/acme/web", displayName: "web", checkouts: [{}, {}, {}] }]);

    // A recorded checkout stays where it is when its `origin` changes later.
    git(second, "remote", "set-url", "origin", "https://github.com/acme/other.git");
    expect(recordCheckout(database, second).repositoryKey).toBe("remote:github.com/acme/web");
  });

  it("keys a clone without an origin and with several remotes by its clone id", async () => {
    const root = await temporary("rerail-registry-remotes-");
    const database = open(join(root, "rerail.sqlite"));
    const checkout = gitRepository(join(root, "checkout"));
    git(checkout, "remote", "add", "upstream", "https://github.com/acme/web.git");
    git(checkout, "remote", "add", "fork", "https://github.com/someone/web.git");
    const recorded = recordCheckout(database, checkout);
    expect(recorded.repositoryKey).toBe(`clone:${recorded.cloneId}`);
  });

  it("records a re-clone at a recorded path again, without the old checkout's settings", async () => {
    const root = await temporary("rerail-registry-reclone-");
    const database = open(join(root, "rerail.sqlite"));
    const path = gitRepository(join(root, "checkout"));
    const before = recordCheckout(database, path);
    checkoutSetting(database, path);
    rmSync(path, { recursive: true, force: true });
    gitRepository(path, "https://github.com/acme/web.git");
    const after = recordCheckout(database, path);
    expect(after.cloneId).not.toBe(before.cloneId);
    expect(after.repositoryKey).toBe("remote:github.com/acme/web");
    expect(settingOwners(database)).toEqual([]);
  });

  it("finds the nearest containing checkout of the same repository first", async () => {
    const root = await temporary("rerail-registry-parent-");
    const database = open(join(root, "rerail.sqlite"));
    const main = gitRepository(join(root, "main"));
    const outer = join(main, "worktrees", "outer");
    const inner = join(outer, "worktrees", "inner");
    const beside = join(root, "beside");
    git(main, "worktree", "add", "-q", "-b", "outer", outer);
    git(main, "worktree", "add", "-q", "-b", "inner", inner);
    git(main, "worktree", "add", "-q", "-b", "beside", beside);
    const [, , innerCheckout] = [main, outer, inner, beside].map(path => recordCheckout(database, path));
    expect(parentCheckouts(database, innerCheckout!).map(row => row.checkoutPath)).toEqual([outer, main]);
  });

  it("puts a submodule checked out under two worktrees of its parent in one repository", async () => {
    const root = await temporary("rerail-registry-submodule-");
    const database = open(join(root, "rerail.sqlite"));
    const library = gitRepository(join(root, "library"));
    const parent = gitRepository(join(root, "parent"));
    git(parent, "submodule", "add", "-q", library, "vendor/library");
    git(parent, "-c", "user.name=Rerail", "-c", "user.email=rerail@example.invalid", "commit", "-q", "-m", "submodule");
    const worktree = join(root, "parent-feature");
    git(parent, "worktree", "add", "-q", "-b", "feature", worktree);
    git(worktree, "submodule", "update", "-q", "--init");
    const submodules = [join(parent, "vendor", "library"), join(worktree, "vendor", "library")];
    for (const submodule of submodules) git(submodule, "remote", "set-url", "origin", "https://github.com/acme/library.git");

    const recorded = submodules.map(path => recordCheckout(database, path));
    expect(recorded[0]!.cloneId).not.toBe(recorded[1]!.cloneId);
    expect(recorded.map(checkout => checkout.repositoryKey)).toEqual(["remote:github.com/acme/library", "remote:github.com/acme/library"]);
    expect(recordCheckout(database, parent).repositoryKey).not.toBe("remote:github.com/acme/library");
  });

  it("links an alias so a checkout with the old address joins the repository", async () => {
    const root = await temporary("rerail-registry-alias-");
    const database = open(join(root, "rerail.sqlite"));
    const current = gitRepository(join(root, "current"), "https://github.com/acme/web.git");
    const renamed = gitRepository(join(root, "renamed"), "git@github.com:acme/old-web.git");
    const key = recordCheckout(database, current).repositoryKey;
    expect(linkAlias(database, key, "https://github.com/acme/old-web")).toBe("remote:github.com/acme/old-web");
    expect(recordCheckout(database, renamed).repositoryKey).toBe(key);

    // An address a set-up repository already owns is refused.
    const other = gitRepository(join(root, "other"), "https://github.com/acme/api.git");
    const otherKey = recordCheckout(database, other).repositoryKey;
    setUpRepository(database, otherKey, { packs: [] });
    expect(() => linkAlias(database, key, "https://github.com/acme/api")).toThrow(/set-up repository/u);
  });

  it("keeps the merged repository's own key and aliases when an alias brings it in", async () => {
    const root = await temporary("rerail-registry-merge-");
    const database = open(join(root, "rerail.sqlite"));
    const web = gitRepository(join(root, "web"), "https://github.com/acme/web.git");
    const mirror = gitRepository(join(root, "mirror"), "https://github.com/acme/web-mirror.git");
    const key = recordCheckout(database, web).repositoryKey;
    const mirrorKey = recordCheckout(database, mirror).repositoryKey;
    linkAlias(database, mirrorKey, "https://github.com/acme/web-old");

    // Linking the mirror's alias to web brings the mirror's repository, with its own key and aliases, into web.
    linkAlias(database, key, "https://github.com/acme/web-old");
    const merged = listRepositories(database).find(repository => repository.repositoryKey === key)!;
    expect([...merged.aliases].sort()).toEqual([mirrorKey, "remote:github.com/acme/web-old"].sort());
    // A fresh clone of the mirror joins web instead of starting a repository of its own.
    const fresh = gitRepository(join(root, "fresh-mirror"), "https://github.com/acme/web-mirror.git");
    expect(recordCheckout(database, fresh).repositoryKey).toBe(key);
  });

  it("adopts a remote added to a clone after it was first recorded", async () => {
    const root = await temporary("rerail-registry-late-remote-");
    const database = open(join(root, "rerail.sqlite"));
    const clone = gitRepository(join(root, "clone"));
    expect(recordCheckout(database, clone).repositoryKey).toMatch(/^clone:/u);
    git(clone, "remote", "add", "origin", "https://github.com/acme/late.git");
    expect(recordCheckout(database, clone).repositoryKey).toBe("remote:github.com/acme/late");
  });

  it("gives an unconfigured target the clone repository's configuration, and moves only checkouts to a configured one", async () => {
    const root = await temporary("rerail-registry-adopt-");
    const database = open(join(root, "rerail.sqlite"));
    const clone = gitRepository(join(root, "clone"));
    const cloneCheckout = recordCheckout(database, clone);
    const cloneKey = cloneCheckout.repositoryKey;
    // A clone-keyed repository keeps its folder's name until it adopts a remote.
    expect(listRepositories(database)).toMatchObject([{ repositoryKey: cloneKey, displayName: "clone" }]);
    setUpRepository(database, cloneKey, {
      packs: [{ packId: "python", folder: "" }],
      settings: [{ owner: { kind: "repository", repositoryKey: cloneKey }, folder: "", targetKind: "rule", targetId: "r", on: false, mode: null }],
    });
    database.$client.prepare("UPDATE repositories SET started = 1 WHERE repository_key = ?").run(cloneKey);

    adoptRemote(database, cloneKey, "remote:github.com/acme/web");
    // Another process adopting the same repository afterwards finds it gone and leaves it as it is.
    adoptRemote(database, cloneKey, "remote:github.com/acme/web");
    expect(listRepositories(database)).toMatchObject([{
      repositoryKey: "remote:github.com/acme/web", displayName: "web", aliases: [cloneKey], started: true, configuredAt: expect.any(Number),
      checkouts: [{ checkoutPath: clone }],
    }]);
    expect(database.$client.prepare("SELECT repository_key, source_id FROM rule_attachments").all())
      .toEqual([{ repository_key: "remote:github.com/acme/web", source_id: "python" }]);
    expect(settingOwners(database)).toEqual([{ owner_kind: "repository", owner_id: "remote:github.com/acme/web" }]);

    // A configured target wins: the other repository keeps its key and loses only its checkouts.
    const stray = gitRepository(join(root, "stray"));
    const strayKey = recordCheckout(database, stray).repositoryKey;
    setUpRepository(database, strayKey, { packs: [{ packId: "web", folder: "" }] });
    adoptRemote(database, strayKey, "remote:github.com/acme/web");
    const repositories = listRepositories(database);
    expect(repositories.find(row => row.repositoryKey === "remote:github.com/acme/web")?.checkouts.map(row => row.checkoutPath).sort())
      .toEqual([clone, stray].sort());
    expect(repositories.find(row => row.repositoryKey === strayKey)?.checkouts).toEqual([]);
  });

  it("prunes a removed worktree and its settings, and keeps live checkouts", async () => {
    const root = await temporary("rerail-registry-prune-");
    const database = open(join(root, "rerail.sqlite"));
    const main = gitRepository(join(root, "main"));
    const worktree = join(root, "feature");
    git(main, "worktree", "add", "-q", "-b", "feature", worktree);
    recordCheckout(database, main);
    recordCheckout(database, worktree);
    checkoutSetting(database, worktree);
    checkoutSetting(database, main);
    git(main, "worktree", "remove", worktree);

    expect(pruneCheckouts(database)).toEqual([worktree]);
    expect(listRepositories(database)[0]?.checkouts.map(row => row.checkoutPath)).toEqual([main]);
    expect(settingOwners(database)).toEqual([{ owner_kind: "checkout", owner_id: main }]);
  });

  it("looks a checkout up without writing anything or creating a clone id", async () => {
    const root = await temporary("rerail-registry-lookup-");
    const database = open(join(root, "rerail.sqlite"));
    const checkout = gitRepository(join(root, "checkout"));
    expect(lookupCheckout(database, checkout)).toBeUndefined();
    expect(existsSync(repositoryIdentityLocation(checkout).identityPath)).toBe(false);
    expect(listRepositories(database)).toEqual([]);
    const recorded = recordCheckout(database, checkout);
    mkdirSync(join(checkout, "src"));
    expect(lookupCheckout(database, join(checkout, "src"))).toEqual(recorded);
  });

  it("finds a fresh clone without an id file by its remote, with no clone id, and records it under that repository", async () => {
    const root = await temporary("rerail-registry-fresh-");
    const database = open(join(root, "rerail.sqlite"));
    const first = gitRepository(join(root, "first"), "https://github.com/acme/web.git");
    const fresh = gitRepository(join(root, "fresh"), "git@github.com:acme/web.git");
    const known = recordCheckout(database, first);

    expect(lookupCheckout(database, fresh)).toEqual({ checkoutPath: fresh, repositoryKey: known.repositoryKey, cloneId: null, git: true, main: true });
    expect(existsSync(repositoryIdentityLocation(fresh).identityPath)).toBe(false);
    expect(listRepositories(database)[0]?.checkouts.map(row => row.checkoutPath)).toEqual([first]);

    const recorded = recordCheckout(database, fresh);
    expect(recorded).toMatchObject({ checkoutPath: fresh, repositoryKey: known.repositoryKey, cloneId: expect.any(String) });
    expect(recorded.cloneId).not.toBe(known.cloneId);
    expect(lookupCheckout(database, fresh)).toEqual(recorded);
  });
});


