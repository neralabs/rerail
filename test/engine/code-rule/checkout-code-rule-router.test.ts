import { execFileSync } from "node:child_process";
import { existsSync, mkdtempSync, rmSync } from "node:fs";
import { mkdir, mkdtemp, realpath, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { CheckoutOwners } from "../../../src/engine/catalog/checkout-owner.js";
import { CheckoutCodeRuleRouter } from "../../../src/engine/code-rule/checkout-code-rule-router.js";
import { openRerailDatabase } from "../../../src/engine/storage/database.js";
import { CheckoutCatalogs, startingCheckout } from "../../../src/engine/code-rule/code-rule-dsl-catalog.js";
import { SqliteAuditReader } from "../../../src/engine/audit/audit-store.js";
import type { AuditRecorder, AuditInput, AuditResult } from "../../../src/engine/audit/audit.js";
import { type CodeRule, codeRuleSchema } from "../../../src/engine/code-rule/code-rule-schema.js";
import { recordCheckout } from "../../../src/engine/catalog/repository-registry.js";
import { setCheckoutStarted, setRepositoryStarted, RerailRuntime } from "../../../src/runtime/index.js";
import { setRuleSetting } from "../../../src/engine/catalog/rule-configuration.js";
import { readRuleCatalog } from "../../../src/engine/catalog/rule-catalog.js";
import type { RuleCatalog } from "../../../src/engine/catalog/repository-model.js";
import { PACKAGE_ROOT } from "../../../src/engine/package-root.js";
import type { RuleMode } from "../../../src/engine/catalog/rule-mode.js";
import { checkouts, repositoryAudits } from "../../../src/engine/storage/schema.js";
import { setUpTestRepository } from "../../support/repository-rules.js";
import { stopTestRepository } from "../../support/started-repository.js";
import { symlinkOrSkip } from "../../support/platform/links.js";

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
  cleanups.push(() => rm(root, { recursive: true, force: true }));
  return root;
}

/** A committed Git repository at `path`; returns its physical path. */
async function repository(path: string): Promise<string> {
  await mkdir(join(path, "api"), { recursive: true });
  await writeFile(join(path, "README.md"), "seed\n");
  git(path, "init", "--quiet");
  git(path, "add", ".");
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
    rmSync(probe, { recursive: true, force: true });
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
    await rm(removed, { recursive: true });
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
