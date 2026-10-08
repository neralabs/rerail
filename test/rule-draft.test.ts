import { execFileSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { defaultRerailDatabasePath } from "../src/database.js";
import { loadBundledRules } from "../src/rule-bootstrap.js";
import { runRuleDraftCommand, type RuleDraftReport } from "../src/rule-draft.js";
import { resolveRepositoryContext, ScopedRuleCatalog } from "../src/scoped-rule-catalog.js";
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

function catalogFor(repo: string): ScopedRuleCatalog {
  return ScopedRuleCatalog.open(defaultRerailDatabasePath(), resolveRepositoryContext(repo));
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

  it("leaves the catalog unchanged", async () => {
    const repo = await repository({ "src/a.ts": "console.log(1);\n" });
    const before = catalogFor(repo);
    const rulesBefore = stableJsonText(before.list());
    before.close();
    const file = writeDraft(repo, { kind: "file", rule: noConsolePolicy() });

    await testReport(file, repo);

    const after = catalogFor(repo);
    try {
      expect(after.get("acme:no-console-log")).toBeUndefined();
      expect(stableJsonText(after.list())).toBe(rulesBefore);
    } finally {
      after.close();
    }
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
  it("stores the rule as an enabled project rule of the repo", async () => {
    const repo = await repository();
    const file = writeDraft(repo, {
      kind: "file",
      rule: noConsolePolicy(),
      cases: { caught: [{ path: "src/a.ts", content: "console.log(1);\n" }], passed: [] },
    });

    const { exit, stdout } = await run("add", [file, "--repo", repo]);

    expect(exit).toBe(0);
    expect(stdout).toMatch(/^● Added acme:no-console-log to rerail-rule-draft-\w+\.\n$/);
    const catalog = catalogFor(repo);
    try {
      expect(catalog.list({ applicability: "project" }).find(rule => rule.rule.id === "acme:no-console-log")).toMatchObject({
        kind: "file",
        enabled: true,
        applicability: "project",
        catalogOrigin: "user",
      });
    } finally {
      catalog.close();
    }
  });

  it("refuses to replace a rule by Rerail with the same id", async () => {
    const repo = await repository();
    const seeded = catalogFor(repo);
    let bundled;
    try {
      seeded.seed(loadBundledRules());
      bundled = seeded.list().find(rule => rule.catalogOrigin === "rerail" && rule.kind === "file");
    } finally {
      seeded.close();
    }
    if (bundled === undefined) throw new Error("expected a file rule by Rerail");
    const file = writeDraft(repo, { kind: "file", rule: { ...noConsolePolicy(), id: bundled.rule.id } });

    const { exit, stdout, stderr } = await run("add", [file, "--repo", repo]);

    expect(exit).toBe(1);
    expect(stdout).toBe("");
    expect(stderr).toContain(`${bundled.rule.id} is a rule by Rerail`);
    const catalog = catalogFor(repo);
    try {
      expect(catalog.get(bundled.rule.id)).toMatchObject({ catalogOrigin: "rerail", payloadHash: bundled.payloadHash });
    } finally {
      catalog.close();
    }
  });
});
