/**
 * Real Git repositories, a Rerail database, and stand-ins for the agent CLI judge and `gh`, for Merge Assist's
 * service-level tests. Nothing here reaches the network or starts an agent CLI.
 */
import { execFileSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { mkdtemp, realpath, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { afterEach } from "vitest";
import {
  openRerailDatabase,
  recordCheckout,
  setUpRepository,
  type GithubReadiness,
  type RerailDatabase,
} from "../../../../src/engine/index.js";
import { setRepositoryStarted } from "../../../../src/runtime/index.js";
import type { AgentJudge, JudgeOutcome, JudgeTask } from "../../../../src/runtime/merge-assist/agent-judge.js";
import type { GithubClient, GithubRepository, PullRequestSummary } from "../../../../src/runtime/merge-assist/github.js";
import { createMergeAssistService, type MergeAssistRuntime } from "../../../../src/runtime/merge-assist/service.js";

/** The notes the session has not been handed, recorded as handed, as a delivery that takes them all would. */
export function takeNotes(service: MergeAssistRuntime, sessionId: string, checkoutPath: string): string[] {
  const notes = service.pendingNotes(sessionId, checkoutPath);
  service.markNotesDelivered(sessionId, checkoutPath, notes.flatMap(note => note.marks));
  return notes.map(note => note.content);
}

/** `src/api/orders.ts` as branch `mine` commits it. */
export const ORDERS_CLEAN = "export const orders = () => client.get(\"/orders\");\n";

const GIT_ENV: NodeJS.ProcessEnv = {
  ...process.env,
  GIT_AUTHOR_NAME: "Tim Belanger",
  GIT_AUTHOR_EMAIL: "tim@example.com",
  GIT_COMMITTER_NAME: "Tim Belanger",
  GIT_COMMITTER_EMAIL: "tim@example.com",
  GIT_CONFIG_NOSYSTEM: "1",
};

export function git(cwd: string, ...args: string[]): string {
  return execFileSync("git", args, { cwd, env: GIT_ENV, encoding: "utf8" }).trim();
}

export function writeFiles(root: string, files: Readonly<Record<string, string>>): void {
  for (const [path, content] of Object.entries(files)) {
    mkdirSync(dirname(join(root, path)), { recursive: true });
    writeFileSync(join(root, path), content);
  }
}

export function commit(root: string, files: Readonly<Record<string, string>>, message = "change"): string {
  writeFiles(root, files);
  git(root, "add", "-A");
  git(root, "commit", "-q", "-m", message);
  return git(root, "rev-parse", "HEAD");
}

/** A judge that answers from `answer` (a value the task parses, or a failure) and records every task. */
export class FakeJudge implements AgentJudge {
  readonly tasks: Array<JudgeTask<unknown>> = [];

  constructor(private readonly answer: (task: JudgeTask<unknown>) => unknown) {}

  readiness() {
    return { available: true, runner: "claude" as const, problem: null };
  }

  budget() {
    return { used: this.tasks.length, limit: 40, windowMinutes: 60 };
  }

  async judge<T>(task: JudgeTask<T>): Promise<JudgeOutcome<T>> {
    this.tasks.push(task as JudgeTask<unknown>);
    const answer = this.answer(task as JudgeTask<unknown>);
    if (typeof answer === "object" && answer !== null && "ok" in answer && answer.ok === false) return answer as JudgeOutcome<T>;
    return { ok: true, value: task.parse(answer), runner: "claude", cached: false };
  }

  count(kind: JudgeTask<unknown>["kind"]): number {
    return this.tasks.filter(task => task.kind === kind).length;
  }
}

/** The quick call's answer for a conflict's status line. */
export const SUMMARY = "They change the same lines as this branch.";

/** What the default judge answers: the quick call's summary. */
export function defaultAnswer(): unknown {
  return { summary: SUMMARY };
}

export interface FakeGithubOptions {
  readiness?: GithubReadiness;
  pullRequests?: PullRequestSummary[];
}

/** `gh` as a scan sees it, from fixed answers; it counts readiness reads. */
export class FakeGithub implements GithubClient {
  readinessReads = 0;

  constructor(private readonly options: FakeGithubOptions = {}) {}

  async readiness(repository: GithubRepository | null): Promise<GithubReadiness> {
    this.readinessReads += 1;
    if (repository === null) return { state: "not-github", login: null, repository: null };
    return this.options.readiness ?? { state: "signed-in", login: "tbelanger", repository: `${repository.owner}/${repository.name}` };
  }

  async openPullRequests(): Promise<PullRequestSummary[]> {
    return this.options.pullRequests ?? [];
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
  /** A Merge Assist over the workspace's database, with these stand-ins. */
  service(options?: { judge?: AgentJudge; github?: GithubClient; now?: () => number }): MergeAssistRuntime;
  /** A linked worktree of the repository at `<folder>/<name>`, on a new branch from `main`. */
  worktree(name: string, options?: { record?: boolean }): string;
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
          judge: options.judge ?? new FakeJudge(defaultAnswer),
          github: options.github ?? new FakeGithub(),
          ...(options.now === undefined ? {} : { now: options.now }),
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
    };
  };
}
