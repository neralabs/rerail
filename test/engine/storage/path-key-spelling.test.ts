import { execFileSync } from "node:child_process";
import { mkdirSync, realpathSync, symlinkSync } from "node:fs";
import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { basename, dirname, join } from "node:path";
import Database from "better-sqlite3";
import { afterEach, describe, expect, it, vi } from "vitest";
import { SqliteAuditControlStore } from "../../../src/engine/audit/audit-control.js";
import { SqliteAuditStore } from "../../../src/engine/audit/audit-store.js";
import { listRepositories, recordCheckout } from "../../../src/engine/catalog/repository-registry.js";
import { setUpRepository } from "../../../src/engine/catalog/rule-configuration.js";
import { openRerailDatabase, type RerailDatabase } from "../../../src/engine/storage/database.js";
import { currentPathKey } from "../../../src/engine/storage/path-key-spelling.js";
import { setCheckoutStarted, SqlitePresentationPreferenceStore, workspaceStartState } from "../../../src/runtime/index.js";
import { removeFolder } from "../../support/platform/remove-folder.js";

type PathKeySpelling = typeof import("../../../src/engine/storage/path-key-spelling.js");

// The key a stored path moves to, which each test sets: on Linux and macOS the stored spelling is the one this build
// gives, so a test stands in for Windows with a spelling of its own. The Windows tests use the real one.
vi.mock("../../../src/engine/storage/path-key-spelling.js", async importOriginal => {
  const original = await importOriginal<PathKeySpelling>();
  return { ...original, currentPathKey: vi.fn(original.currentPathKey) };
});

const STEP = "path-keys-as-stored";
const temporaryDirectories: string[] = [];
afterEach(async () => {
  vi.mocked(currentPathKey).mockReset();
  await Promise.all(temporaryDirectories.splice(0).map(path => removeFolder(path)));
});

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

function rows(database: RerailDatabase, query: string): unknown[] {
  return database.$client.prepare(query).all();
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
      settings: rows(database, "SELECT owner_kind, owner_id, \"on\" FROM rule_settings ORDER BY owner_kind"),
      auditSettings: controls.getWorkspaceSettings(folder),
      // Every row, with the mode it holds: an earlier build looks its own up by the spelling it stored.
      auditSettingsRows: rows(database, "SELECT workspace_root, json_extract(settings_json, '$.mode') AS mode FROM audit_settings ORDER BY workspace_root"),
      presence: preferences.effective(folder, "claude-code"),
      preferences: rows(database, "SELECT repository, host, presence FROM presentation_preferences ORDER BY repository"),
      repositoryAudits: rows(database, "SELECT workspace_root, status FROM repository_audits"),
      // The list of repository audits the dashboard and `repository_audits_list` show, one entry per folder.
      repositoryAuditList: audits.listRepositoryAudits().repositories.map(({ workspaceRoot, repositoryAudit, audit }) =>
        ({ workspaceRoot, status: repositoryAudit?.status, auditId: audit?.auditId })),
      auditHistory: rows(database, "SELECT workspace_root FROM audits UNION SELECT workspace_root FROM audit_requests"),
    };
  } finally {
    audits.close();
    preferences.close();
    controls.close();
    database.$client.close();
  }
}

type Records = ReturnType<typeof recordsAt>;

/**
 * What a checkout the earlier build recorded as `stored` keeps once its keys move to `folder`: every record, under
 * `folder`, and its audit settings under `stored` too, where the earlier build looks them up.
 */
function kept(folder: string, stored: string, repositoryAudit: Omit<Records["repositoryAuditList"][number], "workspaceRoot">): Records {
  return {
    startState: "started",
    checkouts: [folder],
    settings: [
      { owner_kind: "checkout", owner_id: folder, on: 0 },
      { owner_kind: "repository", owner_id: expect.stringMatching(/^clone:/u), on: 1 },
    ],
    auditSettings: { scope: "ai-output", mode: "log", steerStrategy: "message" },
    auditSettingsRows: [folder, stored].sort().map(root => ({ workspace_root: root, mode: "log" })),
    presence: "silent",
    preferences: [
      { repository: "", host: "", presence: "compact" },
      { repository: folder, host: "claude-code", presence: "silent" },
    ],
    repositoryAudits: [{ workspace_root: folder, status: repositoryAudit.status }],
    repositoryAuditList: [{ workspaceRoot: folder, ...repositoryAudit }],
    auditHistory: [{ workspace_root: folder }],
  };
}

describe("path keys an earlier build stored under another spelling", () => {
  it("keep a checkout recording, with its start state, settings, audit and presentation, once they move", async () => {
    const { folder, databasePath } = await checkoutFolder();
    const stored = otherSpelling(folder);
    earlierBuildRecorded(folder, databasePath, stored);
    vi.mocked(currentPathKey).mockImplementation(path => (path === stored ? folder : path));

    expect(recordsAt(folder, databasePath)).toEqual(kept(folder, stored, { status: "completed", auditId: "audit-1" }));
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
      ["DELETE FROM schema_migrations WHERE id = ?", STEP],
    ]);
    vi.mocked(currentPathKey).mockImplementation(path => (path === stored ? folder : path));

    expect(recordsAt(folder, databasePath)).toEqual(kept(folder, stored, { status: "interrupted", auditId: "audit-2" }));
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
      expect(vi.mocked(currentPathKey).mock.calls.map(([path]) => path)).toEqual([stored]);
    } finally {
      database.$client.close();
    }
  });

  it.runIf(process.platform === "win32")("move, on Windows, from a lower-case drive letter and another case to the stored spelling", async () => {
    const actual = await vi.importActual<PathKeySpelling>("../../../src/engine/storage/path-key-spelling.js");
    vi.mocked(currentPathKey).mockImplementation(actual.currentPathKey);
    // A folder outside Git, whose row an earlier version keyed by the folder as the caller spelt it.
    const { folder, databasePath } = await checkoutFolder({ git: false });
    // As VS Code passes a folder: `c:\...`, here with the folder's own name in capitals too.
    const stored = otherSpelling(folder).replace(/^[A-Z]:/u, drive => drive.toLowerCase());
    expect(actual.currentPathKey(stored)).toBe(folder);
    earlierBuildRecorded(folder, databasePath, stored);

    expect(recordsAt(folder, databasePath)).toEqual(kept(folder, stored, { status: "completed", auditId: "audit-1" }));
  });

  it.runIf(process.platform === "win32")("stay, on Windows, on a network share, which the step never looks up", async () => {
    const actual = await vi.importActual<PathKeySpelling>("../../../src/engine/storage/path-key-spelling.js");
    vi.mocked(currentPathKey).mockImplementation(actual.currentPathKey);
    const { folder, databasePath } = await checkoutFolder();
    const stored = otherSpelling(folder);
    earlierBuildRecorded(folder, databasePath, stored);
    // A documentation address no host answers on (RFC 5737): a lookup there waits about 21 s for the connection.
    const share = "\\\\192.0.2.1\\share\\app";
    write(databasePath, [["INSERT INTO audit_settings (workspace_root, settings_json, updated_at) VALUES (?, '{\"scope\":\"repository\",\"mode\":\"log\"}', 1)", share]]);

    const started = Date.now();
    const database = openRerailDatabase(databasePath);
    try {
      expect(Date.now() - started).toBeLessThan(2_000);
      // The folder's audit settings are copied to its stored spelling and stay under the old one; the share's stay.
      expect(rows(database, "SELECT workspace_root FROM audit_settings ORDER BY workspace_root"))
        .toEqual([folder, stored, share].sort().map(root => ({ workspace_root: root })));
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
