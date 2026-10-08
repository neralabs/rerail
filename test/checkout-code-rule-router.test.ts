import { execFileSync } from "node:child_process";
import { mkdir, mkdtemp, realpath, rm, symlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { CheckoutOwners } from "../src/checkout-owner.js";
import { CheckoutCodeRuleRouter } from "../src/checkout-code-rule-router.js";
import { openRerailDatabase } from "../src/database.js";
import { CheckoutCatalogs, startingCheckout } from "../src/code-rule-dsl-catalog.js";
import { SqliteAuditReader } from "../src/audit-store.js";
import type { AuditRecorder, AuditInput, AuditResult } from "../src/audit.js";
import { type CodeRule, codeRuleSchema } from "../src/code-rule-schema.js";
import { recordCheckout } from "../src/repository-registry.js";
import { setRuleSetting } from "../src/rule-configuration.js";
import type { RuleMode } from "../src/rule-mode.js";
import { RerailRuntime } from "../src/runtime.js";
import { repositoryAudits } from "../src/schema.js";
import { setUpTestRepository } from "./support/repository-rules.js";
import { stopTestRepository } from "./support/started-repository.js";

const cleanups: Array<() => Promise<void>> = [];
afterEach(async () => {
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
  return { kind: "file" as const, rule };
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

function router(sessionRoot: string, databasePath: string, recorder: AuditRecorder): CheckoutCodeRuleRouter {
  const owners = new CheckoutOwners();
  const catalogs = new CheckoutCatalogs(databasePath);
  const auditor = new CheckoutCodeRuleRouter({
    sessionRoot,
    startingCheckout: startingCheckout(owners, sessionRoot).checkoutRoot,
    owners,
    catalogs,
    steerDelivery: { requestSteer: async () => undefined },
    audits: recorder,
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

  it("rejects a symlink whose target leaves its checkout", async () => {
    const root = await temporaryRoot();
    const databasePath = join(root, "rerail.sqlite");
    const session = await repository(join(root, "session"));
    const sibling = await repository(join(root, "sibling"));
    setUpTestRepository(session, { library: [file(needle("test:session", "SESSION"))] }, databasePath);
    setUpTestRepository(sibling, { library: [file(needle("test:sibling", "SIBLING"))] }, databasePath);
    await writeFile(join(sibling, "api", "x.py"), "NEEDLE\n");
    await symlink(join(sibling, "api", "x.py"), join(session, "api", "escape.py"));
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

  it("rejects a parent's file that is a symlink into a nested checkout, which the parent's rules would otherwise judge", async () => {
    const root = await temporaryRoot();
    const databasePath = join(root, "rerail.sqlite");
    const { parent, sub } = await withSubmodule(root);
    setUpTestRepository(parent, { library: [file(needle("test:parent", "PARENT"))] }, databasePath);
    setUpTestRepository(sub, { library: [file(needle("test:sub", "SUB"))] }, databasePath);
    await mkdir(join(sub, "api"), { recursive: true });
    await writeFile(join(sub, "api", "b.py"), "NEEDLE\n");
    // Lexically `api/link.py` is the parent's; really it is the submodule's `api/b.py`.
    await symlink(join(sub, "api", "b.py"), join(parent, "api", "link.py"));
    const { recorder, audits } = recordingAudits();
    const auditor = router(parent, databasePath, recorder);

    await auditor.openWorkspace("symlinked-submodule");
    expect(auditor.acceptsPath("api/link.py")).toBe(false);
    expect(auditor.acceptsPath("sub/api/b.py")).toBe(true);
    await edit(auditor, "symlinked-submodule", 1, "api/link.py");

    expect(violations(audits)).toEqual([]);
  });
});

describe("repository audit through the runtime", () => {
  it("records one repository audit per checkout that owns its files; a nested repository nobody set up, or a stray .git, is audited with the checkout around it", async () => {
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
