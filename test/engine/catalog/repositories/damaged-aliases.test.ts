import { execFileSync } from "node:child_process";
import { mkdirSync } from "node:fs";
import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { createRerailApi, DatabaseApiStores, operations } from "../../../../src/api/index.js";
import { listRepositories, lookupCheckout, recordCheckout } from "../../../../src/engine/catalog/repository-registry.js";
import { exportRulesDump } from "../../../../src/engine/catalog/rule-bootstrap.js";
import { openRerailDatabase, type RerailDatabase } from "../../../../src/engine/storage/database.js";
import { removeFolder } from "../../../support/platform/remove-folder.js";

const temporaryDirectories: string[] = [];
const openDatabases: RerailDatabase[] = [];
afterEach(async () => {
  for (const database of openDatabases.splice(0)) database.$client.close();
  await Promise.all(temporaryDirectories.splice(0).map(path => removeFolder(path)));
});

/** A database holding three repositories, each with one checkout: `web` with two aliases, `api` and `cli` with none. */
async function threeRepositories(): Promise<RerailDatabase> {
  const root = await mkdtemp(join(tmpdir(), "rerail-damaged-aliases-"));
  temporaryDirectories.push(root);
  const database = openRerailDatabase(join(root, "rerail.sqlite"));
  openDatabases.push(database);
  const insert = database.$client.prepare(`INSERT INTO repositories (repository_key, display_name, aliases_json, created_at, updated_at)
    VALUES (?, ?, ?, 1, 2)`);
  insert.run("remote:github.com/acme/web", "web", JSON.stringify(["remote:github.com/acme/old-web", "clone:git:web"]));
  insert.run("remote:github.com/acme/api", "api", "[]");
  insert.run("remote:github.com/acme/cli", "cli", "[]");
  const checkout = database.$client.prepare("INSERT INTO checkouts (checkout_path, clone_id, repository_key, created_at, last_seen_at) VALUES (?, ?, ?, 1, 2)");
  for (const name of ["web", "api", "cli"]) checkout.run(join(root, name), `git:${name}`, `remote:github.com/acme/${name}`);
  return database;
}

/** Writes `text` over one repository's aliases, past the CHECK on them, as a damaged file or a hand edit leaves it. */
function damage(database: RerailDatabase, repositoryKey: string, text: string): void {
  database.$client.pragma("ignore_check_constraints = ON");
  try {
    database.$client.prepare("UPDATE repositories SET aliases_json = ? WHERE repository_key = ?").run(text, repositoryKey);
  } finally {
    database.$client.pragma("ignore_check_constraints = OFF");
  }
}

/** A log that keeps each line it is given. */
function lines(): { log: { write: (line: string) => boolean }; written: string[] } {
  const written: string[] = [];
  return { log: { write: line => written.push(line) > 0 }, written };
}

describe("the repository list with a repository whose stored aliases do not read", () => {
  it.each([
    ["does not parse", '{"x":', "Unexpected end of JSON input"],
    ["is another JSON value", '{"x":1}', "it parses to another JSON value"],
    ["holds an item that is no string", '["remote:github.com/acme/old-api", 3]', "an item of it is another JSON value"],
  ])("lists every repository, in order, when one's aliases_json %s: that one with no aliases, each other as stored, and the log names it once", async (_, text, why) => {
    const database = await threeRepositories();
    const undamaged = listRepositories(database, lines().log);
    damage(database, "remote:github.com/acme/api", text);
    const { log, written } = lines();
    const listed = listRepositories(database, log);
    expect(listed).toEqual(undamaged);
    expect(listed.map(repository => [repository.displayName, repository.aliases, repository.checkouts.length])).toEqual([
      ["api", [], 1], ["cli", [], 1], ["web", ["remote:github.com/acme/old-web", "clone:git:web"], 1],
    ]);
    expect(listRepositories(database, log)).toEqual(undamaged);
    expect(written).toEqual([`rerail: ${database.$client.name}: repository remote:github.com/acme/api reads with no aliases: the aliases_json `
      + `of its row is not a JSON array of strings (${why}), which only a damaged file or a hand edit leaves.\n`]);
    // Nothing is written back: the text stays where a person can read and mend it.
    expect(database.$client.prepare("SELECT aliases_json FROM repositories WHERE repository_key = ?").pluck().get("remote:github.com/acme/api")).toBe(text);
  });

  it("reads a repository whose aliases are cut short with none, and each of its other fields as stored", async () => {
    const database = await threeRepositories();
    damage(database, "remote:github.com/acme/web", '["remote:github.com/acme/old-web"');
    const listed = listRepositories(database, lines().log);
    expect(listed.map(repository => [repository.repositoryKey, repository.aliases])).toEqual([
      ["remote:github.com/acme/api", []], ["remote:github.com/acme/cli", []], ["remote:github.com/acme/web", []],
    ]);
    expect(listed.find(repository => repository.displayName === "web")).toMatchObject({ configuredAt: null, started: false, createdAt: 1, updatedAt: 2 });
  });
});

const CLI = "remote:github.com/acme/cli";

function git(cwd: string, ...args: string[]): void {
  execFileSync("git", args, { cwd, stdio: "ignore" });
}

/**
 * `cli` given the alias `old-cli`, and a clone of `old-cli` Rerail has not recorded, with one commit and a worktree:
 * the clone's and the worktree's paths. The repositories read in the order they were added, so a lookup reads `api`'s
 * aliases before it reaches `cli`'s.
 */
function newCloneOfAnAlias(database: RerailDatabase): [string, string] {
  database.$client.prepare("UPDATE repositories SET aliases_json = ? WHERE repository_key = ?").run('["remote:github.com/acme/old-cli"]', CLI);
  const clone = join(dirname(database.$client.name), "new-clone");
  mkdirSync(clone);
  git(clone, "init", "-q", "-b", "main");
  git(clone, "-c", "user.name=Rerail", "-c", "user.email=rerail@example.invalid", "commit", "-q", "--allow-empty", "-m", "base");
  git(clone, "remote", "add", "origin", "https://github.com/acme/old-cli.git");
  const worktree = join(dirname(clone), "new-worktree");
  git(clone, "worktree", "add", "-q", "-b", "feature", worktree);
  return [clone, worktree];
}

/** Spies on this thread's stderr, where a lookup writes the line that names a damaged row: the lines naming `api`. */
function spyOnStderr(): { named: () => string[]; restore: () => void } {
  const stderr = vi.spyOn(process.stderr, "write").mockReturnValue(true);
  return {
    named: () => stderr.mock.calls.map(([chunk]) => String(chunk)).filter(line => line.includes("repository remote:github.com/acme/api reads with no aliases")),
    restore: () => stderr.mockRestore(),
  };
}

describe("a checkout not recorded yet, while another repository's stored aliases do not read", () => {
  it.each([["does not parse", '{"x":'], ["is another JSON value", '{"x":1}']])(
    "is found and recorded by its repository's alias when another's aliases_json %s, and the list's line names that one once",
    async (_, text) => {
      const database = await threeRepositories();
      const [clone, worktree] = newCloneOfAnAlias(database);
      damage(database, "remote:github.com/acme/api", text);
      const stderr = spyOnStderr();
      try {
        expect([clone, worktree].map(path => lookupCheckout(database, path)?.repositoryKey)).toEqual([CLI, CLI]);
        expect(recordCheckout(database, worktree).repositoryKey).toBe(CLI);
        listRepositories(database);
        expect(stderr.named()).toHaveLength(1);
      } finally {
        stderr.restore();
      }
    },
  );

  it("answers GET /api/repository?checkout= for a new clone with its repository", async () => {
    const database = await threeRepositories();
    const [clone] = newCloneOfAnAlias(database);
    damage(database, "remote:github.com/acme/api", '{"x":');
    const path = database.$client.name;
    const stores = new DatabaseApiStores(path, {
      health: () => ({ pid: process.pid, database: { path, exists: true }, native: { ok: true } }), origin: () => "http://127.0.0.1:4737",
    });
    const stderr = spyOnStderr();
    try {
      const response = await createRerailApi(stores, operations, { version: "0.0.0-test" }).request(`/api/repository?checkout=${encodeURIComponent(clone)}`);
      expect(response.status).toBe(200);
      expect(await response.json()).toMatchObject({ repository: { repositoryKey: CLI } });
    } finally {
      stderr.restore();
      stores.close();
    }
  });

  it("exports every repository's rules, the one whose aliases do not read with none", async () => {
    const database = await threeRepositories();
    damage(database, "remote:github.com/acme/api", '{"x":');
    const stderr = spyOnStderr();
    try {
      expect(exportRulesDump(database).repositories.map(repository => [repository.displayName, repository.aliases])).toEqual([
        ["api", []], ["cli", []], ["web", ["remote:github.com/acme/old-web", "clone:git:web"]],
      ]);
      expect(stderr.named()).toHaveLength(1);
    } finally {
      stderr.restore();
    }
  });
});
