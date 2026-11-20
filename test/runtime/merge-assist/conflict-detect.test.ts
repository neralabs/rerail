import { execFileSync } from "node:child_process";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { lookupCheckout } from "../../../src/engine/index.js";
import {
  detectConflicts,
  NO_BASE_PROBLEM,
  REMOTE_LIMIT_PROBLEM,
  UNCOMPARED_KEPT_PROBLEM,
  UNLISTED_BRANCHES_PROBLEM,
  type DetectContext,
} from "../../../src/runtime/merge-assist/conflict-detect.js";
import { readConflicts, writeConflicts } from "../../../src/runtime/merge-assist/conflict-store.js";
import type { PullRequestSummary } from "../../../src/runtime/merge-assist/github.js";
import { commit, git, writeFiles } from "../../support/merge-assist/git.js";
import { removeFolder } from "../../support/platform/remove-folder.js";
import { mergeAssistWorkspaces, type Workspace } from "../../support/merge-assist/workspace.js";
import { withExperiments } from "../../support/environment/experiments.js";

/** `gitReadAsync`'s shape, as the tests stand in for it. */
type GitRead = (root: string, args: readonly string[], options?: never) => Promise<string | undefined>;

// Every Git read of a pass goes through `gitReadAsync`; a test can make one fail, or wait.
const reads = vi.hoisted(() => ({ real: undefined as unknown as GitRead, use: undefined as GitRead | undefined }));
vi.mock("../../../src/engine/merge-assist/branch-git.js", async () => {
  const actual = await vi.importActual<typeof import("../../../src/engine/merge-assist/branch-git.js")>("../../../src/engine/merge-assist/branch-git.js");
  reads.real = actual.gitReadAsync;
  return { ...actual, gitReadAsync: (root: string, args: readonly string[], options?: object) => (reads.use ?? reads.real)(root, args, options as never) };
});

// A pull request listing, and so the remote-tracking branches a pass compares, holds one: a test needs two branches, not 201.
vi.mock("../../../src/runtime/merge-assist/github.js", async () => ({
  ...await vi.importActual<typeof import("../../../src/runtime/merge-assist/github.js")>("../../../src/runtime/merge-assist/github.js"),
  PULL_REQUEST_LIMIT: 1,
}));

// A test can stop a pass's reads right after its store has committed.
vi.mock("../../../src/runtime/merge-assist/conflict-store.js", { spy: true });
const { GitReads } = await vi.importActual<typeof import("../../../src/engine/git.js")>("../../../src/engine/git.js");
const { writeConflicts: actualWriteConflicts } = await vi.importActual<typeof import("../../../src/runtime/merge-assist/conflict-store.js")>(
  "../../../src/runtime/merge-assist/conflict-store.js",
);

// Merge Assist is experimental: every test here runs with it on, as a daemon started with the variable does.
withExperiments("MERGE_ASSIST");
beforeEach(() => {
  reads.use = undefined;
});

const workspace = mergeAssistWorkspaces();
const THEIR_ORDERS = { "src/api/orders.ts": "export const orders = () => client.get(\"/v2/orders\");\n" };

/** A workspace with Merge Assist on, and a pass's context over it: pull requests as `pulls` says. */
async function enabled(): Promise<{ ws: Workspace; context: (pulls: DetectContext["pulls"], login?: string) => DetectContext }> {
  const ws = await workspace();
  await ws.service().setEnabled(ws.repo, true);
  const checkout = lookupCheckout(ws.database, ws.repo)!;
  return { ws, context: (pulls, login) => ({ database: ws.database, checkout, pulls, login: login ?? null, now: Date.now }) };
}

const DAY = 24 * 60 * 60 * 1000;

/** Amend the checkout's tip, unchanged but for its committer date: `days` ago. */
function committedDaysAgo(checkout: string, days: number): void {
  execFileSync("git", ["commit", "-q", "--amend", "--no-edit"], {
    cwd: checkout,
    env: {
      ...process.env, GIT_COMMITTER_NAME: "Alex Kim", GIT_COMMITTER_EMAIL: "alex@example.com", GIT_COMMITTER_DATE: new Date(Date.now() - days * DAY).toISOString(),
      GIT_CONFIG_NOSYSTEM: "1", GIT_CONFIG_GLOBAL: "/dev/null",
    },
  });
}

/** An open pull request from a fork owned by `owner`, from its branch `mine`, at `head`. */
const forkPull = (number: number, owner: string, head: string): PullRequestSummary => ({
  number, title: "Orders", url: `https://github.com/acme/app/pull/${number}`, headRefName: "mine", headCommit: head, baseRefName: "main", author: owner,
  isDraft: false, updatedAt: "2026-11-03T14:00:00Z", crossRepository: true, headOwner: owner,
});

const open = (ws: Workspace): string[] =>
  readConflicts(ws.database, ws.repo).filter(entry => entry.conflict.status === "open").map(entry => entry.conflict.theirs.ref).sort();

// Each test builds Git repositories; under a full run that outlasts the default 5 s.
describe("a detection pass", { timeout: 30_000 }, () => {
  it("leaves alone an open conflict another pass stored while it ran, which it never examined", async () => {
    const { ws, context } = await enabled();
    // A branch only `origin` has: compared only while open pull requests are unread (`pulls: null`).
    const theirs = ws.worktree("remote-only");
    commit(theirs, THEIR_ORDERS, "Version the orders path");
    ws.push(theirs, "remote-only");
    git(ws.repo, "worktree", "remove", "--force", theirs);
    git(ws.repo, "branch", "-D", "remote-only");

    // Pass A (pull requests read, so it lists no remote-tracking branch) begins and waits at its branch listing; pass
    // B (unread) finds the conflict and stores it; then A goes on and writes.
    const gate = Promise.withResolvers<void>();
    reads.use = async (root, args, options) => {
      if (args.some(arg => arg.startsWith("--no-merged=")) && !args.includes("refs/remotes")) await gate.promise;
      return await reads.real(root, args, options);
    };
    const first = detectConflicts(context([]));
    const second = await detectConflicts(context(null));
    expect(second?.after.map(entry => entry.conflict.theirs.ref)).toEqual(["origin/remote-only"]);
    gate.resolve();
    const after = await first;

    expect(after?.after.map(entry => entry.conflict.theirs.ref)).toEqual(["origin/remote-only"]);
    expect(open(ws)).toEqual(["origin/remote-only"]);
  });

  it("answers what it stored when the owner of its Git reads stops them once its store has committed", async () => {
    const { ws, context } = await enabled();
    commit(ws.worktree("jdoe-orders"), THEIR_ORDERS, "Version the orders path");
    const owner = new GitReads();
    vi.mocked(writeConflicts).mockImplementationOnce((...args) => {
      actualWriteConflicts(...args);
      void owner.stop();
    });

    const detected = await detectConflicts({ ...context([]), reads: owner });

    expect(owner.stopped).toBe(true);
    expect(detected?.after.map(entry => entry.conflict.theirs.ref)).toEqual(["jdoe-orders"]);
    expect(open(ws)).toEqual(["jdoe-orders"]);
  });

  it("keeps the open branch conflicts, and says why, when Git cannot list the branches", async () => {
    const { ws, context } = await enabled();
    commit(ws.worktree("jdoe-orders"), THEIR_ORDERS, "Version the orders path");
    expect((await detectConflicts(context([])))?.problems).toEqual([]);
    expect(open(ws)).toEqual(["jdoe-orders"]);

    reads.use = async (root, args, options) =>
      (args[0] === "for-each-ref" && args.some(arg => arg.startsWith("--no-merged=")) ? undefined : await reads.real(root, args, options));
    const detected = await detectConflicts(context([]));

    expect(detected?.problems).toEqual([UNLISTED_BRANCHES_PROBLEM]);
    expect(detected?.after.map(entry => entry.conflict.theirs.ref)).toEqual(["jdoe-orders"]);
    expect(open(ws)).toEqual(["jdoe-orders"]);
  });

  it("compares the other refs when one cannot be read, keeping that one's open conflicts and saying why", async () => {
    const { ws, context } = await enabled();
    commit(ws.worktree("jdoe-orders"), THEIR_ORDERS, "Version the orders path");
    const unreadable = commit(ws.worktree("kchen-orders"), { "src/api/orders.ts": "export const orders = () => client.get(\"/v3/orders\");\n" }, "Move orders");
    await detectConflicts(context([]));
    expect(open(ws)).toEqual(["jdoe-orders", "kchen-orders"]);

    // Git fails to diff that ref (as at the 512 MiB or 60 s bound).
    reads.use = async (root, args, options) => (args.includes(unreadable) && args.includes("diff") ? undefined : await reads.real(root, args, options));
    const detected = await detectConflicts(context([]));

    expect(detected?.problems).toEqual([expect.stringMatching(/^Could not compare kchen-orders: git diff \S+ \S+ failed in /u)]);
    expect(open(ws)).toEqual(["jdoe-orders", "kchen-orders"]);
  });

  it("compares only the most recently committed remote-tracking branches while pull requests are unread, and says so", async () => {
    const { ws, context } = await enabled();
    for (const [branch, days] of [["older-orders", 2], ["newer-orders", 0]] as const) {
      const theirs = ws.worktree(branch);
      commit(theirs, THEIR_ORDERS, "Version the orders path");
      committedDaysAgo(theirs, days);
      ws.push(theirs, branch);
      git(ws.repo, "worktree", "remove", "--force", theirs);
    }
    const detected = await detectConflicts(context(null));
    expect(detected?.problems).toEqual([REMOTE_LIMIT_PROBLEM]);
    expect(open(ws)).toEqual(["origin/newer-orders"]);
  });

  it("compares a branch a worktree is rebasing, and none whose worktree folder is gone", async () => {
    const { ws, context } = await enabled();
    const rebasing = ws.worktree("rebasing-orders");
    commit(rebasing, THEIR_ORDERS, "Version the orders path");
    // A rebase stopped partway: the worktree's HEAD is detached, its branch named only in `rebase-merge/head-name`.
    expect(() => git(rebasing, "rebase", "--exec", "exit 1", "main")).toThrow();
    const gone = ws.worktree("gone-orders");
    commit(gone, { "src/api/orders.ts": "export const orders = () => client.get(\"/v3/orders\");\n" }, "Move orders");
    // Removed without Git: the worktree is prunable, its branch no longer checked out anywhere.
    await removeFolder(gone);

    await detectConflicts(context([]));
    expect(open(ws)).toEqual(["rebasing-orders"]);
  });

  it("leaves out this checkout's own pull request from a fork after an amend, and compares another's of the same branch name", async () => {
    const { ws, context } = await enabled();
    // Pushed to the person's fork and opened as a pull request, then amended here.
    const pushed = git(ws.repo, "rev-parse", "HEAD");
    writeFiles(ws.repo, { "src/api/orders.ts": "export const orders = () => client.get(\"/orders?page=1\");\n" });
    git(ws.repo, "commit", "-q", "--amend", "-am", "orders, second take");
    // Another person's fork, with a branch of the same name that was never this branch.
    const theirs = commit(ws.worktree("jdoe-orders"), THEIR_ORDERS, "Version the orders path");

    await detectConflicts(context([forkPull(5, "akim", pushed), forkPull(6, "jdoe", theirs)]));
    expect(open(ws)).toEqual(["refs/pull/6/head"]);
  });

  it("leaves out a pull request from the signed-in person's own fork of this branch, whatever this branch's reflog holds", async () => {
    const { ws, context } = await enabled();
    // A head this branch never pointed at (pushed from another clone, or its reflog expired), in the person's fork.
    const elsewhere = commit(ws.worktree("other-clone"), THEIR_ORDERS, "Version the orders path");

    await detectConflicts(context([forkPull(5, "AKim", elsewhere)]));
    expect(open(ws)).toEqual(["refs/pull/5/head"]);
    await detectConflicts(context([forkPull(5, "AKim", elsewhere)], "akim"));
    expect(open(ws)).toEqual(["other-clone"]);
  });

  it("keeps the open conflicts, and says so, when there is no base branch to compare with", async () => {
    const { ws, context } = await enabled();
    commit(ws.worktree("jdoe-orders"), THEIR_ORDERS, "Version the orders path");
    await detectConflicts(context([]));
    expect(open(ws)).toEqual(["jdoe-orders"]);

    git(ws.repo, "branch", "-m", "main", "trunk");
    const detected = await detectConflicts(context([]));
    expect(detected?.problems).toEqual([NO_BASE_PROBLEM, UNCOMPARED_KEPT_PROBLEM]);
    expect(detected?.after.map(entry => entry.conflict.theirs.ref)).toEqual(["jdoe-orders"]);
    expect(open(ws)).toEqual(["jdoe-orders"]);
  });
});
