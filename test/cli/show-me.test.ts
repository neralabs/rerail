import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import {
  checkoutStatus,
  codeRuleSchema,
  defaultRerailDatabasePath,
  listLibraryRules,
  listRepositoryRules,
  loadBundledCatalog,
  openRerailDatabase,
  recordCheckout,
  upsertLibraryRule,
} from "../../src/engine/index.js";
import { parseRuleDraft, testRuleDraft } from "../../src/cli/rule-draft.js";
import {
  runShowMeCommand,
  SHOW_ME_FILE,
  SHOW_ME_FIXED,
  SHOW_ME_RULE,
  SHOW_ME_RULE_ID,
  SHOW_ME_VIOLATING,
  type ShowMeStart,
} from "../../src/cli/show-me.js";
import { setUpTestRepository } from "../support/repository-rules.js";
import { startTestRepository } from "../support/started-repository.js";

const directories: string[] = [];
const fileRerailHome = process.env.RERAIL_HOME;

afterEach(async () => {
  process.env.RERAIL_HOME = fileRerailHome;
  await Promise.all(directories.splice(0).map(directory => rm(directory, { recursive: true, force: true })));
});

/** A git repository, with a Rerail home of its own: the library is per machine, so each test starts with an empty one. */
async function repository(): Promise<string> {
  const root = await mkdtemp(join(tmpdir(), "rerail-show-me-"));
  directories.push(root);
  execFileSync("git", ["init", "-q", "-b", "main", "repo"], { cwd: root });
  process.env.RERAIL_HOME = join(root, "home");
  mkdirSync(process.env.RERAIL_HOME);
  return join(root, "repo");
}

function run(args: string[]): { exit: number; stdout: string; stderr: string } {
  let stdout = "";
  let stderr = "";
  const exit = runShowMeCommand(args, () => "usage", {
    stdout: { write: text => (stdout += text) },
    stderr: { write: text => (stderr += text) },
  });
  return { exit, stdout, stderr };
}

/** Your library's rule ids, and `repo`'s repository: whether it is set up and the rules added to it. */
function rulesOf(repo: string) {
  const database = openRerailDatabase(defaultRerailDatabasePath());
  try {
    const checkout = recordCheckout(database, repo);
    return {
      library: listLibraryRules(database).map(rule => rule.document.rule.id).sort(),
      setUp: checkoutStatus(database, checkout).setUp,
      repository: listRepositoryRules(database, loadBundledCatalog(), checkout.repositoryKey).rules.map(rule => rule.source.id).sort(),
    };
  } finally {
    database.$client.close();
  }
}

const OTHER_RULE = {
  format: "rerail.policy/v1",
  id: "acme:no-console-log",
  revision: 1,
  mode: "steer",
  scope: { include: ["src/**/*.js"], exclude: [] },
  clauses: [{
    id: "no-console-log",
    forbid: { regex: { pattern: String.raw`console\.log`, source: "file.content" } },
    violation: { anchor: "witness.span", code: "NO_CONSOLE_LOG", message: "Use the logger." },
  }],
};

describe("rerail show-me", () => {
  it("refuses a repo where Rerail is not on, and changes nothing", async () => {
    const repo = await repository();
    const { exit, stderr } = run(["--repo", repo]);
    expect(exit).toBe(1);
    expect(stderr).toContain("Rerail is not on in");
    expect(rulesOf(repo)).toEqual({ library: [], setUp: false, repository: [] });
  });

  it("refuses a started repo that is not set up, and never sets it up", async () => {
    const repo = await repository();
    startTestRepository(repo);
    const { exit, stderr } = run(["--repo", repo]);
    expect(exit).toBe(1);
    expect(stderr).toContain("not set up yet");
    expect(rulesOf(repo)).toEqual({ library: [], setUp: false, repository: [] });
  });

  it("adds the demo rule next to the repo's own rules and leaves the demo file for the agent to write", async () => {
    const repo = await repository();
    setUpTestRepository(repo, { library: [{ kind: "code", rule: codeRuleSchema.parse(OTHER_RULE) }] });
    const { exit, stdout } = run(["--repo", repo, "--json"]);
    expect(exit).toBe(0);
    const start = JSON.parse(stdout) as ShowMeStart;
    expect(start).toMatchObject({ ruleId: SHOW_ME_RULE_ID, path: SHOW_ME_FILE, violating: SHOW_ME_VIOLATING, fixed: SHOW_ME_FIXED });
    expect(start.file.endsWith(`/${SHOW_ME_FILE}`)).toBe(true);
    expect(existsSync(start.file)).toBe(false);
    expect(rulesOf(repo)).toEqual({ library: [SHOW_ME_RULE_ID, OTHER_RULE.id].sort(), setUp: true, repository: [SHOW_ME_RULE_ID, OTHER_RULE.id].sort() });
  });

  it("cleans up the demo file and rule, keeps the repo's own rules, and is safe to run twice", async () => {
    const repo = await repository();
    setUpTestRepository(repo, { library: [{ kind: "code", rule: codeRuleSchema.parse(OTHER_RULE) }] });
    const start = JSON.parse(run(["--repo", repo, "--json"]).stdout) as ShowMeStart;
    writeFileSync(start.file, SHOW_ME_FIXED);

    const first = run(["--cleanup", "--repo", repo, "--json"]);
    expect(first.exit).toBe(0);
    expect(JSON.parse(first.stdout)).toEqual({ fileRemoved: true, ruleRemoved: true });
    expect(existsSync(start.file)).toBe(false);
    expect(rulesOf(repo)).toEqual({ library: [OTHER_RULE.id], setUp: true, repository: [OTHER_RULE.id] });

    const second = run(["--cleanup", "--repo", repo]);
    expect(second).toMatchObject({ exit: 0, stdout: "● Nothing to clean up: the demo left nothing behind.\n" });
  });

  it("clears what an interrupted demo left behind before starting again", async () => {
    const repo = await repository();
    setUpTestRepository(repo);
    const start = JSON.parse(run(["--repo", repo, "--json"]).stdout) as ShowMeStart;
    writeFileSync(start.file, SHOW_ME_VIOLATING);
    expect(run(["--repo", repo, "--json"]).exit).toBe(0);
    expect(existsSync(start.file)).toBe(false);
    expect(rulesOf(repo).repository).toEqual([SHOW_ME_RULE_ID]);
  });

  it("never touches a rerail-show-me.js the demo did not write", async () => {
    const repo = await repository();
    setUpTestRepository(repo);
    const file = join(repo, SHOW_ME_FILE);
    writeFileSync(file, "export const mine = 1;\n");

    const start = run(["--repo", repo]);
    expect(start.exit).toBe(1);
    expect(start.stderr).toContain("was not written by the demo");
    expect(rulesOf(repo).library).toEqual([]);

    expect(JSON.parse(run(["--cleanup", "--repo", repo, "--json"]).stdout)).toEqual({ fileRemoved: false, ruleRemoved: false });
    expect(readFileSync(file, "utf8")).toBe("export const mine = 1;\n");
  });

  it("never replaces or removes a rule someone else gave the demo's id", async () => {
    const repo = await repository();
    setUpTestRepository(repo);
    const theirs = { ...OTHER_RULE, id: SHOW_ME_RULE_ID };
    const database = openRerailDatabase(defaultRerailDatabasePath());
    try {
      upsertLibraryRule(database, loadBundledCatalog(), theirs, "code");
    } finally {
      database.$client.close();
    }

    const start = run(["--repo", repo]);
    expect(start.exit).toBe(1);
    expect(start.stderr).toContain("the demo did not add");
    expect(JSON.parse(run(["--cleanup", "--repo", repo, "--json"]).stdout)).toEqual({ fileRemoved: false, ruleRemoved: false });
    expect(rulesOf(repo).library).toEqual([SHOW_ME_RULE_ID]);
  });
});

describe("the demo rule", () => {
  it("catches the demo's broken greeting, passes its fix, and matches nothing outside the demo file", async () => {
    const repo = await repository();
    writeFileSync(join(repo, "greeting.js"), "export const greeting = () => \"Hello, world\";\n");
    const report = await testRuleDraft(parseRuleDraft({
      kind: "code",
      rule: SHOW_ME_RULE,
      cases: {
        caught: [{ path: SHOW_ME_FILE, content: SHOW_ME_VIOLATING }],
        passed: [{ path: SHOW_ME_FILE, content: SHOW_ME_FIXED }, { path: "src/greeting.js", content: "export const g = \"Hello, world\";\n" }],
      },
    }), repo);
    expect(report).toMatchObject({ ok: true, valid: true, repository: { matches: 0 } });
  });
});
