import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import Database from "better-sqlite3";
import { afterEach, describe, expect, it, vi } from "vitest";
import { SqliteAuditControlStore } from "../../../src/engine/audit/audit-control.js";
import { DEFAULT_AUDIT_SETTINGS } from "../../../src/engine/audit/audit-control-schema.js";
import { SqliteSessionStore } from "../../../src/runtime/index.js";
import { removeFolder } from "../../support/platform/remove-folder.js";

const directories: string[] = [];
afterEach(async () => {
  vi.restoreAllMocks();
  await Promise.all(directories.splice(0).map(path => removeFolder(path)));
});

/** A session `a` in a checkout, and the store that keeps its settings and scans. */
async function fixture() {
  const root = await mkdtemp(join(tmpdir(), "rerail-damaged-controls-"));
  directories.push(root);
  const path = join(root, "rerail.sqlite");
  const sessions = new SqliteSessionStore(path);
  sessions.openSession({ host: "omp", sessionId: "a", cwd: root });
  sessions.close();
  return { root, path, controls: new SqliteAuditControlStore(path) };
}

/** Spies on this thread's stderr, where the store writes the line that names a damaged row: what it holds so far. */
function spyOnStderr(): () => string {
  const stderr = vi.spyOn(process.stderr, "write").mockReturnValue(true);
  return () => stderr.mock.calls.map(([chunk]) => String(chunk)).join("");
}

/**
 * Cuts the last 2 characters off the stored text of `column` in the rows of `table` that `id` names (every row when it
 * is undefined), with the CHECKs off for the one write, as the issue's steps do; the text it left.
 */
function cutShort(path: string, table: "audit_settings" | "audit_requests", column: string, id?: string): string {
  const client = new Database(path);
  try {
    client.pragma("ignore_check_constraints = ON");
    const where = id === undefined ? "" : " WHERE id = ?";
    const key = id === undefined ? [] : [id];
    client.prepare(`UPDATE ${table} SET ${column} = substr(${column}, 1, length(${column}) - 2)${where}`).run(...key);
    return client.prepare(`SELECT ${column} FROM ${table}${where}`).pluck().get(...key) as string;
  } finally {
    client.close();
  }
}

/** Stores `text` as the settings of every `audit_settings` row, as a newer build or a hand edit leaves it. */
function storeSettings(path: string, text: string): void {
  const client = new Database(path);
  client.prepare("UPDATE audit_settings SET settings_json = ?").run(text);
  client.close();
}

/** The settings text the one `audit_settings` row holds. */
function settingsText(path: string): unknown {
  const client = new Database(path, { readonly: true });
  const text: unknown = client.prepare("SELECT settings_json FROM audit_settings").pluck().get();
  client.close();
  return text;
}

/**
 * Another program's hold on the database's write lock, as `rerail scrub` or another build's migration takes it: a
 * connection of its own in an immediate transaction, so this thread's writes meet `SQLITE_BUSY`. `release` gives it back.
 */
function holdWriteLock(path: string): { release: () => void } {
  const holder = new Database(path);
  holder.exec("BEGIN IMMEDIATE");
  return {
    release: () => {
      if (!holder.open) return;
      holder.exec("ROLLBACK");
      holder.close();
    },
  };
}

/** The message `JSON.parse` gives for `text`, as the line that names its row quotes it. */
function parseError(text: string): string {
  try {
    JSON.parse(text);
  } catch (error) {
    return (error as SyntaxError).message;
  }
  throw new Error(`${text} parses`);
}

describe("a checkout's saved scan settings whose stored text does not read", () => {
  it("read as the defaults, which a session's start and `rerail settings` get, are named once, and a write saves over them", async () => {
    const { root, path, controls } = await fixture();
    controls.updateSettings("a", { mode: "log" });
    const left = cutShort(path, "audit_settings", "settings_json");
    const logged = spyOnStderr();

    const read = controls.getSettings("a");
    expect(read.settings).toEqual(DEFAULT_AUDIT_SETTINGS);
    expect(controls.getWorkspaceSettings(root)).toBeUndefined();
    expect(logged().match(/rerail: /gu)).toHaveLength(1);
    expect(logged()).toContain(`: the scan settings saved for ${read.workspaceRoot} read as the defaults: the settings_json of its `
      + `audit_settings row is not JSON (${parseError(left)}).`);

    expect(controls.updateSettings("a", { mode: "log" }).settings).toEqual({ ...DEFAULT_AUDIT_SETTINGS, mode: "log" });
    expect(controls.getSettings("a").settings).toEqual({ ...DEFAULT_AUDIT_SETTINGS, mode: "log" });
    expect(logged().match(/rerail: /gu)).toHaveLength(1);
    controls.close();
  });

  it("read as the defaults when they parse to a JSON value that is no object, and a write saves over them", async () => {
    const { path, controls } = await fixture();
    controls.updateSettings("a", { mode: "log" });
    storeSettings(path, JSON.stringify(JSON.stringify({ scope: "touched-files", mode: "log" })));
    const logged = spyOnStderr();

    expect(controls.getSettings("a").settings).toEqual(DEFAULT_AUDIT_SETTINGS);
    expect(logged()).toMatch(/read as the defaults: the settings_json of its audit_settings row holds no settings this version reads \(/u);
    controls.updateSettings("a", { mode: "log" });
    expect(settingsText(path)).toBe("{\"scope\":\"ai-output\",\"mode\":\"log\"}");
    controls.close();
  });

  // A session's start reads its checkout's settings beside another program's held write lock, and its owner claim
  // waits in the record writer for that lock (`registerOwner`).
  it("read as the defaults at a session's start beside another program's write lock, and its claim is made once that is given back", async () => {
    const { root, path, controls } = await fixture();
    controls.updateSettings("a", { mode: "log" });
    cutShort(path, "audit_settings", "settings_json");
    const logged = spyOnStderr();
    const lock = holdWriteLock(path);
    try {
      expect(controls.getWorkspaceSettings(root)).toBeUndefined();
      const claim = controls.registerOwner("a", "background");
      expect(controls.holdsOwner("a", claim)).toBe(true);
      expect(controls.getSettings("a")).toMatchObject({ settings: DEFAULT_AUDIT_SETTINGS, active: false });
      lock.release();
      await vi.waitFor(() => expect(controls.getSettings("a").active).toBe(true), { timeout: 10_000, interval: 20 });
      expect(controls.getSettings("a").settings).toEqual(DEFAULT_AUDIT_SETTINGS);
      expect(logged().match(/rerail: /gu)).toHaveLength(1);
      expect(logged()).toContain("read as the defaults: the settings_json of its audit_settings row is not JSON");
    } finally {
      lock.release();
      controls.close();
    }
  }, 20_000);

  // The scan settings save takes the write lock on the caller's thread, where the rule settings save waits for it in the
  // record writer. Its transaction reads the saved row before it writes, and SQLite answers such a transaction's write
  // with the lock held at once, with no busy wait: the save fails, and the damaged row still reads as the defaults.
  it("stay the defaults when a save over them meets another program's write lock, which fails it at once, and the same save is made once that is given back", async () => {
    const { path, controls } = await fixture();
    controls.updateSettings("a", { mode: "log" });
    const left = cutShort(path, "audit_settings", "settings_json");
    const logged = spyOnStderr();
    const lock = holdWriteLock(path);
    try {
      expect(() => controls.updateSettings("a", { scope: "touched-files" })).toThrow(/database is locked/u);
      lock.release();
      const client = new Database(path, { readonly: true });
      expect(client.prepare("SELECT settings_json FROM audit_settings").pluck().get()).toBe(left);
      client.close();
      expect(controls.getSettings("a").settings).toEqual(DEFAULT_AUDIT_SETTINGS);
      expect(controls.updateSettings("a", { scope: "touched-files" }).settings).toEqual({ ...DEFAULT_AUDIT_SETTINGS, scope: "touched-files" });
      expect(logged().match(/rerail: /gu)).toHaveLength(1);
    } finally {
      lock.release();
      controls.close();
    }
  }, 20_000);
});

// A newer build can store settings this version does not know: a key it added, or a value it added to a setting.
describe("a checkout's saved scan settings that parse to settings this version knows in part", () => {
  it("read the rest as stored, are named once, and a save keeps the key this version does not know", async () => {
    const { path, controls } = await fixture();
    controls.updateSettings("a", { mode: "log" });
    storeSettings(path, "{\"scope\":\"touched-files\",\"mode\":\"log\",\"later\":{\"on\":true}}");
    const logged = spyOnStderr();

    const read = controls.getSettings("a");
    expect(read.settings).toEqual({ ...DEFAULT_AUDIT_SETTINGS, scope: "touched-files", mode: "log" });
    expect(controls.updateSettings("a", { mode: "steer" }).settings).toEqual({ ...DEFAULT_AUDIT_SETTINGS, scope: "touched-files" });
    expect(settingsText(path)).toBe("{\"scope\":\"touched-files\",\"mode\":\"steer\",\"later\":{\"on\":true}}");
    expect(logged().match(/rerail: /gu)).toHaveLength(1);
    expect(logged()).toContain(`: the scan settings saved for ${read.workspaceRoot} read in part: the settings_json of its audit_settings row holds later (`);
    controls.close();
  });

  it("read a value this version does not know as its setting's default, and a save keeps it until it sets that setting", async () => {
    const { path, controls } = await fixture();
    controls.updateSettings("a", { mode: "log" });
    storeSettings(path, "{\"scope\":\"touched-files\",\"mode\":\"later\"}");
    const logged = spyOnStderr();

    const read = controls.getSettings("a");
    expect(read.settings).toEqual({ ...DEFAULT_AUDIT_SETTINGS, scope: "touched-files" });
    expect(controls.updateSettings("a", { scope: "ai-output" }).settings).toEqual(DEFAULT_AUDIT_SETTINGS);
    expect(settingsText(path)).toBe("{\"scope\":\"ai-output\",\"mode\":\"later\"}");
    expect(controls.updateSettings("a", { mode: "log" }).settings).toEqual({ ...DEFAULT_AUDIT_SETTINGS, mode: "log" });
    expect(settingsText(path)).toBe("{\"scope\":\"ai-output\",\"mode\":\"log\"}");
    expect(logged().match(/rerail: /gu)).toHaveLength(1);
    expect(logged()).toContain(`: the scan settings saved for ${read.workspaceRoot} read in part: the settings_json of its audit_settings row holds mode (mode: `);
    controls.close();
  });

  // Every object inherits `constructor` and `toString`, so only a test of the patch's own keys tells such a key apart.
  it("keep keys this version does not know whose names every object inherits through a save that does not set them", async () => {
    const { path, controls } = await fixture();
    controls.updateSettings("a", { mode: "log" });
    storeSettings(path, "{\"scope\":\"touched-files\",\"mode\":\"log\",\"constructor\":1,\"toString\":\"later\"}");
    spyOnStderr();

    expect(controls.updateSettings("a", { mode: "steer" }).settings).toEqual({ ...DEFAULT_AUDIT_SETTINGS, scope: "touched-files" });
    expect(settingsText(path)).toBe("{\"scope\":\"touched-files\",\"mode\":\"steer\",\"constructor\":1,\"toString\":\"later\"}");
    controls.close();
  });
});

describe("a session's scan requests whose stored text does not read", () => {
  /** Session `a`'s store with three repository scans queued under a live claim, and their ids in the queue's order. */
  async function threeQueued() {
    const fixed = await fixture();
    const claim = fixed.controls.registerOwner("a", "background");
    for (let made = 0; made < 3; made += 1) await fixed.controls.requestAudit("a", { scope: "repository" });
    const ids = fixed.controls.listAuditRequests("a").map(request => request.id) as [string, string, string];
    return { ...fixed, claim, ids };
  }

  it("are left out of the session's scan list, each named once, and a new scan still queues", async () => {
    const { path, controls, ids: [first, second, third] } = await threeQueued();
    const paths = cutShort(path, "audit_requests", "paths_json", first);
    const auditIds = cutShort(path, "audit_requests", "audit_ids_json", second);
    const logged = spyOnStderr();

    expect(controls.listAuditRequests("a").map(request => request.id)).toEqual([third]);
    const asked = await controls.requestAudit("a", { scope: "repository" });
    expect(asked.status).toBe("queued");
    expect(controls.listAuditRequests("a").map(request => request.id)).toContain(asked.id);
    expect(logged().match(/rerail: /gu)).toHaveLength(2);
    expect(logged()).toContain(`: scan request ${first} of session a is left out of the session's scans: the paths_json of its audit_requests row `
      + `is not a JSON array of strings (${parseError(paths)})`);
    expect(logged()).toContain(`: scan request ${second} of session a is left out of the session's scans: the audit_ids_json of its `
      + `audit_requests row is not a JSON array of strings (${parseError(auditIds)})`);
    controls.close();
  });

  it("fail when they come first in the queue, named once as failed with why, and the next one is claimed", async () => {
    const { path, controls, claim, ids: [first, second, third] } = await threeQueued();
    const paths = cutShort(path, "audit_requests", "paths_json", first);
    const logged = spyOnStderr();

    expect(controls.listAuditRequests("a").map(request => request.id)).toEqual([second, third]);
    expect(controls.claim("a", claim)?.id).toBe(second);
    expect(controls.claim("a", claim)?.id).toBe(third);
    expect(controls.claim("a", claim)).toBeUndefined();
    const client = new Database(path, { readonly: true });
    const failed: unknown = client.prepare("SELECT status, error FROM audit_requests WHERE id = ?").get(first);
    client.close();
    expect(failed).toEqual({ status: "failed", error: "The request's stored paths_json is not a JSON array of strings" });
    expect(controls.hasQueuedRequest("a")).toBe(false);
    expect(logged().match(/rerail: /gu)).toHaveLength(2);
    expect(logged()).toContain(`: scan request ${first} of session a is left out of the session's scans: the paths_json of its audit_requests row `);
    expect(logged()).toContain(`: scan request ${first} of session a failed in place of running: the paths_json of its audit_requests row `
      + `is not a JSON array of strings (${parseError(paths)})`);
    controls.close();
  });

  it("leave a scan's finish whole when they are damaged while it runs, named once as not recorded", async () => {
    const { path, controls, claim, ids: [first] } = await threeQueued();
    expect(controls.claim("a", claim)?.id).toBe(first);
    cutShort(path, "audit_requests", "paths_json", first);
    const logged = spyOnStderr();

    expect(() => controls.finish(first, "completed", ["audit-1"])).not.toThrow();
    const client = new Database(path);
    expect(client.prepare("SELECT status, audit_ids_json FROM audit_requests WHERE id = ?").get(first))
      .toEqual({ status: "completed", audit_ids_json: "[\"audit-1\"]" });
    client.close();
    expect(logged().match(/rerail: /gu)).toHaveLength(1);
    expect(logged()).toContain(`: scan request ${first} of session a ended, but is not recorded as its checkout's repository audit: the paths_json `);
    controls.close();
  });

  // A session's end interrupts its scans and lets its owner go in one record write (`endOwnership`), which waits for
  // another program's held write lock; the damaged request's row is in that write's update too.
  it("end interrupted with the rest at a session's end beside another program's write lock, and a scan asked for beside them ends so too", async () => {
    const { path, controls, claim, ids: [first, second, third] } = await threeQueued();
    cutShort(path, "audit_requests", "paths_json", first);
    const logged = spyOnStderr();
    const asked = await controls.requestAudit("a", { scope: "repository" });
    expect(asked.status).toBe("queued");
    const lock = holdWriteLock(path);
    try {
      controls.endOwnership("a", claim);
      expect(controls.listAuditRequests("a").map(request => [request.id, request.status]))
        .toEqual([[second, "queued"], [third, "queued"], [asked.id, "queued"]]);
      lock.release();
      await vi.waitFor(() => expect(controls.getSettings("a").active).toBe(false), { timeout: 10_000, interval: 20 });
      expect(controls.listAuditRequests("a").map(request => [request.id, request.status]))
        .toEqual([[second, "interrupted"], [third, "interrupted"], [asked.id, "interrupted"]]);
      const client = new Database(path, { readonly: true });
      expect(client.prepare("SELECT status FROM audit_requests WHERE id = ?").pluck().get(first)).toBe("interrupted");
      client.close();
      expect(controls.hasQueuedRequest("a")).toBe(false);
      expect(logged().match(/rerail: /gu)).toHaveLength(1);
      expect(logged()).toContain(`: scan request ${first} of session a is left out of the session's scans`);
    } finally {
      lock.release();
      controls.close();
    }
  }, 20_000);
});
