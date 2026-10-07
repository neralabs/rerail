import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync } from "node:fs";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import Database from "better-sqlite3";
import { afterEach, describe, expect, it } from "vitest";
import { openRerailDatabase } from "../src/database.js";
import { loadBundledRules } from "../src/rule-bootstrap.js";
import { MOCK_COSINE_RULES, seedRerailDatabase } from "../src/seed.js";
import {
  isRerailStarted,
  repositoryStartState,
  startRepository,
  startRerail,
  startStateReader,
  stopRepository,
  stopRerail,
  workspaceStartState,
  type RepositorySetupIo,
} from "../src/repository-start.js";
import { repositoryIdentityLocation } from "../src/repository-identity.js";
import { resolveRepositoryContext, ScopedRuleCatalog } from "../src/scoped-rule-catalog.js";
import { RerailRuntime } from "../src/runtime.js";
import { testEmbedder } from "./hook-fixtures.js";

const temporaryDirectories: string[] = [];
afterEach(async () => {
  await Promise.all(temporaryDirectories.splice(0).map(path => rm(path, { recursive: true, force: true })));
});

async function temporary(prefix: string): Promise<string> {
  const path = await mkdtemp(join(tmpdir(), prefix));
  temporaryDirectories.push(path);
  return path;
}

function gitRepository(path: string): string {
  mkdirSync(path, { recursive: true });
  execFileSync("git", ["init", "-q", "-b", "main"], { cwd: path });
  execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@example.invalid", "commit", "-q", "--allow-empty", "-m", "base"], { cwd: path });
  return path;
}

function recorder(lane: RepositorySetupIo["lane"] = "agent"): RepositorySetupIo & { lines: string[] } {
  const lines: string[] = [];
  return { lane, lines, say: text => { lines.push(text); } };
}

describe("repository start state", () => {
  it("is off until started, leaves no trace while off, and keeps rules when started again", async () => {
    const root = await temporary("rerail-start-");
    const databasePath = join(root, "home", "rerail.sqlite");
    const main = gitRepository(join(root, "acme-web"));

    expect(isRerailStarted(main, databasePath)).toBe(false);
    expect(existsSync(databasePath)).toBe(false);
    expect(existsSync(repositoryIdentityLocation(main).identityPath)).toBe(false);

    const first = recorder();
    const setUp = await startRerail(main, first, { databasePath });
    expect(setUp).toMatchObject({ previous: "never", repositoryName: "acme-web" });
    // Core seeds only its enabled-by-default bundled rules; the opt-in packs stay off.
    expect(setUp.ruleCount).toBe(loadBundledRules().filter(document => document.enabledByDefault !== false).length);
    expect(first.lines).toEqual([`● Rerail is on in acme-web. ${setUp.ruleCount} rules are on.`]);

    const stop = recorder();
    expect(stopRerail(main, stop, databasePath)).toBe("started");
    expect(stop.lines[0]).toMatch(/^○ Rerail is stopped in acme-web\. /u);
    expect(workspaceStartState(main, databasePath)).toBe("stopped");

    // A rule switched off while stopped stays off: starting again does not run the set-up a second time.
    const context = resolveRepositoryContext(main);
    const catalog = ScopedRuleCatalog.open(databasePath, context);
    catalog.upsertCosine({ ...MOCK_COSINE_RULES[0]!, id: "kept-on" });
    catalog.upsertCosine({ ...MOCK_COSINE_RULES[0]!, id: "switched-off" });
    const switchedOff = "switched-off";
    catalog.setEnabled({ ids: [switchedOff] }, false);
    catalog.close();
    const again = recorder("terminal");
    const restarted = await startRerail(main, again, { databasePath });
    const onAgain = setUp.ruleCount + 1;
    expect(restarted).toMatchObject({ previous: "stopped", ruleCount: onAgain });
    expect(again.lines).toEqual([`● Rerail is on in acme-web. ${onAgain} ${onAgain === 1 ? "rule is" : "rules are"} on, as you left them.`]);
    const reopened = ScopedRuleCatalog.open(databasePath, context);
    expect(reopened.get(switchedOff)?.enabled).toBe(false);
    reopened.close();
  });

  it("turns each worktree on or off by itself, and the worktrees inside it follow until changed themselves", async () => {
    const root = await temporary("rerail-worktrees-");
    const databasePath = join(root, "home", "rerail.sqlite");
    const main = gitRepository(join(root, "acme-web"));
    const worktree = (path: string, branch: string): string => {
      execFileSync("git", ["worktree", "add", "-q", "-b", branch, path], { cwd: main });
      return path;
    };
    const beside = worktree(join(root, "acme-web-beside"), "beside");
    const outer = worktree(join(main, ".worktrees", "outer"), "outer");
    const inner = worktree(join(outer, ".worktrees", "inner"), "inner");
    mkdirSync(join(inner, "src"));
    const states = (): Record<string, string> => ({
      main: workspaceStartState(main, databasePath),
      beside: workspaceStartState(beside, databasePath),
      outer: workspaceStartState(outer, databasePath),
      inner: workspaceStartState(join(inner, "src"), databasePath),
    });

    // Set up from a worktree: the repository is set up once, and only that worktree and the one inside it start.
    const setUp = recorder();
    await startRerail(outer, setUp, { databasePath });
    expect(setUp.lines[0]).toMatch(/^● Rerail is on in outer, a worktree of acme-web\. /u);
    expect(states()).toEqual({ main: "never", beside: "never", outer: "started", inner: "started" });

    // The main checkout reaches every worktree that has no setting of its own.
    await startRerail(main, recorder(), { databasePath });
    stopRerail(main, recorder(), databasePath);
    expect(states()).toEqual({ main: "stopped", beside: "stopped", outer: "started", inner: "started" });

    // A worktree nested two deep keeps its own setting when the one around it changes.
    stopRerail(inner, recorder(), databasePath);
    await startRerail(main, recorder(), { databasePath });
    stopRerail(outer, recorder(), databasePath);
    await startRerail(inner, recorder(), { databasePath });
    expect(states()).toEqual({ main: "started", beside: "started", outer: "stopped", inner: "started" });

    // The daemon resolves the same way over its held connection.
    const database = openRerailDatabase(databasePath);
    try {
      const read = startStateReader(database);
      expect([main, beside, outer, inner].map(read)).toEqual(["started", "started", "stopped", "started"]);
    } finally {
      database.$client.close();
    }
  });

  it("keeps the first start time and never turns a repository that was never set up into a stopped one", async () => {
    const root = await temporary("rerail-start-idempotent-");
    const database = openRerailDatabase(join(root, "rerail.sqlite"));
    try {
      stopRepository(database, "repo", 5);
      expect(repositoryStartState(database, "repo")).toBe("never");
      startRepository(database, "repo", 10);
      startRepository(database, "repo", 20);
      stopRepository(database, "repo", 30);
      stopRepository(database, "repo", 40);
      startRepository(database, "repo", 50);
      const row = database.$client.prepare("SELECT state, started_at, stopped_at FROM repository_starts").get();
      expect(row).toEqual({ state: "started", started_at: 50, stopped_at: 30 });
    } finally {
      database.$client.close();
    }
  });

  it("makes a resident runtime record and steer nothing once the repository is stopped elsewhere", async () => {
    const root = await temporary("rerail-start-resident-");
    const databasePath = join(root, "rerail.sqlite");
    const checkout = gitRepository(join(root, "resident"));
    const repositoryId = resolveRepositoryContext(checkout).repositoryId;
    const database = openRerailDatabase(databasePath);
    startRepository(database, repositoryId);
    const runtime = new RerailRuntime({
      storageRoot: join(root, "sessions"), databasePath, workspaceRoot: checkout,
      embedder: testEmbedder, autostart: false, startGate: true,
    });
    const session = { host: "omp" as const, sessionId: "resident", cwd: checkout };
    try {
      await runtime.ingest({ kind: "session-start", eventId: "start", session });
      await runtime.ingest({ kind: "user-prompt", eventId: "before", sessionId: "resident", text: "while started" });
      // Another session, the CLI or the dashboard stops the repository; this runtime stays attached.
      stopRepository(database, repositoryId);
      expect(await runtime.ingest({ kind: "user-prompt", eventId: "after", sessionId: "resident", text: "while stopped" })).toEqual([]);
      startRepository(database, repositoryId);
      await runtime.ingest({ kind: "user-prompt", eventId: "again", sessionId: "resident", text: "started again" });
    } finally {
      await runtime.dispose();
      database.$client.close();
    }
    const raw = new Database(databasePath, { readonly: true });
    try {
      const prompts = raw.prepare("SELECT event_id FROM events WHERE kind = 'user-prompt' ORDER BY sequence").pluck().all();
      expect(prompts).toEqual(["before", "again"]);
    } finally {
      raw.close();
    }
  });

  it("records nothing for an event accepted while started that runs after a stop", async () => {
    const root = await temporary("rerail-start-lane-");
    const databasePath = join(root, "rerail.sqlite");
    const checkout = gitRepository(join(root, "lane"));
    const repositoryId = resolveRepositoryContext(checkout).repositoryId;
    const database = openRerailDatabase(databasePath);
    startRepository(database, repositoryId);
    const publish = Promise.withResolvers<void>();
    const runtime = new RerailRuntime({
      storageRoot: join(root, "sessions"), databasePath, workspaceRoot: checkout,
      embedder: testEmbedder, autostart: false, startGate: true,
      // Holds the session start inside its lane, so the next event queues behind it.
      beforeSessionPublish: () => publish.promise,
    });
    const session = { host: "omp" as const, sessionId: "lane", cwd: checkout };
    try {
      const start = runtime.ingest({ kind: "session-start", eventId: "start", session });
      const queued = runtime.ingest({ kind: "user-prompt", eventId: "queued", sessionId: "lane", text: "accepted while started" });
      stopRepository(database, repositoryId);
      publish.resolve();
      await start;
      expect(await queued).toEqual([]);
    } finally {
      await runtime.dispose();
      database.$client.close();
    }
    const raw = new Database(databasePath, { readonly: true });
    try {
      expect(raw.prepare("SELECT event_id FROM events WHERE kind = 'user-prompt'").pluck().all()).toEqual([]);
    } finally {
      raw.close();
    }
  });

  it("still ends a session that attached while started when it ends after a stop, recording no event", async () => {
    const root = await temporary("rerail-start-end-");
    const databasePath = join(root, "rerail.sqlite");
    const checkout = gitRepository(join(root, "ending"));
    const repositoryId = resolveRepositoryContext(checkout).repositoryId;
    const database = openRerailDatabase(databasePath);
    startRepository(database, repositoryId);
    const runtime = new RerailRuntime({
      storageRoot: join(root, "sessions"), databasePath, workspaceRoot: checkout,
      embedder: testEmbedder, autostart: false, startGate: true,
    });
    const session = { host: "omp" as const, sessionId: "ending", cwd: checkout };
    try {
      await runtime.ingest({ kind: "session-start", eventId: "start", session });
      stopRepository(database, repositoryId);
      expect(await runtime.ingest({ kind: "session-end", eventId: "end", sessionId: "ending" })).toEqual([]);
      const raw = new Database(databasePath, { readonly: true });
      try {
        // Closed now, not at dispose: its policy bridge and polling are released while the runtime lives on.
        expect(raw.prepare("SELECT ended_at FROM sessions WHERE session_id = 'ending'").pluck().get()).not.toBeNull();
        expect(raw.prepare("SELECT event_id FROM events WHERE event_id = 'end'").pluck().all()).toEqual([]);
      } finally {
        raw.close();
      }
    } finally {
      await runtime.dispose();
      database.$client.close();
    }
  });

  it("says there is nothing to stop where Rerail was never set up, naming the command for the lane", async () => {
    const root = await temporary("rerail-stop-never-");
    const checkout = gitRepository(join(root, "landing-page"));
    const io = recorder("terminal");
    expect(stopRerail(checkout, io, join(root, "rerail.sqlite"))).toBe("never");
    expect(io.lines).toEqual(["○ Rerail has not been started in landing-page, so there is nothing to stop. Start it with rerail start."]);
  });

  it("keeps a repository never started after `rerail seed` records its mock session", async () => {
    const root = await temporary("rerail-seed-");
    const databasePath = join(root, "rerail.sqlite");
    const checkout = gitRepository(join(root, "seeded"));
    seedRerailDatabase(databasePath, { cwd: checkout, repositoryId: resolveRepositoryContext(checkout).repositoryId });
    openRerailDatabase(databasePath).$client.close();
    expect(workspaceStartState(checkout, databasePath)).toBe("never");
  });
});
