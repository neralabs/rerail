import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { listRepositories } from "../../../../src/engine/catalog/repository-registry.js";
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
