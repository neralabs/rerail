import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, realpathSync, symlinkSync } from "node:fs";
import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { basename, dirname, join } from "node:path";
import Database from "better-sqlite3";
import { afterEach, describe, expect, it, vi } from "vitest";
import { SqliteAuditControlStore } from "../../../src/engine/audit/audit-control.js";
import { SqliteAuditStore } from "../../../src/engine/audit/audit-store.js";
import { listRepositories, nestedCheckouts, recordCheckout } from "../../../src/engine/catalog/repository-registry.js";
import { setUpRepository } from "../../../src/engine/catalog/rule-configuration.js";
import { openRerailDatabase, type RerailDatabase } from "../../../src/engine/storage/database.js";
import { currentPathKey, pathKeysCanMove, withDriveBudget, type DriveRoot } from "../../../src/engine/storage/path-key-spelling.js";
import { setCheckoutStarted, SqlitePresentationPreferenceStore, workspaceStartState } from "../../../src/runtime/index.js";
import { removeFolder } from "../../support/platform/remove-folder.js";

type PathKeySpelling = typeof import("../../../src/engine/storage/path-key-spelling.js");

// The key a stored path moves to, which each test sets: on Linux and macOS the stored spelling is the one this build
// gives, so a test stands in for Windows with a spelling of its own, and the step runs there as on Windows
// (`pathKeysCanMove`). The Windows tests use the real one. A test stands in for a drive that is not attached, or slow
// to answer, through `withDriveBudget`; no test reaches a real share.
vi.mock("../../../src/engine/storage/path-key-spelling.js", async importOriginal => {
  const original = await importOriginal<PathKeySpelling>();
  return {
    ...original,
    currentPathKey: vi.fn(original.currentPathKey),
    withDriveBudget: vi.fn(original.withDriveBudget),
    pathKeysCanMove: vi.fn(() => true),
  };
});

const STEP = "path-keys-as-stored";
const temporaryDirectories: string[] = [];
afterEach(async () => {
  vi.mocked(currentPathKey).mockReset();
  vi.mocked(withDriveBudget).mockReset();
  vi.mocked(pathKeysCanMove).mockReset().mockImplementation(() => true);
  await Promise.all(temporaryDirectories.splice(0).map(path => removeFolder(path)));
});

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
  it("copy the rows of a folder whose drive was absent as the step ran at a later open, once the drive is back", async () => {
    const actual = await vi.importActual<PathKeySpelling>("../../../src/engine/storage/path-key-spelling.js");
    const { folder, databasePath } = await checkoutFolder();
    const stored = otherSpelling(folder);
    earlierBuildRecorded(folder, databasePath, stored);
    vi.mocked(currentPathKey).mockImplementation(path => (path === stored ? folder : path));
    vi.mocked(withDriveBudget).mockImplementation(keyOf => path => (path === stored ? undefined : keyOf(path)));
    openRerailDatabase(databasePath).$client.close();
    expect(spellings(databasePath)).toEqual([{ stored, current: null }]);

    // The drive is back; the daemon's start opens the file for writing.
    vi.mocked(withDriveBudget).mockImplementation(actual.withDriveBudget);
    openRerailDatabase(databasePath).$client.close();
    expect(spellings(databasePath)).toEqual([{ stored, current: folder }]);
    expect(recordsAt(folder, databasePath)).toEqual(kept(folder, { status: "completed", auditId: "audit-1" }));
    expect(earlierBuildReads(databasePath, stored)).toEqual(EARLIER_BUILD_READS);
  });

  // Round 1 left every key on a drive mapped to a share as it was, so a folder an earlier build keyed `Q:\proj` read as
  // unset where this build looks it up by the share's spelling; and it took a local disk whose root answered after a
  // second (a sleeping one) for such a drive, for good. Each drive's root is read once; then its keys are looked up
  // within a second, and the rest wait for a later open.
  it("look keys up on any drive whose root answers, within a time per drive, and leave the rest waiting", () => {
    const roots: string[] = [];
    let clock = 0;
    // `Z:` is mapped to a share, `Y:` a disk whose root does not answer within the second, `E:` a disk not attached, and
    // `\\host\share` a share; each lookup on `D:` takes 400 ms.
    const answers: Record<string, DriveRoot> = {
      "C:\\": "answers", "Z:\\": "answers", "Y:\\": "slow", "E:\\": "absent", "D:\\": "answers", "\\\\host\\share\\": "answers",
    };
    const probe = {
      root: (root: string): DriveRoot => {
        roots.push(root);
        clock += root === "Y:\\" ? 1_000 : 1;
        return answers[root] ?? "absent";
      },
      now: () => clock,
    };
    // Through the mapping, a key on `Z:` reads as the share spells it.
    const keyOf = withDriveBudget(stored => {
      if (stored.startsWith("D:")) clock += 400;
      return stored.startsWith("Z:") ? `\\\\server\\share${stored.slice(2).toUpperCase()}` : stored.toUpperCase();
    }, "win32", probe);

    expect(["C:\\a", "C:\\b", "Z:\\c", "Y:\\e", "E:\\f", "D:\\g", "D:\\h", "D:\\i", "D:\\j", "\\\\host\\share\\k"].map(keyOf)).toEqual([
      "C:\\A", "C:\\B", "\\\\server\\share\\C", undefined, undefined, "D:\\G", "D:\\H", "D:\\I", undefined, "\\\\HOST\\SHARE\\K",
    ]);
    expect(roots).toEqual(["C:\\", "Z:\\", "Y:\\", "E:\\", "D:\\", "\\\\host\\share\\"]);
    expect(withDriveBudget(stored => stored.toUpperCase(), "linux", probe)("/a")).toBe("/A");
  });

  // Each drive's root is read in a thread of its own, so a share whose host does not answer holds the open a second at
  // most, not the 21 s Windows waits for it. Here only roots that answer at once are read: this computer's, and a drive
  // letter that names no drive.
  it("read a drive's root in a thread of its own: a root that is there answers, a drive that is not is absent", async () => {
    const actual = await vi.importActual<PathKeySpelling>("../../../src/engine/storage/path-key-spelling.js");
    const absent = [..."ZYXWVUTSRQPONMLKJIHGF"].map(letter => `${letter}:\\`).find(root => !existsSync(root));
    const keyOf = actual.withDriveBudget(stored => `${stored}-looked-up`, "win32");
    const here = realpathSync.native(tmpdir());

    expect([here, `${absent}app`].map(keyOf)).toEqual([`${here}-looked-up`, undefined]);
  });

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

  // A lookup on a share whose host does not answer waits about 21 s for the connection. The root read here stands in
  // for one that did not answer within the second (`withDriveBudget` reads it in a thread of its own): no share is reached.
  it.runIf(process.platform === "win32")("wait, on Windows, for a share whose root does not answer, and copy the rest", async () => {
    const actual = await vi.importActual<PathKeySpelling>("../../../src/engine/storage/path-key-spelling.js");
    vi.mocked(currentPathKey).mockImplementation(actual.currentPathKey);
    const probe = { root: (root: string): DriveRoot => (root.startsWith("\\\\") ? "slow" : "answers"), now: () => Date.now() };
    vi.mocked(withDriveBudget).mockImplementation(keyOf => actual.withDriveBudget(keyOf, "win32", probe));
    const { folder, databasePath } = await checkoutFolder();
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
});
