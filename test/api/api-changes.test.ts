import { execFileSync } from "node:child_process";
import { EventEmitter } from "node:events";
import type { FSWatcher, watch as watchFolder } from "node:fs";
import { realpathSync } from "node:fs";
import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { OpenAPIHono } from "@hono/zod-openapi";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createRerailApi } from "../../src/api/http.js";
import { operations } from "../../src/api/operations.js";
import { DatabaseApiStores, type DatabaseApiStoresOptions } from "../../src/api/stores.js";
import { openRerailDatabase, recordCheckout, type RerailDatabase } from "../../src/engine/index.js";
import { SqliteSessionStore, type MergeAssistService } from "../../src/runtime/index.js";
import { installPack, uninstallPack } from "../support/installed-packs.js";
import { gitRepository } from "../support/repository-setup.js";
import { removeFolder } from "../support/platform/remove-folder.js";
import { startDatabaseWriter } from "../support/processes/database-writer.js";

/** The beat of these streams: none comes within a test unless the test moves the clock to it. */
const BEAT_MS = 60 * 60_000;

interface Message { readonly event: string; readonly data: string }

/** Folder watches the test drives: `report(folder, name)` is the OS saying `name` in `folder` changed. */
function drivenWatch(): { watch: typeof watchFolder; report: (folder: string, name: string) => void } {
  const listeners = new Map<string, (event: string, name: string) => void>();
  const watch = ((folder: string, _options: unknown, onChange: (event: string, name: string) => void) => {
    listeners.set(folder, onChange);
    const watcher = new EventEmitter() as FSWatcher;
    watcher.close = () => { listeners.delete(folder); };
    return watcher;
  }) as unknown as typeof watchFolder;
  return { watch, report: (folder, name) => listeners.get(folder)?.("change", name) };
}

/** One server-sent event's lines; `event` is "message" when it names none. */
function parseMessage(text: string): Message {
  const field = (name: string): string | undefined => text.split("\n").find(line => line.startsWith(`${name}: `))?.slice(name.length + 2);
  return { event: field("event") ?? "message", data: field("data") ?? "" };
}

/** The messages of a held response, one at a time as they arrive; `stop()` leaves it, as a closed tab does. */
function messagesOf(response: Response): { next: () => Promise<Message>; stop: () => Promise<void> } {
  const reader = response.body!.pipeThrough(new TextDecoderStream()).getReader();
  let rest = "";
  const next = async (): Promise<Message> => {
    for (let end = rest.indexOf("\n\n"); end === -1; end = rest.indexOf("\n\n")) {
      const { value, done } = await reader.read();
      if (done) throw new Error("the stream ended");
      rest += value;
    }
    const end = rest.indexOf("\n\n");
    const message = parseMessage(rest.slice(0, end));
    rest = rest.slice(end + 2);
    return message;
  };
  return { next, stop: () => reader.cancel() };
}

describe("the daemon's change stream (GET /api/changes)", () => {
  let root: string;
  let databasePath: string;
  let database: RerailDatabase;
  let stores: DatabaseApiStores;
  let app: OpenAPIHono;
  const originalRerailHome = process.env.RERAIL_HOME;

  beforeEach(async () => {
    root = await mkdtemp(join(tmpdir(), "rerail-changes-"));
    process.env.RERAIL_HOME = join(root, "home");
    installPack("complexity");
    databasePath = join(root, "rerail.sqlite");
    database = openRerailDatabase(databasePath);
    stores = new DatabaseApiStores(databasePath, {
      health: () => ({ pid: process.pid, database: { path: databasePath, exists: true }, native: { ok: true } }),
      origin: () => "http://127.0.0.1:4737",
      changeFeed: { beatMs: BEAT_MS },
    });
    app = createRerailApi(stores, operations, { version: "0.0.0-test" });
  });

  afterEach(async () => {
    vi.useRealTimers();
    stores.close();
    database.$client.close();
    if (originalRerailHome === undefined) delete process.env.RERAIL_HOME;
    else process.env.RERAIL_HOME = originalRerailHome;
    await removeFolder(root);
  });

  /** Stores over the test's database, as the daemon makes them, with `options` added. */
  const storesFor = (options: Partial<DatabaseApiStoresOptions>): DatabaseApiStores => new DatabaseApiStores(databasePath, {
    health: () => ({ pid: process.pid, database: { path: databasePath, exists: true }, native: { ok: true } }),
    origin: () => "http://127.0.0.1:4737",
    ...options,
  });

  const open = async (query = ""): Promise<ReturnType<typeof messagesOf>> => {
    const response = await app.request(`/api/changes${query}`);
    expect([response.status, response.headers.get("content-type")]).toEqual([200, "text/event-stream"]);
    return messagesOf(response);
  };

  it("sends its change token at once, and a new one when a person's write commits", async () => {
    const stream = await open();
    const first = await stream.next();

    const written = await app.request("/api/presentation-preferences", {
      method: "PUT", headers: { "content-type": "application/json" }, body: JSON.stringify({ repository: "", host: "", presence: "silent" }),
    });
    expect(written.status).toBe(200);

    const next = await stream.next();
    expect(next.event).toBe("message");
    expect(next.data).not.toBe(first.data);
    await stream.stop();
  });

  it("sends a new token when another process commits through Rerail's own opener, as the CLI's writes do", async () => {
    // The writer syncs the write-ahead log between writing its commit and publishing it, so the daemon hears the
    // write before it can read the commit, and no later notice comes.
    const writer = await startDatabaseWriter(databasePath);
    const stream = await open();
    try {
      const first = await stream.next();
      await writer.commit("remote:github.com/acme/web");

      expect((await stream.next()).data).not.toBe(first.data);
    } finally {
      await stream.stop();
      await writer.close();
    }
  });

  it("sends a new token when another connection of the daemon's process commits, as a hook's records do", async () => {
    const stream = await open();
    const first = await stream.next();
    const other = new SqliteSessionStore(databasePath);
    try {
      other.openSession({ host: "codex", sessionId: "session-b", cwd: root });

      expect((await stream.next()).data).not.toBe(first.data);
    } finally {
      other.close();
      await stream.stop();
    }
  });

  it("sends nothing for a checkpoint that copies the write-ahead log into the database, which commits nothing", async () => {
    // The folder's notices come as the test says, so the checkpoint's are heard alone, before any commit. A passive
    // checkpoint, as SQLite runs after commits, leaves the log's header as it was, so no reader's version moves.
    const folder = drivenWatch();
    const watched = storesFor({ changeFeed: { beatMs: BEAT_MS, watch: folder.watch } });
    const stream = messagesOf(await createRerailApi(watched, operations, { version: "0.0.0-test" }).request("/api/changes"));
    const other = new SqliteSessionStore(databasePath);
    try {
      const first = await stream.next();
      database.$client.pragma("wal_checkpoint(PASSIVE)");
      folder.report(root, "rerail.sqlite");
      folder.report(root, "rerail.sqlite-wal");
      await new Promise(resolve => setImmediate(resolve));

      other.openSession({ host: "codex", sessionId: "session-c", cwd: root });
      folder.report(root, "rerail.sqlite-wal");

      // The first message after the checkpoint is the commit's: the checkpoint's own notices sent nothing.
      expect((await stream.next()).data).not.toBe(first.data);
    } finally {
      other.close();
      await stream.stop();
      watched.close();
    }
  });

  it("tells a conflict event of the Merge Assist the stores answer with as a conflicts message", async () => {
    let conflictsChanged: ((checkoutPath: string) => void) | undefined;
    // Only the conflict events are read here; the stores call nothing else of it unless a Merge Assist route runs.
    const mergeAssist = {
      onConflictsChanged: (listener: (checkoutPath: string) => void) => {
        conflictsChanged = listener;
        return () => { conflictsChanged = undefined; };
      },
    } as unknown as MergeAssistService;
    const withMergeAssist = storesFor({ changeFeed: { beatMs: BEAT_MS }, mergeAssist });
    const stream = messagesOf(await createRerailApi(withMergeAssist, operations, { version: "0.0.0-test" }).request("/api/changes"));
    try {
      await stream.next();

      conflictsChanged?.(root);

      expect((await stream.next()).event).toBe("conflicts");
    } finally {
      await stream.stop();
      withMergeAssist.close();
    }
    expect(conflictsChanged).toBeUndefined();
  });

  it("sends a new token when the stores count a write of their own, before any file notice of its commit", async () => {
    const stream = await open();
    const first = await stream.next();

    stores.markChanged();

    expect((await stream.next()).data).not.toBe(first.data);
    await stream.stop();
  });

  it("sends the token again at each beat, the same while nothing changed", async () => {
    vi.useFakeTimers({ toFake: ["setInterval", "clearInterval"] });
    const stream = await open();
    const first = await stream.next();

    await vi.advanceTimersByTimeAsync(BEAT_MS);

    expect(await stream.next()).toEqual(first);
    await stream.stop();
  });

  it("sends a new token when a pack is removed, with no beat: the packs' folders are watched", async () => {
    const stream = await open();
    const first = await stream.next();

    uninstallPack("complexity");

    expect((await stream.next()).data).not.toBe(first.data);
    await stream.stop();
  });

  it("sends a new token at the beat after origin's default branch moved, which Merge Assist names as the base", async () => {
    vi.useFakeTimers({ toFake: ["setInterval", "clearInterval"] });
    const checkout = gitRepository(join(realpathSync(root), "web"));
    recordCheckout(database, checkout);
    const stream = await open(`?${new URLSearchParams({ checkout })}`);
    const first = await stream.next();

    execFileSync("git", ["symbolic-ref", "refs/remotes/origin/HEAD", "refs/remotes/origin/trunk"], { cwd: checkout });
    await vi.advanceTimersByTimeAsync(BEAT_MS);

    expect((await stream.next()).data).not.toBe(first.data);
    await stream.stop();
  });

  it("sends a new token at the beat after the checkout's branch moved, which Merge Assist's status names", async () => {
    vi.useFakeTimers({ toFake: ["setInterval", "clearInterval"] });
    const checkout = gitRepository(join(realpathSync(root), "web"));
    recordCheckout(database, checkout);
    const stream = await open(`?${new URLSearchParams({ checkout })}`);
    const first = await stream.next();

    execFileSync("git", ["switch", "-q", "-c", "feature"], { cwd: checkout });
    await vi.advanceTimersByTimeAsync(BEAT_MS);

    expect((await stream.next()).data).not.toBe(first.data);
    await stream.stop();
  });

  it("stops listening when the client leaves: no beat is left running", async () => {
    vi.useFakeTimers({ toFake: ["setInterval", "clearInterval"] });
    const stream = await open();
    await stream.next();
    expect(vi.getTimerCount()).toBe(1);

    await stream.stop();

    expect(vi.getTimerCount()).toBe(0);
  });
});
