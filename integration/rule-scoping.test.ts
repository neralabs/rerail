import { execFileSync } from "node:child_process";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { defaultRerailDatabasePath } from "../src/database.js";
import { openScopedRuleCatalog, resolveRepositoryContext } from "../src/scoped-rule-catalog.js";
import { MOCK_COSINE_RULES } from "../src/seed.js";
import { SqliteTranscriptReader, SqliteTranscriptStore } from "../src/transcript-store.js";

const roots: string[] = [];
let previousRerailHome: string | undefined;

afterEach(async () => {
  if (previousRerailHome === undefined) delete process.env.RERAIL_HOME;
  else process.env.RERAIL_HOME = previousRerailHome;
  previousRerailHome = undefined;
  await Promise.all(roots.splice(0).map(path => rm(path, { recursive: true, force: true })));
});

function initializeRepository(path: string): void {
  execFileSync("git", ["init", path]);
  execFileSync("git", ["-C", path, "config", "user.email", "rerail@example.test"]);
  execFileSync("git", ["-C", path, "config", "user.name", "Rerail Test"]);
  execFileSync("git", ["-C", path, "commit", "--allow-empty", "-m", "initial"]);
}

describe("rule scope integration", () => {
  it("isolates histories in the one database while propagating one project catalog across worktrees and repositories", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-rule-scope-integration-"));
    roots.push(root);
    const primaryPath = join(root, "primary");
    const worktreePath = join(root, "primary-worktree");
    const secondPath = join(root, "second");
    const unrelatedPath = join(root, "unrelated");
    initializeRepository(primaryPath);
    initializeRepository(secondPath);
    initializeRepository(unrelatedPath);
    execFileSync("git", ["-C", primaryPath, "worktree", "add", worktreePath]);

    const primaryRepository = resolveRepositoryContext(primaryPath);
    const worktreeRepository = resolveRepositoryContext(worktreePath);
    const secondRepository = resolveRepositoryContext(secondPath);
    const unrelatedRepository = resolveRepositoryContext(unrelatedPath);
    expect(worktreeRepository.repositoryId).toBe(primaryRepository.repositoryId);

    previousRerailHome = process.env.RERAIL_HOME;
    process.env.RERAIL_HOME = join(root, "home");
    const primary = openScopedRuleCatalog(primaryPath);
    primary.assignRepository("project-shared", "Shared", primaryRepository.repositoryId, 1_000);
    primary.assignRepository("project-shared", "Shared", secondRepository.repositoryId, 1_001);
    primary.upsertCosine({ ...MOCK_COSINE_RULES[0]!, id: "shared" }, { applicability: "project", now: 1_100 });

    const worktree = openScopedRuleCatalog(worktreePath);
    const second = openScopedRuleCatalog(secondPath);
    const unrelated = openScopedRuleCatalog(unrelatedPath);
    expect(worktree.cosineRules().map(rule => rule.id)).toEqual(["shared"]);
    expect(second.cosineRules().map(rule => rule.id)).toEqual(["shared"]);
    expect(unrelated.cosineRules()).toEqual([]);

    const database = defaultRerailDatabasePath();
    expect(primary.path).toBe(database);
    for (const [repository, cwd, sessionId] of [
      [primaryRepository, primaryPath, "primary-session"],
      [worktreeRepository, worktreePath, "worktree-session"],
      [secondRepository, secondPath, "second-session"],
    ] as const) {
      const history = new SqliteTranscriptStore(database, { repositoryId: repository.repositoryId });
      history.openSession({ host: "omp", sessionId, cwd }, 2_000);
      history.append({ sessionId, eventId: "prompt", kind: "user-prompt", observedAt: 2_001, payload: { text: sessionId } });
      history.close();
    }
    const primaryHistory = new SqliteTranscriptReader(database, { repositoryId: primaryRepository.repositoryId });
    const secondHistory = new SqliteTranscriptReader(database, { repositoryId: secondRepository.repositoryId });
    const unrelatedHistory = new SqliteTranscriptReader(database, { repositoryId: unrelatedRepository.repositoryId });
    // Worktrees share their repository's history; other repositories, even in the same project, do not.
    expect(primaryHistory.sessions().map(session => session.sessionId)).toEqual(["primary-session", "worktree-session"]);
    expect(secondHistory.sessions().map(session => session.sessionId)).toEqual(["second-session"]);
    expect(unrelatedHistory.sessions()).toEqual([]);
    expect(secondHistory.events("second-session")[0]?.payload).toEqual({ text: "second-session" });
    primaryHistory.close();
    secondHistory.close();
    unrelatedHistory.close();

    worktree.upsertCosine(
      { ...MOCK_COSINE_RULES[0]!, id: "shared", action: { delivery: "steer", content: "Updated once" } },
      { applicability: "project", now: 3_000 },
    );
    second.close();
    const restartedSecond = openScopedRuleCatalog(secondPath);
    expect(restartedSecond.get("shared")).toMatchObject({ revision: 2, rule: { action: { content: "Updated once" } } });

    expect(primary.removeRepository("project-shared", secondRepository.repositoryId, 4_000)).toBe(true);
    expect(restartedSecond.context()).toMatchObject({ projectId: `repository:${secondRepository.repositoryId}` });
    expect(restartedSecond.get("shared")).toBeUndefined();
    expect(worktree.remove(["shared"])).toBe(1);
    expect(primary.get("shared")).toBeUndefined();

    primary.close();
    worktree.close();
    restartedSecond.close();
    unrelated.close();
  });
});
