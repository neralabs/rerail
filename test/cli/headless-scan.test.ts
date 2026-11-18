import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { chmodSync, mkdirSync, mkdtempSync, readFileSync, realpathSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import Database from "better-sqlite3";
import { afterEach, describe, expect, it, vi } from "vitest";
import { runAuditCommand, SCAN_VIOLATIONS_EXIT } from "../../src/cli/audit-cli.js";
import { runRuleApproveCommand } from "../../src/cli/rule-approval-cli.js";
import { runHeadlessScan, type ScanReport } from "../../src/cli/headless-scan.js";
import { acquireDirectoryAnalysisGeneration, analysisScopeHome, queueAnalysisScopeReleases, readAnalysisScopeGeneration, scanContentStore, settleAnalysisScopeReleases } from "../../src/engine/native/analysis-scope.js";
import { loadNativeBinding } from "../../src/engine/native/native.js";
import { openRerailDatabase } from "../../src/engine/storage/database.js";
import { recordCheckout } from "../../src/engine/catalog/repository-registry.js";
import { setCheckoutStarted, stopRerail } from "../../src/runtime/repository-start.js";
import { honoursNoLazyFetch, partialCloneLacks } from "../support/partial-clone.js";
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
  roots.push(root, `${root}-home`);
  git(root, "init", "--quiet", "--initial-branch=main");
  writeRule(root, "needle.json", needle("needle", "NEEDLE"));
  writeFileSync(join(root, "a.py"), "a = 1\n  x = NEEDLE\n");
  git(root, "add", "a.py", ".rerail/rules/needle.json");
  git(root, "commit", "--quiet", "-m", "seed");
  return root;
}

const scan = (repo: string, options: Partial<Parameters<typeof runHeadlessScan>[0]> = {}): Promise<ScanReport> =>
  runHeadlessScan({ repo, scope: "repository", packs: [], version: "0.0.0-test", databasePath: join(`${repo}-home`, "rerail.sqlite"), ...options });

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

  it("scans a historical commit and tree with today's policy and historical paths", async () => {
    const root = checkout();
    const commit = git(root, "rev-parse", "HEAD");
    const tree = git(root, "rev-parse", "HEAD^{tree}");
    rmSync(join(root, "a.py"));
    writeRule(root, "needle.json", needle("current-policy", "NEEDLE"));
    writeFileSync(join(root, "new.py"), "x = NEEDLE\n");
    git(root, "add", "--", "a.py", "new.py", ".rerail/rules/needle.json");
    git(root, "commit", "--quiet", "-m", "today");
    const home = mkdtempSync(join(tmpdir(), "rerail-history-home-"));
    roots.push(home);
    const databasePath = join(home, "rerail.sqlite");

    const analysisScope = "history";
    await expect(scan(root, { revision: "HEAD~1", databasePath })).rejects.toThrow("--analysis-scope");
    const before = await scan(root, { revision: "HEAD~1", analysisScope, databasePath });
    expect(before.status, JSON.stringify(before.failures)).toBe("completed");
    expect(before).toMatchObject({
      source: { revision: "HEAD~1", commit, tree },
      checkout: { head: git(root, "rev-parse", "HEAD") },
      rulesSource: { kind: "worktree" },
      status: "completed",
      summary: { violations: 1, failures: 0 },
    });
    expect(before.rules.map(rule => rule.ruleId)).toEqual(["current-policy"]);
    expect(before.violations[0]).toMatchObject({ ruleId: "current-policy", path: "a.py", start: { line: 2, column: 7 } });
    const selected = await scan(root, { revision: tree, analysisScope, paths: ["a.py", "new.py"], databasePath });
    expect(selected.source).toEqual({ revision: tree, commit: null, tree });
    expect(selected.paths).toEqual(["a.py"]);
    expect(selected.violations).toEqual(before.violations);
    expect((await scan(root, { revision: commit, analysisScope, databasePath })).violations).toEqual(before.violations);
    vi.stubEnv("RERAIL_HOME", home);
    try {
      let output = "";
      const stdout = { write: (text: string) => { output += text; return true; } };
      expect(await runAuditCommand("scan", ["--repo", root, "--revision", "HEAD~1", "--analysis-scope", analysisScope, "--json", "--no-send"], { home, stdout })).toBe(0);
      expect(JSON.parse(output)).toMatchObject({ source: before.source, violations: before.violations });
    } finally {
      vi.unstubAllEnvs();
    }
    expect(git(root, "status", "--porcelain")).toBe("");
  });

  it("scans a bare repository with explicit policy from a ref", async () => {
    const root = checkout();
    const home = mkdtempSync(join(tmpdir(), "rerail-bare-home-"));
    roots.push(home);
    const bare = join(home, "repo.git");
    git(root, "clone", "--quiet", "--bare", ".", bare);
    const report = await scan(bare, { revision: "HEAD", analysisScope: "bare", rulesRef: "main", databasePath: join(home, "rerail.sqlite") });
    expect(report.status, JSON.stringify(report.failures)).toBe("completed");
    expect(report.source?.commit).toBe(git(root, "rev-parse", "HEAD"));
    expect(report.violations).toEqual([expect.objectContaining({ path: "a.py", ruleId: "needle" })]);
  });

  it("places a shared subtree at both paths and applies the revision's ignore rules", async () => {
    const root = checkout();
    const source = readFileSync(join(root, "a.py"));
    for (const folder of ["left", "right", "generated", "node_modules"]) {
      mkdirSync(join(root, folder), { recursive: true });
      writeFileSync(join(root, folder, "shared.py"), source);
    }
    writeFileSync(join(root, ".rerailignore"), "generated/\n");
    git(root, "add", "-f", "--", ".rerailignore", "left/shared.py", "right/shared.py", "generated/shared.py", "node_modules/shared.py");
    git(root, "commit", "--quiet", "-m", "shared trees");
    writeFileSync(join(root, ".rerailignore"), "left/\n");
    const report = await scan(root, { revision: "HEAD", analysisScope: "tree-walk" });
    expect(report.status, JSON.stringify(report.failures)).toBe("completed");
    expect(report.violations.map(finding => finding.path)).toEqual(["a.py", "left/shared.py", "right/shared.py"]);
  });

  it("reads Git metadata through --rr without inventorying an unavailable subtree", async () => {
    const root = checkout();
    const missing = "1".repeat(40);
    const tree = execFileSync("git", ["mktree", "--missing"], {
      cwd: root, input: `040000 tree ${missing}\tunavailable\n`, encoding: "utf8",
    }).trim();
    const commit = git(root, "commit-tree", tree, "-m", "selected Git message");
    const policy = join(root, "message.rr");
    writeFileSync(policy, `
#include "git.rr"
.policy "commit-message" revision 1
git_object_request(r,c) :- git_source_commit(r,c).
.violation Message(at: span, subject: symbol) "{subject}"
Message(@place("commit", 1), subject) :- git_source_commit(r,c), git_subject(r,c,subject).
`);
    let output = "";
    const result = await runAuditCommand("scan", ["--repo", root, "--revision", commit, "--analysis-scope", "message", "--rr", policy, "--json", "--no-send"], {
      home: `${root}-home`, stdout: { write: (chunk: string) => { output += chunk; return true; } },
    });
    const report = JSON.parse(output) as ScanReport;
    expect(result, JSON.stringify(report.failures)).toBe(0);
    expect(report.source?.commit).toBe(commit);
    expect(report.violations).toEqual([expect.objectContaining({ ruleId: "commit-message", message: "selected Git message" })]);
    const missingRoot = execFileSync("git", ["hash-object", "-t", "commit", "-w", "--stdin"], {
      cwd: root, encoding: "utf8", input: `tree ${missing}\nauthor Test <test@example.com> 100 +0000\ncommitter Test <test@example.com> 100 +0000\n\nmessage with no tree\n`,
    }).trim();
    const metadata = await scan(root, { revision: missingRoot, analysisScope: "message", rr: [policy] });
    expect(metadata.status, JSON.stringify(metadata.failures)).toBe("completed");
    expect(metadata.violations.map(finding => finding.message)).toEqual(["message with no tree"]);
    rmSync(join(root, ".rerail/rules/needle.json"));
    writeFileSync(join(root, ".rerail/rules/message.rr"), readFileSync(policy));
    const configured = await scan(root, { revision: missingRoot, analysisScope: "message" });
    expect(configured.status, JSON.stringify(configured.failures)).toBe("completed");
    expect(configured.violations.map(finding => finding.message)).toEqual(["message with no tree"]);
  });

  it("skips unavailable Git subtrees excluded by ignore rules or paths", async () => {
    const root = checkout();
    const blob = git(root, "rev-parse", "HEAD:a.py");
    const makeTree = (entries: string) => execFileSync("git", ["mktree", "--missing"], {
      cwd: root, input: `100644 blob ${blob}\ta.py\n040000 tree ${"1".repeat(40)}\tunavailable\n${entries}`, encoding: "utf8",
    }).trim();
    writeFileSync(join(root, ".rerailignore"), "unavailable/\n!/docs/keep.py\n");
    const ignore = git(root, "hash-object", "-w", ".rerailignore");
    const ignored = await scan(root, { revision: makeTree(`100644 blob ${ignore}\t.rerailignore\n`), analysisScope: "ignore" });
    expect(ignored.status, JSON.stringify(ignored.failures)).toBe("completed");
    expect(ignored.violations.map(finding => finding.path)).toEqual(["a.py"]);
    const selected = await scan(root, { revision: makeTree(""), analysisScope: "path", paths: ["a.py"] });
    expect(selected.status, JSON.stringify(selected.failures)).toBe("completed");
    expect(selected.violations.map(finding => finding.path)).toEqual(["a.py"]);

    const leaf = execFileSync("git", ["mktree"], { cwd: root, input: `100644 blob ${blob}\ta.py\n`, encoding: "utf8" }).trim();
    const data = execFileSync("git", ["mktree", "--missing"], {
      cwd: root, input: `040000 tree ${"1".repeat(40)}\tunavailable\n040000 tree ${leaf}\ta\n`, encoding: "utf8",
    }).trim();
    writeFileSync(join(root, ".rerailignore"), "data/\n!/data/[ab]/a.py\n");
    const wildcardIgnore = git(root, "hash-object", "-w", ".rerailignore");
    const wildcard = await scan(root, { revision: makeTree(`100644 blob ${wildcardIgnore}\t.rerailignore\n040000 tree ${data}\tdata\n`), analysisScope: "ignore", paths: ["a.py", "data"] });
    expect(wildcard.status, JSON.stringify(wildcard.failures)).toBe("completed");
    expect(wildcard.violations.map(finding => finding.path)).toEqual(["a.py", "data/a/a.py"]);

    const missingA = execFileSync("git", ["mktree", "--missing"], {
      cwd: root, input: `040000 tree ${"1".repeat(40)}\ta\n`, encoding: "utf8",
    }).trim();
    for (const [include, exclude] of [
      ["/data/[ab]/a.py", "/data/[ab]/a.py"],
      ["/data/a/a.py", "/data/a/*.py"],
      ["/data/a/[ab].py", "/data/**/[[:alpha:]].py"],
    ]) {
      writeFileSync(join(root, ".rerailignore"), `/data/a/\n!${include}\n${exclude}\n`);
      const cancelledIgnore = git(root, "hash-object", "-w", ".rerailignore");
      const cancelled = await scan(root, {
        revision: makeTree(`100644 blob ${cancelledIgnore}\t.rerailignore\n040000 tree ${missingA}\tdata\n`),
        analysisScope: "ignore",
        paths: ["a.py", "data"],
      });
      expect(cancelled.status, JSON.stringify(cancelled.failures)).toBe("completed");
      expect(cancelled.violations.map(finding => finding.path)).toEqual(["a.py"]);
    }

    const twoFiles = execFileSync("git", ["mktree"], {
      cwd: root, input: `100644 blob ${blob}\ta.py\n100644 blob ${blob}\tb.py\n`, encoding: "utf8",
    }).trim();
    const partialData = execFileSync("git", ["mktree"], {
      cwd: root, input: `040000 tree ${twoFiles}\ta\n`, encoding: "utf8",
    }).trim();
    writeFileSync(join(root, ".rerailignore"), "/data/a/\n!/data/a/[ab].py\n/data/a/a.py\n");
    const partialIgnore = git(root, "hash-object", "-w", ".rerailignore");
    const partial = await scan(root, {
      revision: makeTree(`100644 blob ${partialIgnore}\t.rerailignore\n040000 tree ${partialData}\tdata\n`),
      analysisScope: "ignore",
      paths: ["a.py", "data"],
    });
    expect(partial.status, JSON.stringify(partial.failures)).toBe("completed");
    expect(partial.violations.map(finding => finding.path)).toEqual(["a.py", "data/a/b.py"]);
  });

  it("reports completeness for the requested tree and exposes failures for an incomplete walk", async () => {
    const root = checkout();
    const missing = "1".repeat(40);
    const tree = execFileSync("git", ["mktree", "--missing"], {
      cwd: root, input: `040000 tree ${missing}\tunavailable\n`, encoding: "utf8",
    }).trim();
    const policy = join(root, "walk.rr");
    writeFileSync(policy, `
#include "git.rr"
.policy "tree-walk" revision 1
git_tree_walk_request(r,t) :- git_source_tree(r,t).
.violation Complete(at: span) "complete tree"
Complete(@place("tree", 1)) :- git_tree_walk_complete(_, _).
.violation Unreadable(at: span, object: symbol) "unreadable tree {object}"
Unreadable(@place("tree", 1), o) :- git_tree_walk_error(_, _, o, _).
`);
    const broken = await scan(root, { revision: tree, analysisScope: "explicit-walk", rr: [policy] });
    expect(broken.status, JSON.stringify(broken.failures)).toBe("completed");
    expect(broken.violations.map(finding => finding.message)).toEqual([`unreadable tree ${missing}`]);
    const complete = await scan(root, { revision: "HEAD", analysisScope: "explicit-walk", rr: [policy] });
    expect(complete.status, JSON.stringify(complete.failures)).toBe("completed");
    expect(complete.violations.map(finding => finding.message)).toEqual(["complete tree"]);
  });

  it("reports unreadable requested Git paths while scanning the paths it can read", async () => {
    const root = checkout();
    const blob = git(root, "rev-parse", "HEAD:a.py");
    const tree = execFileSync("git", ["mktree", "--missing"], {
      cwd: root, input: `100644 blob ${blob}\ta.py\n040000 tree ${"1".repeat(40)}\tgaps\n`, encoding: "utf8",
    }).trim();
    const report = await scan(root, {
      revision: tree, analysisScope: "paths", paths: ["a.py", "absent.py", "gaps/a.py"],
    });
    expect(report.status).toBe("failed");
    expect(report.violations.map(finding => finding.path)).toEqual(["a.py"]);
    expect(report.failures).toEqual([expect.objectContaining({ code: "adapter-error", path: "gaps/a.py" })]);
  });

  it("keeps a Git snapshot owned by two scopes after one is deleted", async () => {
    const root = checkout();
    const databasePath = join(`${root}-home`, "rerail.sqlite");
    const storePath = join(`${root}-home`, "scope-test.sqlite");
    vi.stubEnv("RERAIL_CONTENT_STORE_PATH", storePath);
    try {
      for (const analysisScope of ["review", "release"]) {
        const report = await scan(root, { revision: "HEAD", analysisScope, databasePath });
        expect(report.status, JSON.stringify(report.failures)).toBe("completed");
      }
      const subdirectory = join(root, "subdirectory");
      mkdirSync(subdirectory);
      await scan(subdirectory, { databasePath });
      const store = new Database(scanContentStore(databasePath), { readonly: true });
      try {
        const owners = () => store.prepare("SELECT scope, snapshot_id FROM analysis_scope_snapshot UNION SELECT scope, snapshot_id FROM analysis_scope_workspace_snapshot ORDER BY scope")
          .all() as { scope: string; snapshot_id: number }[];
        expect(owners().map(owner => owner.scope)).toEqual([`cwd:${analysisScopeHome(databasePath)}:${subdirectory}`, "named:release", "named:review"]);
        stopRerail(root, { lane: "terminal", say: () => undefined }, { databasePath });
        const both = owners();
        expect(both.map(owner => owner.scope)).toEqual(["named:release", "named:review"]);
        expect(both[0]?.snapshot_id).toBe(both[1]?.snapshot_id);
        loadNativeBinding().deleteAnalysisScope(storePath, "review", false);
        expect(owners()).toEqual([both[0]]);
        loadNativeBinding().deleteAnalysisScope(storePath, "release", false);
        expect(owners()).toEqual([]);
      } finally {
        store.close();
      }
    } finally {
      vi.unstubAllEnvs();
    }
  });

  it("releases a stopped checkout's scope without an active watch", async () => {
    const root = checkout();
    const databasePath = join(`${root}-home`, "rerail.sqlite");
    const storePath = join(`${root}-home`, "scope-stop.sqlite");
    vi.stubEnv("RERAIL_CONTENT_STORE_PATH", storePath);
    try {
      const report = await scan(root, { databasePath });
      expect(report.status, JSON.stringify(report.failures)).toBe("completed");
      const database = openRerailDatabase(databasePath);
      try {
        setCheckoutStarted(database, recordCheckout(database, root), false);
        const pending = () => database.$client.prepare("SELECT count(*) FROM analysis_scope_releases").pluck().get();
        expect(pending()).toBe(1);
        settleAnalysisScopeReleases(databasePath);
        expect(pending()).toBe(0);
        const store = new Database(storePath, { readonly: true });
        try {
          expect(store.prepare("SELECT count(*) FROM analysis_scope_workspace_snapshot").pluck().get()).toBe(0);
        } finally {
          store.close();
        }
      } finally {
        database.$client.close();
      }
    } finally {
      vi.unstubAllEnvs();
    }
  });

  it("keeps a newer directory run when the first delivery of an older stop arrives", async () => {
    const root = checkout();
    const databasePath = join(`${root}-home`, "rerail.sqlite");
    const storePath = join(`${root}-home`, "scope-restart.sqlite");
    vi.stubEnv("RERAIL_CONTENT_STORE_PATH", storePath);
    try {
      const database = openRerailDatabase(databasePath);
      try {
        const checkout = recordCheckout(database, root);
        const beforeStop = readAnalysisScopeGeneration(databasePath);
        await scan(root, { databasePath });
        setCheckoutStarted(database, checkout, false);
        setCheckoutStarted(database, checkout, true);
        expect(readAnalysisScopeGeneration(databasePath)).toBe(beforeStop + 1);
        writeFileSync(join(root, "a.py"), "new_value = 2\n");
        git(root, "add", "a.py");
        git(root, "commit", "-qm", "after stop");
        const report = await scan(root, { databasePath });
        expect(report.status, JSON.stringify(report.failures)).toBe("completed");
        settleAnalysisScopeReleases(databasePath);
        const store = new Database(storePath, { readonly: true });
        try {
          expect(store.prepare("SELECT count(*) FROM analysis_scope_workspace_snapshot").pluck().get()).toBe(1);
        } finally { store.close(); }
      } finally { database.$client.close(); }
    } finally { vi.unstubAllEnvs(); }
  });

  it("drops all staged ownership when one audit cannot retain its snapshot", async () => {
    const root = checkout();
    const databasePath = join(`${root}-home`, "rerail.sqlite");
    writeFileSync(join(root, ".rerail/rules/relational.rr"), `#include "git.rr"\n.policy "relational" revision 1\n.violation Seen(at: span) "tree"\nSeen(@place("tree", 1)) :- git_source_tree(_, _).\n`);
    const binding = loadNativeBinding();
    const fail = vi.spyOn(binding.CodeRuleDslWorkspace.prototype, "retainAnalysisScopeAsync").mockRejectedValue(new Error("test ownership write failed"));
    try {
      const report = await scan(root, { revision: "HEAD", analysisScope: "atomic", databasePath });
      expect(report.status, JSON.stringify(report.failures)).toBe("completed");
      expect(report.violations.map(finding => finding.ruleId).sort()).toEqual(["needle", "relational"]);
      expect(report.cacheWarnings).toEqual(["test ownership write failed"]);
      const store = new Database(scanContentStore(databasePath), { readonly: true });
      try {
        expect(store.prepare("SELECT scope FROM analysis_scope_manifest WHERE scope = 'named:atomic' OR scope GLOB 'pending:*'").all()).toEqual([]);
        expect(store.prepare("SELECT scope FROM analysis_scope_snapshot WHERE scope = 'named:atomic'").all()).toEqual([]);
      } finally { store.close(); }
    } finally { fail.mockRestore(); }
  });

  it("reports findings from the selected tree when its durable cache cannot open", async () => {
    const root = checkout();
    const storePath = join(root, "unreadable.sqlite");
    const invalid = "this is not a database";
    writeFileSync(storePath, invalid);
    writeFileSync(join(root, "a.py"), "x = f(NEEDLE)\n");
    git(root, "add", "a.py");
    git(root, "commit", "-qm", "committed call");
    writeFileSync(join(root, "a.py"), "# no call or violation in the checkout\n");
    const policy = join(root, ".rerail/rules/call.rr");
    writeFileSync(policy, `#include "core.rr"\n.policy "find-call" revision 1\n.language python\n.violation Call(at: node) "call"\nCall(n) :- kind(n, "call").\n`);
    vi.stubEnv("RERAIL_CONTENT_STORE_PATH", storePath);
    try {
      const report = await scan(root, { revision: "HEAD", analysisScope: "fallback" });
      expect(report.status, JSON.stringify(report.failures)).toBe("completed");
      expect(report.violations.map(finding => finding.ruleId).sort()).toEqual(["find-call", "needle"]);
      expect(report.cacheWarnings).toEqual([expect.stringMatching(/file is not a database/u)]);
      expect(readFileSync(storePath, "utf8")).toBe(invalid);
    } finally { vi.unstubAllEnvs(); }
  });

  it("retries a cache failure during Git attachment against the pinned tree", async () => {
    const root = checkout();
    const broken = join(`${root}-home`, "broken.sqlite");
    mkdirSync(`${root}-home`, { recursive: true });
    writeFileSync(broken, "this is not a database");
    writeFileSync(join(root, "a.py"), "# changed after the commit\n");
    const prototype = loadNativeBinding().CodeRuleDslWorkspace.prototype;
    const attach = prototype.enableRevisionStorageAsync;
    const observed: unknown[] = [];
    const failure = vi.spyOn(prototype, "enableRevisionStorageAsync").mockImplementation(function (
      this: typeof prototype, source, store, secrets, analysis,
    ) {
      observed.push(source);
      return attach.call(this, source, store === ":memory:" ? store : broken, secrets, analysis);
    });
    try {
      const report = await scan(root, { revision: "HEAD", analysisScope: "attach" });
      expect(report.status, JSON.stringify(report.failures)).toBe("completed");
      expect(report.violations.map(finding => finding.path)).toEqual(["a.py"]);
      expect(report.cacheWarnings).toEqual([expect.stringMatching(/file is not a database/u)]);
      expect(observed.length).toBeGreaterThanOrEqual(2);
      expect(observed.every(source => source === observed[0])).toBe(true);
    } finally { failure.mockRestore(); }
  });

  it("retries relational evaluation in memory when its selected cache fails", async () => {
    const root = checkout();
    const home = `${root}-home`;
    const broken = join(home, "broken.sqlite");
    const policy = join(home, "committed.rr");
    mkdirSync(home, { recursive: true });
    writeFileSync(broken, "this is not a database");
    writeFileSync(policy, `#include "core.rr"\n.policy "committed" revision 1\n.language python\n.violation Seen(at: node) "needle"\nSeen(n) :- kind(n, "identifier"), text(n, "NEEDLE").\n`);
    writeFileSync(join(root, "a.py"), "# changed after the commit\n");
    const binding = loadNativeBinding();
    const run = binding.scanRrPoliciesAsync;
    const failure = vi.spyOn(binding, "scanRrPoliciesAsync").mockImplementation((paths, root, _store, ...args) =>
      run(paths, root, broken, ...args));
    try {
      const report = await scan(root, { revision: "HEAD", analysisScope: "relational", rr: [policy] });
      expect(report.status, JSON.stringify(report.failures)).toBe("completed");
      expect(report.violations.map(finding => finding.path)).toEqual(["a.py"]);
      expect(report.cacheWarnings).toEqual([expect.stringMatching(/file is not a database/u)]);
    } finally { failure.mockRestore(); }
  });

  it("retries unreadable stored results against the same source and policy", async () => {
    const root = checkout();
    const source = git(root, "rev-parse", "HEAD");
    const prototype = loadNativeBinding().CodeRuleDslWorkspace.prototype;
    const read = vi.spyOn(prototype, "resultPageAsync").mockImplementationOnce(async () => {
      writeFileSync(join(root, "a.py"), "# no finding in the new commit\n");
      writeRule(root, "needle.json", needle("new-rule", "OTHER"));
      git(root, "add", "a.py");
      git(root, "commit", "-qm", "move HEAD");
      throw new Error("WORKSPACE_STORE_UNAVAILABLE\ncache read failed");
    });
    try {
      const report = await scan(root, { revision: "HEAD", analysisScope: "reused-results" });
      expect(report.status, JSON.stringify(report.failures)).toBe("completed");
      expect(report.source?.commit).toBe(source);
      expect(report.violations.map(finding => finding.ruleId)).toEqual(["needle"]);
      expect(report.cacheWarnings).toEqual(["cache read failed"]);
      expect(read.mock.calls.length).toBeGreaterThanOrEqual(2);
    } finally { read.mockRestore(); }
  });

  it("releases directory ownership only for the stopped home in a shared store", async () => {
    const root = checkout();
    const first = join(`${root}-home`, "first.sqlite");
    const second = join(`${root}-home`, "second.sqlite");
    const storePath = join(`${root}-home`, "shared.sqlite");
    vi.stubEnv("RERAIL_CONTENT_STORE_PATH", storePath);
    try {
      expect((await scan(root, { databasePath: first })).status).toBe("completed");
      const db = openRerailDatabase(first);
      try {
        const checkout = recordCheckout(db, root);
        setCheckoutStarted(db, checkout, false);
        setCheckoutStarted(db, checkout, true);
        setCheckoutStarted(db, checkout, false);
      } finally { db.$client.close(); }
      expect((await scan(root, { databasePath: second })).status).toBe("completed");
      settleAnalysisScopeReleases(first);
      const store = new Database(storePath, { readonly: true });
      try {
        expect(store.prepare("SELECT scope FROM analysis_scope_workspace_snapshot").pluck().all())
          .toEqual([`cwd:${analysisScopeHome(second)}:${root}`]);
      } finally { store.close(); }
    } finally { vi.unstubAllEnvs(); }
  });

  it("releases a stopped directory from each store that held it and retries an unavailable store", async () => {
    const root = checkout();
    const other = checkout();
    const home = `${root}-home`;
    const databasePath = join(home, "rerail.sqlite");
    const earlier = join(home, "first.sqlite");
    const current = join(home, "second.sqlite");
    const unrelated = join(home, "unrelated.sqlite");
    const owners = (path: string) => {
      const db = new Database(path, { readonly: true });
      try { return db.prepare("SELECT count(*) FROM analysis_scope_workspace_snapshot").pluck().get(); }
      finally { db.close(); }
    };
    try {
      for (const store of [earlier, current]) {
        vi.stubEnv("RERAIL_CONTENT_STORE_PATH", store);
        expect((await scan(root, { databasePath })).status).toBe("completed");
        expect(owners(store)).toBe(1);
      }
      vi.stubEnv("RERAIL_CONTENT_STORE_PATH", unrelated);
      expect((await scan(other, { databasePath })).status).toBe("completed");
      expect(owners(unrelated)).toBe(1);
      const homeDb = openRerailDatabase(databasePath);
      try {
        const checkout = recordCheckout(homeDb, root);
        setCheckoutStarted(homeDb, checkout, false);
        const binding = loadNativeBinding();
        const release = binding.deleteAnalysisScope;
        const fail = vi.spyOn(binding, "deleteAnalysisScope").mockImplementation((store, ...args) => {
          if (store === earlier) throw new Error("store unavailable");
          if (store === unrelated) throw new Error("unrelated store");
          return release(store, ...args);
        });
        const pending = () => homeDb.$client.prepare("SELECT count(*) FROM analysis_scope_releases").pluck().get();
        try {
          expect(() => settleAnalysisScopeReleases(databasePath)).toThrow("store unavailable");
          expect(pending()).toBe(1);
          expect(owners(earlier)).toBe(1);
          expect(owners(current)).toBe(0);
          expect(fail.mock.calls.every(([store]) => store !== unrelated)).toBe(true);
        } finally { fail.mockRestore(); }
        settleAnalysisScopeReleases(databasePath, -1);
        expect(pending()).toBe(0);
        expect(owners(earlier)).toBe(0);
        expect(owners(unrelated)).toBe(1);
      } finally { homeDb.$client.close(); }
    } finally { vi.unstubAllEnvs(); }
  });

  it("does not register a store that could never open", async () => {
    const root = checkout();
    const home = `${root}-home`;
    mkdirSync(home);
    const file = join(home, "file");
    writeFileSync(file, "");
    const databasePath = join(home, "rerail.sqlite");
    vi.stubEnv("RERAIL_CONTENT_STORE_PATH", join(file, "unusable.sqlite"));
    try {
      const report = await scan(root, { databasePath });
      expect(report.status).toBe("completed");
      expect(report.violations.map(finding => finding.ruleId)).toEqual(["needle"]);
      expect(report.cacheWarnings).toHaveLength(1);
      const database = openRerailDatabase(databasePath);
      try {
        setCheckoutStarted(database, recordCheckout(database, root), false);
        settleAnalysisScopeReleases(databasePath);
        expect(database.$client.prepare("SELECT count(*) FROM analysis_scope_releases").pluck().get()).toBe(0);
      } finally { database.$client.close(); }
    } finally { vi.unstubAllEnvs(); }
  });

  it("services later stops while a full batch of releases waits for an unavailable store", async () => {
    const root = checkout();
    const home = `${root}-home`;
    const databasePath = join(home, "rerail.sqlite");
    const unavailable = join(home, "unavailable.sqlite");
    const available = join(home, "available.sqlite");
    const blocked = Array.from({ length: 32 }, (_, index) => join(home, "blocked", String(index)));
    const database = openRerailDatabase(databasePath);
    try {
      for (const directory of blocked) await acquireDirectoryAnalysisGeneration(databasePath, directory, unavailable);
      database.transaction(tx => queueAnalysisScopeReleases(tx, blocked, databasePath));
      vi.stubEnv("RERAIL_CONTENT_STORE_PATH", available);
      expect((await scan(root, { databasePath })).status).toBe("completed");
      setCheckoutStarted(database, recordCheckout(database, root), false);

      const binding = loadNativeBinding();
      const release = binding.deleteAnalysisScope;
      const unavailableStore = vi.spyOn(binding, "deleteAnalysisScope").mockImplementation((store, ...args) => {
        if (store === unavailable) throw new Error("store unavailable");
        return release(store, ...args);
      });
      const pending = () => database.$client.prepare("SELECT directory FROM analysis_scope_releases ORDER BY directory").pluck().all();
      try {
        expect(() => settleAnalysisScopeReleases(databasePath)).toThrow("store unavailable");
        expect(pending()).toContain(root);
        settleAnalysisScopeReleases(databasePath);
        expect(pending()).toEqual([...blocked].sort());
        const store = new Database(available, { readonly: true });
        try {
          expect(store.prepare("SELECT count(*) FROM analysis_scope_workspace_snapshot").pluck().get()).toBe(0);
        } finally { store.close(); }
      } finally { unavailableStore.mockRestore(); }
      const next = database.$client.prepare("SELECT max(next_attempt_at) FROM analysis_scope_releases").pluck().get() as number;
      const clock = vi.spyOn(Date, "now").mockReturnValue(next);
      try {
        settleAnalysisScopeReleases(databasePath);
        expect(pending()).toEqual([]);
      } finally { clock.mockRestore(); }
    } finally {
      vi.unstubAllEnvs();
      database.$client.close();
    }
  });

  it("rejects duplicate explicit policy IDs and leaves no named ownership", async () => {
    const root = checkout();
    const databasePath = join(`${root}-home`, "rerail.sqlite");
    const policy = (message: string) => `#include "git.rr"\n.policy "same" revision 1\n.violation Seen(at: span) "${message}"\nSeen(@place("tree", 1)) :- git_source_tree(_, _).\n`;
    const files = [join(root, "one.rr"), join(root, "two.rr")];
    files.forEach((file, index) => writeFileSync(file, policy(`message ${index}`)));
    const report = await scan(root, { revision: "HEAD", analysisScope: "duplicates", rr: files, databasePath });
    expect(report.status).toBe("failed");
    expect(report.failures[0]?.message).toContain("duplicate .rr policy ID");
    const store = new Database(scanContentStore(databasePath), { readonly: true });
    try {
      expect(store.prepare("SELECT scope FROM analysis_scope_manifest WHERE scope = 'named:duplicates' OR scope LIKE 'pending:%'").all()).toEqual([]);
    } finally { store.close(); }
  });

  it("discovers and approves worktree .rr policies and reports their source spans", async () => {
    const root = checkout();
    const databasePath = join(`${root}-home`, "rerail.sqlite");
    const policy = join(root, ".rerail", "rules", "call.rr");
    writeFileSync(policy, `#include "core.rr"\n.policy "find-call" revision 1\n.mode block\n.language python\n.violation Call(at: node) "call"\nCall(n) :- kind(n, "call").\n`);
    writeFileSync(join(root, "f.py"), "x = f(1)\n");
    git(root, "add", "f.py");
    git(root, "commit", "-qm", "call fixture");
    const options = { revision: "HEAD", analysisScope: "catalog", databasePath };
    const before = await scan(root, options);
    expect(before.status, JSON.stringify(before.failures)).toBe("completed");
    const finding = before.violations.find(row => row.ruleId === "find-call");
    expect(finding).toMatchObject({ path: "f.py", start: { line: 1, column: 5 }, end: { line: 1, column: 9 }, mode: "log" });
    expect(runRuleApproveCommand(["find-call", "--repo", root], () => "", { stdout: { write: () => true } }, databasePath)).toBe(0);
    writeFileSync(join(root, "f.py"), "# moved\nx = f(1)\n");
    git(root, "add", "f.py");
    git(root, "commit", "-qm", "shift the call");
    const after = await scan(root, options);
    expect(after.status, JSON.stringify(after.failures)).toBe("completed");
    expect(after.violations.find(row => row.ruleId === "find-call")).toMatchObject({ start: { line: 2, column: 5 }, end: { line: 2, column: 9 }, mode: "block", fingerprint: finding!.fingerprint });
  });

  it("keeps --rules-ref policies, includes, approvals, and provenance pinned if the ref moves", async () => {
    const root = checkout();
    const databasePath = join(`${root}-home`, "rerail.sqlite");
    const policy = join(root, ".rerail/rules/ref.rr");
    const include = join(root, ".rerail/rules/lib/ref-data.rr");
    mkdirSync(join(root, ".rerail/rules/lib"));
    writeFileSync(policy, `#include "git.rr"\n#include "lib/ref-data.rr"\n.policy "ref-policy" revision 1\n.mode block\n.violation Seen(at: span, text: symbol) "{text}"\nSeen(@place("commit", 1), s) :- git_source_commit(_, _), ref_message(s).\n`);
    writeFileSync(include, '.decl ref_message(text: symbol)\nref_message("approved include").\n');
    git(root, "add", ".rerail/rules/ref.rr", ".rerail/rules/lib/ref-data.rr");
    git(root, "commit", "-qm", "approved policy");
    const approved = git(root, "rev-parse", "HEAD");
    expect(runRuleApproveCommand(["ref-policy", "--repo", root], () => "", { stdout: { write: () => true } }, databasePath)).toBe(0);
    writeFileSync(include, '.decl ref_message(text: symbol)\nref_message("new include").\n');
    git(root, "add", ".rerail/rules/lib/ref-data.rr");
    git(root, "commit", "-qm", "changed include");
    const changed = git(root, "rev-parse", "HEAD");
    const options = { revision: "HEAD", analysisScope: "ref", databasePath };
    git(root, "branch", "review-rules", approved);
    const prototype = loadNativeBinding().CodeRuleDslWorkspace.prototype;
    const page = prototype.resultPageAsync;
    const move = vi.spyOn(prototype, "resultPageAsync").mockImplementationOnce(function (this: typeof prototype, ...args) {
      git(root, "update-ref", "refs/heads/review-rules", changed);
      return page.call(this, ...args);
    });
    let old: ScanReport;
    try {
      old = await scan(root, { ...options, rulesRef: "review-rules" });
      expect(move).toHaveBeenCalled();
    } finally { move.mockRestore(); }
    expect(old.status, JSON.stringify(old.failures)).toBe("completed");
    const provenance = { kind: "ref", ref: "review-rules", commit: approved };
    expect(old.rulesSource).toEqual(provenance);
    expect(old.rules).toEqual(expect.arrayContaining([
      expect.objectContaining({ ruleId: "needle", source: { ...provenance, id: "needle", packId: null } }),
      expect.objectContaining({ ruleId: "ref-policy", mode: "block", source: { ...provenance, id: ".rerail/rules/ref.rr", packId: null } }),
    ]));
    expect(old.violations).toEqual(expect.arrayContaining([expect.objectContaining({ ruleId: "ref-policy", message: "approved include", mode: "block" })]));
    const current = await scan(root, { ...options, rulesRef: "review-rules" });
    expect(current.status, JSON.stringify(current.failures)).toBe("completed");
    expect(current.rulesSource).toEqual({ ...provenance, commit: changed });
    expect(current.violations).toEqual(expect.arrayContaining([expect.objectContaining({ ruleId: "ref-policy", message: "new include", mode: "log" })]));
  });

  it("fails the CLI when a selected .rr policy cannot load", async () => {
    const root = checkout();
    rmSync(join(root, ".rerail/rules/needle.json"));
    writeFileSync(join(root, ".rerail/rules/broken.rr"), '.policy "broken" revision 1\n');
    git(root, "add", ".rerail/rules/needle.json", ".rerail/rules/broken.rr");
    git(root, "commit", "-qm", "broken policy");
    const home = `${root}-home`;
    vi.stubEnv("RERAIL_HOME", home);
    try {
      for (const rulesRef of [[], ["--rules-ref", "HEAD"]]) {
        let output = "";
        const exit = await runAuditCommand("scan", ["--repo", root, "--revision", "HEAD", "--analysis-scope", "ci", "--json", "--no-send", "--fail-on-violations", ...rulesRef], {
          home, stdout: { write: text => { output += text; return true; } },
        });
        expect(exit).toBe(1);
        expect(JSON.parse(output)).toMatchObject({ status: "failed", rules: [], failures: [expect.objectContaining({ code: "adapter-error", path: expect.stringMatching(/broken\.rr$/u) })] });
      }
    } finally { vi.unstubAllEnvs(); }
  });

  it("reports the registered repository for an explicit .rr policy after the remote changes", async () => {
    const root = checkout();
    const databasePath = join(`${root}-home`, "rerail.sqlite");
    git(root, "remote", "add", "origin", "https://example.com/example/original.git");
    const db = openRerailDatabase(databasePath);
    const registered = recordCheckout(db, root);
    db.$client.close();
    git(root, "remote", "set-url", "origin", "https://example.com/example/renamed.git");
    const policy = join(root, "commit.rr");
    writeFileSync(policy, `#include "git.rr"\n.policy "metadata" revision 1\n.violation Seen(at: span) "tree"\nSeen(@place("commit", 1)) :- git_source_tree(_, _).\n`);
    const before = readFileSync(databasePath);
    const report = await scan(root, { rr: [policy], revision: "HEAD", analysisScope: "identity", databasePath });
    expect(report.status).toBe("completed");
    expect(report.repository.key).toBe(registered.repositoryKey);
    expect(readFileSync(databasePath)).toEqual(before);
  });

  it("retains the evaluated span and fingerprint when the working file changes after evaluation", async () => {
    const root = checkout();
    const policy = join(root, "call.rr");
    const file = join(root, "f.py");
    writeFileSync(policy, `#include "core.rr"\n.policy "find-call" revision 1\n.language python\n.violation Call(at: node) "call"\nCall(n) :- kind(n, "call").\n`);
    writeFileSync(file, "x = f(1)\n");
    const baseline = await scan(root, { rr: [policy] });
    const binding = loadNativeBinding();
    const evaluate = binding.scanRrPoliciesAsync;
    const edit = vi.spyOn(binding, "scanRrPoliciesAsync").mockImplementation(async (...args) => {
      const result = await evaluate(...args);
      writeFileSync(file, "# changed after evaluation\nx = f(1)\n");
      return result;
    });
    try {
      const report = await scan(root, { rr: [policy] });
      expect(report.status, JSON.stringify(report.failures)).toBe("completed");
      expect(report.violations[0]).toMatchObject({ path: "f.py", start: { line: 1, column: 5 }, end: { line: 1, column: 9 } });
      expect(report.violations).toEqual(baseline.violations);
    } finally { edit.mockRestore(); }
  });

  it("classifies the same revision file for configured and explicit .rr policies", async () => {
    const root = checkout();
    writeFileSync(join(root, ".gitignore"), "ignored\n");
    git(root, "add", ".gitignore");
    git(root, "commit", "-qm", "named text file");
    const policy = join(root, ".rerail/rules/language.rr");
    writeFileSync(policy, `#include "core.rr"\n#include "source-language.rr"\n.policy "language-label" revision 1\nsource_language_request(".gitignore").\n.violation Label(at: span, label: symbol) "{label}"\nLabel(@place(".gitignore", 1), g) :- source_language(".gitignore", g).\n`);
    const options = { revision: "HEAD", analysisScope: "registry" };
    const configured = await scan(root, options);
    const explicit = await scan(root, { ...options, rr: [policy] });
    const labels = (report: ScanReport) => report.violations.filter(row => row.ruleId === "language-label").map(row => row.message);
    expect(labels(configured)).toEqual(["text"]);
    expect(labels(explicit)).toEqual(labels(configured));
  });

  it("shares cold tree facts between source discovery and explicit policy requests", () => {
    const root = checkout();
    const home = `${root}-home`;
    mkdirSync(home);
    for (const path of ["src", "ignored"]) {
      mkdirSync(join(root, path));
      writeFileSync(join(root, path, "call.py"), "x = f(1)\n");
    }
    writeFileSync(join(root, ".rerailignore"), "ignored/\n");
    git(root, "add", ".rerailignore", "src/call.py", "ignored/call.py");
    git(root, "commit", "-qm", "source and ignored tree");
    const policy = join(home, "walk.rr");
    writeFileSync(policy, `
#include "git.rr"
.policy "walk-source" revision 1
.language python
.violation Call(at: node) "call"
Call(n) :- kind(n, "call").
.violation Seen(at: span) "file in the tree"
git_tree_walk_request(r, t) :- git_source_tree(r, t).
Seen(@place(p, 1)) :- git_tree_file(_, _, p, _, _), git_tree_walk_complete(_, _).
`);
    const script = `
      import { join } from "node:path";
      import { runHeadlessScan } from "./src/cli/headless-scan.ts";
      const [repo, home, policy] = process.argv.slice(2);
      const report = await runHeadlessScan({
        repo, databasePath: join(home, "rerail.sqlite"), rr: [policy], revision: "HEAD",
        analysisScope: "walk", scope: "repository", packs: [], version: "test",
      });
      console.log(JSON.stringify(report));
    `;
    const profile = join(home, "profile");
    const report = JSON.parse(execFileSync(process.execPath, ["--import", "tsx", "--input-type=module", "-", root, home, policy], {
      input: script, encoding: "utf8", env: { ...process.env, RERAIL_PROFILE: profile },
    })) as ScanReport;
    expect(report.status, JSON.stringify(report.failures)).toBe("completed");
    expect(report.violations.filter(row => row.code === "Call").map(row => row.path)).toEqual(["src/call.py"]);
    expect(report.violations).toContainEqual(expect.objectContaining({ code: "Seen", path: "ignored/call.py" }));
    const uniqueSubtrees = new Set(git(root, "ls-tree", "-d", "-r", "HEAD").split("\n").map(row => row.split(/\s+/u)[2]));
    const trace = readFileSync(join(profile, "profile.txt"), "utf8");
    expect(Number(trace.match(/^\s*count\s+(\d+)\s+git\.tree_decodes$/mu)?.[1])).toBe(1 + uniqueSubtrees.size);
  });

  it("reopens the store after registry and parser version changes", () => {
    const root = checkout();
    const home = `${root}-home`;
    mkdirSync(home);
    const priorPaths = ["a.py", "c.py", "d.py", "e.py", "f.py"];
    for (const [i, path] of priorPaths.entries()) writeFileSync(join(root, path), `a = f(${i})\n`);
    writeFileSync(join(root, "b.sample"), "b = g(2)\n");
    git(root, "add", ...priorPaths, "b.sample");
    git(root, "commit", "-qm", "files for both registry versions");
    const store = join(home, "store.sqlite");
    writeFileSync(join(home, "policy.rr"), `
#include "core.rr"
#include "source-language.rr"
.policy "source-version" revision 1
.language python
.violation Call(at: node) "call"
.violation LanguageKey(at: span, key: symbol) "{key}"
Call(n) :- kind(n, "call").
${[...priorPaths, "b.sample"].map(path => `source_language_request("${path}").`).join("\n")}
LanguageKey(@place(path, 1), k) :- source_language_key(path, k, _).
`);
    const script = `
      import { join } from "node:path";
      import { loadNativeBinding } from "./src/engine/native/native.ts";
      const [root, home, index, registry] = process.argv.slice(2);
      const engine = loadNativeBinding();
      const source = new engine.GitRevisionSource(root, "HEAD", "versions");
      source.setSourceLanguages(JSON.stringify({ names: {}, suffixes: JSON.parse(registry) }));
      const store = join(home, "store.sqlite");
      const run = new engine.AnalysisScopeRun(store, "versions", false, index, 0);
      const [report] = JSON.parse(await engine.scanRrPoliciesAsync([join(home, "policy.rr")], root, store, join(home, "out-" + index), run, undefined, source));
      const paths = report.diagnostics.filter(finding => finding.code === "Call").map(finding => finding.path).sort();
      const keys = Object.fromEntries(report.diagnostics.filter(finding => finding.code === "LanguageKey").map(finding => [finding.path, finding.message]));
      console.log(JSON.stringify({ paths, keys, warning: run.finish(true) }));
    `;
    const measure = (index: number, registry: Record<string, string>, grammar: string) => {
      const profile = join(home, `profile-${index}`);
      const output = execFileSync(process.execPath, ["--import", "tsx", "--input-type=module", "-", root, home, String(index), JSON.stringify(registry)], {
        input: script, encoding: "utf8", env: { ...process.env, RERAIL_PROFILE: profile, RERAIL_TEST_GRAMMAR_BUILD_ID: grammar.repeat(32) },
      });
      const { paths, keys, warning } = JSON.parse(output) as { paths: string[]; keys: Record<string, string>; warning: string | null };
      expect(warning).toBeNull();
      const counters = new Map([...readFileSync(join(profile, "profile.txt"), "utf8")
        .matchAll(/^\s*count\s+(\d+)\s+(source_(?:language|tape)\.[a-z]+)$/gmu)]
        .map(([, count, name]) => [name, Number(count)]));
      const db = new Database(store, { readonly: true });
      try {
        return { paths, keys, classifications: counters.get("source_language.evaluations") ?? 0, parses: counters.get("source_tape.parses") ?? 0,
          tapes: db.prepare("SELECT printf('%015x', grammar_version) AS version, count(*) AS count FROM tape_objects GROUP BY grammar_version ORDER BY grammar_version")
            .all() as { version: string; count: number }[] };
      } finally { db.close(); }
    };
    const original = { ".py": "python" };
    const extended = { ...original, ".sample": "python" };
    const cold = measure(0, original, "a");
    const allPaths = [...priorPaths, "b.sample"].sort();
    expect(cold.paths).toEqual(priorPaths);
    expect(Object.keys(cold.keys).sort()).toEqual(allPaths);
    expect(cold.classifications).toBeGreaterThan(0);
    expect(cold.parses).toBeGreaterThan(0);
    expect(measure(1, original, "a")).toEqual({ ...cold, classifications: 0, parses: 0 });
    const relabeled = measure(2, extended, "a");
    expect(relabeled.paths).toEqual(allPaths);
    for (const path of priorPaths) expect(relabeled.keys[path]).toBe(cold.keys[path]);
    expect(relabeled.keys["b.sample"]).not.toBe(cold.keys["b.sample"]);
    expect(relabeled.classifications).toBeGreaterThan(0);
    expect(relabeled.parses).toBe(1);
    expect(relabeled.tapes[0]?.count).toBe(cold.tapes[0]!.count + 1);
    // Test hooks let a new process change the parser build ID without rebuilding the addon.
    if (cold.tapes[0]?.version === "a".repeat(15)) {
      const reparsed = measure(3, extended, "b");
      expect(reparsed.paths).toEqual(relabeled.paths);
      expect(reparsed.keys).toEqual(relabeled.keys);
      expect(reparsed.classifications).toBe(0);
      expect(reparsed.parses).toBe(allPaths.length);
      expect(reparsed.tapes).toEqual([...relabeled.tapes, { version: "b".repeat(15), count: allPaths.length }]);
    }
  }, 20_000);

  it("gives edited policy programs separate cache keys in the same policy set", async () => {
    const root = checkout();
    const databasePath = join(`${root}-home`, "rerail.sqlite");
    const policy = join(root, "body.rr");
    for (const body of ["before", "after"]) {
      writeFileSync(policy, `.policy "same-set" revision 1\n.decl marker(value: symbol)\nmarker("${body}").\n.violation Seen(at: span, value: symbol) "{value}"\nSeen(@place("result", 1), value) :- marker(value).\n`);
      const report = await scan(root, { databasePath, rr: [policy] });
      expect(report.status, JSON.stringify(report.failures)).toBe("completed");
      expect(report.violations.map(finding => finding.message)).toEqual([body]);
    }
    const store = new Database(scanContentStore(databasePath), { readonly: true });
    try {
      const sets = store.prepare("SELECT count(DISTINCT lane) AS lanes FROM analysis_scope_manifest JOIN corpus_manifests USING(manifest_id) WHERE scope = ? AND policy <> lane GROUP BY policy").all(`cwd:${analysisScopeHome(databasePath)}:${root}`);
      expect(sets).toEqual([{ lanes: 2 }]);
    } finally { store.close(); }
  });

  it("updates renamed files, package modules and ignores in a shared revision cache", async () => {
    const root = checkout();
    const nested = join(root, "src", "shop");
    mkdirSync(nested, { recursive: true });
    writeFileSync(join(nested, "entry.py"), "answer = 42\n");
    git(root, "add", "src/shop/entry.py");
    git(root, "commit", "-qm", "source before package");
    const policy = join(root, "package.rr");
    writeFileSync(policy, `#include "core.rr"\n.policy "module" revision 1\n.language python\n.violation Found(at: span, module: symbol) "{module}"\nFound(@place(p, 1), m) :- doc(_, p, m), contains("src/shop/", p).\n`);
    const options = { revision: "HEAD", analysisScope: "changing-tree", rr: [policy] };
    const before = await scan(root, options);
    expect(before.status, JSON.stringify(before.failures)).toBe("completed");
    expect(before.violations.map(row => [row.path, row.message])).toEqual([["src/shop/entry.py", "src.shop.entry"]]);
    git(root, "mv", "src/shop/entry.py", "src/shop/renamed.py");
    writeFileSync(join(nested, "__init__.py"), "");
    writeFileSync(join(root, ".rerailignore"), "src/shop/\n!/src/shop/renamed.py\n");
    git(root, "add", ".rerailignore", "src/shop/__init__.py");
    git(root, "commit", "-qm", "rename and add package marker");
    const after = await scan(root, options);
    expect(after.status, JSON.stringify(after.failures)).toBe("completed");
    expect(after.violations.map(row => [row.path, row.message])).toEqual([["src/shop/renamed.py", "shop.renamed"]]);
  });

  it("keeps a finding's fingerprint when its line moves, and gives identical matches their own", async () => {
    const root = checkout();
    const before = (await scan(root)).violations[0]!.fingerprint;
    writeFileSync(join(root, "a.py"), "# moved\n\n\nx    =    NEEDLE\ny = NEEDLE\n");
    const after = (await scan(root)).violations.map(violation => violation.fingerprint);
    expect(after[0]).toBe(before);
    expect(new Set(after).size).toBe(2);
  });

  it("keeps the evaluated JSON finding's fingerprint when its file changes before reporting", async () => {
    const root = checkout();
    const baseline = await scan(root);
    const prototype = loadNativeBinding().CodeRuleDslWorkspace.prototype;
    const complete = prototype.completeStorageRevisionAsync;
    const edit = vi.spyOn(prototype, "completeStorageRevisionAsync").mockImplementation(async function (this: typeof prototype) {
      await complete.call(this);
      writeFileSync(join(root, "a.py"), "# edited after evaluation\nx = NEEDLE\n");
    });
    try {
      const raced = await scan(root);
      expect(raced.status, JSON.stringify(raced.failures)).toBe("completed");
      expect(edit).toHaveBeenCalled();
      expect(raced.violations).toEqual(baseline.violations);
    } finally { edit.mockRestore(); }
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
    const provenance = { kind: "ref", ref: "main", commit: git(root, "rev-parse", "main") };
    expect(unapproved.rulesSource).toEqual(provenance);
    expect(unapproved.rules).toEqual([expect.objectContaining({ ruleId: "needle", mode: "log", source: { ...provenance, id: "needle", packId: null } })]);
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

  it("loads distinct Git policy names and reports unreadable policy objects", async () => {
    const root = checkout();
    const missing = "1".repeat(40);
    const tree = (input: string) => execFileSync("git", ["mktree", "--missing"], { cwd: root, input, encoding: "utf8" }).trim();
    const blob = (document: unknown) => execFileSync("git", ["hash-object", "-w", "--stdin"], {
      cwd: root, input: JSON.stringify(document), encoding: "utf8",
    }).trim();
    const policyRef = (rules: string) => git(root, "commit-tree", tree(`040000 tree ${tree(`040000 tree ${rules}\trules\n`)}\t.rerail\n`), "-m", "policy ref");
    const upper = blob(needle("upper", "NEEDLE"));
    const lower = blob(needle("lower", "NEEDLE"));
    const policies = `100644 blob ${upper}\tFinding.json\n100644 blob ${lower}\tfinding.json\n`;
    const distinct = await scan(root, { rulesRef: policyRef(tree(policies)) });
    expect(distinct.status).toBe("completed");
    expect(distinct.violations.map(finding => finding.ruleId).sort()).toEqual(["lower", "upper"]);

    for (const [rules, expectedPath] of [
      [missing, ".rerail/rules"],
      [tree(`${policies}100644 blob ${missing}\tunavailable.json\n`), ".rerail/rules/unavailable.json"],
    ] as const) {
      const rulesRef = policyRef(rules);
      const report = await scan(root, { rulesRef });
      expect(report.status).toBe("failed");
      expect(report.failures).toEqual([expect.objectContaining({ code: "adapter-error", path: `${rulesRef}:${expectedPath}` })]);
    }
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

  it.skipIf(!honoursNoLazyFetch())("lists an edited rename in a partial clone without the source's blob, and fetches nothing", async () => {
    const origin = checkout();
    mkdirSync(join(origin, "old"));
    writeFileSync(join(origin, "old", "before.py"), "a = 1\nb = 2\nc = 3\nx = NEEDLE\n");
    git(origin, "add", "old/before.py");
    git(origin, "commit", "--quiet", "-m", "old");
    git(origin, "config", "uploadpack.allowFilter", "true");
    const clone = `${origin}-clone`;
    roots.push(clone);
    // Sparse: only the top level and the rules are checked out, so old/before.py's blob is never fetched.
    git(origin, "clone", "--quiet", "--filter=blob:none", "--sparse", `file://${origin}`, clone);
    git(clone, "sparse-checkout", "set", ".rerail");
    const source = git(clone, "rev-parse", "HEAD:old/before.py");
    git(clone, "update-index", "--force-remove", "old/before.py");
    writeFileSync(join(clone, "after.py"), "a = 1\nb = 2\nc = 4\nx = NEEDLE\n");
    git(clone, "add", "after.py");

    const report = await scan(clone, { scope: "git-diff" });
    expect(report.status).toBe("completed");
    expect(report.paths).toEqual(["after.py"]);
    expect(report.violations.map(violation => violation.path)).toEqual(["after.py"]);
    expect(partialCloneLacks(clone, source)).toBe(true);
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

  it.each([
    ["nested in the checkout, in a folder it excludes", (root: string) => join(root, ".claude", "worktrees", "probe")],
    ["outside the checkout", (root: string) => `${root}-probe`],
  ])("uses the Rerail home's configuration in a linked worktree %s that no session has recorded", async (_placement, place) => {
    const root = checkout();
    const home = realpathSync(mkdtempSync(join(tmpdir(), "rerail-headless-home-")));
    roots.push(home);
    const databasePath = join(home, "rerail.sqlite");
    setUpTestRepository(root, {}, databasePath);
    writeFileSync(join(root, ".git", "info", "exclude"), ".claude/worktrees/\n");
    const worktree = place(root);
    roots.push(worktree);
    git(root, "worktree", "add", "--quiet", "--detach", worktree, "main");
    const report = await scan(realpathSync(worktree), { databasePath });
    expect({ status: report.status, rulesSource: report.rulesSource, failures: report.failures }).toEqual({ status: "completed", rulesSource: { kind: "home" }, failures: [] });
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
