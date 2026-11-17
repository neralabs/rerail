import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { RerailDaemon } from "../../src/daemon/index.js";
import { addRemote, machineId, SqliteSessionStore } from "../../src/runtime/index.js";
import { startFakeSink, storedRecords, type FakeSink } from "../support/fake-sink.js";
import { removeFolderSync } from "../support/platform/remove-folder.js";
import { withExperiments } from "../support/environment/experiments.js";

withExperiments("REMOTE_SINK");

const cleanups: Array<() => Promise<void> | void> = [];
afterEach(async () => {
  for (const cleanup of cleanups.splice(0).reverse()) await cleanup();
});

async function until(condition: () => boolean, timeoutMs = 15_000): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  while (!condition()) {
    if (Date.now() > deadline) throw new Error("timed out");
    await new Promise(resolve => setTimeout(resolve, 50));
  }
}

describe("the daemon's remote sink", () => {
  it("ships a session recorded while it runs, and catches up after the remote was down", async () => {
    const home = mkdtempSync(join(tmpdir(), "rerail-remote-daemon-"));
    cleanups.push(() => removeFolderSync(home));
    const sink: FakeSink = await startFakeSink();
    cleanups.push(() => sink.close());
    const databasePath = join(home, "rerail.sqlite");
    const daemon = await RerailDaemon.start({ version: "test", databasePath, workspaceRoot: home, port: 0, remoteSink: { intervalMs: 100 } });
    cleanups.push(() => daemon.close());

    // A session the daemon's stores record, as a coding agent's hooks would, while no remote is added: nothing goes out.
    const store = new SqliteSessionStore(databasePath, { repositoryId: "remote:example.com/team/app" });
    cleanups.push(() => store.close());
    store.openSession({ host: "claude-code", sessionId: "s-1", cwd: home }, Date.now());
    store.append({ sessionId: "s-1", eventId: "e-1", kind: "user-prompt", observedAt: Date.now(), payload: { text: "hello" } });
    await new Promise(resolve => setTimeout(resolve, 300));
    expect(sink.requests).toHaveLength(0);

    addRemote({ url: sink.url, token: "test-token", content: true }, home);
    await until(() => storedRecords(sink).some(item => item.type === "event"));
    const machine = machineId(home);
    expect(storedRecords(sink)).toEqual(expect.arrayContaining([
      expect.objectContaining({ type: "session", id: "s-1", host: "claude-code" }),
      expect.objectContaining({ type: "event", id: `${machine}:1`, payload: { text: "hello" } }),
    ]));

    // The remote goes down; the session goes on; nothing is lost when it comes back.
    sink.respondWith(503);
    store.append({ sessionId: "s-1", eventId: "e-2", kind: "user-prompt", observedAt: Date.now(), payload: { text: "while down" } });
    store.closeSession("s-1", Date.now());
    await until(() => sink.requests.some(request => request.records.some(item => item.id === `${machine}:2`)));
    sink.respondWith(200);
    // The first retry waits about 2 s.
    await until(() => storedRecords(sink).some(item => item.id === `${machine}:2`));
    expect(storedRecords(sink).filter(item => item.type === "session").at(-1)).toMatchObject({ id: "s-1", endedAt: expect.any(Number) });
  }, 30_000);

  it("sends nothing to a remote added earlier while the remote sink is off in the daemon, and says so in /healthz", async () => {
    vi.stubEnv("RERAIL_EXPERIMENTAL_REMOTE_SINK", "0");
    const home = mkdtempSync(join(tmpdir(), "rerail-remote-daemon-off-"));
    cleanups.push(() => removeFolderSync(home));
    const sink: FakeSink = await startFakeSink();
    cleanups.push(() => sink.close());
    addRemote({ url: sink.url, token: "test-token", content: true }, home);
    const databasePath = join(home, "rerail.sqlite");
    const daemon = await RerailDaemon.start({ version: "test", databasePath, workspaceRoot: home, port: 0, remoteSink: { intervalMs: 100 } });
    cleanups.push(() => daemon.close());
    expect(daemon.health().experiments).not.toContain("REMOTE_SINK");

    const store = new SqliteSessionStore(databasePath, { repositoryId: "remote:example.com/team/app" });
    cleanups.push(() => store.close());
    store.openSession({ host: "claude-code", sessionId: "s-1", cwd: home }, Date.now());
    store.append({ sessionId: "s-1", eventId: "e-1", kind: "user-prompt", observedAt: Date.now(), payload: { text: "hello" } });
    await new Promise(resolve => setTimeout(resolve, 600));
    expect(sink.requests).toHaveLength(0);
  });
});
