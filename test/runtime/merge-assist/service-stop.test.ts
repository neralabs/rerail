import { type ChildProcess, execFileSync, spawn } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { access } from "node:fs/promises";
import { join, resolve } from "node:path";
import { describe, expect, it, onTestFinished, vi } from "vitest";
import { type MergeAssistStatus, openRerailDatabase } from "../../../src/engine/index.js";
import { createMergeAssistService, type GithubClient, type MergeAssistRuntime, type PullRequestSummary } from "../../../src/runtime/index.js";
import { withExperiments } from "../../support/environment/experiments.js";
import { git } from "../../support/merge-assist/git.js";
import { FakeGithub, mergeAssistWorkspaces, type Workspace } from "../../support/merge-assist/workspace.js";

// Every process Rerail starts goes through this module: the test stops the service as a pass starts a given Git read,
// and sees every Git process started after. On Windows the stop ends a running Git with taskkill, which is no Git read.
vi.mock("node:child_process", { spy: true });
// A pass checks that each worktree's folder is there with `access`: a test holds that call as a drive that stopped
// answering would.
vi.mock("node:fs/promises", { spy: true });

// Merge Assist is experimental: every test here runs with it on, as a daemon started with the variable does.
withExperiments("MERGE_ASSIST");

const workspace = mergeAssistWorkspaces();

/** How many branches the checkout's work conflicts with: a pass reads Git several times for each. */
const PEERS = 12;

/**
 * `origin/peer-<n>` for each of {@link PEERS}, one commit each off `main`, committed now, adding `src/api/orders.ts`
 * as the checkout's own commit does, with other text: each conflicts with the checkout's work. Answers their tips.
 */
function addPeers(ws: Workspace): string[] {
  return Array.from({ length: PEERS }, (_, index) => commitPeer(ws, index, `Orders, take ${index}`));
}

/**
 * Commits `origin/peer-<index>` anew off `main` with `subject`, its change to `src/api/orders.ts` as {@link addPeers}
 * makes it: the same conflict, with another tip. Answers the tip.
 */
function commitPeer(ws: Workspace, index: number, subject: string): string {
  const now = Math.floor(Date.now() / 1000);
  const data = (text: string): string => `data ${Buffer.byteLength(text)}\n${text}\n`;
  const commit = [
    `commit refs/remotes/origin/peer-${index}`,
    `committer Jo Barnes <jo@example.com> ${now} +0000`,
    `${data(subject)}from refs/heads/main^0`,
    "M 100644 inline src/api/orders.ts",
    data(`export const orders = () => client.get("/v${index + 2}/orders");\n`),
  ].join("\n");
  execFileSync("git", ["fast-import", "--quiet", "--force"], { cwd: ws.repo, input: commit, env: { ...process.env, GIT_CONFIG_NOSYSTEM: "1" } });
  return git(ws.repo, "rev-parse", `refs/remotes/origin/peer-${index}`);
}

/**
 * Every peer's conflict stored by a scan, then `origin/peer-0` committed anew: a later pass finds the same conflicts,
 * which tell the hook nothing new, and stores peer-0's new tip. Answers that tip.
 */
async function knownConflictsMoved(ws: Workspace): Promise<string> {
  addPeers(ws);
  const first = ws.service();
  await first.setEnabled(ws.repo, true);
  expect((await first.scan(ws.repo)).conflicts).toHaveLength(PEERS);
  return commitPeer(ws, 0, "Orders, take 0, again");
}

/** The tip of `origin/peer-0` the stored conflict with it names. */
const peerTip = (conflicts: MergeAssistStatus["conflicts"]): string | undefined =>
  conflicts.find(conflict => conflict.theirs.ref === "origin/peer-0")?.theirs.sha;

/** Sets a hook command on the repository that logs each event it hears; answers the events logged so far. */
async function loggedHook(ws: Workspace): Promise<() => string[]> {
  const log = join(ws.folder, "hook.log");
  const script = "require(\"node:fs\").appendFileSync(process.argv[1], process.env.RERAIL_CONFLICT_EVENT + \"\\n\")";
  await ws.service().updateSettings(ws.repo, { hook: { command: [process.execPath, "-e", script, log], timeoutSeconds: 10 } });
  return () => (existsSync(log) ? readFileSync(log, "utf8").split("\n").filter(line => line !== "") : []);
}

/** An open pull request from `peer-<index>`, its head at `sha`. */
function pullOf(sha: string, index: number): PullRequestSummary {
  return {
    number: index + 1, title: `Orders, take ${index}`, url: `https://github.com/acme/app/pull/${index + 1}`, headRefName: `peer-${index}`,
    headCommit: sha, baseRefName: "main", author: "jbarnes", isDraft: false, updatedAt: "2026-11-18T00:00:00Z", crossRepository: false, headOwner: "acme",
  };
}

/** A process the code under test started, as its command line. */
const commandLine = (args: readonly unknown[]): string => [args[0], ...(Array.isArray(args[1]) ? args[1] : [])].join(" ");

/** Whether a process started with these arguments is Git. */
const isGit = (args: readonly unknown[]): boolean => /(?:^|[\\/])git(?:\.exe)?$/iu.test(String(args[0]));

/** Whether a process has not exited yet. */
const running = (child: ChildProcess): boolean => child.exitCode === null && child.signalCode === null;

const { spawn: actualSpawn } = await vi.importActual<typeof import("node:child_process")>("node:child_process");

/** setTimeout's longest delay: a wait for the stop that a pass ends long before, by its own work. */
const UNREACHED_MS = 2 ** 31 - 1;

/**
 * Node waiting on a lock nothing releases: in place of a Git read, a process that ends only when it is ended, so the
 * pass waiting for that read still runs when the stop's wait is over, however fast the machine.
 */
const HELD_READ = ["-e", "Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0)"];

/**
 * Where a test stops the service: as the work starts its first Git read whose arguments hold `at`, or when `trigger`
 * calls the stop it is handed, for a stop at something other than a Git read.
 */
interface StopPoint {
  readonly at?: string;
  readonly trigger?: (stop: () => void) => void;
  /** What the stop is given to wait for the passes running. */
  readonly waitMs: number;
  /** Whether {@link HELD_READ} stands in for the read at `at`. */
  readonly held: boolean;
  /** Runs once the stop has resolved and the Git reads still running are counted, before the work is awaited. */
  readonly afterStop?: () => void;
}

/**
 * Runs `work` and stops `service` at `point`. Answers, once the stop and the work are over, the Git reads the work
 * started that still ran when the stop resolved, and those started once the stop ended the held read or resolved.
 */
async function stopDuring(service: MergeAssistRuntime, work: () => Promise<unknown>, point: StopPoint): Promise<{ runningAtStop: string[]; startedAfter: string[] }> {
  const reads: Array<{ line: string; child: ChildProcess }> = [];
  const startedAfter: string[] = [];
  const stopping = Promise.withResolvers<{ stopped: Promise<void> }>();
  let state: "before" | "stopping" | "stopped" = "before";
  const stopped = (): void => {
    state = "stopped";
  };
  /** Starts the stop once the code that called this has set up what it is doing. */
  const beginStop = (): void => {
    state = "stopping";
    queueMicrotask(() => stopping.resolve({ stopped: service.stop(point.waitMs).then(stopped) }));
  };
  /** The read at `point`: {@link HELD_READ} in its place when `held`. */
  const stopAt = (args: Parameters<typeof spawn>): ChildProcess => {
    beginStop();
    const child = point.held ? actualSpawn(process.execPath, HELD_READ, { stdio: ["pipe", "pipe", "pipe"] }) : actualSpawn(...args);
    // Once the held read has ended, which the stop does at once and a read the stop missed only at its time limit, no
    // Git may start.
    if (point.held) child.once("exit", stopped);
    return child;
  };
  point.trigger?.(beginStop);
  vi.mocked(spawn).mockImplementation((...args: Parameters<typeof spawn>) => {
    const line = commandLine(args);
    const child = state === "before" && point.at !== undefined && line.includes(point.at) ? stopAt(args) : actualSpawn(...args);
    if (isGit(args)) {
      reads.push({ line, child });
      if (state === "stopped") startedAfter.push(line);
    }
    return child;
  });
  try {
    const done = work();
    await (await stopping.promise).stopped;
    const left = reads.filter(read => running(read.child));
    // A held read the stop left running would hold the work for ever: it ends here, once counted.
    for (const { child } of left) child.kill("SIGKILL");
    point.afterStop?.();
    await done;
    return { runningAtStop: left.map(({ line }) => line), startedAfter };
  } finally {
    vi.mocked(spawn).mockImplementation(actualSpawn);
  }
}

/**
 * A Merge Assist over a connection of its own to the workspace's database, and the closing of that connection, as a
 * runtime closes its database once its Merge Assist has stopped. Closed after the test at the latest.
 */
function ownConnection(ws: Workspace, github: GithubClient): { service: MergeAssistRuntime; close: () => void } {
  const database = openRerailDatabase(ws.databasePath);
  let open = true;
  const close = (): void => {
    if (open) database.$client.close();
    open = false;
  };
  onTestFinished(close);
  return { service: createMergeAssistService({ database, github }), close };
}

const { access: actualAccess } = await vi.importActual<typeof import("node:fs/promises")>("node:fs/promises");

/**
 * Holds the first `access` call for `folder` as a drive that stopped answering would: it answers only once `release`
 * is called, and `onHeld` runs as it is held.
 */
function holdAccessTo(folder: string, onHeld: () => void): { release: () => void } {
  const held = Promise.withResolvers<void>();
  const same = (path: string): boolean => resolve(path).toLowerCase() === resolve(folder).toLowerCase();
  let holding = false;
  vi.mocked(access).mockImplementation(async (path, mode) => {
    if (holding || !same(String(path))) return await actualAccess(path, mode);
    holding = true;
    onHeld();
    await held.promise;
    return await actualAccess(path, mode);
  });
  onTestFinished(() => {
    held.resolve();
    vi.mocked(access).mockImplementation(actualAccess);
  });
  return { release: () => held.resolve() };
}

/** `gh` whose listing of open pull requests answers only once `release` is called; `onHeld` runs as it is asked. */
class HeldGithub extends FakeGithub {
  readonly #held = Promise.withResolvers<void>();
  onHeld: () => void = () => undefined;

  override async openPullRequests(): Promise<{ pulls: PullRequestSummary[]; truncated: boolean }> {
    this.onHeld();
    await this.#held.promise;
    return await super.openPullRequests();
  }

  release(): void {
    this.#held.resolve();
  }
}

/** The checkout's last scan, scan mark and conflicts as a new service reads them. */
async function stored(ws: Workspace): Promise<Pick<MergeAssistStatus, "lastScan" | "scanning" | "conflicts">> {
  const { lastScan, scanning, conflicts } = await ws.service().status(ws.repo);
  return { lastScan, scanning, conflicts };
}

describe("Merge Assist's stop and a detection pass in flight", { timeout: 60_000 }, () => {
  it.each([
    ["reads the checkout's own branch and base", "refs/heads/master", false],
    ["compares the checkout's work with other branches", "--name-only", false],
    ["checks which open pull requests are work of their own", "--is-ancestor", true],
  ])("ends a scan that %s, still running when the stop's wait is over, at its next Git read: no Git runs once the stop resolves, and what was found stays", async (_, at, pulls) => {
    const ws = await workspace(pulls ? { remote: "https://github.com/acme/app.git" } : {});
    const tips = addPeers(ws);
    const github = new FakeGithub(pulls ? { pullRequests: tips.map(pullOf) } : {});
    const first = ws.service({ github });
    await first.setEnabled(ws.repo, true);
    const found = await first.scan(ws.repo);
    expect(found.conflicts).toHaveLength(PEERS);

    const service = ws.service({ github });
    const { runningAtStop, startedAfter } = await stopDuring(service, () => service.openSession(ws.repo, "claude-code", "session-1"), { at, waitMs: 0, held: true });

    expect(startedAfter).toEqual([]);
    expect(runningAtStop).toEqual([]);
    expect(await stored(ws)).toEqual({ lastScan: found.lastScan, scanning: false, conflicts: found.conflicts });
  });

  it("lets a scan that ends within the stop's wait store what it found when that tells the hook nothing new, as a scan that ends before the stop does", async () => {
    const ws = await workspace();
    const tip = await knownConflictsMoved(ws);
    const service = ws.service();

    const { runningAtStop } = await stopDuring(service, () => service.openSession(ws.repo, "claude-code", "session-1"), { at: "--name-only", waitMs: UNREACHED_MS, held: false });

    expect(runningAtStop).toEqual([]);
    const scan = await stored(ws);
    expect({ peers: scan.lastScan?.peers, scanning: scan.scanning, conflicts: scan.conflicts.length, tip: peerTip(scan.conflicts) })
      .toEqual({ peers: { branches: PEERS, pullRequests: 0 }, scanning: false, conflicts: PEERS, tip });
  });

  it("lets the detection after an edit that ends within the stop's wait store what it found when that tells the hook nothing new", async () => {
    const ws = await workspace();
    const tip = await knownConflictsMoved(ws);
    const service = ws.service();

    const { runningAtStop } = await stopDuring(service, () => service.detectAfterEdit(ws.repo, "session-1", "claude-code"), { at: "--name-only", waitMs: UNREACHED_MS, held: false });

    expect(runningAtStop).toEqual([]);
    const { conflicts } = await stored(ws);
    expect({ conflicts: conflicts.length, tip: peerTip(conflicts) }).toEqual({ conflicts: PEERS, tip });
  });

  it("lets a session's scan whose checkout Git was still locating when the stop came store what it found within the stop's wait", async () => {
    const ws = await workspace();
    const tip = await knownConflictsMoved(ws);
    const service = ws.service();

    const { runningAtStop } = await stopDuring(service, () => service.openSession(ws.repo, "claude-code", "session-1"), { at: "--show-toplevel", waitMs: UNREACHED_MS, held: false });

    expect(runningAtStop).toEqual([]);
    const scan = await stored(ws);
    expect({ peers: scan.lastScan?.peers, scanning: scan.scanning, conflicts: scan.conflicts.length, tip: peerTip(scan.conflicts) })
      .toEqual({ peers: { branches: PEERS, pullRequests: 0 }, scanning: false, conflicts: PEERS, tip });
  });

  it.each([
    ["whose `gh` listing answers within the stop's wait", "gh"],
    ["already comparing the checkout's work with other branches when the stop came", "--name-only"],
  ])("stores nothing a scan %s finds new, which the stopped hook would not hear: a service opened after finds each of those conflicts, and its hook hears it", async (_, at) => {
    const ws = await workspace({ remote: "https://github.com/acme/app.git" });
    const pullRequests = addPeers(ws).map(pullOf);
    await ws.service().setEnabled(ws.repo, true);
    const heard = await loggedHook(ws);
    const github = new HeldGithub({ pullRequests });
    if (at !== "gh") github.release();
    const service = ws.service({ github });

    const { runningAtStop, startedAfter } = await stopDuring(service, () => service.openSession(ws.repo, "claude-code", "session-1"), {
      ...(at === "gh" ? {} : { at }),
      trigger: stop => {
        if (at !== "gh") return;
        // `gh` answers once the stop has begun, and the stop's wait lasts until the scan ends.
        github.onHeld = () => {
          stop();
          queueMicrotask(() => github.release());
        };
      },
      waitMs: UNREACHED_MS,
      held: false,
    });

    expect(startedAfter).toEqual([]);
    expect(runningAtStop).toEqual([]);
    expect(await stored(ws)).toEqual({ lastScan: null, scanning: false, conflicts: [] });
    const next = ws.service({ github: new FakeGithub({ pullRequests }) });
    expect((await next.scan(ws.repo)).conflicts).toHaveLength(PEERS);
    await next.settled();
    expect(heard()).toEqual(Array.from({ length: PEERS }, () => "detected"));
  });

  it("returns once the Git it ended has exited while a scan waits for a file call that never answers, and the scan then writes nothing to the closed database and starts no Git", async () => {
    const ws = await workspace();
    addPeers(ws);
    await ws.service().setEnabled(ws.repo, true);
    const { service, close } = ownConnection(ws, new FakeGithub());
    let release = (): void => undefined;

    const { runningAtStop, startedAfter } = await stopDuring(service, () => service.openSession(ws.repo, "claude-code", "session-1"), {
      trigger: stop => {
        release = holdAccessTo(ws.repo, stop).release;
      },
      waitMs: 0,
      held: false,
      // The call answers only once the stop has resolved and the database has closed.
      afterStop: () => {
        close();
        release();
      },
    });

    expect(startedAfter).toEqual([]);
    expect(runningAtStop).toEqual([]);
    expect(await stored(ws)).toEqual({ lastScan: null, scanning: false, conflicts: [] });
  });

  it("lets a scan that `gh` answers once the stop has resolved and the database has closed write nothing", async () => {
    const ws = await workspace({ remote: "https://github.com/acme/app.git" });
    addPeers(ws);
    await ws.service().setEnabled(ws.repo, true);
    const github = new HeldGithub();
    const { service, close } = ownConnection(ws, github);

    const { runningAtStop, startedAfter } = await stopDuring(service, () => service.openSession(ws.repo, "claude-code", "session-1"), {
      trigger: stop => {
        github.onHeld = stop;
      },
      waitMs: 0,
      held: false,
      afterStop: () => {
        close();
        github.release();
      },
    });

    expect(startedAfter).toEqual([]);
    expect(runningAtStop).toEqual([]);
    expect(await stored(ws)).toEqual({ lastScan: null, scanning: false, conflicts: [] });
  });

  it("lets a scan that `gh` answers right after a stop with no wait, whose caller closed the database without awaiting it, write nothing and start no Git", async () => {
    const ws = await workspace({ remote: "https://github.com/acme/app.git" });
    addPeers(ws);
    await ws.service().setEnabled(ws.repo, true);
    const github = new HeldGithub();
    const { service, close } = ownConnection(ws, github);
    const asked = Promise.withResolvers<void>();
    github.onHeld = () => asked.resolve();
    const scan = service.openSession(ws.repo, "claude-code", "session-1");
    await asked.promise;
    const started: string[] = [];
    vi.mocked(spawn).mockImplementation((...args: Parameters<typeof spawn>) => {
      if (isGit(args)) started.push(commandLine(args));
      return actualSpawn(...args);
    });
    try {
      // As `DatabaseApiStores.close()` does: the stop, then the database closed in the same call.
      const stopped = service.stop(0);
      close();
      github.release();
      await scan;
      await stopped;
    } finally {
      vi.mocked(spawn).mockImplementation(actualSpawn);
    }

    expect(started).toEqual([]);
    expect(await stored(ws)).toEqual({ lastScan: null, scanning: false, conflicts: [] });
  });

  it("ends a session's locating of its checkout still running when the stop's wait is over: no Git runs once the stop resolves, and nothing is written", async () => {
    const ws = await workspace();
    addPeers(ws);
    await ws.service().setEnabled(ws.repo, true);
    const { service, close } = ownConnection(ws, new FakeGithub());

    const { runningAtStop, startedAfter } = await stopDuring(service, () => service.openSession(ws.repo, "claude-code", "session-1"), {
      at: "--show-toplevel", waitMs: 0, held: true, afterStop: close,
    });

    expect(startedAfter).toEqual([]);
    expect(runningAtStop).toEqual([]);
    expect(await stored(ws)).toEqual({ lastScan: null, scanning: false, conflicts: [] });
  });
});
