import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, realpathSync, writeFileSync } from "node:fs";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import Database from "better-sqlite3";
import { afterEach, describe, expect, it } from "vitest";
import { openRerailDatabase } from "../src/database.js";
import { loadBundledCatalog } from "../src/bundled-rules.js";
import {
  isRerailStarted,
  readOnlyStartState,
  sessionNoteRules,
  setUpWithRecommendedPacks,
  startRerail,
  startStateReader,
  stopRerail,
  workspaceStartState,
  type RepositorySetupIo,
} from "../src/repository-start.js";
import { repositoryIdentityLocation } from "../src/repository-identity.js";
import { lookupCheckout, recordCheckout } from "../src/repository-registry.js";
import { listRepositoryRules, setRuleSetting } from "../src/rule-configuration.js";
import { loadRulePacks } from "../src/rule-packs.js";
import { RerailRuntime } from "../src/runtime.js";
import { testEmbedder } from "./hook-fixtures.js";
import { startTestRepository, stopTestRepository } from "./support/started-repository.js";

const temporaryDirectories: string[] = [];
afterEach(async () => {
  await Promise.all(temporaryDirectories.splice(0).map(path => rm(path, { recursive: true, force: true })));
});

async function temporary(prefix: string): Promise<string> {
  const path = realpathSync(await mkdtemp(join(tmpdir(), prefix)));
  temporaryDirectories.push(path);
  return path;
}

function git(cwd: string, ...args: string[]): void {
  execFileSync("git", args, { cwd, stdio: "ignore" });
}

function gitRepository(path: string, origin?: string): string {
  mkdirSync(path, { recursive: true });
  git(path, "init", "-q", "-b", "main");
  git(path, "-c", "user.name=Rerail", "-c", "user.email=rerail@example.invalid", "commit", "-q", "--allow-empty", "-m", "base");
  if (origin !== undefined) git(path, "remote", "add", "origin", origin);
  return path;
}

function recorder(lane: RepositorySetupIo["lane"] = "agent"): RepositorySetupIo & { lines: string[] } {
  const lines: string[] = [];
  return { lane, lines, say: text => { lines.push(text); } };
}

const tslintRuleCount = loadRulePacks().find(pack => pack.id === "tslint")!.rules.length;

function writeFiles(root: string, files: Record<string, string>): void {
  for (const [path, content] of Object.entries(files)) {
    mkdirSync(join(root, path, ".."), { recursive: true });
    writeFileSync(join(root, path), content);
  }
}

/** Each attached pack of `workspace`'s repository with the folder it was added at. */
function attachedPacks(workspace: string, databasePath: string): string[] {
  const database = openRerailDatabase(databasePath, { readonly: true });
  try {
    const checkout = lookupCheckout(database, workspace)!;
    const { rules } = listRepositoryRules(database, loadBundledCatalog(), checkout.repositoryKey);
    return [...new Set(rules.map(rule => `${rule.source.packId}@${rule.attachmentFolder}`))].sort();
  } finally {
    database.$client.close();
  }
}

describe("repository start state", () => {
  it("is off until started, leaves no trace while off, and keeps rules when started again", async () => {
    const root = await temporary("rerail-start-");
    const databasePath = join(root, "home", "rerail.sqlite");
    const main = gitRepository(join(root, "acme-web"));
    writeFiles(main, { "src/index.ts": "export {};\n" });

    expect(isRerailStarted(main, databasePath)).toBe(false);
    expect(existsSync(databasePath)).toBe(false);
    expect(existsSync(repositoryIdentityLocation(main).identityPath)).toBe(false);
    expect(sessionNoteRules(main, databasePath)).toEqual({ setUp: false });
    expect(readOnlyStartState(main, databasePath)).toBe("stopped");

    const first = recorder();
    const setUp = await startRerail(main, first, { databasePath });
    expect(setUp).toMatchObject({ previous: "stopped", repositoryName: "acme-web", ruleCount: tslintRuleCount, addedPacks: [{ packId: "tslint", folder: "src" }] });
    expect(first.lines).toEqual([[
      `● Rerail is on in acme-web. ${tslintRuleCount} rules are on.`,
      "  Rule packs added:",
      `  · TSLint in src (${tslintRuleCount} rules): matches *.ts files in src`,
    ].join("\n")]);
    expect(workspaceStartState(main, databasePath)).toBe("started");
    expect(readOnlyStartState(main, databasePath)).toBe("started");
    expect(sessionNoteRules(main, databasePath)).toEqual({ setUp: true, ruleCount: tslintRuleCount });

    const stop = recorder();
    expect(stopRerail(main, stop, { databasePath })).toBe("started");
    expect(stop.lines[0]).toMatch(/^○ Rerail is stopped in acme-web\. /u);
    expect(workspaceStartState(main, databasePath)).toBe("stopped");

    // A rule switched off while stopped stays off: starting again does not run the set-up a second time.
    const database = openRerailDatabase(databasePath);
    try {
      const { repositoryKey } = lookupCheckout(database, main)!;
      setRuleSetting(database, { owner: { kind: "repository", repositoryKey }, folder: "src", targetKind: "rule", targetId: "tslint:no-debugger", on: false, mode: null });
    } finally {
      database.$client.close();
    }
    writeFiles(main, { "api/acme_api/app.py": "app = None\n" });
    const again = recorder("terminal");
    const restarted = await startRerail(main, again, { databasePath });
    expect(restarted).toMatchObject({ previous: "stopped", ruleCount: tslintRuleCount - 1, addedPacks: [] });
    expect(again.lines).toEqual([`● Rerail is on in acme-web. ${tslintRuleCount - 1} rules are on, as you left them.`]);
    expect(attachedPacks(main, databasePath)).toEqual(["tslint@src"]);
    // The session-start note counts the rules on in this checkout, as the person left them.
    expect(sessionNoteRules(main, databasePath)).toEqual({ setUp: true, ruleCount: tslintRuleCount - 1 });
  });

  it("sets a repository up with each recommended pack at the folder holding its evidence", async () => {
    const root = await temporary("rerail-start-packs-");
    const databasePath = join(root, "rerail.sqlite");
    const main = gitRepository(join(root, "acme"));
    const typescriptManifest = JSON.stringify({ devDependencies: { typescript: "^5.9.0" } });
    writeFiles(main, {
      "web/admin/package.json": typescriptManifest,
      "web/site/package.json": typescriptManifest,
    });

    const io = recorder("terminal");
    const report = await startRerail(main, io, { databasePath });
    // TSLint goes where both manifests are.
    expect(report.addedPacks).toEqual([{ packId: "tslint", folder: "web" }]);
    expect(attachedPacks(main, databasePath)).toEqual(["tslint@web"]);
    expect(io.lines).toEqual([[
      `● Rerail is on in acme. ${tslintRuleCount} rules are on.`,
      "  Rule packs added:",
      `  · TSLint in web (${tslintRuleCount} rules): matches the typescript dependency in web/admin/package.json`,
    ].join("\n")]);
  });

  it("sets a repository up with no packs when the person picks rule packs themselves, and does not set it up again", async () => {
    const root = await temporary("rerail-start-pick-");
    const databasePath = join(root, "rerail.sqlite");
    const main = gitRepository(join(root, "acme-web"));
    writeFiles(main, { "src/index.ts": "export {};\n" });

    const io = recorder("terminal");
    expect(await startRerail(main, io, { databasePath, rulePacks: "none" })).toMatchObject({ ruleCount: 0, addedPacks: [] });
    expect(io.lines).toEqual(["● Rerail is on in acme-web. 0 rules are on.\n  No rule packs were added. Pick them on the rules page (rerail ui)."]);
    expect(workspaceStartState(main, databasePath)).toBe("started");

    stopRerail(main, recorder(), { databasePath });
    expect(await startRerail(main, recorder(), { databasePath })).toMatchObject({ ruleCount: 0, addedPacks: [] });
    expect(attachedPacks(main, databasePath)).toEqual([]);
  });

  it("sets a repository up without asking with its recommended packs, starting it only when nobody started or stopped it", async () => {
    const root = await temporary("rerail-start-auto-");
    const databasePath = join(root, "rerail.sqlite");
    const fresh = gitRepository(join(root, "fresh"));
    const stopped = gitRepository(join(root, "stopped"));
    for (const checkout of [fresh, stopped]) writeFiles(checkout, { "web/src/main.tsx": "export {};\n" });
    startTestRepository(stopped, databasePath);
    stopRerail(stopped, recorder(), { databasePath, repository: true });

    const database = openRerailDatabase(databasePath);
    try {
      expect(setUpWithRecommendedPacks(database, recordCheckout(database, fresh))).toEqual([{ packId: "tslint", folder: "web/src" }]);
      expect(setUpWithRecommendedPacks(database, recordCheckout(database, stopped))).toEqual([{ packId: "tslint", folder: "web/src" }]);
    } finally {
      database.$client.close();
    }
    expect(workspaceStartState(fresh, databasePath)).toBe("started");
    expect(workspaceStartState(stopped, databasePath)).toBe("stopped");
    expect(attachedPacks(fresh, databasePath)).toEqual(["tslint@web/src"]);
  });

  it("takes a checkout's own value, then the nearest containing checkout's, then the repository's", async () => {
    const root = await temporary("rerail-worktrees-");
    const databasePath = join(root, "home", "rerail.sqlite");
    const main = gitRepository(join(root, "acme-web"));
    const worktree = (path: string, branch: string): string => {
      git(main, "worktree", "add", "-q", "-b", branch, path);
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

    // Set-up from a worktree starts the repository as a whole.
    const setUp = recorder();
    await startRerail(outer, setUp, { databasePath });
    expect(setUp.lines[0]).toMatch(/^● Rerail is on in acme-web\. /u);
    expect(states()).toEqual({ main: "started", beside: "started", outer: "started", inner: "started" });

    // A checkout stop reaches the worktrees inside it, not the ones beside it.
    stopRerail(outer, recorder(), { databasePath });
    expect(states()).toEqual({ main: "started", beside: "started", outer: "stopped", inner: "stopped" });
    await startRerail(inner, recorder(), { databasePath });
    stopRerail(main, recorder(), { databasePath });
    // The main checkout's stop clears the settings inside it; the sibling worktree keeps following the repository.
    expect(states()).toEqual({ main: "stopped", beside: "started", outer: "stopped", inner: "stopped" });
    await startRerail(inner, recorder(), { databasePath });
    expect(states()).toEqual({ main: "stopped", beside: "started", outer: "stopped", inner: "started" });

    // The daemon resolves the same way over its held connection.
    const database = openRerailDatabase(databasePath);
    try {
      const read = startStateReader(database);
      expect([main, beside, outer, inner].map(read)).toEqual(["stopped", "started", "stopped", "started"]);
    } finally {
      database.$client.close();
    }

    // `--repository` acts on every checkout and clears their own settings.
    stopRerail(inner, recorder(), { databasePath, repository: true });
    expect(states()).toEqual({ main: "stopped", beside: "stopped", outer: "stopped", inner: "stopped" });
    await startRerail(beside, recorder(), { databasePath, repository: true });
    expect(states()).toEqual({ main: "started", beside: "started", outer: "started", inner: "started" });
  });

  it("starts a fresh clone of a started repository by its remote, and records it on the first trusted session", async () => {
    const root = await temporary("rerail-start-clone-");
    const databasePath = join(root, "rerail.sqlite");
    const first = gitRepository(join(root, "first"), "git@github.com:acme/web.git");
    const second = gitRepository(join(root, "second"), "https://github.com/Acme/web");
    const stranger = gitRepository(join(root, "stranger"), "https://github.com/acme/other.git");
    await startRerail(first, recorder(), { databasePath });
    const recordedPaths = (): unknown[] => {
      const database = new Database(databasePath, { readonly: true });
      try {
        return database.prepare("SELECT checkout_path FROM checkouts ORDER BY checkout_path").pluck().all();
      } finally {
        database.close();
      }
    };

    // The passive check finds the clone through its remote without creating its id file or recording it.
    expect(workspaceStartState(second, databasePath)).toBe("started");
    expect(isRerailStarted(second, databasePath)).toBe(true);
    expect(existsSync(repositoryIdentityLocation(second).identityPath)).toBe(false);
    expect(recordedPaths()).toEqual([first]);
    // A fresh clone of a remote Rerail does not know stays stopped.
    expect(workspaceStartState(stranger, databasePath)).toBe("stopped");
    expect(existsSync(repositoryIdentityLocation(stranger).identityPath)).toBe(false);

    // The trusted runtime the hook lets through creates the id and records the clone in the same repository.
    const runtime = new RerailRuntime({
      storageRoot: join(root, "sessions"), databasePath, workspaceRoot: second,
      embedder: testEmbedder, autostart: false, startGate: true,
    });
    try {
      await runtime.ingest({ kind: "session-start", eventId: "start", session: { host: "omp", sessionId: "fresh", cwd: second } });
    } finally {
      await runtime.dispose();
    }
    expect(recordedPaths()).toEqual([first, second]);
    const database = openRerailDatabase(databasePath);
    try {
      const recorded = lookupCheckout(database, second);
      expect(recorded).toMatchObject({ checkoutPath: second, repositoryKey: "remote:github.com/acme/web" });
      expect(recorded?.cloneId).toEqual(expect.any(String));
      expect(recorded?.cloneId).not.toBe(lookupCheckout(database, first)?.cloneId);
    } finally {
      database.$client.close();
    }
    stopRerail(second, recorder(), { databasePath, repository: true });
    expect(workspaceStartState(first, databasePath)).toBe("stopped");
  });

  it("keeps the other checkouts' own settings when the first start sets the repository up", async () => {
    const root = await temporary("rerail-start-setup-overrides-");
    const databasePath = join(root, "rerail.sqlite");
    const main = gitRepository(join(root, "acme"));
    const feature = join(root, "acme-feature");
    const nested = join(feature, "nested");
    git(main, "worktree", "add", "-q", "-b", "feature", feature);
    git(main, "worktree", "add", "-q", "-b", "nested", nested);
    // Started but not set up, with the main checkout and the feature stopped.
    startTestRepository(main, databasePath);
    // Started but not set up: the session-start note says how to set it up rather than counting rules.
    expect(readOnlyStartState(main, databasePath)).toBe("not-set-up");
    expect(sessionNoteRules(main, databasePath)).toEqual({ setUp: false });
    stopRerail(main, recorder(), { databasePath });
    stopRerail(feature, recorder(), { databasePath });
    expect(workspaceStartState(nested, databasePath)).toBe("stopped");

    // Set up from inside the stopped feature worktree: it starts itself, and main keeps its stop.
    await startRerail(nested, recorder(), { databasePath });
    expect([main, feature, nested].map(path => workspaceStartState(path, databasePath))).toEqual(["stopped", "stopped", "started"]);
  });

  it("makes a resident runtime record and steer nothing once the repository is stopped elsewhere", async () => {
    const root = await temporary("rerail-start-resident-");
    const databasePath = join(root, "rerail.sqlite");
    const checkout = gitRepository(join(root, "resident"));
    startTestRepository(checkout, databasePath);
    const runtime = new RerailRuntime({
      storageRoot: join(root, "sessions"), databasePath, workspaceRoot: checkout,
      embedder: testEmbedder, autostart: false, startGate: true,
    });
    const session = { host: "omp" as const, sessionId: "resident", cwd: checkout };
    try {
      await runtime.ingest({ kind: "session-start", eventId: "start", session });
      await runtime.ingest({ kind: "user-prompt", eventId: "before", sessionId: "resident", text: "while started" });
      // Another session, the CLI or the dashboard stops the repository; this runtime stays attached.
      stopTestRepository(checkout, databasePath);
      expect(await runtime.ingest({ kind: "user-prompt", eventId: "after", sessionId: "resident", text: "while stopped" })).toEqual([]);
      startTestRepository(checkout, databasePath);
      await runtime.ingest({ kind: "user-prompt", eventId: "again", sessionId: "resident", text: "started again" });
    } finally {
      await runtime.dispose();
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
    startTestRepository(checkout, databasePath);
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
      stopTestRepository(checkout, databasePath);
      publish.resolve();
      await start;
      expect(await queued).toEqual([]);
    } finally {
      await runtime.dispose();
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
    startTestRepository(checkout, databasePath);
    const runtime = new RerailRuntime({
      storageRoot: join(root, "sessions"), databasePath, workspaceRoot: checkout,
      embedder: testEmbedder, autostart: false, startGate: true,
    });
    const session = { host: "omp" as const, sessionId: "ending", cwd: checkout };
    try {
      await runtime.ingest({ kind: "session-start", eventId: "start", session });
      stopTestRepository(checkout, databasePath);
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
    }
  });

  it("says there is nothing to stop where Rerail never recorded the checkout, naming the command for the lane", async () => {
    const root = await temporary("rerail-stop-never-");
    const checkout = gitRepository(join(root, "landing-page"));
    const io = recorder("terminal");
    expect(stopRerail(checkout, io, { databasePath: join(root, "rerail.sqlite") })).toBe("stopped");
    expect(io.lines).toEqual(["○ Rerail has not been started in landing-page, so there is nothing to stop. Start it with rerail start."]);
  });
});
