import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { mergeAssistState } from "../../../src/engine/index.js";
import type { PullRequestSummary } from "../../../src/runtime/merge-assist/github.js";
import { createMergeAssistService } from "../../../src/runtime/merge-assist/service.js";
import { commit, git } from "../../support/git.js";
import { FakeGithub, mergeAssistWorkspaces } from "../../support/merge-assist/workspace.js";

const workspace = mergeAssistWorkspaces();

/** `gh` whose pull request listings wait until released, counting each. */
class HeldGithub extends FakeGithub {
  listings = 0;
  readonly #release = Promise.withResolvers<void>();
  readonly started = Promise.withResolvers<void>();

  override async openPullRequests(): Promise<PullRequestSummary[]> {
    this.listings += 1;
    this.started.resolve();
    await this.#release.promise;
    return await super.openPullRequests();
  }

  release(): void {
    this.#release.resolve();
  }
}

// Each test builds Git repositories; under a full run that outlasts the default 5 s.
describe("Merge Assist service", { timeout: 30_000 }, () => {
  it("reports a checkout's state without running a scan", async () => {
    const ws = await workspace();
    const github = new FakeGithub();
    const status = await ws.service({ github }).status(ws.repo);

    expect(status).toEqual({
      repositoryKey: ws.repositoryKey,
      checkout: ws.repo,
      branch: "mine",
      baseBranch: "main",
      enabled: false,
      settings: { hook: null },
      readiness: {
        github: { state: "not-github", login: null, repository: null },
      },
      scanning: false,
      lastScan: null,
      conflicts: [],
    });
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
      expect(await service.status(unrecorded)).toMatchObject({ checkout: unrecorded, enabled: true });
    } finally {
      await rm(folder, { recursive: true, force: true });
    }
  });

  it("refuses to scan while Merge Assist is off", async () => {
    const ws = await workspace();
    const service = ws.service();
    await expect(service.scan(ws.repo)).rejects.toMatchObject({ code: "disabled" });
  });

  it("records each scan: when, how many branches and pull requests it compared, and what it could not read", async () => {
    const ws = await workspace({ remote: "https://github.com/acme/app.git" });
    const theirs = ws.worktree("jbarnes-orders");
    const head = commit(theirs, { "src/api/orders.ts": "export const orders = () => client.get(\"/v2/orders\");\n" }, "Version the orders path");
    commit(ws.worktree("jbarnes-docs"), { "README.md": "app\n" }, "Readme");
    const pull: PullRequestSummary = {
      number: 12, title: "Version the API", url: "https://github.com/acme/app/pull/12", headRefName: "jbarnes-orders", headCommit: head,
      baseRefName: "main", author: "jbarnes", isDraft: false, updatedAt: "2026-11-03T14:00:00Z",
    };
    const now = Date.parse("2026-11-03T14:00:00Z");
    const service = ws.service({ github: new FakeGithub({ pullRequests: [pull] }), now: () => now });
    await service.setEnabled(ws.repo, true);

    // A pull request's head counts once, as the pull request: the branch holding it is not counted again.
    const status = await service.scan(ws.repo);
    expect(status.lastScan).toEqual({ at: "2026-11-03T14:00:00Z", peers: { branches: 1, pullRequests: 1 }, problems: [] });
    expect(status.conflicts).toEqual([expect.objectContaining({ theirs: expect.objectContaining({ label: "PR #12", ref: "refs/pull/12/head", author: "jbarnes" }) })]);

    // gh failing leaves the branches compared, and says why the pull requests are not.
    const failing = new class extends FakeGithub {
      override async openPullRequests(): Promise<PullRequestSummary[]> {
        throw new Error("HTTP 502");
      }
    }();
    const failed = await ws.service({ github: failing, now: () => now }).scan(ws.repo);
    expect(failed.lastScan).toEqual({
      at: "2026-11-03T14:00:00Z", peers: { branches: 2, pullRequests: 0 }, problems: ["Rerail could not list open pull requests: HTTP 502"],
    });
  });

  it("reports a checkout without a base branch as a problem of its scan, with no conflicts", async () => {
    const ws = await workspace();
    git(ws.repo, "branch", "-m", "main", "trunk");
    const service = ws.service();
    await service.setEnabled(ws.repo, true);
    expect(await service.scan(ws.repo)).toMatchObject({
      baseBranch: null,
      lastScan: { peers: { branches: 0, pullRequests: 0 }, problems: [expect.stringMatching(/^No base branch found/u)] },
      conflicts: [],
    });
  });

  it("runs one scan of a checkout at a time, and reports it running", async () => {
    const ws = await workspace({ remote: "https://github.com/acme/app.git" });
    commit(ws.worktree("jbarnes-orders"), { "src/api/orders.ts": "export const orders = () => client.get(\"/v2/orders\");\n" }, "Version the orders path");
    const github = new HeldGithub();
    const service = ws.service({ github });
    await service.setEnabled(ws.repo, true);

    const first = service.scan(ws.repo);
    await github.started.promise;
    const second = service.scan(ws.repo);
    expect((await service.status(ws.repo)).scanning).toBe(true);
    // Another process sees the scan running and starts none of its own: it answers at once with what is there.
    const elsewhere = createMergeAssistService({ database: ws.database, github });
    expect(await elsewhere.scan(ws.repo)).toMatchObject({ scanning: true, lastScan: null, conflicts: [] });
    github.release();
    const statuses = await Promise.all([first, second]);

    expect(github.listings).toBe(1);
    expect(statuses.map(status => status.conflicts.length)).toEqual([1, 1]);
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
