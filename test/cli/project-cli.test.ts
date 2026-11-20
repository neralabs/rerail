import { chmodSync, existsSync, mkdirSync, readdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { randomUUID } from "node:crypto";
import { execFileSync, spawnSync } from "node:child_process";
import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import Database from "better-sqlite3";
import { afterEach, describe, expect, it, vi } from "vitest";
import { preUpdateSnapshotPath, runResetCommand, runRulesCommand, type SnapshotUpdater, updateWithSnapshot } from "../../src/cli/project-cli.js";
import type { UpdateResult, UpdateStatus } from "../../src/setup/index.js";
import {
  defaultRerailDatabasePath, loadRuleCatalog, openRerailDatabase, recordCheckout, setUpRepository, STATUS_LINES_FOLDER, upsertLibraryRule,
} from "../../src/engine/index.js";
import { AGENT_SESSION_VARIABLES, agentSession, MOCK_CODE_RULES, runRuleApproveCommand } from "../../src/cli/index.js";
import { CLEARED_AGENT_SESSION_VARIABLES } from "../support/environment/agent-session-variables.js";
import { POSIX_MODES } from "../support/platform/file-mode.js";
import { removeFolder } from "../support/platform/remove-folder.js";

/** Set to make `eraseFreedPages` fail as a busy database or a full disk makes it fail. */
const erase = vi.hoisted(() => ({ failure: undefined as Error | undefined }));
vi.mock("../../src/engine/storage/database.js", async () => {
  const actual = await vi.importActual<typeof import("../../src/engine/storage/database.js")>("../../src/engine/storage/database.js");
  return {
    ...actual,
    eraseFreedPages: (database: Parameters<typeof actual.eraseFreedPages>[0]) => {
      if (erase.failure !== undefined) throw erase.failure;
      return actual.eraseFreedPages(database);
    },
  };
});

const directories: string[] = [];

afterEach(async () => {
  erase.failure = undefined;
  vi.unstubAllEnvs();
  vi.restoreAllMocks();
  await Promise.all(directories.splice(0).map(directory => removeFolder(directory)));
});

describe("rerail rules export", () => {
  it("creates the folder it is told to write into", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-rules-export-"));
    directories.push(root);
    vi.stubEnv("RERAIL_HOME", join(root, "home"));
    vi.spyOn(process.stdout, "write").mockReturnValue(true);
    const output = join(root, "new-dir", "rules.json");

    expect(runRulesCommand(["export", "--output", output], () => "usage")).toBe(0);
    expect(existsSync(output)).toBe(true);
  });
});

describe("rerail rules import", () => {
  it("says what runs in Log after an import, and with --approve approves each imported rule as it is imported", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-rules-import-"));
    directories.push(root);
    vi.stubEnv("RERAIL_HOME", join(root, "home"));
    const rule = MOCK_CODE_RULES[0]!;
    const db = openRerailDatabase(defaultRerailDatabasePath());
    upsertLibraryRule(db, loadRuleCatalog(), rule, "code");
    db.$client.close();
    let output = "";
    const io = { stdout: { write: (text: string) => (output += text) } };
    const dump = join(root, "rules.json");
    expect(runRulesCommand(["export", "--output", dump], () => "usage", io)).toBe(0);

    output = "";
    expect(runRulesCommand(["import", dump], () => "usage", io)).toBe(0);
    expect(output).toContain("1 imported rule runs in Log until a person approves it: rerail rules approve\n");

    output = "";
    expect(runRulesCommand(["import", dump, "--approve"], () => "usage", io)).toBe(0);
    expect(output).toContain(`● Approved ${rule.id}: it runs in the mode it is set to from the next edit.\n`);
    expect(output).not.toContain("runs in Log");
  });
});

async function repository(): Promise<string> {
  const root = await mkdtemp(join(tmpdir(), "rerail-agent-session-"));
  directories.push(root);
  execFileSync("git", ["init", "-q", "-b", "main"], { cwd: root });
  return root;
}

describe("a coding agent's session", () => {
  it("is read from the variable each coding agent sets in its shell", () => {
    expect(agentSession({ CLAUDECODE: "1" })).toEqual({ agent: "Claude Code", variable: "CLAUDECODE" });
    expect(agentSession({ CODEX_THREAD_ID: "019a-thread" })).toEqual({ agent: "Codex", variable: "CODEX_THREAD_ID" });
    expect(agentSession({ CURSOR_AGENT: "1" })).toEqual({ agent: "Cursor", variable: "CURSOR_AGENT" });
    expect(agentSession({ PI_CODING_AGENT: "true", AI_AGENT: "pi" })).toEqual({ agent: "Pi", variable: "PI_CODING_AGENT" });
    expect(agentSession({ AI_AGENT: "some-agent" })).toEqual({ agent: "a coding agent", variable: "AI_AGENT" });
  });

  it("is cleared for every test, so the suite reads as a person's terminal whoever runs it", () => {
    expect(CLEARED_AGENT_SESSION_VARIABLES).toEqual(AGENT_SESSION_VARIABLES);
    expect(agentSession()).toBeUndefined();
  });

  it("is not a person's terminal, where none is set or one is empty", () => {
    expect(agentSession({})).toBeUndefined();
    expect(agentSession({ CLAUDECODE: "", AI_AGENT: "" })).toBeUndefined();
    // The integrated terminal of Cursor's editor sets this for a person too.
    expect(agentSession({ CURSOR_TRACE_ID: "abc" })).toBeUndefined();
  });

  it("is refused rules approve of a named rule or pack, and told where a person approves it, but lists what waits", async () => {
    const repo = await repository();
    // A rule created by you, added here and not approved, so it waits.
    const database = openRerailDatabase(defaultRerailDatabasePath());
    try {
      upsertLibraryRule(database, loadRuleCatalog(), {
        format: "rerail.policy/v1", id: "acme:no-console-log", revision: 1, mode: "steer", title: "No console.log",
        scope: { include: ["src/**/*.ts"], exclude: [] },
        clauses: [{ id: "c", forbid: { regex: { pattern: String.raw`console\.log`, source: "file.content" } }, violation: { anchor: "witness.span", code: "X", message: "Use the logger." } }],
      }, "code");
      setUpRepository(database, recordCheckout(database, repo).repositoryKey, { packs: [], rules: [{ sourceKind: "library_rule", sourceId: "acme:no-console-log", folder: "" }] });
    } finally {
      database.$client.close();
    }
    let listed = "";
    const write = { stdout: { write: (text: string) => (listed += text) } };

    expect(() => runRuleApproveCommand(["acme:no-console-log", "--repo", repo], () => "usage", write, undefined, { CLAUDECODE: "1" })).toThrow(
      "rules approve is a person's, and Claude Code runs this command (CLAUDECODE is set). A person reads and approves acme:no-console-log on its "
        + "page in the dashboard, which rerail manage opens, or with rerail rules approve in a terminal of their own.",
    );
    expect(() => runRuleApproveCommand(["a", "b", "--repo", repo], () => "usage", write, undefined, { AI_AGENT: "some-agent" })).toThrow(
      "rules approve is a person's, and a coding agent runs this command (AI_AGENT is set). A person reads and approves a, b on their page",
    );
    expect(runRuleApproveCommand(["--repo", repo], () => "usage", write, undefined, { CLAUDECODE: "1" })).toBe(0);
    expect(listed).toBe([
      "Waiting for approval; each runs in Log, and steers no one, until a person approves it:",
      "  rule acme:no-console-log  (created by you)",
      "A person reads each and approves it on its page in the dashboard, which rerail manage opens.",
      "",
    ].join("\n"));
  });

  it("is refused rules import, which replaces your rules, approved ones included", () => {
    expect(() => runRulesCommand(["import", "rules.json", "--approve"], () => "usage", {}, { CURSOR_AGENT: "1" })).toThrow(
      "rules import is a person's, and Cursor runs this command (CURSOR_AGENT is set): it replaces your rules, approved ones included. Run it in a terminal of your own.",
    );
  });
});

/** The managed updater, by version: `update` moves to the version it is given, finds nothing newer, or fails. */
class FakeUpdater implements SnapshotUpdater {
  current = "1.0.0";
  previous: string | undefined;
  #status(): UpdateStatus {
    const previous = this.previous === undefined ? {} : { previous: { version: this.previous, packageRoot: `/v/${this.previous}` } };
    return { channel: "latest", current: { version: this.current, packageRoot: `/v/${this.current}` }, ...previous };
  }
  async update(version?: string): Promise<UpdateResult> {
    if (version === "broken") throw new Error("Installed package identity mismatch");
    if (version === undefined || version === this.current) return { ...this.#status(), changed: false };
    this.previous = this.current;
    this.current = version;
    return { ...this.#status(), changed: true };
  }
}

describe("rerail update", () => {
  async function database(): Promise<string> {
    const root = await mkdtemp(join(tmpdir(), "rerail-update-"));
    directories.push(root);
    vi.spyOn(process.stdout, "write").mockReturnValue(true);
    vi.spyOn(process.stderr, "write").mockReturnValue(true);
    const path = join(root, "rerail.sqlite");
    const created = openRerailDatabase(path);
    created.$client.exec("INSERT INTO repositories (repository_key, display_name, created_at, updated_at) VALUES ('before', 'before', 0, 0)");
    created.$client.close();
    return path;
  }

  /** The updated version's first writer: it records a repository. */
  function migrate(path: string): void {
    const migrated = openRerailDatabase(path);
    migrated.$client.exec("INSERT INTO repositories (repository_key, display_name, created_at, updated_at) VALUES ('after', 'after', 0, 0)");
    migrated.$client.close();
  }

  function repositoryKeys(path: string): string[] {
    const opened = new Database(path, { readonly: true });
    try {
      return opened.prepare("SELECT repository_key FROM repositories ORDER BY repository_key").pluck().all() as string[];
    } finally {
      opened.close();
    }
  }

  it("keeps a copy of the database as it was before the version change, readable only by its owner", async () => {
    const path = await database();
    const umask = process.umask(0o022);
    try {
      await expect(updateWithSnapshot(new FakeUpdater(), path, "2.0.0")).resolves.toMatchObject({ changed: true, current: { version: "2.0.0" } });
    } finally {
      process.umask(umask);
    }
    migrate(path);
    expect(repositoryKeys(path)).toEqual(["after", "before"]);
    expect(repositoryKeys(preUpdateSnapshotPath(path))).toEqual(["before"]);
    if (POSIX_MODES) expect(statSync(preUpdateSnapshotPath(path)).mode & 0o777).toBe(0o600);
    expect(vi.mocked(process.stdout.write)).toHaveBeenCalledWith(`Copied ${path} to ${preUpdateSnapshotPath(path)}\n`);
  });

  // An update that finds nothing newer, or fails, must not replace the copy taken before the last version change.
  it.each(["2.0.0", "broken"])("keeps the copy of the last version change through a later update to %s", async channel => {
    const path = await database();
    const updater = new FakeUpdater();
    await updateWithSnapshot(updater, path, "2.0.0");
    migrate(path);
    await updateWithSnapshot(updater, path, channel).catch(() => undefined);
    expect(updater.current).toBe("2.0.0");
    expect(repositoryKeys(preUpdateSnapshotPath(path))).toEqual(["before"]);
    expect(existsSync(`${preUpdateSnapshotPath(path)}.${process.pid}.tmp`)).toBe(false);
  });

  it("copies nothing when there is no database yet", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-update-"));
    directories.push(root);
    vi.spyOn(process.stdout, "write").mockReturnValue(true);
    const path = join(root, "rerail.sqlite");
    await expect(updateWithSnapshot(new FakeUpdater(), path, "2.0.0")).resolves.toMatchObject({ changed: true });
    expect(existsSync(preUpdateSnapshotPath(path))).toBe(false);
  });

  // A version change without a kept copy leaves no copy of an earlier change behind, and failing to keep the copy
  // does not fail the update.
  it("keeps no copy of an earlier change when the copy fails on a version change", async () => {
    const path = await database();
    const updater = new FakeUpdater();
    await updateWithSnapshot(updater, path, "2.0.0");
    expect(existsSync(preUpdateSnapshotPath(path))).toBe(true);
    mkdirSync(`${preUpdateSnapshotPath(path)}.${process.pid}.tmp`);
    await expect(updateWithSnapshot(updater, path, "3.0.0")).resolves.toMatchObject({ changed: true, current: { version: "3.0.0" } });
    expect(existsSync(preUpdateSnapshotPath(path))).toBe(false);
    expect(vi.mocked(process.stderr.write)).toHaveBeenCalledWith(expect.stringContaining(`rerail: could not copy ${path} before the update:`));
  });

  // When neither a new copy nor the old one's removal works (a folder this user cannot write), the update has still
  // happened: it is reported, with what to remove by hand, not failed.
  it("reports an old copy it cannot remove instead of failing the update", async ({ skip }) => {
    skip(process.platform === "win32", "Windows ignores a folder's read-only mode, so no folder here refuses this user's writes");
    const path = await database();
    const updater = new FakeUpdater();
    await updateWithSnapshot(updater, path, "2.0.0");
    chmodSync(dirname(path), 0o500);
    try {
      await expect(updateWithSnapshot(updater, path, "3.0.0")).resolves.toMatchObject({ changed: true, current: { version: "3.0.0" } });
    } finally {
      chmodSync(dirname(path), 0o700);
    }
    const target = preUpdateSnapshotPath(path);
    expect(vi.mocked(process.stderr.write)).toHaveBeenCalledWith(expect.stringContaining(`rerail: ${target} is from an earlier update; remove it yourself:`));
  });
});

/**
 * The bytes of the database and its WAL, read by a child process: closing a descriptor of the file in this process
 * would drop the locks this process's connections hold on it.
 */
function databaseBytes(path: string): string {
  const script = "const fs = require('node:fs'); for (const file of process.argv.slice(1)) if (fs.existsSync(file)) process.stdout.write(fs.readFileSync(file).toString('latin1'));";
  const read = spawnSync(process.execPath, ["-e", script, path, `${path}-wal`], { encoding: "latin1", maxBuffer: 256 * 1024 * 1024 });
  if (read.status !== 0) throw new Error(read.stderr);
  return read.stdout;
}

describe("rerail reset", () => {
  // The daemon holds the database open across a reset, so no closing connection checkpoints for it.
  it("leaves no byte of a deleted event in the database file or its WAL, under a connection that keeps working", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-reset-"));
    directories.push(root);
    vi.stubEnv("RERAIL_HOME", join(root, "home"));
    const checkout = join(root, "repo");
    mkdirSync(checkout);
    vi.spyOn(process, "cwd").mockReturnValue(checkout);
    vi.spyOn(process.stdout, "write").mockReturnValue(true);
    const path = join(root, "home", "rerail.sqlite");
    const secret = `distinctive-${randomUUID()}`;
    const live = openRerailDatabase(path);
    try {
      const { repositoryKey } = recordCheckout(live, checkout);
      const session = live.$client.prepare("INSERT INTO sessions (session_id, host, cwd, repository_id, started_at, ended_at) VALUES (?, 'omp', ?, ?, 1, 2)");
      session.run("s1", checkout, repositoryKey);
      const event = live.$client.prepare("INSERT INTO events (session_id, event_id, kind, observed_at, payload_json) VALUES (?, ?, 'user-prompt', 1, ?)");
      // One copy in the database file, one still in the WAL.
      event.run("s1", "in-file", JSON.stringify({ text: secret }));
      live.$client.pragma("wal_checkpoint(TRUNCATE)");
      event.run("s1", "in-wal", JSON.stringify({ text: secret }));
      expect(databaseBytes(path)).toContain(secret);

      expect(runResetCommand([], () => "usage")).toBe(0);

      expect(databaseBytes(path)).not.toContain(secret);
      expect(live.$client.prepare("SELECT count(*) AS n FROM events").get()).toEqual({ n: 0 });
      session.run("s2", checkout, repositoryKey);
      event.run("s2", "after-reset", "{}");
      expect(live.$client.prepare("SELECT event_id AS id FROM events").all()).toEqual([{ id: "after-reset" }]);
    } finally {
      live.$client.close();
    }
  });

  it("reports a rewrite of the file that fails, keeps the history deleted, and exits 1", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-reset-"));
    directories.push(root);
    vi.stubEnv("RERAIL_HOME", join(root, "home"));
    const checkout = join(root, "repo");
    mkdirSync(checkout);
    vi.spyOn(process, "cwd").mockReturnValue(checkout);
    const output = vi.spyOn(process.stdout, "write").mockReturnValue(true);
    const live = openRerailDatabase(join(root, "home", "rerail.sqlite"));
    try {
      const { repositoryKey } = recordCheckout(live, checkout);
      live.$client.prepare("INSERT INTO sessions (session_id, host, cwd, repository_id, started_at, ended_at) VALUES ('s1', 'omp', ?, ?, 1, 2)")
        .run(checkout, repositoryKey);
      live.$client.prepare("INSERT INTO events (session_id, event_id, kind, observed_at, payload_json) VALUES ('s1', 'e1', 'user-prompt', 1, '{}')").run();
      erase.failure = new Error("database or disk is full");

      expect(runResetCommand([], () => "usage")).toBe(1);

      const printed = output.mock.calls.map(([chunk]) => String(chunk)).join("");
      expect(printed).toContain("database or disk is full");
      expect(live.$client.prepare("SELECT count(*) AS n FROM events").get()).toEqual({ n: 0 });
      erase.failure = undefined;
      expect(runResetCommand([], () => "usage")).toBe(0);
    } finally {
      live.$client.close();
    }
  });

  it("removes the Claude Code status line of each session it deletes, and keeps the lines of the sessions it keeps", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-reset-"));
    directories.push(root);
    vi.stubEnv("RERAIL_HOME", join(root, "home"));
    const checkout = join(root, "repo");
    mkdirSync(checkout);
    vi.spyOn(process, "cwd").mockReturnValue(checkout);
    vi.spyOn(process.stdout, "write").mockReturnValue(true);
    const live = openRerailDatabase(join(root, "home", "rerail.sqlite"));
    try {
      const { repositoryKey } = recordCheckout(live, checkout);
      const session = live.$client.prepare("INSERT INTO sessions (session_id, host, cwd, repository_id, started_at, ended_at) VALUES (?, 'claude-code', ?, ?, 1, ?)");
      session.run("ended-here", checkout, repositoryKey, 2);
      session.run("running-here", checkout, repositoryKey, null);
      session.run("in-another-repository", root, "another-repository", 2);
      const status = join(root, "home", STATUS_LINES_FOLDER);
      mkdirSync(status);
      for (const sessionId of ["ended-here", "running-here", "in-another-repository"]) writeFileSync(join(status, sessionId), "a line\n");
      // A file of the person's own, with a name no session id can have.
      writeFileSync(join(status, "notes.txt"), "mine\n");

      expect(runResetCommand([], () => "usage")).toBe(0);

      expect(readdirSync(status).sort()).toEqual(["in-another-repository", "notes.txt", "running-here"]);
    } finally {
      live.$client.close();
    }
  });
});
