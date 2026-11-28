import { execFileSync } from "node:child_process";
import { mkdirSync, readFileSync, realpathSync } from "node:fs";
import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { runSupportBundleCommand, type SupportBundle } from "../../../src/cli/support-bundle.js";
import { defaultRerailDatabasePath, openRerailDatabase } from "../../../src/engine/index.js";
import { SqliteSessionStore } from "../../../src/runtime/index.js";
import { startTestRepository } from "../../support/started-repository.js";
import { removeFolder } from "../../support/platform/remove-folder.js";

const temporaryDirectories: string[] = [];
const fileRerailHome = process.env.RERAIL_HOME;

afterEach(async () => {
  if (fileRerailHome === undefined) delete process.env.RERAIL_HOME;
  else process.env.RERAIL_HOME = fileRerailHome;
  await Promise.all(temporaryDirectories.splice(0).map(directory => removeFolder(directory)));
});

describe("rerail support-bundle for a session with an event whose stored text is no JSON", () => {
  it("reports the session with that event by its kind and time, and names it on the error output the command was given", async () => {
    const root = realpathSync(await mkdtemp(join(tmpdir(), "rerail-bundle-event-")));
    temporaryDirectories.push(root);
    execFileSync("git", ["init", "-q", "-b", "main", "repo"], { cwd: root });
    const home = join(root, "home");
    mkdirSync(home);
    process.env.RERAIL_HOME = home;
    const repo = join(root, "repo");
    const repositoryId = startTestRepository(repo);
    const store = new SqliteSessionStore(defaultRerailDatabasePath(), { repositoryId });
    const session = { host: "claude-code" as const, sessionId: "s1", cwd: repo };
    try {
      store.openSession(session);
      store.recordSteer(session, "r1", ["ts:no-any"]);
      store.recordSteer(session, "r2", ["acme:no-print"]);
    } finally {
      store.close();
    }
    // One byte changed, as a damaged file leaves it: the rule ids' "[" made "{".
    const database = openRerailDatabase(defaultRerailDatabasePath());
    try {
      database.$client.prepare("UPDATE events SET payload_json = replace(payload_json, '\"ruleIds\":[', '\"ruleIds\":{') WHERE event_id = ?")
        .run("steering-delivered:r2");
    } finally {
      database.$client.close();
    }
    let stderr = "";
    const output = join(root, "bundle.json");
    const exit = await runSupportBundleCommand(["--repo", repo, "--session", "s1", "--output", output], () => "usage", {
      packageVersion: "9.9.9",
      stdout: { write: () => true },
      stderr: { write: text => { stderr += text; return true; } },
    });
    expect(exit).toBe(0);
    const bundle = JSON.parse(readFileSync(output, "utf8")) as SupportBundle;
    expect(bundle.problems.filter(problem => problem.startsWith("session:"))).toEqual([]);
    expect(bundle.session?.timeline.filter(entry => entry.kind === "steering-delivered").map(({ at: _at, ...entry }) => entry)).toEqual([
      { kind: "steering-delivered", ruleIds: ["ts:no-any"] },
      { kind: "steering-delivered" },
    ]);
    expect(stderr.split("\n").filter(line => line.includes("event steering-delivered:r2 of session s1"))).toHaveLength(1);
  });
});
