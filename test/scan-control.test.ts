import { execFileSync } from "node:child_process";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { SqliteScanControlStore } from "../src/scan-control.js";
import { SqliteTranscriptStore } from "../src/transcript-store.js";

const directories: string[] = [];
afterEach(async () => { await Promise.all(directories.splice(0).map(path => rm(path, { recursive: true, force: true }))); });

async function fixture() {
  const root = await mkdtemp(join(tmpdir(), "rerail-scan-control-"));
  directories.push(root);
  const path = join(root, "rerail.sqlite");
  const transcripts = new SqliteTranscriptStore(path);
  transcripts.openSession({ host: "omp", sessionId: "a", cwd: root });
  transcripts.openSession({ host: "omp", sessionId: "b", cwd: join(root, "other") });
  return { root, transcripts, controls: new SqliteScanControlStore(path) };
}

describe("scan controls", () => {

  it("persists workspace defaults across reopening without leaking to another checkout", async () => {
    const { controls, transcripts } = await fixture();
    controls.updateSettings("a", { scope: "repository", mode: "log" });
    expect(controls.getSettings("a").settings).toEqual({ scope: "repository", mode: "log" });
    expect(controls.getSettings("b").settings).toEqual({ scope: "touched-files", mode: "resteer" });
    controls.close();
    const reopened = new SqliteScanControlStore(transcripts.path);
    expect(reopened.getSettings("a").settings).toEqual({ scope: "repository", mode: "log" });
    reopened.close(); transcripts.close();
  });

  it("never repeats a failed first repository diagnostic in a later session", async () => {
    const { root, controls, transcripts } = await fixture();
    expect(controls.claimRepositoryDiagnostic("a")).toBe(true);
    controls.finishRepositoryDiagnostic("a", "failed-run", "failed", "Evaluation failed");
    transcripts.openSession({ host: "omp", sessionId: "later", cwd: root });
    expect(controls.claimRepositoryDiagnostic("later")).toBe(false);
    expect(controls.getSettings("later").repositoryDiagnostic).toMatchObject({
      sessionId: "a",
      runId: "failed-run",
      status: "failed",
      error: "Evaluation failed",
    });
    controls.close(); transcripts.close();
  });

  it("records and interrupts scans without claiming another session's work", async () => {
    const { controls, transcripts } = await fixture();
    const owner = controls.registerOwner("a", "next-event");
    const queued = controls.requestScan("a", { scope: "repository" });
    expect(queued.mode).toBe("log");
    expect(controls.claim("b", owner)).toBeUndefined();
    expect(controls.claim("a", owner)).toMatchObject({ id: queued.id, status: "running" });
    controls.interrupt("a");
    expect(controls.listScans("a").find(scan => scan.id === queued.id)).toMatchObject({ status: "interrupted" });
    const interrupted = controls.listScans("a").find(scan => scan.id === queued.id);
    controls.finish(queued.id, "interrupted", ["partial-run"], "Late native completion");
    expect(controls.listScans("a").find(scan => scan.id === queued.id)).toEqual({
      ...interrupted,
      runIds: ["partial-run"],
    });
    controls.finish(queued.id, "completed", ["replacement-run"]);
    expect(controls.listScans("a").find(scan => scan.id === queued.id)).toEqual({
      ...interrupted,
      runIds: ["partial-run"],
    });
    controls.close(); transcripts.close();
  });

  it("resolves git-diff scans to the current tracked and untracked files", async () => {
    const { root, controls, transcripts } = await fixture();
    await writeFile(join(root, ".gitignore"), "rerail.sqlite*\n");
    await writeFile(join(root, "tracked.ts"), "export const value = 1;\n");
    execFileSync("git", ["init"], { cwd: root });
    execFileSync("git", ["add", ".gitignore", "tracked.ts"], { cwd: root });
    execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@example.invalid", "commit", "-m", "baseline"], { cwd: root });
    await writeFile(join(root, "tracked.ts"), "export const value = 2;\n");
    await writeFile(join(root, "untracked.ts"), "export const added = true;\n");
    const owner = controls.registerOwner("a", "next-event");

    const queued = controls.requestScan("a", { scope: "git-diff" });

    expect(queued).toMatchObject({
      scope: "touched-files",
      mode: "log",
      paths: ["tracked.ts", "untracked.ts"],
      status: "queued",
    });
    expect(controls.claim("a", owner)).toMatchObject({ id: queued.id, status: "running" });
    controls.finish(queued.id, "completed", ["diff-run"]);
    expect(controls.getSettings("a").repositoryDiagnostic).toMatchObject({
      runId: "diff-run",
      status: "completed",
    });
    controls.close(); transcripts.close();
  });

  it("isolates replacement owners and interrupts work stranded by a prior owner", async () => {
    const { controls, transcripts } = await fixture();
    const first = controls.registerOwner("a", "next-event");
    const queued = controls.requestScan("a", { scope: "repository" });
    controls.releaseOwner("a", first);
    const replacement = controls.registerOwner("a", "background");
    expect(controls.claim("a", first)).toBeUndefined();
    expect(controls.claim("a", replacement)).toBeUndefined();
    expect(controls.listScans("a").find(scan => scan.id === queued.id)).toMatchObject({ status: "interrupted" });
    controls.close(); transcripts.close();
  });
});
