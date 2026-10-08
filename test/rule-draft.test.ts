import { execFileSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { loadBundledCatalog } from "../src/bundled-rules.js";
import { defaultRerailDatabasePath, openRerailDatabase } from "../src/database.js";
import { recordCheckout } from "../src/repository-registry.js";
import { checkoutStatus } from "../src/repository-start.js";
import { listRepositoryRules, setUpRepository } from "../src/rule-configuration.js";
import { runRuleDraftCommand, type RuleDraftReport } from "../src/rule-draft.js";
import { listLibraryRules } from "../src/rule-library.js";
import { stableJsonText } from "../src/stable-json.js";

const directories: string[] = [];

afterEach(async () => {
  await Promise.all(directories.splice(0).map(directory => rm(directory, { recursive: true, force: true })));
});

async function repository(files: Record<string, string> = {}): Promise<string> {
  const root = await mkdtemp(join(tmpdir(), "rerail-rule-draft-"));
  directories.push(root);
  execFileSync("git", ["init", "-q", "-b", "main"], { cwd: root });
  for (const [path, content] of Object.entries(files)) {
    mkdirSync(dirname(join(root, path)), { recursive: true });
    writeFileSync(join(root, path), content);
  }
  return root;
}

function noConsolePolicy(pattern = String.raw`console\.log`) {
  return {
    format: "rerail.policy/v1",
    id: "acme:no-console-log",
    revision: 1,
    mode: "steer",
    description: "Use the logger, not console.log.",
    scope: { include: ["src/**/*.ts"], exclude: [] },
    clauses: [{
      id: "no-console-log",
      forbid: { regex: { pattern, source: "file.content" } },
      violation: { anchor: "witness.span", code: "NO_CONSOLE_LOG", message: "Use the logger." },
    }],
  };
}

function writeDraft(root: string, draft: unknown): string {
  const file = join(root, "draft.json");
  writeFileSync(file, JSON.stringify(draft));
  return file;
}

async function run(action: "test" | "add", args: string[]): Promise<{ exit: number; stdout: string; stderr: string }> {
  let stdout = "";
  let stderr = "";
  const exit = await runRuleDraftCommand(action, args, () => "usage", {
    stdout: { write: text => (stdout += text) },
    stderr: { write: text => (stderr += text) },
  });
  return { exit, stdout, stderr };
}

async function testReport(file: string, repo: string): Promise<{ exit: number; report: RuleDraftReport }> {
  const { exit, stdout } = await run("test", [file, "--repo", repo, "--json"]);
  return { exit, report: JSON.parse(stdout) as RuleDraftReport };
}

/** Your library, and `repo`'s repository: whether it is set up and the rules added to it, by id and folder. */
function rulesOf(repo: string) {
  const database = openRerailDatabase(defaultRerailDatabasePath());
  try {
    const checkout = recordCheckout(database, repo);
    return {
      library: listLibraryRules(database).map(rule => rule.document.rule.id),
      setUp: checkoutStatus(database, checkout).setUp,
      repository: listRepositoryRules(database, loadBundledCatalog(), checkout.repositoryKey).rules
        .map(rule => ({ id: rule.source.id, kind: rule.source.kind, folder: rule.attachmentFolder, on: rule.on })),
    };
  } finally {
    database.$client.close();
  }
}

/** Set `repo`'s repository up with nothing in it. */
function setUpEmpty(repo: string): void {
  const database = openRerailDatabase(defaultRerailDatabasePath());
  try {
    setUpRepository(database, recordCheckout(database, repo).repositoryKey, { packs: [] });
  } finally {
    database.$client.close();
  }
}

describe("rerail rules test", () => {
  it("passes when the rule catches its caught cases and passes its passed cases", async () => {
    const repo = await repository();
    const file = writeDraft(repo, {
      kind: "file",
      rule: noConsolePolicy(),
      cases: {
        caught: [{ path: "src/a.ts", content: "console.log(1);\n" }],
        passed: [{ path: "src/b.ts", content: "logger.info(1);\n" }, { path: "lib/c.ts", content: "console.log(1);\n" }],
      },
    });

    const { exit, report } = await testReport(file, repo);

    expect(exit).toBe(0);
    expect(report).toMatchObject({ ok: true, valid: true });
    expect(report.cases.map(({ expected, caught, ok }) => ({ expected, caught, ok }))).toEqual([
      { expected: "caught", caught: true, ok: true },
      { expected: "passed", caught: false, ok: true },
      { expected: "passed", caught: false, ok: true },
    ]);
  });

  it("fails when a case does not match its expectation", async () => {
    const repo = await repository();
    const file = writeDraft(repo, {
      kind: "file",
      rule: noConsolePolicy(),
      cases: { caught: [{ path: "src/a.ts", content: "logger.info(1);\n" }], passed: [] },
    });

    const { exit, report } = await testReport(file, repo);

    expect(exit).toBe(1);
    expect(report).toMatchObject({ ok: false, valid: true });
    expect(report.cases[0]).toMatchObject({ expected: "caught", caught: false, ok: false, label: "src/a.ts" });
  });

  it("reports a policy the Engine cannot compile as not valid", async () => {
    const repo = await repository();
    const file = writeDraft(repo, { kind: "file", rule: noConsolePolicy("(unclosed") });

    const { exit, report } = await testReport(file, repo);

    expect(exit).toBe(1);
    expect(report).toMatchObject({ ok: false, valid: false, cases: [], repository: null });
    expect(report.error).toContain("acme:no-console-log does not compile");
  });

  it("reports a rule body that does not match its kind as not valid", async () => {
    const repo = await repository();
    const file = writeDraft(repo, { kind: "conversation", rule: noConsolePolicy() });

    const { exit, report } = await testReport(file, repo);

    expect(exit).toBe(1);
    expect(report).toMatchObject({ valid: false, error: expect.stringContaining("does not match kind conversation") });
  });

  it("counts the rule's matches across the repo's files it covers, ignored directories excluded", async () => {
    const repo = await repository({
      "src/a.ts": "const a = 1;\n  console.log(a);\nconsole.log(a);\n",
      "src/nested/b.ts": "console.log('b');\n",
      "src/clean.ts": "logger.info(1);\n",
      "lib/outside.ts": "console.log('outside scope');\n",
      "src/node_modules/dep.ts": "console.log('ignored');\n",
    });
    const file = writeDraft(repo, { kind: "file", rule: noConsolePolicy() });

    const { exit, report } = await testReport(file, repo);

    expect(exit).toBe(0);
    expect(report.repository).toEqual({
      matches: 3,
      examples: [
        { path: "src/a.ts", line: 2, column: 3, text: "console.log(a);" },
        { path: "src/a.ts", line: 3, column: 1, text: "console.log(a);" },
        { path: "src/nested/b.ts", line: 1, column: 1, text: "console.log('b');" },
      ],
    });
  });

  it("writes nothing: no library rule, and the repository is left as it was", async () => {
    const repo = await repository({ "src/a.ts": "console.log(1);\n" });
    const before = stableJsonText(rulesOf(repo));
    const file = writeDraft(repo, { kind: "file", rule: noConsolePolicy() });

    await testReport(file, repo);

    expect(stableJsonText(rulesOf(repo))).toBe(before);
    expect(rulesOf(repo).library).not.toContain("acme:no-console-log");
  });

  it("runs a tool-call rule's cases and skips the repo pass", async () => {
    const repo = await repository();
    const file = writeDraft(repo, {
      kind: "conversation",
      rule: {
        id: "acme:no-force-push",
        mode: "steer",
        match: { field: "input.command", regex: String.raw`\bgit\s+push\b[^\n]*--force\b` },
        sample: { source: "tool:Bash", boundary: "tool-call" },
        action: { content: "Do not force-push." },
        repeat: { mode: "always" },
      },
      cases: {
        caught: [{ source: "tool:Bash", boundary: "tool-call", text: "git push --force", input: stableJsonText({ command: "git push --force origin main" }) }],
        passed: [{ source: "tool:Bash", boundary: "tool-call", text: "git push", input: stableJsonText({ command: "git push origin main" }) }],
      },
    });

    const { exit, report } = await testReport(file, repo);

    expect(exit).toBe(0);
    expect(report.repository).toBeNull();
    expect(report.cases.map(({ caught, ok }) => ({ caught, ok }))).toEqual([{ caught: true, ok: true }, { caught: false, ok: true }]);
  });
});

describe("rerail rules add", () => {
  it("stores the rule in the library and adds it, on, to the whole of a set-up repository", async () => {
    const repo = await repository();
    setUpEmpty(repo);
    const file = writeDraft(repo, {
      kind: "file",
      rule: noConsolePolicy(),
      cases: { caught: [{ path: "src/a.ts", content: "console.log(1);\n" }], passed: [] },
    });

    const { exit, stdout } = await run("add", [file, "--repo", repo]);

    expect(exit).toBe(0);
    expect(stdout).toMatch(/^● Added acme:no-console-log to rerail-rule-draft-\w+\.\n$/);
    const after = rulesOf(repo);
    expect(after.library).toContain("acme:no-console-log");
    expect(after).toMatchObject({
      setUp: true,
      repository: [{ id: "acme:no-console-log", kind: "library_rule", folder: "", on: true }],
    });
  });

  it("adds the rule at the folder the draft names", async () => {
    const repo = await repository();
    setUpEmpty(repo);
    const file = writeDraft(repo, { kind: "file", rule: noConsolePolicy(), folder: "web" });

    const { exit, stdout } = await run("add", [file, "--repo", repo]);

    expect(exit).toBe(0);
    expect(stdout).toContain("● Added acme:no-console-log to rerail-rule-draft-");
    expect(stdout).toContain("/web.\n");
    expect(rulesOf(repo).repository).toEqual([{ id: "acme:no-console-log", kind: "library_rule", folder: "web", on: true }]);
  });

  it("sets a repository that is not set up up with only the new rule", async () => {
    const repo = await repository();
    const file = writeDraft(repo, { kind: "file", rule: noConsolePolicy() });

    const { exit, stdout } = await run("add", [file, "--repo", repo]);

    expect(exit).toBe(0);
    expect(stdout).toContain("was not set up, so it is now set up with this rule.");
    expect(rulesOf(repo)).toMatchObject({
      setUp: true,
      repository: [{ id: "acme:no-console-log", kind: "library_rule", folder: "", on: true }],
    });
  });

  it("refuses a draft that names a repository other than the checkout's", async () => {
    const repo = await repository();
    const file = writeDraft(repo, { kind: "file", rule: noConsolePolicy(), repository: "remote:github.com/acme/other" });

    const { exit, stderr } = await run("add", [file, "--repo", repo]);

    expect(exit).toBe(1);
    expect(stderr).toContain("cannot name repository");
    expect(rulesOf(repo)).toMatchObject({ setUp: false, repository: [] });
  });

  it("refuses to replace a rule by Rerail with the same id", async () => {
    const repo = await repository();
    const bundled = [...loadBundledCatalog().documents.values()].find(document => document.kind === "file");
    if (bundled === undefined) throw new Error("expected a file rule by Rerail");
    const file = writeDraft(repo, { kind: "file", rule: { ...noConsolePolicy(), id: bundled.rule.id } });

    const { exit, stdout, stderr } = await run("add", [file, "--repo", repo]);

    expect(exit).toBe(1);
    expect(stdout).toBe("");
    expect(stderr).toContain(`${bundled.rule.id} is a rule by Rerail, so it was not replaced`);
    const after = rulesOf(repo);
    expect(after.library).not.toContain(bundled.rule.id);
    expect(after).toMatchObject({ setUp: false, repository: [] });
  });
});
