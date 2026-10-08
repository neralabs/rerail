import { execFileSync } from "node:child_process";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { SqliteAuditControlStore } from "../src/audit-control.js";
import { SqliteSessionStore } from "../src/session-store.js";

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
    const { root, controls, sessionStore: sessionStore } = await fixture();
    expect(controls.claimRepositoryAudit("a")).toBe(true);
    controls.finishRepositoryAudit("a", "failed-run", "failed", "Evaluation failed");
    sessionStore.openSession({ host: "omp", sessionId: "later", cwd: root });
    expect(controls.claimRepositoryAudit("later")).toBe(false);
    expect(controls.getSettings("later").repositoryAudit).toMatchObject({
      sessionId: "a",
      auditId: "failed-run",
      status: "failed",
      error: "Evaluation failed",
    });
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
