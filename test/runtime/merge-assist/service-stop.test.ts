import { type ChildProcess, execFileSync, spawn } from "node:child_process";
import { describe, expect, it, vi } from "vitest";
import type { GithubClient, PullRequestSummary } from "../../../src/runtime/index.js";
import { withExperiments } from "../../support/environment/experiments.js";
import { git } from "../../support/merge-assist/git.js";
import { FakeGithub, mergeAssistWorkspaces, type Workspace } from "../../support/merge-assist/workspace.js";

// Every process Rerail starts goes through this module: the test stops the service as a pass starts a given Git read,
// and sees every Git process started after. On Windows the stop ends a running Git with taskkill, which is no Git read.
vi.mock("node:child_process", { spy: true });

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
  const now = Math.floor(Date.now() / 1000);
  const data = (text: string): string => `data ${Buffer.byteLength(text)}\n${text}\n`;
  const commits = Array.from({ length: PEERS }, (_, index) => [
    `commit refs/remotes/origin/peer-${index}`,
    `committer Jo Barnes <jo@example.com> ${now} +0000`,
    `${data(`Orders, take ${index}`)}from refs/heads/main^0`,
    "M 100644 inline src/api/orders.ts",
    data(`export const orders = () => client.get("/v${index + 2}/orders");\n`),
  ].join("\n"));
  execFileSync("git", ["fast-import", "--quiet"], { cwd: ws.repo, input: commits.join("\n"), env: { ...process.env, GIT_CONFIG_NOSYSTEM: "1" } });
  return Array.from({ length: PEERS }, (_, index) => git(ws.repo, "rev-parse", `refs/remotes/origin/peer-${index}`));
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

const { spawn: actualSpawn } = await vi.importActual<typeof import("node:child_process")>("node:child_process");

/**
 * Starts a session's scan of the workspace's checkout and stops the service as the scan starts its first Git read
 * whose arguments hold `at`. Answers, once the stop and the scan are over, the Git processes the scan started that still
 * ran when the stop resolved, and those started from the stop on.
 */
async function stopDuringScan(ws: Workspace, github: GithubClient, at: string): Promise<{ runningAtStop: string[]; startedAfter: string[] }> {
  const service = ws.service({ github });
  const gits: Array<{ line: string; child: ChildProcess }> = [];
  const startedAfter: string[] = [];
  const stopping = Promise.withResolvers<Promise<void>>();
  let stopped = false;
  vi.mocked(spawn).mockImplementation((...args: Parameters<typeof spawn>) => {
    const line = commandLine(args);
    const child = actualSpawn(...args);
    if (isGit(args)) {
      if (stopped) startedAfter.push(line);
      gits.push({ line, child });
    }
    if (!stopped && line.includes(at)) {
      stopped = true;
      stopping.resolve(service.stop());
    }
    return child;
  });
  try {
    const scan = service.openSession(ws.repo, "claude-code", "session-1");
    await (await stopping.promise);
    const runningAtStop = gits.filter(({ child }) => child.exitCode === null && child.signalCode === null).map(({ line }) => line);
    await scan;
    return { runningAtStop, startedAfter };
  } finally {
    vi.mocked(spawn).mockImplementation(actualSpawn);
  }
}

describe("Merge Assist's stop ends a detection pass in flight", { timeout: 60_000 }, () => {
  it.each([
    ["reads the checkout's own branch and base", "refs/heads/master", false],
    ["compares the checkout's work with other branches", "--name-only", false],
    ["checks which open pull requests are work of their own", "--is-ancestor", true],
  ])("ends a scan that %s at its next Git read: no Git starts once stopped or runs when the stop resolves, and what was found stays", async (_, at, pulls) => {
    const ws = await workspace(pulls ? { remote: "https://github.com/acme/app.git" } : {});
    const tips = addPeers(ws);
    const github = new FakeGithub(pulls ? { pullRequests: tips.map(pullOf) } : {});
    const first = ws.service({ github });
    await first.setEnabled(ws.repo, true);
    const found = await first.scan(ws.repo);
    expect(found.conflicts).toHaveLength(PEERS);

    const { runningAtStop, startedAfter } = await stopDuringScan(ws, github, at);

    expect(startedAfter).toEqual([]);
    expect(runningAtStop).toEqual([]);
    const status = await ws.service({ github }).status(ws.repo);
    expect({ lastScan: status.lastScan, scanning: status.scanning, conflicts: status.conflicts })
      .toEqual({ lastScan: found.lastScan, scanning: false, conflicts: found.conflicts });
  });
});
