import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { OpenAPIHono } from "@hono/zod-openapi";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createRerailApi } from "../../src/api/http.js";
import { operations } from "../../src/api/operations.js";
import { DatabaseApiStores } from "../../src/api/stores.js";
import { openRerailDatabase, type RerailDatabase } from "../../src/engine/index.js";
import { SqliteSessionStore } from "../../src/runtime/index.js";
import { installPack, uninstallPack } from "../support/installed-packs.js";
import { removeFolder } from "../support/platform/remove-folder.js";

/** The beat of these streams: none comes within a test unless the test moves the clock to it. */
const BEAT_MS = 60 * 60_000;

interface Message { readonly event: string; readonly data: string }

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

  const open = async (): Promise<ReturnType<typeof messagesOf>> => {
    const response = await app.request("/api/changes");
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

  it("sends a new token when another connection commits, as a hook's records and the CLI's writes do", async () => {
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

  it("sends nothing for a checkpoint of the write-ahead log, which commits nothing", async () => {
    const stream = await open();
    const first = await stream.next();

    database.$client.pragma("wal_checkpoint(TRUNCATE)");
    const other = new SqliteSessionStore(databasePath);
    try {
      other.openSession({ host: "codex", sessionId: "session-c", cwd: root });
      // The first message after the checkpoint is the commit's: the checkpoint's own notice sent nothing.
      expect((await stream.next()).data).not.toBe(first.data);
    } finally {
      other.close();
      await stream.stop();
    }
  });

  it("tells a Merge Assist conflict event as a conflicts message", async () => {
    const stream = await open();
    await stream.next();

    stores.changes().notify("conflicts");

    expect((await stream.next()).event).toBe("conflicts");
    await stream.stop();
  });

  it("sends the token at each beat, read again for what no notice covers: a pack removed", async () => {
    vi.useFakeTimers({ toFake: ["setInterval", "clearInterval"] });
    const stream = await open();
    const first = await stream.next();

    await vi.advanceTimersByTimeAsync(BEAT_MS);
    expect(await stream.next()).toEqual(first);

    uninstallPack("complexity");
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
