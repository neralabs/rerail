import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { mergeAssistState } from "../../../src/engine/index.js";
import type { AgentJudge, JudgeOutcome, JudgeTask } from "../../../src/runtime/merge-assist/agent-judge.js";
import { createMergeAssistService } from "../../../src/runtime/merge-assist/service.js";
import {
  candidate,
  candidateFiles,
  commit,
  compareAnswer,
  FakeGithub,
  FakeJudge,
  git,
  mergeAssistWorkspaces,
  writeFiles,
} from "./support/merge-assist-workspace.js";

const workspace = mergeAssistWorkspaces();

const MINE = candidate({ id: "candidate:errors-are-thrown-7a8b9c", decision: "API errors are thrown as exceptions.", author: "tbelanger" });
const THEIRS = candidate({ id: "candidate:name-files-by-resource-2c3d4e", decision: "Name API files after their resource." });

/** A judge whose comparisons wait until released, counting each. */
class HeldJudge implements AgentJudge {
  compares = 0;
  readonly #release = Promise.withResolvers<void>();
  readonly started = Promise.withResolvers<void>();

  readiness() {
    return { available: true, runner: "codex" as const, problem: null };
  }

  budget() {
    return { used: this.compares, limit: 40, windowMinutes: 60 };
  }

  async judge<T>(task: JudgeTask<T>): Promise<JudgeOutcome<T>> {
    this.compares += 1;
    this.started.resolve();
    await this.#release.promise;
    return { ok: true, value: task.parse(compareAnswer(task as JudgeTask<unknown>)), runner: "codex", cached: false };
  }

  release(): void {
    this.#release.resolve();
  }
}

// Each test builds Git repositories and compiles rules in the Engine; under a full run that outlasts the default 5 s.
describe("Merge Assist service", { timeout: 30_000 }, () => {
  it("reports a checkout's state without running a judge or a scan", async () => {
    const ws = await workspace();
    const judge = new FakeJudge(() => {
      throw new Error("status never judges");
    });
    const github = new FakeGithub();
    const status = await ws.service({ judge, github }).status(ws.repo);

    expect(status).toEqual({
      repositoryKey: ws.repositoryKey,
      checkout: ws.repo,
      branch: "mine",
      baseBranch: "main",
      enabled: false,
      readiness: {
        judge: { available: true, runner: "claude", problem: null },
        github: { state: "not-github", login: null, repository: null },
        candidatesIgnored: false,
        rulesActive: true,
      },
      budget: { used: 0, limit: 40, windowMinutes: 60 },
      scanning: false,
      lastScan: null,
      own: [],
      findings: [],
      conflicts: [],
      decisionLog: { entries: [], problems: [] },
    });
    expect(judge.tasks).toEqual([]);
    // A repository off GitHub never runs gh.
    expect(github.readinessReads).toBe(0);
  });

  it("reads gh's sign-in at most every five minutes for status, and afresh for a scan", async () => {
    const ws = await workspace({ remote: "https://github.com/acme/app.git" });
    let now = Date.parse("2026-11-03T14:00:00Z");
    const github = new FakeGithub();
    const service = ws.service({ github, now: () => now });

    expect((await service.status(ws.repo)).readiness.github).toEqual({ state: "signed-in", login: "tbelanger", repository: "acme/app" });
    await Promise.all([service.status(ws.repo), service.status(ws.repo)]);
    expect(github.readinessReads).toBe(1);
    // Another process over the same database reuses it too.
    await ws.service({ github, now: () => now }).status(ws.repo);
    expect(github.readinessReads).toBe(1);
    now += 5 * 60_000;
    await service.status(ws.repo);
    expect(github.readinessReads).toBe(2);

    await service.setEnabled(ws.repo, true);
    await service.scan(ws.repo);
    expect(github.readinessReads).toBe(3);
  });

  it("refuses a folder that is not a Git checkout, and one Rerail has not recorded, until turned on there", async () => {
    const ws = await workspace();
    const service = ws.service();
    const folder = await mkdtemp(join(tmpdir(), "rerail-merge-assist-plain-"));
    try {
      await expect(service.status(folder)).rejects.toMatchObject({ code: "not-a-checkout", message: `Merge Assist works in a Git checkout, and ${folder} is not one.` });
      const unrecorded = join(ws.folder, "unrecorded");
      git(ws.folder, "init", "-q", "-b", "main", unrecorded);
      await expect(service.status(unrecorded)).rejects.toMatchObject({ code: "not-a-checkout" });
      expect(await service.setEnabled(unrecorded, true)).toMatchObject({ checkout: unrecorded, enabled: true });
      expect((await service.status(unrecorded)).readiness.rulesActive).toBe(false);
    } finally {
      await rm(folder, { recursive: true, force: true });
    }
  });

  it("refuses to scan or resolve a finding while Merge Assist is off", async () => {
    const ws = await workspace();
    const service = ws.service();
    await expect(service.scan(ws.repo)).rejects.toMatchObject({ code: "disabled" });
    await expect(service.resolveFinding(ws.repo, "ma-000000000000", "dismiss")).rejects.toMatchObject({ code: "disabled" });
  });

  it("runs one scan of a checkout at a time, and reports it running", async () => {
    const ws = await workspace();
    writeFiles(ws.repo, candidateFiles(MINE));
    commit(ws.worktree("jbarnes-files"), candidateFiles(THEIRS));
    const judge = new HeldJudge();
    const service = ws.service({ judge });
    await service.setEnabled(ws.repo, true);

    const first = service.scan(ws.repo);
    await judge.started.promise;
    const second = service.scan(ws.repo);
    expect((await service.status(ws.repo)).scanning).toBe(true);
    // Another process sees the scan running and starts none of its own: it answers at once with what is there.
    const elsewhere = createMergeAssistService({ database: ws.database, ruleCatalog: ws.ruleCatalog, judge, github: new FakeGithub() });
    expect(await elsewhere.scan(ws.repo)).toMatchObject({ scanning: true, findings: [] });
    judge.release();
    const statuses = await Promise.all([first, second]);

    expect(judge.compares).toBe(1);
    expect(statuses.map(status => status.findings.length)).toEqual([1, 1]);
    expect(statuses[0]!.scanning).toBe(false);
  });

  it("scans again once another process's mark is ten minutes old", async () => {
    const ws = await workspace();
    let now = Date.parse("2026-11-03T14:00:00Z");
    const service = ws.service({ now: () => now });
    await service.setEnabled(ws.repo, true);
    ws.database.insert(mergeAssistState).values({ key: `scanning:${ws.repo}`, valueJson: JSON.stringify({ since: now }), updatedAt: now }).run();

    expect(await service.scan(ws.repo)).toMatchObject({ scanning: true, lastScan: null });
    now += 10 * 60_000;
    expect(await service.scan(ws.repo)).toMatchObject({ scanning: false, lastScan: { at: "2026-11-03T14:10:00Z" } });
  });
});
