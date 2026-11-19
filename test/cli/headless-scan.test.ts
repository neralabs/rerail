import { execFileSync, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { chmodSync, existsSync, mkdirSync, mkdtempSync, readFileSync, realpathSync, renameSync, rmSync, statSync, utimesSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { deflateSync } from "node:zlib";
import Database from "better-sqlite3";
import { afterAll, afterEach, describe, expect, it, vi } from "vitest";
import { runAuditCommand, SCAN_VIOLATIONS_EXIT } from "../../src/cli/audit-cli.js";
import { runRuleApproveCommand } from "../../src/cli/rule-approval-cli.js";
import { runHeadlessScan, type ScanReport } from "../../src/cli/headless-scan.js";
import { acquireDirectoryAnalysisGeneration, analysisScopeHome, collectAnalysisCaches, queueAnalysisScopeReleases, readAnalysisScopeGeneration, scanContentStore, settleAnalysisScopeReleases } from "../../src/engine/native/analysis-scope.js";
import { loadNativeBinding } from "../../src/engine/native/native.js";
import * as builds from "../../src/engine/native/rerail-build.js";
import { openRerailDatabase } from "../../src/engine/storage/database.js";
import { recordCheckout } from "../../src/engine/catalog/repository-registry.js";
import { setCheckoutStarted, stopRerail } from "../../src/runtime/repository-start.js";
import { honoursNoLazyFetch, partialCloneLacks } from "../support/partial-clone.js";
import { queryContentStore } from "../support/query-content-store.js";
import { setUpTestRepository } from "../support/repository-rules.js";

const roots: string[] = [];
afterEach(() => {
  for (const root of roots.splice(0)) {
    loadNativeBinding().retireUpkeepUnder?.(root);
    rmSync(root, { recursive: true, force: true });
  }
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

const directoryOwnerCount = (store: string): number | undefined =>
  queryContentStore<{ count: number }>(store, "SELECT count(*) AS count FROM analysis_scope_workspace_snapshot")[0]?.count;

describe("headless rerail scan", () => {
  const suiteStore = process.env.RERAIL_CONTENT_STORE_PATH;
  afterAll(() => {
    if (suiteStore) loadNativeBinding().retireUpkeepUnder?.(dirname(suiteStore));
  });

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

  it("keeps the selected checkout, source and rule repository together inside a foreign Git environment", async () => {
    const root = checkout(), other = checkout(), home = root + "-home";
    git(other, "switch", "-qc", "other");
    git(other, "rm", "-q", "a.py");
    git(other, "commit", "-qm", "other HEAD");
    const head = git(root, "rev-parse", "HEAD");
    expect(git(other, "rev-parse", "HEAD")).not.toBe(head);
    const engine = loadNativeBinding(), pinned = new engine.GitRevisionSource(root, "HEAD", "explicit");
    writeFileSync(join(root, ".git", "rerail-repository-id"), "selected-checkout\n");
    const policy = join(root, "which.rr");
    writeFileSync(policy, `#include "git.rr"
.policy "which" revision 1
.violation Seen(at: span, oid: symbol) "{oid}"
git_revision_request(r, "HEAD") :- git_root_repo(r).
git_revision_request(r, ":a.py") :- git_root_repo(r).
Seen(@place("git",1), oid) :- git_revision(r, "HEAD", oid), git_revision(r, ":a.py", _).
`);
    vi.stubEnv("GIT_DIR", join(other, ".git"));
    try {
      for (const worktree of [false, true]) {
        if (worktree) {
          vi.stubEnv("GIT_WORK_TREE", other);
          vi.stubEnv("GIT_INDEX_FILE", join(other, ".git", "index"));
        }
        let output = "";
        const stdout = { write: (value: string) => { output += value; return true; } };
        expect(await runAuditCommand("scan", ["--repo", root, "--revision", "HEAD", "--analysis-scope", "explicit",
          "--rr", policy, "--json", "--no-send"], { home, stdout })).toBe(0);
        expect(JSON.parse(output)).toMatchObject({
          status: "completed", checkout: { path: root, ref: "refs/heads/main", head }, source: { commit: head },
          repository: { key: "clone:selected-checkout" }, violations: [{ message: head }],
        });
      }
      const packDirectory = join(root, ".git", "objects", "pack");
      const earlier = new Date("2020-01-01T00:00:00Z");
      utimesSync(packDirectory, earlier, earlier);
      const rows = JSON.parse(await engine.scanRrPoliciesAsync([policy], root, join(home, "reopened.sqlite"), "", undefined, undefined, pinned));
      expect(rows[0].diagnostics.map((row: { message: string }) => row.message)).toEqual([head]);
      expect(existsSync(join(other, ".git", "rerail-repository-id"))).toBe(false);
    } finally { vi.unstubAllEnvs(); }
  }, 30_000);

  it.skipIf(process.platform === "win32")("preserves spaces in the checkout and in paths from a list", async () => {
    const sibling = checkout();
    const root = sibling + " ";
    const home = sibling + "-home";
    roots.push(root);
    git(sibling, "clone", "--quiet", sibling, root);
    writeFileSync(join(root, " lead.py"), "NEEDLE\n");
    git(root, "add", "--", " lead.py");
    git(root, "commit", "-qm", "distinct checkout");
    const list = join(root, "paths.txt");
    writeFileSync(list, " lead.py\r\nmissing.py\n");
    vi.stubEnv("RERAIL_HOME", home);
    try {
      for (const selection of [["--path", " lead.py", "--path", "missing.py"], ["--paths-from", list]]) {
        let output = "";
        const stdout = { write: (text: string) => { output += text; return true; } };
        const args = ["--repo", root, "--revision", "HEAD", "--analysis-scope", "spaces", "--json", "--no-send", "--fail-on-violations", ...selection];
        expect(await runAuditCommand("scan", args, { home, stdout })).toBe(SCAN_VIOLATIONS_EXIT);
        expect(JSON.parse(output)).toMatchObject({
          checkout: { path: root }, source: { commit: git(root, "rev-parse", "HEAD") }, paths: [" lead.py"],
          requestedPaths: [" lead.py", "missing.py"], skippedPaths: ["missing.py"], violations: [{ path: " lead.py" }],
        });
      }
    } finally {
      vi.unstubAllEnvs();
    }
  });

  it("reports the original ref bytes when a packed branch has a replacement-character sibling", async () => {
    const root = checkout(), head = git(root, "rev-parse", "HEAD");
    git(root, "commit", "--allow-empty", "-qm", "other tip");
    const sibling = git(root, "rev-parse", "HEAD");
    const textName = "refs/heads/raw-�", rawName = Buffer.concat([Buffer.from("refs/heads/raw-"), Buffer.from([255])]);
    writeFileSync(join(root, ".git", "packed-refs"), Buffer.concat([Buffer.from(`${sibling} ${textName}\n${head} `), rawName, Buffer.from("\n")]));
    writeFileSync(join(root, ".git", "HEAD"), Buffer.concat([Buffer.from("ref: "), rawName, Buffer.from("\n")]));
    const report = await scan(root, { revision: "HEAD", analysisScope: "ref-bytes" });
    expect(report).toMatchObject({ status: "completed", checkout: { head, ref: null, refHex: rawName.toString("hex") }, source: { commit: head } });
    const reported = Buffer.from(report.checkout.refHex!, "hex");
    expect(execFileSync("git", ["cat-file", "--batch-check=%(objectname)"], {
      cwd: root, encoding: "utf8", input: Buffer.concat([reported, Buffer.from("\n")]),
    }).trim()).toBe(head);
    expect(git(root, "rev-parse", textName)).toBe(sibling);
    writeFileSync(join(root, ".git", "HEAD"), head + "\n");
    const detached = await scan(root, { revision: "HEAD", analysisScope: "ref-bytes" });
    expect(detached.checkout).toEqual({ path: root, head, ref: null });
  }, 30_000);

  it("selects a name starting with two dashes from the command line and a path list", async () => {
    const root = checkout(), home = root + "-home", path = "--alpha.py";
    writeFileSync(join(root, path), "NEEDLE\n");
    git(root, "add", "--", path);
    git(root, "commit", "-qm", "leading dashes");
    const list = join(root, "paths.txt");
    writeFileSync(list, path + "\n");
    const args = ["--repo", root, "--revision", "HEAD", "--analysis-scope", "literal", "--fail-on-violations", "--no-send", "--json"];
    for (const selection of [[`--path=${path}`], ["--paths-from", list]]) {
      let output = "";
      const stdout = { write: (text: string) => { output += text; return true; } };
      expect(await runAuditCommand("scan", [...args, ...selection], { home, stdout })).toBe(SCAN_VIOLATIONS_EXIT);
      expect(JSON.parse(output)).toMatchObject({ paths: [path], requestedPaths: [path], skippedPaths: [], violations: [{ path }] });
    }
    for (const missing of [["--path="], ["--path", "--json"]]) {
      await expect(runAuditCommand("scan", [...args, ...missing], { home })).rejects.toThrow("--path requires a value");
    }
  }, 30_000);

  it("rejects an invalid UTF-8 path list before it can select a replacement-character name", async () => {
    const root = checkout();
    writeFileSync(join(root, "f�.py"), "NEEDLE");
    const list = join(root, "paths.txt");
    writeFileSync(list, Buffer.concat([Buffer.from("a.py\r\nf"), Buffer.from([0xff]), Buffer.from(".py\n")]));
    await expect(runAuditCommand("scan", ["--repo", root, "--paths-from", list], { home: root + "-home" }))
      .rejects.toThrow(/paths\.txt line 2 contains an invalid UTF-8 path/u);
  });

  it("preserves distinct Git diff names when one name contains invalid UTF-8", async () => {
    const root = checkout();
    const valid = "f�.py";
    const invalid = Buffer.from([0x66, 0xff, 0x2e, 0x70, 0x79]);
    const blob = git(root, "rev-parse", "HEAD:a.py");
    writeFileSync(join(root, valid), "NEEDLE");
    const names = [Buffer.from(valid), invalid];
    execFileSync("git", ["update-index", "-z", "--index-info"], { cwd: root, input: Buffer.concat(
      names.flatMap(name => [Buffer.from(`100644 ${blob}\t`), name, Buffer.from([0])])) });
    execFileSync("git", ["update-index", "--skip-worktree", "-z", "--stdin"], { cwd: root, input: Buffer.concat(
      names.flatMap(name => [name, Buffer.from([0])])) });
    const report = await scan(root, { scope: "git-diff" });
    expect(report.paths).toEqual([valid]);
    expect(report.status).toBe("failed");
    expect(report.failures).toContainEqual(expect.objectContaining({ path: "[hex:66ff2e7079]", code: "source-incomplete" }));
    expect(report.violations.map(row => row.path)).toContain(valid);
  });

  it("returns JSON and RR findings without writing a scan or phase directory", () => {
    const root = checkout(), home = root + "-home";
    const tmp = join(home, "temp");
    mkdirSync(tmp, { recursive: true });
    writeFileSync(join(root, ".rerail/rules/policy.rr"), '#include "core.rr"\n.policy "calls" revision 1\n.language python\n.violation C(at: node) "call"\nC(n) :- kind(n, "call").\n');
    writeFileSync(join(root, "a.py"), "NEEDLE()\n");
    git(root, "add", "a.py");
    git(root, "commit", "-qm", "two kinds of findings");
    const script = `
      import {watch} from "node:fs";
      import {runHeadlessScan} from "./src/cli/headless-scan.ts";
      const [repo, databasePath, temp] = process.argv.slice(2);
      const created = new Set();
      const watcher = watch(temp, (_, name) => { if (name?.startsWith("rerail-scan-") || name?.startsWith("rerail-phase-")) created.add(name); });
      try {
        const report = await runHeadlessScan({ repo, databasePath, packs: [], scope: "repository", revision: "HEAD", analysisScope: "memory", version: "test" });
        await new Promise(done => setTimeout(done, 50));
        console.log(JSON.stringify({ created: [...created], status: report.status, rules: report.violations.map(v => v.ruleId).sort(), failures: report.failures }));
      } finally { watcher.close(); }`;
    const result = execFileSync(process.execPath, ["--import", "tsx", "--input-type=module", "-", root, join(home, "home.sqlite"), tmp], {
      input: script, encoding: "utf8", env: { ...process.env, TMPDIR: tmp, TMP: tmp, TEMP: tmp, RERAIL_CONTENT_STORE_PATH: join(home, "content.sqlite") },
    });
    expect(JSON.parse(result)).toEqual({ created: [], status: "completed", rules: ["calls", "needle"], failures: [] });
  }, 45_000);

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

  it.each(["revision", "filesystem"])("renders the current diagnostic text after a message-only edit on the %s provider", async provider => {
    const root = checkout();
    const policy = join(root, "wording.rr");
    const options = provider === "revision" ? { revision: "HEAD", analysisScope: "wording" } : {};
    for (const word of ["first", "second", "first"]) {
      writeFileSync(policy, `#include "git.rr"
.policy "wording" revision 1
.violation Seen(at: span, value: symbol) "${word} {value}"
Seen(@place("git", 1), "message") :- git_root_repo(_).
`);
      const report = await scan(root, { ...options, rr: [policy] });
      expect(report.status).toBe("completed");
      expect(report.violations.map(row => row.message)).toEqual([`${word} message`]);
    }
  });

  it("uses the current include and its matching policy digest after an edit and a revert", async () => {
    const root = checkout();
    const policy = join(root, "live.rr");
    writeFileSync(policy, `
#include "core.rr"
#include "live-message.rr"
.policy "live" revision 1
.violation Message(at: span, text: symbol) "{text}"
Message(@place("policy", 1), t) :- live_message(t).
`);
    const run = async (message: string) => {
      writeFileSync(join(root, "live-message.rr"), `.decl live_message(m: symbol)\nlive_message("${message}").\n`);
      const result = await scan(root, { revision: "HEAD", analysisScope: "live", rr: [policy] });
      expect(result.status, JSON.stringify(result.failures)).toBe("completed");
      expect(result.violations.map(v => v.message)).toEqual([message]);
      return result.rules[0]!.payloadHash;
    };
    const first = await run("first");
    expect(await run("second")).not.toBe(first);
    expect(await run("first")).toBe(first);
  });

  it("selects the whole revision when --path names the root", async () => {
    const root = checkout();
    const home = `${root}-home`;
    vi.stubEnv("RERAIL_HOME", home);
    try {
      for (const path of [".", "./"]) {
        let output = "";
        const stdout = { write: (text: string) => { output += text; return true; } };
        const result = await runAuditCommand("scan", [
          "--repo", root, "--revision", "HEAD", "--analysis-scope", "root", "--path", path,
          "--json", "--no-send", "--fail-on-violations",
        ], { home, stdout });
        expect(result).toBe(SCAN_VIOLATIONS_EXIT);
        expect(JSON.parse(output)).toMatchObject({
          paths: null, status: "completed", violations: [expect.objectContaining({ path: "a.py" })],
        });
      }
    } finally {
      vi.unstubAllEnvs();
    }
  });

  it("reports invalid UTF-8 in a JSON policy loaded from --rules-ref", async () => {
    const root = checkout();
    const path = ".rerail/rules/bad.json";
    const policy = JSON.stringify(needle("bad-utf8", "NEEDLE")).replace("No NEEDLE", "No ÿ NEEDLE");
    writeFileSync(join(root, path), Buffer.from(policy, "latin1"));
    git(root, "add", path);
    git(root, "commit", "-qm", "invalid policy encoding");
    const report = await scan(root, { revision: "HEAD~1", analysisScope: "encoding", rulesRef: "HEAD" });
    expect(report.status).toBe("failed");
    expect(report.failures).toEqual([expect.objectContaining({ path: `HEAD:${path}`, message: "Git JSON policy has invalid UTF-8" })]);
    expect(report.violations.map(finding => finding.ruleId)).toEqual(["needle"]);
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
    writeFileSync(policy, `
#include "git.rr"
.policy "resolve-head" revision 1
git_revision_request(r, "HEAD") :- git_root_repo(r).
.violation Resolved(at: span, oid: symbol) "{oid}"
Resolved(@place("commit", 1), oid) :- git_revision(_, "HEAD", oid).
`);
    for (let attempt = 0; attempt < 2; attempt++) {
      const result = await scan(root, { revision: missingRoot, analysisScope: "resolve", rr: [policy] });
      expect(result.status, JSON.stringify(result.failures)).toBe("completed");
      expect(result.violations.map(finding => finding.message)).toEqual([git(root, "rev-parse", "HEAD")]);
    }
  });

  it("resolves relative imports using package markers outside the selected revision paths", async () => {
    const root = checkout();
    mkdirSync(join(root, "pkg"));
    const paths = ["pkg/a.py", "pkg/b.py"];
    writeFileSync(join(root, paths[0]!), "from .b import helper\ndef caller():\n    helper()\n");
    writeFileSync(join(root, paths[1]!), "def helper():\n    pass\n");
    git(root, "add", "--", ...paths);
    git(root, "commit", "-qm", "loose modules");
    writeFileSync(join(root, "pkg/__init__.py"), "");
    git(root, "add", "pkg/__init__.py");
    git(root, "commit", "-qm", "package marker");
    const policy = join(root, "links.rr");
    writeFileSync(policy, `#include "core.rr"
.policy "links" revision 1
.language python
.violation Edge(at: node, via: symbol) "{via}"
Edge(n, via) :- reference_edge(n, _, via).
`);
    for (const [revision, expected] of [["HEAD", ["pkg/a.py"]], ["HEAD~1", []], ["HEAD", ["pkg/a.py"]]] as const) {
      const result = await scan(root, { revision, paths, analysisScope: "package", rr: [policy] });
      expect(result.status, JSON.stringify(result.failures)).toBe("completed");
      expect(result.violations.map(finding => finding.path)).toEqual(expected);
    }
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
    const rawDirectory = execFileSync("git", ["mktree", "-z", "--missing"], {
      cwd: root, input: Buffer.concat([Buffer.from(`040000 tree ${missing}\t`), Buffer.from([0xfc, 0])]), encoding: "utf8",
    }).trim();
    const raw = await scan(root, { revision: rawDirectory, analysisScope: "explicit-walk", rr: [policy] });
    expect(raw.violations.map(finding => finding.message).sort()).toEqual(
      [`unreadable tree ${rawDirectory}`, `unreadable tree ${missing}`].sort(),
    );
  }, 30_000);

  it("derives public directory placements only for readable trees while keeping errors on the frontier", async () => {
    const root = checkout(), head = git(root, "rev-parse", "HEAD"), missing = "1".repeat(40);
    const blob = git(root, "rev-parse", "HEAD:a.py");
    const leaf = execFileSync("git", ["mktree"], { cwd: root, input: `100644 blob ${blob}\ta.py\n`, encoding: "utf8" }).trim();
    const entries = [["absent", missing], ["left", leaf], ["right", leaf], ["wrong", head]];
    const tree = execFileSync("git", ["hash-object", "--literally", "-t", "tree", "-w", "--stdin"], {
      cwd: root, input: Buffer.concat(entries.flatMap(([name, id]) => [Buffer.from(`40000 ${name}\0`), Buffer.from(id!, "hex")])), encoding: "utf8",
    }).trim();
    const policy = join(root, "directories.rr");
    writeFileSync(policy, `#include "git.rr"
.policy "directories" revision 1
git_revision_request(r,"HEAD") :- git_root_repo(r).
git_tree_walk_request(r,t) :- git_source_tree(r,t).
git_tree_walk_request(r,c) :- git_revision(r,"HEAD",c).
.violation Directory(at: span, root: symbol, path: symbol) "{root}/{path}"
Directory(@place("git",1), root, p) :- git_tree_directory(_,root,p,_).
.violation Problem(at: span, root: symbol, id: symbol, reason: symbol) "{root}/{id} {reason}"
Problem(@place("git",1), root, id, reason) :- git_tree_walk_error(_,root,id,reason).
`);
    const result = await scan(root, { revision: tree, analysisScope: "directories", rr: [policy] });
    expect(result.status, JSON.stringify(result.failures)).toBe("completed");
    expect(result.violations.map(v => v.message).sort()).toEqual([
      `${tree}/`, `${tree}/left`, `${tree}/right`, `${tree}/${missing} missing`,
      `${tree}/${head} not-a-tree`, `${head}/${head} not-a-tree`,
    ].sort());
  });

  it("identifies duplicate tree entries and rejects malformed commit metadata at selection", async () => {
    const root = checkout();
    const blob = git(root, "rev-parse", "HEAD:a.py");
    const entry = Buffer.concat([Buffer.from("100644 same.py\0"), Buffer.from(blob, "hex")]);
    const literal = (kind: string, input: Buffer | string) => execFileSync("git",
      ["hash-object", "-t", kind, "--literally", "-w", "--stdin"], { cwd: root, input, encoding: "utf8" }).trim();
    const duplicate = literal("tree", Buffer.concat([entry, entry]));
    const source = new (loadNativeBinding().GitRevisionSource)(root, duplicate, "malformed");
    expect(() => source.readDirectory(".")).toThrow(/duplicate entry.*same\.py/u);
    const tree = execFileSync("git", ["mktree"], { cwd: root, input: `040000 tree ${duplicate}\tbad\n`, encoding: "utf8" }).trim();
    const result = await scan(root, { revision: tree, analysisScope: "malformed" });
    expect(result.status).toBe("failed");
    expect(result.failures).toContainEqual(expect.objectContaining({ path: "bad", message: expect.stringContaining(duplicate) }));
    const commit = literal("commit", `tree ${tree}\n\nmissing signatures\n`);
    await expect(scan(root, { revision: commit, analysisScope: "malformed" })).rejects.toThrow(`malformed Git commit ${commit}`);
    const engine = loadNativeBinding();
    const open = (revision: string) => new engine.GitRevisionSource(root, revision, "malformed");
    const tag = (id: string, kind: string) => literal("tag", `object ${id}\ntype ${kind}\ntag audit\ntagger Test <a@b> 123 +0000\n\nfixture\n`);
    const wrongTag = tag(tree, "commit");
    expect(() => open(tag(wrongTag, "tag"))).toThrow(/declares.*commit, found tree/u);
    expect(open(tag(tree, "tree")).tree).toBe(tree);
    expect(() => open(tag("1".repeat(40), "tree"))).toThrow(/11111111/u);
    for (const expression of [`${commit}^{tree}`, `${commit}:bad`]) {
      expect(() => open(expression)).toThrow(`malformed Git commit ${commit}`);
    }
    const unsorted = literal("tree", Buffer.concat(["z", "a"].flatMap(name => [Buffer.from(`100644 ${name}\0`), Buffer.from(blob, "hex")])));
    expect(() => open(unsorted).readDirectory(".")).toThrow(/entries out of order/u);
    const head = git(root, "rev-parse", "HEAD");
    const makeCommit = (tree: string, parent: string) => literal("commit",
      `tree ${tree}\nparent ${parent}\nauthor Test <a@b> 123 +0000\ncommitter Test <a@b> 123 +0000\n\nlinks\n`);
    const wrongRoot = makeCommit(head, head);
    for (const revision of [wrongRoot, tag(wrongRoot, "commit")]) {
      await expect(scan(root, { revision, analysisScope: "malformed" })).rejects.toThrow(/is commit, expected tree/u);
    }
    const singleEntry = (mode: string, id: string) => literal("tree", Buffer.concat([Buffer.from(`${mode} fake\0`), Buffer.from(id, "hex")]));
    const wrongLinks = [
      `${wrongRoot}^{tree}`, `${makeCommit(tree, tree)}^1`, `${makeCommit(tree, tree)}~1`,
      `${singleEntry("100644", tree)}:fake`, `${singleEntry("040000", head)}:fake/`, `${singleEntry("120000", tree)}:fake`,
    ];
    for (const expression of wrongLinks) expect(() => open(expression)).toThrow(/expected (tree|commit|blob)/u);
    expect(open(`${makeCommit(tree, head)}^1`).commit).toBe(head);
    expect(open(`${singleEntry("040000", tree)}:fake/`).tree).toBe(tree);
    const foreign = "1".repeat(40);
    const gitlink = `${singleEntry("160000", foreign)}:fake`;
    const errors = new Map([
      [`${commit}^{tree}`, "malformed-object"], [`${commit}:bad`, "malformed-object"],
      [`${wrongTag}^{}`, "wrong-object-kind"], [`${unsorted}:a`, "malformed-object"],
      ...wrongLinks.map(expr => [expr, "wrong-object-kind"] as const),
    ]);
    const policy = join(root, "navigation.rr");
    writeFileSync(policy, `#include "git.rr"
.policy "malformed-links" revision 1
.violation Error(at: span, exp: symbol, reason: symbol) "{exp} {reason}"
${[...errors.keys(), gitlink].map(expr => `git_revision_request(r, "${expr}") :- git_root_repo(r).`).join("\n")}
Error(@place("git", 1), e, reason) :- git_revision_error(_, e, reason).
.violation Found(at: span, oid: symbol) "foreign {oid}"
Found(@place("git", 1), oid) :- git_revision(_, "${gitlink}", oid).
`);
    const checked = await scan(root, { revision: "HEAD", analysisScope: "malformed", rr: [policy] });
    expect(checked.violations.map(row => row.message).sort()).toEqual([
      ...[...errors].map(([expr, reason]) => `${expr} ${reason}`), `foreign ${foreign}`,
    ].sort());
  });

  it("reads commit metadata with byte header names and returns dates independently of signature text", async () => {
    const root = checkout(), tree = git(root, "rev-parse", "HEAD^{tree}");
    const policy = join(root, "bytes.rr");
    writeFileSync(policy, `#include "git.rr"
.policy "commit-bytes" revision 1
git_object_request(r,c) :- git_source_commit(r,c).
.violation Field(at: span, value: symbol) "{value}"
Field(@place("name",1), s) :- git_source_commit(r,c), git_subject(r,c,s).
Field(@place("header",1), n) :- git_header(_,_,0,n,"6f6b").
Field(@place("raw-header",1), n) :- git_header_bytes(_,_,1,n,"76616c").
Field(@place("time",1), role) :- git_signature_time(_,_,role,123,9000).
Field(@place("raw-signature",1), role) :- git_signature_bytes(_,_,role,_).
`);
    for (const role of ["author", "committer"]) {
      const signature = (r: string) => Buffer.concat([Buffer.from(r + " "), r === role ? Buffer.from([0xfc]) : Buffer.from("Test"), Buffer.from(" <a@example.org> 123 +0230\n")]);
      const bytes = Buffer.concat([Buffer.from(`tree ${tree}\n`), signature("author"), signature("committer"),
        Buffer.from("normal ok\nx"), Buffer.from([0xff]), Buffer.from(" val\n\nbyte metadata\n")]);
      const commit = execFileSync("git", ["hash-object", "-t", "commit", "--literally", "-w", "--stdin"], { cwd: root, input: bytes, encoding: "utf8" }).trim();
      expect(git(root, "show", "-s", "--format=%s", commit)).toBe("byte metadata");
      expect(git(root, "show", "-s", "--format=%at %ct", commit)).toBe("123 123");
      git(root, "fsck", "--strict", "--no-reflogs", commit);
      for (const revision of [commit, `${commit}^{/byte metadata}`]) {
        const report = await scan(root, { revision, analysisScope: "bytes", rr: [policy] });
        expect(report.source).toMatchObject({ commit, tree });
        expect(report.status, JSON.stringify(report.failures)).toBe("completed");
        expect(report.violations.map(v => `${v.path} ${v.message}`).sort()).toEqual([
          "header normal", "name byte metadata", "raw-header 78ff", `raw-signature ${role}`, "time author", "time committer",
        ]);
      }
    }
  }, 30_000);

  it("requires a directory for explicit paths ending in a separator or slash dot on both providers", async () => {
    const root = checkout();
    mkdirSync(join(root, "src"));
    writeFileSync(join(root, "src/b.py"), "NEEDLE\n");
    git(root, "add", "src/b.py");
    git(root, "commit", "-qm", "directory");
    const source = new (loadNativeBinding().GitRevisionSource)(root, "HEAD", "paths");
    for (const path of ["a.py/", "a.py/."]) expect(source.containsPath(path)).toBe(false);
    for (const path of ["src/", "src/.", "./"]) expect(source.containsPath(path)).toBe(true);
    expect(source.containsPath("src/../a.py")).toBe(true);
    expect(source.readFile("src/../a.py", 100)).toEqual(source.readFile("a.py", 100));
    expect(source.readFile("src//b.py", 100)).toEqual(source.readFile("src/b.py", 100));
    expect(source.readDirectory("src/")).toEqual(source.readDirectory("src"));
    expect(source.readDirectory("./")).toEqual(source.readDirectory("."));
    expect(source.containsPath("../outside")).toBe(false);
    expect(() => source.readFile("../outside", 100)).toThrow();
    expect(() => source.readDirectory("../outside")).toThrow();
    expect(() => source.readFile("a.py/.", 100)).toThrow();
    expect(() => source.readDirectory("a.py/.")).toThrow();
    for (const limit of [-1, 1.9, 2 ** 32, NaN, Infinity]) {
      expect(() => source.readFile("a.py", limit)).toThrow(/byte limit/u);
    }
    expect(source.readFile("a.py", 0)).toBeNull();
    expect(source.readFile("a.py", 2 ** 32 - 1)).toEqual(readFileSync(join(root, "a.py")));
    for (const input of [{}, { revision: "HEAD", analysisScope: "paths" }]) {
      const paths = ["a.py/", "a.py/.", "src/"];
      const report = await scan(root, { ...input, paths });
      expect(report.status).toBe("completed");
      expect(report.requestedPaths).toEqual(paths);
      expect(report.skippedPaths).toEqual(["a.py/", "a.py/."]);
      expect(report.violations.map(row => row.path)).toEqual(["src/b.py"]);
    }
  }, 30_000);

  it("normalizes repository-relative selections before both lookup and rule evaluation", async () => {
    const root = checkout();
    mkdirSync(join(root, "src"));
    writeFileSync(join(root, "src/a.py"), "NEEDLE\n");
    git(root, "add", "--", "src/a.py");
    git(root, "commit", "-qm", "nested file");
    const paths = ["src/../src/a.py", "./src//a.py", "./src/absent", "src/a.py/."];
    for (const input of [{}, { revision: "HEAD", analysisScope: "paths" }]) {
      const report = await scan(root, { ...input, paths });
      expect(report.status).toBe("completed");
      expect(report.paths).toEqual(["src/a.py"]);
      expect(report.requestedPaths).toEqual(paths);
      expect(report.skippedPaths).toEqual(["./src/absent", "src/a.py/."]);
      expect(report.violations.map(v => v.path)).toEqual(["src/a.py"]);
      for (const outside of ["../outside", "src/../../outside", join(root, "src/a.py")]) {
        await expect(scan(root, { ...input, paths: [outside] })).rejects.toThrow(/repository-relative/u);
      }
    }
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
    expect(report.skippedPaths).toEqual(["absent.py"]);
    expect(report.violations.map(finding => finding.path)).toEqual(["a.py"]);
    expect(report.failures).toEqual([expect.objectContaining({ code: "adapter-error", path: "gaps/a.py" })]);
  });

  it("reports each placement of a missing subtree and keeps findings in readable files", async () => {
    const root = checkout();
    const blob = git(root, "rev-parse", "HEAD:a.py");
    const missing = "1".repeat(40);
    const tree = execFileSync("git", ["mktree", "--missing"], {
      cwd: root,
      input: `100644 blob ${blob}\ta.py\n040000 tree ${missing}\tfirst\n040000 tree ${missing}\tsecond\n`,
      encoding: "utf8",
    }).trim();
    const report = await scan(root, { revision: tree, analysisScope: "partial-tree" });
    expect(report.status).toBe("failed");
    expect(report.violations.map(finding => finding.path)).toEqual(["a.py"]);
    expect(report.failures.map(failure => [failure.path, failure.message])).toEqual([
      ["first", `cannot inventory Git tree ${missing}: missing`],
      ["second", `cannot inventory Git tree ${missing}: missing`],
    ]);
  });

  it("rechecks loose parent trees and clears repaired errors in a reused revision view", async () => {
    const root = checkout();
    mkdirSync(join(root, "src"));
    writeFileSync(join(root, "src/nested.py"), "nested()\n");
    writeFileSync(join(root, "plain.py"), "plain()\n");
    git(root, "add", "src/nested.py", "plain.py");
    git(root, "commit", "-qm", "two readable files");
    const home = `${root}-home`;
    mkdirSync(home);
    const policy = join(home, "calls.rr");
    writeFileSync(policy, `
#include "core.rr"
.policy "calls" revision 1
.language python
.violation Call(at: node) "call"
Call(n) :- kind(n, "call").
`);
    const engine = loadNativeBinding();
    const source = new engine.GitRevisionSource(root, "HEAD", "repair");
    const store = join(home, "content.sqlite");
    let sequence = 0;
    const evaluate = async (): Promise<string[]> => {
      const [report] = JSON.parse(await engine.scanRrPoliciesAsync(
        [policy], root, store, join(home, `output-${sequence++}`), undefined, undefined, source,
      ));
      return report.diagnostics.map((finding: { path: string }) => finding.path).sort();
    };
    const corrupt = (revision: string, bad: string) => {
      const oid = git(root, "rev-parse", revision);
      const path = join(root, ".git", "objects", oid.slice(0, 2), oid.slice(2));
      const original = readFileSync(path);
      chmodSync(path, 0o644);
      writeFileSync(path, deflateSync(Buffer.from(bad)));
      return () => writeFileSync(path, original);
    };

    expect(await evaluate()).toEqual(["plain.py", "src/nested.py"]);
    const restoreTree = corrupt("HEAD:src", "tree 0\0");
    expect(() => source.readFile("src/nested.py", 100)).toThrow(/checksum/u);
    expect(await evaluate()).toEqual(["plain.py"]);
    expect(source.sourceFailures()).toEqual([expect.objectContaining({ path: "src" })]);
    restoreTree();
    expect(source.readDirectory("src")).toEqual([expect.objectContaining({ name: "nested.py" })]);
    expect(source.sourceFailures()).toEqual([]);
    expect(await evaluate()).toEqual(["plain.py", "src/nested.py"]);
    expect(source.sourceFailures()).toEqual([]);

    const restoreBlob = corrupt("HEAD:src/nested.py", "blob 1\0x");
    expect(await evaluate()).toEqual(["plain.py"]);
    expect(source.sourceFailures()).toEqual([expect.objectContaining({ path: "src/nested.py" })]);
    restoreBlob();
    expect(source.readFile("src/nested.py", 100)?.toString()).toBe("nested()\n");
    expect(source.sourceFailures()).toEqual([]);
    expect(await evaluate()).toEqual(["plain.py", "src/nested.py"]);
    expect(source.sourceFailures()).toEqual([]);
  });

  it.each(["commit", "tag"])("rejects a selected Git %s with a mismatched checksum", async kind => {
    const root = checkout();
    if (kind === "tag") git(root, "tag", "-am", "audit tag", "selected");
    const revision = kind === "tag" ? "selected" : "HEAD";
    const oid = git(root, "rev-parse", revision);
    const object = join(root, ".git", "objects", oid.slice(0, 2), oid.slice(2));
    expect((await scan(root, { revision, analysisScope: "selection" })).status).toBe("completed");
    const old = execFileSync("git", ["cat-file", kind, oid], { cwd: root });
    const body = Buffer.concat([old, Buffer.from("\nchanged message\n")]);
    rmSync(object);
    writeFileSync(object, deflateSync(Buffer.concat([Buffer.from(`${kind} ${body.length}\0`), body])));
    await expect(scan(root, { revision, analysisScope: "selection" })).rejects.toThrow(/Git object checksum failed/u);
  });

  it("rechecks packed trees after corruption and restoration in the same revision view", () => {
    const root = checkout();
    mkdirSync(join(root, "src"));
    writeFileSync(join(root, "src", "nested.py"), "nested = 1\n");
    git(root, "add", "src");
    git(root, "commit", "-qm", "nested source");
    const tree = git(root, "rev-parse", "HEAD:src");
    const objects = join(root, ".git", "objects");
    const base = join(objects, "pack", "test-tree");
    const hash = execFileSync("git", ["pack-objects", base], { cwd: root, input: `${tree}\n`, encoding: "utf8" }).trim();
    const pack = `${base}-${hash}.pack`;
    const index = `${base}-${hash}.idx`;
    const row = git(root, "verify-pack", "-v", index).split("\n").find(line => line.startsWith(tree))!;
    const good = readFileSync(pack);
    let payload = Number(row.split(/\s+/u)[4]);
    while ((good[payload++]! & 128) !== 0) { /* Skip the entry header. */ }
    const broken = Buffer.from(good);
    broken[payload] = 0; // Invalid Zlib header.
    rmSync(join(objects, tree.slice(0, 2), tree.slice(2)));
    const source = new (loadNativeBinding().GitRevisionSource)(root, "HEAD", "warm-pack");
    const entries = [{ name: "nested.py", mode: 0o100644 }];
    expect(source.readDirectory("src")).toEqual(entries);

    chmodSync(pack, 0o644);
    writeFileSync(pack, broken);
    expect(() => source.readDirectory("src")).toThrow(/pack entry|decompress|checksum/iu);
    writeFileSync(`${pack}.replacement`, good);
    renameSync(`${pack}.replacement`, pack);
    expect(source.readDirectory("src")).toEqual(entries);
  });

  it.each(["loose", "packed"])("rejects revision expressions redirected by a corrupt %s object", async storage => {
    const root = checkout();
    const first = git(root, "rev-parse", "HEAD");
    const tree = git(root, "rev-parse", "HEAD^{tree}");
    const other = git(root, "commit-tree", tree, "-m", "unrelated root");
    git(root, "commit", "--allow-empty", "-qm", "current commit");
    git(root, "tag", "-am", "selects the first commit", "selected", first);
    const objects = join(root, ".git", "objects");
    const revisions = new Map([
      ["HEAD^", first], ["HEAD~1", first], ["selected^{}", first], ["HEAD^{tree}", tree],
      ["HEAD^{/seed}", first], ["HEAD:a.py", git(root, "rev-parse", "HEAD:a.py")],
    ]);
    const source = loadNativeBinding().GitRevisionSource;
    const policy = join(root, "expressions.rr");
    writeFileSync(policy, `
#include "git.rr"
.policy "expressions" revision 1
.violation Resolved(at: span, expression: symbol, oid: symbol) "{expression} {oid}"
.violation Unreadable(at: span, expression: symbol) "{expression}"
${[...revisions.keys()].map(expr => `git_revision_request(r, "${expr}") :- git_root_repo(r).`).join("\n")}
Resolved(@place("git", 1), e, o) :- git_revision(_, e, o).
Unreadable(@place("git", 1), e) :- git_revision_error(_, e, _).
`);
    const options = { revision: first, analysisScope: "expressions", rr: [policy] };
    const evaluate = async () => (await scan(root, options)).violations.map(finding => finding.message).sort();
    const resolved = [...revisions].map(([expr, oid]) => `${expr} ${oid}`).sort();
    expect(await evaluate()).toEqual(resolved);
    const replaced = (kind: string, revision: string) => {
      const oid = git(root, "rev-parse", revision);
      const old = execFileSync("git", ["cat-file", kind, oid], { cwd: root });
      const body = Buffer.from(old.toString().replace(first, other));
      expect(body.equals(old)).toBe(false);
      const loose = join(objects, oid.slice(0, 2), oid.slice(2));
      if (storage === "loose") {
        const saved = readFileSync(loose);
        chmodSync(loose, 0o644);
        writeFileSync(loose, deflateSync(Buffer.concat([Buffer.from(`${kind} ${body.length}\0`), body])));
        return () => writeFileSync(loose, saved);
      }
      const base = join(objects, "pack", `corrupt-${kind}`);
      const suffix = execFileSync("git", ["-c", "pack.compression=0", "pack-objects", base], {
        cwd: root, input: `${oid}\n`, encoding: "utf8",
      }).trim();
      const pack = `${base}-${suffix}.pack`;
      const row = git(root, "verify-pack", "-v", `${base}-${suffix}.idx`).split("\n")
        .find(line => line.startsWith(oid))!;
      const [, , , size, start] = row.split(/\s+/u).map(Number);
      const saved = readFileSync(pack);
      let payload = start!;
      while ((saved[payload++]! & 128) !== 0) { /* Skip the entry header. */ }
      const compressed = deflateSync(body, { level: 0 });
      expect(payload + compressed.length).toBe(start! + size!);
      const altered = Buffer.from(saved);
      compressed.copy(altered, payload);
      chmodSync(pack, 0o644);
      writeFileSync(pack, altered);
      rmSync(loose);
      return () => writeFileSync(pack, saved);
    };
    const restoreCommit = replaced("commit", "HEAD");
    const restoreTag = replaced("tag", "selected");
    for (const revision of revisions.keys()) {
      expect(() => new source(root, revision, "expressions")).toThrow(/Git object checksum failed/u);
    }
    expect(await evaluate()).toEqual([...revisions.keys()].sort());
    restoreTag();
    restoreCommit();
    expect(await evaluate()).toEqual(resolved);
  }, 30_000);

  it("invalidates a Git tree after a same-size rewrite with restored modification time", async () => {
    const root = checkout(), path = "pkg/inside.py";
    mkdirSync(join(root, "pkg"));
    writeFileSync(join(root, path), "NEEDLE\n");
    git(root, "add", path);
    git(root, "commit", "-qm", "nested finding");
    const tree = git(root, "rev-parse", "HEAD:pkg");
    const file = join(root, ".git", "objects", tree.slice(0, 2), tree.slice(2));
    chmodSync(file, 0o600);
    const time = new Date("2020-01-02T03:04:05Z"), original = readFileSync(file);
    utimesSync(file, time, time);
    const initial = statSync(file, { bigint: true });
    const open = () => new (loadNativeBinding().GitRevisionSource)(root, "HEAD", "mutation");
    const view = open();
    const run = async (name: string) => {
      vi.stubEnv("RERAIL_CONTENT_STORE_PATH", join(root + "-home", name + ".sqlite"));
      try { return await scan(root, { revision: "HEAD", analysisScope: "mutation", paths: [path] }); }
      finally { vi.unstubAllEnvs(); }
    };
    expect(view.readDirectory("pkg")).toHaveLength(1);
    expect(await run("warm")).toMatchObject({ status: "completed", violations: [{ path }] });
    const rewrite = (bytes: Buffer) => { writeFileSync(file, bytes); utimesSync(file, time, time); };
    rewrite(Buffer.alloc(original.length));
    const changed = statSync(file, { bigint: true });
    expect([changed.size, changed.mtimeNs, changed.ino]).toEqual([initial.size, initial.mtimeNs, initial.ino]);
    if (process.platform === "win32") expect(changed.birthtimeNs).toBe(initial.birthtimeNs);
    expect(() => open().readDirectory("pkg")).toThrow();
    expect(() => view.readDirectory("pkg")).toThrow();
    const cold = await run("cold"), warm = await run("warm");
    expect(cold.status).toBe("failed");
    expect(cold.failures.length).toBeGreaterThan(0);
    expect(warm).toMatchObject({ status: "failed", failures: cold.failures, violations: [] });
    rewrite(original);
    expect(view.readDirectory("pkg")).toHaveLength(1);
    expect(await run("warm")).toMatchObject({ status: "completed", violations: [{ path }] });
  }, 30_000);

  it("compares Git message selectors with the installed Git", () => {
    const root = checkout();
    git(root, "commit", "--allow-empty", "-qm", "code 12 echo echo", "-m", "second paragraph");
    git(root, "commit", "--allow-empty", "-qm", "code dd café");
    const Source = loadNativeBinding().GitRevisionSource;
    const patterns = ["^(seed|code)", "^seed$", "[[:digit:]]{2}", "echo.*paragraph", "^second paragraph",
      String.raw`\bcode\s+dd`, String.raw`^code \w+`, String.raw`\$seed`, "[]$]", "[[:alpha:]$]",
      "[[.e.]]", "[[=e=]]", "a{99999}",
      "missing", "café", "(?i)SEED", String.raw`\d+`, String.raw`(echo) \1`, "["];
    for (const pattern of patterns) {
      for (const selector of [`:/${pattern}`, `HEAD^{/${pattern}}`]) {
        const expected = spawnSync("git", ["rev-parse", "--verify", "--quiet", selector], { cwd: root, encoding: "utf8" });
        expect(expected.error).toBeUndefined();
        expect([0, 1, 128]).toContain(expected.status);
        const select = () => new Source(root, selector, "message-patterns").commit;
        if (expected.status === 0) expect(select(), selector).toBe(expected.stdout.trim());
        else expect(select, selector).toThrow();
      }
    }
  });

  it("resolves refs, IDs, reflogs and index entries through the same rule relation", async () => {
    const root = checkout();
    mkdirSync(join(root, "src"));
    writeFileSync(join(root, "src", "nested.py"), "nested = 1\n");
    git(root, "add", "src");
    git(root, "commit", "-qm", "second commit");
    git(root, "tag", "-am", "annotated release", "release");
    git(root, "pack-refs", "--all");
    git(root, "checkout", "--quiet", "--detach", "HEAD");
    const short = git(root, "rev-parse", "--short", "HEAD");
    const invalidExpressions = ["HEAD:./a.py/", `z-g${short}`, `release-1-g${short}-dirty`, `${short}-dirty`];
    const expressions = [
      short, "@", "main", "HEAD@{1}", "@{0}", "@{1}", "HEAD:src/", "HEAD^{/}", "release", "release^{}", ":a.py", ":./a.py",
      "HEAD:./src", ":/seed", "HEAD^{/^seed}", ":/^(seed|missing)", "HEAD^{/!-^second}", `release-1-g${short}`,
    ];
    const policy = join(root, "refs.rr");
    writeFileSync(policy, `
#include "git.rr"
.policy "refs" revision 1
.violation Resolved(at: span, expression: symbol, oid: symbol) "{expression} {oid}"
${[...expressions, ...invalidExpressions].map(expr => `git_revision_request(r, "${expr}") :- git_root_repo(r).`).join("\n")}
Resolved(@place("git", 1), e, o) :- git_revision(_, e, o).
`);
    const report = await scan(root, { rr: [policy] });
    expect(report.status).toBe("completed");
    expect(report.violations.map(finding => finding.message).sort()).toEqual(
      expressions.map(expr => `${expr} ${git(root, "rev-parse", "--verify", expr)}`).sort(),
    );
    for (const [revision, field] of [["HEAD:src/", "tree"], ["HEAD^{/}", "commit"], ["@{0}", "commit"], ["@{1}", "commit"]] as const) {
      const selected = await scan(root, { revision, analysisScope: "expressions" });
      expect(selected.source?.[field]).toBe(git(root, "rev-parse", "--verify", revision));
    }
    const nested = new (loadNativeBinding().GitRevisionSource)(join(root, "src"), "HEAD:./", "expressions");
    expect(nested.readFile("nested.py", 100)?.toString()).toBe("nested = 1\n");
    for (const [revision, field] of [["HEAD:./", "tree"], ["HEAD^{/^seed}", "commit"], [":/^seed", "commit"]] as const) {
      const selected = await scan(join(root, "src"), { revision, analysisScope: "expressions" });
      expect(selected.source?.[field]).toBe(git(join(root, "src"), "rev-parse", "--verify", revision));
    }
    const expected = git(root, "rev-parse", "main");
    writeFileSync(join(root, ".git/HEAD"), "1".repeat(40) + "\n");
    expect(git(root, "rev-parse", "--verify", ":/second")).toBe(expected);
    expect(new (loadNativeBinding().GitRevisionSource)(root, ":/second", "expressions").commit).toBe(expected);
  }, 30_000);

  it("reads physical objects and explicitly addresses replacements in both interfaces", async () => {
    const root = checkout();
    const original = git(root, "rev-parse", "HEAD");
    writeFileSync(join(root, "a.py"), "clean = 1\n");
    git(root, "commit", "-qam", "replacement");
    const replacement = git(root, "rev-parse", "HEAD");
    git(root, "replace", original, replacement);
    const selector = `refs/replace/${original}`;
    const policy = join(root, "physical.rr");
    writeFileSync(policy, `
#include "git.rr"
.policy "physical" revision 1
.violation Message(at: span, expr: symbol, message: symbol) "{expr} {message}"
git_revision_request(r, "${original}") :- git_root_repo(r).
git_revision_request(r, "${selector}") :- git_root_repo(r).
git_object_request(r, o) :- git_revision(r, _, o).
Message(@place("git", 1), expr, msg) :- git_revision(r, expr, o), git_subject(r, o, msg).
`);
    const resolved = await scan(root, { revision: original, analysisScope: "replace", rr: [policy] });
    expect(resolved.violations.map(v => v.message).sort()).toEqual([`${original} seed`, `${selector} replacement`].sort());
    expect((await scan(root, { revision: original, analysisScope: "replace" })).violations.map(v => v.path)).toEqual(["a.py"]);
    expect((await scan(root, { revision: selector, analysisScope: "replace" })).violations).toEqual([]);
  });

  it("finds a commit past an unreadable blob tag and retracts a legacy answer for a damaged commit", async () => {
    const root = checkout();
    const first = git(root, "rev-parse", "HEAD");
    git(root, "commit", "--allow-empty", "-qm", "second");
    const blob = execFileSync("git", ["hash-object", "-w", "--stdin"], { cwd: root, input: "other", encoding: "utf8" }).trim();
    git(root, "tag", "unrelated", blob);
    const damage = (id: string) => {
      const file = join(root, ".git/objects", id.slice(0, 2), id.slice(2));
      const saved = readFileSync(file);
      chmodSync(file, 0o644);
      writeFileSync(file, deflateSync(Buffer.from("blob 1\0x")));
      return () => writeFileSync(file, saved);
    };
    const policy = join(root, "damage.rr");
    writeFileSync(policy, `
#include "git.rr"
.policy "damage" revision 1
.violation Found(at: span) "found"
.violation Commit(at: span) "commit"
.violation Broken(at: span, oid: symbol) "{oid}"
git_revision_request(r, ":/second") :- git_root_repo(r).
git_object_request(r, "${first}") :- git_root_repo(r).
git_object_request(r, "${blob}") :- git_root_repo(r).
Found(@place("git", 1)) :- git_revision(_, ":/second", _).
Commit(@place("git", 1)) :- git_root_repo(_), @git_commit("${first}") = 1.
Broken(@place("git", 1), o) :- git_object_error(_, o, "unreadable").
`);
    const evaluate = async () => (await scan(root, { rr: [policy], revision: "HEAD", analysisScope: "damage" })).violations.map(v => v.message).sort();
    expect(await evaluate()).toEqual(["commit", "found"]);
    const restoreTagTarget = damage(blob);
    expect(await evaluate()).toEqual([blob, "commit", "found"].sort());
    const selected = await scan(root, { revision: ":/second", analysisScope: "damage" });
    expect(selected.source?.commit).toBe(git(root, "rev-parse", "HEAD"));
    const restoreCommit = damage(first);
    expect(await evaluate()).toEqual([blob, first, "found"].sort());
    restoreTagTarget();
    restoreCommit();
    expect(await evaluate()).toEqual(["commit", "found"]);
  });

  it("continues a message search through readable history after a missing parent", () => {
    const root = checkout();
    const missing = "1".repeat(40);
    const tree = git(root, "rev-parse", "HEAD^{tree}");
    const signature = "Test <a@b> 2147483647 +0000";
    const broken = execFileSync("git", ["hash-object", "-w", "-t", "commit", "--stdin"], {
      cwd: root, input: `tree ${tree}\nparent ${missing}\nauthor ${signature}\ncommitter ${signature}\n\nincomplete\n`, encoding: "utf8",
    }).trim();
    git(root, "update-ref", "refs/heads/incomplete", broken);
    const open = (expr: string) => new (loadNativeBinding().GitRevisionSource)(root, expr, "history");
    expect(open(":/seed").commit).toBe(git(root, "rev-parse", "--verify", ":/seed"));
    expect(() => open(":/unmatched")).toThrow(missing);
  });

  it("matches Git on readable history when enumerating another loose ref fails", async () => {
    const root = checkout();
    writeFileSync(join(root, ".git/refs/heads/aaa-broken"), "invalid ref bytes\n");
    const expected = git(root, "rev-parse", "--verify", ":/seed");
    const open = (expression: string) => new (loadNativeBinding().GitRevisionSource)(root, expression, "ref-errors");
    expect(open(":/seed").commit).toBe(expected);
    expect(() => open(":/absent")).toThrow(/ref|reference/iu);
    const policy = join(root, "ref-errors.rr");
    writeFileSync(policy, `#include "git.rr"
.policy "ref-errors" revision 1
.violation Found(at: span, oid: symbol) "{oid}"
git_revision_request(r, ":/seed") :- git_root_repo(r).
Found(@place("git", 1), oid) :- git_revision(_, ":/seed", oid).
`);
    const report = await scan(root, { revision: expected, analysisScope: "ref-errors", rr: [policy] });
    expect(report.status).toBe("completed");
    expect(report.violations.map(v => v.message)).toEqual([expected]);
  });

  it("reports distinct revision error facts for missing names, wrong kinds and corruption", async () => {
    const root = checkout();
    const blob = git(root, "rev-parse", "HEAD:a.py");
    git(root, "tag", "-am", "corrupt policy tag", "broken");
    const tag = git(root, "rev-parse", "broken");
    const object = join(root, ".git", "objects", tag.slice(0, 2), tag.slice(2));
    chmodSync(object, 0o644);
    writeFileSync(object, deflateSync(Buffer.from("blob 4\0oops")));
    const literalTree = (bytes: Buffer) => execFileSync("git", ["hash-object", "--literally", "-w", "-t", "tree", "--stdin"], {
      cwd: root, input: bytes, encoding: "utf8",
    }).trim();
    const malformed = literalTree(Buffer.from("absent entry terminator"));
    const gitlink = literalTree(Buffer.concat([Buffer.from("160000 link\0"), Buffer.from(git(root, "rev-parse", "HEAD^{tree}"), "hex")]));
    const errors = new Map([
      [`${gitlink}:link/a.py`, "wrong-object-kind"],
      [`${malformed}:a.py`, "malformed-object"],
      ["mian", "missing-revision"],
      [`${blob}^{tree}`, "wrong-object-kind"],
      ["broken^{}", "corrupt-object"],
    ]);
    const policy = join(root, "revision-errors.rr");
    writeFileSync(policy, `
#include "git.rr"
.policy "revision-errors" revision 1
.violation Error(at: span, expression: symbol, reason: symbol) "{expression} {reason}"
${[...errors.keys()].map(expr => `git_revision_request(r, "${expr}") :- git_root_repo(r).`).join("\n")}
Error(@place("git", 1), e, k) :- git_revision_error(_, e, k).
`);
    const report = await scan(root, { rr: [policy] });
    expect(report.status).toBe("completed");
    expect(report.violations.map(finding => finding.message).sort()).toEqual(
      [...errors].map(([expr, reason]) => `${expr} ${reason}`).sort(),
    );
    await expect(scan(root, { rulesRef: "broken" })).rejects.toThrow(/--rules-ref broken.*Git object checksum failed/u);
  });

  it("uses Git byte names for selection before reporting names with invalid UTF-8", async () => {
    const root = checkout();
    const withByteName = (tree: string, suffix: string, blob: string): string => {
      const listed = execFileSync("git", ["ls-tree", "-z", tree], { cwd: root });
      const input = Buffer.concat([listed, Buffer.from(`100644 blob ${blob}\t`), Buffer.from([0xff]), Buffer.from(`${suffix}\0`)]);
      return execFileSync("git", ["mktree", "-z"], { cwd: root, input, encoding: "utf8" }).trim();
    };
    const blob = git(root, "rev-parse", "HEAD:a.py");
    const revision = withByteName("HEAD", ".py", blob);
    const options = { revision, analysisScope: "bytes" };
    const selected = await scan(root, { ...options, paths: ["a.py"] });
    expect(selected).toMatchObject({ status: "completed", paths: ["a.py"], failures: [] });
    expect(selected.violations.map(finding => finding.path)).toEqual(["a.py"]);

    const all = await scan(root, options);
    expect(all.status).toBe("failed");
    expect(all.violations.map(finding => finding.path)).toEqual(["a.py"]);
    expect(all.failures).toEqual([expect.objectContaining({ path: "[hex:ff2e7079]", message: "Git path has invalid UTF-8" })]);

    const mktree = (input: string) => execFileSync("git", ["mktree"], { cwd: root, input, encoding: "utf8" }).trim();
    const refWith = (suffix: string) => {
      const rules = withByteName("HEAD:.rerail/rules", suffix, blob);
      const rerail = mktree(`040000 tree ${rules}\trules\n`);
      const tree = mktree(`040000 tree ${rerail}\t.rerail\n100644 blob ${blob}\ta.py\n`);
      return git(root, "commit-tree", tree, "-m", "byte name near the policies");
    };
    const source = { revision: "HEAD", analysisScope: "bytes" };
    const unrelated = await scan(root, { ...source, rulesRef: refWith(".txt") });
    expect(unrelated).toMatchObject({ status: "completed", failures: [] });
    expect(unrelated.violations.map(finding => finding.path)).toEqual(["a.py"]);
    const selectedPolicy = await scan(root, { ...source, rulesRef: refWith(".rr") });
    expect(selectedPolicy.status).toBe("failed");
    expect(selectedPolicy.violations.map(finding => finding.path)).toEqual(["a.py"]);
    expect(selectedPolicy.failures).toEqual([expect.objectContaining({ path: expect.stringMatching(/\.rerail\/rules\/\[hex:ff2e7272\]$/u) })]);
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
      const owners = () => queryContentStore<{ scope: string; snapshot_id: number }>(storePath,
        "SELECT scope, snapshot_id FROM analysis_scope_snapshot UNION SELECT scope, snapshot_id FROM analysis_scope_workspace_snapshot ORDER BY scope");
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
      vi.unstubAllEnvs();
    }
  });

  it("returns collection failures in the report and preserves scan findings", async () => {
    const root = checkout();
    const failure = vi.spyOn(loadNativeBinding(), "collectAnalysisCacheAsync").mockRejectedValueOnce(new Error("collector unavailable"));
    try {
      const report = await scan(root, { revision: "HEAD", analysisScope: "collection" });
      expect(report.status).toBe("completed");
      expect(report.violations.map(row => row.path)).toEqual(["a.py"]);
      expect(report.cacheWarnings).toContain("Analysis cache collection failed: collector unavailable");
    } finally {
      failure.mockRestore();
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
        expect(directoryOwnerCount(storePath)).toBe(0);
      } finally {
        database.$client.close();
      }
    } finally {
      vi.unstubAllEnvs();
    }
  });

  it("rejects malformed numbered stops before creating a store or changing owners", async () => {
    const root = checkout(), databasePath = join(root + "-home", "rerail.sqlite");
    const store = join(root + "-home", "content.sqlite"), absent = store + ".absent";
    vi.stubEnv("RERAIL_CONTENT_STORE_PATH", store);
    try {
      expect((await scan(root, { databasePath })).status).toBe("completed");
      const home = analysisScopeHome(databasePath), binding = loadNativeBinding();
      const owners = () => queryContentStore(store, "SELECT * FROM analysis_scope_workspace_snapshot ORDER BY scope, workspace");
      const before = owners();
      expect(before).toHaveLength(1);
      for (const id of [`invalid:receipt:${home}`, "-1:receipt", "1.5:receipt", "1:", "1:id:", "1:id:extra:field", "9007199254740992:id"]) {
        for (const path of [absent, store]) expect(() => binding.deleteAnalysisScope(path, root, true, id)).toThrow(/invalid directory release receipt/u);
      }
      expect(existsSync(absent)).toBe(false);
      expect(owners()).toEqual(before);
      expect(queryContentStore(store, "SELECT * FROM analysis_scope_release")).toHaveLength(0);
    } finally { vi.unstubAllEnvs(); }
  });

  it("waits for the original store after a swap and releases both identities that used one path", async () => {
    const root = checkout(), home = root + "-home", databasePath = join(home, "home.sqlite");
    const mounted = join(home, "mount"), first = mounted + "-first", second = mounted + "-second";
    const store = join(mounted, "content.sqlite"), binding = loadNativeBinding();
    vi.stubEnv("RERAIL_CONTENT_STORE_PATH", store);
    const unmount = (destination: string) => {
      binding.retireUpkeepUnder?.(mounted);
      renameSync(mounted, destination);
    };
    const stop = () => {
      const db = openRerailDatabase(databasePath);
      try { db.transaction(tx => queueAnalysisScopeReleases(tx, [root], databasePath)); } finally { db.$client.close(); }
    };
    const pending = () => {
      const db = openRerailDatabase(databasePath);
      try { return db.$client.prepare("SELECT count(*) FROM analysis_scope_releases").pluck().get(); }
      finally { db.$client.close(); }
    };
    try {
      expect((await scan(root, { databasePath })).status).toBe("completed");
      expect(directoryOwnerCount(store)).toBe(1);
      unmount(first);
      // A valid replacement with no registration must never acknowledge the first store's stop.
      await binding.prepareAnalysisStoreAsync(store);
      stop();
      expect(() => settleAnalysisScopeReleases(databasePath, -1)).toThrow(/identity changed/u);
      expect(queryContentStore(store, "SELECT id FROM analysis_scope_release")).toEqual([]);
      expect(pending()).toBe(1);
      expect((await scan(root, { databasePath })).status).toBe("completed");
      unmount(second);
      expect(() => settleAnalysisScopeReleases(databasePath, -1)).toThrow();
      expect(existsSync(mounted)).toBe(false);
      renameSync(first, mounted);
      settleAnalysisScopeReleases(databasePath, -1);
      expect(directoryOwnerCount(store)).toBe(0);
      expect(pending()).toBe(0);
      expect((await scan(root, { databasePath })).status).toBe("completed");
      stop(); // Both stores now own this directory in the ended generation.
      expect(() => settleAnalysisScopeReleases(databasePath, -1)).toThrow(/identity changed/u);
      expect(directoryOwnerCount(store)).toBe(0);
      expect(pending()).toBe(1);
      unmount(first);
      renameSync(second, mounted);
      expect(directoryOwnerCount(store)).toBe(1);
      settleAnalysisScopeReleases(databasePath, -1);
      expect(directoryOwnerCount(store)).toBe(0);
      expect(pending()).toBe(0);
      const index = new Database(databasePath + ".analysis-stores.sqlite", { readonly: true });
      try {
        expect(index.prepare("SELECT count(*) FROM analysis_directory_store").pluck().get()).toBe(0);
        expect(index.prepare("SELECT path FROM analysis_maintenance_store").pluck().all()).toEqual([store]);
      } finally { index.close(); }
      vi.unstubAllEnvs();
      const collect = vi.spyOn(binding, "collectAnalysisCacheAsync").mockResolvedValue(undefined);
      try {
        await collectAnalysisCaches(databasePath);
        expect(collect.mock.calls.filter(([path]) => path === store)).toHaveLength(1);
      } finally { collect.mockRestore(); }
    } finally { vi.unstubAllEnvs(); }
  });

  it("keeps a registration acquired between delivery and acknowledgement and skips unrelated routes", async () => {
    const root = checkout(), home = root + "-home", databasePath = join(home, "rerail.sqlite"), store = join(home, "content.sqlite");
    vi.stubEnv("RERAIL_CONTENT_STORE_PATH", store);
    try {
      expect((await scan(root, { databasePath })).status).toBe("completed");
      const registry = new Database(databasePath + ".analysis-stores.sqlite");
      const db = openRerailDatabase(databasePath);
      try {
        registry.prepare(`WITH RECURSIVE n(i) AS (VALUES(1) UNION ALL SELECT i+1 FROM n WHERE i<4096)
          INSERT INTO analysis_directory_store SELECT ? || i, ? || i FROM n`).run(root + "-other/", home + "/other-");
        db.transaction(tx => queueAnalysisScopeReleases(tx, [root], databasePath));
        const binding = loadNativeBinding(), release = binding.deleteAnalysisScope;
        const delivery = vi.spyOn(binding, "deleteAnalysisScope").mockImplementationOnce((...args) => {
          expect(args[0]).toBe(store);
          release(...args);
          execFileSync(process.execPath, ["--import", "tsx", "--input-type=module", "-", databasePath, root, store], {
            input: 'import {acquireDirectoryAnalysisGeneration} from "./src/engine/native/analysis-scope.ts"; const [db,dir,store]=process.argv.slice(2); await acquireDirectoryAnalysisGeneration(db,dir,store);',
          });
        });
        try {
          settleAnalysisScopeReleases(databasePath);
          expect(delivery).toHaveBeenCalledTimes(1);
        } finally { delivery.mockRestore(); }
        const registrations = () => registry.prepare("SELECT count(*) FROM analysis_directory_store").pluck().get();
        expect(registrations()).toBe(4097);
        db.transaction(tx => queueAnalysisScopeReleases(tx, [root], databasePath));
        settleAnalysisScopeReleases(databasePath);
        expect(registrations()).toBe(4096);
      } finally { db.$client.close(); registry.close(); }
    } finally { vi.unstubAllEnvs(); }
  });

  it("requires safe integer generations before acquiring a run or attaching its workspace", async () => {
    const root = checkout(), engine = loadNativeBinding(), store = join(root + "-home", "generation.sqlite");
    const workspace = new engine.CodeRuleDslWorkspace();
    for (const value of [NaN, Infinity, -1, 1.9, Number.MAX_SAFE_INTEGER + 1]) {
      expect(() => new engine.AnalysisScopeRun(store, root, true, "run", value)).toThrow(/nonnegative safe integer/u);
      await expect(async () => workspace.enableStorageAsync(root, store, { files: [], folders: [] }, undefined, value)).rejects.toThrow(/nonnegative safe integer/u);
    }
    expect(existsSync(store)).toBe(false);
    for (const value of [0, Number.MAX_SAFE_INTEGER]) {
      new engine.AnalysisScopeRun(store, root, true, "run", value).finish(false);
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
        expect(directoryOwnerCount(storePath)).toBe(1);
      } finally { database.$client.close(); }
    } finally { vi.unstubAllEnvs(); }
  });

  it("publishes after a control reset or rollback and honors a stop before the next scan", async () => {
    const root = checkout(), home = root + "-home";
    const databasePath = join(home, "rerail.sqlite"), store = join(home, "content.sqlite");
    vi.stubEnv("RERAIL_CONTENT_STORE_PATH", store);
    const stop = () => {
      const db = openRerailDatabase(databasePath);
      try { db.transaction(tx => queueAnalysisScopeReleases(tx, [root], databasePath)); }
      finally { db.$client.close(); }
      settleAnalysisScopeReleases(databasePath);
    };
    const run = async () => {
      const report = await scan(root, { databasePath });
      expect(report.status).toBe("completed");
      expect(report.cacheWarnings).toBeUndefined();
      expect(directoryOwnerCount(store)).toBe(1);
    };
    try {
      await run();
      const db = openRerailDatabase(databasePath);
      try { await db.$client.backup(databasePath + ".backup"); }
      finally { db.$client.close(); }
      const aborted = openRerailDatabase(databasePath);
      try {
        expect(() => aborted.transaction(tx => {
          queueAnalysisScopeReleases(tx, [root], databasePath);
          throw new Error("abort stop");
        })).toThrow("abort stop");
      } finally { aborted.$client.close(); }
      settleAnalysisScopeReleases(databasePath);
      expect(directoryOwnerCount(store)).toBe(1);
      stop();
      expect(directoryOwnerCount(store)).toBe(0);
      rmSync(databasePath);
      // Model an older registry. The ancestor receipt in the store must protect a child scan.
      const registry = new Database(databasePath + ".analysis-stores.sqlite");
      try { registry.exec("DROP TABLE analysis_registry_generation"); } finally { registry.close(); }
      const child = join(root, "sub");
      mkdirSync(child);
      const generation = await acquireDirectoryAnalysisGeneration(databasePath, child, store);
      const receipts = queryContentStore<{ before: number }>(store, "SELECT before FROM analysis_scope_release");
      expect(receipts.length).toBeGreaterThan(0);
      for (const { before } of receipts) expect(generation).toBeGreaterThan(before);
      await run();
      renameSync(databasePath + ".backup", databasePath);
      stop();
      expect(directoryOwnerCount(store)).toBe(0);
      expect(readAnalysisScopeGeneration(databasePath)).toBeGreaterThan(generation);
      await run();
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
      const store = scanContentStore(databasePath);
      expect(queryContentStore(store, "SELECT scope FROM analysis_scope_manifest WHERE scope = 'named:atomic' OR scope GLOB 'pending:*'")).toEqual([]);
      expect(queryContentStore(store, "SELECT scope FROM analysis_scope_snapshot WHERE scope = 'named:atomic'")).toEqual([]);
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
      if (store === ":memory:") return attach.call(this, source, store, secrets, analysis);
      expect(analysis).toBeDefined();
      // The injected store fails to open. The real run's owner belongs to the original store.
      return attach.call(this, source, broken, secrets);
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

  it("reports a source resolution error even if cache cleanup also fails", async () => {
    const root = checkout();
    const prototype = loadNativeBinding().AnalysisScopeRun.prototype;
    const finish = vi.spyOn(prototype, "finish").mockImplementationOnce(() => {
      throw new Error("cache cleanup failed");
    });
    try {
      await expect(scan(root, { revision: "missing-source-revision", analysisScope: "error" }))
        .rejects.toThrow("missing-source-revision");
      expect(finish).toHaveBeenCalledWith(false);
    } finally { finish.mockRestore(); }
  });

  it("retries relational evaluation in memory when its selected cache fails", async () => {
    const root = checkout();
    const home = `${root}-home`;
    const cache = join(home, "cache");
    const store = join(cache, "content.sqlite");
    const policy = join(home, "committed.rr");
    mkdirSync(cache, { recursive: true });
    vi.stubEnv("RERAIL_CONTENT_STORE_PATH", store);
    writeFileSync(policy, `#include "core.rr"\n.policy "committed" revision 1\n.language python\n.violation Seen(at: node) "needle"\nSeen(n) :- kind(n, "identifier"), text(n, "NEEDLE").\n`);
    writeFileSync(join(root, "a.py"), "# changed after the commit\n");
    const binding = loadNativeBinding();
    const run = binding.scanRrPoliciesAsync;
    const failure = vi.spyOn(binding, "scanRrPoliciesAsync").mockImplementation(async (...args) => {
      expect(args[2]).toBe(store);
      renameSync(cache, cache + ".saved");
      try {
        mkdirSync(cache);
        writeFileSync(store, "this is not a database");
        return await run(...args);
      } finally {
        rmSync(cache, { recursive: true, force: true });
        renameSync(cache + ".saved", cache);
      }
    });
    try {
      const report = await scan(root, { revision: "HEAD", analysisScope: "relational", rr: [policy] });
      expect(report.status, JSON.stringify(report.failures)).toBe("completed");
      expect(report.violations.map(finding => finding.path)).toEqual(["a.py"]);
      expect(report.cacheWarnings).toEqual([expect.stringMatching(/file is not a database/u)]);
    } finally { failure.mockRestore(); vi.unstubAllEnvs(); }
  });

  it("retries unreadable stored results against the same source and policy", async () => {
    const root = checkout();
    const source = git(root, "rev-parse", "HEAD");
    const policy = join(root, ".rerail/rules/metadata.rr");
    const include = join(root, ".rerail/rules/lib/labels.rr");
    mkdirSync(join(root, ".rerail/rules/lib"));
    writeFileSync(include, '.decl marker(text: symbol)\nmarker("original include").\n');
    writeFileSync(policy, `#include "git.rr"
#include "lib/labels.rr"
.policy "metadata" revision 1
.violation Seen(at: span, text: symbol) "original policy {text}"
Seen(@place("commit", 1), text) :- git_source_commit(_, _), marker(text).
`);
    const prototype = loadNativeBinding().CodeRuleDslWorkspace.prototype;
    const read = vi.spyOn(prototype, "resultPageAsync").mockImplementationOnce(async () => {
      writeFileSync(join(root, "a.py"), "# no finding in the new commit\n");
      writeRule(root, "needle.json", needle("new-rule", "OTHER"));
      writeFileSync(policy, readFileSync(policy, "utf8").replace("original policy", "new policy"));
      writeFileSync(include, '.decl marker(text: symbol)\nmarker("new include").\n');
      git(root, "add", "a.py");
      git(root, "commit", "-qm", "move HEAD");
      throw new Error("WORKSPACE_STORE_UNAVAILABLE\ncache read failed");
    });
    try {
      const report = await scan(root, { revision: "HEAD", analysisScope: "reused-results" });
      expect(report.status, JSON.stringify(report.failures)).toBe("completed");
      expect(report.source?.commit).toBe(source);
      expect(report.violations).toEqual(expect.arrayContaining([
        expect.objectContaining({ ruleId: "needle" }),
        expect.objectContaining({ ruleId: "metadata", message: "original policy original include" }),
      ]));
      expect(report.violations).toHaveLength(2);
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
      expect(queryContentStore(storePath, "SELECT scope FROM analysis_scope_workspace_snapshot"))
        .toEqual([{ scope: `cwd:${analysisScopeHome(second)}:${root}` }]);
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
    try {
      for (const store of [earlier, current]) {
        vi.stubEnv("RERAIL_CONTENT_STORE_PATH", store);
        expect((await scan(root, { databasePath })).status).toBe("completed");
        expect(directoryOwnerCount(store)).toBe(1);
      }
      vi.stubEnv("RERAIL_CONTENT_STORE_PATH", unrelated);
      expect((await scan(other, { databasePath })).status).toBe("completed");
      expect(directoryOwnerCount(unrelated)).toBe(1);
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
          expect(directoryOwnerCount(earlier)).toBe(1);
          expect(directoryOwnerCount(current)).toBe(0);
          expect(fail.mock.calls.every(([store]) => store !== unrelated)).toBe(true);
        } finally { fail.mockRestore(); }
        settleAnalysisScopeReleases(databasePath, -1);
        expect(pending()).toBe(0);
        expect(directoryOwnerCount(earlier)).toBe(0);
        expect(directoryOwnerCount(unrelated)).toBe(1);
      } finally { homeDb.$client.close(); }
    } finally { vi.unstubAllEnvs(); }
  });

  it.each(["missing", "corrupt"])("leaves a stop clear of a %s store that never opened", async kind => {
    const root = checkout();
    const home = `${root}-home`;
    mkdirSync(home);
    const file = join(home, "file");
    writeFileSync(file, "this is not a database");
    const databasePath = join(home, "rerail.sqlite");
    vi.stubEnv("RERAIL_CONTENT_STORE_PATH", kind === "missing" ? join(file, "unusable.sqlite") : file);
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

  it("applies a second language registry to a reused Git source", async () => {
    const root = checkout(), home = root + "-home";
    mkdirSync(home);
    const engine = loadNativeBinding(), policy = join(home, "languages.rr");
    writeFileSync(policy, '#include "source-language.rr"\n.policy "languages" revision 1\n.violation Language(at: span, l: symbol) "{l}"\nsource_language_request("a.py").\nLanguage(@place("a.py", 1), l) :- source_language("a.py", l).\n');
    const open = () => new engine.GitRevisionSource(root, "HEAD", "languages");
    let seq = 0;
    const read = async (source: ReturnType<typeof open>, language: string) => {
      source.setSourceLanguages(JSON.stringify({ names: {}, suffixes: { ".py": language } }));
      const [result] = JSON.parse(await engine.scanRrPoliciesAsync([policy], root, join(home, "lang.sqlite"), join(home, String(++seq)), undefined, undefined, source));
      return result.diagnostics.map((row: { message: string }) => row.message);
    };
    const view = open();
    expect(await read(view, "python")).toEqual(["python"]);
    expect(await read(view, "rust")).toEqual(["rust"]);
    expect(await read(open(), "rust")).toEqual(["rust"]);
  });

  it("publishes each policy under the current owner when two runs share one source view", async () => {
    const root = checkout(), home = root + "-home";
    mkdirSync(home);
    const engine = loadNativeBinding();
    const source = new engine.GitRevisionSource(root, "HEAD", "reuse");
    const store = join(home, "shared.sqlite");
    const manifests = () => queryContentStore<{ scope: string; manifest_id: number }>(store,
      "SELECT scope, manifest_id FROM analysis_scope_manifest ORDER BY manifest_id");
    let prior: number[] = [];
    for (const name of ["first", "second"]) {
      const policy = join(home, name + ".rr");
      writeFileSync(policy, `#include "git.rr"\n.policy "${name}" revision 1\n.violation Seen(at: span) "${name}"\nSeen(@place("git",1)) :- git_source_tree(_, _).\n`);
      const run = new engine.AnalysisScopeRun(store, "reuse", false, name, 0);
      expect(() => engine.scanRrPoliciesAsync([policy], root, join(home, "other.sqlite"), "", run, undefined, source))
        .toThrow(/another content store/u);
      const [report] = JSON.parse(await engine.scanRrPoliciesAsync([policy], root, store, join(home, name), run, undefined, source));
      expect(report.diagnostics.map((row: { message: string }) => row.message)).toEqual([name]);
      expect(run.finish(true)).toBeNull();
      expect(() => engine.scanRrPoliciesAsync([policy], root, store, "", run, undefined, source))
        .toThrow(/has finished/u);
      const rows = manifests();
      expect(rows.map(row => row.scope)).toEqual(rows.map(() => "named:reuse"));
      const ids = rows.map(row => row.manifest_id);
      expect(ids).toEqual(expect.arrayContaining(prior));
      expect(ids.length).toBeGreaterThan(prior.length);
      prior = ids;
    }
  }, 30_000);

  it("selects an inspected Git policy by repository and revision in either catalog load order", async () => {
    const root = checkout(), other = checkout(), home = root + "-home";
    mkdirSync(home);
    const engine = loadNativeBinding(), path = "policies/which.rr", store = join(home, "versions.sqlite");
    mkdirSync(join(root, "policies"));
    const commitPolicy = (word: string) => {
      writeFileSync(join(root, path), `#include "git.rr"\n.policy "which" revision 1\n.violation Seen(at: span) "${word}"\nSeen(@place("git",1)) :- git_source_commit(_, _).\n`);
      git(root, "add", path);
      git(root, "commit", "-qm", word);
      return git(root, "rev-parse", "HEAD");
    };
    const first = commitPolicy("one"), second = commitPolicy("two");
    git(other, "fetch", "-q", root, "main");
    const open = (id: string, repo = root) => new engine.GitRevisionSource(repo, id, "versions");
    const source = open(first);
    for (const order of [[first, second], [second, first]]) {
      const catalog = new engine.LoadedRrPolicies(store);
      const run = async (policySource?: typeof source, spelling = path, loaded = true) => JSON.parse(await engine.scanRrPoliciesAsync(
        [spelling], root, store, "", undefined, undefined, source, policySource, loaded ? catalog : undefined,
      ))[0];
      catalog.load(path, open(order[0]!));
      expect((await run()).diagnostics[0].message).toBe(order[0] === first ? "one" : "two");
      catalog.load("policies//which.rr", open(order[1]!));
      for (const spelling of [path, "./policies//which.rr", "policies/./which.rr"]) {
        expect((await run(open(first), spelling)).diagnostics[0].message).toBe("one");
        expect((await run(open(second), spelling)).diagnostics[0].message).toBe("two");
      }
      for (const loaded of [true, false]) {
        expect((await run(open(first), path + "/.", loaded)).error).toContain("Expected a policy file");
        expect((await run(open(first), "../" + path, loaded)).error).toBeDefined();
      }
      expect((await run(open(first, other))).error).toMatch(/not loaded from the selected Git source/u);
      expect((await run()).error).toMatch(/select a policy source/u);
    }
  }, 30_000);

  it("runs the inspected policy after an include changes and refreshes it on the next inspection", async () => {
    const root = checkout(), home = root + "-home";
    mkdirSync(home);
    const engine = loadNativeBinding(), policy = join(home, "files.rr"), include = join(home, "word.rr");
    const catalog = new engine.LoadedRrPolicies(join(home, "inspection.sqlite"));
    const store = join(home, "execution.sqlite");
    const view = new engine.GitRevisionSource(root, "HEAD", "policy-edits");
    const word = (value: string) => `.decl ref_message(text: symbol)\nref_message("${value}").\n`;
    writeFileSync(policy, '#include "git.rr"\n#include "word.rr"\n.policy "words" revision 1\n.violation Seen(at: span, w: symbol) "{w}"\nSeen(@place("a.py", 1), w) :- git_source_commit(_, _), ref_message(w).\n');
    writeFileSync(include, word("one"));
    const inspect = () => JSON.parse(catalog.load(policy)).programInputs.find((input: { path: string }) => input.path === include)?.sha256;
    const run = async (pinned: boolean) => {
      const [report] = JSON.parse(await engine.scanRrPoliciesAsync([policy], root, store, "", undefined, undefined, view, undefined, pinned ? catalog : undefined));
      return report.diagnostics.map((row: { message: string }) => row.message);
    };
    expect(inspect()).toBe(createHash("sha256").update(word("one")).digest("hex"));
    writeFileSync(include, word("two"));
    expect(await run(true)).toEqual(["one"]);
    expect(await run(false)).toEqual(["two"]);
    expect(inspect()).toBe(createHash("sha256").update(word("two")).digest("hex"));
    expect(await run(true)).toEqual(["two"]);
  });

  it("retains the inventory plans of a reused view after scope deletion and across stores", async () => {
    const root = checkout(), home = root + "-home";
    mkdirSync(home);
    const engine = loadNativeBinding(), policy = join(home, "files.rr");
    writeFileSync(policy, `#include "core.rr"
.policy "files" revision 1
.language python
.violation Seen(at: node) "identifier"
Seen(n) :- kind(n, "identifier").
`);
    const open = () => new engine.GitRevisionSource(root, "HEAD", "repeat");
    const view = open(), first = join(home, "first.sqlite"), other = join(home, "other.sqlite");
    const retained = (store: string, scope = "repeat") => queryContentStore<{ policy: string; plan: string }>(store,
      `SELECT DISTINCT hex(m.policy) AS policy, hex(p.plan_digest) AS plan FROM analysis_scope_manifest a
       JOIN corpus_manifests m USING(manifest_id) JOIN corpus_manifest_plans p USING(manifest_id)
       WHERE a.scope=? ORDER BY policy,plan`, ["named:" + scope]);
    const priming = join(home, "priming.sqlite");
    const inspected = new engine.LoadedRrPolicies(priming);
    inspected.load(policy);
    const evaluate = async (source: typeof view, store: string, publish: boolean, file = policy, scope = "repeat") => {
      const run = new engine.AnalysisScopeRun(store, scope, false, "test", 0);
      const loaded = store === first && file === policy ? inspected : undefined;
      const [report] = JSON.parse(await engine.scanRrPoliciesAsync([file], root, store, "", run, undefined, source, undefined, loaded));
      expect(report.diagnostics.length).toBeGreaterThan(0);
      expect(run.finish(publish)).toBeNull();
    };
    // A daemon can have already inspected these libraries for a different store.
    await evaluate(open(), priming, true);
    await evaluate(view, first, true);
    const baseline = retained(first);
    expect(new Set(baseline.map(row => row.policy)).size).toBeGreaterThan(2);
    engine.deleteAnalysisScope(first, "repeat", false);
    await evaluate(view, first, false);
    expect(retained(first)).toEqual([]);
    await evaluate(view, first, true);
    expect(retained(first)).toEqual(baseline);
    await evaluate(view, other, true);
    const otherPlans = retained(other);
    expect(new Set(otherPlans.map(row => row.policy))).toEqual(new Set(baseline.map(row => row.policy)));
    engine.deleteAnalysisScope(other, "repeat", false);
    await evaluate(open(), other, true);
    expect(retained(other)).toEqual(otherPlans);
    const control = join(home, "control.rr");
    writeFileSync(control, readFileSync(policy, "utf8").replace('policy "files"', 'policy "control"').replace('kind(n, "identifier")', 'kind(n, "integer")'));
    await evaluate(open(), first, true, control, "release-only");
    const survivingPlans = new Set(baseline.map(row => row.plan));
    const controlPlans = retained(first, "release-only").map(row => row.plan);
    const exclusive = controlPlans.filter(plan => !survivingPlans.has(plan));
    const shared = controlPlans.filter(plan => survivingPlans.has(plan));
    const memoRows = (plans: string[]) => queryContentStore<{ count: number }>(first,
      "SELECT count(*) AS count FROM facts_results WHERE hex(plan_digest) IN (SELECT value FROM json_each(?))",
      [JSON.stringify(plans)])[0]!.count;
    expect(memoRows(exclusive)).toBeGreaterThan(0);
    expect(memoRows(shared)).toBeGreaterThan(0);
    engine.deleteAnalysisScope(first, "release-only", false);
    expect(memoRows(exclusive)).toBeGreaterThan(0);
    await engine.collectAnalysisCacheAsync(first);
    expect(memoRows(exclusive)).toBe(0);
    expect(memoRows(shared)).toBeGreaterThan(0);
    expect(retained(first)).toEqual(baseline);
    const profile = join(home, "reopened");
    const count = execFileSync(process.execPath, ["--import", "tsx", "--input-type=module", "-", root, policy, first], {
      encoding: "utf8", env: { ...process.env, RERAIL_PROFILE: profile },
      input: `import { loadNativeBinding } from "./src/engine/native/native.ts";
        const [repo, policy, store] = process.argv.slice(2), engine = loadNativeBinding();
        const view = new engine.GitRevisionSource(repo, "HEAD", "repeat");
        const run = new engine.AnalysisScopeRun(store, "repeat", false, "reopened", 0);
        const [report] = JSON.parse(await engine.scanRrPoliciesAsync([policy], repo, store, "", run, undefined, view));
        if (run.finish(true) != null) throw Error("cache publication failed");
        console.log(report.diagnostics.length);`,
    });
    expect(Number(count)).toBeGreaterThan(0);
    const repeated = readFileSync(join(profile, "profile.txt"), "utf8").split("\n")
      .filter(line => /^\s*count\s+[1-9]\d*\s+(git\.tree_decodes$|source_tape\.(?:parses$|input\.))/u.test(line));
    expect(repeated).toEqual([]);
  }, 60_000);

  it.skipIf(process.platform === "win32" || process.getuid?.() === 0)("retries an unreadable store with real directory ownership", async () => {
    const root = checkout(), home = root + "-home";
    const databasePath = join(home, "rerail.sqlite"), store = join(home, "legacy.sqlite");
    vi.stubEnv("RERAIL_CONTENT_STORE_PATH", store);
    try {
      expect((await scan(root, { databasePath })).status).toBe("completed");
      const script = `import { loadNativeBinding } from "./src/engine/native/native.ts";
        try { console.log(loadNativeBinding().existingDirectoryAnalysisOwnership(...process.argv.slice(2))); }
        catch { console.log("unavailable"); }`;
      const read = () => execFileSync(process.execPath, ["--import", "tsx", "--input-type=module", "-", store, root, analysisScopeHome(databasePath)], { input: script, encoding: "utf8" }).trim();
      expect(read()).toBe("true");
      chmodSync(store, 0);
      try { expect(read()).toBe("unavailable"); }
      finally { chmodSync(store, 0o600); }
      expect(read()).toBe("true");
      loadNativeBinding().retireUpkeepUnder?.(root + "-home");
      renameSync(store, store + ".saved");
      try {
        writeFileSync(store, "temporarily damaged SQLite header");
        expect(read()).toBe("unavailable");
      } finally { renameSync(store + ".saved", store); }
      expect(read()).toBe("true");
      const directory = dirname(store);
      const mode = statSync(directory).mode;
      chmodSync(directory, 0);
      try {
        const binding = loadNativeBinding();
        expect(() => binding.deleteAnalysisScope(store, "unrelated", false)).toThrow(/permission denied/iu);
        await expect(binding.collectAnalysisCacheAsync(store)).rejects.toThrow(/permission denied/iu);
      } finally { chmodSync(directory, mode); }
    } finally { vi.unstubAllEnvs(); }
  });

  it("releases matching ownership from a store that predates directory registration", async () => {
    const root = checkout();
    const home = `${root}-home`;
    const databasePath = join(home, "rerail.sqlite");
    const storePath = join(home, "legacy.sqlite");
    vi.stubEnv("RERAIL_CONTENT_STORE_PATH", storePath);
    try {
      expect((await scan(root, { databasePath })).status).toBe("completed");
      const registry = new Database(`${databasePath}.analysis-stores.sqlite`);
      try { registry.prepare("UPDATE analysis_directory_store SET directory=?").run(join(root, "..", "another-checkout")); }
      finally { registry.close(); }
      const db = openRerailDatabase(databasePath);
      try {
        expect(directoryOwnerCount(storePath)).toBe(1);
        setCheckoutStarted(db, recordCheckout(db, root), false);
        settleAnalysisScopeReleases(databasePath);
        expect(directoryOwnerCount(storePath)).toBe(0);
        expect(db.$client.prepare("SELECT count(*) FROM analysis_scope_releases").pluck().get()).toBe(0);
      } finally {
        db.$client.close();
      }
    } finally { vi.unstubAllEnvs(); }
  });

  it("ignores an inaccessible legacy candidate and keeps a registered store pending", async () => {
    const root = checkout();
    const home = `${root}-home`;
    const main = join(home, "main");
    mkdirSync(main, { recursive: true });
    const unregistered = join(main, "unregistered.sqlite");
    const registered = join(main, "registered.sqlite");
    const databasePath = join(home, "rerail.sqlite");
    writeFileSync(unregistered, "invalid database");
    const registeredRoot = join(root, "registered");
    await acquireDirectoryAnalysisGeneration(databasePath, registeredRoot, registered);
    const machineStore = vi.spyOn(builds, "mainContentStoreDirectory").mockReturnValue(main);
    vi.stubEnv("RERAIL_CONTENT_STORE_PATH", unregistered);
    const db = openRerailDatabase(databasePath);
    try {
      const pending = () => db.$client.prepare("SELECT directory FROM analysis_scope_releases").pluck().all();
      const unrelated = join(root, "unrelated");
      db.transaction(tx => queueAnalysisScopeReleases(tx, [unrelated], databasePath));
      settleAnalysisScopeReleases(databasePath);
      expect(pending()).toEqual([]);
      db.transaction(tx => queueAnalysisScopeReleases(tx, [registeredRoot], databasePath));
      expect(() => settleAnalysisScopeReleases(databasePath)).toThrow(/registered\.sqlite belongs to the main Rerail daemon/u);
      expect(pending()).toEqual([registeredRoot]);
    } finally {
      db.$client.close();
      vi.unstubAllEnvs();
      machineStore.mockRestore();
    }
  });

  it("does not wait for a legacy store when its ownership query fails", async () => {
    const root = checkout();
    const home = `${root}-home`;
    const databasePath = join(home, "rerail.sqlite");
    const legacy = join(home, "legacy.sqlite");
    mkdirSync(home);
    writeFileSync(legacy, "");
    vi.stubEnv("RERAIL_CONTENT_STORE_PATH", legacy);
    const binding = loadNativeBinding();
    const lookup = binding.owningDirectoryAnalysisStore;
    const probe = vi.spyOn(binding, "owningDirectoryAnalysisStore").mockImplementation((store, ...args) => {
      if (store === legacy) throw new Error("disk I/O error during ownership check");
      return lookup(store, ...args);
    });
    const database = openRerailDatabase(databasePath);
    try {
      setCheckoutStarted(database, recordCheckout(database, root), false);
      settleAnalysisScopeReleases(databasePath);
      expect(probe).toHaveBeenCalledWith(legacy, root, analysisScopeHome(databasePath));
      expect(database.$client.prepare("SELECT count(*) FROM analysis_scope_releases").pluck().get()).toBe(0);
    } finally {
      database.$client.close();
      probe.mockRestore();
      vi.unstubAllEnvs();
    }
  });

  it("releases legacy ownership after a failed probe and preserves a new generation", async () => {
    const root = checkout();
    const databasePath = join(`${root}-home`, "rerail.sqlite");
    const legacy = join(`${root}-home`, "legacy.sqlite");
    vi.stubEnv("RERAIL_CONTENT_STORE_PATH", legacy);
    const binding = loadNativeBinding();
    const lookup = binding.owningDirectoryAnalysisStore;
    try {
      expect((await scan(root, { databasePath })).status).toBe("completed");
      expect(directoryOwnerCount(legacy)).toBe(1);
      const registry = new Database(`${databasePath}.analysis-stores.sqlite`);
      try {
        registry.exec("CREATE TABLE analysis_store (path TEXT PRIMARY KEY); DELETE FROM analysis_directory_store");
        registry.prepare("INSERT INTO analysis_store VALUES (?)").run(legacy);
      } finally { registry.close(); }
      const database = openRerailDatabase(databasePath);
      try {
        database.transaction(tx => queueAnalysisScopeReleases(tx, [root], databasePath));
        const probe = vi.spyOn(binding, "owningDirectoryAnalysisStore").mockImplementation((store, ...args) => {
          if (store === legacy) throw new Error("temporary I/O error during ownership check");
          return lookup(store, ...args);
        });
        try {
          settleAnalysisScopeReleases(databasePath);
          expect(database.$client.prepare("SELECT count(*) FROM analysis_scope_releases").pluck().get()).toBe(0);
          expect(database.$client.prepare("SELECT store FROM analysis_scope_release_probes").pluck().all()).toEqual([legacy]);
          expect(directoryOwnerCount(legacy)).toBe(1);
        } finally { probe.mockRestore(); }

        const subdirectory = join(root, "src");
        mkdirSync(subdirectory);
        expect((await scan(subdirectory, { databasePath })).status).toBe("completed");
        expect(directoryOwnerCount(legacy)).toBe(2);
        vi.unstubAllEnvs();
        settleAnalysisScopeReleases(databasePath, -1);
        expect(database.$client.prepare("SELECT count(*) FROM analysis_scope_release_probes").pluck().get()).toBe(0);
        expect(directoryOwnerCount(legacy)).toBe(1);
      } finally { database.$client.close(); }
    } finally { vi.unstubAllEnvs(); }
  });

  it("releases an unregistered store after its directory comes back online", async () => {
    const root = checkout();
    const databasePath = join(`${root}-home`, "rerail.sqlite");
    const mount = join(`${root}-home`, "mount");
    const offline = `${mount}-offline`;
    mkdirSync(mount, { recursive: true });
    const store = join(mount, "legacy.sqlite");
    vi.stubEnv("RERAIL_CONTENT_STORE_PATH", store);
    try {
      expect((await scan(root, { databasePath })).status).toBe("completed");
      expect(directoryOwnerCount(store)).toBe(1);
      const registry = new Database(`${databasePath}.analysis-stores.sqlite`);
      try { registry.exec("DELETE FROM analysis_directory_store"); }
      finally { registry.close(); }
      loadNativeBinding().retireUpkeepUnder?.(mount);
      renameSync(mount, offline);
      const db = openRerailDatabase(databasePath);
      try {
        db.transaction(tx => queueAnalysisScopeReleases(tx, [root], databasePath));
        settleAnalysisScopeReleases(databasePath);
        expect(db.$client.prepare("SELECT store FROM analysis_scope_release_probes").pluck().all()).toEqual([store]);
        renameSync(offline, mount);
        vi.unstubAllEnvs();
        settleAnalysisScopeReleases(databasePath, -1);
        expect(directoryOwnerCount(store)).toBe(0);
        expect(db.$client.prepare("SELECT count(*) FROM analysis_scope_release_probes").pluck().get()).toBe(0);
      } finally { db.$client.close(); }
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
        expect(directoryOwnerCount(available)).toBe(0);
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

  it("isolates concurrent evaluations of the tracking predicate", async () => {
    const first = checkout(), second = checkout();
    writeFileSync(join(second, "marked"), "staged\n");
    git(second, "add", "marked");
    const policy = join(first, "track.rr");
    writeFileSync(policy, '#include "git.rr"\n.policy "tracked" revision 1\n.violation Value(at: span, v: number) "{v}"\nValue(@place("git",1), v) :- git_root_repo(_), v = @git_tracked("marked").\n');
    const engine = loadNativeBinding();
    const evaluate = async (root: string, id: number) => {
      const [report] = JSON.parse(await engine.scanRrPoliciesAsync([policy], root, ":memory:", join(first, "out", String(id)), undefined, undefined, undefined));
      return report.diagnostics.map((row: { message: string }) => row.message);
    };
    for (let batch = 0; batch < 3; batch++) {
      const answers = await Promise.all([first, second, first, second].map((root, i) => evaluate(root, batch * 4 + i)));
      expect(answers).toEqual([["0"], ["1"], ["0"], ["1"]]);
    }
  }, 60_000);

  it.skipIf(process.platform === "win32")("revalidates split-index tracking and preserves literal backslashes", async () => {
    const root = checkout();
    const name = "folder\\file.py";
    writeFileSync(join(root, name), "staged\n");
    git(root, "add", "--", name);
    git(root, "update-index", "--split-index");
    const shared = join(root, git(root, "rev-parse", "--shared-index-path"));
    const home = root + "-home";
    mkdirSync(home);
    const policy = join(home, "tracked.rr");
    writeFileSync(policy, `#include "git.rr"
.policy "tracked" revision 1
.violation Value(at: span, v: number) "{v}"
Value(@place("git",1), v) :- git_root_repo(_), v = @git_tracked(${JSON.stringify(name)}).
`);
    const engine = loadNativeBinding();
    let sequence = 0;
    const evaluate = async (store: string) => {
      const source = new engine.GitRevisionSource(root, "HEAD", "tracked");
      const [report] = JSON.parse(await engine.scanRrPoliciesAsync([policy], root, store, join(home, String(++sequence)), undefined, undefined, source));
      return report.diagnostics.map((row: { message: string }) => row.message);
    };
    const store = join(home, "tracked.sqlite");
    expect(await evaluate(store)).toEqual(["1"]);
    renameSync(shared, shared + ".away");
    expect(await evaluate(":memory:")).toEqual([]);
    expect(await evaluate(store)).toEqual([]);
    renameSync(shared + ".away", shared);
    expect(await evaluate(store)).toEqual(["1"]);
  }, 60_000);

  it("keeps findings from healthy policies in either order when a references setting fails", async () => {
    const root = checkout();
    const valid = join(root, "valid.rr"), invalid = join(root, "invalid.rr");
    writeFileSync(valid, '#include "git.rr"\n.policy "good" revision 1\n.violation Seen(at: span) "found"\nSeen(@place("git", 1)) :- git_source_commit(_, _).\n');
    writeFileSync(invalid, '#include "core.rr"\n.policy "bad" revision 1\n.language python\n.references {"unknownParameter":true}\n.violation V(at: node) "edge"\nV(n) :- reference_edge(n,_,_).\n');
    for (const rr of [[valid, invalid], [invalid, valid]]) {
      const report = await scan(root, { revision: "HEAD", analysisScope: "partial-policies", rr });
      expect(report.status).toBe("failed");
      expect(report.violations.map(row => row.message)).toEqual(["found"]);
      expect(report.failures).toEqual([expect.objectContaining({ path: invalid, ruleId: "bad", message: expect.stringContaining("unknownParameter") })]);
    }
  }, 30_000);

  it("rejects duplicate explicit policy IDs and leaves no named ownership", async () => {
    const root = checkout();
    const databasePath = join(`${root}-home`, "rerail.sqlite");
    const policy = (message: string) => `#include "git.rr"\n.policy "same" revision 1\n.violation Seen(at: span) "${message}"\nSeen(@place("tree", 1)) :- git_source_tree(_, _).\n`;
    const files = [join(root, "one.rr"), join(root, "two.rr")];
    files.forEach((file, index) => writeFileSync(file, policy(`message ${index}`)));
    const report = await scan(root, { revision: "HEAD", analysisScope: "duplicates", rr: files, databasePath });
    expect(report.status).toBe("failed");
    expect(report.failures[0]?.message).toContain("duplicate .rr policy ID");
    expect(queryContentStore(scanContentStore(databasePath),
      "SELECT scope FROM analysis_scope_manifest WHERE scope = 'named:duplicates' OR scope LIKE 'pending:%'")).toEqual([]);
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

  it("shares cold tree facts and one tape for multiple placements of the same blob", () => {
    const root = checkout();
    const home = `${root}-home`;
    mkdirSync(home);
    for (const path of ["src", "copy", "ignored"]) {
      mkdirSync(join(root, path));
      writeFileSync(join(root, path, "call.py"), path === "ignored" ? "x = g(9)\n" : "x = f(1)\n");
    }
    writeFileSync(join(root, ".rerailignore"), "ignored/\n");
    git(root, "add", ".rerailignore", "src/call.py", "copy/call.py", "ignored/call.py");
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
    const measure = (directory: string) => {
      const runHome = join(home, directory);
      const profile = join(runHome, "profile");
      const report = JSON.parse(execFileSync(process.execPath, ["--import", "tsx", "--input-type=module", "-", root, runHome, policy], {
        input: script, encoding: "utf8",
        env: { ...process.env, RERAIL_PROFILE: profile, RERAIL_CONTENT_STORE_PATH: join(runHome, "content.sqlite") },
      })) as ScanReport;
      expect(report.status, JSON.stringify(report.failures)).toBe("completed");
      expect(report.violations.filter(row => row.code === "Call").map(row => row.path).sort()).toEqual(["copy/call.py", "src/call.py"]);
      expect(report.violations).toContainEqual(expect.objectContaining({ code: "Seen", path: "ignored/call.py" }));
      const uniqueSubtrees = new Set(git(root, "ls-tree", "-d", "-r", "HEAD").split("\n").map(row => row.split(/\s+/u)[2]));
      const trace = readFileSync(join(profile, "profile.txt"), "utf8");
      expect(Number(trace.match(/^\s*count\s+(\d+)\s+git\.tree_decodes$/mu)?.[1])).toBe(1 + uniqueSubtrees.size);
      return Number(trace.match(/^\s*count\s+(\d+)\s+source_tape\.parses$/mu)?.[1]);
    };
    const shared = measure("shared");
    writeFileSync(join(root, "copy/call.py"), "x = f(2)\n");
    git(root, "add", "copy/call.py");
    git(root, "commit", "-qm", "separate blobs for both placements");
    expect(measure("distinct")).toBe(shared + 1);
  }, 30_000);

  it("reuses reference bindings until a decorator changes a file's sibling target", () => {
    const root = checkout(), home = root + "-home";
    mkdirSync(home);
    writeFileSync(join(root, "tasks.py"), "class Jobs:\n    @task\n    def run_one(self):\n        pass\n    def do_one(self):\n        pass\n");
    writeFileSync(join(root, "other.py"), "def unrelated():\n    pass\n");
    git(root, "add", "tasks.py", "other.py");
    git(root, "commit", "-qm", "decorated method and sibling");
    const policy = join(home, "links.rr");
    const evaluate = (decorator: string, phase: string) => {
      const settings = { siblingLinks: [{ decorator, name: { replace: "^run_", by: "do_" } }] };
      writeFileSync(policy, `#include "core.rr"\n.policy "links" revision 1\n.language python\n.references ${JSON.stringify(settings)}\n.violation Linked(at: node) "sibling"\nLinked(n) :- reference_edge(n,_, "sibling").\n`);
      const profile = join(home, phase);
      const script = 'import {runHeadlessScan} from "./src/cli/headless-scan.ts"; const [repo,rr,databasePath] = process.argv.slice(2); console.log(JSON.stringify(await runHeadlessScan({repo,rr:[rr],databasePath,scope:"repository",revision:"HEAD",analysisScope:"links",packs:[],version:"test"})));';
      const report = JSON.parse(execFileSync(process.execPath, ["--import", "tsx", "--input-type=module", "-", root, policy, join(home, "home.sqlite")], {
        input: script, encoding: "utf8", env: { ...process.env, RERAIL_PROFILE: profile, RERAIL_CONTENT_STORE_PATH: join(home, "store.sqlite") },
      })) as ScanReport;
      expect(report.status, JSON.stringify(report.failures)).toBe("completed");
      const trace = readFileSync(join(profile, "profile.txt"), "utf8");
      if (phase === "warm") expect(trace).not.toMatch(/^\s*count\s+[1-9]\d*\s+source_tape\.parses$/mu);
      const count = trace.match(/^\s*count\s+(\d+)\s+reference\.binding_contributions$/mu)?.[1];
      return [report.violations.map(v => v.path), Number(count ?? 0)];
    };
    expect(evaluate("never", "cold")).toEqual([[], 3]);
    expect(evaluate("never", "warm")).toEqual([[], 0]);
    expect(evaluate("absent", "unused")).toEqual([[], 0]);
    expect(evaluate("task", "matched")).toEqual([["tasks.py"], 1]);
  }, 90_000);

  it("shares verified trees in either policy order and reuses independent reference links across revisions", () => {
    const root = realpathSync(mkdtempSync(join(tmpdir(), "rerail-tree-order-")));
    roots.push(root);
    git(root, "init", "-q");
    for (const name of ["other", "src"]) {
      mkdirSync(join(root, name));
      writeFileSync(join(root, name, "__init__.py"), "# package\n");
      writeFileSync(join(root, name, "a.py"), "from .b import helper\n\ndef caller():\n    helper()\n");
      writeFileSync(join(root, name, "b.py"), "def helper():\n    pass\n");
    }
    writeFileSync(join(root, "src/independent.py"), "from .local import another\n\ndef separate():\n    another()\n");
    writeFileSync(join(root, "src/local.py"), "def another():\n    pass\n");
    writeFileSync(join(root, ".rerailignore"), "ignored/\n");
    git(root, "add", "other", "src", ".rerailignore");
    git(root, "commit", "-qm", "source");
    const firstRevision = git(root, "rev-parse", "HEAD");
    const walk = join(root, "walk.rr");
    writeFileSync(walk, `
#include "git.rr"
.policy "walk" revision 1
.violation Seen(at: span) "tree file"
git_tree_walk_request(r, t) :- git_source_tree(r, t).
Seen(@place(p, 1)) :- git_tree_file(_, _, p, _, _), p != ".rerailignore", p != "src/__init__.py", p != "other/__init__.py".
`);
    const references = join(root, "references.rr");
    writeFileSync(references, `
#include "core.rr"
.policy "references" revision 1
.language python
.violation Referenced(at: node) "reference"
Referenced(src) :- reference_edge(src, _, _).
`);
    const script = `
      import { runHeadlessScan } from "./src/cli/headless-scan.ts";
      const [repo, databasePath, revision, ...rr] = process.argv.slice(2);
      console.log(JSON.stringify(await runHeadlessScan({
        repo, databasePath, rr, revision, analysisScope: "order",
        scope: "repository", packs: [], version: "test",
      })));
    `;
    const evaluate = (caseName: string, paths: string[], state: string, revision = "HEAD") => {
      const home = join(root, caseName);
      const profile = join(home, `profile-${state}`);
      const report = JSON.parse(execFileSync(process.execPath, ["--import", "tsx", "--input-type=module", "-", root, join(home, "home.sqlite"), revision, ...paths], {
        input: script, encoding: "utf8",
        env: { ...process.env, RERAIL_PROFILE: profile, RERAIL_CONTENT_STORE_PATH: join(home, "content.sqlite") },
      })) as ScanReport;
      expect(report.status, JSON.stringify(report.failures)).toBe("completed");
      expect(report.violations.filter(finding => finding.code === "Seen").map(finding => finding.path).sort())
        .toEqual(["other/a.py", "other/b.py", "src/a.py", "src/b.py", "src/independent.py", "src/local.py"]);
      expect(report.violations.filter(finding => finding.code === "Referenced").map(finding => finding.path).sort())
        .toEqual(state === "removed" || state === "independent" ? ["other/a.py", "src/independent.py"]
          : ["other/a.py", "src/a.py", "src/independent.py"]);
      const counters = new Map([...readFileSync(join(profile, "profile.txt"), "utf8").matchAll(/^\s*count\s+(\d+)\s+([a-z._]+)$/gmu)]
        .map(([, count, name]) => [name, Number(count)]));
      if (state !== "revisited") expect(counters.get("git.tree_decodes") ?? 0).toBe(["warm", "settings", "revisited"].includes(state) ? 0 : state === "cold" ? 3 : 2);
      expect(counters.get("reference.tape_summaries") ?? 0).toBe(["warm", "revisited", "settings"].includes(state) ? 0 : state === "cold" ? 8 : 1);
      expect(counters.get("reference.binding_contributions") ?? 0).toBe(["warm", "revisited", "settings"].includes(state) ? 0 : state === "cold" ? 8 : 1);
      expect(counters.get("source_ast.parses") ?? 0).toBe(0);
      expect(counters.get("reference.name_links") ?? 0).toBe(["warm", "settings", "revisited"].includes(state) ? 0 : state === "cold" ? 3 : 1);
    };
    evaluate("walk-first", [walk, references], "cold");
    evaluate("references-first", [references, walk], "cold");
    evaluate("references-first", [references, walk], "warm");
    const originalPolicy = readFileSync(references, "utf8");
    writeFileSync(references, originalPolicy.replace(".language python", '.language python\n.references {"constructorFields":{"fieldToParameter":{"replace":"^state_","by":""}}}'));
    evaluate("references-first", [references, walk], "settings");
    writeFileSync(references, originalPolicy);
    writeFileSync(join(root, "src/b.py"), "def replacement():\n    pass\n");
    git(root, "add", "src/b.py");
    git(root, "commit", "-qm", "change the target file");
    evaluate("references-first", [references, walk], "removed");
    writeFileSync(join(root, "src/local.py"), "def another():\n    return 1\n");
    git(root, "add", "src/local.py");
    git(root, "commit", "-qm", "change an independent definition in the same package");
    evaluate("references-first", [references, walk], "independent");
    writeFileSync(join(root, "src/b.py"), "def helper():\n    return 42\n");
    git(root, "add", "src/b.py");
    git(root, "commit", "-qm", "bind the name again");
    evaluate("references-first", [references, walk], "restored");
    evaluate("references-first", [references, walk], "revisited", firstRevision);
    const store = join(root, "references-first/content.sqlite");
    const owned = () => queryContentStore<{ n: number }>(store, `
      SELECT count(DISTINCT u.space) AS n FROM frame_program_used u
      JOIN corpus_manifest_plans p ON p.plan_digest=u.program
      JOIN analysis_scope_manifest a USING(manifest_id)
      WHERE u.space=u.program AND a.scope='named:order'
    `)[0]!.n;
    // Both the source and link phases have an owned image for each of the four trees.
    expect(owned()).toBeGreaterThanOrEqual(8);
    loadNativeBinding().deleteAnalysisScope(store, "order", false);
    expect(owned()).toBe(0);
  }, 120_000);

  it("refreshes the field link that consulted a changed constructor mapping", async () => {
    const root = checkout();
    const policy = join(root, "constructor.rr");
    writeFileSync(join(root, "a.py"), [
      "class DB:", "    def read(self): pass",
      "class Client:", "    def __init__(self, dependency: DB): pass",
      "    def run(self): return self.state_dependency.read()", "",
    ].join("\n"));
    git(root, "commit", "-qam", "constructor field");
    const evaluate = async (fields: boolean) => {
      writeFileSync(policy, `
#include "core.rr"
.policy "constructor" revision 1
.language python
${fields ? '.references {"constructorFields":{"fieldToParameter":{"replace":"^state_","by":""}}}' : ""}
.violation Field(at: node) "field resolves"
Field(src) :- reference_edge(src, dst, "field"), node_text(dst, body), contains("def read", body).
`);
      const report = await scan(root, { revision: "HEAD", analysisScope: "fields", rr: [policy] });
      expect(report.status, JSON.stringify(report.failures)).toBe("completed");
      return report.violations.map(v => v.message);
    };
    expect(await evaluate(false)).toEqual([]);
    expect(await evaluate(true)).toEqual(["field resolves"]);
    expect(await evaluate(false)).toEqual([]);
  });

  it("reuses an inherited method link across unrelated edits and invalidates its absent override", () => {
    const root = realpathSync(mkdtempSync(join(tmpdir(), "rerail-method-link-")));
    roots.push(root);
    git(root, "init", "-q");
    mkdirSync(join(root, "pkg"));
    writeFileSync(join(root, "pkg/__init__.py"), "# package\n");
    const service = [
      "class Base:", "    def hook(self):", "        return 'base'",
      "", "class Derived(Base):", "    def call(self):", "        self.hook()", "",
    ].join("\n");
    writeFileSync(join(root, "pkg/service.py"), service);
    writeFileSync(join(root, "pkg/other.py"), "def unrelated():\n    pass\n");
    const commit = (message: string) => {
      git(root, "add", "pkg");
      git(root, "commit", "-qm", message);
    };
    commit("inherited method");
    const policy = join(root, "method.rr");
    writeFileSync(policy, `
#include "core.rr"
.policy "method" revision 1
.language python
.violation Method(at: node, target: symbol) "{target}"
Method(src, text) :- reference_edge(src, dst, "self"), node_text(dst, text).
`);
    const script = `
      import { runHeadlessScan } from "./src/cli/headless-scan.ts";
      const [repo, databasePath, policy] = process.argv.slice(2);
      console.log(JSON.stringify(await runHeadlessScan({
        repo, databasePath, rr: [policy], revision: "HEAD", analysisScope: "methods",
        scope: "repository", packs: [], version: "test",
      })));
    `;
    const evaluate = (state: string, result: string, contributions: number, links: number) => {
      const home = join(root, "home");
      const profile = join(home, state);
      const report = JSON.parse(execFileSync(process.execPath, ["--import", "tsx", "--input-type=module", "-", root, join(home, "home.sqlite"), policy], {
        input: script, encoding: "utf8",
        env: { ...process.env, RERAIL_PROFILE: profile, RERAIL_CONTENT_STORE_PATH: join(home, "content.sqlite") },
      })) as ScanReport;
      expect(report.status, JSON.stringify(report.failures)).toBe("completed");
      expect(report.violations).toHaveLength(1);
      expect(report.violations[0]!.message).toContain(`return '${result}'`);
      const counters = new Map([...readFileSync(join(profile, "profile.txt"), "utf8").matchAll(/^\s*count\s+(\d+)\s+([a-z._]+)$/gmu)]
        .map(([, count, name]) => [name, Number(count)]));
      expect(counters.get("reference.binding_contributions") ?? 0).toBe(contributions);
      expect(counters.get("reference.name_links") ?? 0).toBe(links);
    };
    // The graph also links the inheritance reference to Base.
    evaluate("cold", "base", 3, 2);
    evaluate("warm", "base", 0, 0);
    writeFileSync(join(root, "pkg/other.py"), "def renamed():\n    pass\n");
    commit("unrelated definition renamed");
    evaluate("renamed", "base", 1, 0);
    writeFileSync(join(root, "pkg/service.py"), `${service}\n    def hook(self):\n        return 'override'\n`);
    commit("add the prioritized override");
    evaluate("override", "override", 1, 1);
  }, 90_000);

  it.each(["loose", "packed"])("rechecks a %s tree after corruption and restoration with the same cache", (representation) => {
    const root = checkout();
    const home = `${root}-home`;
    mkdirSync(home);
    mkdirSync(join(root, "src"));
    writeFileSync(join(root, "src/a.py"), "x = 1\n");
    git(root, "add", "src/a.py");
    git(root, "commit", "-qm", "nested source");
    const object = git(root, "rev-parse", "HEAD:src");
    const objects = join(root, ".git/objects");
    const loose = join(objects, object.slice(0, 2), object.slice(2));
    let target = loose;
    let offset = 0;
    if (representation === "packed") {
      const base = join(objects, "pack/verification");
      const suffix = execFileSync("git", ["pack-objects", base], { cwd: root, input: `${object}\n`, encoding: "utf8" }).trim();
      const index = `${base}-${suffix}.idx`;
      target = `${base}-${suffix}.pack`;
      const row = git(root, "verify-pack", "-v", index).split("\n").find(line => line.startsWith(object))!;
      const [, , , size, start] = row.split(/\s+/u);
      offset = Number(start) + Math.floor(Number(size) / 2);
      rmSync(loose);
    }
    const policy = join(home, "walk.rr");
    writeFileSync(policy, `
#include "git.rr"
.policy "walk" revision 1
.violation Complete(at: span) "complete"
.violation Unreadable(at: span, reason: symbol) "{reason}"
git_tree_walk_request(r, t) :- git_source_tree(r, t).
Complete(@place("tree", 1)) :- git_tree_walk_complete(_, _).
Unreadable(@place("tree", 1), why) :- git_tree_walk_error(_, _, _, why).
`);
    const script = `
      import { join } from "node:path";
      import { runHeadlessScan } from "./src/cli/headless-scan.ts";
      const [repo, home, policy] = process.argv.slice(2);
      const report = await runHeadlessScan({
        repo, databasePath: join(home, "rerail.sqlite"), rr: [policy], revision: "HEAD",
        analysisScope: "readability", scope: "repository", packs: [], version: "test",
      });
      console.log(JSON.stringify(report.violations.map(({ code, message }) => [code, message])));
    `;
    const evaluate = (profile?: string) => JSON.parse(execFileSync(process.execPath, ["--import", "tsx", "--input-type=module", "-", root, home, policy], {
      input: script, encoding: "utf8",
      env: { ...process.env, RERAIL_CONTENT_STORE_PATH: join(home, "readability.sqlite"), ...(profile === undefined ? {} : { RERAIL_PROFILE: profile }) },
    }));
    const original = readFileSync(target);
    expect(evaluate()).toEqual([["Complete", "complete"]]);
    const unrelated = git(root, "rev-parse", "HEAD:a.py");
    const base = join(objects, "pack/unrelated");
    const suffix = execFileSync("git", ["pack-objects", base], { cwd: root, input: `${unrelated}\n`, encoding: "utf8" }).trim();
    for (const state of ["created", "touched", "multi-index"]) {
      if (state === "multi-index") git(root, "multi-pack-index", "write");
      if (state === "touched") utimesSync(`${base}-${suffix}.pack`, new Date(), new Date());
      const profile = join(home, `unrelated-pack-${state}`);
      expect(evaluate(profile)).toEqual([["Complete", "complete"]]);
      const decodes = readFileSync(join(profile, "profile.txt"), "utf8").match(/^\s*count\s+(\d+)\s+git\.tree_decodes$/mu)?.[1];
      expect(Number(decodes ?? 0)).toBe(0);
    }
    const corrupted = Buffer.from(original);
    corrupted[offset] = corrupted[offset]! ^ 0xff;
    chmodSync(target, 0o644);
    writeFileSync(target, corrupted);
    expect(evaluate()).toEqual([["Unreadable", "unreadable"]]);
    writeFileSync(target, original);
    expect(evaluate()).toEqual([["Complete", "complete"]]);
  }, 30_000);

  it("keeps excluded worktree names out of the durable language classifier", async () => {
    const root = checkout();
    const home = `${root}-home`;
    const store = join(home, "content.sqlite");
    const secret = "untracked-private-client-7db9.pem";
    const ignored = "ignored-private-client-1b07.py";
    writeFileSync(join(root, secret), "certificate");
    writeFileSync(join(root, ".gitignore"), "*.hidden.py\n");
    const ignoredPath = ignored.replace(".py", ".hidden.py");
    writeFileSync(join(root, ignoredPath), "NEEDLE");
    const document = { ...(needle("wide", "NEEDLE") as object), scope: { include: ["**/*"], exclude: [] } };
    writeRule(root, "needle.json", document);
    vi.stubEnv("RERAIL_CONTENT_STORE_PATH", store);
    try {
      const report = await scan(root);
      expect(report.status, JSON.stringify(report.failures)).toBe("completed");
      expect(report.violations.map(v => v.path)).toContain("a.py");
      expect(report.violations.map(v => v.path)).not.toContain(ignoredPath);
      for (const basename of [secret, ignoredPath]) {
        // The atom dictionary can hold plain strings as well as packed segments.
        const found = queryContentStore(store, "SELECT 1 FROM atom_segment WHERE instr(bytes, cast(? AS blob))>0 LIMIT 1", [basename]);
        expect(found, basename).toEqual([]);
        expect(readFileSync(store).includes(Buffer.from(basename)), basename).toBe(false);
      }
    } finally { vi.unstubAllEnvs(); }
  });

  it("retains language decisions across inventory batches and process restarts", () => {
    const root = checkout();
    const home = `${root}-home`;
    mkdirSync(home);
    // More than two 4096-file batches. Only the Python file needs parsing.
    for (let i = 0; i < 8204; i++) writeFileSync(join(root, `unlisted-${i}.unlisted`), "");
    writeRule(root, "needle.json", {
      format: "rerail.policy/v1", id: "python", revision: 1, mode: "log", scope: { include: ["**/*"], exclude: [] },
      clauses: [{ id: "assignment", violation: { code: "ASSIGN", anchor: "witness.span", message: "assignment" },
        forbid: { ast: { language: "python", rule: { kind: "assignment" } } } }],
    });
    const evaluate = (state: string) => {
      const profile = join(home, state);
      const report: ScanReport = JSON.parse(execFileSync(process.execPath, ["--import", "tsx", "--input-type=module", "-", root, home], {
        encoding: "utf8", env: { ...process.env, RERAIL_PROFILE: profile, RERAIL_CONTENT_STORE_PATH: join(home, "content.sqlite") },
        input: `
          import { join } from "node:path";
          import { runHeadlessScan } from "./src/cli/headless-scan.ts";
          const [repo, home] = process.argv.slice(2);
          console.log(JSON.stringify(await runHeadlessScan({ repo, databasePath: join(home, "rerail.sqlite"), scope: "repository", packs: [], version: "test" })));
        `,
      }));
      expect(report.status, JSON.stringify(report.failures)).toBe("completed");
      expect(report.violations.map(v => v.path)).toContain("a.py");
      return Number(readFileSync(join(profile, "profile.txt"), "utf8").match(/^\s*count\s+(\d+)\s+source_language\.evaluations$/mu)?.[1] ?? 0);
    };
    expect(evaluate("cold")).toBeGreaterThan(0);
    expect(evaluate("reopened")).toBe(0);
  }, 120_000);

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
      const tapes = queryContentStore<{ version: string; count: number }>(store,
        "SELECT printf('%015x', grammar_version) AS version, count(*) AS count FROM tape_objects GROUP BY grammar_version ORDER BY grammar_version");
      return { paths, keys, classifications: counters.get("source_language.evaluations") ?? 0,
        parses: counters.get("source_tape.parses") ?? 0, tapes };
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

  it("changes only the renamed file's grammar in both worktree and Git scans", async () => {
    const root = checkout();
    const home = `${root}-home`;
    const databasePath = join(home, "rerail.sqlite");
    const tapeRows = (store: string) => queryContentStore<{ tape_id: number; source_id: number; grammar_id: number }>(
      store, "SELECT tape_id, source_id, grammar_id FROM tape_objects",
    );
    const rule = (language: string, kind: string) => ({
      format: "rerail.policy/v1", id: `rename-${language}`, revision: 1, mode: "log",
      scope: { include: ["a.py", "a.json", "b.py"], exclude: [] },
      clauses: [{
        id: "dictionary", violation: { code: language.toUpperCase(), anchor: "witness.span", message: language },
        forbid: { ast: { language, rule: { kind } } },
      }],
    });
    writeRule(root, "rename-python.json", rule("python", "dictionary"));
    writeRule(root, "rename-json.json", rule("json", "object"));
    writeFileSync(join(root, "a.py"), '{"changed_extension": 1}\n');
    writeFileSync(join(root, "b.py"), '{"stable_extension": 2}\n');
    git(root, "add", "a.py", "b.py");
    git(root, "commit", "-qm", "Python files");
    const modes = ["worktree", "git"] as const;
    const evaluate = async (mode: typeof modes[number]) => {
      const store = join(home, `${mode}.sqlite`);
      vi.stubEnv("RERAIL_CONTENT_STORE_PATH", store);
      const report = await scan(root, {
        databasePath,
        ...(mode === "git" ? { revision: "HEAD", analysisScope: "rename" } : {}),
      });
      expect(report.status, JSON.stringify(report.failures)).toBe("completed");
      return {
        findings: report.violations.map(finding => [finding.path, finding.message]).sort(),
        tapes: tapeRows(store),
      };
    };
    try {
      const before = new Map<typeof modes[number], ReturnType<typeof tapeRows>>();
      for (const mode of modes) {
        const result = await evaluate(mode);
        expect(result.findings).toEqual([["a.py", "python"], ["b.py", "python"]]);
        before.set(mode, result.tapes);
      }
      git(root, "mv", "a.py", "a.json");
      git(root, "commit", "-qm", "rename to JSON");
      for (const mode of modes) {
        const result = await evaluate(mode);
        expect(result.findings).toEqual([["a.json", "json"], ["b.py", "python"]]);
        const previous = before.get(mode)!;
        const added = result.tapes.filter(tape => !previous.some(old => old.tape_id === tape.tape_id));
        expect(added).toHaveLength(1);
        expect(previous.some(old => old.source_id === added[0]!.source_id && old.grammar_id !== added[0]!.grammar_id)).toBe(true);
        expect(result.tapes.some(tape => previous.some(old => old.source_id !== added[0]!.source_id && old.tape_id === tape.tape_id)),
          JSON.stringify({ mode, previous, current: result.tapes })).toBe(true);
      }
    } finally { vi.unstubAllEnvs(); }
  }, 30_000);

  it("adopts a new worktree suffix while keeping tapes for the existing suffixes", () => {
    const root = checkout();
    const home = `${root}-home`;
    const store = join(home, "worktree.sqlite");
    writeRule(root, "needle.json", {
      format: "rerail.policy/v1", id: "suffixes", revision: 1, mode: "log",
      scope: { include: ["a.py", "b.sample"], exclude: [] },
      clauses: [{
        id: "call", violation: { code: "CALL", anchor: "witness.span", message: "call" },
        forbid: { ast: { language: "python", rule: { kind: "call" } } },
      }],
    });
    writeFileSync(join(root, "a.py"), "f(1)\n");
    writeFileSync(join(root, "b.sample"), "f(2)\n");
    git(root, "add", "a.py", "b.sample");
    git(root, "commit", "-qm", "two source suffixes");
    const script = `
      import { join } from "node:path";
      import { SOURCE_LANGUAGES } from "./src/engine/code-rule/code-rule-languages.ts";
      const [repo, home, extended] = process.argv.slice(2);
      if (extended === "yes") SOURCE_LANGUAGES.set(".sample", "python");
      const { runHeadlessScan } = await import("./src/cli/headless-scan.ts");
      console.log(JSON.stringify(await runHeadlessScan({
        repo, databasePath: join(home, "rerail.sqlite"), scope: "repository", packs: [], version: "test",
      })));
    `;
    const evaluate = (extended: string) => {
      const report = JSON.parse(execFileSync(process.execPath, ["--import", "tsx", "--input-type=module", "-", root, home, extended], {
        input: script, encoding: "utf8", env: { ...process.env, RERAIL_CONTENT_STORE_PATH: store },
      })) as ScanReport;
      expect(report.status, JSON.stringify(report.failures)).toBe("completed");
      return {
        paths: report.violations.map(finding => finding.path),
        tapes: queryContentStore<{ tape_id: number }>(store, "SELECT tape_id FROM tape_objects ORDER BY tape_id").map(row => row.tape_id),
      };
    };
    const initial = evaluate("no");
    expect(initial.paths).toEqual(["a.py"]);
    expect(initial.tapes).toHaveLength(1);
    expect(evaluate("no")).toEqual(initial);
    const extended = evaluate("yes");
    expect(extended.paths).toEqual(["a.py", "b.sample"]);
    expect(extended.tapes).toHaveLength(2);
    expect(extended.tapes).toContain(initial.tapes[0]);
  }, 30_000);

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
    const sets = queryContentStore(scanContentStore(databasePath),
      "SELECT count(DISTINCT lane) AS lanes FROM analysis_scope_manifest JOIN corpus_manifests USING(manifest_id) WHERE scope = ? AND policy <> lane GROUP BY policy",
      [`cwd:${analysisScopeHome(databasePath)}:${root}`]);
    expect(sets).toEqual([{ lanes: 2 }]);
  });

  it.each(["configured", "rr"])("reports a corrupt selected blob and keeps other %s findings", async policy => {
    const root = checkout();
    const home = `${root}-home`;
    mkdirSync(home);
    writeFileSync(join(root, "b.py"), "b = NEEDLE\n");
    git(root, "add", "b.py");
    git(root, "commit", "-qm", "second source");
    const rr = join(home, "source.rr");
    writeFileSync(rr, `
.policy "source" revision 1
.language python
.violation Needle(at: node) "needle"
Needle(n) :- kind(n, "identifier"), text(n, "NEEDLE").
`);
    const oid = git(root, "rev-parse", "HEAD:b.py");
    const loose = join(root, ".git", "objects", oid.slice(0, 2), oid.slice(2));
    const original = readFileSync(loose);
    const run = (paths?: string[]) => scan(root, {
      databasePath: join(home, "rerail.sqlite"), revision: "HEAD", analysisScope: "source-objects",
      ...(paths === undefined ? {} : { paths }), ...(policy === "rr" ? { rr: [rr] } : {}),
    });
    const allPaths = ["a.py", "b.py"];
    const findings = (report: ScanReport) => report.violations.map(finding => finding.path).sort();
    vi.stubEnv("RERAIL_CONTENT_STORE_PATH", join(home, "content.sqlite"));
    try {
      const cold = await run();
      expect(cold.status, JSON.stringify(cold.failures)).toBe("completed");
      expect(findings(cold)).toEqual(allPaths);
      rmSync(loose);
      const corrupt = Buffer.from("b = BROKEN\n");
      writeFileSync(loose, deflateSync(Buffer.concat([Buffer.from(`blob ${corrupt.length}\0`), corrupt])));
      const warm = await run();
      expect(warm.status).toBe("failed");
      expect(warm.failures).toEqual([expect.objectContaining({ path: "b.py", message: expect.stringMatching(/unreadable|checksum/iu) })]);
      expect(findings(warm)).toEqual(["a.py"]);
      const selected = await run(["a.py"]);
      expect(selected.status, JSON.stringify(selected.failures)).toBe("completed");
      expect(findings(selected)).toEqual(["a.py"]);
      writeFileSync(loose, original);
      const restored = await run();
      expect(restored.status, JSON.stringify(restored.failures)).toBe("completed");
      expect(findings(restored)).toEqual(allPaths);
    } finally {
      vi.unstubAllEnvs();
    }
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

  it("distinguishes a missing --rules-ref from a tree", async () => {
    const root = checkout();
    await expect(scan(root, { rulesRef: "mian" })).rejects.toThrow(/--rules-ref mian.*No Git reference matches/u);
    await expect(scan(root, { rulesRef: "HEAD^{tree}" })).rejects.toThrow(/--rules-ref .*resolved to tree.*requires a commit/u);
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
