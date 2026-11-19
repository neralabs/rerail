import { execFileSync } from "node:child_process";
import { chmodSync, existsSync, mkdirSync, mkdtempSync, realpathSync, renameSync, rmSync, writeFileSync } from "node:fs";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { openRerailDatabase, type RerailDatabase } from "../../../src/engine/storage/database.js";
import {
  adoptRemote,
  createCheckoutResolver,
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

/** Whether this filesystem matches names case-insensitively, as macOS does by default. */
function caseInsensitiveFilesystem(): boolean {
  const probe = realpathSync(mkdtempSync(join(tmpdir(), "rerail-case-")));
  try {
    return existsSync(probe.toUpperCase()) && existsSync(probe.toLowerCase());
  } finally {
    rmSync(probe, { recursive: true, force: true });
  }
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
  it("resolves the current linked checkout through moves, replacement, and restoration", async () => {
    const root = await temporary("rerail-resolver-replace-"), db = open(join(root, "home.sqlite"));
    const a = gitRepository(join(root, "a")), b = gitRepository(join(root, "b"));
    const first = recordCheckout(db, a), second = recordCheckout(db, b);
    const path = join(root, "linked"), moved = join(root, "moved");
    git(a, "worktree", "add", "--detach", path);
    const lookup = createCheckoutResolver(db);
    expect(lookup(path)?.repositoryKey).toBe(first.repositoryKey);
    git(a, "worktree", "move", path, moved);
    expect(lookup(path)).toBeUndefined();
    git(b, "worktree", "add", "--detach", path);
    expect(lookup(path)).toEqual(lookupCheckout(db, path));
    expect(lookup(path)?.repositoryKey).toBe(second.repositoryKey);
    git(b, "worktree", "remove", path);
    git(a, "worktree", "move", moved, path);
    expect(lookup(path)?.repositoryKey).toBe(first.repositoryKey);
  });

  it("observes changed, removed, and included origin URLs in a long-lived resolver", async () => {
    const root = await temporary("rerail-resolver-remote-"), db = open(join(root, "home.sqlite"));
    const a = "https://example.invalid/team/alpha", b = "https://example.invalid/team/beta";
    const first = recordCheckout(db, gitRepository(join(root, "a"), a));
    const second = recordCheckout(db, gitRepository(join(root, "b"), b));
    const clone = gitRepository(join(root, "clone"), a), lookup = createCheckoutResolver(db);
    expect(lookup(clone)?.repositoryKey).toBe(first.repositoryKey);
    git(clone, "remote", "set-url", "origin", b);
    expect(lookup(clone)?.repositoryKey).toBe(second.repositoryKey);
    git(clone, "remote", "remove", "origin");
    expect(lookup(clone)).toBeUndefined();
    const include = join(root, "remotes.config");
    git(clone, "config", "include.path", include);
    for (const [url, key] of [[a, first.repositoryKey], [b, second.repositoryKey]] as const) {
      writeFileSync(include, `[remote "origin"]\n  url = ${url}\n`);
      expect(lookup(clone)?.repositoryKey).toBe(key);
    }
  });
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

  it("uses one clone ID for a bare repository and linked checkout in either discovery order", async () => {
    const root = await temporary("rerail-registry-bare-");
    const database = open(join(root, "rerail.sqlite")), source = gitRepository(join(root, "source"));
    for (const first of ["bare", "linked"] as const) {
      const bare = join(root, first + ".git"), linked = join(root, first + "-linked");
      git(root, "clone", "--bare", source, bare);
      git(bare, "worktree", "add", "--detach", linked, "HEAD");
      const [a, b] = first === "bare" ? [bare, linked] : [linked, bare];
      const one = recordCheckout(database, a), two = recordCheckout(database, b);
      expect(two.cloneId).toBe(one.cloneId);
      expect(two.repositoryKey).toBe(one.repositoryKey);
      expect(repositoryIdentityLocation(bare)).toMatchObject({ workspaceRoot: bare, git: true, main: true, identityPath: join(bare, "rerail-repository-id") });
      expect(existsSync(join(bare, ".rerail", "repository-id"))).toBe(false);
    }
  });

  it("adopts the earlier bare ID and preserves bare checkout settings when a linked ID already exists", async () => {
    const root = await temporary("rerail-registry-older-bare-");
    const database = open(join(root, "rerail.sqlite")), source = gitRepository(join(root, "source"));
    for (const existing of [false, true]) {
      const bare = join(root, String(existing) + ".git"), linked = join(root, "linked-" + existing);
      // Earlier releases treated the bare path as a non-Git folder.
      mkdirSync(bare);
      const old = recordCheckout(database, bare);
      checkoutSetting(database, bare);
      git(root, "init", "--bare", bare);
      git(bare, "fetch", source, "HEAD:refs/heads/main");
      git(bare, "worktree", "add", "--detach", linked, "refs/heads/main");
      if (existing) writeFileSync(join(bare, "rerail-repository-id"), "git:previous-linked-id\n");
      const worktree = recordCheckout(database, linked);
      expect(worktree.cloneId).toBe(existing ? "git:previous-linked-id" : old.cloneId);
      const upgraded = recordCheckout(database, bare);
      expect(upgraded.cloneId).toBe(worktree.cloneId);
      expect(upgraded.repositoryKey).toBe(worktree.repositoryKey);
      expect(settingOwners(database)).toContainEqual({ owner_kind: "checkout", owner_id: bare });
    }
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

  it("adopts a clone's remote in both catalogs and retries a rolled-back adoption", async () => {
    const root = await temporary("rerail-registry-catalog-adopt-");
    const first = open(join(root, "first.sqlite")), second = open(join(root, "second.sqlite"));
    const clone = gitRepository(join(root, "clone"));
    const { repositoryKey } = recordCheckout(second, clone);
    expect(recordCheckout(first, clone).repositoryKey).toBe(repositoryKey);
    git(clone, "remote", "add", "origin", "https://example.com/acme/web.git");
    first.$client.exec("CREATE TRIGGER refuse_adoption BEFORE UPDATE OF repository_key ON repositories BEGIN SELECT RAISE(ABORT, 'fixture'); END");
    expect(() => recordCheckout(first, clone)).toThrow(/fixture/u);
    expect(recordCheckout(second, clone).repositoryKey).toBe("remote:example.com/acme/web");
    expect(listRepositories(first)[0]?.repositoryKey).toBe(repositoryKey);
    first.$client.exec("DROP TRIGGER refuse_adoption");
    expect(recordCheckout(first, clone).repositoryKey).toBe("remote:example.com/acme/web");
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

  it("keeps a main checkout with a separate Git directory and its linked worktree", async () => {
    const root = await temporary("rerail-registry-separate-git-");
    const database = open(join(root, "rerail.sqlite"));
    const checkout = gitRepository(join(root, "checkout")), linked = join(root, "linked");
    git(checkout, "init", "--quiet", "--separate-git-dir", join(root, "metadata"));
    git(checkout, "worktree", "add", "-q", "--detach", linked);
    for (const dir of [checkout, linked]) {
      recordCheckout(database, dir);
      checkoutSetting(database, dir);
    }
    expect(pruneCheckouts(database)).toEqual([]);
    expect(settingOwners(database)).toHaveLength(2);
  });

  it.skipIf(process.platform === "win32" || process.getuid?.() === 0)("keeps settings while a checkout parent is inaccessible", async () => {
    const root = await temporary("rerail-registry-inaccessible-");
    const database = open(join(root, "rerail.sqlite"));
    const parent = join(root, "private"), dir = gitRepository(join(parent, "checkout"));
    recordCheckout(database, dir);
    checkoutSetting(database, dir);
    chmodSync(parent, 0);
    try {
      expect(pruneCheckouts(database)).toEqual([]);
    } finally { chmodSync(parent, 0o700); }
    expect(pruneCheckouts(database)).toEqual([]);
    expect(settingOwners(database)).toHaveLength(1);
    renameSync(dir, join(root, "moved"));
    expect(pruneCheckouts(database)).toEqual([dir]);
  });

  it.each(["restored", "replaced"])("keeps a checkout %s between enumeration and pruning", async change => {
    const root = await temporary("rerail-registry-prune-race-"), path = join(root, "catalog.sqlite");
    const db = open(path), other = open(path), moved = join(root, "moved");
    const dir = gitRepository(join(root, "checkout"));
    const original = recordCheckout(db, dir);
    checkoutSetting(db, dir);
    renameSync(dir, moved);
    const transaction = db.transaction.bind(db);
    vi.spyOn(db, "transaction").mockImplementationOnce((run, options) => {
      if (change === "restored") renameSync(moved, dir);
      else {
        gitRepository(dir);
        expect(recordCheckout(other, dir).cloneId).not.toBe(original.cloneId);
        checkoutSetting(other, dir);
      }
      return transaction(run, options);
    });
    expect(pruneCheckouts(db)).toEqual([]);
    expect(lookupCheckout(db, dir)?.cloneId).toBe(recordCheckout(other, dir).cloneId);
    expect(settingOwners(db)).toEqual([{ owner_kind: "checkout", owner_id: dir }]);
  });

  it("requires a replacement directory's own identity before using the recorded checkout", async () => {
    const root = await temporary("rerail-registry-replaced-"), db = open(join(root, "catalog.sqlite"));
    const dir = gitRepository(join(root, "checkout")), moved = join(root, "moved");
    const recorded = recordCheckout(db, dir);
    renameSync(dir, moved);
    mkdirSync(dir);
    expect(lookupCheckout(db, dir)).toBeUndefined();
    rmSync(dir, { recursive: true });
    renameSync(moved, dir);
    expect(lookupCheckout(db, dir)).toEqual(recorded);
  });

  it.skipIf(!caseInsensitiveFilesystem())("keeps a worktree Git registered under a path typed in another case than the folder has", async () => {
    const root = await temporary("rerail-registry-prune-case-");
    const database = open(join(root, "rerail.sqlite"));
    const main = gitRepository(join(root, "main"));
    mkdirSync(join(root, "elsewhere"));
    git(main, "worktree", "add", "-q", "-b", "feature", join(root, "Elsewhere", "feature"));
    recordCheckout(database, main);
    const worktree = recordCheckout(database, join(root, "elsewhere", "feature")).checkoutPath;
    expect(worktree).toBe(join(root, "elsewhere", "feature"));

    expect(pruneCheckouts(database)).toEqual([]);
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

  it.skipIf(process.platform === "win32" || process.getuid?.() === 0)("recognizes a moved read-only checkout and ignores its unrelated replacement", async () => {
    const root = await temporary("rerail-registry-readonly-");
    const database = open(join(root, "rerail.sqlite"));
    const path = gitRepository(join(root, "checkout")), moved = join(root, "moved");
    chmodSync(join(path, ".git"), 0o555);
    try {
      const recorded = recordCheckout(database, path);
      expect(existsSync(repositoryIdentityLocation(path).identityPath)).toBe(false);
      renameSync(path, moved);
      gitRepository(path);
      expect(lookupCheckout(database, moved)).toEqual({ ...recorded, checkoutPath: moved });
      expect(lookupCheckout(database, path)).toBeUndefined();
      expect(existsSync(repositoryIdentityLocation(path).identityPath)).toBe(false);
    } finally {
      chmodSync(join(existsSync(moved) ? moved : path, ".git"), 0o755);
    }
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


