import { execFileSync } from "node:child_process";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { defaultRerailDatabasePath, openScopedRuleCatalog, resolveRepositoryContext } from "../src/engine/index.js";
import { MOCK_CONVERSATION_RULES } from "../src/cli/index.js";
import { SqliteSessionReader, SqliteSessionStore } from "../src/runtime/index.js";

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
    primary.upsertConversationRule({ ...MOCK_CONVERSATION_RULES[0]!, id: "shared" }, { applicability: "project", now: 1_100 });

    const worktree = openScopedRuleCatalog(worktreePath);
    const second = openScopedRuleCatalog(secondPath);
    const unrelated = openScopedRuleCatalog(unrelatedPath);
    expect(worktree.conversationRules().map(rule => rule.id)).toEqual(["shared"]);
    expect(second.conversationRules().map(rule => rule.id)).toEqual(["shared"]);
    expect(unrelated.conversationRules()).toEqual([]);

    const database = defaultRerailDatabasePath();
    expect(primary.path).toBe(database);
    for (const [repository, cwd, sessionId] of [
      [primaryRepository, primaryPath, "primary-session"],
      [worktreeRepository, worktreePath, "worktree-session"],
      [secondRepository, secondPath, "second-session"],
    ] as const) {
      const history = new SqliteSessionStore(database, { repositoryId: repository.repositoryId });
      history.openSession({ host: "omp", sessionId, cwd }, 2_000);
      history.append({ sessionId, eventId: "prompt", kind: "user-prompt", observedAt: 2_001, payload: { text: sessionId } });
      history.close();
    }
    const primaryHistory = new SqliteSessionReader(database, { repositoryId: primaryRepository.repositoryId });
    const secondHistory = new SqliteSessionReader(database, { repositoryId: secondRepository.repositoryId });
    const unrelatedHistory = new SqliteSessionReader(database, { repositoryId: unrelatedRepository.repositoryId });
    // Worktrees share their repository's history; other repositories, even in the same project, do not.
    expect(primaryHistory.sessions().map(session => session.sessionId)).toEqual(["primary-session", "worktree-session"]);
    expect(secondHistory.sessions().map(session => session.sessionId)).toEqual(["second-session"]);
    expect(unrelatedHistory.sessions()).toEqual([]);
    expect(secondHistory.events("second-session")[0]?.payload).toEqual({ text: "second-session" });
    primaryHistory.close();
    secondHistory.close();
    unrelatedHistory.close();

    worktree.upsertConversationRule(
      { ...MOCK_CONVERSATION_RULES[0]!, id: "shared", action: { content: "Updated once" } },
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
