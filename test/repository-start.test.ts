import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, writeFileSync } from "node:fs";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { basename, join } from "node:path";
import Database from "better-sqlite3";
import { afterEach, describe, expect, it } from "vitest";
import { openRerailDatabase } from "../src/database.js";
import { seedRerailDatabase } from "../src/seed.js";
import {
  effectiveStartState,
  setCheckoutState,
  isRerailStarted,
  readRepositoryStartState,
  repositoryStartState,
  startRepository,
  startRerail,
  startStateReader,
  stopRepository,
  stopRerail,
  workspaceStartState,
  type RepositorySetupIo,
} from "../src/repository-start.js";
import { readRepositoryId, repositoryIdentityLocation } from "../src/repository-identity.js";
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
    expect(setUp.ruleCount).toBeGreaterThan(0);
    expect(first.lines).toEqual([`● Rerail is on in acme-web. ${setUp.ruleCount} rules are on.`]);

    const stop = recorder();
    expect(stopRerail(main, stop, databasePath)).toBe("started");
    expect(stop.lines[0]).toMatch(/^○ Rerail is stopped in acme-web\. /u);
    expect(workspaceStartState(main, databasePath)).toBe("stopped");

    // A rule switched off while stopped stays off: starting again does not run the set-up a second time.
    const context = resolveRepositoryContext(main);
    const catalog = ScopedRuleCatalog.open(databasePath, context);
    const switchedOff = catalog.list({ enabled: true })[0]!.rule.id;
    catalog.setEnabled({ ids: [switchedOff] }, false);
    catalog.close();
    const again = recorder("terminal");
    const restarted = await startRerail(main, again, { databasePath });
    expect(restarted).toMatchObject({ previous: "stopped", ruleCount: setUp.ruleCount - 1 });
    expect(again.lines).toEqual([`● Rerail is on in acme-web. ${setUp.ruleCount - 1} rules are on, as you left them.`]);
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

  it("treats a development workspace that runs its own container as started until it is stopped, even never set up", async () => {
    const root = await temporary("rerail-start-dev-");
    const databasePath = join(root, "rerail.sqlite");
    const checkout = gitRepository(join(root, "checkout"));
    mkdirSync(join(checkout, ".nera"));
    writeFileSync(join(checkout, ".nera", "instance.env"), 'NERA_RERAIL_ENABLED="true"\nNERA_RERAIL_PUBLIC_URL="http://127.0.0.1:4737"\n');
    expect(isRerailStarted(checkout, databasePath)).toBe(true);

    const io = recorder("terminal");
    expect(stopRerail(checkout, io, databasePath)).toBe("started");
    expect(io.lines[0]).toMatch(/^○ Rerail is stopped in checkout\. /u);
    expect(isRerailStarted(checkout, databasePath)).toBe(false);
    expect(workspaceStartState(checkout, databasePath)).toBe("stopped");

    await startRerail(checkout, recorder(), { databasePath });
    expect(isRerailStarted(checkout, databasePath)).toBe(true);
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
        // Closed now, not at dispose: its code-rule bridge and polling are released while the runtime lives on.
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

  it("never starts a repository from a session this release recorded under a development workspace's flag", async () => {
    const root = await temporary("rerail-dev-flag-");
    const databasePath = join(root, "rerail.sqlite");
    const main = gitRepository(join(root, "acme"));
    const flagged = join(root, "acme-flagged");
    const sibling = join(root, "acme-sibling");
    execFileSync("git", ["worktree", "add", "-q", "-b", "flagged", flagged], { cwd: main });
    execFileSync("git", ["worktree", "add", "-q", "-b", "sibling", sibling], { cwd: main });
    mkdirSync(join(flagged, ".nera"));
    writeFileSync(join(flagged, ".nera", "instance.env"), 'NERA_RERAIL_ENABLED="true"\nNERA_RERAIL_PUBLIC_URL="http://127.0.0.1:4737"\n');
    // A gated runtime in this database (the event stream or engine service) runs there through the flag.
    const runtime = new RerailRuntime({
      storageRoot: join(root, "sessions"), databasePath, workspaceRoot: flagged,
      embedder: testEmbedder, autostart: false, startGate: true,
    });
    try {
      await runtime.ingest({ kind: "session-start", eventId: "start", session: { host: "omp", sessionId: "flagged", cwd: flagged } });
    } finally {
      await runtime.dispose();
    }
    const raw = new Database(databasePath, { readonly: true });
    try {
      expect(raw.prepare("SELECT session_id FROM sessions").pluck().all()).toEqual(["flagged"]);
    } finally {
      raw.close();
    }

    // A writable open runs the carry-over; the flag alone must not start the repository.
    openRerailDatabase(databasePath).$client.close();
    expect(workspaceStartState(main, databasePath)).toBe("never");
    expect(workspaceStartState(sibling, databasePath)).toBe("never");
    expect(effectiveStartState(flagged, databasePath)).toBe("started");
  });
});

describe("the v16 upgrade", () => {
  it("keeps every repository with history or a catalog checkout started, and stops one with the retired marker", async () => {
    const root = await temporary("rerail-upgrade-");
    const databasePath = join(root, "rerail.sqlite");
    // A pre-v13 session carries no repository; its checkout's identity file names it.
    const unattributed = gitRepository(join(root, "unattributed"));
    const unattributedId = resolveRepositoryContext(unattributed).repositoryId;
    const markedCheckout = join(root, "marked");
    mkdirSync(join(markedCheckout, ".rerail"), { recursive: true });
    writeFileSync(join(markedCheckout, ".rerail", "disabled"), "");

    openRerailDatabase(databasePath).$client.close();
    const raw = new Database(databasePath);
    raw.exec([
      "INSERT INTO sessions (session_id, host, cwd, repository_id, started_at) VALUES ('a', 'omp', '/gone/a', 'repo-a', 1)",
      `INSERT INTO sessions (session_id, host, cwd, repository_id, started_at) VALUES ('b', 'omp', '${unattributed}', NULL, 1)`,
      "INSERT INTO sessions (session_id, host, cwd, repository_id, started_at) VALUES ('c', 'omp', '/gone/never-identified', NULL, 1)",
      `INSERT INTO rule_repositories (repository_id, last_known_path, created_at, updated_at) VALUES ('repo-marked', '${markedCheckout}', 1, 1)`,
      "INSERT INTO rule_repositories (repository_id, last_known_path, created_at, updated_at) VALUES ('repo-member-only', '', 1, 1)",
      "DROP TABLE repository_starts",
      "UPDATE rerail_schema_revision SET revision = 15",
    ].join(";"));
    raw.close();

    expect(readRepositoryStartState(databasePath, "repo-a")).toBe("started");
    const upgraded = new Database(databasePath, { readonly: true });
    try {
      const states = Object.fromEntries((upgraded.prepare("SELECT repository_id, state FROM repository_starts").all() as Array<{ repository_id: string; state: string }>)
        .map(row => [row.repository_id, row.state]));
      expect(states).toEqual({ "repo-a": "started", [unattributedId]: "started", "repo-marked": "stopped" });
      expect(upgraded.prepare("SELECT revision FROM rerail_schema_revision").pluck().get()).toBe(16);
      expect(upgraded.pragma("user_version", { simple: true })).toBe(15);
    } finally {
      upgraded.close();
    }
  });

  it("upgrades a v15 database the first time a passive start check reads it", async () => {
    const root = await temporary("rerail-upgrade-passive-");
    const databasePath = join(root, "rerail.sqlite");
    const checkout = gitRepository(join(root, basename(root)));
    const repositoryId = resolveRepositoryContext(checkout).repositoryId;
    openRerailDatabase(databasePath).$client.close();
    const raw = new Database(databasePath);
    raw.exec([
      `INSERT INTO sessions (session_id, host, cwd, repository_id, started_at) VALUES ('s', 'claude-code', '${checkout}', '${repositoryId}', 1)`,
      "DROP TABLE repository_starts",
      "UPDATE rerail_schema_revision SET revision = 15",
    ].join(";"));
    raw.close();

    expect(readRepositoryId(checkout)).toBe(repositoryId);
    expect(isRerailStarted(checkout, databasePath)).toBe(true);
  });

  it("backfills a database a later build stamped past v16 without the table, once", async () => {
    const root = await temporary("rerail-upgrade-v17-");
    const databasePath = join(root, "rerail.sqlite");
    const markedCheckout = join(root, "marked");
    mkdirSync(join(markedCheckout, ".rerail"), { recursive: true });
    writeFileSync(join(markedCheckout, ".rerail", "disabled"), "");
    openRerailDatabase(databasePath).$client.close();
    const raw = new Database(databasePath);
    raw.exec([
      "INSERT INTO sessions (session_id, host, cwd, repository_id, started_at) VALUES ('a', 'omp', '/gone/a', 'repo-a', 1)",
      `INSERT INTO rule_repositories (repository_id, last_known_path, created_at, updated_at) VALUES ('repo-marked', '${markedCheckout}', 1, 1)`,
      // A parallel line's build (rule packs, v17) opened the file first and never created repository_starts.
      "DROP TABLE repository_starts",
      "UPDATE rerail_schema_revision SET revision = 17",
    ].join(";"));
    raw.close();

    expect(readRepositoryStartState(databasePath, "repo-a")).toBe("started");
    expect(readRepositoryStartState(databasePath, "repo-marked")).toBe("stopped");
    const reopened = new Database(databasePath, { readonly: true });
    try {
      // The later stamp is kept; this build never lowers a revision.
      expect(reopened.prepare("SELECT revision FROM rerail_schema_revision").pluck().get()).toBe(17);
    } finally {
      reopened.close();
    }

    // The backfill runs once; the carry-over after it never changes a row, so later choices survive every reopen.
    const database = openRerailDatabase(databasePath);
    try {
      startRepository(database, "repo-marked", 100);
      stopRepository(database, "repo-a", 100);
    } finally {
      database.$client.close();
    }
    expect(readRepositoryStartState(databasePath, "repo-a")).toBe("stopped");
    expect(readRepositoryStartState(databasePath, "repo-marked")).toBe("started");
  });

  it("starts repositories an older release used after a rollback, and keeps every choice made before", async () => {
    const root = await temporary("rerail-rollback-");
    const databasePath = join(root, "rerail.sqlite");
    // This release ran first: one repository started, one stopped.
    const database = openRerailDatabase(databasePath);
    startRepository(database, "repo-started");
    startRepository(database, "repo-chosen-off");
    stopRepository(database, "repo-chosen-off");
    // Set up from a worktree only: its main checkout was never turned on.
    setCheckoutState(database, { repositoryId: "repo-worktree-only", root: "/elsewhere/feature", main: false }, "started");
    database.$client.close();
    // Rolled back, a release from before v16 records in every repository, without attribution, and its
    // `rerail disable` writes the retired marker.
    const usedWhileRolledBack = gitRepository(join(root, "used"));
    const usedId = resolveRepositoryContext(usedWhileRolledBack).repositoryId;
    const disabledWhileRolledBack = gitRepository(join(root, "disabled"));
    const disabledId = resolveRepositoryContext(disabledWhileRolledBack).repositoryId;
    mkdirSync(join(disabledWhileRolledBack, ".rerail"), { recursive: true });
    writeFileSync(join(disabledWhileRolledBack, ".rerail", "disabled"), "");
    // A repository whose old marker sits in one linked worktree only.
    const split = gitRepository(join(root, "split"));
    const splitWorktree = join(root, "split-feature");
    execFileSync("git", ["worktree", "add", "-q", "-b", "feature", splitWorktree], { cwd: split });
    resolveRepositoryContext(split);
    mkdirSync(join(splitWorktree, ".rerail"), { recursive: true });
    writeFileSync(join(splitWorktree, ".rerail", "disabled"), "");
    const raw = new Database(databasePath);
    raw.exec([
      `INSERT INTO sessions (session_id, host, cwd, started_at) VALUES ('used', 'claude-code', '${usedWhileRolledBack}', 2)`,
      `INSERT INTO sessions (session_id, host, cwd, started_at) VALUES ('disabled', 'claude-code', '${disabledWhileRolledBack}', 2)`,
      `INSERT INTO sessions (session_id, host, cwd, started_at) VALUES ('split', 'claude-code', '${split}', 2)`,
      `INSERT INTO sessions (session_id, host, cwd, started_at) VALUES ('split-feature', 'claude-code', '${splitWorktree}', 2)`,
      "INSERT INTO sessions (session_id, host, cwd, repository_id, started_at) VALUES ('off', 'omp', '/gone/off', 'repo-chosen-off', 2)",
      "INSERT INTO sessions (session_id, host, cwd, repository_id, started_at) VALUES ('feature', 'omp', '/elsewhere/feature', 'repo-worktree-only', 2)",
    ].join(";"));
    raw.close();

    // Updated again: the first passive hook check carries the history over, with no writer before it.
    expect(workspaceStartState(usedWhileRolledBack, databasePath)).toBe("started");
    expect(readRepositoryStartState(databasePath, usedId)).toBe("started");
    expect(readRepositoryStartState(databasePath, disabledId)).toBe("stopped");
    expect(readRepositoryStartState(databasePath, "repo-chosen-off")).toBe("stopped");
    expect(readRepositoryStartState(databasePath, "repo-started")).toBe("started");
    expect(readRepositoryStartState(databasePath, "repo-worktree-only")).toBe("never");
    expect(workspaceStartState(split, databasePath)).toBe("started");
    expect(workspaceStartState(splitWorktree, databasePath)).toBe("stopped");
  });

  it("keeps a repository never started after `rerail seed` records its mock session", async () => {
    const root = await temporary("rerail-seed-");
    const databasePath = join(root, "rerail.sqlite");
    const checkout = gitRepository(join(root, "seeded"));
    seedRerailDatabase(databasePath, { cwd: checkout, repositoryId: resolveRepositoryContext(checkout).repositoryId });
    openRerailDatabase(databasePath).$client.close();
    expect(workspaceStartState(checkout, databasePath)).toBe("never");
  });

  it("carries over sessions an older release records after a reset reused rowids below the watermark", async () => {
    const root = await temporary("rerail-reset-watermark-");
    const databasePath = join(root, "rerail.sqlite");
    const database = openRerailDatabase(databasePath);
    startRepository(database, "repo-earlier");
    database.$client.close();
    const older = (statements: readonly string[]) => {
      const raw = new Database(databasePath);
      try {
        raw.exec(statements.join(";"));
      } finally {
        raw.close();
      }
    };
    older([
      "INSERT INTO sessions (session_id, host, cwd, repository_id, started_at) VALUES ('one', 'omp', '/gone/one', 'repo-earlier', 1)",
      "INSERT INTO sessions (session_id, host, cwd, repository_id, started_at) VALUES ('two', 'omp', '/gone/two', 'repo-earlier', 2)",
    ]);
    openRerailDatabase(databasePath).$client.close();
    // `rerail reset` deletes the newest sessions; the next rowids handed out sit below the old watermark.
    older(["DELETE FROM sessions"]);
    openRerailDatabase(databasePath).$client.close();
    older(["INSERT INTO sessions (session_id, host, cwd, repository_id, started_at) VALUES ('after', 'omp', '/gone/after', 'repo-after-reset', 3)"]);
    expect(readRepositoryStartState(databasePath, "repo-after-reset")).toBe("started");
  });

  it("stops a linked worktree disabled before it recorded any session, and only that worktree", async () => {
    const root = await temporary("rerail-upgrade-worktree-marker-");
    const databasePath = join(root, "rerail.sqlite");
    const main = gitRepository(join(root, "acme"));
    const repositoryId = resolveRepositoryContext(main).repositoryId;
    const disabled = join(root, "acme-disabled");
    execFileSync("git", ["worktree", "add", "-q", "-b", "disabled", disabled], { cwd: main });
    mkdirSync(join(disabled, ".rerail"), { recursive: true });
    writeFileSync(join(disabled, ".rerail", "disabled"), "");
    openRerailDatabase(databasePath).$client.close();
    const raw = new Database(databasePath);
    raw.exec([
      // History only in the main checkout; the worktree was disabled before it ever ran.
      `INSERT INTO sessions (session_id, host, cwd, repository_id, started_at) VALUES ('main', 'omp', '${main}', '${repositoryId}', 1)`,
      "DROP TABLE repository_starts",
      "DROP TABLE checkout_starts",
      "UPDATE rerail_schema_revision SET revision = 15",
    ].join(";"));
    raw.close();

    expect(workspaceStartState(main, databasePath)).toBe("started");
    expect(workspaceStartState(disabled, databasePath)).toBe("stopped");
  });
});
