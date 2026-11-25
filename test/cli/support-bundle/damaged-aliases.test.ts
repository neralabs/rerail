import { execFileSync } from "node:child_process";
import { mkdirSync, readFileSync, realpathSync } from "node:fs";
import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { runSupportBundleCommand, type SupportBundle } from "../../../src/cli/support-bundle.js";
import { defaultRerailDatabasePath, openRerailDatabase } from "../../../src/engine/index.js";
import { startTestRepository } from "../../support/started-repository.js";
import { removeFolder } from "../../support/platform/remove-folder.js";

const temporaryDirectories: string[] = [];
const fileRerailHome = process.env.RERAIL_HOME;

afterEach(async () => {
  if (fileRerailHome === undefined) delete process.env.RERAIL_HOME;
  else process.env.RERAIL_HOME = fileRerailHome;
  await Promise.all(temporaryDirectories.splice(0).map(directory => removeFolder(directory)));
});

describe("rerail support-bundle with a repository whose stored aliases do not read", () => {
  it("lists the repositories it reads and names the damaged one on the error output the command was given", async () => {
    const root = realpathSync(await mkdtemp(join(tmpdir(), "rerail-bundle-aliases-")));
    temporaryDirectories.push(root);
    execFileSync("git", ["init", "-q", "-b", "main", "repo"], { cwd: root });
    const home = join(root, "home");
    mkdirSync(home);
    process.env.RERAIL_HOME = home;
    const repo = join(root, "repo");
    const repositoryKey = startTestRepository(repo);
    // A hand edit past the CHECK on `repositories`.
    const database = openRerailDatabase(defaultRerailDatabasePath());
    try {
      database.$client.pragma("ignore_check_constraints = ON");
      database.$client.prepare("UPDATE repositories SET aliases_json = ? WHERE repository_key = ?").run('{"x":', repositoryKey);
    } finally {
      database.$client.close();
    }
    let stderr = "";
    const output = join(root, "bundle.json");
    const exit = await runSupportBundleCommand(["--repo", repo, "--output", output], () => "usage", {
      packageVersion: "9.9.9",
      stdout: { write: () => true },
      stderr: { write: text => { stderr += text; return true; } },
    });
    expect(exit).toBe(0);
    const bundle = JSON.parse(readFileSync(output, "utf8")) as SupportBundle;
    expect(bundle.problems.filter(problem => problem.startsWith("repositories:"))).toEqual([]);
    expect(stderr).toContain(`repository ${repositoryKey} reads with no aliases: the aliases_json of its row is not a JSON array of strings`);
  });
});
