import { execFileSync } from "node:child_process";
import { eq } from "drizzle-orm";
import { mkdirSync, writeFileSync } from "node:fs";
import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import {
  getLibraryRule,
  loadRuleCatalog,
  defaultRerailDatabasePath,
  openRerailDatabase,
  recordCheckout,
  listRepositoryRules,
  setRuleSetting,
  setUpRepository,
  listLibraryRules,
  stableJsonText,
  checkoutStatus,
  repositories,
  installedPacksPrefix,
} from "../../src/engine/index.js";
import { runRuleDraftCommand, type RuleDraftReport } from "../../src/cli/rule-draft.js";
import { BLOCK_STEERS_NOTE } from "../../src/engine/index.js";
import { installPack } from "../support/installed-packs.js";
import { linkFolder, symlinkOrSkip } from "../support/platform/links.js";
import { removeFolder } from "../support/platform/remove-folder.js";

// Core ships no packs; these tests add, configure and copy the complexity pack's rules, and show the typescript pack's
// with option values set.
installPack("complexity");
installPack("typescript");

// rules add compiles every installed pack's rules, once in a process while the installed packs stay the same: done
// here, that one compile is no single case's work, and does not count against that case's time limit.
beforeAll(() => { loadRuleCatalog(); });

const directories: string[] = [];

afterEach(async () => {
  await Promise.all(directories.splice(0).map(directory => removeFolder(directory)));
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

async function run(action: "show" | "test" | "add", args: string[]): Promise<{ exit: number; stdout: string; stderr: string }> {
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

/** Library rule `id`'s stored rule document. */
function getLibraryRuleDocument(id: string): unknown {
  const database = openRerailDatabase(defaultRerailDatabasePath());
  try {
    return getLibraryRule(database, id)?.document.rule;
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
      folders: [""],
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

  it("counts only where rules add would have the rule run: the draft's folder, and the folders it is added at already", async () => {
    const repo = await repository({
      "web/src/a.ts": "console.log('web');\n",
      "web2/src/b.ts": "console.log('web2');\n",
      "lib/c.ts": "console.log('lib');\n",
      "d.ts": "console.log('root');\n",
    });
    setUpEmpty(repo);
    const rule = { ...noConsolePolicy(), id: "no-console-in-folders", scope: { include: ["**/*.ts"], exclude: [] } };
    const tested = async (draft: object) => {
      const { report, stdout } = await testReportText(writeDraft(repo, { kind: "code", rule, ...draft }), repo);
      return { report, folders: report.repository?.folders, paths: report.repository?.matches.map(match => match.path), stdout };
    };

    // Not added yet: a draft that names no folder is added to the whole repository, and one that names a folder there.
    expect(await tested({})).toMatchObject({ folders: [""], paths: ["d.ts", "lib/c.ts", "web/src/a.ts", "web2/src/b.ts"] });
    const nested = await tested({ folder: "web/src" });
    expect(nested).toMatchObject({ folders: ["web/src"], paths: ["web/src/a.ts"] });
    expect(nested.report).not.toHaveProperty("refusal");
    expect(nested.report.repository).not.toHaveProperty("notNarrowed");
    expect(nested.stdout).toContain("  Repo: 1 place matches under web/src/, where the rule runs.\n    web/src/a.ts:1:1  console.log('web');");

    // Added at web: it stays there when the draft names no folder, and a folder the draft names is one more.
    expect((await run("add", [writeDraft(repo, { kind: "code", rule, folder: "web" }), "--repo", repo])).exit).toBe(0);
    expect(await tested({})).toMatchObject({ folders: ["web"], paths: ["web/src/a.ts"] });
    const both = await tested({ folder: "lib" });
    expect(both).toMatchObject({ folders: ["lib", "web"], paths: ["lib/c.ts", "web/src/a.ts"] });
    expect(both.stdout).toContain("  Repo: 2 places match under lib/, web/, where the rule runs.\n");
    const whole = await tested({ folder: "" });
    expect(whole).toMatchObject({ folders: [""], paths: ["d.ts", "lib/c.ts", "web/src/a.ts", "web2/src/b.ts"] });
    expect(whole.stdout).toContain("  Repo: 4 places match.\n");
  });

  it("says so where it cannot read where the rule is added, and still counts the repository", async () => {
    // A home of its own: the installed packs cannot be listed there, since their node_modules is a file.
    const home = await mkdtemp(join(tmpdir(), "rerail-rule-draft-home-"));
    directories.push(home);
    const prior = process.env.RERAIL_HOME;
    process.env.RERAIL_HOME = home;
    try {
      const repo = await repository({ "src/a.ts": "console.log(1);\n" });
      setUpEmpty(repo);
      mkdirSync(installedPacksPrefix(home), { recursive: true });
      writeFileSync(join(installedPacksPrefix(home), "node_modules"), "");
      const file = writeDraft(repo, { kind: "code", rule: { ...noConsolePolicy(), id: "no-console-unread" } });

      const { exit, report, stdout } = await testReportText(file, repo);

      expect(exit).toBe(0);
      expect(report.repository).toMatchObject({ folders: [""], matches: [{ path: "src/a.ts" }] });
      expect(report.repository?.notNarrowed).toContain(join(installedPacksPrefix(home), "node_modules"));
      expect(stdout).toContain(`  Rerail could not read where the rule is added (${report.repository?.notNarrowed}), so the count may not be what the added rule would catch.\n`);
    } finally {
      if (prior === undefined) delete process.env.RERAIL_HOME;
      else process.env.RERAIL_HOME = prior;
    }
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
      folders: [""],
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

/** A `.rr` rule with one test that passes and one that fails (decision 0130). */
const RR_WITH_TESTS = [
  '.policy "local:no-print" revision 1',
  ".language python",
  '.scope include "**/*.py"',
  '.violation P(at: node) code "P1" "no print"',
  'P(c) :- kind(c, "call"), field(c, "function", f), text(f, "print").',
  ".test print_is_reported",
  '    .fixture "a.py" ```',
  '        print(1)',
  "        ```",
  '    .expect P1 at "a.py":1',
  ".end",
  ".test wrong_line",
  '    .fixture "a.py" ```',
  '        print(1)',
  "        ```",
  '    .expect P1 at "a.py":2',
  ".end",
  "",
].join("\n");

describe("rerail rules test on a .rr file", () => {
  it("runs the tests the file carries, in the Engine, and fails when one fails", async () => {
    const repo = await repository();
    const file = join(repo, "no-print.rr");
    writeFileSync(file, RR_WITH_TESTS);

    const json = await run("test", [file, "--json"]);
    const text = await run("test", [file]);

    expect(json.exit).toBe(1);
    expect(JSON.parse(json.stdout)).toMatchObject([
      { test: "print_is_reported", line: 6, status: "pass", failures: [] },
      { test: "wrong_line", line: 12, status: "fail", failures: ['line 16: .expect P1 at "a.py":2: reported P1 at "a.py":1'] },
    ]);
    expect(text.stdout).toContain("pass print_is_reported (line 6)");
    expect(text.stdout).toContain("1 passed, 1 failed");
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

  it("keeps a rule added already where it is when the draft names no folder, and widens it when it names \"\"", async () => {
    const repo = await repository();
    setUpEmpty(repo);
    const rule = { ...noConsolePolicy(), id: "acme:no-console-widen" };
    expect((await run("add", [writeDraft(repo, { kind: "code", rule, folder: "web" }), "--repo", repo])).exit).toBe(0);

    expect((await run("add", [writeDraft(repo, { kind: "code", rule }), "--repo", repo])).stdout).toContain("/web.\n");
    expect(rulesOf(repo).repository.map(added => added.folder)).toEqual(["web"]);

    const widened = await run("add", [writeDraft(repo, { kind: "code", rule, folder: "" }), "--repo", repo]);

    expect(widened.stdout).toMatch(/^● Added acme:no-console-widen to rerail-rule-draft-\w+\.\n/);
    expect(rulesOf(repo).repository.map(added => added.folder)).toEqual(["", "web"]);
  });

  it("shows your rule with where saving changes it: this repository, and the others that add it", async () => {
    const first = await repository();
    const second = await repository();
    const rule = { ...noConsolePolicy(), id: "acme:no-console-shared" };
    for (const repo of [first, second]) expect((await run("add", [writeDraft(repo, { kind: "code", rule }), "--repo", repo])).exit).toBe(0);
    const elsewhere = await repository();

    const shown = await run("show", [rule.id, "--repo", first]);

    expect(JSON.parse(shown.stdout)).toEqual({ kind: "code", rule });
    expect(shown.stderr).toMatch(new RegExp(`^● acme:no-console-shared is yours, added to rerail-rule-draft-\\w+; saving changes it where it is\\. It is also added to rerail-rule-draft-\\w+, where saving changes it too\\.\\n$`));
    expect((await run("show", [rule.id, "--repo", elsewhere])).stderr).toContain("acme:no-console-shared is yours and not added to rerail-rule-draft-");
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

  describe("a draft that keeps a By Rerail rule's id", () => {
    // Each test copies its own rule: the library is shared by this file's tests, and a second copy takes `-copy-2`.
    const SHIPPED = "complexity-cyclomatic-python";

    /** `repo` set up with the complexity pack at `folder`. */
    function setUpWithComplexity(repo: string, folder = ""): void {
      const database = openRerailDatabase(defaultRerailDatabasePath());
      try {
        setUpRepository(database, recordCheckout(database, repo).repositoryKey, { packs: [{ packId: "complexity", folder }] });
      } finally {
        database.$client.close();
      }
    }

    async function show(repo: string, id: string): Promise<{ exit: number; draft: { kind: string; rule: Record<string, unknown> }; stderr: string }> {
      const { exit, stdout, stderr } = await run("show", [id, "--repo", repo]);
      return { exit, draft: JSON.parse(stdout === "" ? "null" : stdout) as { kind: string; rule: Record<string, unknown> }, stderr };
    }

    /** The rules `repo` runs whose id starts with `prefix`, by rule id, with where they come from. */
    function rulesNamed(repo: string, prefix: string) {
      const database = openRerailDatabase(defaultRerailDatabasePath());
      try {
        return listRepositoryRules(database, loadRuleCatalog(), recordCheckout(database, repo).repositoryKey).rules
          .filter(rule => rule.document.rule.id.startsWith(prefix))
          .map(rule => ({ id: rule.document.rule.id, kind: rule.source.kind, folder: rule.attachmentFolder, on: rule.on }))
          .sort((a, b) => a.id.localeCompare(b.id));
      } finally {
        database.$client.close();
      }
    }

    it("is stored as your copy with only its scope changed, added where the original is, which keeps running until approval", async () => {
      const repo = await repository();
      setUpWithComplexity(repo, "api");
      const shown = await show(repo, SHIPPED);
      expect(shown.exit).toBe(0);
      expect(shown.stderr).toContain(`${SHIPPED} ships with Rerail. rules add stores your edit of it as your copy`);
      const scope = { include: ["**/*.py"], exclude: ["**/tests/**", "**/scripts/**"] };
      const file = writeDraft(repo, { ...shown.draft, rule: { ...shown.draft.rule, scope } });

      const { exit, stdout } = await run("add", [file, "--repo", repo]);

      expect(exit).toBe(0);
      expect(stdout).toMatch(new RegExp(`^● Saved your copy of ${SHIPPED} as ${SHIPPED}-copy, added to rerail-rule-draft-\\w+/api where ${SHIPPED} is\\.\\n`));
      expect(stdout).toContain(`It runs in Log beside ${SHIPPED} until a person approves it, which turns ${SHIPPED} off there: rerail rules approve ${SHIPPED}-copy\n`);
      expect(getLibraryRuleDocument(`${SHIPPED}-copy`)).toEqual({ ...shown.draft.rule, id: `${SHIPPED}-copy`, scope });
      expect(rulesNamed(repo, SHIPPED)).toEqual([
        { id: SHIPPED, kind: "pack", folder: "api", on: true },
        { id: `${SHIPPED}-copy`, kind: "library_rule", folder: "api", on: true },
      ]);
    });

    /** A Python function over the complexity pack's limit: one place its Python rule matches, at line 1. */
    const BRANCHY = `def branchy(n):\n${Array.from({ length: 7 }, (_, i) => `    if n == ${i}:\n        return ${i}\n`).join("")}    return -1\n`;

    // No copy is made here: the test above shows rules add putting it where this pass looks.
    it("is tested where the original is added, so rules test counts the places the copy would catch, and records nothing", async () => {
      const repo = await repository({ "api/core.py": BRANCHY, "api/legacy/old.py": BRANCHY, "api2/near.py": BRANCHY, "other/outside.py": BRANCHY });
      setUpWithComplexity(repo, "api");
      const { draft } = await show(repo, SHIPPED);
      const shipped = draft.rule.scope as { include: string[]; exclude: string[] };
      const scope = { ...shipped, exclude: [...shipped.exclude, "**/legacy/**"] };
      const file = writeDraft(repo, { ...draft, rule: { ...draft.rule, scope } });
      const before = stableJsonText(rulesOf(repo));

      const { exit, report, stdout } = await testReportText(file, repo);

      expect(exit).toBe(0);
      expect(report.repository).toMatchObject({ folders: ["api"], matches: [{ path: "api/core.py", line: 1 }] });
      expect(stdout).toContain("  Repo: 1 place matches under api/, where the rule runs.\n    api/core.py:1:");
      expect(report).not.toHaveProperty("refusal");
      expect(report.repository).not.toHaveProperty("notNarrowed");
      expect(stableJsonText(rulesOf(repo))).toBe(before);
    });

    it("is tested across the whole repository where the original is added to all of it, and where it is not added", async () => {
      const files = { "api/core.py": BRANCHY, "other/outside.py": BRANCHY };
      const everywhere = await repository(files);
      setUpWithComplexity(everywhere);
      const nowhere = await repository(files);
      setUpEmpty(nowhere);

      for (const repo of [everywhere, nowhere]) {
        const { report, stdout } = await testReportText(writeDraft(repo, (await show(repo, SHIPPED)).draft), repo);

        expect(report.repository).toMatchObject({ folders: [""], matches: [{ path: "api/core.py" }, { path: "other/outside.py" }] });
        expect(stdout).toContain("  Repo: 2 places match.\n");
      }
    });

    it("is tested where the copy would run when its draft names a folder, with the refusal rules add gives", async () => {
      const repo = await repository({ "api/core.py": BRANCHY, "other/outside.py": BRANCHY });
      setUpWithComplexity(repo, "api");
      const file = writeDraft(repo, { ...(await show(repo, SHIPPED)).draft, folder: "other" });

      const { exit, report, stdout } = await testReportText(file, repo);

      expect(exit).toBe(0);
      expect(report).toMatchObject({ ok: true, refusal: `folder: your copy of ${SHIPPED} is added where ${SHIPPED} is, so the draft cannot name a folder` });
      expect(report.repository).toMatchObject({ folders: ["api"], matches: [{ path: "api/core.py", line: 1 }] });
      expect(report.repository?.matches).toHaveLength(1);
      expect(stdout).toContain("  Repo: 1 place matches under api/.\n    api/core.py:1:");
      expect(stdout).toContain(`  rules add would refuse this draft: ${report.refusal}\n`);
      expect((await run("add", [file, "--repo", repo])).stderr).toBe(`● The rule was not added: ${report.refusal}\n`);
    });

    it("keeps the copy where the original is when it is changed again, from rules show of the copy", async () => {
      const repo = await repository();
      setUpWithComplexity(repo, "api");
      const id = "complexity-cyclomatic-javascript";
      expect((await run("add", [writeDraft(repo, (await show(repo, id)).draft), "--repo", repo])).exit).toBe(0);
      const copy = await show(repo, `${id}-copy`);
      expect(copy.stderr).toMatch(new RegExp(`^● ${id}-copy is yours, added to rerail-rule-draft-\\w+; saving changes it where it is\\.\\n$`));
      const scope = { include: ["**/*.js"], exclude: ["**/vendor/**"] };

      const { exit, stdout } = await run("add", [writeDraft(repo, { ...copy.draft, rule: { ...copy.draft.rule, scope } }), "--repo", repo]);

      expect(exit).toBe(0);
      expect(stdout).toMatch(new RegExp(`^● Added ${id}-copy to rerail-rule-draft-\\w+/api\\.\\n`));
      expect(getLibraryRuleDocument(`${id}-copy`)).toMatchObject({ scope });
      expect(rulesNamed(repo, `${id}-copy`)).toEqual([{ id: `${id}-copy`, kind: "library_rule", folder: "api", on: true }]);
    });

    it("is shown as the repository runs it, so a copy keeps the mode a person set there", async () => {
      const repo = await repository();
      setUpWithComplexity(repo);
      const id = "complexity-cyclomatic-blade";
      const database = openRerailDatabase(defaultRerailDatabasePath());
      try {
        const repositoryKey = recordCheckout(database, repo).repositoryKey;
        setRuleSetting(database, { owner: { kind: "repository", repositoryKey }, folder: "", targetKind: "rule", targetId: id, on: null, mode: "log" });
      } finally {
        database.$client.close();
      }

      const shown = await show(repo, id);

      expect(shown.draft.rule).toEqual({ ...loadRuleCatalog().documents.get(id)?.rule, mode: "log" });
    });

    it("is shown with the option values the repository sets, which the copy keeps, and as shipped when they no longer apply", async () => {
      const repo = await repository();
      const id = "typescript-eslint-consistent-type-definitions";
      const configure = (config: Record<string, string | boolean>) => {
        const database = openRerailDatabase(defaultRerailDatabasePath());
        try {
          const repositoryKey = recordCheckout(database, repo).repositoryKey;
          setUpRepository(database, repositoryKey, { packs: [{ packId: "typescript", folder: "" }] });
          setRuleSetting(database, { owner: { kind: "repository", repositoryKey }, folder: "", targetKind: "rule", targetId: id, on: true, mode: null, config });
        } finally {
          database.$client.close();
        }
      };
      const shipped = loadRuleCatalog().documents.get(id)?.rule;

      configure({ retired: true });
      expect((await show(repo, id)).draft.rule).toEqual(shipped);

      configure({ option: "type" });
      const configured = (await show(repo, id)).draft;

      expect(JSON.stringify(configured.rule)).toContain(`"config":{"option":"type"}`);
      expect((await run("add", [writeDraft(repo, configured), "--repo", repo])).exit).toBe(0);
      expect(getLibraryRuleDocument(`${id}-copy`)).toEqual({ ...configured.rule, id: `${id}-copy` });
    });

    it("says a rule that is off here would run on as a copy", async () => {
      const repo = await repository();
      setUpWithComplexity(repo);
      const id = "complexity-cyclomatic-typescript";
      const database = openRerailDatabase(defaultRerailDatabasePath());
      try {
        const repositoryKey = recordCheckout(database, repo).repositoryKey;
        setRuleSetting(database, { owner: { kind: "repository", repositoryKey }, folder: "", targetKind: "rule", targetId: id, on: false, mode: null });
      } finally {
        database.$client.close();
      }

      const { stderr } = await show(repo, id);

      expect(stderr).toMatch(new RegExp(`^● ${id} is off in rerail-rule-draft-\\w+, and your copy would run there, on\\.`));
    });

    it("refuses your copy made for another repository where its original runs, which show points at and test says", async () => {
      const first = await repository();
      setUpWithComplexity(first);
      const id = "complexity-cyclomatic-vue";
      expect((await run("add", [writeDraft(first, (await show(first, id)).draft), "--repo", first])).exit).toBe(0);
      const second = await repository();
      setUpWithComplexity(second);
      const shown = await show(second, `${id}-copy`);
      expect(shown.stderr).toContain(`${id}-copy is your copy of ${id} for another repository, and is not added to rerail-rule-draft-`);
      expect(shown.stderr).toContain(`To change ${id} here: rerail rules show ${id}\n`);

      const file = writeDraft(second, shown.draft);
      const tested = await testReport(file, second);

      const { exit, stderr } = await run("add", [file, "--repo", second]);

      expect(exit).toBe(1);
      expect(stderr).toContain(`${id}-copy is your copy of ${id} for another repository, and ${id} runs in rerail-rule-draft-`);
      expect(tested).toMatchObject({ exit: 0, report: { ok: true, repository: { folders: [""] } } });
      expect(stderr).toBe(`● The rule was not added: ${tested.report.refusal}\n`);
      expect(rulesNamed(second, id)).toEqual([{ id, kind: "pack", folder: "", on: true }]);
    });

    it("is refused where the repository does not run the original, which show and test say first, storing nothing", async () => {
      const repo = await repository();
      setUpEmpty(repo);
      const id = "complexity-cyclomatic-php";
      const shown = await show(repo, id);
      expect(shown.stderr).toContain(`${id} is not added to rerail-rule-draft-`);
      const file = writeDraft(repo, shown.draft);
      const tested = await testReport(file, repo);

      const { exit, stdout, stderr } = await run("add", [file, "--repo", repo]);

      expect(exit).toBe(1);
      expect(stdout).toBe("");
      expect(stderr).toContain(`${id} is not added to rerail-rule-draft-`);
      expect(tested).toMatchObject({ exit: 0, report: { ok: true, repository: { folders: [""] } } });
      expect(stderr).toBe(`● The rule was not added: ${tested.report.refusal}\n`);
      expect(rulesOf(repo).library.filter(rule => rule.startsWith(id))).toEqual([]);
    });

    it("is refused where your copy of it is already added, which show, test and add all name", async () => {
      const repo = await repository();
      setUpWithComplexity(repo);
      const id = "complexity-cyclomatic-rust";
      const file = writeDraft(repo, (await show(repo, id)).draft);
      expect((await run("add", [file, "--repo", repo])).exit).toBe(0);

      const tested = await testReport(file, repo);

      const again = await run("add", [file, "--repo", repo]);

      expect(again.exit).toBe(1);
      expect(again.stderr).toContain(`already has your copy of ${id}, ${id}-copy. Change that one: rerail rules show ${id}-copy`);
      expect(tested).toMatchObject({ exit: 0, report: { ok: true, repository: { folders: [""] } } });
      expect(again.stderr).toBe(`● The rule was not added: ${tested.report.refusal}\n`);
      expect((await show(repo, id)).stderr).toContain(`Change that one: rerail rules show ${id}-copy`);
      expect(rulesOf(repo).library.filter(rule => rule.startsWith(id))).toEqual([`${id}-copy`]);
    });
  });
});
