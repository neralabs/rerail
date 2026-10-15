import { execFileSync } from "node:child_process";
import { mkdir, mkdtemp, realpath, rm, symlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { CheckoutOwners } from "../../../src/engine/catalog/checkout-owner.js";
import { CheckoutCodeRuleRouter } from "../../../src/engine/code-rule/checkout-code-rule-router.js";
import { openRerailDatabase } from "../../../src/engine/storage/database.js";
import { CheckoutCatalogs, startingCheckout } from "../../../src/engine/code-rule/code-rule-dsl-catalog.js";
import { SqliteAuditReader } from "../../../src/engine/audit/audit-store.js";
import type { AuditRecorder, AuditInput, AuditResult } from "../../../src/engine/audit/audit.js";
import { type CodeRule, codeRuleSchema } from "../../../src/engine/code-rule/code-rule-schema.js";
import { recordCheckout } from "../../../src/engine/catalog/repository-registry.js";
import { setCheckoutStarted, RerailRuntime } from "../../../src/runtime/index.js";
import { setRuleSetting } from "../../../src/engine/catalog/rule-configuration.js";
import { readBundledCatalog } from "../../../src/engine/catalog/bundled-rules.js";
import type { BundledCatalog } from "../../../src/engine/catalog/repository-model.js";
import { PACKAGE_ROOT } from "../../../src/engine/package-root.js";
import type { RuleMode } from "../../../src/engine/catalog/rule-mode.js";
import { repositoryAudits } from "../../../src/engine/storage/schema.js";
import { setUpTestRepository } from "../../support/repository-rules.js";
import { stopTestRepository } from "../../support/started-repository.js";

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

function router(sessionRoot: string, databasePath: string, recorder: AuditRecorder, bundle?: BundledCatalog): CheckoutCodeRuleRouter {
  const owners = new CheckoutOwners();
  const catalogs = new CheckoutCatalogs(databasePath, bundle === undefined ? {} : { bundle });
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

describe("repository settings that reach evaluation", () => {
  const RULE = "typescript-eslint-consistent-type-definitions";
  const CODE = "@typescript-eslint/consistent-type-definitions";
  const typescriptPack = readBundledCatalog([join(PACKAGE_ROOT, "packs", "typescript")]);

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
    const catalogs = new CheckoutCatalogs(databasePath, { bundle: typescriptPack });
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
});
