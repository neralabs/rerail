import { execFileSync } from "node:child_process";
import { realpathSync } from "node:fs";
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { SqliteAuditControlStore } from "../src/audit-control.js";
import { SqliteSessionStore } from "../src/session-store.js";
import { openRerailDatabase } from "../src/database.js";
import { recordCheckout } from "../src/repository-registry.js";

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
    expect(controls.getSettings("a").settings).toEqual({ scope: "repository", mode: "log" });
    expect(controls.getSettings("b").settings).toEqual({ scope: "touched-files", mode: "steer" });
    controls.close();
    const reopened = new SqliteAuditControlStore(sessionStore.path);
    expect(reopened.getSettings("a").settings).toEqual({ scope: "repository", mode: "log" });
    reopened.close(); sessionStore.close();
  });

  it("never repeats a failed first repository audit in a later session", async () => {
    const { root, controls, sessionStore } = await fixture();
    const checkout = realpathSync(root);
    expect(controls.claimRepositoryAudit("a", checkout)).toBe(true);
    controls.finishRepositoryAudit("a", checkout, "failed-run", "failed", "Evaluation failed");
    sessionStore.openSession({ host: "omp", sessionId: "later", cwd: root });
    expect(controls.claimRepositoryAudit("later", checkout)).toBe(false);
    expect(controls.getSettings("later").repositoryAudit).toMatchObject({
      sessionId: "a",
      auditId: "failed-run",
      status: "failed",
      error: "Evaluation failed",
    });
    controls.close(); sessionStore.close();
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
