import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, realpathSync, symlinkSync, writeFileSync } from "node:fs";
import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { basename, dirname, join, win32 } from "node:path";
import Database from "better-sqlite3";
import { afterEach, describe, expect, it, vi } from "vitest";
import { createRerailApi, DatabaseApiStores, invokeOperation, operations } from "../../../src/api/index.js";
import { startRerail } from "../../../src/cli/index.js";
import { SqliteAuditControlStore } from "../../../src/engine/audit/audit-control.js";
import { SqliteAuditStore } from "../../../src/engine/audit/audit-store.js";
import { listRepositories, nestedCheckouts, recordCheckout } from "../../../src/engine/catalog/repository-registry.js";
import { setUpRepository } from "../../../src/engine/catalog/rule-configuration.js";
import { openRerailDatabase, type RerailDatabase } from "../../../src/engine/storage/database.js";
import { currentPathKey, pathKeysCanMove, readableRoots, withDriveBudget } from "../../../src/engine/storage/path-key-spelling.js";
import { hookRerailStarted } from "../../../src/harness/index.js";
import {
  setCheckoutStarted, SqlitePresentationPreferenceStore, startStateReader, stopRerail, workspaceStartState, type RepositorySetupIo,
} from "../../../src/runtime/index.js";
import { fakeSetupEnvironment } from "../../support/fake-setup-environment.js";
import { holdEngineEndpoint, testEngineEndpoint } from "../../support/platform/engine-endpoint.js";
import { removeFolder } from "../../support/platform/remove-folder.js";

type PathKeySpelling = typeof import("../../../src/engine/storage/path-key-spelling.js");

// The key a stored path moves to, which each test sets: on Linux and macOS the stored spelling is the one this build
// gives, so a test stands in for Windows with a spelling of its own, and the step runs there as on Windows
// (`pathKeysCanMove`). The Windows tests use the real one. A test stands in for a drive the open does not read through
// `withDriveBudget`, given Windows; no test reaches a real share.
vi.mock("../../../src/engine/storage/path-key-spelling.js", async importOriginal => {
  const original = await importOriginal<PathKeySpelling>();
  return {
    ...original,
    currentPathKey: vi.fn(original.currentPathKey),
    withDriveBudget: vi.fn(original.withDriveBudget),
    pathKeysCanMove: vi.fn(() => true),
  };
});

// Every thread a module of Rerail starts: a thread blocked in a read of a share whose host does not answer is waited for
// at the process's exit, about 21 s on Windows, and nothing can end it sooner.
const threads = vi.hoisted(() => ({ started: 0 }));
vi.mock("node:worker_threads", async importOriginal => {
  const original = await importOriginal<typeof import("node:worker_threads")>();
  class CountedWorker extends original.Worker {
    constructor(...parameters: ConstructorParameters<typeof original.Worker>) {
      super(...parameters);
      threads.started += 1;
    }
  }
  return { ...original, Worker: CountedWorker };
});

const STEP = "path-keys-as-stored";
const temporaryDirectories: string[] = [];
/** What a test started that outlives it unless ended: a daemon's stores and engine endpoint, a `subst` drive letter. */
const ends: Array<() => void | Promise<void>> = [];
afterEach(async () => {
  for (const end of ends.splice(0).reverse()) await end();
  vi.mocked(currentPathKey).mockReset();
  vi.mocked(withDriveBudget).mockReset();
  vi.mocked(pathKeysCanMove).mockReset().mockImplementation(() => true);
  vi.unstubAllEnvs();
  vi.unstubAllGlobals();
  threads.started = 0;
  await Promise.all(temporaryDirectories.splice(0).map(path => removeFolder(path)));
});

/** `withDriveBudget` as on Windows, whatever this system is, so a test reads only the roots an open on Windows reads. */
async function onWindows(): Promise<void> {
  const actual = await vi.importActual<PathKeySpelling>("../../../src/engine/storage/path-key-spelling.js");
  vi.mocked(withDriveBudget).mockImplementation((keyOf, _platform, readable) => actual.withDriveBudget(keyOf, "win32", readable));
}

/**
 * A lookup that holds as long as Windows holds one on a share whose host does not answer, about 21 s, for a key the
 * test says is there: it stands in for that read, and a test that ends at once never started it.
 */
function holdsLikeAGoneShare(onShare: (path: string) => boolean, looked: string[]): (path: string) => string {
  return path => {
    looked.push(path);
    if (onShare(path)) Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 21_000);
    return path.toUpperCase();
  };
}

/**
 * Whether a writer other than the open holds the file's write lock now: a second connection that may not wait tries to
 * take it.
 */
function writeLockHeld(databasePath: string): boolean {
  const client = new Database(databasePath, { timeout: 0 });
  try {
    client.exec("BEGIN IMMEDIATE");
    client.exec("ROLLBACK");
    return false;
  } catch (error) {
    if ((error as { code?: string }).code === "SQLITE_BUSY") return true;
    throw error;
  } finally {
    client.close();
  }
}

/**
 * A checkout in a folder of its own, a Git one unless `git` is false, with the database beside it; `folder` is spelt
 * as Git and Rerail spell it.
 */
async function checkoutFolder({ git = true } = {}): Promise<{ folder: string; databasePath: string }> {
  const root = realpathSync.native(await mkdtemp(join(tmpdir(), "rerail-key-spelling-")));
  temporaryDirectories.push(root);
  const folder = join(root, "app");
  mkdirSync(folder);
  if (git) execFileSync("git", ["init", "-q", "-b", "main"], { cwd: folder, stdio: "ignore" });
  return { folder, databasePath: join(root, "rerail.sqlite") };
}

/**
 * Makes `folder`'s drive this computer's system drive, the one drive a writable open that names no folder reads, as it
 * is for a checkout under the person's profile. A test runner can keep its temporary folder on another (GitHub's Windows
 * runners keep it on `D:`).
 */
function onTheSystemDrive(folder: string): void {
  vi.stubEnv("SystemDrive", win32.parse(folder).root.slice(0, 2));
}

/** Holds the file's write lock on a connection of its own, as another writer does, until the returned end runs. */
function holdWriteLock(databasePath: string): () => void {
  const writer = new Database(databasePath);
  writer.exec("BEGIN IMMEDIATE");
  const end = (): void => {
    if (!writer.open) return;
    writer.exec("ROLLBACK");
    writer.close();
  };
  ends.push(end);
  return end;
}

/** `folder` spelt another way that names the same folder on Windows: its last name in capitals. */
function otherSpelling(folder: string): string {
  return join(dirname(folder), basename(folder).toUpperCase());
}

function rows(database: RerailDatabase, query: string, ...parameters: string[]): unknown[] {
  return database.$client.prepare(query).all(...parameters);
}

/**
 * Each statement run with its own parameters, paths as parameters rather than text, on a plain connection that runs no
 * migration step: the file stays as the earlier build left it.
 */
function write(databasePath: string, statements: ReadonlyArray<readonly [string, ...string[]]>): void {
  const client = new Database(databasePath);
  try {
    for (const [statement, ...parameters] of statements) client.prepare(statement).run(...parameters);
  } finally {
    client.close();
  }
}

/** An audit that ran in `root`, as the audit history keeps it. */
function auditRow(auditId: string, root: string): readonly [string, ...string[]] {
  return [`INSERT INTO audits (audit_id, session_id, workspace_root, scope, mode, revision, rule_provenance_json, paths_json,
    started_at, finished_at, status, violations_json, failures_json)
    VALUES (?, 'earlier', ?, 'repository', 'log', 0, '[]', '[]', 1, 2, 'completed', '[]', '[]')`, auditId, root];
}

/**
 * What an earlier build left for a checkout it recorded as `stored`: the checkout started on its own (its repository
 * is set up but not started), a rule turned off there, its audit settings, a presentation preference, and its
 * repository audit with the audit and the audit request it came from. Written by this build under `folder`, then moved
 * to `stored`, with the step not yet run.
 */
function earlierBuildRecorded(folder: string, databasePath: string, stored: string): void {
  const database = openRerailDatabase(databasePath);
  try {
    const checkout = recordCheckout(database, folder);
    setUpRepository(database, checkout.repositoryKey, { packs: [] });
    setCheckoutStarted(database, checkout, true);
    database.$client.prepare(`INSERT INTO rule_settings (owner_kind, owner_id, folder, target_kind, target_id, "on", updated_at)
      VALUES ('checkout', ?, '', 'rule', 'no-console', 0, 1), ('repository', ?, '', 'rule', 'no-console', 1, 1)`)
      .run(folder, checkout.repositoryKey);
    database.$client.prepare(`INSERT INTO repository_audits (workspace_root, session_id, audit_id, status, started_at, finished_at)
      VALUES (?, 'earlier', 'audit-1', 'completed', 1, 2)`).run(folder);
    const [statement, ...parameters] = auditRow("audit-1", folder);
    database.$client.prepare(statement).run(...parameters);
    database.$client.prepare(`INSERT INTO audit_requests (id, session_id, workspace_root, scope, mode, paths_json, status,
      created_at, started_at, finished_at, audit_ids_json) VALUES ('request-1', 'earlier', ?, 'repository', 'log', '[]',
      'completed', 1, 1, 2, '["audit-1"]')`).run(folder);
  } finally {
    database.$client.close();
  }
  const controls = new SqliteAuditControlStore(databasePath);
  controls.updateWorkspaceSettings(folder, { mode: "log" });
  controls.close();
  const preferences = new SqlitePresentationPreferenceStore(databasePath);
  preferences.upsert({ repository: folder, host: "claude-code", presence: "silent" });
  preferences.upsert({ repository: "", host: "", presence: "compact" });
  preferences.close();
  write(databasePath, [
    ["UPDATE checkouts SET checkout_path = ?", stored],
    ["UPDATE rule_settings SET owner_id = ? WHERE owner_kind = 'checkout'", stored],
    ["UPDATE audit_settings SET workspace_root = ?", stored],
    ["UPDATE repository_audits SET workspace_root = ?", stored],
    ["UPDATE audits SET workspace_root = ?", stored],
    ["UPDATE audit_requests SET workspace_root = ?", stored],
    ["UPDATE presentation_preferences SET repository = ? WHERE repository <> ''", stored],
    ["DELETE FROM schema_migrations WHERE id = ?", STEP],
    ["DROP TABLE IF EXISTS path_key_spellings"],
  ]);
}

/** Each record a checkout's earlier build left, as this build reads it for `folder`. */
function recordsAt(folder: string, databasePath: string) {
  // The hook's own check, on a read-only open: the first open after the update runs the step before it reads.
  const startState = workspaceStartState(folder, databasePath);
  const database = openRerailDatabase(databasePath);
  const controls = new SqliteAuditControlStore(databasePath);
  const preferences = new SqlitePresentationPreferenceStore(databasePath);
  const audits = new SqliteAuditStore(databasePath);
  try {
    return {
      startState,
      checkouts: listRepositories(database).flatMap(repository => repository.checkouts.map(row => row.checkoutPath)),
      settings: rows(database, `SELECT "on" FROM rule_settings WHERE owner_kind = 'checkout' AND owner_id = ?`, folder),
      auditSettings: controls.getWorkspaceSettings(folder),
      presence: preferences.effective(folder, "claude-code"),
      preferences: preferences.list().map(({ repository, host, presence }) => ({ repository, host, presence })),
      // The list of repository audits the dashboard and `repository_audits_list` show, one entry per folder.
      repositoryAuditList: audits.listRepositoryAudits().repositories.map(({ workspaceRoot, repositoryAudit, audit }) =>
        ({ workspaceRoot, status: repositoryAudit?.status, auditId: audit?.auditId })),
    };
  } finally {
    audits.close();
    preferences.close();
    controls.close();
    database.$client.close();
  }
}

type Records = ReturnType<typeof recordsAt>;

/** What a checkout the earlier build recorded as `stored` keeps under `folder`, its key in this build: every record. */
function kept(folder: string, repositoryAudit: Omit<Records["repositoryAuditList"][number], "workspaceRoot">): Records {
  return {
    startState: "started",
    checkouts: [folder],
    settings: [{ on: 0 }],
    auditSettings: { scope: "ai-output", mode: "log", steerStrategy: "message" },
    presence: "silent",
    preferences: [
      { repository: "", host: "", presence: "compact" },
      { repository: folder, host: "claude-code", presence: "silent" },
    ],
    repositoryAuditList: [{ workspaceRoot: folder, ...repositoryAudit }],
  };
}

/**
 * What an earlier build reads for its checkout `stored`, by the key it looks each record up by: the spelling it stored.
 * 0.1.4's stores look a checkout, its settings, its audit settings and audit, and its preferences up so.
 */
function earlierBuildReads(databasePath: string, stored: string) {
  const client = new Database(databasePath, { readonly: true });
  try {
    const one = (query: string): unknown => client.prepare(query).get(stored);
    return {
      started: one("SELECT started FROM checkouts WHERE checkout_path = ?"),
      setting: one("SELECT \"on\" FROM rule_settings WHERE owner_kind = 'checkout' AND owner_id = ?"),
      auditSettings: one("SELECT json_extract(settings_json, '$.mode') AS mode FROM audit_settings WHERE workspace_root = ?"),
      repositoryAudit: one("SELECT status FROM repository_audits WHERE workspace_root = ?"),
      presence: one("SELECT presence FROM presentation_preferences WHERE repository = ? AND host = 'claude-code'"),
      // The audit history is never rewritten.
      audits: client.prepare("SELECT audit_id FROM audits WHERE workspace_root = ? ORDER BY audit_id").all(stored),
    };
  } finally {
    client.close();
  }
}

/** All the earlier build recorded for its checkout, as `earlierBuildRecorded` wrote it. */
const EARLIER_BUILD_READS = {
  started: { started: 1 },
  setting: { on: 0 },
  auditSettings: { mode: "log" },
  repositoryAudit: { status: "completed" },
  presence: { presence: "silent" },
  audits: [{ audit_id: "audit-1" }],
};

/**
 * The daemon a coding agent's hooks find while it runs, over `databasePath`: its stores, made as the daemon makes them
 * as it starts (a writable open that names no folder), and its API, reached through the state file in a Rerail home as
 * a hook reaches it. A hook asks it first, and reads the database itself only when none answers.
 */
async function daemonRunning(databasePath: string): Promise<void> {
  const home = join(dirname(databasePath), "home");
  mkdirSync(home);
  const stores = new DatabaseApiStores(databasePath, {
    health: () => ({ pid: process.pid, database: { path: databasePath, exists: true }, native: { ok: true } }),
    origin: () => "http://127.0.0.1:4737",
  });
  // A running daemon has opened its database by the time a hook asks it anything.
  stores.database();
  const api = createRerailApi(stores, operations, { version: "0.0.0-test" });
  // On Windows a hook counts a recorded daemon alive only while its engine pipe exists.
  const kernelSocketPath = testEngineEndpoint(home);
  const release = await holdEngineEndpoint(kernelSocketPath);
  ends.push(async () => {
    stores.close();
    await release();
  });
  writeFileSync(join(home, "daemon.json"), JSON.stringify({ pid: process.pid, host: "127.0.0.1", port: 4737,
    url: "http://127.0.0.1:4737/", version: "test", databasePath, kernelSocketPath, ui: "bundled", startedAt: 1, packageRoot: home }));
  vi.stubEnv("RERAIL_HOME", home);
  vi.stubGlobal("fetch", (input: URL | string, init?: RequestInit) => api.request(String(input), init));
}

/**
 * What this build reads for `folder`, spelt `spelled` by its caller, by readers that look nothing up: its own rule
 * setting under its key, its audit settings and its presentation in Claude Code.
 */
function heldFor(databasePath: string, folder: string, spelled = folder) {
  const controls = new SqliteAuditControlStore(databasePath);
  const preferences = new SqlitePresentationPreferenceStore(databasePath);
  const client = new Database(databasePath, { readonly: true });
  try {
    return {
      setting: client.prepare(`SELECT "on" FROM rule_settings WHERE owner_kind = 'checkout' AND owner_id = ?`).get(folder),
      auditSettings: controls.getWorkspaceSettings(spelled),
      presence: preferences.effective(spelled, "claude-code"),
    };
  } finally {
    client.close();
    preferences.close();
    controls.close();
  }
}

/** The settings `earlierBuildRecorded` left for its checkout, as `heldFor` reads them once they are copied. */
const HELD = { setting: { on: 0 }, auditSettings: { scope: "ai-output", mode: "log", steerStrategy: "message" }, presence: "silent" };

/** A terminal `rerail start` or `rerail stop` runs in, keeping what it says. */
function terminal(): RepositorySetupIo & { said: string[] } {
  const said: string[] = [];
  return { lane: "terminal", said, say: text => { said.push(text); } };
}

function spellings(databasePath: string): unknown[] {
  const client = new Database(databasePath, { readonly: true });
  try {
    return client.prepare("SELECT stored, current FROM path_key_spellings ORDER BY stored").all();
  } finally {
    client.close();
  }
}

describe("path keys an earlier build stored under another spelling", () => {
  it("keep a checkout recording, with its start state, settings, audit and presentation, once they are copied", async () => {
    const { folder, databasePath } = await checkoutFolder();
    const stored = otherSpelling(folder);
    earlierBuildRecorded(folder, databasePath, stored);
    vi.mocked(currentPathKey).mockImplementation(path => (path === stored ? folder : path));

    expect(recordsAt(folder, databasePath)).toEqual(kept(folder, { status: "completed", auditId: "audit-1" }));
    expect(spellings(databasePath)).toEqual([{ stored, current: folder }]);
  });

  // The step moved the rows to the new spelling, so an earlier build sharing the file (after a downgrade, or an older
  // install's hooks) read the folder stopped, the default presentation and no repository audit, and ran it again.
  it("leave every record where an earlier build sharing the file looks it up", async () => {
    const { folder, databasePath } = await checkoutFolder();
    const stored = otherSpelling(folder);
    earlierBuildRecorded(folder, databasePath, stored);
    vi.mocked(currentPathKey).mockImplementation(path => (path === stored ? folder : path));
    openRerailDatabase(databasePath).$client.close();

    expect(earlierBuildReads(databasePath, stored)).toEqual(EARLIER_BUILD_READS);
  });

  it("merge with rows this build already wrote for the folder: the newer row stays, and the start state is kept", async () => {
    const { folder, databasePath } = await checkoutFolder();
    const stored = otherSpelling(folder);
    earlierBuildRecorded(folder, databasePath, stored);
    // A build without the step opened a session in the folder: a row of its own, seen later and following its
    // repository, an older setting of the same rule, a newer audit, and audit settings and a preference saved before
    // the earlier build's.
    write(databasePath, [
      [`INSERT INTO checkouts (checkout_path, repository_key, clone_id, started, created_at, last_seen_at)
        SELECT ?, repository_key, clone_id, NULL, created_at, last_seen_at + 1 FROM checkouts`, folder],
      [`INSERT INTO rule_settings (owner_kind, owner_id, folder, target_kind, target_id, "on", updated_at)
        VALUES ('checkout', ?, '', 'rule', 'no-console', 1, 0)`, folder],
      [`INSERT INTO repository_audits (workspace_root, session_id, audit_id, status, started_at, finished_at)
        VALUES (?, 'later', 'audit-2', 'interrupted', 3, 4)`, folder],
      auditRow("audit-2", folder),
      [`INSERT INTO audit_settings (workspace_root, settings_json, updated_at)
        VALUES (?, '{"scope":"repository","mode":"steer"}', 0)`, folder],
      ["DELETE FROM presentation_preferences WHERE repository = ?", stored],
      ["INSERT INTO presentation_preferences (repository, host, presence) VALUES (?, 'claude-code', 'compact')", folder],
      ["INSERT INTO presentation_preferences (repository, host, presence) VALUES (?, 'claude-code', 'silent')", stored],
    ]);
    vi.mocked(currentPathKey).mockImplementation(path => (path === stored ? folder : path));

    expect(recordsAt(folder, databasePath)).toEqual(kept(folder, { status: "interrupted", auditId: "audit-2" }));
    expect(earlierBuildReads(databasePath, stored)).toEqual(EARLIER_BUILD_READS);
  });

  it("list a folder once where only its audit history holds the earlier spelling, and leave that history as it was", async () => {
    const { folder, databasePath } = await checkoutFolder();
    const stored = otherSpelling(folder);
    earlierBuildRecorded(folder, databasePath, stored);
    // The earlier build keyed the checkout and its repository audit by Git's spelling and its audit history by the
    // spelling the session gave, and saved no audit settings or preference there: no key holds `stored`.
    write(databasePath, [
      ["UPDATE checkouts SET checkout_path = ?", folder],
      ["UPDATE rule_settings SET owner_id = ? WHERE owner_kind = 'checkout'", folder],
      ["UPDATE repository_audits SET workspace_root = ?", folder],
      ["DELETE FROM audit_settings WHERE workspace_root = ?", stored],
      ["DELETE FROM presentation_preferences WHERE repository = ?", stored],
    ]);
    vi.mocked(currentPathKey).mockImplementation(path => (path === stored ? folder : path));

    expect(recordsAt(folder, databasePath)).toMatchObject({
      repositoryAuditList: [{ workspaceRoot: folder, status: "completed", auditId: "audit-1" }],
    });
    expect(earlierBuildReads(databasePath, stored)).toMatchObject({ audits: [{ audit_id: "audit-1" }] });
  });

  it("leave out of this build's lists the copies kept for an earlier build, and remove a preference's copies with it", async () => {
    const { folder, databasePath } = await checkoutFolder();
    onTheSystemDrive(folder);
    const stored = otherSpelling(folder);
    earlierBuildRecorded(folder, databasePath, stored);
    // A checkout of the same repository inside the folder, which the earlier build recorded under its own spelling.
    const [inner, storedInner] = [join(folder, "inner"), join(folder, "INNER")];
    write(databasePath, [[`INSERT INTO checkouts (checkout_path, repository_key, clone_id, started, created_at, last_seen_at)
      SELECT ?, repository_key, clone_id, NULL, created_at, last_seen_at FROM checkouts`, storedInner]]);
    vi.mocked(currentPathKey).mockImplementation(path => ({ [stored]: folder, [storedInner]: inner })[path] ?? path);

    const database = openRerailDatabase(databasePath);
    try {
      const { repositoryKey } = listRepositories(database).find(repository => repository.checkouts.length > 0)!;
      expect(nestedCheckouts(database, { checkoutPath: folder, repositoryKey }).map(row => row.checkoutPath)).toEqual([inner]);
    } finally {
      database.$client.close();
    }
    const preferences = new SqlitePresentationPreferenceStore(databasePath);
    try {
      expect(preferences.remove(folder, "claude-code")).toBe(1);
    } finally {
      preferences.close();
    }
    expect(earlierBuildReads(databasePath, stored).presence).toBeUndefined();
  });

  it("stay where the key cannot be resolved or is no whole path, and the database still opens", async () => {
    const { folder, databasePath } = await checkoutFolder();
    onTheSystemDrive(folder);
    const stored = otherSpelling(folder);
    const relative = join("relative", "checkout");
    earlierBuildRecorded(folder, databasePath, stored);
    write(databasePath, [[`INSERT INTO rule_settings (owner_kind, owner_id, folder, target_kind, target_id, "on", updated_at)
      VALUES ('checkout', ?, '', 'rule', 'no-console', 0, 1)`, relative]]);
    vi.mocked(currentPathKey).mockImplementation(path => {
      if (path === stored) throw Object.assign(new Error("EPERM: operation not permitted"), { code: "EPERM" });
      return `${path}-moved`;
    });

    const database = openRerailDatabase(databasePath);
    try {
      expect(rows(database, "SELECT checkout_path FROM checkouts")).toEqual([{ checkout_path: stored }]);
      expect(rows(database, "SELECT owner_id FROM rule_settings WHERE owner_kind = 'checkout' ORDER BY owner_id"))
        .toEqual([stored, relative].sort().map(owner => ({ owner_id: owner })));
      expect(rows(database, "SELECT id FROM schema_migrations WHERE id = 'path-keys-as-stored'")).toEqual([{ id: STEP }]);
      expect(rows(database, "SELECT stored FROM path_key_spellings")).toEqual([]);
      expect(vi.mocked(currentPathKey).mock.calls.map(([path]) => path)).toEqual([stored]);
    } finally {
      database.$client.close();
    }
  });

  // The step ran once per file: a folder on a disk not attached at the first open kept its earlier spelling for good,
  // and this build read its start state, settings, audit and preference as unset once the disk was back.
  it("copy the rows of a folder on a drive the step did not read once Rerail checks its start state there", async () => {
    await onWindows();
    const { folder, databasePath } = await checkoutFolder();
    // Another drive than the one the test's folder is on: on Linux, and on Windows, whose temporary folder is on `C:`.
    vi.stubEnv("SystemDrive", "Q:");
    const stored = otherSpelling(folder);
    earlierBuildRecorded(folder, databasePath, stored);
    vi.mocked(currentPathKey).mockImplementation(path => (path === stored ? folder : path));
    // The daemon's start opens the file for writing: it reads no drive but `Q:`.
    openRerailDatabase(databasePath).$client.close();
    expect(spellings(databasePath)).toEqual([{ stored, current: null }]);
    expect(vi.mocked(currentPathKey)).not.toHaveBeenCalled();

    // A coding agent's hook checks whether Rerail runs in the folder: the drive answers, and the folder keeps every record.
    expect(recordsAt(folder, databasePath)).toEqual(kept(folder, { status: "completed", auditId: "audit-1" }));
    expect(spellings(databasePath)).toEqual([{ stored, current: folder }]);
    expect(earlierBuildReads(databasePath, stored)).toEqual(EARLIER_BUILD_READS);
  });

  // Round 1 left every key on a drive mapped to a share as it was, so a folder an earlier build keyed `Q:\proj` read as
  // unset where this build looks it up by the share's spelling. The keys on the drives an open reads are looked up
  // within a second each, and the rest wait without a read.
  it("look keys up on the drives an open reads only, within a time per drive, and leave the rest waiting unread", () => {
    const looked: string[] = [];
    let clock = 0;
    // `C:` is this computer's system drive, `z:` the drive of the folder the caller works in, mapped to a share. Each
    // lookup on it takes 400 ms. `Y:`, `E:` and `\\host\share` are drives and a share no caller works on.
    const keyOf = withDriveBudget(stored => {
      looked.push(stored);
      if (stored.startsWith("Z:")) clock += 400;
      // Through the mapping, a key on `Z:` reads as the share spells it.
      return stored.startsWith("Z:") ? `\\\\server\\share${stored.slice(2).toUpperCase()}` : stored.toUpperCase();
    }, "win32", { roots: ["C:\\", "z:\\"], now: () => clock });

    expect(["C:\\a", "c:\\b", "Z:\\c", "Z:\\d", "Z:\\e", "Z:\\f", "Y:\\g", "E:\\h", "\\\\host\\share\\i"].map(keyOf)).toEqual([
      "C:\\A", "C:\\B", "\\\\server\\share\\C", "\\\\server\\share\\D", "\\\\server\\share\\E", undefined, undefined, undefined, undefined,
    ]);
    expect(looked).toEqual(["C:\\a", "c:\\b", "Z:\\c", "Z:\\d", "Z:\\e"]);
    expect(withDriveBudget(stored => stored.toUpperCase(), "linux", { roots: [], now: () => clock })("/a")).toBe("/A");
    expect(readableRoots(undefined, { SystemDrive: "D:" }).roots).toEqual(["D:\\"]);
    expect(readableRoots("\\\\host\\share\\proj", {}).roots).toEqual(["C:\\", "\\\\host\\share\\"]);
  });

  // Each drive's root was read in a thread of its own, waiting a second at most. A share whose host does not answer held
  // that read about 21 s, the thread could not be ended, and the process waited for it at its exit: on a Windows 11 VM a
  // writing open with one such key waiting ended its process 21.1 s after the read began, every time.
  it("start no thread and read no drive no caller works on at a writable open, so no exit waits for a share", async () => {
    await onWindows();
    // The test's folder stands for a share no caller works on: `Q:` is the system drive.
    vi.stubEnv("SystemDrive", "Q:");
    const looked: string[] = [];
    vi.mocked(currentPathKey).mockImplementation(holdsLikeAGoneShare(() => true, looked));
    const { folder, databasePath } = await checkoutFolder();
    const stored = otherSpelling(folder);
    earlierBuildRecorded(folder, databasePath, stored);
    const started = performance.now();
    // The first writable open runs the step; the next one looks up what waits.
    openRerailDatabase(databasePath).$client.close();
    openRerailDatabase(databasePath).$client.close();

    expect({ threads: threads.started, looked }).toEqual({ threads: 0, looked: [] });
    expect(performance.now() - started).toBeLessThan(5_000);
    expect(spellings(databasePath)).toEqual([{ stored, current: null }]);
  }, 60_000);

  // Every writable open looked a key waiting on a drive not attached up again, a thread each time, with no end: a disk
  // that held a repository once and is never attached again.
  it("look a key waiting on another drive up once Rerail runs in a folder there, and at no other writable open", async () => {
    await onWindows();
    vi.stubEnv("SystemDrive", "Q:");
    const { folder, databasePath } = await checkoutFolder();
    const stored = otherSpelling(folder);
    earlierBuildRecorded(folder, databasePath, stored);
    const looked: string[] = [];
    vi.mocked(currentPathKey).mockImplementation(path => {
      looked.push(path);
      return path === stored ? folder : path;
    });
    for (let open = 0; open < 100; open += 1) openRerailDatabase(databasePath).$client.close();
    expect({ looked, threads: threads.started }).toEqual({ looked: [], threads: 0 });

    // A coding agent's hook checks each event's folder: the first check looks the key up, and no later one does.
    for (let check = 0; check < 100; check += 1) expect(workspaceStartState(folder, databasePath)).toBe("started");
    expect({ looked, threads: threads.started }).toEqual({ looked: [stored], threads: 0 });
    expect(spellings(databasePath)).toEqual([{ stored, current: folder }]);
  }, 60_000);

  // While a daemon ran, every Claude Code, Codex, Cursor and OMP hook took its start state from the daemon, whose opens
  // read no drive but the system drive and which made no start-state check of its own: a folder an earlier build keyed
  // on another disk read stopped, with its settings unset, until a command such as `rerail status` ran there.
  it("copy the rows of a folder on another drive at the first hook a running daemon answers there, and look up nothing after", async () => {
    await onWindows();
    vi.stubEnv("SystemDrive", "Q:");
    const { folder, databasePath } = await checkoutFolder();
    const stored = otherSpelling(folder);
    earlierBuildRecorded(folder, databasePath, stored);
    const looked: string[] = [];
    vi.mocked(currentPathKey).mockImplementation(path => {
      looked.push(path);
      return path === stored ? folder : path;
    });
    await daemonRunning(databasePath);
    expect(spellings(databasePath)).toEqual([{ stored, current: null }]);

    expect(await hookRerailStarted("claude-code", folder)).toBe(true);
    expect(heldFor(databasePath, folder)).toEqual(HELD);
    for (let event = 1; event < 100; event += 1) expect(await hookRerailStarted("claude-code", folder)).toBe(true);
    expect({ looked, threads: threads.started }).toEqual({ looked: [stored], threads: 0 });
    expect(earlierBuildReads(databasePath, stored)).toEqual(EARLIER_BUILD_READS);
  }, 60_000);

  // A hook named its folder to the daemon by its realpath, which for a drive letter made with `subst` or mapped to a
  // share names the folder or share the letter stands for: a key an earlier build stored through the letter waited on
  // the letter's drive, which no request named, and the folder read stopped at every event.
  it.runIf(process.platform === "win32")("copy, on Windows, a key stored through a drive letter that stands for another folder, at the first hook there", async () => {
    const actual = await vi.importActual<PathKeySpelling>("../../../src/engine/storage/path-key-spelling.js");
    vi.mocked(currentPathKey).mockImplementation(actual.currentPathKey);
    vi.mocked(withDriveBudget).mockImplementation(actual.withDriveBudget);
    const { folder, databasePath } = await checkoutFolder();
    const letter = [..."GHIJKLMNOPQRSTUVWXYZ"].map(name => `${name}:`).find(drive => !existsSync(`${drive}\\`))!;
    execFileSync("subst", [letter, dirname(folder)], { stdio: "ignore" });
    ends.push(() => { execFileSync("subst", [letter, "/d"], { stdio: "ignore" }); });
    const throughLetter = `${letter}\\${basename(folder)}`;
    expect(actual.currentPathKey(throughLetter)).toBe(folder);
    // As VS Code passes a folder: the drive letter in lower case.
    const stored = throughLetter.toLowerCase();
    earlierBuildRecorded(folder, databasePath, stored);
    await daemonRunning(databasePath);
    expect(spellings(databasePath)).toEqual([{ stored, current: null }]);

    expect(await hookRerailStarted("claude-code", throughLetter)).toBe(true);
    expect(heldFor(databasePath, folder, throughLetter)).toEqual(HELD);
    expect(spellings(databasePath)).toEqual([{ stored, current: folder }]);
  }, 60_000);

  // `rerail start` in a repository set up already read whether it was set up, and recorded the checkout, through opens
  // that read only the system drive: it said Rerail was off there before, and counted rules the person had turned off.
  it("copy the rows of a folder on another drive when `rerail start` runs there in a repository set up already", async () => {
    await onWindows();
    vi.stubEnv("SystemDrive", "Q:");
    const { folder, databasePath } = await checkoutFolder();
    const stored = otherSpelling(folder);
    earlierBuildRecorded(folder, databasePath, stored);
    vi.mocked(currentPathKey).mockImplementation(path => (path === stored ? folder : path));
    openRerailDatabase(databasePath).$client.close();

    const io = terminal();
    expect(await startRerail(folder, io, { databasePath, environment: fakeSetupEnvironment() }))
      .toMatchObject({ previous: "started", setUpRan: false, status: "started" });
    expect(io.said).toEqual([expect.stringMatching(/^● Rerail is already on in /u)]);
    expect(heldFor(databasePath, folder)).toEqual(HELD);
  }, 60_000);

  // `rerail stop` there said Rerail had been off already, while the copy a later check made read it started.
  it("copy the rows of a folder on another drive when `rerail stop` runs there, so it stops what the earlier build started", async () => {
    await onWindows();
    vi.stubEnv("SystemDrive", "Q:");
    const { folder, databasePath } = await checkoutFolder();
    const stored = otherSpelling(folder);
    earlierBuildRecorded(folder, databasePath, stored);
    vi.mocked(currentPathKey).mockImplementation(path => (path === stored ? folder : path));
    openRerailDatabase(databasePath).$client.close();

    expect(stopRerail(folder, terminal(), { databasePath })).toBe("started");
    expect(workspaceStartState(folder, databasePath)).toBe("stopped");
    expect(heldFor(databasePath, folder)).toEqual(HELD);
  }, 60_000);

  it.runIf(process.platform === "win32")("look, on Windows, a key on a drive not attached up at no writable open", async () => {
    const actual = await vi.importActual<PathKeySpelling>("../../../src/engine/storage/path-key-spelling.js");
    vi.mocked(currentPathKey).mockImplementation(actual.currentPathKey);
    const absent = [..."ZYXWVUTSRQPONMLKJIHGF"].map(letter => `${letter}:`).find(drive => !existsSync(`${drive}\\`));
    const stored = `${absent}\\work\\app`;
    const { databasePath } = await checkoutFolder();
    openRerailDatabase(databasePath).$client.close();
    write(databasePath, [
      [`INSERT INTO audit_settings (workspace_root, settings_json, updated_at) VALUES (?, '{"scope":"repository","mode":"log"}', 1)`, stored],
      ["DELETE FROM schema_migrations WHERE id = ?", STEP],
      ["DROP TABLE IF EXISTS path_key_spellings"],
    ]);
    for (let open = 0; open < 100; open += 1) openRerailDatabase(databasePath).$client.close();

    expect({ lookups: vi.mocked(currentPathKey).mock.calls.length, threads: threads.started }).toEqual({ lookups: 0, threads: 0 });
    expect(spellings(databasePath)).toEqual([{ stored, current: null }]);
  }, 60_000);

  // The step looked every key up inside the open's write lock, and read the whole audit history there: with the file
  // out of the page cache that took seconds (7.1 s over 20,000 audits), and a slow drive as long, while every other
  // writer waited. The keys a later open looks up again waited the same way.
  it("look every key up before the open takes the write lock, at the first open and at a later one", async () => {
    const { folder, databasePath } = await checkoutFolder();
    const stored = otherSpelling(folder);
    earlierBuildRecorded(folder, databasePath, stored);
    // A second folder, looked up at the first open; the first one's drive is absent then, so a later open looks it up.
    const other = join(dirname(folder), "other");
    write(databasePath, [["INSERT INTO audit_settings (workspace_root, settings_json, updated_at) VALUES (?, '{\"scope\":\"repository\",\"mode\":\"log\"}', 1)", other]]);
    const lookups: { path: string; lockHeld: boolean }[] = [];
    let attached = false;
    vi.mocked(currentPathKey).mockImplementation(path => {
      lookups.push({ path, lockHeld: writeLockHeld(databasePath) });
      return path === stored ? folder : path;
    });
    vi.mocked(withDriveBudget).mockImplementation(keyOf => path => (path === stored && !attached ? undefined : keyOf(path)));
    openRerailDatabase(databasePath).$client.close();
    attached = true;
    openRerailDatabase(databasePath).$client.close();

    expect(lookups).toEqual([{ path: other, lockHeld: false }, { path: stored, lockHeld: false }]);
    expect(recordsAt(folder, databasePath)).toEqual(kept(folder, { status: "completed", auditId: "audit-1" }));
  });

  it("read and look up no key outside Windows, where none can move", async () => {
    const actual = await vi.importActual<PathKeySpelling>("../../../src/engine/storage/path-key-spelling.js");
    const { folder, databasePath } = await checkoutFolder();
    const stored = otherSpelling(folder);
    earlierBuildRecorded(folder, databasePath, stored);
    // As `pathKeysCanMove` answers on macOS and Linux.
    vi.mocked(pathKeysCanMove).mockImplementation(() => false);
    openRerailDatabase(databasePath).$client.close();
    openRerailDatabase(databasePath).$client.close();

    expect(vi.mocked(currentPathKey)).not.toHaveBeenCalled();
    expect(spellings(databasePath)).toEqual([]);
    expect(earlierBuildReads(databasePath, stored)).toEqual(EARLIER_BUILD_READS);
    expect(["linux", "darwin", "win32"].map(platform => actual.pathKeysCanMove(platform as NodeJS.Platform))).toEqual([false, false, true]);
  });

  it.runIf(process.platform === "win32")("copy, on Windows, from a lower-case drive letter and another case to the stored spelling", async () => {
    const actual = await vi.importActual<PathKeySpelling>("../../../src/engine/storage/path-key-spelling.js");
    vi.mocked(currentPathKey).mockImplementation(actual.currentPathKey);
    // A folder outside Git, whose row an earlier version keyed by the folder as the caller spelt it.
    const { folder, databasePath } = await checkoutFolder({ git: false });
    // As VS Code passes a folder: `c:\...`, here with the folder's own name in capitals too.
    const stored = otherSpelling(folder).replace(/^[A-Z]:/u, drive => drive.toLowerCase());
    expect(actual.currentPathKey(stored)).toBe(folder);
    earlierBuildRecorded(folder, databasePath, stored);

    expect(recordsAt(folder, databasePath)).toEqual(kept(folder, { status: "completed", auditId: "audit-1" }));
    expect(earlierBuildReads(databasePath, stored)).toEqual(EARLIER_BUILD_READS);
  });

  // A lookup on a share whose host does not answer waits about 21 s for the connection, and the process that started it
  // cannot exit sooner. The step reads no share no caller works on: the address is one no host answers on, never read.
  it.runIf(process.platform === "win32")("leave, on Windows, a share's keys waiting without reading it, and copy the rest", async () => {
    const actual = await vi.importActual<PathKeySpelling>("../../../src/engine/storage/path-key-spelling.js");
    vi.mocked(currentPathKey).mockImplementation(actual.currentPathKey);
    const { folder, databasePath } = await checkoutFolder();
    onTheSystemDrive(folder);
    const stored = otherSpelling(folder);
    earlierBuildRecorded(folder, databasePath, stored);
    // A documentation address no host answers on (RFC 5737).
    const share = "\\\\192.0.2.1\\share\\app";
    write(databasePath, [["INSERT INTO audit_settings (workspace_root, settings_json, updated_at) VALUES (?, '{\"scope\":\"repository\",\"mode\":\"log\"}', 1)", share]]);

    const database = openRerailDatabase(databasePath);
    try {
      // The folder's audit settings are copied to its stored spelling and stay under the old one; the share's stay, and
      // its key waits for a later open.
      expect(rows(database, "SELECT workspace_root FROM audit_settings ORDER BY workspace_root"))
        .toEqual([folder, stored, share].sort().map(root => ({ workspace_root: root })));
      expect(rows(database, "SELECT stored, current FROM path_key_spellings ORDER BY stored"))
        .toEqual([{ stored: share, current: null }, { stored, current: folder }].sort((left, right) => (left.stored < right.stored ? -1 : 1)));
    } finally {
      database.$client.close();
    }
  });

  it("keep their key where the folder is gone, since the file system stores no spelling for it", async () => {
    const actual = await vi.importActual<PathKeySpelling>("../../../src/engine/storage/path-key-spelling.js");
    const root = realpathSync.native(await mkdtemp(join(tmpdir(), "rerail-key-spelling-")));
    temporaryDirectories.push(root);
    // A parent reached through a link (a junction on Windows) is spelt another way once looked up, as `c:\` is there.
    mkdirSync(join(root, "real", "here"), { recursive: true });
    symlinkSync(join(root, "real"), join(root, "link"), "junction");

    expect(actual.currentPathKey(join(root, "link", "here"), "win32")).toBe(join(root, "real", "here"));
    expect(actual.currentPathKey(join(root, "link", "gone"), "win32")).toBe(join(root, "link", "gone"));
  });

  it.runIf(process.platform === "win32")("stay, on Windows, where the folder is gone, so a session opened there as `c:\\` still reads its audit settings", async () => {
    const actual = await vi.importActual<PathKeySpelling>("../../../src/engine/storage/path-key-spelling.js");
    vi.mocked(currentPathKey).mockImplementation(actual.currentPathKey);
    const { folder, databasePath } = await checkoutFolder({ git: false });
    onTheSystemDrive(folder);
    const stored = otherSpelling(folder).replace(/^[A-Z]:/u, drive => drive.toLowerCase());
    earlierBuildRecorded(folder, databasePath, stored);
    await removeFolder(folder);

    // The audit controls look a gone folder up by the spelling a session gives, which the old key still has.
    const controls = new SqliteAuditControlStore(databasePath);
    try {
      expect(controls.getWorkspaceSettings(stored)).toEqual({ scope: "ai-output", mode: "log", steerStrategy: "message" });
    } finally {
      controls.close();
    }
    const database = openRerailDatabase(databasePath);
    try {
      expect(rows(database, `SELECT checkout_path AS key FROM checkouts
        UNION ALL SELECT owner_id FROM rule_settings WHERE owner_kind = 'checkout'
        UNION ALL SELECT workspace_root FROM audit_settings UNION ALL SELECT workspace_root FROM repository_audits
        UNION ALL SELECT repository FROM presentation_preferences WHERE repository <> ''
        UNION ALL SELECT workspace_root FROM audits UNION ALL SELECT workspace_root FROM audit_requests`)).toEqual(Array(7).fill({ key: stored }));
      expect(rows(database, "SELECT stored FROM path_key_spellings")).toEqual([]);
    } finally {
      database.$client.close();
    }
  });

  it("keep their spelling outside Windows, where the realpath did not change", async () => {
    const actual = await vi.importActual<PathKeySpelling>("../../../src/engine/storage/path-key-spelling.js");
    expect(actual.currentPathKey("/no/such/Folder", "linux")).toBe("/no/such/Folder");
    expect(actual.currentPathKey("/no/such/Folder", "darwin")).toBe("/no/such/Folder");
  });

  // The start-state check copied through a writable open that waited up to 5 s for another writer's lock, then threw
  // SQLITE_BUSY into the hook, where the read-only open the check made before never waits on a writer.
  it("check the start state while another writer holds the lock as a read-only open does, and copy at the next check", async () => {
    await onWindows();
    const { folder, databasePath } = await checkoutFolder();
    vi.stubEnv("SystemDrive", "Q:");
    const stored = otherSpelling(folder);
    earlierBuildRecorded(folder, databasePath, stored);
    vi.mocked(currentPathKey).mockImplementation(path => (path === stored ? folder : path));
    openRerailDatabase(databasePath).$client.close();
    const release = holdWriteLock(databasePath);
    const reader = openRerailDatabase(databasePath, { readonly: true });
    const asTheFileHoldsIt = startStateReader(reader)(folder);
    reader.$client.close();

    const started = performance.now();
    expect(workspaceStartState(folder, databasePath)).toBe(asTheFileHoldsIt);
    expect(performance.now() - started).toBeLessThan(1_000);
    expect(spellings(databasePath)).toEqual([{ stored, current: null }]);
    release();
    expect(workspaceStartState(folder, databasePath)).toBe("started");
    expect(spellings(databasePath)).toEqual([{ stored, current: folder }]);
  }, 60_000);

  // The daemon copied before it answered, through a writable open that held its thread up to 5 s for another writer's
  // lock and then failed the request: the hook lost the daemon's answer, and every later request from the drive retried.
  it("answer a hook while another writer holds the lock, say why in the daemon's log, and copy at the next request", async () => {
    await onWindows();
    vi.stubEnv("SystemDrive", "Q:");
    const { folder, databasePath } = await checkoutFolder();
    const stored = otherSpelling(folder);
    earlierBuildRecorded(folder, databasePath, stored);
    vi.mocked(currentPathKey).mockImplementation(path => (path === stored ? folder : path));
    await daemonRunning(databasePath);
    const release = holdWriteLock(databasePath);
    const log = vi.spyOn(process.stderr, "write").mockImplementation(() => true);
    ends.push(() => { log.mockRestore(); });
    const ask = (): Promise<Response> => fetch(`http://127.0.0.1:4737/api/presentation?${new URLSearchParams({ repository: folder, host: "claude-code" })}`);

    const started = performance.now();
    const answer = await ask();
    expect({ status: answer.status, quick: performance.now() - started < 1_000 }).toEqual({ status: 200, quick: true });
    expect(log.mock.calls.map(([line]) => String(line)).filter(line => line.includes("earlier build")))
      .toEqual([`rerail: the keys an earlier build stored on ${win32.parse(folder).root} wait: database is locked\n`]);
    expect(spellings(databasePath)).toEqual([{ stored, current: null }]);
    release();
    expect((await ask()).status).toBe(200);
    expect(heldFor(databasePath, folder)).toEqual(HELD);
  }, 60_000);

  // The daemon copied for the folder a request named before the handler checked that a coding agent may read there, so
  // an agent's request looked keys up on a drive none of its folders is on, and was refused only after.
  it("copy for a coding agent's request only on the drive of its own checkout, and refuse one elsewhere as before", async () => {
    await onWindows();
    vi.stubEnv("SystemDrive", "Q:");
    const { folder, databasePath } = await checkoutFolder();
    const stored = otherSpelling(folder);
    earlierBuildRecorded(folder, databasePath, stored);
    vi.mocked(currentPathKey).mockImplementation(path => (path === stored ? folder : path));
    const stores = new DatabaseApiStores(databasePath, {
      health: () => ({ pid: process.pid, database: { path: databasePath, exists: true }, native: { ok: true } }),
      origin: () => "http://127.0.0.1:4737",
    });
    ends.push(() => { stores.close(); });
    // The daemon's start: a writable open that names no folder, which reads no drive but `Q:`.
    stores.database();
    const presentation = operations.find(operation => operation.id === "presentation_get")!;
    const input = { repository: folder, host: "claude-code" };

    // An agent whose checkout is on another drive, which Rerail has not seen: the handler refuses it.
    await expect(invokeOperation(presentation, stores, input, { kind: "agent", checkout: "R:\\elsewhere", folder: "" }))
      .rejects.toMatchObject({ code: "checkout-not-found" });
    expect({ looked: vi.mocked(currentPathKey).mock.calls.length, spellings: spellings(databasePath) })
      .toEqual({ looked: 0, spellings: [{ stored, current: null }] });
    await expect(invokeOperation(presentation, stores, input, { kind: "agent", checkout: folder, folder: "" }))
      .resolves.toMatchObject({ presence: "silent", repositoryState: "started" });
    expect(spellings(databasePath)).toEqual([{ stored, current: folder }]);
  }, 60_000);

  // A lookup that failed for any reason read as a folder that keeps its spelling, and the key was never looked up again:
  // a share that did not answer once, or a busy file, left that folder's settings unread for good.
  it("leave a key waiting where its lookup failed for a reason that can pass, and copy its rows at a later open", async () => {
    const { folder, databasePath } = await checkoutFolder();
    onTheSystemDrive(folder);
    const stored = otherSpelling(folder);
    earlierBuildRecorded(folder, databasePath, stored);
    let answers = false;
    vi.mocked(currentPathKey).mockImplementation(path => {
      if (path === stored && !answers) throw Object.assign(new Error("ETIMEDOUT: connection timed out, access"), { code: "ETIMEDOUT" });
      return path === stored ? folder : path;
    });
    openRerailDatabase(databasePath).$client.close();
    expect(spellings(databasePath)).toEqual([{ stored, current: null }]);

    answers = true;
    expect(recordsAt(folder, databasePath)).toEqual(kept(folder, { status: "completed", auditId: "audit-1" }));
    expect(earlierBuildReads(databasePath, stored)).toEqual(EARLIER_BUILD_READS);
  });

  // `existsSync` reads any failure as a folder that is not there, so a folder the check could not read kept its spelling.
  // Two links that lead to each other fail the check with ELOOP; Windows answers it for two such junctions without an
  // error, so the case is made off Windows, with the lookup run as on Windows.
  it.runIf(process.platform !== "win32")("throw, rather than read as a folder that is gone, where the check of a folder fails otherwise", async () => {
    const actual = await vi.importActual<PathKeySpelling>("../../../src/engine/storage/path-key-spelling.js");
    const root = realpathSync.native(await mkdtemp(join(tmpdir(), "rerail-key-spelling-")));
    temporaryDirectories.push(root);
    symlinkSync(join(root, "b"), join(root, "a"));
    symlinkSync(join(root, "a"), join(root, "b"));

    expect(() => actual.currentPathKey(join(root, "a"), "win32")).toThrow(expect.objectContaining({ code: "ELOOP" }));
    expect(actual.currentPathKey(join(root, "gone"), "win32")).toBe(join(root, "gone"));
  });
});
