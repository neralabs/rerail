import { execFileSync } from "node:child_process";
import { realpathSync } from "node:fs";
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { SqliteAuditControlStore } from "../../../src/engine/audit/audit-control.js";
import { SqliteSessionStore } from "../../../src/runtime/index.js";
import { SqliteAuditStore } from "../../../src/engine/audit/audit-store.js";
import { omittedFailuresSummary, type AuditFailure } from "../../../src/engine/audit/audit.js";
import { openRerailDatabase } from "../../../src/engine/storage/database.js";
import { auditSettings } from "../../../src/engine/storage/schema.js";
import { recordCheckout } from "../../../src/engine/catalog/repository-registry.js";

const directories: string[] = [];
afterEach(async () => { await Promise.all(directories.splice(0).map(path => rm(path, { recursive: true, force: true }))); });

async function fixture() {
  const root = await mkdtemp(join(tmpdir(), "rerail-scan-control-"));
  directories.push(root);
  const path = join(root, "rerail.sqlite");
  const sessionStore = new SqliteSessionStore(path);
  sessionStore.openSession({ host: "omp", sessionId: "a", cwd: root });
  sessionStore.openSession({ host: "omp", sessionId: "b", cwd: join(root, "other") });
  return { root, sessionStore: sessionStore, controls: new SqliteAuditControlStore(path) };
}

describe("audit controls", () => {

  it("persists workspace defaults across reopening without leaking to another checkout", async () => {
    const { controls, sessionStore: sessionStore } = await fixture();
    controls.updateSettings("a", { scope: "repository", mode: "log" });
    expect(controls.getSettings("a").settings).toEqual({ scope: "repository", mode: "log", steerStrategy: "message" });
    expect(controls.getSettings("b").settings).toEqual({ scope: "ai-output", mode: "steer", steerStrategy: "message" });
    controls.close();
    const reopened = new SqliteAuditControlStore(sessionStore.path);
    expect(reopened.getSettings("a").settings).toEqual({ scope: "repository", mode: "log", steerStrategy: "message" });
    reopened.close(); sessionStore.close();
  });

  it("reads settings saved before the steer strategy existed as the message strategy", async () => {
    const { root, controls, sessionStore } = await fixture();
    const database = openRerailDatabase(sessionStore.path);
    try {
      database.insert(auditSettings).values({ workspaceRoot: realpathSync(root), settings: { scope: "ai-output", mode: "log" }, updatedAt: 1 }).run();
    } finally { database.$client.close(); }
    expect(controls.getSettings("a").settings).toEqual({ scope: "ai-output", mode: "log", steerStrategy: "message" });
    controls.updateSettings("a", { steerStrategy: "subagent" });
    expect(controls.getSettings("a").settings).toEqual({ scope: "ai-output", mode: "log", steerStrategy: "subagent" });
    controls.close(); sessionStore.close();
  });

  it("saves a checkout's settings by its folder before any session there, and a later patch keeps what it leaves out", async () => {
    const { root, controls, sessionStore } = await fixture();
    const unseen = join(root, "unseen");
    await mkdir(unseen);
    expect(controls.updateWorkspaceSettings(unseen, { steerStrategy: "rewind" })).toEqual({ scope: "ai-output", mode: "steer", steerStrategy: "rewind" });
    expect(controls.getWorkspaceSettings(unseen)).toEqual({ scope: "ai-output", mode: "steer", steerStrategy: "rewind" });
    sessionStore.openSession({ host: "pi", sessionId: "c", cwd: unseen });
    expect(controls.updateSettings("c", { mode: "log" }).settings).toEqual({ scope: "ai-output", mode: "log", steerStrategy: "rewind" });
    expect(() => controls.updateWorkspaceSettings(unseen, { steerStrategy: "undo" as never })).toThrow();
    controls.close(); sessionStore.close();
  });

  it("retries an interrupted repository audit in a later session, and never a completed one", async () => {
    const { root, controls, sessionStore } = await fixture();
    const checkout = realpathSync(root);
    expect(controls.claimRepositoryAudit("a", checkout)).toBe(true);
    controls.interruptRepositoryAudit("a");
    sessionStore.openSession({ host: "omp", sessionId: "later", cwd: root });
    expect(controls.claimRepositoryAudit("later", checkout)).toBe(true);
    expect(controls.getSettings("later").repositoryAudit).toMatchObject({ sessionId: "later", auditId: null, status: "running", error: null });
    controls.interruptRepositoryAudit("later");
    sessionStore.openSession({ host: "omp", sessionId: "third", cwd: root });
    expect(controls.claimRepositoryAudit("third", checkout)).toBe(true);
    controls.finishRepositoryAudit("third", checkout, "completed-run", "completed");
    sessionStore.openSession({ host: "omp", sessionId: "fourth", cwd: root });
    expect(controls.claimRepositoryAudit("fourth", checkout)).toBe(false);
    expect(controls.getSettings("fourth").repositoryAudit).toMatchObject({ sessionId: "third", auditId: "completed-run", status: "completed" });
    controls.close(); sessionStore.close();
  });

  describe("a failed repository audit", () => {
    const sourceGap = { clauseId: "source", code: "source-partial" as const, message: "file has syntax errors", path: "a.ts", stage: "read" as const };
    const scanLimit = {
      clauseId: "scan", code: "budget-exceeded" as const, message: "Repository scan stopped after reaching 20 failures", stage: "scan" as const,
    };
    const budget = {
      clauseId: "scan", code: "budget-exceeded" as const, message: "Repository scan exceeded its five-minute work budget", stage: "scan" as const,
    };
    const adapterError = { clauseId: "catalog", code: "adapter-error" as const, message: "Workspace unavailable", stage: "evaluate" as const };

    /** A checkout whose repository audit failed with `failures` against the catalog `ranAgainst`, and a later session. */
    async function failedAudit(failures: AuditFailure[], ranAgainst = "catalog-1") {
      const { root, controls, sessionStore } = await fixture();
      const checkout = realpathSync(root);
      const audits = new SqliteAuditStore(sessionStore.path);
      const auditId = audits.start({
        sessionId: "a", workspaceRoot: checkout, scope: "repository", mode: "log", revision: 0, catalogRevision: ranAgainst, ruleRevisions: [], paths: [],
      });
      audits.finish(auditId, { status: "failed", violations: [], failures });
      audits.close();
      expect(controls.claimRepositoryAudit("a", checkout, ranAgainst)).toBe(true);
      controls.finishRepositoryAudit("a", checkout, auditId, "failed", "Evaluation failed");
      sessionStore.openSession({ host: "omp", sessionId: "later", cwd: root });
      return { checkout, controls, close: () => { controls.close(); sessionStore.close(); } };
    }

    it("runs again when it failed only for files read in part, which the old rule counted", async () => {
      const { checkout, controls, close } = await failedAudit([sourceGap, scanLimit]);
      expect(controls.claimRepositoryAudit("later", checkout, "catalog-1")).toBe(true);
      close();
    });

    it("runs again when it failed only for files read in part and the failures its cap dropped were counted", async () => {
      const { checkout, controls, close } = await failedAudit([sourceGap, omittedFailuresSummary(4)]);
      expect(controls.claimRepositoryAudit("later", checkout, "catalog-1")).toBe(true);
      close();
    });

    it("does not run again for a budget, while its catalog is the one it ran against", async () => {
      const { checkout, controls, close } = await failedAudit([sourceGap, budget]);
      expect(controls.claimRepositoryAudit("later", checkout, "catalog-1")).toBe(false);
      close();
    });

    it("does not run again for an adapter error, while its catalog is the one it ran against", async () => {
      const { checkout, controls, close } = await failedAudit([adapterError]);
      expect(controls.claimRepositoryAudit("later", checkout, "catalog-1")).toBe(false);
      expect(controls.claimRepositoryAudit("later", checkout)).toBe(false);
      close();
    });

    it("runs again once the compiled catalog is another one, and not before", async () => {
      const { checkout, controls, close } = await failedAudit([adapterError, budget]);
      expect(controls.claimRepositoryAudit("later", checkout, "catalog-1")).toBe(false);
      expect(controls.claimRepositoryAudit("later", checkout, "catalog-2")).toBe(true);
      close();
    });
  });

  it("keeps one repository audit per checkout and shows a session the audit of the checkout it is in", async () => {
    const { root, controls, sessionStore } = await fixture();
    await mkdir(join(root, "other"));
    const checkout = realpathSync(root);
    const nested = join(checkout, "other");
    expect(controls.claimRepositoryAudit("a", checkout)).toBe(true);
    expect(controls.claimRepositoryAudit("a", nested)).toBe(true);
    controls.finishRepositoryAudit("a", checkout, "parent-run", "completed");
    // Session `b` works in `other`: the nearest audited folder at or above it is the nested checkout.
    expect(controls.getSettings("b").repositoryAudit).toMatchObject({ status: "running", auditId: null });
    controls.interruptRepositoryAudit("a");
    expect(controls.getSettings("b").repositoryAudit).toMatchObject({ status: "interrupted" });
    expect(controls.getSettings("a").repositoryAudit).toMatchObject({ status: "completed", auditId: "parent-run" });
    controls.close(); sessionStore.close();
  });

  it("records and interrupts audits without claiming another session's work", async () => {
    const { controls, sessionStore: sessionStore } = await fixture();
    const owner = controls.registerOwner("a", "next-event");
    const queued = controls.requestAudit("a", { scope: "repository" });
    expect(queued.mode).toBe("log");
    expect(controls.claim("b", owner)).toBeUndefined();
    expect(controls.claim("a", owner)).toMatchObject({ id: queued.id, status: "running" });
    controls.interrupt("a");
    expect(controls.listAuditRequests("a").find(request => request.id === queued.id)).toMatchObject({ status: "interrupted" });
    const interrupted = controls.listAuditRequests("a").find(request => request.id === queued.id);
    controls.finish(queued.id, "interrupted", ["partial-run"], "Late native completion");
    expect(controls.listAuditRequests("a").find(request => request.id === queued.id)).toEqual({
      ...interrupted,
      auditIds: ["partial-run"],
    });
    controls.finish(queued.id, "completed", ["replacement-run"]);
    expect(controls.listAuditRequests("a").find(request => request.id === queued.id)).toEqual({
      ...interrupted,
      auditIds: ["partial-run"],
    });
    controls.close(); sessionStore.close();
  });

  it("resolves git-diff audits to the current tracked and untracked files", async () => {
    const { root, controls, sessionStore: sessionStore } = await fixture();
    await writeFile(join(root, ".gitignore"), "rerail.sqlite*\n");
    await writeFile(join(root, "tracked.ts"), "export const value = 1;\n");
    execFileSync("git", ["init"], { cwd: root });
    execFileSync("git", ["add", ".gitignore", "tracked.ts"], { cwd: root });
    execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@example.invalid", "commit", "-m", "baseline"], { cwd: root });
    await writeFile(join(root, "tracked.ts"), "export const value = 2;\n");
    await writeFile(join(root, "untracked.ts"), "export const added = true;\n");
    const owner = controls.registerOwner("a", "next-event");

    const queued = controls.requestAudit("a", { scope: "git-diff" });

    expect(queued).toMatchObject({
      scope: "touched-files",
      mode: "log",
      paths: ["tracked.ts", "untracked.ts"],
      status: "queued",
    });
    expect(controls.claim("a", owner)).toMatchObject({ id: queued.id, status: "running" });
    controls.finish(queued.id, "completed", ["diff-run"]);
    expect(controls.getSettings("a").repositoryAudit).toMatchObject({
      auditId: "diff-run",
      status: "completed",
    });
    controls.close(); sessionStore.close();
  });

  it("lists the own files of a dirty submodule and a recorded nested checkout, and leaves any other nested repository a folder", async () => {
    const { root, controls, sessionStore } = await fixture();
    const git = (cwd: string, ...args: string[]) => execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@example.invalid", "-c", "protocol.file.allow=always", ...args], { cwd, stdio: "pipe" });
    const library = await mkdtemp(join(tmpdir(), "rerail-scan-control-sub-"));
    directories.push(library);
    await writeFile(join(library, "lib.ts"), "export const lib = 1;\n");
    git(library, "init");
    git(library, "add", "lib.ts");
    git(library, "commit", "-m", "lib");
    await writeFile(join(root, ".gitignore"), "rerail.sqlite*\n");
    git(root, "init");
    git(root, "add", ".gitignore");
    git(root, "commit", "-m", "baseline");
    git(root, "submodule", "add", library, "vendor/sub");
    git(root, "commit", "-m", "submodule");
    await writeFile(join(root, "vendor/sub/lib.ts"), "export const lib = 2;\n");
    await writeFile(join(root, "vendor/sub/added.ts"), "export const added = true;\n");
    // Git is never run in a nested repository the checkout does not declare and Rerail never recorded.
    await mkdir(join(root, "nested"));
    git(join(root, "nested"), "init");
    await writeFile(join(root, "nested/scratch.ts"), "export const scratch = 1;\n");
    await mkdir(join(root, "recorded"));
    git(join(root, "recorded"), "init");
    await writeFile(join(root, "recorded/kept.ts"), "export const kept = 1;\n");
    const database = openRerailDatabase(join(root, "rerail.sqlite"));
    try {
      recordCheckout(database, join(root, "recorded"));
    } finally {
      database.$client.close();
    }
    controls.registerOwner("a", "next-event");

    expect(controls.requestAudit("a", { scope: "git-diff" }).paths)
      .toEqual(["nested", "recorded/kept.ts", "vendor/sub/added.ts", "vendor/sub/lib.ts"]);
    controls.close(); sessionStore.close();
  });

  it("isolates replacement owners and interrupts work stranded by a prior owner", async () => {
    const { controls, sessionStore: sessionStore } = await fixture();
    const first = controls.registerOwner("a", "next-event");
    const queued = controls.requestAudit("a", { scope: "repository" });
    controls.releaseOwner("a", first);
    const replacement = controls.registerOwner("a", "background");
    expect(controls.claim("a", first)).toBeUndefined();
    expect(controls.claim("a", replacement)).toBeUndefined();
    expect(controls.listAuditRequests("a").find(request => request.id === queued.id)).toMatchObject({ status: "interrupted" });
    controls.close(); sessionStore.close();
  });
});
