/**
 * Real Git repositories, a Rerail database, and a stand-in for `gh`, for Merge Assist's service-level tests. Nothing
 * here reaches the network.
 */
import { existsSync, mkdirSync } from "node:fs";
import { mkdtemp, realpath, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach } from "vitest";
import {
  openRerailDatabase,
  recordCheckout,
  setUpRepository,
  type GithubReadiness,
  type RerailDatabase,
} from "../../../src/engine/index.js";
import {
  createMergeAssistService,
  GithubError,
  setRepositoryStarted,
  type GithubClient,
  type GithubRepository,
  type MergeAssistRuntime,
  type PullRequestSummary,
} from "../../../src/runtime/index.js";
import { commit, git } from "../git.js";

/** The notes the session has not been handed, recorded as handed, as a delivery that takes them all would. */
export function takeNotes(service: MergeAssistRuntime, sessionId: string, checkoutPath: string): string[] {
  const notes = service.pendingNotes(sessionId, checkoutPath);
  service.markNotesDelivered(sessionId, checkoutPath, notes.flatMap(note => note.marks));
  return notes.map(note => note.content);
}

/** `src/api/orders.ts` as branch `mine` commits it. */
const ORDERS_CLEAN = "export const orders = () => client.get(\"/orders\");\n";

export interface FakeGithubOptions {
  readiness?: GithubReadiness;
  /** `gh`'s sign-in could not be read (a timeout, a crash): readiness is `unavailable`. */
  readinessError?: boolean;
  pullRequests?: PullRequestSummary[];
  /** Whether the listing stopped at its limit. */
  truncated?: boolean;
  /** `gh pr list` fails: `openPullRequests` throws. */
  pullsError?: boolean;
  /** Pull requests closed without merging. */
  closedPullRequests?: PullRequestSummary[];
  /** `gh pr list --state closed` fails: `closedPullRequests` throws. */
  closedError?: boolean;
}

/** `gh` as a scan sees it, from fixed answers a test may change between scans; it counts readiness reads. */
export class FakeGithub implements GithubClient {
  readinessReads = 0;

  constructor(readonly options: FakeGithubOptions = {}) {}

  async readiness(repository: GithubRepository | null): Promise<GithubReadiness> {
    this.readinessReads += 1;
    if (repository === null) return { state: "not-github", login: null, repository: null };
    if (this.options.readinessError === true) return { state: "unavailable", login: null, repository: `${repository.owner}/${repository.name}` };
    return this.options.readiness ?? { state: "signed-in", login: "tbelanger", repository: `${repository.owner}/${repository.name}` };
  }

  async openPullRequests(): Promise<{ pulls: PullRequestSummary[]; truncated: boolean }> {
    if (this.options.pullsError === true) throw new GithubError("failed", "HTTP 502");
    return { pulls: this.options.pullRequests ?? [], truncated: this.options.truncated ?? false };
  }

  async closedPullRequests(): Promise<{ pulls: PullRequestSummary[]; truncated: boolean }> {
    if (this.options.closedError === true) throw new GithubError("failed", "HTTP 502");
    return { pulls: this.options.closedPullRequests ?? [], truncated: false };
  }
}

export interface Workspace {
  /** The temporary folder holding everything. */
  readonly folder: string;
  /** The main checkout, on branch `mine`, branched from `main`. */
  readonly repo: string;
  readonly database: RerailDatabase;
  readonly databasePath: string;
  readonly repositoryKey: string;
  /** A Merge Assist over the workspace's database, with this `gh` stand-in. */
  service(options?: { github?: GithubClient; now?: () => number; noteWaitMs?: number }): MergeAssistRuntime;
  /** A linked worktree of the repository at `<folder>/<name>`, on a new branch from `main`. */
  worktree(name: string, options?: { record?: boolean }): string;
  /**
   * Push `branch` from `checkout` (force: a rewritten branch replaces the pushed one) to the workspace's bare remote at
   * `<folder>/origin.git`, then fetch every branch of it into the main checkout's `refs/remotes/origin/*`, pruning
   * the deleted ones; with `upstream`, the main checkout's local `branch` tracks `origin/<branch>`.
   */
  push(checkout: string, branch: string, options?: { upstream?: boolean }): void;
  /** Delete `branch` from the bare remote and prune it from the main checkout's remote-tracking branches. */
  deleteRemoteBranch(branch: string): void;
}

/**
 * A fresh workspace per call, removed after the test: a repository with a commit on `main`, the main checkout on a
 * branch `mine` with a commit of its own (`src/api/orders.ts`), recorded, set up and started (unless `setUp` is false:
 * then only started), with Merge Assist off.
 */
export function mergeAssistWorkspaces(): (options?: { setUp?: boolean; remote?: string; files?: Record<string, string> }) => Promise<Workspace> {
  const cleanups: Array<() => Promise<void>> = [];
  afterEach(async () => {
    for (const cleanup of cleanups.splice(0).reverse()) await cleanup();
  });
  return async ({ setUp = true, remote, files = {} } = {}) => {
    const folder = await realpath(await mkdtemp(join(tmpdir(), "rerail-merge-assist-")));
    const repo = join(folder, "repo");
    mkdirSync(repo);
    git(repo, "init", "-q", "-b", "main");
    git(repo, "config", "user.name", "Tim Belanger");
    if (remote !== undefined) git(repo, "remote", "add", "origin", remote);
    commit(repo, { "src/api/client.ts": "export const client = { get: (path: string) => fetch(path) };\n", ...files }, "start");
    git(repo, "switch", "-q", "-c", "mine");
    commit(repo, { "src/api/orders.ts": ORDERS_CLEAN }, "orders");
    const databasePath = join(folder, "rerail.sqlite");
    const database = openRerailDatabase(databasePath);
    const { repositoryKey } = recordCheckout(database, repo);
    if (setUp) setUpRepository(database, repositoryKey, { packs: [] });
    setRepositoryStarted(database, repositoryKey, true);
    const services: MergeAssistRuntime[] = [];
    const bare = join(folder, "origin.git");
    /** The bare remote, made on first use. `origin`'s URL stays as the test set it: pushes and fetches name the folder. */
    const bareRemote = (): string => {
      if (!existsSync(bare)) git(folder, "init", "-q", "--bare", "-b", "main", bare);
      return bare;
    };
    const fetchRemote = (): void => {
      git(repo, "fetch", "-q", "--prune", bareRemote(), "+refs/heads/*:refs/remotes/origin/*");
    };
    cleanups.push(async () => {
      // The conflict hook's handlers run in the background: let them finish before the database closes.
      await Promise.all(services.map(service => service.settled()));
      database.$client.close();
      await rm(folder, { recursive: true, force: true });
    });
    return {
      folder,
      repo,
      database,
      databasePath,
      repositoryKey,
      service: (options = {}) => {
        const service = createMergeAssistService({
          database,
          github: options.github ?? new FakeGithub(),
          ...(options.now === undefined ? {} : { now: options.now }),
          ...(options.noteWaitMs === undefined ? {} : { noteWaitMs: options.noteWaitMs }),
        });
        services.push(service);
        return service;
      },
      worktree: (name, { record = false } = {}) => {
        const path = join(folder, name);
        git(repo, "worktree", "add", "-q", "-b", name, path, "main");
        if (record) recordCheckout(database, path);
        return path;
      },
      push: (checkout, branch, { upstream = false } = {}) => {
        git(checkout, "push", "-q", "--force", bareRemote(), `refs/heads/${branch}:refs/heads/${branch}`);
        fetchRemote();
        if (upstream) {
          git(repo, "config", `branch.${branch}.remote`, "origin");
          git(repo, "config", `branch.${branch}.merge`, `refs/heads/${branch}`);
        }
      },
      deleteRemoteBranch: branch => {
        git(bareRemote(), "branch", "-q", "-D", branch);
        fetchRemote();
      },
    };
  };
}
