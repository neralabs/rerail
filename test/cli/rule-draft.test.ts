import { execFileSync } from "node:child_process";
import { eq } from "drizzle-orm";
import { mkdirSync, writeFileSync } from "node:fs";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import {
  getLibraryRule,
  loadRuleCatalog,
  defaultRerailDatabasePath,
  openRerailDatabase,
  recordCheckout,
  listRepositoryRules,
  setUpRepository,
  listLibraryRules,
  stableJsonText,
  checkoutStatus,
  repositories,
} from "../../src/engine/index.js";
import { runRuleDraftCommand, type RuleDraftReport } from "../../src/cli/rule-draft.js";
import { BLOCK_STEERS_NOTE } from "../../src/engine/index.js";
import { installPack } from "../support/installed-packs.js";
import { linkFolder, symlinkOrSkip } from "../support/platform/links.js";

// Core ships no packs; these tests add, configure and copy the complexity pack's rules.
installPack("complexity");

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

/** The JSON report and the text one `rerail rules test` prints for the same draft. */
async function testReportText(file: string, repo: string): Promise<{ exit: number; report: RuleDraftReport; stdout: string }> {
  const { exit, report } = await testReport(file, repo);
  const { stdout } = await run("test", [file, "--repo", repo]);
  return { exit, report, stdout };
}

/** Your library, and `repo`'s repository: whether it is set up and the rules added to it, by id and folder. */
function rulesOf(repo: string) {
  const database = openRerailDatabase(defaultRerailDatabasePath());
  try {
    const checkout = recordCheckout(database, repo);
    return {
      library: listLibraryRules(database).map(rule => rule.document.rule.id),
      setUp: checkoutStatus(database, checkout).setUp,
      repository: listRepositoryRules(database, loadRuleCatalog(), checkout.repositoryKey).rules
        .map(rule => ({ id: rule.source.id, kind: rule.source.kind, folder: rule.attachmentFolder, on: rule.on })),
    };
  } finally {
    database.$client.close();
  }
}

/** The mode library rule `id`'s stored content sets. */
function getLibraryRuleMode(id: string): string | undefined {
  const database = openRerailDatabase(defaultRerailDatabasePath());
  try {
    return getLibraryRule(database, id)?.document.rule.mode;
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
      kind: "code",
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
      kind: "code",
      rule: noConsolePolicy(),
      cases: { caught: [{ path: "src/a.ts", content: "logger.info(1);\n" }], passed: [] },
    });

    const { exit, report } = await testReport(file, repo);

    expect(exit).toBe(1);
    expect(report).toMatchObject({ ok: false, valid: true });
    expect(report.cases[0]).toMatchObject({ expected: "caught", caught: false, ok: false, label: "src/a.ts" });
  });

  it("reads a case's own text into a Next app's server graph, not the repository's file at its path", async () => {
    // The repository holds a client page; the caught case is a server page at the same path, the passed one a new
    // client page.
    const repo = await repository({
      "package.json": '{ "dependencies": { "next": "16.0.0" } }\n',
      "app/page.tsx": "'use client';\nexport default function Page() {}\n",
    });
    const file = writeDraft(repo, {
      kind: "code",
      rule: {
        format: "rerail.policy/v1", id: "acme:server-graph", revision: 1, mode: "steer",
        scope: { include: ["**/*.tsx"], exclude: [] },
        clauses: [{
          id: "server",
          forbid: { ast: { language: "typescript", file: { package: { serverGraph: true } }, rule: { kind: "program" } } },
          violation: { anchor: "witness.span", code: "SERVER", message: "A server module." },
        }],
      },
      cases: {
        caught: [{ path: "app/page.tsx", content: "export default function Page() {}\n" }],
        passed: [{ path: "app/new/page.tsx", content: "'use client';\nexport default function Page() {}\n" }],
      },
    });

    const { report } = await testReport(file, repo);

    expect(report.cases.map(({ expected, caught, ok }) => ({ expected, caught, ok }))).toEqual([
      { expected: "caught", caught: true, ok: true },
      { expected: "passed", caught: false, ok: true },
    ]);
  });

  it("reports a policy the Engine cannot compile as not valid", async () => {
    const repo = await repository();
    const file = writeDraft(repo, { kind: "code", rule: noConsolePolicy("(unclosed") });

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

  it("names the field a code rule lacks, rather than the union of rule kinds failing", async () => {
    const repo = await repository();
    const { exclude: _exclude, ...scope } = noConsolePolicy().scope;
    const file = writeDraft(repo, { kind: "code", rule: { ...noConsolePolicy(), scope } });

    const { exit, report } = await testReport(file, repo);

    expect(exit).toBe(1);
    expect(report).toMatchObject({ valid: false, error: expect.stringContaining("rule.scope.exclude:") });
  });

  it("counts the rule's matches across the repo's files it covers, ignored directories excluded", async () => {
    const repo = await repository({
      "src/a.ts": "const a = 1;\n  console.log(a);\nconsole.log(a);\n",
      "src/nested/b.ts": "console.log('b');\n",
      "src/clean.ts": "logger.info(1);\n",
      "lib/outside.ts": "console.log('outside scope');\n",
      "src/node_modules/dep.ts": "console.log('ignored');\n",
    });
    const file = writeDraft(repo, { kind: "code", rule: noConsolePolicy() });

    const { exit, report } = await testReport(file, repo);

    expect(exit).toBe(0);
    expect(report.repository).toEqual({
      matches: [
        { path: "src/a.ts", line: 2, column: 3, text: "console.log(a);" },
        { path: "src/a.ts", line: 3, column: 1, text: "console.log(a);" },
        { path: "src/nested/b.ts", line: 1, column: 1, text: "console.log('b');" },
      ],
      unchecked: { files: [], unattributed: [] },
      binary: 0,
      leftOut: 0,
    });
  });

  it("lists every match in the JSON report, and the first five in the text one", async () => {
    const repo = await repository({ "src/a.ts": "console.log(1);\n".repeat(7) });
    const file = writeDraft(repo, { kind: "code", rule: noConsolePolicy() });

    const { report, stdout } = await testReportText(file, repo);

    expect(report.repository?.matches.map(match => match.line)).toEqual([1, 2, 3, 4, 5, 6, 7]);
    expect(stdout).toContain("  Repo: 7 places match.\n    src/a.ts:1:1  console.log(1);");
    expect(stdout).toContain("    src/a.ts:5:1  console.log(1);\n    … and 2 more.");
  });

  it("reports every match once across a repo too large for one request, cross-file clauses included", async () => {
    // 70 files of 600 KiB: more than the 40 MiB one request to the Engine may carry. The `select_when` holds where its
    // `exists` witness is the last file, sent long after the first, which it selects.
    const padding = `// ${"x".repeat(1021)}\n`.repeat(600);
    const files: Record<string, string> = {};
    for (let index = 0; index < 70; index += 1) {
      const name = `src/f${String(index).padStart(2, "0")}.ts`;
      files[name] = `${padding}console.log(${index});\n${index === 0 ? "debugger;\n" : ""}${index === 69 ? "// DEBUG_BUILD\n" : ""}`;
    }
    const repo = await repository(files);
    const policy = noConsolePolicy();
    const file = writeDraft(repo, {
      kind: "code",
      rule: {
        ...policy,
        clauses: [...policy.clauses, {
          id: "debugger-in-debug-build",
          forbid: {
            select_when: {
              exists: { regex: { pattern: "DEBUG_BUILD", source: "file.content" } },
              select: { ast: { language: "typescript", rule: { kind: "debugger_statement" } } },
            },
          },
          violation: { anchor: "witness.span", code: "DEBUGGER", message: "No debugger." },
        }],
      },
    });

    const { exit, report } = await testReport(file, repo);

    expect(exit).toBe(0);
    expect(report.repository).toMatchObject({ unchecked: { files: [], unattributed: [] }, binary: 0, leftOut: 0 });
    expect(report.repository?.matches).toEqual([
      ...Object.keys(files).flatMap((path, index) => [
        { path, line: 601, column: 1, text: `console.log(${index});` },
        ...(index === 0 ? [{ path, line: 602, column: 1, text: "debugger;" }] : []),
      ]),
    ]);
  });

  it("reads files no language lists as text for a regex rule, skipping binary ones", async () => {
    const repo = await repository({
      Justfile: "release:\n\techo TODO\n",
      "Cargo.lock": "# TODO\n",
      NOTICE: "Nothing to do.\n",
      "assets/logo.png": "\u0000PNG TODO\n",
      "src/blob.ts": "\u0000TODO\n",
    });
    const drafts = await mkdtemp(join(tmpdir(), "rerail-rule-draft-file-"));
    directories.push(drafts);
    const file = writeDraft(drafts, {
      kind: "code",
      rule: { ...noConsolePolicy("TODO"), scope: { include: ["**"], exclude: [] } },
      cases: { caught: [{ path: "LICENSE", content: "TODO\n" }], passed: [{ path: ".gitattributes", content: "* text=auto\n" }] },
    });

    const { exit, report, stdout } = await testReportText(file, repo);

    expect(exit).toBe(0);
    expect(report.cases.map(({ label, caught, ok }) => ({ label, caught, ok }))).toEqual([
      { label: "LICENSE", caught: true, ok: true },
      { label: ".gitattributes", caught: false, ok: true },
    ]);
    expect(report.repository).toEqual({
      matches: [
        { path: "Cargo.lock", line: 1, column: 3, text: "# TODO" },
        { path: "Justfile", line: 2, column: 7, text: "echo TODO" },
      ],
      unchecked: { files: [], unattributed: [] },
      binary: 2,
      leftOut: 0,
    });
    expect(stdout).toContain("2 files were skipped as binary: no rule reads a file holding a NUL byte.");
  });

  it("never reads a file no language lists for a rule with no regex or file clause", async () => {
    const repo = await repository({ Justfile: "debugger\n" });
    const file = writeDraft(repo, {
      kind: "code",
      rule: {
        ...noConsolePolicy(),
        scope: { include: ["**"], exclude: [] },
        clauses: [{
          id: "debugger",
          forbid: { ast: { language: "typescript", rule: { kind: "debugger_statement" } } },
          violation: { anchor: "witness.span", code: "DEBUGGER", message: "No debugger." },
        }],
      },
      cases: { caught: [], passed: [{ path: "Justfile", content: "debugger;\n" }] },
    });

    const { report } = await testReport(file, repo);

    expect(report.cases).toEqual([{
      expected: "passed",
      caught: false,
      ok: true,
      label: "Justfile (not read: no language lists this file, and this rule has no regex or file clause)",
    }]);
    expect(report.repository).toMatchObject({ matches: [], binary: 0 });
  });

  it.each([
    ["a file.path regex", { include: ["**"], exclude: [] }, { regex: { source: "file.path", pattern: String.raw`\.orig$` } }],
    ["a file query", { include: ["**/*.orig"], exclude: [] }, { file: {} }],
  ])("reports a match on the file's path, through %s, with no line", async (_name, scope, forbid) => {
    const repo = await repository({ "src/a.ts.orig": "console.log(1);\n", "src/a.ts": "const a = 1;\n" });
    const drafts = await mkdtemp(join(tmpdir(), "rerail-rule-draft-file-"));
    directories.push(drafts);
    const file = writeDraft(drafts, {
      kind: "code",
      rule: {
        ...noConsolePolicy(),
        scope,
        clauses: [{ id: "orig", forbid, violation: { anchor: "witness.span", code: "ORIG", message: "Delete merge leftovers." } }],
      },
    });

    const { report, stdout } = await testReportText(file, repo);

    expect(report.repository?.matches).toEqual([{ path: "src/a.ts.orig" }]);
    expect(stdout).toContain("  Repo: 1 place matches.\n    src/a.ts.orig  (the whole file: no line)");
  });

  it("never reads a hidden, sensitive or Git-ignored file; a person turning followGitignore off lets ignored files in", async () => {
    const repo = await repository({
      ".env": "TOKEN=SECRET\n",
      "config/.secrets.json": "{ \"token\": \"SECRET\" }\n",
      id_rsa: "SECRET\n",
      "keys/server.pem": "SECRET\n",
      ".github/workflows/ci.yml": "token: SECRET\n",
      ".gitignore": "*.json\n",
      "local.json": "{ \"token\": \"SECRET\" }\n",
      "app.json": "{ \"token\": \"SECRET\" }\n",
    });
    // Tracked, so Git never ignores it whatever `.gitignore` says.
    execFileSync("git", ["add", "--force", "app.json"], { cwd: repo });
    const drafts = await mkdtemp(join(tmpdir(), "rerail-rule-draft-file-"));
    directories.push(drafts);
    const file = writeDraft(drafts, {
      kind: "code",
      rule: { ...noConsolePolicy("SECRET"), scope: { include: ["**"], exclude: [] } },
    });

    const following = await testReportText(file, repo);

    // A regex rule reads every file, so each withheld one counts: .env, config/.secrets.json, id_rsa, keys/server.pem,
    // the .github folder, .gitignore and the ignored local.json.
    expect(following.report.repository).toMatchObject({ matches: [{ path: "app.json" }], leftOut: 7 });
    expect(following.stdout).toContain("7 files or folders were left out unread: hidden, sensitive by name, or ignored by Git.");
    expect(following.stdout).not.toMatch(/\.env|\.secrets|id_rsa|server\.pem|ci\.yml|local\.json/);

    const database = openRerailDatabase(defaultRerailDatabasePath());
    try {
      database.update(repositories).set({ followGitignore: false })
        .where(eq(repositories.repositoryKey, recordCheckout(database, repo).repositoryKey)).run();
    } finally {
      database.$client.close();
    }
    const { report } = await testReport(file, repo);

    expect(report.repository).toMatchObject({ matches: [{ path: "app.json" }, { path: "local.json" }], leftOut: 6 });
  });

  it("never follows a symlink out of the checkout, to a file or a folder", async context => {
    const outside = await mkdtemp(join(tmpdir(), "rerail-rule-draft-outside-"));
    directories.push(outside);
    mkdirSync(join(outside, "folder"));
    writeFileSync(join(outside, "secret.ts"), "console.log('outside');\n");
    writeFileSync(join(outside, "folder", "b.ts"), "console.log('outside folder');\n");
    const repo = await repository({ "src/a.ts": "console.log('inside');\n" });
    symlinkOrSkip(context, join(outside, "secret.ts"), join(repo, "src", "linked.ts"));
    linkFolder(join(outside, "folder"), join(repo, "src", "linked"));
    const file = writeDraft(repo, { kind: "code", rule: noConsolePolicy() });

    const { exit, report } = await testReport(file, repo);

    expect(exit).toBe(0);
    expect(report.repository).toMatchObject({ matches: [{ path: "src/a.ts", text: "console.log('inside');" }] });
  });

  it("tests a worktree's rule document on its own, with no cases, over the repo; rules add still needs a draft", async () => {
    const repo = await repository({ "src/a.ts": "console.log(1);\n" });
    mkdirSync(join(repo, ".rerail", "rules"), { recursive: true });
    const file = join(repo, ".rerail", "rules", "no-console-log.json");
    writeFileSync(file, JSON.stringify(noConsolePolicy()));

    const { exit, report, stdout } = await testReportText(file, repo);

    expect(exit).toBe(0);
    expect(report).toMatchObject({ valid: true, cases: [], repository: { matches: [{ path: "src/a.ts", line: 1 }] } });
    expect(stdout).toContain("● No cases to run; only the repo was checked.");
    const { exit: added, stderr } = await run("add", [file, "--repo", repo]);
    expect(added).toBe(1);
    expect(stderr).toContain("The rule was not added");
  });

  it("reports a case and repo files a clause could not check, rather than passing them", async () => {
    // The negated `inside` is an absence claim over ancestors the parser may not have read, so the clause does not
    // run on a file with syntax errors.
    const policy = {
      ...noConsolePolicy(),
      clauses: [{
        id: "top-level-call",
        forbid: { ast: { language: "typescript", rule: { kind: "call_expression", not: { inside: { kind: "function_declaration", stopBy: "end" } } } } },
        violation: { anchor: "witness.span", code: "TOP_LEVEL_CALL", message: "No top-level calls." },
      }],
    };
    const broken = "f();\nconst x = (;\n";
    const repo = await repository({ "src/broken.ts": broken, "src/clean.ts": "f();\n" });
    const file = writeDraft(repo, {
      kind: "code",
      rule: policy,
      cases: { caught: [{ path: "src/a.ts", content: broken }], passed: [{ path: "src/b.ts", content: "const y = (;\n" }] },
    });

    const { exit, report, stdout } = await testReportText(file, repo);

    expect(exit).toBe(1);
    expect(report.cases).toEqual([
      { expected: "caught", caught: false, ok: false, label: "src/a.ts", notChecked: "file has syntax errors" },
      { expected: "passed", caught: false, ok: false, label: "src/b.ts", notChecked: "file has syntax errors" },
    ]);
    expect(report.repository).toMatchObject({
      matches: [{ path: "src/clean.ts" }],
      unchecked: { files: [{ path: "src/broken.ts", message: "file has syntax errors" }], unattributed: [] },
    });
    expect(stdout).toContain("✗ not checked (file has syntax errors), expected caught: src/a.ts");
    expect(stdout).toContain("1 file was not checked, so the rule may miss places there:\n    src/broken.ts  file has syntax errors");
  });

  // The grammar recovers `debugger;`, then reads the prose as one ERROR to the end: 95% of the bytes, so the parser did
  // not read the file and no AST clause runs there, sound or not.
  const prose = `debugger;\n${"Install it: run the script, then open the page.\n".repeat(5)}`;
  const unread = "file did not parse: error regions cover 95% of it, first at line 2, column 1";
  const debuggerPolicy = (forbid: unknown) => ({
    ...noConsolePolicy(),
    clauses: [{ id: "debugger", forbid, violation: { anchor: "witness.span", code: "DEBUGGER", message: "No debugger." } }],
  });

  it.each([
    ["a sound local clause", { ast: { language: "typescript", rule: { kind: "debugger_statement" } } }],
    ["a cross-file select_when clause", {
      select_when: {
        exists: { regex: { pattern: "debugger", source: "file.content" } },
        select: { ast: { language: "typescript", rule: { kind: "debugger_statement" } } },
      },
    }],
  ])("never passes a file the parser did not read, through %s", async (_name, forbid) => {
    const repo = await repository({ "src/notes.ts": prose, "src/clean.ts": "const a = 1;\n" });
    const file = writeDraft(repo, {
      kind: "code",
      rule: debuggerPolicy(forbid),
      cases: { caught: [], passed: [{ path: "src/notes.ts", content: prose }] },
    });

    const { exit, report, stdout } = await testReportText(file, repo);

    expect(exit).toBe(1);
    expect(report.cases).toEqual([{ expected: "passed", caught: false, ok: false, label: "src/notes.ts", notChecked: unread }]);
    expect(report.repository).toMatchObject({
      matches: [],
      unchecked: { files: [{ path: "src/notes.ts", message: unread }], unattributed: [] },
    });
    expect(stdout).toContain(`✗ not checked (${unread}), expected passed: src/notes.ts`);
    expect(stdout).toContain(`1 file was not checked, so the rule may miss places there:\n    src/notes.ts  ${unread}`);
  });

  it("never passes a case on a failure that names no file, and reports it in the repo pass", async () => {
    // The `llm` clause fails without a path: the rule's verdict on any file is unknown.
    const policy = {
      ...noConsolePolicy(),
      clauses: [{
        id: "intent",
        llm: { input: { source: "changed-symbol" }, rubric: "Fail on direct persistence access", verdicts: ["pass", "fail"] },
        violation: { anchor: "witness.span", code: "INTENT", message: "intent" },
      }],
    };
    const repo = await repository({ "src/a.ts": "const a = 1;\n" });
    const file = writeDraft(repo, { kind: "code", rule: policy, cases: { caught: [], passed: [{ path: "src/b.ts", content: "const b = 1;\n" }] } });

    const { exit, report, stdout } = await testReportText(file, repo);

    expect(exit).toBe(1);
    expect(report.cases).toEqual([
      { expected: "passed", caught: false, ok: false, label: "src/b.ts", notChecked: "LLM capability is unavailable" },
    ]);
    expect(report.repository?.unchecked).toEqual({ files: [], unattributed: ["LLM capability is unavailable"] });
    expect(stdout).toContain("Not checked in any one file (LLM capability is unavailable), so the rule may miss places anywhere.");
  });

  it("writes nothing: no library rule, and the repository is left as it was", async () => {
    const repo = await repository({ "src/a.ts": "console.log(1);\n" });
    const before = stableJsonText(rulesOf(repo));
    const file = writeDraft(repo, { kind: "code", rule: noConsolePolicy() });

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

  it.each(["test", "add"] as const)("rules %s --help prints the draft format and an example that passes rules test", async action => {
    const { exit, stdout } = await run(action, ["--help"]);

    expect(exit).toBe(0);
    for (const key of ['"kind"', '"rule"', '"folder"?', '"cases"', '"caught"', '"passed"', '"path"', '"content"', '"source"', '"text"']) {
      expect(stdout).toContain(key);
    }
    const example: unknown = JSON.parse(stdout.slice(stdout.indexOf("\n{", stdout.indexOf("Example")) + 1));
    const repo = await repository();
    const { exit: tested, report } = await testReport(writeDraft(repo, example), repo);
    expect(tested).toBe(0);
    expect(report.cases.map(({ expected, ok }) => ({ expected, ok }))).toEqual([{ expected: "caught", ok: true }, { expected: "passed", ok: true }]);
  });

  it("points at --help when no draft file is given", async () => {
    await expect(run("test", [])).rejects.toThrow("rerail rules test --help shows its format");
  });
});

describe("rerail rules add", () => {
  it("stores the rule in the library and adds it, on, to the whole of a set-up repository", async () => {
    const repo = await repository();
    setUpEmpty(repo);
    const file = writeDraft(repo, {
      kind: "code",
      rule: noConsolePolicy(),
      cases: { caught: [{ path: "src/a.ts", content: "console.log(1);\n" }], passed: [] },
    });

    const { exit, stdout } = await run("add", [file, "--repo", repo]);

    expect(exit).toBe(0);
    expect(stdout).toMatch(/^● Added acme:no-console-log to rerail-rule-draft-\w+\.\n {2}It runs in Log until a person approves it: rerail rules approve acme:no-console-log\n$/);
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
    const file = writeDraft(repo, { kind: "code", rule: noConsolePolicy(), folder: "web" });

    const { exit, stdout } = await run("add", [file, "--repo", repo]);

    expect(exit).toBe(0);
    expect(stdout).toContain("● Added acme:no-console-log to rerail-rule-draft-");
    expect(stdout).toContain("/web.\n");
    expect(rulesOf(repo).repository).toEqual([{ id: "acme:no-console-log", kind: "library_rule", folder: "web", on: true }]);
  });

  it("says a rule added on Block steers in this release, and stores it on Block", async () => {
    const repo = await repository();
    setUpEmpty(repo);
    const file = writeDraft(repo, { kind: "code", rule: { ...noConsolePolicy(), id: "acme:no-console-block", mode: "block" } });

    const { exit, stdout } = await run("add", [file, "--repo", repo]);

    expect(exit).toBe(0);
    expect(stdout).toContain(BLOCK_STEERS_NOTE);
    expect(getLibraryRuleMode("acme:no-console-block")).toBe("block");

    const steering = writeDraft(repo, { kind: "code", rule: { ...noConsolePolicy(), id: "acme:no-console-steer" } });
    expect((await run("add", [steering, "--repo", repo])).stdout).not.toContain(BLOCK_STEERS_NOTE);
  });

  it("sets a repository that is not set up up with only the new rule", async () => {
    const repo = await repository();
    const file = writeDraft(repo, { kind: "code", rule: noConsolePolicy() });

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
    const file = writeDraft(repo, { kind: "code", rule: noConsolePolicy(), repository: "remote:github.com/acme/other" });

    const { exit, stderr } = await run("add", [file, "--repo", repo]);

    expect(exit).toBe(1);
    expect(stderr).toContain("cannot name repository");
    expect(rulesOf(repo)).toMatchObject({ setUp: false, repository: [] });
  });

  it("refuses to replace a rule by Rerail with the same id", async () => {
    const repo = await repository();
    const bundled = [...loadRuleCatalog().documents.values()].find(document => document.kind === "code");
    if (bundled === undefined) throw new Error("expected a code rule by Rerail");
    const file = writeDraft(repo, { kind: "code", rule: { ...noConsolePolicy(), id: bundled.rule.id } });

    const { exit, stdout, stderr } = await run("add", [file, "--repo", repo]);

    expect(exit).toBe(1);
    expect(stdout).toBe("");
    expect(stderr).toContain(`${bundled.rule.id} is a rule by Rerail, so it was not replaced`);
    const after = rulesOf(repo);
    expect(after.library).not.toContain(bundled.rule.id);
    expect(after).toMatchObject({ setUp: false, repository: [] });
  });
});
