import { beforeEach, describe, expect, it, vi } from "vitest";
import { lookupCheckout } from "../../../src/engine/index.js";
import { detectConflicts, UNLISTED_BRANCHES_PROBLEM, type DetectContext } from "../../../src/runtime/merge-assist/conflict-detect.js";
import { readConflicts } from "../../../src/runtime/merge-assist/conflict-store.js";
import { commit, git } from "../../support/merge-assist/git.js";
import { mergeAssistWorkspaces, type Workspace } from "../../support/merge-assist/workspace.js";

/** `gitReadAsync`'s shape, as the tests stand in for it. */
type GitRead = (root: string, args: readonly string[], options?: never) => Promise<string | undefined>;

// Every Git read of a pass goes through `gitReadAsync`; a test can make one fail, or wait.
const reads = vi.hoisted(() => ({ real: undefined as unknown as GitRead, use: undefined as GitRead | undefined }));
vi.mock("../../../src/engine/merge-assist/branch-git.js", async () => {
  const actual = await vi.importActual<typeof import("../../../src/engine/merge-assist/branch-git.js")>("../../../src/engine/merge-assist/branch-git.js");
  reads.real = actual.gitReadAsync;
  return { ...actual, gitReadAsync: (root: string, args: readonly string[], options?: object) => (reads.use ?? reads.real)(root, args, options as never) };
});

// Merge Assist is experimental: every test here runs with it on, as a daemon started with the variable does.
beforeEach(() => {
  vi.stubEnv("RERAIL_EXPERIMENTAL_MERGE_ASSIST", "1");
  reads.use = undefined;
});

const workspace = mergeAssistWorkspaces();
const THEIR_ORDERS = { "src/api/orders.ts": "export const orders = () => client.get(\"/v2/orders\");\n" };

/** A workspace with Merge Assist on, and a pass's context over it: pull requests as `pulls` says. */
async function enabled(): Promise<{ ws: Workspace; context: (pulls: DetectContext["pulls"]) => DetectContext }> {
  const ws = await workspace();
  await ws.service().setEnabled(ws.repo, true);
  const checkout = lookupCheckout(ws.database, ws.repo)!;
  return { ws, context: pulls => ({ database: ws.database, checkout, pulls, now: Date.now }) };
}

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

  it("keeps the open branch conflicts, and says why, when Git cannot list the branches", async () => {
    const { ws, context } = await enabled();
    commit(ws.worktree("jbarnes-orders"), THEIR_ORDERS, "Version the orders path");
    expect((await detectConflicts(context([])))?.problems).toEqual([]);
    expect(open(ws)).toEqual(["jbarnes-orders"]);

    reads.use = async (root, args, options) =>
      (args[0] === "for-each-ref" && args.some(arg => arg.startsWith("--no-merged=")) ? undefined : await reads.real(root, args, options));
    const detected = await detectConflicts(context([]));

    expect(detected?.problems).toEqual([UNLISTED_BRANCHES_PROBLEM]);
    expect(detected?.after.map(entry => entry.conflict.theirs.ref)).toEqual(["jbarnes-orders"]);
    expect(open(ws)).toEqual(["jbarnes-orders"]);
  });

  it("compares the other refs when one cannot be read, keeping that one's open conflicts and saying why", async () => {
    const { ws, context } = await enabled();
    commit(ws.worktree("jbarnes-orders"), THEIR_ORDERS, "Version the orders path");
    const unreadable = commit(ws.worktree("kchen-orders"), { "src/api/orders.ts": "export const orders = () => client.get(\"/v3/orders\");\n" }, "Move orders");
    await detectConflicts(context([]));
    expect(open(ws)).toEqual(["jbarnes-orders", "kchen-orders"]);

    // Git fails to diff that ref (as at the 512 MiB or 60 s bound).
    reads.use = async (root, args, options) => (args.includes(unreadable) && args.includes("diff") ? undefined : await reads.real(root, args, options));
    const detected = await detectConflicts(context([]));

    expect(detected?.problems).toEqual([expect.stringMatching(/^Could not compare kchen-orders: git diff \S+ \S+ failed in /u)]);
    expect(open(ws)).toEqual(["jbarnes-orders", "kchen-orders"]);
  });
});
