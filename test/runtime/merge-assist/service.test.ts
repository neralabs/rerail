import { spawnSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { eq } from "drizzle-orm";
import { describe, expect, it } from "vitest";
import { removeFolder } from "../../support/platform/remove-folder.js";
import { mergeAssistState, openRerailDatabase, type GithubReadiness, type MergeAssistStatus } from "../../../src/engine/index.js";
import { PULL_REQUEST_LIMIT, type GithubRepository, type PullRequestListing, type PullRequestSummary } from "../../../src/runtime/merge-assist/github.js";
import { createMergeAssistService, type MergeAssistRuntime } from "../../../src/runtime/merge-assist/service.js";
import { commit, git } from "../../support/merge-assist/git.js";
import { FakeGithub, mergeAssistWorkspaces, type Workspace } from "../../support/merge-assist/workspace.js";
import { withExperiments } from "../../support/environment/experiments.js";

// Merge Assist is experimental: every test here runs with it on, as a daemon started with the variable does.
withExperiments("MERGE_ASSIST");

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

/**
 * `gh` whose sign-in reads, once held, wait until released, and then answer on a later turn of the event loop, as a
 * process's exit arrives: a caller sees the answer only by waiting for the read.
 */
class HeldReadiness extends FakeGithub {
  #release: PromiseWithResolvers<void> | undefined;
  readonly asked = Promise.withResolvers<void>();
  /** Reads started, held ones included (readinessReads counts them once they answer). */
  starts = 0;

  hold(): void {
    this.#release = Promise.withResolvers<void>();
  }

  override async readiness(repository: GithubRepository | null): Promise<GithubReadiness> {
    this.starts += 1;
    const held = this.#release;
    if (held !== undefined) {
      this.asked.resolve();
      await held.promise;
      await new Promise<void>(resolve => setImmediate(resolve));
    }
    return await super.readiness(repository);
  }

  release(): void {
    this.#release?.resolve();
    this.#release = undefined;
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
        github: { state: "unchecked", login: null, repository: null, readAt: null },
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

    expect((await service.status(ws.repo)).readiness.github).toEqual({ state: "unchecked", login: null, repository: "acme/app", readAt: null });
    expect(await service.settings(ws.repo)).toEqual({ hook: null });
    expect(github.readinessReads).toBe(0);

    await service.setEnabled(ws.repo, true);
    await service.setEnabled(ws.repo, false);
    const readsWhileOn = github.readinessReads;
    expect((await service.status(ws.repo)).readiness.github.state).toBe("unchecked");
    expect(github.readinessReads).toBe(readsWhileOn);
  });

  it("shows gh's sign-in as last read, with when, and never waits on gh for a status read once one is kept", async () => {
    const ws = await workspace({ remote: "https://github.com/acme/app.git" });
    let now = Date.parse("2026-11-03T14:00:00Z");
    const github = new HeldReadiness();
    const service = ws.service({ github, now: () => now });
    // Turning it on reads gh's sign-in, so the person sees at once whether open pull requests can be read.
    const on = await service.setEnabled(ws.repo, true);
    const starts = github.starts;

    // Within five minutes, status reads from this process and another one over the same database run no gh.
    now += 5 * 60_000 - 1_000;
    const within = await Promise.all([service.status(ws.repo), ws.service({ github, now: () => now }).status(ws.repo)]);
    expect(github.starts).toBe(starts);
    // Hours later, with gh stalled, they answer at once with what gh said then, and each process starts one read of gh
    // for its next status.
    now += 3 * 60 * 60_000;
    github.hold();
    const other = ws.service({ github, now: () => now });
    const later = await Promise.all([service.status(ws.repo), service.status(ws.repo), other.status(ws.repo)]);
    expect(github.starts).toBe(starts + 2);
    github.release();
    await Promise.all([service.settled(), other.settled()]);
    expect(on.readiness.github).toEqual({ state: "signed-in", login: "tbelanger", repository: "acme/app", readAt: "2026-11-03T14:00:00Z", readAgainInMs: 5 * 60_000 });
    // The same reading each time, with how long until a status reads gh again: 0 once due, while that read runs.
    expect([...within, ...later].map(status => status.readiness.github)).toEqual([1_000, 1_000, 0, 0, 0].map(readAgainInMs => ({ ...on.readiness.github, readAgainInMs })));
  });

  it("shows a token GitHub refuses at the status after the one that finds the reading five minutes old, and that one waits on no gh", async () => {
    const ws = await workspace({ remote: "https://github.com/acme/app.git" });
    let now = Date.parse("2026-11-03T14:00:00Z");
    const github = new HeldReadiness();
    const service = ws.service({ github, now: () => now });
    const on = await service.setEnabled(ws.repo, true);
    const starts = github.starts;

    // The token is revoked: gh auth status fails, and no file of gh's moves.
    github.options.readinessError = true;
    now += 5 * 60_000 - 1_000;
    const within = await service.status(ws.repo);
    expect(github.starts).toBe(starts);
    // Past five minutes, with gh slow, two status reads answer with the kept reading and its time; one gh run serves both.
    now += 1_000;
    github.hold();
    const past = await Promise.all([service.status(ws.repo), service.status(ws.repo)]);
    expect(github.starts).toBe(starts + 1);
    github.release();
    await service.settled();
    const next = await service.status(ws.repo);
    expect(github.starts).toBe(starts + 1);
    expect([within, ...past].map(status => status.readiness.github)).toEqual([1_000, 0, 0].map(readAgainInMs => ({ ...on.readiness.github, readAgainInMs })));
    expect(next.readiness.github).toEqual({ state: "unavailable", login: null, repository: "acme/app", readAt: "2026-11-03T14:05:00Z", readAgainInMs: 60_000 });
  });

  it("keeps one reading per gh environment, so a CLI and a daemon whose token variables differ each find their own", async () => {
    const ws = await workspace({ remote: "https://github.com/acme/app.git" });
    let now = Date.parse("2026-11-03T14:00:00Z");
    const daemonGh = new FakeGithub({ signInEnvironment: "the daemon's, no token variable" });
    const shellGh = new FakeGithub({ signInEnvironment: "a shell with GITHUB_TOKEN", readiness: { state: "signed-in", login: "jbarnes", repository: "acme/app" } });
    const daemon = ws.service({ github: daemonGh, now: () => now });
    const on = await daemon.setEnabled(ws.repo, true);
    // A CLI process for each command, as `rerail merge-assist status` runs.
    const cli = async (): Promise<MergeAssistStatus> => await ws.service({ github: shellGh, now: () => now, answersOnce: true }).status(ws.repo);

    // The shell's first status reads its own gh once; from then on each status after the other's runs none.
    const first = await cli();
    const shown: Array<MergeAssistStatus["readiness"]["github"]> = [];
    for (let round = 0; round < 3; round += 1) {
      now += 60_000;
      shown.push((await daemon.status(ws.repo)).readiness.github, (await cli()).readiness.github);
    }
    expect([daemonGh.readinessReads, shellGh.readinessReads]).toEqual([1, 1]);
    expect(first.readiness.github).toEqual({ state: "signed-in", login: "jbarnes", repository: "acme/app", readAt: "2026-11-03T14:00:00Z", readAgainInMs: 5 * 60_000 });
    expect(shown).toEqual([1, 2, 3].flatMap(minutes => [on, first].map(status => ({ ...status.readiness.github, readAgainInMs: (5 - minutes) * 60_000 }))));
  });

  it("keeps the readings of four gh environments, the one written longest ago dropped first", async () => {
    const ws = await workspace({ remote: "https://github.com/acme/app.git" });
    const now = Date.parse("2026-11-03T14:00:00Z");
    const environments = ["first", "second", "third", "fourth", "fifth"].map(name => new FakeGithub({ signInEnvironment: name }));
    for (const [index, github] of environments.entries()) {
      const service = ws.service({ github, now: () => now });
      await (index === 0 ? service.setEnabled(ws.repo, true) : service.status(ws.repo));
    }

    // The newest four answer from what they kept; the first, dropped when the fifth was kept, reads gh again.
    for (const github of environments.toReversed()) await ws.service({ github, now: () => now }).status(ws.repo);
    expect(environments.map(github => github.readinessReads)).toEqual([2, 1, 1, 1, 1]);
  });

  it("keeps the stat of gh's hosts file that answers after gh did, so the statuses after run no gh", async () => {
    const ws = await workspace({ remote: "https://github.com/acme/app.git" });
    const now = Date.parse("2026-11-03T14:00:00Z");
    // gh is not installed: it answers with no process, before the stat taken as it started.
    const github = new FakeGithub({ readiness: { state: "missing", login: null, repository: "acme/app" }, signInStatSlow: true });
    const service = ws.service({ github, now: () => now });
    await service.setEnabled(ws.repo, true);
    const reads = github.readinessReads;

    await service.status(ws.repo);
    await ws.service({ github, now: () => now, answersOnce: true }).status(ws.repo);
    expect(github.readinessReads).toBe(reads);
  });

  it("answers a status with the kept reading and its time when the stat of gh's hosts file does not answer", async () => {
    const ws = await workspace({ remote: "https://github.com/acme/app.git" });
    let now = Date.parse("2026-11-03T14:00:00Z");
    const github = new FakeGithub();
    const service = ws.service({ github, now: () => now });
    const on = await service.setEnabled(ws.repo, true);
    const reads = github.readinessReads;

    // gh's folder sits on a drive that stops answering: the daemon's status and the CLI's answer once their Git reads end.
    github.options.signInStatStalls = true;
    now += 60_000;
    const stalled = await Promise.all([service.status(ws.repo), ws.service({ github, now: () => now, answersOnce: true }).status(ws.repo)]);
    expect(github.readinessReads).toBe(reads);
    // With no reading kept, gh is read, and kept with a stat that never answered: once the drive answers, the next status
    // reads gh again, and the one after runs none.
    ws.database.delete(mergeAssistState).where(eq(mergeAssistState.key, `github:${ws.repositoryKey}`)).run();
    await service.status(ws.repo);
    await service.status(ws.repo);
    expect(github.readinessReads).toBe(reads + 1);
    github.options.signInStatStalls = false;
    await service.status(ws.repo);
    await service.status(ws.repo);
    expect(github.readinessReads).toBe(reads + 2);
    expect(stalled.map(status => status.readiness.github)).toEqual([on, on].map(status => ({ ...status.readiness.github, readAgainInMs: 4 * 60_000 })));
  });

  it("starts a listing from the kept reading while it is under five minutes old and gh's hosts file holds, else reads gh's sign-in first", async () => {
    const ws = await workspace({ remote: "https://github.com/acme/app.git" });
    let now = Date.parse("2026-11-03T14:00:00Z");
    const github = new FakeGithub();
    // Each listing from a service of its own, as the listing of the last five minutes would serve one service again.
    const listing = async (): Promise<boolean> => await ws.service({ github, now: () => now }).detectAfterEdit(ws.repo);
    await ws.service({ github, now: () => now }).setEnabled(ws.repo, true);
    const reads = github.readinessReads;

    // A minute later the agent's edit lists the pull requests: gh's sign-in is as kept, and no gh reads it.
    now += 60_000;
    expect(await listing()).toBe(true);
    expect(github.readinessReads).toBe(reads);

    // The person signs gh out, which rewrites its hosts file: the next listing reads gh first, and a status shows it.
    github.options.readiness = { state: "signed-out", login: null, repository: "acme/app" };
    github.options.signInStamp = "after gh auth logout";
    now += 60_000;
    expect(await listing()).toBe(true);
    expect(github.readinessReads).toBe(reads + 1);
    expect((await ws.service({ github, now: () => now }).status(ws.repo)).readiness.github).toEqual({ state: "signed-out", login: null, repository: "acme/app", readAt: "2026-11-03T14:02:00Z", readAgainInMs: 5 * 60_000 });

    // Five minutes after that reading, the next listing reads gh first.
    delete github.options.readiness;
    now += 5 * 60_000;
    expect(await listing()).toBe(true);
    expect(github.readinessReads).toBe(reads + 2);
  });

  it("says the stamp of gh's sign-in a status compares, with no gh run, only while Merge Assist is on for a GitHub repository", async () => {
    const ws = await workspace({ remote: "https://github.com/acme/app.git" });
    const github = new FakeGithub();
    const service = ws.service({ github });
    expect(await service.signInStamp(ws.repo)).toBe("");
    await service.setEnabled(ws.repo, true);
    const reads = github.readinessReads;

    const before = await service.signInStamp(ws.repo);
    github.options.signInStamp = "after gh auth logout";
    const after = await service.signInStamp(ws.repo);

    expect([before, after === before, github.readinessReads]).toEqual([expect.stringContaining("hosts.yml as first written"), false, reads]);
    expect(await service.signInStamp(ws.folder)).toBe("");
    await service.setEnabled(ws.repo, false);
    expect(await service.signInStamp(ws.repo)).toBe("");
    const local = await workspace();
    const localService = local.service({ github });
    await localService.setEnabled(local.repo, true);
    expect(await localService.signInStamp(local.repo)).toBe("");
  });

  it("reads gh's sign-in afresh at a scan, and for a listing whose stat of gh's hosts file has not answered when its listing did", async () => {
    const ws = await workspace({ remote: "https://github.com/acme/app.git" });
    let now = Date.parse("2026-11-03T14:00:00Z");
    const github = new FakeGithub();
    const service = ws.service({ github, now: () => now });
    await service.setEnabled(ws.repo, true);
    const reads = github.readinessReads;

    now += 60_000;
    expect((await service.scan(ws.repo)).readiness.github).toEqual({ state: "signed-in", login: "tbelanger", repository: "acme/app", readAt: "2026-11-03T14:01:00Z", readAgainInMs: 5 * 60_000 });
    expect(github.readinessReads).toBe(reads + 1);
    github.options.signInStatStalls = true;
    expect(await ws.service({ github, now: () => now }).detectAfterEdit(ws.repo)).toBe(true);
    expect(github.readinessReads).toBe(reads + 2);

    // Turned off and on again: what gh said before is not kept.
    github.options.signInStatStalls = false;
    expect((await service.setEnabled(ws.repo, false)).readiness.github).toEqual({ state: "unchecked", login: null, repository: "acme/app", readAt: null });
    await service.setEnabled(ws.repo, true);
    expect(github.readinessReads).toBe(reads + 3);
  });

  it("says how long until it reads gh's sign-in again, on its own clock, so a page can ask then whatever its clock says", async () => {
    const ws = await workspace({ remote: "https://github.com/acme/app.git" });
    let now = Date.parse("2026-11-03T14:00:00Z");
    const service = ws.service({ github: new FakeGithub(), now: () => now });
    await service.setEnabled(ws.repo, true);

    now += 4 * 60_000;
    const reused = (await service.status(ws.repo)).readiness.github.readAgainInMs;
    // Due: the status shows the kept reading and starts the read of gh, which the next status shows.
    now += 60_000;
    const due = (await service.status(ws.repo)).readiness.github.readAgainInMs;
    await service.settled();
    const readAgain = (await service.status(ws.repo)).readiness.github.readAgainInMs;

    expect([reused, due, readAgain]).toEqual([60_000, 0, 5 * 60_000]);
  });

  it("says when gh could not tell, makes it a problem of the scan, and asks gh again past a minute without waiting for it", async () => {
    const ws = await workspace({ remote: "https://github.com/acme/app.git" });
    let now = Date.parse("2026-11-03T14:00:00Z");
    const github = new HeldReadiness({ readinessError: true });
    const service = ws.service({ github, now: () => now });
    const status = await service.setEnabled(ws.repo, true);
    expect((await service.scan(ws.repo)).lastScan).toMatchObject({ peers: { pullRequests: 0 }, problems: [expect.any(String)] });
    const starts = github.starts;

    // Within the minute a status runs no gh.
    now += 59_000;
    const within = await service.status(ws.repo);
    expect(github.starts).toBe(starts);
    // The network is back and gh is slow: past the minute, status reads answer at once with the kept reading and its
    // time, and one gh run serves them all; the next status shows what it said.
    github.options.readinessError = false;
    github.hold();
    now += 1_000;
    const past = await Promise.all([service.status(ws.repo), service.status(ws.repo)]);
    expect(github.starts).toBe(starts + 1);
    github.release();
    // settled() waits for the read in flight: what gh said is kept before any status asks.
    await service.settled();
    const kept = ws.database.select().from(mergeAssistState).where(eq(mergeAssistState.key, `github:${ws.repositoryKey}`)).get();
    const next = await service.status(ws.repo);
    expect(github.starts).toBe(starts + 1);
    expect(JSON.parse(kept?.valueJson ?? "{}")).toMatchObject({ readiness: { state: "signed-in" } });
    expect(status.readiness.github).toEqual({ state: "unavailable", login: null, repository: "acme/app", readAt: "2026-11-03T14:00:00Z", readAgainInMs: 60_000 });
    expect(past.map(shown => shown.readiness.github)).toEqual([0, 0].map(readAgainInMs => ({ ...within.readiness.github, readAgainInMs })));
    expect(within.readiness.github).toEqual({ ...status.readiness.github, readAgainInMs: 1_000 });
    expect(next.readiness.github).toEqual({ state: "signed-in", login: "tbelanger", repository: "acme/app", readAt: "2026-11-03T14:01:00Z", readAgainInMs: 5 * 60_000 });
  });

  it("shows a gh auth logout, login or switch made outside Rerail at the next status read, and runs gh only then", async () => {
    const ws = await workspace({ remote: "https://github.com/acme/app.git" });
    let now = Date.parse("2026-11-03T14:00:00Z");
    const github = new FakeGithub();
    const service = ws.service({ github, now: () => now });
    await service.setEnabled(ws.repo, true);
    const reads = github.readinessReads;

    // Each rewrites gh's hosts file, which moves the stamp of gh's sign-in.
    now += 1_000;
    github.options.readiness = { state: "signed-out", login: null, repository: "acme/app" };
    github.options.signInStamp = "after gh auth logout";
    const loggedOut = await service.status(ws.repo);
    now += 1_000;
    github.options.readiness = { state: "signed-in", login: "jbarnes", repository: "acme/app" };
    github.options.signInStamp = "after gh auth login as jbarnes";
    const loggedIn = await service.status(ws.repo);
    now += 1_000;
    delete github.options.readiness;
    github.options.signInStamp = "after gh auth switch to tbelanger";
    const switched = await service.status(ws.repo);
    // The file unchanged, status reads from this process and another one within five minutes run no gh.
    now += 5 * 60_000 - 1_000;
    const later = await Promise.all([service.status(ws.repo), ws.service({ github, now: () => now }).status(ws.repo)]);
    expect(github.readinessReads).toBe(reads + 3);
    expect(loggedOut.readiness.github).toEqual({ state: "signed-out", login: null, repository: "acme/app", readAt: "2026-11-03T14:00:01Z", readAgainInMs: 5 * 60_000 });
    expect(loggedIn.readiness.github).toEqual({ state: "signed-in", login: "jbarnes", repository: "acme/app", readAt: "2026-11-03T14:00:02Z", readAgainInMs: 5 * 60_000 });
    expect(switched.readiness.github).toEqual({ state: "signed-in", login: "tbelanger", repository: "acme/app", readAt: "2026-11-03T14:00:03Z", readAgainInMs: 5 * 60_000 });
    expect(later.map(shown => shown.readiness.github)).toEqual([switched, switched].map(status => ({ ...status.readiness.github, readAgainInMs: 1_000 })));
  });

  it("asks gh again for a missing gh or a sign-out past five minutes, in the answer of a process that answers once", async () => {
    const ws = await workspace({ remote: "https://github.com/acme/app.git" });
    let now = Date.parse("2026-11-03T14:00:00Z");
    const github = new FakeGithub({ readiness: { state: "missing", login: null, repository: "acme/app" } });
    const service = ws.service({ github, now: () => now });
    await service.setEnabled(ws.repo, true);
    const reads = github.readinessReads;

    // gh is installed, not signed in yet: no file of gh's tells it.
    github.options.readiness = { state: "signed-out", login: null, repository: "acme/app" };
    now += 5 * 60_000 - 1_000;
    const within = await service.status(ws.repo);
    expect(github.readinessReads).toBe(reads);
    // The person's `rerail merge-assist status` past the five minutes shows what gh says now.
    now += 1_000;
    const once = await ws.service({ github, now: () => now, answersOnce: true }).status(ws.repo);
    expect(github.readinessReads).toBe(reads + 1);
    // Five minutes on, a status here answers with the kept sign-out and asks gh for the next one.
    now += 5 * 60_000;
    const past = await service.status(ws.repo);
    await service.settled();
    const next = await service.status(ws.repo);
    expect(github.readinessReads).toBe(reads + 2);
    expect(within.readiness.github).toEqual({ state: "missing", login: null, repository: "acme/app", readAt: "2026-11-03T14:00:00Z", readAgainInMs: 1_000 });
    expect(once.readiness.github).toEqual({ state: "signed-out", login: null, repository: "acme/app", readAt: "2026-11-03T14:05:00Z", readAgainInMs: 5 * 60_000 });
    expect(past.readiness.github).toEqual({ ...once.readiness.github, readAgainInMs: 0 });
    expect(next.readiness.github).toEqual({ state: "signed-out", login: null, repository: "acme/app", readAt: "2026-11-03T14:10:00Z", readAgainInMs: 5 * 60_000 });
  });

  it("reads gh once more for a reading an older build kept without the stamp of gh's sign-in", async () => {
    const ws = await workspace({ remote: "https://github.com/acme/app.git" });
    const now = Date.parse("2026-11-03T14:00:00Z");
    const github = new FakeGithub();
    const service = ws.service({ github, now: () => now });
    await service.setEnabled(ws.repo, true);
    const reads = github.readinessReads;
    const older = { at: now, readiness: { state: "signed-in", login: "tbelanger", repository: "acme/app" } };
    ws.database.update(mergeAssistState).set({ valueJson: JSON.stringify(older) }).where(eq(mergeAssistState.key, `github:${ws.repositoryKey}`)).run();

    await service.status(ws.repo);
    await service.status(ws.repo);
    expect(github.readinessReads).toBe(reads + 1);
  });

  it("keeps nothing gh says after the service stopped", async () => {
    const ws = await workspace({ remote: "https://github.com/acme/app.git" });
    let now = Date.parse("2026-11-03T14:00:00Z");
    const github = new HeldReadiness({ readinessError: true });
    const service = ws.service({ github, now: () => now });
    const status = await service.setEnabled(ws.repo, true);
    github.hold();
    now += 60_000;
    await service.status(ws.repo);
    await service.stop();

    github.options.readinessError = false;
    github.release();
    await service.settled();
    const after = await ws.service({ github: new FakeGithub({ readinessError: true }), now: () => now }).status(ws.repo);
    expect(after.readiness.github).toEqual({ ...status.readiness.github, readAgainInMs: 0 });
  });

  it("reads gh's sign-in for a status only while none is kept, and once for listings at the same time", async () => {
    const ws = await workspace({ remote: "https://github.com/acme/app.git" });
    const github = new HeldReadiness();
    const service = ws.service({ github });
    await service.setEnabled(ws.repo, true);
    const reads = github.readinessReads;

    // The kept reading is gone, as when the repository took a new key: the next status reads gh, the one after not.
    ws.database.delete(mergeAssistState).where(eq(mergeAssistState.key, `github:${ws.repositoryKey}`)).run();
    expect((await service.status(ws.repo)).readiness.github.state).toBe("signed-in");
    await service.status(ws.repo);
    expect(github.readinessReads).toBe(reads + 1);

    // Two checkouts of the repository scanned at once: both listings ask while one read is out, and it serves both.
    const other = ws.worktree("jbarnes-docs");
    github.hold();
    const scans = [service.scan(ws.repo), service.scan(other)];
    github.release();
    await Promise.all(scans);
    expect(github.readinessReads).toBe(reads + 2);
  });

  it("keeps nothing gh says after Merge Assist went off, so turning it on again reads gh afresh", async () => {
    const ws = await workspace({ remote: "https://github.com/acme/app.git" });
    const github = new HeldReadiness();
    const service = ws.service({ github });
    await service.setEnabled(ws.repo, true);
    ws.database.delete(mergeAssistState).where(eq(mergeAssistState.key, `github:${ws.repositoryKey}`)).run();
    github.hold();
    const reading = service.status(ws.repo);
    await github.asked.promise;

    await ws.service({ github: new FakeGithub() }).setEnabled(ws.repo, false);
    github.release();
    await reading;
    const reads = github.readinessReads;
    await service.setEnabled(ws.repo, true);
    expect(github.readinessReads).toBe(reads + 1);
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
      await removeFolder(folder);
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
