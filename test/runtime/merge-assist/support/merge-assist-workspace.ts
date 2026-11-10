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
  checkouts,
  listRepositoryRules,
  type CandidateSet,
  loadRuleCatalog,
  lookupCheckout,
  openRerailDatabase,
  recordCheckout,
  serializeCandidate,
  setUpRepository,
  type Candidate,
  type GithubReadiness,
  type RerailDatabase,
  type RuleCatalog,
} from "../../../../src/engine/index.js";
import { setRepositoryStarted } from "../../../../src/runtime/index.js";
import type { AgentJudge, JudgeOutcome, JudgeTask } from "../../../../src/runtime/merge-assist/agent-judge.js";
import type { AuthorPermission, GithubClient, GithubRepository, PullRequestSummary } from "../../../../src/runtime/merge-assist/github.js";
import type { CaptureVerdict, CompareVerdict } from "../../../../src/runtime/merge-assist/judge-prompts.js";
import { createMergeAssistService, type MergeAssistRuntime } from "../../../../src/runtime/merge-assist/service.js";

/** The notes the session has not been handed, recorded as handed, as a delivery that takes them all would. */
export function takeNotes(service: MergeAssistRuntime, sessionId: string, checkoutPath: string): string[] {
  const notes = service.pendingNotes(sessionId, checkoutPath);
  service.markNotesDelivered(sessionId, checkoutPath, notes.flatMap(note => note.marks));
  return notes.map(note => note.content);
}

/** `src/api/orders.ts` as branch `mine` commits it: one call, no retry loop. */
export const ORDERS_CLEAN = "export const orders = () => client.get(\"/orders\");\n";
/** The same file with the retry loop a teammate's decision forbids. */
export const ORDERS_RETRY = "export const orders = () => retry(() => client.get(\"/orders\"));\n";

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

/** A candidate file's path and text, as `writeFiles` and `commit` take them. */
export function candidateFiles(...candidates: Candidate[]): Record<string, string> {
  return Object.fromEntries(candidates.map(candidate => [`.rerail/candidates/${candidate.id.slice("candidate:".length)}.json`, serializeCandidate(candidate)]));
}

/** A code rule forbidding `pattern` in `scope` (the candidate's scope), as a candidate carries it. */
export function forbidRule(id: string, pattern: string, scope: readonly string[] = ["src/api/**"], message = "Retries live in the transport layer."): NonNullable<Candidate["rule"]> {
  return {
    format: "rerail.policy/v1",
    id,
    revision: 1,
    mode: "steer",
    scope: { include: [...scope], exclude: [] },
    clauses: [{
      id: "check",
      forbid: { regex: { pattern, source: "file.content" } },
      violation: { code: "MERGE_ASSIST_DECISION", anchor: "witness.span", message },
    }],
  } as unknown as NonNullable<Candidate["rule"]>;
}

export function candidate(overrides: Partial<Candidate> & Pick<Candidate, "id">): Candidate {
  return {
    format: "rerail.candidate/v1",
    decision: "Retries live in the transport layer; callers don't wrap requests in retry loops.",
    generality: "repository",
    scope: ["src/api/**"],
    author: "jbarnes",
    createdAt: "2026-11-03T14:02:11Z",
    state: "active",
    supersedes: null,
    ...overrides,
  };
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

export function captureVerdict(overrides: Partial<CaptureVerdict> = {}): CaptureVerdict {
  return {
    generality: "repository",
    duplicateOf: null,
    amends: null,
    decision: "Retries live in the transport layer; callers don't wrap requests in retry loops.",
    scope: ["src/api/**"],
    check: { kind: "regex", pattern: String.raw`\bretry\(` },
    message: "Retries live in the transport layer; call the client once.",
    cases: {
      caught: [{ path: "src/api/orders.ts", content: "export const orders = () => retry(() => client.get(\"/orders\"));\n" }],
      passed: [{ path: "src/api/orders.ts", content: "export const orders = () => client.get(\"/orders\");\n" }],
    },
    signals: [],
    ...overrides,
  };
}

/** A comparison answer: each of theirs `compatible` unless `relations` says otherwise. */
export function compareAnswer(
  task: JudgeTask<unknown>,
  relations: Readonly<Record<string, Partial<CompareVerdict["items"][number]>>> = {},
): CompareVerdict {
  // Theirs are the lines that name an author; this branch's do not.
  const ids = [...new Set([...task.prompt.matchAll(/^- (candidate:[a-z0-9-]+) \(scope: [^;\n]*; author: /gmu)].map(match => match[1]!))];
  return {
    items: ids.map(candidateId => ({
      candidateId, relation: "compatible", mineId: null, explanation: "They agree.", message: null, ...relations[candidateId],
    })),
  };
}

export interface FakeGithubOptions {
  readiness?: GithubReadiness;
  pullRequests?: PullRequestSummary[];
  candidates?: Readonly<Record<string, CandidateSet>>;
  permissions?: Readonly<Record<string, AuthorPermission>>;
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

  async candidatesAt(_repository: GithubRepository, commit: string): Promise<CandidateSet> {
    return this.options.candidates?.[commit] ?? { candidates: [], problems: [], truncated: false };
  }

  async authorPermission(_repository: GithubRepository, login: string): Promise<AuthorPermission> {
    return this.options.permissions?.[login] ?? "none";
  }
}

export interface Workspace {
  /** The temporary folder holding everything. */
  readonly folder: string;
  /** The main checkout, on branch `mine`, branched from `main`. */
  readonly repo: string;
  readonly database: RerailDatabase;
  readonly databasePath: string;
  readonly ruleCatalog: RuleCatalog;
  readonly repositoryKey: string;
  /** A Merge Assist over the workspace's database, with these stand-ins; the experimental decision ledger only when `decisionLedger`. */
  service(options?: { judge?: AgentJudge; github?: GithubClient; now?: () => number; decisionLedger?: boolean }): MergeAssistRuntime;
  /** A linked worktree of the repository at `<folder>/<name>`, on a new branch from `main`. */
  worktree(name: string, options?: { record?: boolean }): string;
  /** The rule ids on in the checkout at `path`, as a session there would load them. */
  rulesOn(path: string): string[];
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
    const ruleCatalog = loadRuleCatalog();
    const { repositoryKey } = recordCheckout(database, repo);
    if (setUp) setUpRepository(database, repositoryKey, { packs: [] });
    setRepositoryStarted(database, repositoryKey, true);
    cleanups.push(async () => {
      database.$client.close();
      await rm(folder, { recursive: true, force: true });
    });
    return {
      folder,
      repo,
      database,
      databasePath,
      ruleCatalog,
      repositoryKey,
      service: (options = {}) => createMergeAssistService({
        database,
        ruleCatalog,
        judge: options.judge ?? new FakeJudge(task => (task.kind === "compare" ? compareAnswer(task) : captureVerdict())),
        github: options.github ?? new FakeGithub(),
        ...(options.now === undefined ? {} : { now: options.now }),
        decisionLedger: options.decisionLedger ?? false,
      }),
      worktree: (name, { record = false } = {}) => {
        const path = join(folder, name);
        git(repo, "worktree", "add", "-q", "-b", name, path, "main");
        if (record) recordCheckout(database, path);
        return path;
      },
      rulesOn: path => {
        const checkout = lookupCheckout(database, path)!;
        // As a session there loads them: a parent checkout's settings apply unless this checkout's own say otherwise.
        const parentCheckoutPaths = database.select().from(checkouts).all()
          .filter(row => row.repositoryKey === checkout.repositoryKey && checkout.checkoutPath.startsWith(`${row.checkoutPath}/`))
          .map(row => row.checkoutPath);
        return listRepositoryRules(database, ruleCatalog, checkout.repositoryKey, { checkoutPath: checkout.checkoutPath, parentCheckoutPaths }).rules
          .filter(rule => rule.on).map(rule => rule.document.rule.id).sort();
      },
    };
  };
}
