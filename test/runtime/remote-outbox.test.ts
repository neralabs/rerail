import { randomUUID } from "node:crypto";
import { linkSync, mkdtempSync, readFileSync, renameSync, rmSync, statSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import Database from "better-sqlite3";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { runRemoteCommand, seedRerailDatabase } from "../../src/cli/index.js";
import { SqliteAuditStore } from "../../src/engine/index.js";
import { addRemote, machineId, readRemoteConfig, setRepositoryExcluded } from "../../src/runtime/remote-config.js";
import { backoffMs, MAX_BACKOFF_MS, RemoteOutbox } from "../../src/runtime/remote-outbox.js";
import { SqliteSessionStore } from "../../src/runtime/session-store.js";
import { startFakeSink, storedRecords, type FakeSink } from "../support/fake-sink.js";
import { removeFolderSync } from "../support/platform/remove-folder.js";
import { withExperiments } from "../support/environment/experiments.js";

withExperiments("REMOTE_SINK");

vi.mock("node:fs", { spy: true });
const real = await vi.importActual<typeof import("node:fs")>("node:fs");

const REPOSITORY = "remote:example.com/team/app";
let home: string;
let sink: FakeSink;
let clock: number;
const outbox = (): RemoteOutbox => new RemoteOutbox({ home, now: () => clock });
const databasePath = (): string => join(home, "rerail.sqlite");

beforeEach(async () => {
  home = mkdtempSync(join(tmpdir(), "rerail-remote-home-"));
  sink = await startFakeSink();
  clock = 1_000_000;
});
afterEach(async () => {
  vi.mocked(writeFileSync).mockReset();
  vi.mocked(linkSync).mockReset();
  vi.mocked(renameSync).mockReset();
  await sink.close();
  removeFolderSync(home);
});

/** One finished session of four events (`seedRerailDatabase`) in {@link REPOSITORY}, and a finished audit with a violation. */
function seed(): { sessionId: string; auditId: string } {
  seedRerailDatabase(databasePath(), { repositoryId: REPOSITORY, cwd: "/work/app", now: 500_000 });
  const database = new Database(databasePath());
  database.prepare("INSERT INTO repositories (repository_key, display_name, created_at, updated_at) VALUES (?, 'app', 1, 1)").run(REPOSITORY);
  const { session_id: sessionId } = database.prepare("SELECT session_id FROM sessions").get() as { session_id: string };
  database.close();
  const audits = new SqliteAuditStore(databasePath());
  const auditId = audits.start({ sessionId, workspaceRoot: "/work/app", scope: "repository", mode: "log", revision: 1, catalogRevision: "c1", ruleRevisions: [{ ruleId: "no-todo", revision: 1 }], paths: [] });
  audits.finish(auditId, {
    status: "completed",
    violations: [{
      id: "v1", ruleId: "no-todo", clauseId: "c", code: "TODO", mode: "log", message: "No TODO", path: "a.ts",
      span: { startByte: 0, endByte: 4, startLine: 0, startColumn: 0, endLine: 0, endColumn: 4 }, captures: {}, variables: {},
    }],
  });
  audits.close();
  return { sessionId, auditId };
}

function addEvent(sessionId: string, eventId: string): void {
  const store = new SqliteSessionStore(databasePath(), { repositoryId: REPOSITORY });
  store.append({ sessionId, eventId, kind: "user-prompt", observedAt: 600_000, payload: { text: "secret plans" } });
  store.close();
}

describe("remote sink outbox", () => {
  it("sends nothing and makes no request while no remote is added", async () => {
    seed();
    expect(await outbox().sync({ force: true })).toEqual([]);
    expect(sink.requests).toHaveLength(0);
    expect(readRemoteConfig(home).remotes).toEqual([]);
  });

  it("ships repositories, sessions, events, audits and violations as NDJSON per the sink protocol", async () => {
    const { sessionId, auditId } = seed();
    addRemote({ url: sink.url, token: "test-token", content: true }, home);
    const [result] = await outbox().sync({ force: true });
    expect(result).toMatchObject({ batches: 1 });
    expect(result!.error).toBeUndefined();
    const [request] = sink.requests;
    expect(request!.authorization).toBe("Bearer test-token");
    expect(request!.contentType).toBe("application/x-ndjson");
    expect(request!.header).toMatchObject({ type: "batch", protocol: 1, machineId: machineId(home), sentAt: clock });
    const records = request!.records;
    const machine = machineId(home);
    expect(records.find(item => item.type === "repository")).toMatchObject({ id: REPOSITORY, displayName: "app", aliases: [] });
    expect(records.find(item => item.type === "session")).toMatchObject({ id: sessionId, host: "omp", cwd: "/work/app", repositoryKey: REPOSITORY, endedAt: expect.any(Number) });
    const events = records.filter(item => item.type === "event");
    expect(events.map(item => item.id)).toEqual([1, 2, 3, 4].map(sequence => `${machine}:${sequence}`));
    expect(events[1]).toMatchObject({ sessionId, kind: "user-prompt", payload: { text: "Add a users page that calls /api/users." } });
    expect(records.find(item => item.type === "audit")).toMatchObject({ id: auditId, status: "completed", scope: "repository" });
    expect(records.find(item => item.type === "violation")).toMatchObject({
      id: `${auditId}:0`, auditId, ruleId: "no-todo", path: "a.ts", violationKey: expect.any(String),
      // 1-based with an exclusive end, as a scan reports; `span` keeps the engine's 0-based one.
      start: { line: 1, column: 1 }, end: { line: 1, column: 5 }, span: { startLine: 0, startColumn: 0, endColumn: 4 },
    });
    // Dependencies first: what a record refers to arrives before it or in the same batch.
    const order = records.map(item => item.type);
    expect(order.indexOf("session")).toBeLessThan(order.indexOf("event"));
    expect(order.indexOf("audit")).toBeLessThan(order.indexOf("violation"));
  });

  it("names this machine by the remote's --label when it has one", async () => {
    seed();
    addRemote({ url: sink.url, token: "test-token", content: true, label: "Jeff's laptop" }, home);
    await outbox().sync({ force: true });
    expect(sink.requests[0]!.header.machineLabel).toBe("Jeff's laptop");
    expect(() => addRemote({ url: sink.url, token: "t", content: true, name: "other", label: "bad\nlabel" }, home)).toThrow(/control characters/u);
  });

  it("sends only what changed since the last acknowledged batch", async () => {
    const { sessionId } = seed();
    addRemote({ url: sink.url, token: "test-token", content: true }, home);
    await outbox().sync({ force: true });
    await outbox().sync({ force: true });
    expect(sink.requests).toHaveLength(1);
    addEvent(sessionId, "later");
    await outbox().sync({ force: true });
    expect(sink.requests).toHaveLength(2);
    expect(sink.requests[1]!.records.map(item => item.type)).toEqual(["event"]);
    expect(outbox().status()[0]).toMatchObject({ pendingEvents: 0, sentBatches: 2, lastError: null });
  });

  it("keeps the cursor on failure, backs off, and resends the same batch id once the remote is back", async () => {
    seed();
    addRemote({ url: sink.url, token: "test-token", content: true }, home);
    sink.respondWith(503);
    const [failed] = await outbox().sync();
    expect(failed!.error).toMatch(/503/u);
    const status = outbox().status()[0]!;
    expect(status).toMatchObject({ consecutiveFailures: 1, pendingEvents: 4, lastSuccessAt: null });
    expect(status.nextAttemptAt).toBeGreaterThan(clock);
    expect((await outbox().sync())[0]).toMatchObject({ skipped: "backoff" });

    sink.respondWith(200);
    clock = status.nextAttemptAt! + 1;
    await outbox().sync();
    expect(sink.requests[1]!.header.batchId).toBe(sink.requests[0]!.header.batchId);
    expect(sink.stored.size).toBe(1);
    expect(outbox().status()[0]).toMatchObject({ consecutiveFailures: 0, pendingEvents: 0, lastError: null });
  });

  it("waits the longest backoff on a refused token, and tries a new token at once", async () => {
    seed();
    addRemote({ url: sink.url, token: "wrong", content: true, name: "team" }, home);
    expect((await outbox().sync())[0]!.error).toMatch(/unauthorized/u);
    expect(outbox().status()[0]!.nextAttemptAt).toBe(clock + MAX_BACKOFF_MS);
    expect(await runRemoteCommand(["remove", "team"], { home, stdout: { write: () => true } })).toBe(0);
    addRemote({ url: sink.url, token: "test-token", content: true, name: "team" }, home);
    expect((await outbox().sync())[0]).toMatchObject({ batches: 1 });
  });

  it("splits into batches within the size limit and still delivers every event once", async () => {
    const { sessionId } = seed();
    for (let index = 0; index < 40; index += 1) addEvent(sessionId, `bulk-${index}`);
    addRemote({ url: sink.url, token: "test-token", content: true }, home);
    const budget = 4 * 1024;
    const [result] = await outbox().sync({ force: true, batchBytes: budget });
    expect(result!.batches).toBeGreaterThan(1);
    for (const request of sink.requests) expect(request.bytes).toBeLessThanOrEqual(budget);
    const ids = storedRecords(sink).filter(item => item.type === "event").map(item => item.id);
    expect(ids).toHaveLength(44);
    expect(new Set(ids).size).toBe(44);
  });

  it("omits event content for a --no-content remote", async () => {
    seed();
    addRemote({ url: sink.url, token: "test-token", content: false }, home);
    await outbox().sync({ force: true });
    const events = storedRecords(sink).filter(item => item.type === "event");
    expect(events.every(item => item.content === "omitted" && !("payload" in item))).toBe(true);
  });

  it("sends nothing of an excluded repository", async () => {
    seed();
    addRemote({ url: sink.url, token: "test-token", content: true }, home);
    setRepositoryExcluded(REPOSITORY, true, home);
    await outbox().sync({ force: true });
    expect(storedRecords(sink).filter(item => item.type !== "gap")).toEqual([]);
    expect(outbox().status()[0]!.pendingEvents).toBe(0);
  });

  it("excludes a repository under every key it has had, and a session placed in it only by its folder", async () => {
    seed();
    const database = new Database(databasePath());
    // The repository was known by a clone id before it had a remote: sessions from then carry the old key.
    database.prepare("UPDATE repositories SET aliases_json = ? WHERE repository_key = ?").run(JSON.stringify(["clone:old"]), REPOSITORY);
    database.prepare("INSERT INTO checkouts (checkout_path, repository_key, clone_id, created_at, last_seen_at) VALUES ('/work/app', ?, 'c', 1, 1)").run(REPOSITORY);
    database.prepare("INSERT INTO sessions (session_id, host, cwd, repository_id, started_at) VALUES ('old-key', 'omp', '/elsewhere', 'clone:old', 1)").run();
    database.prepare("INSERT INTO sessions (session_id, host, cwd, repository_id, started_at) VALUES ('by-folder', 'omp', '/work/app/sub', NULL, 1)").run();
    database.prepare("INSERT INTO sessions (session_id, host, cwd, repository_id, started_at) VALUES ('unplaced', 'omp', '/nowhere', NULL, 1)").run();
    database.close();
    addRemote({ url: sink.url, token: "test-token", content: true }, home);
    setRepositoryExcluded(REPOSITORY, true, home);
    await outbox().sync({ force: true });
    // A session no checkout places is held back while any repository is excluded.
    expect(storedRecords(sink).filter(item => item.type === "session")).toEqual([]);
    setRepositoryExcluded(REPOSITORY, false, home, 450_000);
    const later = new Database(databasePath());
    later.prepare("INSERT INTO sessions (session_id, host, cwd, repository_id, started_at) VALUES ('unplaced-later', 'omp', '/nowhere', NULL, 460000)").run();
    later.close();
    await outbox().sync({ force: true });
    expect(storedRecords(sink).filter(item => item.type === "session").map(item => item.id)).toEqual(["unplaced-later"]);
  });

  it("never sends what a repository recorded while excluded, even when no sync ran until after include", async () => {
    const { sessionId } = seed();
    addRemote({ url: sink.url, token: "test-token", content: true }, home);
    setRepositoryExcluded(REPOSITORY, true, home);
    addEvent(sessionId, "while-excluded");
    setRepositoryExcluded(REPOSITORY, false, home, 650_000);
    const store = new SqliteSessionStore(databasePath(), { repositoryId: REPOSITORY });
    store.openSession({ host: "omp", sessionId: "after", cwd: "/work/app" }, 700_000);
    store.append({ sessionId: "after", eventId: "a1", kind: "user-prompt", observedAt: 700_001, payload: { text: "new" } });
    store.close();
    await outbox().sync({ force: true });
    const stored = storedRecords(sink);
    expect(stored.filter(item => item.type === "session").map(item => item.id)).toEqual(["after"]);
    expect(stored.filter(item => item.type === "event").map(item => item.eventId)).toEqual(["a1"]);
    expect(stored.filter(item => item.type === "audit")).toEqual([]);
    expect(outbox().status()[0]!.pendingEvents).toBe(0);
  });

  it("lets a sync that was running when its remote was removed write nothing, so the remote added again starts from nothing", async () => {
    seed();
    const quiet = { stdout: { write: () => true } };
    addRemote({ url: sink.url, token: "test-token", content: true, name: "team" }, home);
    sink.duringNextRequest(() => {
      void runRemoteCommand(["remove", "team"], { home, ...quiet });
      addRemote({ url: sink.url, token: "test-token", content: true, name: "team" }, home);
    });
    await outbox().sync({ force: true });
    expect(sink.requests).toHaveLength(1);
    await outbox().sync({ force: true });
    expect(sink.requests).toHaveLength(2);
    expect(sink.requests[1]!.records.filter(item => item.type === "event")).toHaveLength(4);
    expect(outbox().status()[0]).toMatchObject({ sentBatches: 1, pendingEvents: 0 });
  });

  it("reports events deleted before they were sent as a gap", async () => {
    seed();
    const database = new Database(databasePath());
    database.prepare("DELETE FROM events WHERE sequence IN (2, 3)").run();
    database.close();
    addRemote({ url: sink.url, token: "test-token", content: true }, home);
    await outbox().sync({ force: true });
    const records = storedRecords(sink);
    expect(records.find(item => item.type === "gap")).toMatchObject({ stream: "event", fromSequence: 2, toSequence: 3 });
    expect(records.filter(item => item.type === "event")).toHaveLength(2);
  });

  it("narrows a refused batch down to the one record the remote refuses, skips only it, and reports it", async () => {
    seed();
    addRemote({ url: sink.url, token: "test-token", content: true }, home);
    const machine = machineId(home);
    sink.refuseWhen(records => records.some(item => item.id === `${machine}:3`));
    const [result] = await outbox().sync({ force: true });
    expect(result!.error).toBeUndefined();
    const stored = storedRecords(sink);
    expect(stored.filter(item => item.type === "event").map(item => item.id)).toEqual([1, 2, 4].map(sequence => `${machine}:${sequence}`));
    expect(stored.find(item => item.type === "gap")).toMatchObject({ stream: "batch", fromSequence: 3, toSequence: 3, reason: expect.stringMatching(/400/u) });
  });

  it("stops and backs off when the remote refuses record after record, instead of emptying the queue", async () => {
    seed();
    addRemote({ url: sink.url, token: "test-token", content: true }, home);
    sink.respondWith(400);
    const [result] = await outbox().sync({ force: true });
    expect(result!.error).toMatch(/400/u);
    expect(outbox().status()[0]).toMatchObject({ consecutiveFailures: 1 });
    expect(outbox().status()[0]!.pendingEvents).toBeGreaterThan(0);
  });

  it("starts a remote added again under a removed one's name from nothing", async () => {
    seed();
    const stdout = { write: () => true };
    addRemote({ url: sink.url, token: "test-token", content: true, name: "team" }, home);
    await outbox().sync({ force: true });
    await runRemoteCommand(["remove", "team"], { home, stdout });
    addRemote({ url: sink.url, token: "test-token", content: true, name: "team" }, home);
    await outbox().sync({ force: true });
    expect(sink.requests).toHaveLength(2);
    expect(sink.requests[1]!.records.filter(item => item.type === "event")).toHaveLength(4);
  });

  it("queues a headless scan for every remote and sends it as a scan record", async () => {
    addRemote({ url: sink.url, token: "test-token", content: true }, home);
    const report = { schema: "rerail.scan/1", scanId: "scan-1", repository: { key: REPOSITORY, displayName: "app" }, checkout: { path: home }, rules: [], violations: [] };
    expect(outbox().queueScan(report)).toBe(true);
    await outbox().sync({ force: true });
    expect(storedRecords(sink)).toEqual([expect.objectContaining({ type: "scan", id: "scan-1", schema: "rerail.scan/1" })]);
    // Every remote has it: the outbox lets it go.
    const state = new Database(join(home, "remote-outbox.sqlite"), { readonly: true });
    expect(state.prepare("SELECT count(*) AS n FROM scans").get()).toEqual({ n: 0 });
    state.close();
    setRepositoryExcluded(REPOSITORY, true, home);
    expect(outbox().queueScan({ ...report, scanId: "scan-2" })).toBe(false);
  });

  it("keeps the token, configuration and machine id owner-only, and never prints the token", async () => {
    const tokenFile = join(home, "token.txt");
    writeFileSync(tokenFile, "s3cret-token\n");
    let output = "";
    const stdout = { write: (text: string) => { output += text; return true; } };
    await runRemoteCommand(["add", sink.url, "--token-file", tokenFile, "--name", "team"], { home, stdout });
    await runRemoteCommand(["list"], { home, stdout });
    await runRemoteCommand(["status"], { home, stdout });
    expect(output).not.toContain("s3cret");
    const id = machineId(home);
    expect(machineId(home)).toBe(id);
    if (process.platform !== "win32") {
      for (const file of ["remote.json", "remote-tokens/team", "machine-id"]) expect(statSync(join(home, file)).mode & 0o777).toBe(0o600);
      expect(statSync(join(home, "remote-tokens")).mode & 0o777).toBe(0o700);
    }
    await expect(runRemoteCommand(["add", "http://example.com", "--token-file", tokenFile], { home, stdout })).rejects.toThrow(/https/u);
  });

  it("gives the token folder an owner-only ACL on Windows on every add, in a home an earlier release made", () => {
    // The home is already there, so nothing gives it an ACL now: an earlier release made it with its parent's, under C:\.
    const restricted: Array<[string, boolean]> = [];
    const acl = { restrictToOwner: (path: string, folder: boolean): void => { restricted.push([path, folder]); }, otherReaders: (): string[] => [] };
    const windows = { acl, platform: "win32" as const, profile: String.raw`C:\Users\nobody-here` };
    addRemote({ url: "https://rerail.example", token: "t", content: true, name: "team" }, home, clock, windows);
    expect(restricted).toEqual([[join(home, "remote-tokens"), true]]);
    // A tokens folder an earlier release made is fixed too: each add sets the ACL again, which its files inherit.
    addRemote({ url: "https://rerail.example", token: "t", content: true, name: "other" }, home, clock, windows);
    expect(restricted).toEqual([[join(home, "remote-tokens"), true], [join(home, "remote-tokens"), true]]);
  });

  it("backs off exponentially within bounds", () => {
    expect(backoffMs(1, () => 0.5)).toBe(2_000);
    expect(backoffMs(3, () => 0.5)).toBe(8_000);
    expect(backoffMs(30, () => 1)).toBe(MAX_BACKOFF_MS);
  });
});

describe("machineId", () => {
  const ID = /^[0-9a-f-]{36}$/u;

  /**
   * Has another process ask for the machine id the moment the machine-id file first appears in the home, however it is
   * made: created and then written, linked in, or renamed in. Returns what that process got.
   */
  function askedWhenMade(): string[] {
    const file = join(home, "machine-id");
    const got: string[] = [];
    let asked = false;
    const ask = (): void => {
      asked = true;
      got.push(machineId(home));
    };
    vi.mocked(writeFileSync).mockImplementation((path, data, options) => {
      if (path !== file || asked) return real.writeFileSync(path, data, options);
      // Created empty, as an exclusive create leaves it, then written.
      real.writeFileSync(path, "", options);
      ask();
      real.writeFileSync(path, data, { ...(typeof options === "object" ? options : {}), flag: "w" });
    });
    vi.mocked(linkSync).mockImplementation((from, to) => {
      real.linkSync(from, to);
      if (to === file && !asked) ask();
    });
    vi.mocked(renameSync).mockImplementation((from, to) => {
      real.renameSync(from, to);
      if (to === file && !asked) ask();
    });
    return got;
  }

  it("gives another process asking the moment the id file appears the whole id, not an empty one", () => {
    const askedMeanwhile = askedWhenMade();

    const id = machineId(home);

    expect(id).toMatch(ID);
    expect(askedMeanwhile).toEqual([id]);
  });

  it("returns the id of the process that made the file first when it loses the race to make it", () => {
    const winner = randomUUID();
    vi.mocked(linkSync).mockImplementationOnce((from, to) => {
      const theirs = join(home, "their.tmp");
      real.writeFileSync(theirs, `${winner}\n`);
      real.linkSync(theirs, to);
      real.rmSync(theirs);
      real.linkSync(from, to);
    });

    expect(machineId(home)).toBe(winner);
    expect(machineId(home)).toBe(winner);
  });

  it("replaces a file that holds no id, as one left empty by a process that died making it, and keeps the new id", () => {
    writeFileSync(join(home, "machine-id"), "");

    const id = machineId(home);

    expect(id).toMatch(ID);
    expect(readFileSync(join(home, "machine-id"), "utf8").trim()).toBe(id);
    expect(machineId(home)).toBe(id);
  });

  it("leaves no temporary file behind", () => {
    machineId(home);
    rmSync(join(home, "machine-id"));
    writeFileSync(join(home, "machine-id"), "not an id\n");
    machineId(home);

    expect(real.readdirSync(home).filter(name => name.startsWith("machine-id"))).toEqual(["machine-id"]);
  });
});
