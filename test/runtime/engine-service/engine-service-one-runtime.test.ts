import { cp, mkdir, mkdtemp, realpath } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import Database from "better-sqlite3";
import { afterEach, describe, expect, it } from "vitest";
import { SqliteAuditControlStore, type AdapterEvent } from "../../../src/engine/index.js";
import { ingestEngineEvent, startEngineService, type EngineRuntimeConfiguration, type EngineService } from "../../../src/runtime/engine-service.js";
import { testEmbedder } from "../../support/hook-events.js";
import { startTestRepository } from "../../support/started-repository.js";
import { removeFolder } from "../../support/platform/remove-folder.js";

const roots: string[] = [];
const services: EngineService[] = [];
afterEach(async () => {
  await Promise.all(services.splice(0).map(service => service.close()));
  await Promise.all(roots.splice(0).map(root => removeFolder(root)));
});

/**
 * An engine service over two started checkouts, `first` and `second`, whose sessions are suspended after a minute of
 * `clock` without an event and whose daemon keeps every runtime no session uses. Its hooks' configuration (`configured`)
 * names the checkout's database, the service's storage root and the bundled model, as a hook of one Rerail version
 * does; `otherModel` is a copy of that model in a folder of its own, as another version's package holds one.
 */
async function service() {
  // Resolved: on macOS the run folder is under `/tmp`, a link, and the service records the paths it resolves.
  const root = await realpath(await mkdtemp(join(tmpdir(), "rerail-one-runtime-")));
  roots.push(root);
  const databasePath = join(root, "rerail.sqlite");
  const [first, second] = [join(root, "first"), join(root, "second")];
  for (const checkout of [first, second]) {
    await mkdir(checkout);
    startTestRepository(checkout, databasePath);
  }
  const otherModel = join(root, "other-model");
  await cp(resolve("model"), otherModel, { recursive: true });
  const socketPath = process.platform === "win32" ? `\\\\.\\pipe\\rerail-one-runtime-${process.pid}-${Date.now()}` : join(root, "engine.sock");
  const clock = { now: 1_000_000 };
  const engine = await startEngineService({
    socketPath, storageRoot: join(root, "state"), databasePath, embedder: testEmbedder, idleSessionMs: 60_000, now: () => clock.now, needsRoom: () => false,
  });
  services.push(engine);
  const configured = (change: Partial<EngineRuntimeConfiguration> = {}): EngineRuntimeConfiguration =>
    ({ storageRoot: join(root, "state"), modelPath: resolve("model"), databasePath, workspaceRoot: first, ...change });
  const session = (sessionId: string, cwd: string) => ({ host: "claude-code" as const, sessionId, cwd });
  const send = (event: AdapterEvent, configuration: EngineRuntimeConfiguration) => ingestEngineEvent({ socketPath }, event, configuration);
  /** `sessionId`'s start (`name` tells a second one apart), prompt `text` or end, sent with `configuration`, in the checkout it names. */
  const start = (sessionId: string, configuration: EngineRuntimeConfiguration, name = "start") =>
    send({ kind: "session-start", eventId: `${sessionId}:${name}`, session: session(sessionId, configuration.workspaceRoot!) }, configuration);
  const prompt = (sessionId: string, text: string, configuration: EngineRuntimeConfiguration) =>
    send({ kind: "user-prompt", eventId: `${sessionId}:${text}`, sessionId, session: session(sessionId, configuration.workspaceRoot!), text }, configuration);
  const end = (sessionId: string, configuration: EngineRuntimeConfiguration) =>
    send({ kind: "session-end", eventId: `${sessionId}:end`, sessionId, session: session(sessionId, configuration.workspaceRoot!) }, configuration);
  return { root, databasePath, first, second, otherModel, clock, engine, configured, start, prompt, end };
}

/** The ids of the events `databasePath` records for `sessionId`, in the order they were recorded. */
function recorded(databasePath: string, sessionId: string): string[] {
  const database = new Database(databasePath, { readonly: true });
  try {
    const rows = database.prepare("SELECT event_id AS eventId FROM events WHERE session_id = ? ORDER BY sequence").all(sessionId) as Array<{ eventId: string }>;
    return rows.map(row => row.eventId);
  } finally {
    database.close();
  }
}

/** Whether a live runtime owns `sessionId` in `databasePath`. */
function ownedLive(databasePath: string, sessionId: string): boolean {
  const controls = new SqliteAuditControlStore(databasePath);
  try {
    return controls.getSettings(sessionId).active;
  } finally {
    controls.close();
  }
}

describe("one session's runtime in the engine service", () => {
  // An event a hook left untaken while the daemon did not read names the model folder of the Rerail version that hook
  // ran; checked after an update, beside the hooks of the new version, it opened the session in a runtime of its own.
  // The next runtime's claim on the session was refused while the first held it, and every later event of the session
  // went unrecorded: `Rerail session <id> is already owned by a live runtime`. `RERAIL_MODEL_PATH` set for some of a
  // session's hooks did the same, and so did `RERAIL_STORAGE_ROOT`.
  it.each([
    { setting: "model folder", other: (p: Awaited<ReturnType<typeof service>>) => p.configured({ modelPath: p.otherModel }) },
    { setting: "storage root", other: (p: Awaited<ReturnType<typeof service>>) => p.configured({ storageRoot: join(p.root, "other-state") }) },
  ])("records every event of a session whose hooks name another $setting, in the one runtime it opened in", async ({ other }) => {
    const p = await service();
    await p.start("s", p.configured());
    await p.prompt("s", "one", other(p));
    await p.prompt("s", "two", p.configured());
    await p.prompt("s", "three", other(p));
    expect(recorded(p.databasePath, "s")).toEqual(["s:start", "s:one", "s:two", "s:three"]);
    expect(p.engine.openRuntimes()).toBe(1);
  }, 30_000);

  it("opens another session whose hooks name another model folder in that folder's runtime, beside the one a session holds", async () => {
    const p = await service();
    await p.start("s", p.configured());
    await p.start("t", p.configured({ modelPath: p.otherModel }));
    await p.prompt("s", "one", p.configured({ modelPath: p.otherModel }));
    expect([recorded(p.databasePath, "s"), recorded(p.databasePath, "t")]).toEqual([["s:start", "s:one"], ["t:start"]]);
    expect(p.engine.openRuntimes()).toBe(2);
  }, 30_000);

  // Another checkout over the same database is another runtime's, and the session's claim there is refused while its
  // own runtime holds it, as before: what a model folder or a storage root decides, a checkout does not.
  it("refuses a session's event that names another checkout over the same database, as before", async () => {
    const p = await service();
    await p.start("s", p.configured());
    await expect(p.prompt("s", "elsewhere", p.configured({ workspaceRoot: p.second }))).rejects.toThrow("Rerail session s is already owned by a live runtime");
    expect(recorded(p.databasePath, "s")).toEqual(["s:start"]);
  }, 30_000);

  it("records a session's event that names another database in a runtime of that database, as before", async () => {
    const p = await service();
    const otherDatabase = join(p.root, "other", "rerail.sqlite");
    const otherCheckout = join(p.root, "other", "repo");
    await mkdir(otherCheckout, { recursive: true });
    startTestRepository(otherCheckout, otherDatabase);
    await p.start("s", p.configured());
    await p.prompt("s", "elsewhere", p.configured({ databasePath: otherDatabase, workspaceRoot: otherCheckout }));
    expect([recorded(p.databasePath, "s"), recorded(otherDatabase, "s")]).toEqual([["s:start"], ["s:elsewhere"]]);
    expect(p.engine.openRuntimes()).toBe(2);
  }, 30_000);

  it("opens a session ended in one runtime and started again under another model folder in that folder's runtime, as a new session opens", async () => {
    const p = await service();
    await p.start("s", p.configured());
    await p.end("s", p.configured());
    const again = p.configured({ modelPath: p.otherModel });
    await p.start("s", again, "start-again");
    await p.prompt("s", "back", again);
    expect(recorded(p.databasePath, "s")).toEqual(["s:start", "s:end", "s:start-again", "s:back"]);
    expect([p.engine.openRuntimes(), ownedLive(p.databasePath, "s")]).toEqual([2, true]);
  }, 30_000);

  // A quiet session's runtime let it go: its next event opens it again where that event's configuration says.
  it("opens a quiet session again under the model folder its next event names, once the idle close let it go", async () => {
    const p = await service();
    await p.start("s", p.configured());
    p.clock.now += 60_000;
    await p.engine.closeIdleSessions();
    await p.prompt("s", "back", p.configured({ modelPath: p.otherModel }));
    expect(recorded(p.databasePath, "s")).toEqual(["s:start", "s:back"]);
    expect([p.engine.openRuntimes(), ownedLive(p.databasePath, "s")]).toEqual([2, true]);
  }, 30_000);
});
