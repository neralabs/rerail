import { spawnSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { eq } from "drizzle-orm";
import { describe, expect, it } from "vitest";
import { mergeAssistState, openRerailDatabase } from "../../../src/engine/index.js";
import { PULL_REQUEST_LIMIT, type PullRequestListing, type PullRequestSummary } from "../../../src/runtime/merge-assist/github.js";
import { createMergeAssistService, type MergeAssistRuntime } from "../../../src/runtime/merge-assist/service.js";
import { commit, git } from "../../support/git.js";
import { FakeGithub, mergeAssistWorkspaces, type Workspace } from "../../support/merge-assist/workspace.js";

const workspace = mergeAssistWorkspaces();

/** `gh` whose pull request listings wait until released, counting each. */
class HeldGithub extends FakeGithub {
  listings = 0;
  readonly #release = Promise.withResolvers<void>();
  readonly started = Promise.withResolvers<void>();

  override async openPullRequests(): Promise<PullRequestListing> {
    this.listings += 1;
    this.started.resolve();
    await this.#release.promise;
    return await super.openPullRequests();
  }

  release(): void {
    this.#release.resolve();
  }
}

/** A conflict in the workspace: branch `jbarnes-orders` changes the line `mine` changes. */
function overlap(ws: Workspace): void {
  commit(ws.worktree("jbarnes-orders"), { "src/api/orders.ts": "export const orders = () => client.get(\"/v2/orders\");\n" }, "Version the orders path");
}

/** Set a hook command on the service's repository that logs each event it hears; returns the log's lines. */
async function loggedHook(ws: Workspace, service: MergeAssistRuntime): Promise<() => string[]> {
  const log = join(ws.folder, "hook.log");
  const script = "require(\"node:fs\").appendFileSync(process.argv[1], process.env.RERAIL_CONFLICT_EVENT + \"\\n\")";
  await service.updateSettings(ws.repo, { hook: { command: [process.execPath, "-e", script, log], timeoutSeconds: 10 } });
  return () => (existsSync(log) ? readFileSync(log, "utf8").split("\n").filter(line => line !== "") : []);
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
        github: { state: "unchecked", login: null, repository: null },
      },
      scanning: false,
      lastScan: null,
      conflicts: [],
    });
    expect(github.readinessReads).toBe(0);
  });

  it("runs no gh while Merge Assist is off for the repository", async () => {
    const ws = await workspace({ remote: "https://github.com/acme/app.git" });
    const github = new FakeGithub();
    const service = ws.service({ github });

    expect((await service.status(ws.repo)).readiness.github).toEqual({ state: "unchecked", login: null, repository: "acme/app" });
    expect(await service.settings(ws.repo)).toEqual({ hook: null });
    expect(github.readinessReads).toBe(0);

    await service.setEnabled(ws.repo, true);
    await service.setEnabled(ws.repo, false);
    const readsWhileOn = github.readinessReads;
    expect((await service.status(ws.repo)).readiness.github.state).toBe("unchecked");
    expect(github.readinessReads).toBe(readsWhileOn);
  });

  it("reads gh's sign-in at most every five minutes for status while on, afresh for a scan, and forgets it when off", async () => {
    const ws = await workspace({ remote: "https://github.com/acme/app.git" });
    let now = Date.parse("2026-11-03T14:00:00Z");
    const github = new FakeGithub();
    const service = ws.service({ github, now: () => now });
    await service.setEnabled(ws.repo, true);
    const reads = github.readinessReads;

    expect((await service.status(ws.repo)).readiness.github).toEqual({ state: "signed-in", login: "tbelanger", repository: "acme/app" });
    await Promise.all([service.status(ws.repo), service.status(ws.repo)]);
    // Another process over the same database reuses it too.
    await ws.service({ github, now: () => now }).status(ws.repo);
    expect(github.readinessReads).toBe(reads);
    now += 5 * 60_000;
    await service.status(ws.repo);
    expect(github.readinessReads).toBe(reads + 1);

    await service.scan(ws.repo);
    expect(github.readinessReads).toBe(reads + 2);

    // Turned off and on again within the five minutes: what gh said before is not reused.
    await service.setEnabled(ws.repo, false);
    await service.setEnabled(ws.repo, true);
    expect(github.readinessReads).toBe(reads + 3);
  });

  it("says when gh could not tell whether it is signed in, keeps no such answer, and makes it a problem of the scan", async () => {
    const ws = await workspace({ remote: "https://github.com/acme/app.git" });
    const github = new FakeGithub({ readinessError: true });
    const service = ws.service({ github });
    const status = await service.setEnabled(ws.repo, true);

    expect(status.readiness.github).toEqual({ state: "unavailable", login: null, repository: "acme/app" });
    await service.status(ws.repo);
    expect(github.readinessReads).toBe(2);
    expect((await service.scan(ws.repo)).lastScan).toMatchObject({ peers: { pullRequests: 0 }, problems: [expect.any(String)] });

    // Once gh answers, the next read says so: nothing kept the failure.
    github.options.readinessError = false;
    expect((await service.status(ws.repo)).readiness.github.state).toBe("signed-in");
  });

  it("makes a pull request listing cut at its limit a problem of the scan", async () => {
    const ws = await workspace({ remote: "https://github.com/acme/app.git" });
    const service = ws.service({ github: new FakeGithub({ truncated: true }) });
    await service.setEnabled(ws.repo, true);
    expect((await service.scan(ws.repo)).lastScan?.problems).toEqual([expect.stringContaining(String(PULL_REQUEST_LIMIT))]);
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

  it("keeps the person's hook per repository: shared by its worktrees, kept when off, none when unreadable", async () => {
    const ws = await workspace();
    const service = ws.service();
    const hook = { command: ["./notify.sh"], timeoutSeconds: 30 };
    await service.updateSettings(ws.repo, { hook });
    expect(await service.settings(ws.worktree("other", { record: true }))).toEqual({ hook });
    await service.setEnabled(ws.repo, true);
    await service.setEnabled(ws.repo, false);
    expect(await service.settings(ws.repo)).toEqual({ hook });

    ws.database.update(mergeAssistState).set({ valueJson: JSON.stringify({ hook: { command: [], timeoutSeconds: 0 } }) })
      .where(eq(mergeAssistState.key, `settings:${ws.repositoryKey}`)).run();
    expect(await service.settings(ws.repo)).toEqual({ hook: null });
    expect(service.hookHoldsNotes(ws.repo)).toBe(false);
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
      baseRefName: "main", author: "jbarnes", isDraft: false, updatedAt: "2026-11-03T14:00:00Z", crossRepository: false, headOwner: "acme",
    };
    const now = Date.parse("2026-11-03T14:00:00Z");
    const service = ws.service({ github: new FakeGithub({ pullRequests: [pull] }), now: () => now });
    await service.setEnabled(ws.repo, true);

    // A pull request's head counts once, as the pull request: the branch holding it is not counted again.
    const status = await service.scan(ws.repo);
    expect(status.lastScan).toEqual({ at: "2026-11-03T14:00:00Z", peers: { branches: 1, pullRequests: 1 }, problems: [] });
    expect(status.conflicts).toEqual([expect.objectContaining({ theirs: expect.objectContaining({ label: "PR #12", ref: "refs/pull/12/head", author: "jbarnes" }) })]);

    // gh failing leaves the worktrees' branches compared (remote-tracking ones too), and says why the pull requests are not.
    const failed = await ws.service({ github: new FakeGithub({ pullsError: true }), now: () => now }).scan(ws.repo);
    expect(failed.lastScan).toEqual({
      at: "2026-11-03T14:00:00Z", peers: { branches: 2, pullRequests: 0 }, problems: ["Rerail could not list open pull requests (HTTP 502), so remote-tracking branches committed in the last 14 days were compared instead of open pull requests."],
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
    overlap(ws);
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

  it("scans again once another process's mark is ten minutes old, or at once when that process is gone", async () => {
    const ws = await workspace();
    let now = Date.parse("2026-11-03T14:00:00Z");
    const service = ws.service({ now: () => now });
    await service.setEnabled(ws.repo, true);
    const mark = (pid: number): void => {
      ws.database.insert(mergeAssistState).values({ key: `scanning:${ws.repo}`, valueJson: JSON.stringify({ since: now, pid }), updatedAt: now }).run();
    };
    // This test's own process is alive: its mark holds.
    mark(process.pid);

    expect(await service.scan(ws.repo)).toMatchObject({ scanning: true, lastScan: null });
    now += 10 * 60_000;
    expect(await service.scan(ws.repo)).toMatchObject({ scanning: false, lastScan: { at: "2026-11-03T14:10:00Z" } });

    const gone = spawnSync(process.execPath, ["-e", ""]).pid;
    mark(gone);
    now += 60_000;
    expect(await service.scan(ws.repo)).toMatchObject({ scanning: false, lastScan: { at: "2026-11-03T14:11:00Z" } });
  });

  it("lets go of a scan still waiting for gh when it stops, so another process scans at once", async () => {
    const ws = await workspace({ remote: "https://github.com/acme/app.git" });
    const github = new HeldGithub();
    const database = openRerailDatabase(ws.databasePath);
    const service = createMergeAssistService({ database, github });
    await service.setEnabled(ws.repo, true);
    const scanning = service.scan(ws.repo).catch((error: unknown) => error);
    await github.started.promise;

    await service.stop(100);
    database.$client.close();
    expect((await ws.service().status(ws.repo)).scanning).toBe(false);
    // gh answering afterwards finds the database closed and records nothing.
    github.release();
    expect(await scanning).toBeInstanceOf(Error);
    expect((await ws.service().status(ws.repo)).lastScan).toBeNull();
  });

  it("tells the hook a conflict closed once when two dismiss it at once", async () => {
    const ws = await workspace();
    overlap(ws);
    const first = ws.service();
    const second = ws.service();
    await first.setEnabled(ws.repo, true);
    const hook = await loggedHook(ws, first);
    const [conflict] = (await first.scan(ws.repo)).conflicts;
    await first.settled();

    const results = await Promise.allSettled([first.dismissConflict(ws.repo, conflict!.id), second.dismissConflict(ws.repo, conflict!.id)]);
    await Promise.all([first.settled(), second.settled()]);
    expect(results.map(result => result.status).sort()).toEqual(["fulfilled", "rejected"]);
    expect(hook()).toEqual(["detected", "closed"]);
  });

  it("tells the hook every open conflict closed when Merge Assist is turned off", async () => {
    const ws = await workspace();
    overlap(ws);
    const service = ws.service();
    await service.setEnabled(ws.repo, true);
    const hook = await loggedHook(ws, service);
    expect((await service.scan(ws.repo)).conflicts).toHaveLength(1);
    await service.settled();

    expect((await service.setEnabled(ws.repo, false)).conflicts).toEqual([]);
    await service.settled();
    expect(hook()).toEqual(["detected", "closed"]);
  });
});
