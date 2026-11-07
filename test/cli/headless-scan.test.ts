import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { chmodSync, mkdirSync, mkdtempSync, readFileSync, realpathSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import Database from "better-sqlite3";
import { afterEach, describe, expect, it } from "vitest";
import { runAuditCommand, SCAN_VIOLATIONS_EXIT } from "../../src/cli/audit-cli.js";
import { runRuleApproveCommand } from "../../src/cli/rule-approval-cli.js";
import { runHeadlessScan, type ScanReport } from "../../src/cli/headless-scan.js";
import { setUpTestRepository } from "../support/repository-rules.js";

const roots: string[] = [];
afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

function git(cwd: string, ...args: string[]): string {
  return execFileSync("git", ["-c", "user.email=test@example.com", "-c", "user.name=Test", ...args], { cwd, encoding: "utf8" }).trim();
}

function needle(id: string, code: string, mode = "steer"): unknown {
  return {
    format: "rerail.policy/v1", id, revision: 1, mode, description: `Keep ${code} out of Python files.`,
    scope: { include: ["**/*.py"], exclude: [] },
    clauses: [{ id: "needle", violation: { code, anchor: "witness.span", message: `No ${code}` }, forbid: { regex: { pattern: code, source: "file.content" } } }],
  };
}

function writeRule(root: string, name: string, document: unknown): void {
  mkdirSync(join(root, ".rerail", "rules"), { recursive: true });
  writeFileSync(join(root, ".rerail", "rules", name), JSON.stringify(document));
}

/** A committed checkout on `main` with one NEEDLE rule and `a.py`, which breaks it on line 2. */
function checkout(): string {
  const root = realpathSync(mkdtempSync(join(tmpdir(), "rerail-headless-scan-")));
  roots.push(root);
  git(root, "init", "--quiet", "--initial-branch=main");
  writeRule(root, "needle.json", needle("needle", "NEEDLE"));
  writeFileSync(join(root, "a.py"), "a = 1\n  x = NEEDLE\n");
  git(root, "add", "a.py", ".rerail/rules/needle.json");
  git(root, "commit", "--quiet", "-m", "seed");
  return root;
}

const scan = (repo: string, options: Partial<Parameters<typeof runHeadlessScan>[0]> = {}): Promise<ScanReport> =>
  runHeadlessScan({ repo, scope: "repository", packs: [], version: "0.0.0-test", databasePath: `${repo}-no-home.sqlite`, ...options });

describe("headless rerail scan", () => {
  it("scans a checkout with no daemon and no set-up, reporting 1-based positions with an exclusive end", async () => {
    const root = checkout();
    const report = await scan(root);
    expect(report).toMatchObject({
      schema: "rerail.scan/1",
      tool: { name: "rerail", version: "0.0.0-test" },
      checkout: { path: root, head: git(root, "rev-parse", "HEAD"), ref: "refs/heads/main" },
      scope: "repository",
      base: null,
      paths: null,
      rulesSource: { kind: "worktree" },
      status: "completed",
      summary: { violations: 1, failures: 0 },
    });
    expect(report.repository.key).toMatch(/^clone:/u);
    // Worktree rules nobody approved run in Log.
    expect(report.rules).toEqual([{ ruleId: "needle", payloadHash: expect.stringMatching(/^sha256:/u), mode: "log", source: { kind: "pack", id: "worktree", packId: "worktree" } }]);
    expect(report.violations).toEqual([{
      ruleId: "needle", payloadHash: report.rules[0]!.payloadHash, mode: "log", code: "NEEDLE", clauseId: "needle", path: "a.py",
      start: { line: 2, column: 7 }, end: { line: 2, column: 13 }, message: "No NEEDLE", fingerprint: expect.stringMatching(/^[0-9a-f]{32}$/u),
    }]);
  });

  it("keeps a finding's fingerprint when its line moves, and gives identical matches their own", async () => {
    const root = checkout();
    const before = (await scan(root)).violations[0]!.fingerprint;
    writeFileSync(join(root, "a.py"), "# moved\n\n\nx    =    NEEDLE\ny = NEEDLE\n");
    const after = (await scan(root)).violations.map(violation => violation.fingerprint);
    expect(after[0]).toBe(before);
    expect(new Set(after).size).toBe(2);
  });

  it("runs the rules at --rules-ref, not the working tree's, in Log until a person approves that content", async () => {
    const root = checkout();
    git(root, "checkout", "--quiet", "-b", "feature");
    // The change under review weakens the rule and adds one of its own; neither may judge it.
    writeRule(root, "needle.json", needle("needle", "NOTHING_MATCHES"));
    writeRule(root, "extra.json", needle("extra", "x"));
    git(root, "add", "a.py", ".rerail/rules/needle.json");
    git(root, "commit", "--quiet", "-m", "weaken");
    const home = realpathSync(mkdtempSync(join(tmpdir(), "rerail-headless-home-")));
    roots.push(home);
    const databasePath = join(home, "rerail.sqlite");
    const quiet = { stdout: { write: () => true } };

    const unapproved = await scan(root, { rulesRef: "main", databasePath });
    expect(unapproved.rulesSource).toEqual({ kind: "ref", ref: "main" });
    expect(unapproved.rules).toEqual([expect.objectContaining({ ruleId: "needle", mode: "log", source: { kind: "ref", id: "needle", packId: null, ref: "main" } })]);
    expect(unapproved.violations).toEqual([expect.objectContaining({ ruleId: "needle", mode: "log", path: "a.py" })]);

    // Approving the pull request's own content approves nothing the base branch runs.
    runRuleApproveCommand(["needle", "--repo", root], () => "usage", quiet, databasePath);
    expect((await scan(root, { rulesRef: "main", databasePath })).violations[0]!.mode).toBe("log");

    // Approving the base branch's content, from a checkout of it, does.
    const base = join(home, "base");
    git(root, "worktree", "add", "--quiet", "--detach", base, "main");
    runRuleApproveCommand(["needle", "--repo", base], () => "usage", quiet, databasePath);
    const approved = await scan(root, { rulesRef: "main", databasePath });
    expect(approved.rules).toEqual([expect.objectContaining({ ruleId: "needle", mode: "steer" })]);
    expect(approved.violations).toEqual([expect.objectContaining({ ruleId: "needle", mode: "steer", path: "a.py" })]);
  });

  it("runs the working tree's own rules at their mode once a person approved them in the Rerail home", async () => {
    const root = checkout();
    const home = realpathSync(mkdtempSync(join(tmpdir(), "rerail-headless-home-")));
    roots.push(home);
    const databasePath = join(home, "rerail.sqlite");
    runRuleApproveCommand(["needle", "--repo", root], () => "usage", { stdout: { write: () => true } }, databasePath);
    expect((await scan(root, { databasePath })).violations).toEqual([expect.objectContaining({ mode: "steer" })]);
  });

  it("never writes the home's database: it scans with a read-only copy of its configuration, and refuses one it would have to upgrade", async () => {
    const root = checkout();
    const home = realpathSync(mkdtempSync(join(tmpdir(), "rerail-headless-home-")));
    roots.push(home);
    const databasePath = join(home, "rerail.sqlite");
    setUpTestRepository(root, {}, databasePath);
    const digest = (): string => createHash("sha256").update(readFileSync(databasePath)).digest("hex");
    const before = digest();
    chmodSync(databasePath, 0o444);
    const report = await scan(root, { databasePath });
    expect(report.rulesSource).toEqual({ kind: "home" });
    expect(report.violations).toHaveLength(1);
    expect(digest()).toBe(before);

    // A file this build would have to upgrade first is refused, not upgraded.
    const other = realpathSync(mkdtempSync(join(tmpdir(), "rerail-headless-home-")));
    roots.push(other);
    const olderPath = join(other, "rerail.sqlite");
    setUpTestRepository(root, {}, olderPath);
    const database = new Database(olderPath);
    const step = database.prepare("SELECT id FROM schema_migrations ORDER BY rowid DESC LIMIT 1").get() as { id: string };
    database.prepare("DELETE FROM schema_migrations WHERE id = ?").run(step.id);
    database.close();
    const olderDigest = (): string => createHash("sha256").update(readFileSync(olderPath)).digest("hex");
    const pending = olderDigest();
    await expect(scan(root, { databasePath: olderPath })).rejects.toThrow(/before (?:this Rerail can read it|it can be read)/u);
    expect(olderDigest()).toBe(pending);
  });

  it("loads the rules at --rules-ref as a checkout's own: a broken file or a repeated id is left out, with why, and the rest run", async () => {
    const root = checkout();
    writeRule(root, "a-broken.json", "{ not json");
    writeRule(root, "z-again.json", needle("needle", "OTHER"));
    git(root, "add", ".rerail/rules/a-broken.json", ".rerail/rules/z-again.json");
    git(root, "commit", "--quiet", "-m", "more rules");
    const report = await scan(root, { rulesRef: "HEAD" });
    expect(report.rules.map(rule => rule.ruleId)).toEqual(["needle"]);
    expect(report.violations).toEqual([expect.objectContaining({ ruleId: "needle", code: "NEEDLE" })]);
    expect(report.failures).toEqual(expect.arrayContaining([
      expect.objectContaining({ code: "rule-left-out", path: "HEAD:.rerail/rules/a-broken.json" }),
      expect.objectContaining({ code: "rule-left-out", path: "HEAD:.rerail/rules/z-again.json", message: expect.stringContaining("rule needle is already in needle.json") }),
    ]));
    expect(report.status).toBe("completed");
  });

  it("refuses a --pack that is not installed rather than scanning without it", async () => {
    const root = checkout();
    await expect(scan(root, { packs: ["@neralabs/rerail-pack-nope"] })).rejects.toThrow("--pack @neralabs/rerail-pack-nope is not installed");
  });

  it("refuses a --rules-ref that is not a commit rather than scanning with no rules", async () => {
    const root = checkout();
    await expect(scan(root, { rulesRef: "mian" })).rejects.toThrow("--rules-ref mian is not a commit");
  });

  it("scans only what changed since the merge base with --base, and only the paths asked for", async () => {
    const root = checkout();
    git(root, "checkout", "--quiet", "-b", "feature");
    writeFileSync(join(root, "b.py"), "NEEDLE\n");
    git(root, "add", "b.py");
    git(root, "commit", "--quiet", "-m", "b");
    writeFileSync(join(root, "c.py"), "NEEDLE\n");
    const diff = await scan(root, { scope: "git-diff", base: "main" });
    expect(diff.base).toEqual({ ref: "main", mergeBase: git(root, "rev-parse", "main") });
    expect(diff.paths).toEqual(["b.py", "c.py"]);
    expect(diff.violations.map(violation => violation.path).sort()).toEqual(["b.py", "c.py"]);

    const listed = await scan(root, { paths: ["./b.py", "gone.py"] });
    expect(listed.paths).toEqual(["b.py"]);
    expect(listed.violations.map(violation => violation.path)).toEqual(["b.py"]);
  });

  it("uses the Rerail home's configuration when Rerail is set up and started in the checkout", async () => {
    const root = checkout();
    const home = realpathSync(mkdtempSync(join(tmpdir(), "rerail-headless-home-")));
    roots.push(home);
    const databasePath = join(home, "rerail.sqlite");
    setUpTestRepository(root, {}, databasePath);
    const report = await scan(root, { databasePath });
    expect(report.rulesSource).toEqual({ kind: "home" });
    expect(report.violations).toHaveLength(1);
  });

  it("prints JSON and exits 3 on violations only with --fail-on-violations", async () => {
    const root = checkout();
    let output = "";
    const stdout = { write: (text: string) => { output += text; return true; } };
    expect(await runAuditCommand("scan", ["--repo", root, "--json"], { home: root, version: "1.2.3", stdout })).toBe(0);
    expect(JSON.parse(output)).toMatchObject({ schema: "rerail.scan/1", tool: { version: "1.2.3" } });
    expect(await runAuditCommand("scan", ["--repo", root, "--fail-on-violations"], { home: root, stdout })).toBe(SCAN_VIOLATIONS_EXIT);
    await expect(runAuditCommand("scan", ["--repo", root, "--base", "main"], { home: root, stdout })).rejects.toThrow("--base applies only to --scope git-diff");
  });
});
