import { execFileSync } from "node:child_process";
import { existsSync, mkdtempSync } from "node:fs";
import { mkdir, mkdtemp, realpath, rename, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, relative, resolve, sep } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { CheckoutOwners } from "../../../src/engine/catalog/checkout-owner.js";
import { CheckoutCodeRuleRouter } from "../../../src/engine/code-rule/checkout-code-rule-router.js";
import { CodeRuleBridge } from "../../../src/engine/code-rule/code-rule-bridge.js";
import { NativeCodeRuleEnforcer } from "../../../src/engine/code-rule/code-rule-enforcer.js";
import { openRerailDatabase } from "../../../src/engine/storage/database.js";
import { CheckoutCatalogs, CheckoutCodeRuleCatalog, startingCheckout } from "../../../src/engine/code-rule/code-rule-dsl-catalog.js";
import { SqliteAuditReader } from "../../../src/engine/audit/audit-store.js";
import { SqliteAuditControlStore } from "../../../src/engine/audit/audit-control.js";
import type { AuditRecorder, AuditInput, AuditResult } from "../../../src/engine/audit/audit.js";
import { type CodeRule, codeRuleSchema } from "../../../src/engine/code-rule/code-rule-schema.js";
import { recordCheckout } from "../../../src/engine/catalog/repository-registry.js";
import { setCheckoutStarted, setRepositoryStarted, RerailRuntime, SqliteSessionReader } from "../../../src/runtime/index.js";
import { setRuleSetting } from "../../../src/engine/catalog/rule-configuration.js";
import { readRuleCatalog } from "../../../src/engine/catalog/rule-catalog.js";
import type { RuleCatalog } from "../../../src/engine/catalog/repository-model.js";
import { PACKAGE_ROOT } from "../../../src/engine/package-root.js";
import type { RuleMode } from "../../../src/engine/catalog/rule-mode.js";
import { checkouts, repositoryAudits } from "../../../src/engine/storage/schema.js";
import { testEmbedder } from "../../support/hook-events.js";
import { setUpTestRepository } from "../../support/repository-rules.js";
import { startTestRepository, stopTestRepository } from "../../support/started-repository.js";
import { symlinkOrSkip } from "../../support/platform/links.js";
import { removeFolder, removeFolderSync } from "../../support/platform/remove-folder.js";
import { GIT_OPEN_AGAIN_MS, nativeGitAccess } from "../../../src/engine/native/git-access.js";
import type * as GitAccessModule from "../../../src/engine/native/git-access.js";
import type { NativeGitAccess } from "../../../src/engine/native/native.js";
import { GitIgnoreMatchers } from "../../../src/engine/native/git-ignore-matchers.js";

/**
 * The owner of Git reads `nativeGitAccess` hands out while a case swaps it, as a process does once Git answers an ask
 * at last; the process's own otherwise.
 */
const swappedOwner = vi.hoisted(() => {
  const swapped: { current: NativeGitAccess | undefined } = { current: undefined };
  return swapped;
});
vi.mock("../../../src/engine/native/git-access.js", async importOriginal => {
  const original = await importOriginal<typeof GitAccessModule>();
  return {
    ...original,
    nativeGitAccess: (...args: Parameters<typeof original.nativeGitAccess>) => swappedOwner.current ?? original.nativeGitAccess(...args),
  };
});

const cleanups: Array<() => Promise<void>> = [];
afterEach(async () => {
  vi.unstubAllEnvs();
  for (const cleanup of cleanups.splice(0).reverse()) await cleanup();
});

/** A rule that forbids `NEEDLE` in every Python file and reports `code`. */
function needle(id: string, code: string, mode: RuleMode = "block"): CodeRule {
  return codeRuleSchema.parse({
    format: "rerail.policy/v1", id, revision: 1, mode,
    scope: { include: ["**/*.py"], exclude: [] },
    clauses: [{ id: "needle", violation: { code, anchor: "witness.span", message: `No ${code}` }, forbid: { regex: { pattern: "NEEDLE", source: "file.content" } } }],
  });
}

function file(rule: CodeRule) {
  return { kind: "code" as const, rule };
}

function git(cwd: string, ...args: string[]): void {
  execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@local", "-c", "protocol.file.allow=always", ...args], { cwd, stdio: "ignore" });
}

async function temporaryRoot(): Promise<string> {
  const root = await realpath(await mkdtemp(join(tmpdir(), "rerail-checkout-router-")));
  cleanups.push(() => removeFolder(root));
  return root;
}

/** The rule count the newest rule coverage of `sessionId` recorded, current or not; null where none was recorded. */
function recordedRuleCount(databasePath: string, sessionId: string): number | null {
  const database = openRerailDatabase(databasePath);
  try {
    const row = database.$client
      .prepare("select json_extract(payload_json, '$.activeRuleCount') as count from events where session_id = ? and kind = 'rule-coverage' order by sequence desc limit 1")
      .get(sessionId) as { count: number | null } | undefined;
    return row?.count ?? null;
  } finally {
    database.$client.close();
  }
}

/** A committed Git repository at `path`; returns its physical path. */
async function repository(path: string): Promise<string> {
  await mkdir(join(path, "api"), { recursive: true });
  await writeFile(join(path, "README.md"), "seed\n");
  git(path, "init", "--quiet");
  git(path, "add", "--", "README.md");
  git(path, "commit", "--quiet", "-m", "seed");
  return await realpath(path);
}

/** `parent` with a real submodule at `sub`, cloned from a repository beside it. */
async function withSubmodule(root: string): Promise<{ parent: string; sub: string }> {
  const source = await repository(join(root, "sub-source"));
  const parent = await repository(join(root, "parent"));
  git(parent, "submodule", "add", "--quiet", source, "sub");
  git(parent, "commit", "--quiet", "-m", "submodule");
  return { parent, sub: await realpath(join(parent, "sub")) };
}

type RecordedRun = AuditInput & { result?: AuditResult };

function recordingAudits(): { recorder: AuditRecorder; audits: RecordedRun[]; interrupts: string[] } {
  const audits: RecordedRun[] = [];
  const interrupts: string[] = [];
  return {
    recorder: {
      start(input) {
        audits.push({ ...input, paths: [...input.paths] });
        return String(audits.length - 1);
      },
      finish(auditId, result) { audits[Number(auditId)]!.result = result; },
      progress(auditId, result) { audits[Number(auditId)]!.result = { status: "completed", ...result }; },
      interrupt(sessionId) { interrupts.push(sessionId); },
    },
    audits,
    interrupts,
  };
}

/** Each violation of every finished audit: the checkout that judged it, its checkout-relative path, code and mode. */
function violations(audits: readonly RecordedRun[]) {
  return audits.flatMap(audit => (audit.result?.violations ?? []).map(violation => ({
    checkout: audit.workspaceRoot, path: violation.path, code: violation.code, mode: violation.mode,
  })));
}

function router(
  sessionRoot: string,
  databasePath: string,
  recorder: AuditRecorder,
  ruleCatalog?: RuleCatalog,
  followGitignore = false,
  requestSteer: () => Promise<undefined> = async () => undefined,
  gitIgnoreMatchers?: GitIgnoreMatchers,
): CheckoutCodeRuleRouter {
  const owners = new CheckoutOwners();
  const catalogs = new CheckoutCatalogs(databasePath, ruleCatalog === undefined ? {} : { ruleCatalog });
  const auditor = new CheckoutCodeRuleRouter({
    sessionRoot,
    startingCheckout: startingCheckout(owners, sessionRoot).checkoutRoot,
    owners,
    catalogs,
    steerDelivery: { requestSteer },
    audits: recorder,
    followGitignore,
    gitIgnoreMatchers,
  });
  cleanups.push(async () => {
    await auditor.close();
    catalogs.close();
  });
  return auditor;
}

async function edit(auditor: CheckoutCodeRuleRouter, sessionId: string, revision: number, ...paths: string[]): Promise<void> {
  await auditor.auditGeneratedEdits(sessionId, {
    revision,
    checkpoints: ["edit"],
    edits: paths.map(path => ({ path, language: "python" as const, generatedText: "NEEDLE\n" })),
  });
  await auditor.settleGeneratedEdits();
}

describe("CheckoutCodeRuleRouter", () => {
  it("interrupts the session's audits when it closes, unless it is told they belong to a session that replaced it", async () => {
    const root = await temporaryRoot();
    const databasePath = join(root, "rerail.sqlite");
    const session = await repository(join(root, "session"));
    setUpTestRepository(session, { library: [file(needle("test:session", "SESSION"))] }, databasePath);
    const { recorder, interrupts } = recordingAudits();
    const closing = router(session, databasePath, recorder);
    const replaced = router(session, databasePath, recorder);
    await closing.openWorkspace("closing");
    await replaced.openWorkspace("replaced");
    // Each opening interrupted the session's earlier runs once.
    expect(interrupts).toEqual(["closing", "replaced"]);

    await replaced.close({ interruptAudits: false });
    expect(interrupts).toEqual(["closing", "replaced"]);
    await closing.close();
    expect(interrupts).toEqual(["closing", "replaced", "closing"]);
  });

  it("judges a submodule's file by the submodule's own rules", async () => {
    const root = await temporaryRoot();
    const databasePath = join(root, "rerail.sqlite");
    const { parent, sub } = await withSubmodule(root);
    setUpTestRepository(parent, { library: [file(needle("test:parent", "PARENT"))] }, databasePath);
    setUpTestRepository(sub, { library: [file(needle("test:sub", "SUB"))] }, databasePath);
    const { recorder, audits } = recordingAudits();
    const auditor = router(parent, databasePath, recorder);

    await auditor.openWorkspace("submodule");
    await edit(auditor, "submodule", 1, "api/a.py", "sub/api/b.py");

    expect(violations(audits).toSorted((a, b) => a.code.localeCompare(b.code))).toEqual([
      { checkout: parent, path: "api/a.py", code: "PARENT", mode: "block" },
      { checkout: sub, path: "api/b.py", code: "SUB", mode: "block" },
    ]);
  });

  it("leaves out what Git ignores in a checkout that runs no rules, not set up or stopped, while the session follows .gitignore", async () => {
    const root = await temporaryRoot();
    const databasePath = join(root, "rerail.sqlite");
    const session = await repository(join(root, "session"));
    await writeFile(join(session, ".gitignore"), "scratch/\n*.log\n");
    // Started but not set up: no lane runs, and a walk of the session's folder asks this of every folder and file in it.
    startTestRepository(session, databasePath);
    const { recorder } = recordingAudits();
    const following = router(session, databasePath, recorder, undefined, true);
    await following.openWorkspace("not-set-up");
    expect(following.acceptsPath("api/x.py")).toBe(false);
    expect([following.ignoresPath("scratch", true), following.ignoresPath("run.log", false)]).toEqual([true, true]);
    expect([following.ignoresPath("api", true), following.ignoresPath("api/x.py", false)]).toEqual([false, false]);

    const unfollowed = router(session, databasePath, recorder);
    await unfollowed.openWorkspace("not-set-up-unfollowed");
    expect(unfollowed.ignoresPath("scratch", true)).toBe(false);

    setUpTestRepository(session, { library: [file(needle("test:session", "SESSION"))] }, databasePath);
    stopTestRepository(session, databasePath);
    const stopped = router(session, databasePath, recorder, undefined, true);
    await stopped.openWorkspace("stopped");
    expect(stopped.acceptsPath("api/x.py")).toBe(false);
    expect([stopped.ignoresPath("scratch", true), stopped.ignoresPath("api", true)]).toEqual([true, false]);
  });

  it("keeps following a checkout's Git ignore rules after one path's rules cannot be read", async () => {
    const root = await temporaryRoot();
    const databasePath = join(root, "rerail.sqlite");
    const session = await repository(join(root, "session"));
    await writeFile(join(session, ".gitignore"), "scratch/\n*.log\n");
    startTestRepository(session, databasePath);
    // A nested `.gitignore` the matcher cannot read (or the index caught mid-rewrite) fails that one path's question.
    const access = nativeGitAccess();
    const open = access.openGitIgnoreMatcher.bind(access);
    const opens = vi.spyOn(access, "openGitIgnoreMatcher").mockImplementation(checkoutRoot => {
      const matcher = open(checkoutRoot);
      return matcher === null ? null : {
        ignored: (path, directory) => {
          if (path.startsWith("locked/")) throw new Error(`cannot read Git excludes for ${path}: Permission denied`);
          return matcher.ignored(path, directory);
        },
        ignoredBy: (path, directory) => matcher.ignoredBy(path, directory),
        sourcesUnchanged: () => matcher.sourcesUnchanged(),
      };
    });
    cleanups.push(async () => { opens.mockRestore(); });
    const errors = vi.spyOn(process.stderr, "write").mockImplementation(() => true);
    cleanups.push(async () => { errors.mockRestore(); });
    const { recorder } = recordingAudits();
    const following = router(session, databasePath, recorder, undefined, true);
    await following.openWorkspace("unreadable-path");

    expect([following.ignoresPath("locked/a.log", false), following.ignoresPath("locked/b.log", false)]).toEqual([false, false]);
    expect([following.ignoresPath("scratch", true), following.ignoresPath("run.log", false)]).toEqual([true, true]);
    expect(following.ignoresPath("api/x.py", false)).toBe(false);
    expect(opens).toHaveBeenCalledTimes(1);
    expect(errors.mock.calls.filter(([text]) => String(text).includes("cannot read Git ignore rules"))).toHaveLength(1);
  });

  it("takes the daemon's Git ignore matcher once for every walk question about a checkout that runs no rules", async () => {
    const root = await temporaryRoot();
    const databasePath = join(root, "rerail.sqlite");
    const session = await repository(join(root, "session"));
    await writeFile(join(session, ".gitignore"), "scratch/\n*.log\n");
    startTestRepository(session, databasePath);
    const matchers = new GitIgnoreMatchers();
    // Each take checks the shared matcher's sources: Git's configuration and three whole files read again.
    const takes = vi.spyOn(matchers, "matcher");
    const { recorder } = recordingAudits();
    const following = router(session, databasePath, recorder, undefined, true, undefined, matchers);
    await following.openWorkspace("walk-questions");

    for (const view of [following.walkView(), following.walkView()]) {
      const answers = [view.ignoresPath("scratch", true), view.ignoresPath("api", true), view.ignoresPath("api/x.py", false), view.ignoresPath("run.log", false)];
      expect(answers).toEqual([true, false, false, true]);
    }
    expect(takes).toHaveBeenCalledTimes(1);
  });

  it("opens a checkout's Git ignore rules again once GIT_OPEN_AGAIN_MS has passed after a timeout, and reports the refusal that follows too", async () => {
    const root = await temporaryRoot();
    const databasePath = join(root, "rerail.sqlite");
    const session = await repository(join(root, "session"));
    await writeFile(join(session, ".gitignore"), "*.log\n");
    // Started but not set up: no lane runs, so the router opens the checkout's rules itself.
    startTestRepository(session, databasePath);
    const access = nativeGitAccess();
    const timedOut = Object.assign(new Error("opening Git's ignore rules gave no answer within 10 s"), { code: "GitReadTimedOut" });
    const opens = vi.spyOn(access, "openGitIgnoreMatcher")
      .mockImplementationOnce(() => { throw timedOut; })
      .mockImplementation(() => { throw new Error("detected dubious ownership"); });
    cleanups.push(async () => { opens.mockRestore(); });
    const errors = vi.spyOn(process.stderr, "write").mockImplementation(() => true);
    cleanups.push(async () => { errors.mockRestore(); });
    const { recorder } = recordingAudits();
    const following = router(session, databasePath, recorder, undefined, true);
    await following.openWorkspace("timed-out-then-refused");

    let clock = Date.now();
    const now = vi.spyOn(Date, "now").mockImplementation(() => clock);
    cleanups.push(async () => { now.mockRestore(); });
    expect(["a.log", "b.log"].map(path => following.ignoresPath(path, false))).toEqual([false, false]);
    expect(opens).toHaveBeenCalledTimes(1);
    clock += GIT_OPEN_AGAIN_MS;
    expect(["c.log", "d.log"].map(path => following.ignoresPath(path, false))).toEqual([false, false]);
    expect(opens).toHaveBeenCalledTimes(2);
    const logged = errors.mock.calls.map(([text]) => String(text)).filter(text => text.includes("cannot read Git ignore rules"));
    expect(logged).toEqual([expect.stringContaining("gave no answer within 10 s"), expect.stringContaining("detected dubious ownership")]);
  });

  it("looks up the checkout of a walk's files once per folder, not once per file", async () => {
    const root = await temporaryRoot();
    const databasePath = join(root, "rerail.sqlite");
    const session = await repository(join(root, "session"));
    await mkdir(join(session, "api", "deep"), { recursive: true });
    for (let index = 0; index < 20; index += 1) {
      await writeFile(join(session, "api", `f${index}.py`), "seed\n");
      await writeFile(join(session, "api", "deep", `g${index}.py`), "seed\n");
    }
    setUpTestRepository(session, { library: [file(needle("test:session", "SESSION"))] }, databasePath);
    const { recorder, audits } = recordingAudits();
    const bridge = new CodeRuleBridge(session, router(session, databasePath, recorder, undefined, true));
    cleanups.push(() => bridge.close());
    await bridge.openWorkspace("walk");
    await bridge.checkpoint("turn");
    // Each lookup resolves the path's real path and looks for `.git` on disk; a walk asked two of each file.
    const lookups = vi.spyOn(CheckoutOwners.prototype, "ownerOf");
    cleanups.push(async () => { lookups.mockRestore(); });
    await bridge.checkpoint("turn");
    // The session's folder, api and api/deep; and, once a pass whatever the files, the checkout that owns the session
    // folder and the one that owns the `.git` entry the walk passed, whose unmerged entries each pass reads.
    expect(lookups.mock.calls.length).toBeLessThanOrEqual(5);

    lookups.mockRestore();
    // A command writes it: what is stamped while one of the session's calls runs is the session's own.
    await bridge.begin("write-1", "shell", { command: "./write.sh" });
    await writeFile(join(session, "api", "deep", "g3.py"), "NEEDLE\n");
    await bridge.finish("write-1");
    await bridge.checkpoint("turn");
    await bridge.settle();
    expect(violations(audits)).toEqual([{ checkout: session, path: "api/deep/g3.py", code: "SESSION", mode: "block" }]);
  });

  it("answers a walk's questions as it answers each path alone, across nested checkouts and ignored folders", async () => {
    const root = await temporaryRoot();
    const databasePath = join(root, "rerail.sqlite");
    const { parent, sub } = await withSubmodule(root);
    await writeFile(join(parent, ".gitignore"), "scratch/\n*.log\n");
    await mkdir(join(parent, "scratch"));
    await mkdir(join(sub, "api"), { recursive: true });
    setUpTestRepository(parent, { library: [file(needle("test:parent", "PARENT"))] }, databasePath);
    setUpTestRepository(sub, { library: [file(needle("test:sub", "SUB"))] }, databasePath);
    const { recorder } = recordingAudits();
    const auditor = router(parent, databasePath, recorder, undefined, true);
    await auditor.openWorkspace("view");
    const view = auditor.walkView();

    const folders = ["api", "scratch", "sub", "sub/api"];
    const files = ["README.md", "run.log", "api/a.py", "scratch/x.py", "sub/README.md", "sub/api/b.py"];
    const answers = (judge: Pick<CheckoutCodeRuleRouter, "acceptsPath" | "ignoresPath">) => ({
      folders: folders.map(folder => judge.ignoresPath(folder, true)),
      files: files.map(path => [judge.ignoresPath(path, false), judge.acceptsPath(path)]),
    });
    expect(answers(view)).toEqual({
      folders: [false, true, false, false],
      files: [[false, false], [true, false], [false, true], [true, false], [false, false], [false, true]],
    });
    expect(answers(view)).toEqual(answers(auditor));
  });

  it("names the checkout that owns a `.git` entry as it does for the files beside it: a submodule's file, the session checkout's folder", async () => {
    const root = await temporaryRoot();
    const databasePath = join(root, "rerail.sqlite");
    const { parent, sub } = await withSubmodule(root);
    setUpTestRepository(parent, { library: [file(needle("test:parent", "PARENT"))] }, databasePath);
    setUpTestRepository(sub, { library: [file(needle("test:sub", "SUB"))] }, databasePath);
    const auditor = router(parent, databasePath, recordingAudits().recorder);

    await auditor.openWorkspace("git-entries");
    expect(auditor.sourceRoot("sub/.git")).toBe(sub);
    expect(auditor.sourceRoot(".git")).toBe(parent);
  });

  it("judges a submodule's file by no rules while the submodule's repository is not set up", async () => {
    const root = await temporaryRoot();
    const databasePath = join(root, "rerail.sqlite");
    const { parent, sub } = await withSubmodule(root);
    setUpTestRepository(parent, { library: [file(needle("test:parent", "PARENT"))] }, databasePath);
    const database = openRerailDatabase(databasePath);
    try {
      // Recorded and started through its parent, but its own repository was never set up.
      recordCheckout(database, sub);
    } finally {
      database.$client.close();
    }
    const { recorder, audits } = recordingAudits();
    const auditor = router(parent, databasePath, recorder);

    await auditor.openWorkspace("submodule-not-set-up");
    expect(auditor.acceptsPath("sub/api/b.py")).toBe(false);
    await edit(auditor, "submodule-not-set-up", 1, "sub/api/b.py");

    expect(violations(audits)).toEqual([]);
  });

  it("judges a sibling repository's file, named by absolute path, by the sibling's rules while it is eligible", async () => {
    const root = await temporaryRoot();
    const databasePath = join(root, "rerail.sqlite");
    const session = await repository(join(root, "session"));
    const sibling = await repository(join(root, "sibling"));
    setUpTestRepository(session, { library: [file(needle("test:session", "SESSION"))] }, databasePath);
    setUpTestRepository(sibling, { library: [file(needle("test:sibling", "SIBLING"))] }, databasePath);
    const { recorder, audits } = recordingAudits();
    const auditor = router(session, databasePath, recorder);

    await auditor.openWorkspace("sibling");
    const absolute = join(sibling, "api", "x.py");
    expect(auditor.acceptsPath(absolute)).toBe(true);
    await edit(auditor, "sibling", 1, absolute);

    expect(violations(audits)).toEqual([{ checkout: sibling, path: "api/x.py", code: "SIBLING", mode: "block" }]);
  });

  it("ignores a sibling repository's file while that checkout is stopped", async () => {
    const root = await temporaryRoot();
    const databasePath = join(root, "rerail.sqlite");
    const session = await repository(join(root, "session"));
    const sibling = await repository(join(root, "sibling"));
    setUpTestRepository(session, { library: [file(needle("test:session", "SESSION"))] }, databasePath);
    setUpTestRepository(sibling, { library: [file(needle("test:sibling", "SIBLING"))] }, databasePath);
    stopTestRepository(sibling, databasePath);
    const { recorder, audits } = recordingAudits();
    const auditor = router(session, databasePath, recorder);

    await auditor.openWorkspace("stopped-sibling");
    const absolute = join(sibling, "api", "x.py");
    expect(auditor.acceptsPath(absolute)).toBe(false);
    await edit(auditor, "stopped-sibling", 1, absolute);

    expect(violations(audits)).toEqual([]);
  });

  it("rejects a symlink whose target leaves its checkout", async context => {
    const root = await temporaryRoot();
    const databasePath = join(root, "rerail.sqlite");
    const session = await repository(join(root, "session"));
    const sibling = await repository(join(root, "sibling"));
    setUpTestRepository(session, { library: [file(needle("test:session", "SESSION"))] }, databasePath);
    setUpTestRepository(sibling, { library: [file(needle("test:sibling", "SIBLING"))] }, databasePath);
    await writeFile(join(sibling, "api", "x.py"), "NEEDLE\n");
    symlinkOrSkip(context, join(sibling, "api", "x.py"), join(session, "api", "escape.py"));
    const { recorder, audits } = recordingAudits();
    const auditor = router(session, databasePath, recorder);

    await auditor.openWorkspace("symlink");
    expect(auditor.acceptsPath("api/escape.py")).toBe(false);
    await edit(auditor, "symlink", 1, "api/escape.py");

    expect(violations(audits)).toEqual([]);
  });

  it("interrupts the session's earlier runs once, so opening a second lane leaves the first lane's runs alone", async () => {
    const root = await temporaryRoot();
    const databasePath = join(root, "rerail.sqlite");
    const session = await repository(join(root, "session"));
    const sibling = await repository(join(root, "sibling"));
    setUpTestRepository(session, { library: [file(needle("test:session", "SESSION"))] }, databasePath);
    setUpTestRepository(sibling, { library: [file(needle("test:sibling", "SIBLING"))] }, databasePath);
    const { recorder, audits, interrupts } = recordingAudits();
    const auditor = router(session, databasePath, recorder);

    await auditor.openWorkspace("lanes");
    expect(interrupts).toEqual(["lanes"]);
    // The first lane's edit is still queued when the sibling's edit opens the second lane.
    await auditor.auditGeneratedEdits("lanes", {
      revision: 1, checkpoints: ["edit"], edits: [{ path: "api/a.py", language: "python", generatedText: "NEEDLE\n" }],
    });
    await edit(auditor, "lanes", 2, join(sibling, "api", "b.py"));

    expect(interrupts).toEqual(["lanes"]);
    expect(audits.filter(audit => audit.workspaceRoot === session && audit.paths.includes("api/a.py")).map(audit => audit.result?.status))
      .toEqual(["completed"]);
    expect(violations(audits).map(violation => violation.code).toSorted()).toEqual(["SESSION", "SIBLING"]);
  });

  it("gives each folder the mode its placement decides: a checkout's Log in one folder, the repository's Steer elsewhere", async () => {
    const root = await temporaryRoot();
    const databasePath = join(root, "rerail.sqlite");
    const session = await repository(join(root, "session"));
    const repositoryKey = setUpTestRepository(session, { library: [file(needle("test:placed", "PLACED", "block"))] }, databasePath);
    const database = openRerailDatabase(databasePath);
    try {
      setRuleSetting(database, { owner: { kind: "repository", repositoryKey }, folder: "", targetKind: "rule", targetId: "test:placed", on: null, mode: "steer" });
      setRuleSetting(database, { owner: { kind: "checkout", checkoutPath: session }, folder: "web", targetKind: "rule", targetId: "test:placed", on: null, mode: "log" });
    } finally {
      database.$client.close();
    }
    const { recorder, audits } = recordingAudits();
    const auditor = router(session, databasePath, recorder);

    await auditor.openWorkspace("placement");
    await edit(auditor, "placement", 1, "api/a.py", "web/b.py");

    expect(violations(audits).toSorted((a, b) => a.path!.localeCompare(b.path!))).toEqual([
      { checkout: session, path: "api/a.py", code: "PLACED", mode: "steer" },
      { checkout: session, path: "web/b.py", code: "PLACED", mode: "log" },
    ]);
  });

  it("judges files under a repository an agent created inside the checkout, or under a stray .git, by the checkout's rules", async () => {
    const root = await temporaryRoot();
    const databasePath = join(root, "rerail.sqlite");
    const session = await repository(join(root, "session"));
    setUpTestRepository(session, { library: [file(needle("test:session", "SESSION"))] }, databasePath);
    // `git init src` makes an unrecorded repository nobody set up; `lib/.git` is an empty file Git does not confirm.
    await repository(join(session, "src"));
    await mkdir(join(session, "lib"));
    await writeFile(join(session, "lib", ".git"), "");
    const { recorder, audits } = recordingAudits();
    const auditor = router(session, databasePath, recorder);

    await auditor.openWorkspace("dodge");
    expect(auditor.acceptsPath("src/api/x.py")).toBe(true);
    expect(auditor.acceptsPath("lib/y.py")).toBe(true);
    await edit(auditor, "dodge", 1, "src/api/x.py", "lib/y.py");

    expect(violations(audits).toSorted((a, b) => a.path!.localeCompare(b.path!))).toEqual([
      { checkout: session, path: "lib/y.py", code: "SESSION", mode: "block" },
      { checkout: session, path: "src/api/x.py", code: "SESSION", mode: "block" },
    ]);
  });

  it("judges a stopped nested worktree's files, and a recorded nested repository that is not set up, by no rules", async () => {
    const root = await temporaryRoot();
    const databasePath = join(root, "rerail.sqlite");
    const session = await repository(join(root, "session"));
    setUpTestRepository(session, { library: [file(needle("test:session", "SESSION"))] }, databasePath);
    // A worktree inside the checkout, as Claude Code keeps them, stopped on its own.
    git(session, "worktree", "add", "--quiet", "-b", "nested", join(session, ".claude", "worktrees", "w"));
    const worktree = await realpath(join(session, ".claude", "worktrees", "w"));
    // A repository Rerail recorded inside the checkout, started but never set up.
    const vendored = await repository(join(session, "vendored"));
    const database = openRerailDatabase(databasePath);
    try {
      setCheckoutStarted(database, recordCheckout(database, worktree), false);
      recordCheckout(database, vendored);
    } finally {
      database.$client.close();
    }
    const { recorder, audits } = recordingAudits();
    const auditor = router(session, databasePath, recorder);

    await auditor.openWorkspace("nested");
    expect(auditor.acceptsPath(".claude/worktrees/w/api/x.py")).toBe(false);
    expect(auditor.acceptsPath("vendored/api/y.py")).toBe(false);
    expect(auditor.acceptsPath("api/z.py")).toBe(true);
    await edit(auditor, "nested", 1, ".claude/worktrees/w/api/x.py", "vendored/api/y.py", "api/z.py");

    expect(violations(audits)).toEqual([{ checkout: session, path: "api/z.py", code: "SESSION", mode: "block" }]);
  });

  it("rejects a parent's file that is a symlink into a nested checkout, which the parent's rules would otherwise judge", async context => {
    const root = await temporaryRoot();
    const databasePath = join(root, "rerail.sqlite");
    const { parent, sub } = await withSubmodule(root);
    setUpTestRepository(parent, { library: [file(needle("test:parent", "PARENT"))] }, databasePath);
    setUpTestRepository(sub, { library: [file(needle("test:sub", "SUB"))] }, databasePath);
    await mkdir(join(sub, "api"), { recursive: true });
    await writeFile(join(sub, "api", "b.py"), "NEEDLE\n");
    // Lexically `api/link.py` is the parent's; really it is the submodule's `api/b.py`.
    symlinkOrSkip(context, join(sub, "api", "b.py"), join(parent, "api", "link.py"));
    const { recorder, audits } = recordingAudits();
    const auditor = router(parent, databasePath, recorder);

    await auditor.openWorkspace("symlinked-submodule");
    expect(auditor.acceptsPath("api/link.py")).toBe(false);
    expect(auditor.acceptsPath("sub/api/b.py")).toBe(true);
    await edit(auditor, "symlinked-submodule", 1, "api/link.py");

    expect(violations(audits)).toEqual([]);
  });
});

/** Whether this filesystem matches names case-insensitively, as macOS does by default. */
function caseInsensitiveFilesystem(): boolean {
  const probe = mkdtempSync(join(tmpdir(), "rerail-case-"));
  try {
    return existsSync(probe.toUpperCase()) && existsSync(probe.toLowerCase());
  } finally {
    removeFolderSync(probe);
  }
}

describe("CheckoutCodeRuleRouter and linked worktrees", () => {
  function recordedCheckouts(databasePath: string): string[] {
    const database = openRerailDatabase(databasePath);
    try {
      return database.select({ path: checkouts.checkoutPath }).from(checkouts).all().map(row => row.path).toSorted();
    } finally {
      database.$client.close();
    }
  }

  /** A set-up, started repository with a linked worktree; the worktree is not recorded, as no session started there. */
  async function repositoryWithWorktree(place: (root: string, session: string) => string) {
    const root = await temporaryRoot();
    const databasePath = join(root, "rerail.sqlite");
    const session = await repository(join(root, "session"));
    setUpTestRepository(session, { library: [file(needle("test:session", "SESSION"))] }, databasePath);
    git(session, "worktree", "add", "--quiet", "-b", "linked", place(root, session));
    return { root, databasePath, session, worktree: await realpath(place(root, session)) };
  }

  /** `path` in `worktree` as a walk of the session's folder names it: relative to the session folder, with `/`. */
  const walked = (session: string, worktree: string, path: string) => relative(session, join(worktree, path)).split(sep).join("/");

  const nested = (_root: string, session: string) => join(session, ".claude", "worktrees", "w");
  const outside = (root: string) => join(root, "elsewhere", "w");

  it("judges a worktree inside the checkout by its own lane, though the checkout around it ignores the folder", async () => {
    const { databasePath, session, worktree } = await repositoryWithWorktree(nested);
    await writeFile(join(session, ".git", "info", "exclude"), ".claude/worktrees/\n");
    const { recorder, audits } = recordingAudits();
    const auditor = router(session, databasePath, recorder, undefined, true);

    await auditor.openWorkspace("nested-worktree");
    expect(auditor.acceptsPath(".claude/worktrees/w/api/x.py")).toBe(true);
    await edit(auditor, "nested-worktree", 1, ".claude/worktrees/w/api/x.py");

    expect(violations(audits)).toEqual([{ checkout: worktree, path: "api/x.py", code: "SESSION", mode: "block" }]);
    expect(recordedCheckouts(databasePath)).toEqual([session, worktree]);
  });

  it("judges a worktree outside the checkout by its own lane", async () => {
    const { databasePath, session, worktree } = await repositoryWithWorktree(outside);
    const { recorder, audits } = recordingAudits();
    const auditor = router(session, databasePath, recorder);

    await auditor.openWorkspace("outside-worktree");
    const absolute = join(worktree, "api", "x.py");
    expect(auditor.acceptsPath(absolute)).toBe(true);
    await edit(auditor, "outside-worktree", 1, absolute);

    expect(violations(audits)).toEqual([{ checkout: worktree, path: "api/x.py", code: "SESSION", mode: "block" }]);
  });

  it("answers a walk's questions about a worktree with the session checkout's rules without opening a lane there", async () => {
    const { databasePath, session, worktree } = await repositoryWithWorktree(outside);
    await mkdir(join(worktree, "api"), { recursive: true });
    await writeFile(join(worktree, "api", "x.py"), "seed\n");
    const { recorder, audits } = recordingAudits();
    const auditor = router(session, databasePath, recorder, undefined, true);
    const opened = vi.spyOn(NativeCodeRuleEnforcer.prototype, "openWorkspace");
    cleanups.push(async () => { opened.mockRestore(); });

    await auditor.openWorkspace("worktree-questions");
    const view = auditor.walkView();
    const asked = (path: string) => join(worktree, path);
    const named = (path: string) => walked(session, worktree, path);
    expect([view.ignoresPath(named("api"), true), view.acceptsPath(named("api/x.py")), view.acceptsPath(named("README.md"))]).toEqual([false, true, false]);
    expect([auditor.acceptsPath(asked("api/x.py")), auditor.acceptsPath(asked("README.md"))]).toEqual([true, false]);
    // A lane holds a native workspace and the checkout's index: only the session's own opened.
    expect(opened).toHaveBeenCalledTimes(1);

    // An edit there is judged by the worktree's own lane, which opens for it.
    await edit(auditor, "worktree-questions", 1, asked("api/x.py"));
    expect(violations(audits)).toEqual([{ checkout: worktree, path: "api/x.py", code: "SESSION", mode: "block" }]);
    expect(opened).toHaveBeenCalledTimes(2);
  });

  it("answers a walk's questions about a worktree with rules of its own by them, opening its lane", async () => {
    const { databasePath, session, worktree } = await repositoryWithWorktree(outside);
    // The worktree's own rules (`.rerail/rules`) read Markdown files too, which the session checkout's do not.
    await mkdir(join(worktree, ".rerail", "rules"), { recursive: true });
    await writeFile(join(worktree, ".rerail", "rules", "notes.json"), JSON.stringify({ ...needle("worktree:notes", "NOTES"), scope: { include: ["**/*.md"], exclude: [] } }));
    const { recorder } = recordingAudits();
    const auditor = router(session, databasePath, recorder, undefined, true);
    const opened = vi.spyOn(NativeCodeRuleEnforcer.prototype, "openWorkspace");
    cleanups.push(async () => { opened.mockRestore(); });

    await auditor.openWorkspace("own-rules");
    const view = auditor.walkView();
    expect([view.acceptsPath("README.md"), view.acceptsPath(walked(session, worktree, "README.md")), view.acceptsPath(walked(session, worktree, "api/x.py"))]).toEqual([false, true, true]);
    expect(opened).toHaveBeenCalledTimes(2);
  });

  it("holds one Git ignore matcher for a worktree whether a walk's question or an edit reached it first", async () => {
    const { databasePath, session, worktree } = await repositoryWithWorktree(outside);
    await mkdir(join(worktree, "api"), { recursive: true });
    const { recorder, audits } = recordingAudits();
    const auditor = router(session, databasePath, recorder, undefined, true);
    const opened = vi.spyOn(nativeGitAccess(), "openGitIgnoreMatcher");
    cleanups.push(async () => { opened.mockRestore(); });

    await auditor.openWorkspace("one-matcher");
    expect(auditor.walkView().ignoresPath(walked(session, worktree, "api"), true)).toBe(false);
    await edit(auditor, "one-matcher", 1, join(worktree, "api", "x.py"));
    expect(violations(audits)).toEqual([{ checkout: worktree, path: "api/x.py", code: "SESSION", mode: "block" }]);
    // A matcher holds its checkout's index: the lane the edit opened takes the one the question opened.
    expect(opened.mock.calls.filter(([root]) => root === worktree)).toHaveLength(1);
  });

  it("keeps the daemon's matcher its own lane took once a walk's question reaches a worktree with no lane", async () => {
    const { databasePath, session, worktree } = await repositoryWithWorktree(outside);
    const sessionRoot = await realpath(session);
    for (const checkout of [session, worktree]) await mkdir(join(checkout, "api"), { recursive: true });
    const matchers = new GitIgnoreMatchers();
    const opened = vi.spyOn(nativeGitAccess(), "openGitIgnoreMatcher");
    cleanups.push(async () => { opened.mockRestore(); });
    const opensIn = (checkout: string) => opened.mock.calls.filter(([root]) => root === checkout).length;
    const { recorder } = recordingAudits();
    const first = router(session, databasePath, recorder, undefined, true, undefined, matchers);
    await first.openWorkspace("lane-first");
    // The session checkout's own lane answers there and takes the daemon's matcher; then the walk reaches the worktree.
    expect(first.walkView().ignoresPath("api", true)).toBe(false);
    expect(first.walkView().ignoresPath(walked(session, worktree, "api"), true)).toBe(false);

    const second = router(session, databasePath, recorder, undefined, true, undefined, matchers);
    await second.openWorkspace("lane-first-second");
    expect(second.walkView().ignoresPath("api", true)).toBe(false);
    // One matcher per checkout for both sessions: the walk's question let go of none the lane took.
    expect([opensIn(sessionRoot), opensIn(worktree)]).toEqual([1, 1]);
  });

  it("answers a walk's questions about a worktree another lane answers for by that worktree's own Git ignore rules", async () => {
    const { databasePath, session, worktree } = await repositoryWithWorktree(outside);
    // Only the worktree ignores gen/: its .gitignore is not the session checkout's.
    await writeFile(join(worktree, ".gitignore"), "gen/\n");
    for (const checkout of [session, worktree]) {
      await mkdir(join(checkout, "gen"), { recursive: true });
      await writeFile(join(checkout, "gen", "x.py"), "seed\n");
    }
    const { recorder } = recordingAudits();
    const auditor = router(session, databasePath, recorder, undefined, true);
    await auditor.openWorkspace("worktree-ignores");
    const view = auditor.walkView();
    expect([view.acceptsPath("gen/x.py"), view.acceptsPath(walked(session, worktree, "gen/x.py"))]).toEqual([true, false]);
  });

  it("reads again the rules of a worktree another lane answers for, and answers by its own once they differ", async () => {
    const { databasePath, session, worktree } = await repositoryWithWorktree(outside);
    const { recorder } = recordingAudits();
    const auditor = router(session, databasePath, recorder, undefined, true);
    await auditor.openWorkspace("worktree-rules-later");
    const readme = walked(session, worktree, "README.md");
    expect(auditor.walkView().acceptsPath(readme)).toBe(false);

    // The worktree takes rules of its own that read Markdown, which the session checkout's do not.
    await mkdir(join(worktree, ".rerail", "rules"), { recursive: true });
    await writeFile(join(worktree, ".rerail", "rules", "notes.json"), JSON.stringify({ ...needle("worktree:notes", "NOTES"), scope: { include: ["**/*.md"], exclude: [] } }));
    auditor.refreshCodeRules();
    expect(auditor.walkView().acceptsPath(readme)).toBe(true);
  });

  it("stops answering for a worktree another lane answered for once that worktree is stopped", async () => {
    const { databasePath, session, worktree } = await repositoryWithWorktree(outside);
    await mkdir(join(worktree, "api"), { recursive: true });
    await writeFile(join(worktree, "api", "x.py"), "seed\n");
    const { recorder } = recordingAudits();
    const auditor = router(session, databasePath, recorder, undefined, true);
    await auditor.openWorkspace("worktree-stopped-later");
    const file = walked(session, worktree, "api/x.py");
    expect(auditor.walkView().acceptsPath(file)).toBe(true);

    const database = openRerailDatabase(databasePath);
    try {
      setCheckoutStarted(database, recordCheckout(database, worktree), false);
    } finally {
      database.$client.close();
    }
    auditor.refreshCodeRules();
    expect(auditor.walkView().acceptsPath(file)).toBe(false);
  });

  it("answers a question after its close with a Git ignore matcher of its own, leaving the daemon's shared one dropped", async () => {
    const { databasePath, session, worktree } = await repositoryWithWorktree(outside);
    await mkdir(join(worktree, "api"), { recursive: true });
    const matchers = new GitIgnoreMatchers();
    const opened = vi.spyOn(nativeGitAccess(), "openGitIgnoreMatcher");
    cleanups.push(async () => { opened.mockRestore(); });
    const opensIn = () => opened.mock.calls.filter(([root]) => root === worktree).length;
    const { recorder } = recordingAudits();
    const auditor = router(session, databasePath, recorder, undefined, true, undefined, matchers);
    await auditor.openWorkspace("late-question");
    const view = auditor.walkView();
    const api = walked(session, worktree, "api");
    expect(view.ignoresPath(api, true)).toBe(false);
    expect(opensIn()).toBe(1);

    await auditor.close();
    expect(matchers.size).toBe(0);
    // A walk still running asks once more: the router no longer holds the shared matcher, nor takes it again.
    expect(view.ignoresPath(api, true)).toBe(false);
    expect([opensIn(), matchers.size]).toEqual([2, 0]);
  });

  /** A worktree whose own `.gitignore` ignores `gen/`, which holds a Python file, and the router of a session beside it. */
  async function worktreeIgnoringGen(id: string) {
    const { databasePath, session, worktree } = await repositoryWithWorktree(outside);
    await writeFile(join(worktree, ".gitignore"), "gen/\n");
    await mkdir(join(worktree, "gen"), { recursive: true });
    await writeFile(join(worktree, "gen", "x.py"), "seed\n");
    const { recorder, audits } = recordingAudits();
    const auditor = router(session, databasePath, recorder, undefined, true);
    await auditor.openWorkspace(id);
    return { databasePath, session, worktree, audits, auditor, gen: walked(session, worktree, "gen") };
  }

  function setStarted(databasePath: string, checkout: string, started: boolean): void {
    const database = openRerailDatabase(databasePath);
    try {
      setCheckoutStarted(database, recordCheckout(database, checkout), started);
    } finally {
      database.$client.close();
    }
  }

  it("gives a worktree's lane, as it opens, Git's ignore rules as they are then, not those a walk's question read before they changed", async () => {
    const { worktree, audits, auditor, gen } = await worktreeIgnoringGen("lane-reads-now");
    expect(auditor.walkView().ignoresPath(gen, true)).toBe(true);

    await writeFile(join(worktree, ".gitignore"), "other/\n");
    // The edit opens the worktree's lane, which reads Git's ignore rules now, as a lane that opened its own did.
    await edit(auditor, "lane-reads-now", 1, join(worktree, "gen", "x.py"));
    expect(violations(audits)).toEqual([{ checkout: worktree, path: "gen/x.py", code: "SESSION", mode: "block" }]);
    expect(auditor.ignoredEdit(join(worktree, "gen", "x.py"))).toBeUndefined();
  });

  it("reads a worktree's Git ignore rules again once it is started after a stop, as its new lane did", async () => {
    const { databasePath, worktree, auditor, gen } = await worktreeIgnoringGen("started-again");
    const view = () => auditor.walkView();
    expect([view().ignoresPath(gen, true), view().acceptsPath(`${gen}/x.py`)]).toEqual([true, false]);

    setStarted(databasePath, worktree, false);
    auditor.refreshCodeRules();
    await writeFile(join(worktree, ".gitignore"), "other/\n");
    setStarted(databasePath, worktree, true);
    auditor.refreshCodeRules();
    expect([view().ignoresPath(gen, true), view().acceptsPath(`${gen}/x.py`)]).toEqual([false, true]);
  });

  it("lets go of the daemon's matcher of a worktree it drops once that worktree stops, which another router holding it keeps", async () => {
    const { databasePath, session, worktree } = await repositoryWithWorktree(outside);
    await mkdir(join(worktree, "api"), { recursive: true });
    const matchers = new GitIgnoreMatchers();
    const opened = vi.spyOn(nativeGitAccess(), "openGitIgnoreMatcher");
    cleanups.push(async () => { opened.mockRestore(); });
    const { recorder } = recordingAudits();
    const first = router(session, databasePath, recorder, undefined, true, undefined, matchers);
    const second = router(session, databasePath, recorder, undefined, true, undefined, matchers);
    await first.openWorkspace("let-go-first");
    await second.openWorkspace("let-go-second");
    const api = walked(session, worktree, "api");
    expect([first.walkView().ignoresPath(api, true), second.walkView().ignoresPath(api, true)]).toEqual([false, false]);
    expect(opened.mock.calls.filter(([root]) => root === worktree)).toHaveLength(1);
    const held = matchers.size;

    setStarted(databasePath, worktree, false);
    first.refreshCodeRules();
    expect(matchers.size).toBe(held);
    // The last router that held the worktree's matcher drops it: the daemon holds it, and its index, no longer.
    second.refreshCodeRules();
    expect(matchers.size).toBe(held - 1);
    setStarted(databasePath, worktree, true);
    second.refreshCodeRules();
    expect(second.walkView().ignoresPath(api, true)).toBe(false);
    expect(opened.mock.calls.filter(([root]) => root === worktree)).toHaveLength(2);
  });

  it("lets go of the daemon's matchers it holds once the process's Git owner changes, which another router holding them keeps until it asks too", async () => {
    const { databasePath, session, worktree } = await repositoryWithWorktree(outside);
    await mkdir(join(worktree, "api"), { recursive: true });
    const matchers = new GitIgnoreMatchers();
    const { recorder } = recordingAudits();
    const first = router(session, databasePath, recorder, undefined, true, undefined, matchers);
    const second = router(session, databasePath, recorder, undefined, true, undefined, matchers);
    await first.openWorkspace("owner-change-first");
    await second.openWorkspace("owner-change-second");
    const api = walked(session, worktree, "api");
    expect([first.walkView().ignoresPath(api, true), second.walkView().ignoresPath(api, true)]).toEqual([false, false]);
    const held = matchers.size;

    // Git answered at last: the process has a new owner of Git reads, which opens matchers as the old one did.
    const old = nativeGitAccess();
    swappedOwner.current = {
      systemConfig: old.systemConfig,
      installationConfig: old.installationConfig,
      openGitIgnoreMatcher: root => old.openGitIgnoreMatcher(root),
    };
    cleanups.push(async () => { swappedOwner.current = undefined; });
    // Each router asks only about the session's checkout, through its lane, which takes the new owner's matcher: the
    // worktree's old one goes once neither router holds it, not because one asked about the worktree again.
    expect(first.ignoresPath("api", true)).toBe(false);
    expect(matchers.size).toBe(held + 1);
    expect(second.ignoresPath("api", true)).toBe(false);
    expect(matchers.size).toBe(1);
  });

  it("keeps a worktree's refused Git ignore rules as none for the walk's questions and says it once, while its lane opens them as it opens", async () => {
    const { worktree, audits, auditor, gen } = await worktreeIgnoringGen("open-again");
    const access = nativeGitAccess();
    const open = access.openGitIgnoreMatcher.bind(access);
    let failures = 1;
    const opens = vi.spyOn(access, "openGitIgnoreMatcher").mockImplementation(root => {
      if (root !== worktree || failures === 0) return open(root);
      failures -= 1;
      throw new Error("cannot read Git excludes: the configuration was being written");
    });
    cleanups.push(async () => { opens.mockRestore(); });
    const errors = vi.spyOn(process.stderr, "write").mockImplementation(() => true);
    cleanups.push(async () => { errors.mockRestore(); });

    // The walk's questions find no rules and leave the folder in, as rules Git refuses do, under the owner that tried.
    expect([auditor.walkView().ignoresPath(gen, true), auditor.walkView().ignoresPath(gen, true)]).toEqual([false, false]);
    // The edit opens the worktree's lane, which opens the rules as it opens and leaves the ignored file unchecked.
    await edit(auditor, "open-again", 1, join(worktree, "gen", "x.py"));
    expect(violations(audits)).toEqual([]);
    expect(errors.mock.calls.filter(([text]) => String(text).includes("cannot read Git ignore rules"))).toHaveLength(1);
  });

  /**
   * Has `root`'s rules fail to load while `failing()` says so, as a rule the catalog cannot represent makes them: its
   * catalog's first read throws (no catalog is made), and a catalog made before throws as it reads them again.
   */
  function failingCatalogOf(root: string, failing: () => boolean): void {
    const catalogOf = CheckoutCatalogs.prototype.catalog;
    const ofRoot = new Set<CheckoutCodeRuleCatalog>();
    const catalogs = vi.spyOn(CheckoutCatalogs.prototype, "catalog").mockImplementation(function (this: CheckoutCatalogs, asked: string) {
      if (asked === root && failing() && ofRoot.size === 0) throw new Error("Policy worktree:notes is not JSON representable");
      const catalog = catalogOf.call(this, asked);
      if (asked === root) ofRoot.add(catalog);
      return catalog;
    });
    const refresh = CheckoutCodeRuleCatalog.prototype.refresh;
    const refreshes = vi.spyOn(CheckoutCodeRuleCatalog.prototype, "refresh").mockImplementation(function (this: CheckoutCodeRuleCatalog) {
      if (ofRoot.has(this) && failing()) throw new Error("Policy worktree:notes is not JSON representable");
      return refresh.call(this);
    });
    cleanups.push(async () => { catalogs.mockRestore(); refreshes.mockRestore(); });
  }

  /** The session checkout takes a rule that reads Markdown, so a lane that read its rules again accepts README.md. */
  async function sessionReadsMarkdown(session: string): Promise<void> {
    await mkdir(join(session, ".rerail", "rules"), { recursive: true });
    await writeFile(join(session, ".rerail", "rules", "notes.json"), JSON.stringify({ ...needle("session:notes", "NOTES"), scope: { include: ["**/*.md"], exclude: [] } }));
  }

  it("refreshes every lane though the rules of a worktree another lane answers for fail to load, and records the failure as that worktree's lane", async () => {
    const { databasePath, session, worktree } = await repositoryWithWorktree(outside);
    await mkdir(join(worktree, "api"), { recursive: true });
    await writeFile(join(worktree, "api", "x.py"), "seed\n");
    let broken = false;
    failingCatalogOf(worktree, () => broken);
    const { recorder } = recordingAudits();
    const auditor = router(session, databasePath, recorder, undefined, true);
    await auditor.openWorkspace("answered-fails");
    expect(auditor.walkView().acceptsPath(walked(session, worktree, "api/x.py"))).toBe(true);

    broken = true;
    await sessionReadsMarkdown(session);
    expect(() => auditor.refreshCodeRules()).not.toThrow();
    expect(auditor.acceptsPath("README.md")).toBe(true);
    const edits = [{ path: "api/x.py", language: "python" as const, generatedText: "seed\n" }];
    const response = await auditor.auditGeneratedEdits("answered-fails", { revision: 1, checkpoints: ["edit"], edits });
    // The session's lane installs its new rules before the router closes, so no store of theirs is still opening then.
    await auditor.settleGeneratedEdits();
    expect(response.failures).toContainEqual(expect.objectContaining({ clauseId: "catalog", code: "adapter-error", message: expect.stringContaining("not JSON representable") }));
  });

  it("answers a walk by a worktree's last rules once they fail to load at an event that changed no rule, and records the failure once as that worktree's lane", async () => {
    const { databasePath, session, worktree } = await repositoryWithWorktree(outside);
    await mkdir(join(worktree, "api"), { recursive: true });
    await writeFile(join(worktree, "api", "x.py"), "seed\n");
    let broken = false;
    failingCatalogOf(worktree, () => broken);
    const { recorder } = recordingAudits();
    const auditor = router(session, databasePath, recorder, undefined, true);
    await auditor.openWorkspace("answered-fails-quiet");
    const file = walked(session, worktree, "api/x.py");
    expect(auditor.walkView().acceptsPath(file)).toBe(true);
    // The session's first event reads every rule configuration; the next finds none changed, so only the walk's next
    // question reads the worktree's rules again.
    auditor.refreshCodeRules();

    broken = true;
    auditor.refreshCodeRules();
    const view = auditor.walkView();
    expect([view.acceptsPath(file), view.acceptsPath(file), view.acceptsPath(file)]).toEqual([true, true, true]);
    const edits = [{ path: "api/x.py", language: "python" as const, generatedText: "seed\n" }];
    const response = await auditor.auditGeneratedEdits("answered-fails-quiet", { revision: 1, checkpoints: ["edit"], edits });
    await auditor.settleGeneratedEdits();
    expect(response.failures.filter(failure => failure.clauseId === "catalog")).toEqual([
      expect.objectContaining({ code: "adapter-error", message: expect.stringContaining("not JSON representable") }),
    ]);
  });

  it("refreshes every lane after a walk asked about a worktree whose rules could not be read at all", async () => {
    const { databasePath, session, worktree } = await repositoryWithWorktree(outside);
    let broken = true;
    failingCatalogOf(worktree, () => broken);
    const { recorder } = recordingAudits();
    const auditor = router(session, databasePath, recorder, undefined, true);
    await auditor.openWorkspace("never-read");
    // As when a lane opened for the question: the catalog's error reaches the walk.
    expect(() => auditor.walkView().acceptsPath(walked(session, worktree, "api/x.py"))).toThrow("not JSON representable");

    await sessionReadsMarkdown(session);
    expect(() => auditor.refreshCodeRules()).not.toThrow();
    expect(auditor.acceptsPath("README.md")).toBe(true);
    // The session's lane installs its new rules before the router closes, so no store of theirs is still opening then.
    await edit(auditor, "never-read", 1, "api/x.py");
    broken = false;
  });

  it("judges a sibling worktree from a session that started in a worktree", async () => {
    const { databasePath, session, worktree, root } = await repositoryWithWorktree(outside);
    const sibling = join(root, "elsewhere", "sibling");
    git(session, "worktree", "add", "--quiet", "-b", "sibling", sibling);
    const database = openRerailDatabase(databasePath);
    try {
      recordCheckout(database, worktree);
    } finally {
      database.$client.close();
    }
    const { recorder, audits } = recordingAudits();
    const auditor = router(worktree, databasePath, recorder);

    await auditor.openWorkspace("from-worktree");
    await edit(auditor, "from-worktree", 1, join(sibling, "api", "x.py"));

    expect(violations(audits)).toEqual([{ checkout: await realpath(sibling), path: "api/x.py", code: "SESSION", mode: "block" }]);
  });

  it("judges the main checkout from a session in a worktree, when only the worktree was recorded", async () => {
    const root = await temporaryRoot();
    const databasePath = join(root, "rerail.sqlite");
    const main = await repository(join(root, "main"));
    git(main, "worktree", "add", "--quiet", "-b", "linked", join(root, "w"));
    const worktree = await realpath(join(root, "w"));
    setUpTestRepository(worktree, { library: [file(needle("test:session", "SESSION"))] }, databasePath);
    const { recorder, audits } = recordingAudits();
    const auditor = router(worktree, databasePath, recorder);

    await auditor.openWorkspace("main-from-worktree");
    await edit(auditor, "main-from-worktree", 1, join(main, "api", "x.py"));

    expect(violations(audits)).toEqual([{ checkout: main, path: "api/x.py", code: "SESSION", mode: "block" }]);
  });

  it("records a worktree's findings from a Log batch, as another worktree's walk sends them, and steers on none", async () => {
    const { databasePath, session, worktree } = await repositoryWithWorktree(outside);
    const { recorder, audits } = recordingAudits();
    const requestSteer = vi.fn(async () => undefined);
    const auditor = router(session, databasePath, recorder, undefined, false, requestSteer);
    await auditor.openWorkspace("log-batch");
    const edits = [{ path: join(worktree, "api", "x.py"), language: "python" as const, generatedText: "NEEDLE\n" }];

    await auditor.auditGeneratedEdits("log-batch", { revision: 1, checkpoints: ["edit"], mode: "log", edits });
    await auditor.settleGeneratedEdits();
    expect(audits.map(audit => [audit.workspaceRoot, audit.mode])).toEqual([[worktree, "log"]]);
    expect(violations(audits)).toEqual([{ checkout: worktree, path: "api/x.py", code: "SESSION", mode: "block" }]);
    expect(requestSteer).not.toHaveBeenCalled();

    // The same edit as the session's own steers it.
    await auditor.auditGeneratedEdits("log-batch", { revision: 2, checkpoints: ["edit"], edits });
    await auditor.settleGeneratedEdits();
    expect(requestSteer).toHaveBeenCalledTimes(1);
  });

  it("opens one Git ignore matcher for a worktree that sessions in two other worktrees audit, and drops it when both close", async () => {
    const { databasePath, session, worktree, root } = await repositoryWithWorktree(outside);
    git(session, "worktree", "add", "--quiet", "-b", "second", join(root, "elsewhere", "second"));
    const second = await realpath(join(root, "elsewhere", "second"));
    const matchers = new GitIgnoreMatchers();
    const opened = vi.spyOn(nativeGitAccess(), "openGitIgnoreMatcher");
    cleanups.push(async () => { opened.mockRestore(); });
    const { recorder, audits } = recordingAudits();
    const sessions = [
      { id: "first", auditor: router(session, databasePath, recorder, undefined, true, undefined, matchers) },
      { id: "second", auditor: router(second, databasePath, recorder, undefined, true, undefined, matchers) },
    ];
    const edits = [{ path: join(worktree, "api", "x.py"), language: "python" as const, generatedText: "NEEDLE\n" }];
    for (const { id, auditor } of sessions) {
      await auditor.openWorkspace(id);
      await auditor.auditGeneratedEdits(id, { revision: 1, checkpoints: ["edit"], mode: "log", edits });
      await auditor.settleGeneratedEdits();
    }
    expect(violations(audits)).toEqual([
      { checkout: worktree, path: "api/x.py", code: "SESSION", mode: "block" },
      { checkout: worktree, path: "api/x.py", code: "SESSION", mode: "block" },
    ]);
    // A matcher holds its checkout's index, and its answers are the checkout's: the two sessions share one.
    expect(opened.mock.calls.filter(([path]) => path === worktree)).toHaveLength(1);
    for (const { auditor } of sessions) await auditor.close();
    expect(matchers.size).toBe(0);
  });

  it("passes a tool call's wait for the edits that may steer on to each lane's evaluator", async () => {
    const { databasePath, session, worktree } = await repositoryWithWorktree(outside);
    const { recorder } = recordingAudits();
    const auditor = router(session, databasePath, recorder);
    await auditor.openWorkspace("steering-wait");
    const edits = [join(session, "api", "x.py"), join(worktree, "api", "y.py")].map(path => ({ path, language: "python" as const, generatedText: "NEEDLE\n" }));
    await auditor.auditGeneratedEdits("steering-wait", { revision: 1, checkpoints: ["edit"], edits });
    // What each lane's evaluator is asked is the subject, not how long its checks take: on a loaded Windows machine the
    // two lanes' checks outlasted the case's 30 s limit. The router's clean-up ends them after the case.
    const settle = vi.spyOn(NativeCodeRuleEnforcer.prototype, "settleGeneratedEdits").mockResolvedValue(undefined);
    try {
      await auditor.settleGeneratedEdits("steering");
      // A walk's log edits in a lane steer no one: a lane told to wait for every edit would hold the result for them.
      expect(settle.mock.calls).toEqual([["steering"], ["steering"]]);
    } finally {
      settle.mockRestore();
    }
  });

  // In repository scope each lane checks every batch, a walk's `log` batch too, by one audit of the whole repository.
  it("answers that a wait for the edits that may steer waits for a walk's log edits too only while its scope is the repository", async () => {
    const { databasePath, session } = await repositoryWithWorktree(outside);
    const auditor = router(session, databasePath, recordingAudits().recorder);
    const answers = [auditor.steeringWaitsForLog()];
    for (const scope of ["repository", "touched-files"] as const) {
      auditor.updateSettings({ scope, mode: "steer" });
      answers.push(auditor.steeringWaitsForLog());
    }
    expect(answers).toEqual([false, true, false]);
  });

  it("keeps the stopped value a worktree outside the checkout has of its own", async () => {
    const { databasePath, session, worktree } = await repositoryWithWorktree(outside);
    const database = openRerailDatabase(databasePath);
    try {
      setCheckoutStarted(database, recordCheckout(database, worktree), false);
    } finally {
      database.$client.close();
    }
    const { recorder, audits } = recordingAudits();
    const auditor = router(session, databasePath, recorder);

    await auditor.openWorkspace("stopped-worktree");
    const absolute = join(worktree, "api", "x.py");
    expect(auditor.acceptsPath(absolute)).toBe(false);
    await edit(auditor, "stopped-worktree", 1, absolute);

    expect(violations(audits)).toEqual([]);
  });

  it("records nothing for a worktree of a repository that is stopped", async () => {
    const { databasePath, session, worktree } = await repositoryWithWorktree(outside);
    const { recorder, audits } = recordingAudits();
    const auditor = router(session, databasePath, recorder);
    await auditor.openWorkspace("stopped-repository");
    const database = openRerailDatabase(databasePath);
    try {
      setRepositoryStarted(database, recordCheckout(database, session).repositoryKey, false);
    } finally {
      database.$client.close();
    }
    auditor.refreshCodeRules();

    expect(auditor.acceptsPath(join(worktree, "api", "x.py"))).toBe(false);
    expect(violations(audits)).toEqual([]);
    expect(recordedCheckouts(databasePath)).toEqual([session]);
  });

  it("never lets a .git file that names the repository's Git folder take files from the checkout's rules", async () => {
    const { databasePath, session } = await repositoryWithWorktree(outside);
    // Git confirms each as a top level sharing the session's common directory; only `git worktree add` registers one.
    await mkdir(join(session, "forged-main"));
    await writeFile(join(session, "forged-main", ".git"), `gitdir: ${join(session, ".git")}\n`);
    await mkdir(join(session, "forged-linked"));
    await writeFile(join(session, "forged-linked", ".git"), `gitdir: ${join(session, ".git", "worktrees", "w")}\n`);
    const { recorder, audits } = recordingAudits();
    const auditor = router(session, databasePath, recorder);

    await auditor.openWorkspace("forged");
    await edit(auditor, "forged", 1, "forged-main/x.py", "forged-linked/y.py");

    expect(violations(audits).toSorted((a, b) => a.path!.localeCompare(b.path!))).toEqual([
      { checkout: session, path: "forged-linked/y.py", code: "SESSION", mode: "block" },
      { checkout: session, path: "forged-main/x.py", code: "SESSION", mode: "block" },
    ]);
    expect(recordedCheckouts(databasePath)).toEqual([session]);
  });

  it("names the repository's other worktrees that may run rules for the session's walk, and no other folder", async () => {
    const { root, databasePath, session, worktree } = await repositoryWithWorktree(outside);
    const stopped = join(root, "elsewhere", "stopped");
    git(session, "worktree", "add", "--quiet", "-b", "stopped", stopped);
    const removed = join(root, "elsewhere", "removed");
    git(session, "worktree", "add", "--quiet", "-b", "removed", removed);
    await removeFolder(removed);
    await mkdir(join(session, "forged"));
    await writeFile(join(session, "forged", ".git"), `gitdir: ${join(session, ".git")}\n`);
    const database = openRerailDatabase(databasePath);
    try {
      setCheckoutStarted(database, recordCheckout(database, stopped), false);
    } finally {
      database.$client.close();
    }
    const { recorder } = recordingAudits();
    const auditor = router(session, databasePath, recorder);

    await auditor.openWorkspace("roots");

    expect(auditor.worktreeRoots()).toEqual([worktree]);
  });

  it("names where Git registers the worktrees it named for the session's walk", async () => {
    const { databasePath, session } = await repositoryWithWorktree(outside);
    const { recorder } = recordingAudits();
    const auditor = router(session, databasePath, recorder);
    await auditor.openWorkspace("registry");

    auditor.worktreeRoots();

    expect(auditor.worktreeRegistry()).toBe(join(await realpath(session), ".git", "worktrees"));
  });

  it("names a worktree Git made, while the session is open, at a folder the session looked up while it was plain", async () => {
    const { root, databasePath, session, worktree } = await repositoryWithWorktree(outside);
    const later = join(root, "elsewhere", "later");
    const { recorder } = recordingAudits();
    const auditor = router(session, databasePath, recorder);
    await auditor.openWorkspace("later");
    expect(auditor.worktreeRoots()).toEqual([worktree]);
    await mkdir(join(later, "api"), { recursive: true });
    expect(auditor.acceptsPath(join(later, "api", "x.py"))).toBe(false);
    await removeFolder(later);

    git(session, "worktree", "add", "--quiet", "-b", "later", later);

    expect(auditor.worktreeRoots().toSorted()).toEqual([await realpath(later), worktree].toSorted());
  });

  it("names a worktree whose .git file Git wrote after a walk had found it listed", async () => {
    const { root, databasePath, session, worktree } = await repositoryWithWorktree(outside);
    const later = join(root, "elsewhere", "later");
    const { recorder } = recordingAudits();
    const auditor = router(session, databasePath, recorder);
    await auditor.openWorkspace("listed");
    await mkdir(join(later, "api"), { recursive: true });
    expect(auditor.acceptsPath(join(later, "api", "x.py"))).toBe(false);
    await removeFolder(later);
    git(session, "worktree", "add", "--quiet", "-b", "later", later);
    // `git worktree add` lists the worktree before it writes the worktree's .git file: a walk in between finds a folder
    // with none.
    await rename(join(later, ".git"), join(later, ".git-pending"));
    expect(auditor.worktreeRoots()).toEqual([worktree]);
    await rename(join(later, ".git-pending"), join(later, ".git"));

    expect(auditor.worktreeRoots().toSorted()).toEqual([await realpath(later), worktree].toSorted());
  });

  it.skipIf(!caseInsensitiveFilesystem())("names a worktree Git registered under a path typed in another case than the folder has", async () => {
    const { root, databasePath, session, worktree } = await repositoryWithWorktree(outside);
    const typed = join(root, "Elsewhere", "typed");
    git(session, "worktree", "add", "--quiet", "-b", "typed", typed);
    const { recorder } = recordingAudits();
    const auditor = router(session, databasePath, recorder);

    await auditor.openWorkspace("typed-case");

    expect(auditor.worktreeRoots().toSorted()).toEqual([join(root, "elsewhere", "typed"), worktree].toSorted());
  });

  it("gives no lane to a .git file outside the checkout that names the repository's Git folder", async () => {
    const { root, databasePath, session } = await repositoryWithWorktree(outside);
    const forged = join(root, "forged");
    await mkdir(forged);
    await writeFile(join(forged, ".git"), `gitdir: ${join(session, ".git")}\n`);
    const { recorder, audits } = recordingAudits();
    const auditor = router(session, databasePath, recorder);

    await auditor.openWorkspace("forged-outside");
    await edit(auditor, "forged-outside", 1, join(forged, "x.py"));

    expect(violations(audits)).toEqual([]);
    expect(recordedCheckouts(databasePath)).toEqual([session]);
  });

  it("leaves another clone of the repository, which shares no Git folder, unrecorded", async () => {
    const { root, databasePath, session } = await repositoryWithWorktree(outside);
    const clone = join(root, "clone");
    git(root, "clone", "--quiet", session, clone);
    const { recorder, audits } = recordingAudits();
    const auditor = router(session, databasePath, recorder);

    await auditor.openWorkspace("clone");
    await edit(auditor, "clone", 1, join(clone, "api", "x.py"));

    expect(violations(audits)).toEqual([]);
    expect(recordedCheckouts(databasePath)).toEqual([session]);
  });
});

describe("repository settings that reach evaluation", () => {
  const RULE = "typescript-eslint-consistent-type-definitions";
  const CODE = "@typescript-eslint/consistent-type-definitions";
  const typescriptPack = readRuleCatalog([join(PACKAGE_ROOT, "packs", "typescript")]);

  async function editTypeScript(auditor: CheckoutCodeRuleRouter, sessionId: string, revision: number, files: Record<string, string>): Promise<void> {
    await auditor.auditGeneratedEdits(sessionId, {
      revision,
      checkpoints: ["edit"],
      edits: Object.entries(files).map(([path, generatedText]) => ({ path, language: "typescript" as const, generatedText })),
    });
    await auditor.settleGeneratedEdits();
  }

  async function sessionWithRule(): Promise<{ session: string; databasePath: string; repositoryKey: string }> {
    const root = await temporaryRoot();
    const databasePath = join(root, "rerail.sqlite");
    const session = await repository(join(root, "session"));
    const repositoryKey = setUpTestRepository(session, { rerailRules: [RULE] }, databasePath);
    return { session, databasePath, repositoryKey };
  }

  function setRepositorySetting(databasePath: string, repositoryKey: string, value: { suppressions?: "honour" | "ignore"; config?: Record<string, string> }): void {
    const database = openRerailDatabase(databasePath);
    try {
      setRuleSetting(database, { owner: { kind: "repository", repositoryKey }, folder: "", targetKind: "rule", targetId: RULE, on: null, mode: null, ...value });
    } finally {
      database.$client.close();
    }
  }

  it("lets an eslint-disable comment silence a ported rule until the repository ignores suppression comments", async () => {
    const { session, databasePath, repositoryKey } = await sessionWithRule();
    const { recorder, audits } = recordingAudits();
    const auditor = router(session, databasePath, recorder, typescriptPack);
    const disabled = `// eslint-disable-next-line ${CODE}\nexport type Hidden = { a: string };\n`;

    await auditor.openWorkspace("suppressions");
    await editTypeScript(auditor, "suppressions", 1, { "api/hidden.ts": disabled });
    expect(violations(audits)).toEqual([]);

    setRepositorySetting(databasePath, repositoryKey, { suppressions: "ignore" });
    await editTypeScript(auditor, "suppressions", 2, { "api/hidden.ts": `${disabled}\n` });
    expect(violations(audits)).toEqual([{ checkout: session, path: "api/hidden.ts", code: CODE, mode: "steer" }]);
  });

  it("runs a ported rule with the option values the repository sets", async () => {
    const { session, databasePath, repositoryKey } = await sessionWithRule();
    const { recorder, audits } = recordingAudits();
    const auditor = router(session, databasePath, recorder, typescriptPack);
    const files = { "api/kept.ts": "export interface Kept { a: string }\n", "api/alias.ts": "export type X = { a: string };\n" };

    await auditor.openWorkspace("config");
    await editTypeScript(auditor, "config", 1, files);
    expect(violations(audits).map(violation => violation.path)).toEqual(["api/alias.ts"]);

    setRepositorySetting(databasePath, repositoryKey, { config: { option: "type" } });
    audits.length = 0;
    await editTypeScript(auditor, "config", 2, Object.fromEntries(Object.entries(files).map(([path, text]) => [path, `${text}\n`])));
    expect(violations(audits).map(violation => violation.path)).toEqual(["api/kept.ts"]);
  });

  it("runs a ported rule nowhere, quarantined, when the repository's stored config no longer applies to it", async () => {
    const { session, databasePath, repositoryKey } = await sessionWithRule();
    setRepositorySetting(databasePath, repositoryKey, { config: { nope: "x" } });
    const catalogs = new CheckoutCatalogs(databasePath, { ruleCatalog: typescriptPack });
    try {
      const catalog = catalogs.catalog(session);
      expect(catalog.snapshot().codeRules).toEqual([]);
      expect(catalog.quarantined()).toEqual([
        expect.objectContaining({ ruleId: RULE, reason: expect.stringMatching(/`nope` is not an option this document declares/u) }),
      ]);
    } finally {
      catalogs.close();
    }
  });
});

describe("repository audit through the runtime", () => {
  it("records one repository audit per checkout that owns its files; a nested repository nobody set up, or a stray .git, is audited with the checkout around it", async () => {
    vi.stubEnv("RERAIL_EXPERIMENTAL_FIRST_REPOSITORY_AUDIT", "1");
    const root = await temporaryRoot();
    const databasePath = join(root, "rerail.sqlite");
    const { parent, sub } = await withSubmodule(root);
    await writeFile(join(parent, "api", "a.py"), "NEEDLE\n");
    await mkdir(join(sub, "api"), { recursive: true });
    await writeFile(join(sub, "api", "b.py"), "NEEDLE\n");
    // Neither an untracked `git init` nobody set up, nor a `.git` an agent left in a folder, exempts files from the parent.
    const ignored = await repository(join(parent, "tools", "ignored"));
    await writeFile(join(ignored, "api", "c.py"), "NEEDLE\n");
    await mkdir(join(parent, "src"));
    await writeFile(join(parent, "src", ".git"), "");
    await writeFile(join(parent, "src", "d.py"), "NEEDLE\n");
    setUpTestRepository(parent, { library: [file(needle("test:parent", "PARENT"))] }, databasePath);
    setUpTestRepository(sub, { library: [file(needle("test:sub", "SUB"))] }, databasePath);
    const runtime = new RerailRuntime({
      storageRoot: join(root, "sessions"), databasePath, workspaceRoot: parent, modelPath: resolve("model"), autostart: false,
    });
    cleanups.push(() => runtime.dispose());

    await runtime.ingest({ kind: "session-start", eventId: "start", session: { host: "omp", sessionId: "audit", cwd: parent } });

    const database = openRerailDatabase(databasePath);
    cleanups.push(async () => { database.$client.close(); });
    const scopeAudits = () => database.select({
      workspaceRoot: repositoryAudits.workspaceRoot,
      sessionId: repositoryAudits.sessionId,
      status: repositoryAudits.status,
    }).from(repositoryAudits).all().toSorted((a, b) => a.workspaceRoot.localeCompare(b.workspaceRoot));
    await expect.poll(() => scopeAudits().map(row => row.status), { timeout: 30_000 }).toEqual(["completed", "completed"]);
    expect(scopeAudits()).toEqual([
      { workspaceRoot: parent, sessionId: "audit", status: "completed" },
      { workspaceRoot: sub, sessionId: "audit", status: "completed" },
    ]);
    const reader = new SqliteAuditReader(databasePath);
    try {
      expect(reader.list({ scope: "repository", sessionId: "audit" }).audits
        .map(audit => ({ workspaceRoot: audit.workspaceRoot, violations: audit.violations.map(violation => `${violation.path} ${violation.code}`).toSorted() }))
        .toSorted((a, b) => a.workspaceRoot.localeCompare(b.workspaceRoot))).toEqual([
        { workspaceRoot: parent, violations: ["api/a.py PARENT", "src/d.py PARENT", "tools/ignored/api/c.py PARENT"] },
        { workspaceRoot: sub, violations: ["api/b.py SUB"] },
      ]);
    } finally {
      reader.close();
    }
  }, 60_000);
});

describe("rules as they are at a scan and at a session's start", () => {
  it("scans with the rules as they are when a scan starts, though no event came after a rule was turned off elsewhere", async () => {
    const root = await temporaryRoot();
    const databasePath = join(root, "rerail.sqlite");
    const main = await repository(join(root, "main"));
    await writeFile(join(main, "api", "a.py"), "NEEDLE\n");
    const repositoryKey = setUpTestRepository(main, { library: [file(needle("test:main", "MAIN"))] }, databasePath);
    const runtime = new RerailRuntime({ storageRoot: join(root, "sessions"), databasePath, workspaceRoot: main, embedder: testEmbedder, autostart: false });
    cleanups.push(() => runtime.dispose());
    const controls = new SqliteAuditControlStore(databasePath);
    cleanups.push(async () => { controls.close(); });
    const reader = new SqliteAuditReader(databasePath);
    cleanups.push(async () => { reader.close(); });
    await runtime.ingest({ kind: "session-start", eventId: "start", session: { host: "omp", sessionId: "scan", cwd: main } });
    await runtime.codeRulesOpened("scan");
    // A scan the runtime's timer picks up, as `rerail scan` asks for one: the violations it records.
    const scan = async () => {
      const { id } = controls.requestAudit("scan", { scope: "repository" });
      const request = () => controls.listAuditRequests("scan").find(item => item.id === id)!;
      await expect.poll(() => request().status, { timeout: 30_000 }).toBe("completed");
      return request().auditIds.flatMap(auditId => reader.get(auditId)!.violations.map(violation => `${violation.path} ${violation.code}`));
    };
    expect(await scan()).toEqual(["api/a.py MAIN"]);
    const database = openRerailDatabase(databasePath);
    try {
      setRuleSetting(database, { owner: { kind: "repository", repositoryKey }, folder: "", targetKind: "rule", targetId: "test:main", on: false, mode: null });
    } finally {
      database.$client.close();
    }
    expect(await scan()).toEqual([]);
  }, 60_000);

  it("reports a worktree's rules as they are when a session starts there, though another session's lane read them before a change", async () => {
    const root = await temporaryRoot();
    const databasePath = join(root, "rerail.sqlite");
    const main = await repository(join(root, "main"));
    git(main, "worktree", "add", "--quiet", "--detach", join(root, "w"), "HEAD");
    const worktree = await realpath(join(root, "w"));
    const repositoryKey = setUpTestRepository(main, { library: [file(needle("test:main", "MAIN"))] }, databasePath);
    const runtime = new RerailRuntime({ storageRoot: join(root, "sessions"), databasePath, workspaceRoot: main, embedder: testEmbedder, autostart: false });
    cleanups.push(() => runtime.dispose());
    const reader = new SqliteSessionReader(databasePath);
    cleanups.push(async () => { reader.close(); });

    await runtime.ingest({ kind: "session-start", eventId: "first-start", session: { host: "claude-code", sessionId: "first", cwd: main } });
    await runtime.codeRulesOpened("first");
    // Its edit in the worktree opens a lane there, which reads the worktree's one rule.
    const edited = { sessionId: "first", toolCallId: "write", toolName: "Write" };
    await runtime.ingest({ kind: "call", eventId: "write", ...edited, input: { file_path: join(worktree, "api", "x.py"), content: "x = 1\n" } });
    await runtime.ingest({ kind: "result", eventId: "written", ...edited, output: "ok" });
    // A setting made elsewhere turns that rule off. No walk reads a setting, and no event of the first session follows,
    // so its lane in the worktree holds the rules as it read them when the second session starts there.
    const database = openRerailDatabase(databasePath);
    try {
      setRuleSetting(database, { owner: { kind: "repository", repositoryKey }, folder: "", targetKind: "rule", targetId: "test:main", on: false, mode: null });
    } finally {
      database.$client.close();
    }
    await runtime.ingest({ kind: "session-start", eventId: "second-start", session: { host: "claude-code", sessionId: "second", cwd: worktree } });

    // The count the start wrote, which an older read would leave at 1, and the one a person sees, null while stale.
    expect(recordedRuleCount(databasePath, "second")).toBe(0);
    expect(reader.hud("second").activeRuleCount).toBe(0);
  }, 60_000);

  it.runIf(process.platform === "win32")("leaves a nested checkout's files out of the folder around it, recorded under another spelling of that folder", async () => {
    const root = await temporaryRoot();
    const databasePath = join(root, "rerail.sqlite");
    const stored = join(root, "workspace");
    await mkdir(join(stored, "packages"), { recursive: true });
    await writeFile(join(stored, "top.py"), "NEEDLE\n");
    await writeFile(join(stored, "packages", "outer.py"), "NEEDLE\n");
    const service = await repository(join(stored, "packages", "service"));
    await writeFile(join(service, "api", "inner.py"), "NEEDLE\n");
    // A session in VS Code names the drive in lower case, and a folder outside Git is recorded as named; Git names the
    // nested checkout as the folder is stored.
    const drive = stored[0]!;
    const typed = `${drive === drive.toUpperCase() ? drive.toLowerCase() : drive.toUpperCase()}${stored.slice(1)}`;
    setUpTestRepository(typed, { library: [file(needle("test:outer", "OUTER"))] }, databasePath);
    setUpTestRepository(service, { library: [file(needle("test:service", "SERVICE"))] }, databasePath);
    // As the runtime starts a session in a folder outside Git.
    const owners = new CheckoutOwners();
    owners.addPlainRoot(typed);
    const catalogs = new CheckoutCatalogs(databasePath);
    const { recorder, audits } = recordingAudits();
    const auditor = new CheckoutCodeRuleRouter({
      sessionRoot: typed, startingCheckout: startingCheckout(owners, typed).checkoutRoot, owners, catalogs,
      steerDelivery: { requestSteer: async () => undefined }, audits: recorder,
    });
    cleanups.push(async () => {
      await auditor.close();
      catalogs.close();
    });
    expect(catalogs.recordedCheckoutsWithin(typed)).toContain(service);

    await auditor.openWorkspace("spelling");
    await auditor.audit("spelling", { scope: "touched-files", mode: "log", paths: ["packages"] });
    await auditor.audit("spelling", { scope: "repository", mode: "log" });
    await auditor.settle();

    // The folder audit, then the repository audit, in the folder around the nested checkout, whichever spelling the
    // audits name it by: none holds the nested checkout's file.
    expect(violations(audits).filter(violation => violation.checkout !== service).map(violation => violation.path))
      .toEqual(["packages/outer.py", "packages/outer.py", "top.py"]);
  });
});

describe("CheckoutCodeRuleRouter lanes after review", () => {
  it("drops a checkout's steering result once that checkout is stopped, so another checkout's batch does not carry it", async () => {
    const root = await temporaryRoot();
    const databasePath = join(root, "rerail.sqlite");
    const session = await repository(join(root, "session"));
    const sibling = await repository(join(root, "sibling"));
    setUpTestRepository(session, { library: [file(needle("test:session", "SESSION", "steer"))] }, databasePath);
    setUpTestRepository(sibling, { library: [file(needle("test:sibling", "SIBLING", "steer"))] }, databasePath);
    const { recorder } = recordingAudits();
    const auditor = router(session, databasePath, recorder);

    await auditor.openWorkspace("stopped-lane");
    await edit(auditor, "stopped-lane", 1, join(sibling, "api", "x.py"));
    // A batch with nothing for a lane reports that lane's current status.
    const steered = await auditor.auditGeneratedEdits("stopped-lane", { revision: 2, checkpoints: ["edit"], edits: [] });
    expect(steered.disposition).toBe("steer");
    expect(steered.violations.map(violation => violation.code)).toEqual(["SIBLING"]);

    stopTestRepository(sibling, databasePath);
    auditor.refreshCodeRules();
    const next = await auditor.auditGeneratedEdits("stopped-lane", { revision: 3, checkpoints: ["edit"], edits: [] });
    expect(next.violations).toEqual([]);
    expect(next.disposition).toBe("continue");
  });

  it("audits a recorded checkout nested in a folder a folder-scoped audit names, in its own lane", async () => {
    const root = await temporaryRoot();
    const databasePath = join(root, "rerail.sqlite");
    const session = await repository(join(root, "session"));
    const service = await repository(join(session, "packages", "service"));
    await writeFile(join(session, "packages", "outer.py"), "NEEDLE\n");
    await writeFile(join(service, "api", "inner.py"), "NEEDLE\n");
    setUpTestRepository(session, { library: [file(needle("test:session", "SESSION"))] }, databasePath);
    setUpTestRepository(service, { library: [file(needle("test:service", "SERVICE"))] }, databasePath);
    const { recorder, audits } = recordingAudits();
    const auditor = router(session, databasePath, recorder);

    await auditor.openWorkspace("folder");
    await auditor.audit("folder", { scope: "touched-files", mode: "log", paths: ["packages"] });
    await auditor.settle();

    expect(violations(audits).toSorted((a, b) => a.code.localeCompare(b.code))).toEqual([
      { checkout: service, path: "api/inner.py", code: "SERVICE", mode: "block" },
      { checkout: session, path: "packages/outer.py", code: "SESSION", mode: "block" },
    ]);
  });

  it("names the checkouts whose ignore rules the session follows: its own and, each level down, the nested ones that run rules", async () => {
    const root = await temporaryRoot();
    const databasePath = join(root, "rerail.sqlite");
    const session = await repository(join(root, "session"));
    const service = await repository(join(session, "packages", "service"));
    const inner = await repository(join(service, "vendor", "inner"));
    // Recorded inside the checkout, started but never set up: it runs no rules, so its ignore rules decide nothing.
    const vendored = await repository(join(session, "vendored"));
    for (const checkout of [session, service, inner]) setUpTestRepository(checkout, { library: [file(needle("test:x", "X"))] }, databasePath);
    const database = openRerailDatabase(databasePath);
    try {
      recordCheckout(database, vendored);
    } finally {
      database.$client.close();
    }
    const auditor = router(session, databasePath, recordingAudits().recorder);

    await auditor.openWorkspace("ignore-roots");
    expect(auditor.ignoreRuleRoots()).toEqual([session, service, inner]);
  });
});
