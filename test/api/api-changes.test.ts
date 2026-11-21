import { execFileSync } from "node:child_process";
import { appendFileSync, mkdirSync, readFileSync, realpathSync, writeFileSync } from "node:fs";
import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { OpenAPIHono } from "@hono/zod-openapi";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createRerailApi } from "../../src/api/http.js";
import { operations } from "../../src/api/operations.js";
import { DatabaseApiStores, type DatabaseApiStoresOptions } from "../../src/api/stores.js";
import { openRerailDatabase, recordCheckout, type RerailDatabase } from "../../src/engine/index.js";
import { createMergeAssistService, SqliteSessionStore, type MergeAssistService } from "../../src/runtime/index.js";
import { commit } from "../support/merge-assist/git.js";
import { FakeGithub, mergeAssistWorkspaces } from "../support/merge-assist/workspace.js";
import { installPack, installPackDirectory, thirdPartyPack, uninstallPack } from "../support/installed-packs.js";
import { gitRepository } from "../support/repository-setup.js";
import { removeFolder } from "../support/platform/remove-folder.js";
import { startDatabaseWriter } from "../support/processes/database-writer.js";
import { drivenWatch } from "../support/platform/driven-watch.js";
import { withExperiments } from "../support/environment/experiments.js";

/** The beat of these streams: none comes within a test unless the test moves the clock to it. */
const BEAT_MS = 60 * 60_000;

const mergeWorkspace = mergeAssistWorkspaces();

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

/**
 * The token of the first message, of a beat's and the one its read of the checkout's files may add, that differs from
 * `token`; undefined when neither does. A beat says the daemon is alive at once and reads the checkout's files after.
 */
async function movedFrom(stream: ReturnType<typeof messagesOf>, token: string): Promise<string | undefined> {
  for (let message = 0; message < 2; message += 1) {
    const { data } = await stream.next();
    if (data !== token) return data;
  }
  return undefined;
}

/**
 * The first message of a stream scoped to a Git checkout no stream followed before, then the token its read of the
 * checkout's files sends: the first message waits for no read, so it holds none of them yet.
 */
async function filesRead(stream: ReturnType<typeof messagesOf>): Promise<Message> {
  const first = await stream.next();
  const read = await stream.next();
  expect(read.data).not.toBe(first.data);
  return read;
}

/**
 * The first message of a stream scoped to a checkout no stream followed before, then the tokens its `reads` reads of
 * the checkout's parts send, each moving the token; the last of them.
 */
async function partsRead(stream: ReturnType<typeof messagesOf>, reads: number): Promise<Message> {
  let last = await stream.next();
  for (let read = 0; read < reads; read += 1) {
    const next = await stream.next();
    expect(next.data).not.toBe(last.data);
    last = next;
  }
  return last;
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

  it("asks a proxy in front of the daemon to hand each message on as it comes, as nginx holds a response by default", async () => {
    const response = await app.request("/api/changes");
    await response.body?.cancel();

    expect(response.headers.get("x-accel-buffering")).toBe("no");
  });

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

  it("sends a new token when another connection of the daemon's process commits a record, as a hook's records do", async () => {
    // The record writer tells the feed itself: on Windows the folder's watch hears nothing of a commit that does not wait
    // for the disk.
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

  it("sends a new token at the beat after a pack linked from a folder changed its pack.json, which no watch hears", async () => {
    const copy = thirdPartyPack(join(root, "copies"), "@acme/rerail-pack-lint", "lint");
    installPackDirectory(copy, "@acme/rerail-pack-lint");
    vi.useFakeTimers({ toFake: ["setInterval", "clearInterval"] });
    const stream = await open();
    const first = await stream.next();

    const pack = JSON.parse(readFileSync(join(copy, "pack.json"), "utf8")) as Record<string, unknown>;
    writeFileSync(join(copy, "pack.json"), JSON.stringify({ ...pack, name: "Lint, renamed" }));
    await vi.advanceTimersByTimeAsync(BEAT_MS);

    expect((await stream.next()).data).not.toBe(first.data);
    await stream.stop();
  });

  it("sends a new token at the beat after origin's default branch moved, which Merge Assist names as the base", async () => {
    vi.useFakeTimers({ toFake: ["setInterval", "clearInterval"] });
    const checkout = gitRepository(join(realpathSync(root), "web"));
    recordCheckout(database, checkout);
    const stream = await open(`?${new URLSearchParams({ checkout })}`);
    const first = await filesRead(stream);

    execFileSync("git", ["symbolic-ref", "refs/remotes/origin/HEAD", "refs/remotes/origin/trunk"], { cwd: checkout });
    await vi.advanceTimersByTimeAsync(BEAT_MS);

    expect(await movedFrom(stream, first.data)).toBeDefined();
    await stream.stop();
  });

  it("sends a new token at the beat after origin's default branch moved between two branches a pack of refs holds", async () => {
    vi.useFakeTimers({ toFake: ["setInterval", "clearInterval"] });
    const checkout = gitRepository(join(realpathSync(root), "web"));
    for (const branch of ["main", "trunk"]) execFileSync("git", ["update-ref", `refs/remotes/origin/${branch}`, "HEAD"], { cwd: checkout });
    execFileSync("git", ["symbolic-ref", "refs/remotes/origin/HEAD", "refs/remotes/origin/main"], { cwd: checkout });
    execFileSync("git", ["pack-refs", "--all"], { cwd: checkout });
    recordCheckout(database, checkout);
    const stream = await open(`?${new URLSearchParams({ checkout })}`);
    const first = await filesRead(stream);

    execFileSync("git", ["symbolic-ref", "refs/remotes/origin/HEAD", "refs/remotes/origin/trunk"], { cwd: checkout });
    await vi.advanceTimersByTimeAsync(BEAT_MS);

    expect(await movedFrom(stream, first.data)).toBeDefined();
    await stream.stop();
  });

  it("sends a new token at the beat after a branch Merge Assist tries for the base first exists, as after a first fetch", async () => {
    vi.useFakeTimers({ toFake: ["setInterval", "clearInterval"] });
    const checkout = gitRepository(join(realpathSync(root), "web"));
    recordCheckout(database, checkout);
    const stream = await open(`?${new URLSearchParams({ checkout })}`);
    const first = await filesRead(stream);

    execFileSync("git", ["update-ref", "refs/remotes/origin/main", "HEAD"], { cwd: checkout });
    await vi.advanceTimersByTimeAsync(BEAT_MS);

    expect(await movedFrom(stream, first.data)).toBeDefined();
    await stream.stop();
  });

  it("sends a new token at the beat after the branch origin/HEAD names first exists, though it is no usual base name", async () => {
    vi.useFakeTimers({ toFake: ["setInterval", "clearInterval"] });
    const checkout = gitRepository(join(realpathSync(root), "web"));
    execFileSync("git", ["symbolic-ref", "refs/remotes/origin/HEAD", "refs/remotes/origin/trunk"], { cwd: checkout });
    recordCheckout(database, checkout);
    const stream = await open(`?${new URLSearchParams({ checkout })}`);
    const first = await filesRead(stream);

    execFileSync("git", ["update-ref", "refs/remotes/origin/trunk", "HEAD"], { cwd: checkout });
    await vi.advanceTimersByTimeAsync(BEAT_MS);

    expect(await movedFrom(stream, first.data)).toBeDefined();
    await stream.stop();
  });

  it("sends a new token at the beat after packed-refs gains a base branch with no loose ref, as a clone or a fetch writes it", async () => {
    vi.useFakeTimers({ toFake: ["setInterval", "clearInterval"] });
    const checkout = gitRepository(join(realpathSync(root), "web"));
    execFileSync("git", ["pack-refs", "--all"], { cwd: checkout });
    recordCheckout(database, checkout);
    const stream = await open(`?${new URLSearchParams({ checkout })}`);
    const first = await filesRead(stream);

    const head = execFileSync("git", ["rev-parse", "HEAD"], { cwd: checkout, encoding: "utf8" }).trim();
    appendFileSync(join(checkout, ".git", "packed-refs"), `${head} refs/remotes/origin/main\n`);
    await vi.advanceTimersByTimeAsync(BEAT_MS);

    expect(await movedFrom(stream, first.data)).toBeDefined();
    await stream.stop();
  });

  it("sends a new token at the beat after a rule file is added to the checkout's own rules, which its catalog reads", async () => {
    vi.useFakeTimers({ toFake: ["setInterval", "clearInterval"] });
    const checkout = gitRepository(join(realpathSync(root), "web"));
    mkdirSync(join(checkout, ".rerail", "rules"), { recursive: true });
    recordCheckout(database, checkout);
    const stream = await open(`?${new URLSearchParams({ checkout })}`);
    const first = await filesRead(stream);

    writeFileSync(join(checkout, ".rerail", "rules", "local.json"), "{}\n");
    await vi.advanceTimersByTimeAsync(BEAT_MS);

    expect(await movedFrom(stream, first.data)).toBeDefined();
    await stream.stop();
  });

  it("sends a new token at the beat after the checkout's branch moved, which Merge Assist's status names", async () => {
    vi.useFakeTimers({ toFake: ["setInterval", "clearInterval"] });
    const checkout = gitRepository(join(realpathSync(root), "web"));
    recordCheckout(database, checkout);
    const stream = await open(`?${new URLSearchParams({ checkout })}`);
    const first = await filesRead(stream);

    execFileSync("git", ["switch", "-q", "-c", "feature"], { cwd: checkout });
    await vi.advanceTimersByTimeAsync(BEAT_MS);

    expect(await movedFrom(stream, first.data)).toBeDefined();
    await stream.stop();
  });

  it("starts a stream naming another spelling of a recorded checkout from what the last read of that checkout found", async () => {
    const checkout = gitRepository(join(realpathSync(root), "web"));
    recordCheckout(database, checkout);
    const stream = await open(`?${new URLSearchParams({ checkout })}`);
    const read = await filesRead(stream);
    await stream.stop();

    const again = await open(`?${new URLSearchParams({ checkout: `${checkout}/` })}`);
    expect((await again.next()).data).toBe(read.data);
    await again.stop();
  });

  describe("while Merge Assist is on", () => {
    // Merge Assist is experimental: these cases run with it on, as a daemon started with the variable does.
    withExperiments("MERGE_ASSIST");

    it("sends a new token when the daemon's Merge Assist records a hook command's result on the stores' own connection", async () => {
      const ws = await mergeWorkspace();
      // A conflict: branch `jdoe-orders` changes the line `mine` changes.
      commit(ws.worktree("jdoe-orders"), { "src/api/orders.ts": "export const orders = () => client.get(\"/v2/orders\");\n" }, "Version the orders path");
      // As the daemon makes them: one Merge Assist for the API and the sessions, writing on the stores' own connection.
      let daemonStores: DatabaseApiStores | undefined;
      const mergeAssist = createMergeAssistService({ database: () => daemonStores!.database(), github: new FakeGithub() });
      daemonStores = new DatabaseApiStores(ws.databasePath, {
        health: () => ({ pid: process.pid, database: { path: ws.databasePath, exists: true }, native: { ok: true } }),
        origin: () => "http://127.0.0.1:4737",
        changeFeed: { beatMs: BEAT_MS },
        mergeAssist,
      });
      // The hook command ends once the test makes this file, so its result is written while the stream listens.
      const release = join(ws.folder, "release-hook");
      const script = "const fs = require(\"node:fs\"); const wait = () => { if (!fs.existsSync(process.argv[1])) setTimeout(wait, 10); }; wait();";
      await mergeAssist.setEnabled(ws.repo, true);
      await mergeAssist.updateSettings(ws.repo, { hook: { command: [process.execPath, "-e", script, release], timeoutSeconds: 20 } });
      expect((await mergeAssist.scan(ws.repo)).conflicts).toHaveLength(1);
      const stream = messagesOf(await createRerailApi(daemonStores, operations, { version: "0.0.0-test" }).request("/api/changes"));
      // A wait of its own, under the case's: a commit the stream never hears fails here, not at the case's timeout.
      const within = async <T>(waiting: Promise<T>): Promise<T | string> =>
        await Promise.race([waiting, new Promise<string>(resolve => setTimeout(resolve, 2_000, "nothing within 2 s"))]);
      // The next token other than `token`: the conflict's note wait (1 s) may end before the command does and send the
      // same token again, in a `conflicts` message.
      const nextToken = async (token: string): Promise<string> => {
        let message = await stream.next();
        while (message.data === token) message = await stream.next();
        return message.data;
      };
      try {
        const first = await stream.next();

        writeFileSync(release, "");
        await mergeAssist.settled();

        expect(await within(nextToken(first.data).then(() => true, () => false))).toBe(true);
      } finally {
        await stream.stop();
        // The hook command has ended (`settled`) before its folder goes with the workspace.
        await mergeAssist.settled();
        await mergeAssist.stop();
        daemonStores.close();
      }
    });

    it("sends a new token at the beat after gh's hosts file moved, while Merge Assist is on for the checkout's GitHub repository", async () => {
      vi.useFakeTimers({ toFake: ["setInterval", "clearInterval"] });
      const checkout = gitRepository(join(realpathSync(root), "web"));
      execFileSync("git", ["remote", "add", "origin", "https://github.com/acme/web.git"], { cwd: checkout });
      const github = new FakeGithub();
      const mergeAssist = createMergeAssistService({ database, github });
      await mergeAssist.setEnabled(checkout, true);
      const signedIn = storesFor({ changeFeed: { beatMs: BEAT_MS }, mergeAssist });
      const stream = messagesOf(await createRerailApi(signedIn, operations, { version: "0.0.0-test" }).request(`/api/changes?${new URLSearchParams({ checkout })}`));
      // Each wait within a bound of its own, under the case's: a sign-in stamp the stream does not read fails here, not at
      // the case's timeout.
      const within = async <T>(waiting: Promise<T>): Promise<T | string> =>
        await Promise.race([waiting, new Promise<string>(resolve => setTimeout(resolve, 2_000, "nothing within 2 s"))]);
      try {
        // The checkout's files and gh's sign-in, each read once at the start.
        const read = await within(partsRead(stream, 2));
        expect(read).not.toBe("nothing within 2 s");

        // `gh auth logout` rewrites the hosts file, so its stamp moves.
        github.options.signInStamp = "after gh auth logout";
        await vi.advanceTimersByTimeAsync(BEAT_MS);

        expect(await within(movedFrom(stream, (read as Message).data))).not.toBe("nothing within 2 s");
      } finally {
        await stream.stop();
        signedIn.close();
        await mergeAssist.stop();
      }
    });

    it("reads the checkout's files at each beat while a stat of gh's hosts file never answers", async () => {
      vi.useFakeTimers({ toFake: ["setInterval", "clearInterval"] });
      const checkout = gitRepository(join(realpathSync(root), "web"));
      mkdirSync(join(checkout, ".rerail", "rules"), { recursive: true });
      recordCheckout(database, checkout);
      // Only the sign-in stamp is read here, and it never answers, as on a home folder whose share dropped.
      const mergeAssist = { signInStamp: () => new Promise<never>(() => undefined) } as unknown as MergeAssistService;
      const stalled = storesFor({ changeFeed: { beatMs: BEAT_MS }, mergeAssist });
      const stream = messagesOf(await createRerailApi(stalled, operations, { version: "0.0.0-test" }).request(`/api/changes?${new URLSearchParams({ checkout })}`));
      // Each wait within a bound of its own, under the case's: files read behind the stalled stat fail here, not at the
      // case's timeout.
      const within = async <T>(waiting: Promise<T>): Promise<T | string> =>
        await Promise.race([waiting, new Promise<string>(resolve => setTimeout(resolve, 2_000, "nothing within 2 s"))]);
      try {
        const first = await within(filesRead(stream));
        expect(first).not.toBe("nothing within 2 s");

        writeFileSync(join(checkout, ".rerail", "rules", "local.json"), "{}\n");
        await vi.advanceTimersByTimeAsync(BEAT_MS);

        expect(await within(movedFrom(stream, (first as Message).data))).not.toBe("nothing within 2 s");
      } finally {
        await stream.stop();
        stalled.close();
      }
    });
  });

  it("reads nothing of Merge Assist's at the stream's start or its beats while its experiment is off", async () => {
    vi.useFakeTimers({ toFake: ["setInterval", "clearInterval"] });
    const checkout = gitRepository(join(realpathSync(root), "web"));
    recordCheckout(database, checkout);
    const signInStamp = vi.fn(async () => "hosts.yml as first written");
    const mergeAssist = { signInStamp } as unknown as MergeAssistService;
    const off = storesFor({ changeFeed: { beatMs: BEAT_MS }, mergeAssist });
    const stream = messagesOf(await createRerailApi(off, operations, { version: "0.0.0-test" }).request(`/api/changes?${new URLSearchParams({ checkout })}`));
    try {
      const first = await filesRead(stream);
      await vi.advanceTimersByTimeAsync(BEAT_MS);
      expect(await stream.next()).toEqual(first);

      expect(signInStamp).not.toHaveBeenCalled();
    } finally {
      await stream.stop();
      off.close();
    }
  });

  it("stops listening when the client leaves before reading anything, with the read of the checkout's files still out", async () => {
    vi.useFakeTimers({ toFake: ["setInterval", "clearInterval"] });
    const checkout = gitRepository(join(realpathSync(root), "web"));
    recordCheckout(database, checkout);
    const response = await app.request(`/api/changes?${new URLSearchParams({ checkout })}`);

    await response.body!.cancel();

    // Nothing listens any more, so the beat is gone, though the stream's read of the checkout's files may end after.
    expect(vi.getTimerCount()).toBe(0);
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
