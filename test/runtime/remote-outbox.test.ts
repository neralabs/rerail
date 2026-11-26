import { createHash, randomUUID } from "node:crypto";
import { existsSync, linkSync, mkdtempSync, readFileSync, renameSync, rmSync, statSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import Database from "better-sqlite3";
import { afterEach, beforeEach, describe, expect, it, onTestFinished, vi } from "vitest";
import { runRemoteCommand, seedRerailDatabase } from "../../src/cli/index.js";
import { SqliteAuditReader, SqliteAuditStore } from "../../src/engine/index.js";
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

/** Writes `text` over an audit's stored paths, past the CHECK on `audits`, as a hand edit or a damaged file leaves it. */
function setPathsText(auditId: string, text: string): void {
  const database = new Database(databasePath());
  database.pragma("ignore_check_constraints = ON");
  database.prepare("UPDATE audits SET paths_json = ? WHERE audit_id = ?").run(text, auditId);
  database.close();
}

/** Writes `text` over the stored rule provenance copy an audit names, which every audit naming it reads, past its CHECK. */
function setProvenanceCopyText(auditId: string, text: string): void {
  const database = new Database(databasePath());
  database.pragma("ignore_check_constraints = ON");
  database.prepare("UPDATE audit_rule_provenances SET rule_provenance_json = ? WHERE provenance_id = (SELECT rule_provenance_id FROM audits WHERE audit_id = ?)").run(text, auditId);
  database.close();
}

/** `count` more finished audits of the session, each with no violation. */
function addAudits(sessionId: string, count: number): string[] {
  const audits = new SqliteAuditStore(databasePath());
  const auditIds = Array.from({ length: count }, () => {
    const auditId = audits.start({ sessionId, workspaceRoot: "/work/app", scope: "repository", mode: "log", revision: 1, catalogRevision: "c1", ruleRevisions: [], paths: [] });
    audits.finish(auditId, { status: "completed" });
    return auditId;
  });
  audits.close();
  return auditIds;
}

/** The hash the outbox keeps for an audit as sent, and the audit's readable and marked hashes (`auditHash`). */
function auditHashes(auditId: string): { kept: unknown; readable: string; marked: string } {
  const rerail = new Database(databasePath(), { readonly: true });
  const row = rerail.prepare("SELECT status, finished_at, length(violations_json) AS size FROM audits WHERE audit_id = ?").get(auditId) as { status: string; finished_at: number; size: number };
  rerail.close();
  const state = new Database(join(home, "remote-outbox.sqlite"), { readonly: true });
  const kept = state.prepare("SELECT hash FROM sent WHERE stream = 'audit' AND id = ?").pluck().get(auditId);
  state.close();
  const hash = (...mark: string[]): string => createHash("sha256").update(JSON.stringify([row.status, row.finished_at, row.size, ...mark])).digest("hex");
  return { kept, readable: hash(), marked: hash("unreadable") };
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

  // A scan's violations stay out of the dashboard's history, not out of a remote's: its audit says it is a scan.
  it("sends a scan's audit with its origin, and its violations keyed as an ongoing audit's", async () => {
    const { sessionId, auditId } = seed();
    const audits = new SqliteAuditStore(databasePath());
    const violation = {
      id: "v1", ruleId: "no-todo", clauseId: "c", code: "TODO", mode: "steer", message: "No TODO", path: "a.ts",
      span: { startByte: 0, endByte: 4, startLine: 0, startColumn: 0, endLine: 0, endColumn: 4 }, captures: {}, variables: {},
    } as const;
    const scanId = audits.start({ sessionId, workspaceRoot: "/work/app", scope: "repository", mode: "log", origin: "scan", revision: 1, catalogRevision: "c1", ruleRevisions: [{ ruleId: "no-todo", revision: 1 }], paths: [] });
    // The same violation twice: only its first copy is a finding.
    audits.finish(scanId, { status: "completed", violations: [violation, { ...violation, id: "v2" }] });
    audits.close();
    addRemote({ url: sink.url, token: "test-token", content: true }, home);
    await outbox().sync({ force: true });
    const records = sink.requests.flatMap(request => request.records);
    const of = (type: string, id: string) => records.filter(item => item.type === type && String(item.id).startsWith(id));
    expect([...of("audit", auditId), ...of("audit", scanId)].map(item => item.origin)).toEqual(["ongoing", "scan"]);
    const [ongoing, scanned] = [of("violation", auditId), of("violation", scanId)];
    expect([ongoing.map(item => [item.id, item.logged]), scanned.map(item => [item.id, item.logged])])
      .toEqual([[[`${auditId}:0`, true]], [[`${scanId}:0`, true]]]);
    expect(scanned[0]!.violationKey).toBe(ongoing[0]!.violationKey);
  });

  // The update that adds `origin` marks the scans a database already holds, some of them sent already.
  it("sends again an audit marked a scan after it was sent, and no audit that stays ongoing", async () => {
    const { sessionId, auditId } = seed();
    const audits = new SqliteAuditStore(databasePath());
    const otherId = audits.start({ sessionId, workspaceRoot: "/work/app", scope: "touched-files", mode: "steer", revision: 1, catalogRevision: "c1", ruleRevisions: [], paths: ["b.ts"] });
    audits.finish(otherId, { status: "completed", violations: [] });
    audits.close();
    addRemote({ url: sink.url, token: "test-token", content: true }, home);
    await outbox().sync({ force: true });
    expect(sink.requests).toHaveLength(1);

    const database = new Database(databasePath());
    database.prepare("UPDATE audits SET origin = 'scan' WHERE audit_id = ?").run(auditId);
    database.close();
    await outbox().sync({ force: true });
    expect(sink.requests).toHaveLength(2);
    // Its violation is keyed and logged as before, so the remote holds it already.
    expect(sink.requests[1]!.records.map(item => [item.type, item.id, item.origin])).toEqual([["audit", auditId, "scan"]]);
  });

  it("names this machine by the remote's --label when it has one", async () => {
    seed();
    addRemote({ url: sink.url, token: "test-token", content: true, label: "Alex's laptop" }, home);
    await outbox().sync({ force: true });
    expect(sink.requests[0]!.header.machineLabel).toBe("Alex's laptop");
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

  it("sends an audit whose stored paths do not read with its violations and a failure that names them, as no error", async () => {
    const { auditId } = seed();
    setPathsText(auditId, '["a.ts"');
    addRemote({ url: sink.url, token: "test-token", content: true }, home);
    const [result] = await outbox().sync({ force: true });
    expect(result!.error).toBeUndefined();
    expect(outbox().status()[0]).toMatchObject({ lastError: null, pendingEvents: 0 });
    const records = storedRecords(sink);
    expect(records.find(item => item.type === "audit")).toMatchObject({
      id: auditId, paths: [], failures: [expect.objectContaining({ clauseId: "paths", code: "adapter-error" })], failureCount: 1,
    });
    expect(records.filter(item => item.type === "violation").map(item => item.id)).toEqual([`${auditId}:0`]);
  });

  it("sends an audit it sent with stored paths that did not read again, whole, once they are mended", async () => {
    const { auditId } = seed();
    setPathsText(auditId, '["a.ts"');
    addRemote({ url: sink.url, token: "test-token", content: true }, home);
    await outbox().sync({ force: true });
    setPathsText(auditId, '["a.ts"]');
    await outbox().sync({ force: true });
    expect(sink.requests).toHaveLength(2);
    const records = sink.requests[1]!.records;
    expect(records.find(item => item.type === "audit")).toMatchObject({ id: auditId, paths: ["a.ts"], failures: [], failureCount: 0 });
  });

  it("reads no audit it sent with a stored list that did not read again while that list stays as it is", async () => {
    const { auditId } = seed();
    setPathsText(auditId, '["a.ts"');
    addRemote({ url: sink.url, token: "test-token", content: true }, home);
    await outbox().sync({ force: true });
    const reads = vi.spyOn(SqliteAuditReader.prototype, "get");
    onTestFinished(() => reads.mockRestore());
    await outbox().sync({ force: true });
    expect(reads).not.toHaveBeenCalled();
    expect(sink.requests).toHaveLength(1);
  });

  it("marks a readable audit sent under the hash earlier versions sent it under, and sends it no more", async () => {
    const { auditId } = seed();
    addRemote({ url: sink.url, token: "test-token", content: true }, home);
    await outbox().sync({ force: true });
    const rerail = new Database(databasePath(), { readonly: true });
    const row = rerail.prepare("SELECT status, finished_at, length(violations_json) AS size FROM audits WHERE audit_id = ?").get(auditId) as { status: string; finished_at: number; size: number };
    rerail.close();
    const state = new Database(join(home, "remote-outbox.sqlite"), { readonly: true });
    const sent = state.prepare("SELECT hash FROM sent WHERE stream = 'audit' AND id = ?").pluck().get(auditId);
    state.close();
    // The hash `main` computes for an ongoing audit (`auditHash`): an audit it sent counts as sent here.
    expect(sent).toBe(createHash("sha256").update(JSON.stringify([row.status, row.finished_at, row.size])).digest("hex"));
    await outbox().sync({ force: true });
    expect(sink.requests).toHaveLength(1);
  });

  it("sends each audit naming a damaged stored rule provenance once with a failure that names it, and again whole once it is mended", async () => {
    const { sessionId, auditId } = seed();
    const audits = new SqliteAuditStore(databasePath());
    const other = audits.start({ sessionId, workspaceRoot: "/work/app", scope: "repository", mode: "log", revision: 1, catalogRevision: "c1", ruleRevisions: [], paths: [] });
    audits.finish(other, { status: "completed" });
    audits.close();
    const sentAudits = (request: number): unknown[] => sink.requests[request]!.records.filter(item => item.type === "audit")
      .map(item => [item.id, (item.failures as Array<{ clauseId: string }>).map(failure => failure.clauseId)]).sort();
    setProvenanceCopyText(auditId, "[");
    addRemote({ url: sink.url, token: "test-token", content: true }, home);
    await outbox().sync({ force: true });
    await outbox().sync({ force: true });
    expect(sink.requests).toHaveLength(1);
    expect(sentAudits(0)).toEqual([[auditId, ["rule-provenance"]], [other, ["rule-provenance"]]].sort());
    setProvenanceCopyText(auditId, "[]");
    await outbox().sync({ force: true });
    expect(sink.requests).toHaveLength(2);
    expect(sentAudits(1)).toEqual([[auditId, []], [other, []]].sort());
  });

  it("sends an audit it sent whole no more once a stored list of it is damaged", async () => {
    const { auditId } = seed();
    addRemote({ url: sink.url, token: "test-token", content: true }, home);
    await outbox().sync({ force: true });
    setPathsText(auditId, '["a.ts"');
    await outbox().sync({ force: true });
    expect(sink.requests).toHaveLength(1);
  });

  it("pushes with exit 0 while an audit whose stored paths do not read waits", async () => {
    const { auditId } = seed();
    setPathsText(auditId, '["a.ts"');
    addRemote({ url: sink.url, token: "test-token", content: true }, home);
    let output = "";
    const stdout = { write: (text: string) => { output += text; return true; } };
    expect(await runRemoteCommand(["push"], { home, stdout })).toBe(0);
    expect(output).not.toContain("failed");
    expect(storedRecords(sink).filter(item => item.type === "audit").map(item => item.id)).toEqual([auditId]);
  });

  it("tests no stored list of an audit it never sent, over a sync of many batches", async () => {
    const { sessionId } = seed();
    addAudits(sessionId, 20);
    addRemote({ url: sink.url, token: "test-token", content: true }, home);
    const tests = vi.spyOn(SqliteAuditReader.prototype, "readableAudits");
    onTestFinished(() => tests.mockRestore());
    const [result] = await outbox().sync({ force: true, batchBytes: 4 * 1024 });
    expect(result!.batches).toBeGreaterThan(1);
    expect(tests).not.toHaveBeenCalled();
    expect(storedRecords(sink).filter(item => item.type === "audit")).toHaveLength(21);
  });

  it("marks the hash it keeps for an audit by the record it sent, not by a test of the stored lists", async () => {
    const { sessionId, auditId } = seed();
    // A failure of the audit's own leaves its hash readable.
    const audits = new SqliteAuditStore(databasePath());
    const intact = audits.start({ sessionId, workspaceRoot: "/work/app", scope: "repository", mode: "log", revision: 1, catalogRevision: "c1", ruleRevisions: [], paths: [] });
    audits.finish(intact, { status: "completed", failures: [{ clauseId: "c", code: "adapter-error", stage: "persistence", message: "The audit's own failure." }] });
    audits.close();
    setPathsText(auditId, '["a.ts"');
    // The test of the stored lists says the opposite of each record.
    const tests = vi.spyOn(SqliteAuditReader.prototype, "readableAudits").mockImplementation(ids => new Set(ids.filter(id => id === auditId)));
    onTestFinished(() => tests.mockRestore());
    addRemote({ url: sink.url, token: "test-token", content: true }, home);
    await outbox().sync({ force: true });
    const damaged = auditHashes(auditId);
    expect(damaged.kept).toBe(damaged.marked);
    const whole = auditHashes(intact);
    expect(whole.kept).toBe(whole.readable);
  });

  it("marks the hash it keeps for an audit by the stage, code and clause id of the failure that names a stored list, whatever its message says", async () => {
    const { auditId } = seed();
    setPathsText(auditId, '["a.ts"');
    // The record says what a later Rerail may word otherwise.
    const read = SqliteAuditReader.prototype.get;
    const reads = vi.spyOn(SqliteAuditReader.prototype, "get").mockImplementation(function (this: SqliteAuditReader, id: string) {
      const audit = read.call(this, id);
      return audit && { ...audit, failures: audit.failures.map(failure => ({ ...failure, message: "The audit's paths are damaged." })) };
    });
    onTestFinished(() => reads.mockRestore());
    addRemote({ url: sink.url, token: "test-token", content: true }, home);
    await outbox().sync({ force: true });
    expect(storedRecords(sink).find(item => item.type === "audit")).toMatchObject({ failures: [{ clauseId: "paths", message: "The audit's paths are damaged." }] });
    const hashes = auditHashes(auditId);
    expect(hashes.kept).toBe(hashes.marked);
  });

  it("tests the stored lists of an audit it sent with one that did not read once a sync, over many batches", async () => {
    const { sessionId, auditId } = seed();
    setPathsText(auditId, '["a.ts"');
    addRemote({ url: sink.url, token: "test-token", content: true }, home);
    await outbox().sync({ force: true });
    addAudits(sessionId, 20);
    const tests = vi.spyOn(SqliteAuditReader.prototype, "readableAudits");
    onTestFinished(() => tests.mockRestore());
    const [result] = await outbox().sync({ force: true, batchBytes: 4 * 1024 });
    expect(result!.batches).toBeGreaterThan(1);
    expect(tests.mock.calls).toEqual([[[auditId]]]);
  });

  it("tests the stored lists of the audits it sent naming a damaged stored rule provenance in one statement a sync, however many, and reads none", async () => {
    const { sessionId, auditId } = seed();
    addAudits(sessionId, 20);
    setProvenanceCopyText(auditId, "[");
    addRemote({ url: sink.url, token: "test-token", content: true }, home);
    await outbox().sync({ force: true });
    const prepared = vi.spyOn(Database.prototype, "prepare");
    const reads = vi.spyOn(SqliteAuditReader.prototype, "get");
    onTestFinished(() => {
      prepared.mockRestore();
      reads.mockRestore();
    });
    await outbox().sync({ force: true });
    // Each list test in SQL (`holdsList`) begins with `json_valid`; no other statement of a sync has it.
    expect(prepared.mock.calls.filter(([source]) => source.includes("json_valid"))).toHaveLength(1);
    expect(reads).not.toHaveBeenCalled();
    expect(sink.requests).toHaveLength(1);
  });

  it("sends an audit holding its own rule provenance, as an older Rerail wrote it, again whole once that is mended", async () => {
    const { auditId } = seed();
    const setOwnProvenanceText = (text: string): void => {
      const database = new Database(databasePath());
      database.pragma("ignore_check_constraints = ON");
      database.prepare("UPDATE audits SET rule_provenance_id = NULL, rule_provenance_json = ? WHERE audit_id = ?").run(text, auditId);
      database.close();
    };
    setOwnProvenanceText("[");
    addRemote({ url: sink.url, token: "test-token", content: true }, home);
    await outbox().sync({ force: true });
    await outbox().sync({ force: true });
    expect(sink.requests).toHaveLength(1);
    setOwnProvenanceText("[]");
    await outbox().sync({ force: true });
    expect(sink.requests).toHaveLength(2);
    expect(sink.requests[1]!.records.find(item => item.type === "audit")).toMatchObject({ id: auditId, failures: [], failureCount: 0 });
  });

  it("sends a scan's audit whose stored violations do not read with a failure that names them, and again whole once they are mended", async () => {
    const { sessionId } = seed();
    const audits = new SqliteAuditStore(databasePath());
    const scanId = audits.start({ sessionId, workspaceRoot: "/work/app", scope: "repository", mode: "log", origin: "scan", revision: 1, catalogRevision: "c1", ruleRevisions: [{ ruleId: "no-todo", revision: 1 }], paths: [] });
    audits.finish(scanId, {
      status: "completed",
      violations: [{
        id: "v1", ruleId: "no-todo", clauseId: "c", code: "TODO", mode: "log", message: "No TODO", path: "a.ts",
        span: { startByte: 0, endByte: 4, startLine: 0, startColumn: 0, endLine: 0, endColumn: 4 }, captures: {}, variables: {},
      }],
    });
    audits.close();
    // Writes what `edit` makes of the scan's stored violations past the CHECK. The history triggers parse the violations
    // on UPDATE, so they are set aside for the write and put back: the state a damaged file leaves.
    const editViolations = (edit: (text: string) => string): void => {
      const database = new Database(databasePath());
      database.pragma("ignore_check_constraints = ON");
      const triggers = database.prepare("SELECT name, sql FROM sqlite_master WHERE type = 'trigger' AND tbl_name = 'audits' AND sql LIKE '%violations_json%'").all() as Array<{ name: string; sql: string }>;
      database.transaction(() => {
        const text = database.prepare("SELECT violations_json FROM audits WHERE audit_id = ?").pluck().get(scanId) as string;
        for (const trigger of triggers) database.exec(`DROP TRIGGER "${trigger.name}"`);
        database.prepare("UPDATE audits SET violations_json = ? WHERE audit_id = ?").run(edit(text), scanId);
        for (const trigger of triggers) database.exec(trigger.sql);
      })();
      database.close();
    };
    const sentOfScan = (request: number): unknown[] => sink.requests[request]!.records.filter(item => String(item.id).startsWith(scanId))
      .map(item => [item.type, item.id, ...(item.type === "audit" ? [(item.failures as Array<{ clauseId: string }>).map(failure => failure.clauseId)] : [])]);
    // Its closing bracket made a space and its length kept, so only the test of its lists tells the outbox it is mended.
    editViolations(text => `${text.slice(0, -1)} `);
    addRemote({ url: sink.url, token: "test-token", content: true }, home);
    const [result] = await outbox().sync({ force: true });
    expect(result!.error).toBeUndefined();
    expect(sentOfScan(0)).toEqual([["audit", scanId, ["violations"]]]);
    await outbox().sync({ force: true });
    expect(sink.requests).toHaveLength(1);
    editViolations(text => `${text.slice(0, -1)}]`);
    await outbox().sync({ force: true });
    expect(sink.requests).toHaveLength(2);
    expect(sentOfScan(1)).toEqual([["audit", scanId, []], ["violation", `${scanId}:0`]]);
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

  /** An audit the repository owns (a change in another worktree no session made), of a checkout of {@link REPOSITORY}. */
  function seedRepositoryAudit(sessionId: string): string {
    const database = new Database(databasePath());
    database.prepare("INSERT INTO checkouts (checkout_path, repository_key, clone_id, created_at, last_seen_at) VALUES ('/work/w', ?, 'c', 1, 1)").run(REPOSITORY);
    database.close();
    const audits = new SqliteAuditStore(databasePath());
    const auditId = audits.start({
      sessionId, owner: "repository", workspaceRoot: "/work/w", scope: "ai-output", mode: "log", revision: 1, catalogRevision: "c1",
      ruleRevisions: [{ ruleId: "no-todo", revision: 1 }], paths: ["a.ts"],
    });
    audits.finish(auditId, {
      status: "completed",
      violations: [{
        id: "v1", ruleId: "no-todo", clauseId: "c", code: "TODO", mode: "steer", message: "No TODO", path: "a.ts",
        span: { startByte: 0, endByte: 4, startLine: 0, startColumn: 0, endLine: 0, endColumn: 4 }, captures: {}, variables: {},
      }],
    });
    audits.close();
    return auditId;
  }

  it("sends an audit the repository owns with no session, naming its repository, and its violations with no session", async () => {
    const { sessionId, auditId: sessions } = seed();
    const owned = seedRepositoryAudit(sessionId);
    addRemote({ url: sink.url, token: "test-token", content: true }, home);
    await outbox().sync({ force: true });
    const records = storedRecords(sink);
    expect(records.filter(item => item.type === "audit").map(item => [item.id, item.sessionId, item.repositoryKey]))
      .toEqual(expect.arrayContaining([[sessions, sessionId, null], [owned, null, REPOSITORY]]));
    expect(records.find(item => item.type === "violation" && item.auditId === owned)).toMatchObject({ sessionId: null, logged: true });
  });

  it("holds back an audit the repository owns while that repository is excluded", async () => {
    const { sessionId } = seed();
    const owned = seedRepositoryAudit(sessionId);
    addRemote({ url: sink.url, token: "test-token", content: true }, home);
    setRepositoryExcluded(REPOSITORY, true, home);
    await outbox().sync({ force: true });
    expect(storedRecords(sink).filter(item => item.id === owned || item.auditId === owned)).toEqual([]);
  });

  // An audit with no session is told by its own repository: a test of its session would read as a repository not
  // known, which any exclusion covers.
  it("sends an audit the repository owns while only another repository is excluded", async () => {
    const { sessionId } = seed();
    const owned = seedRepositoryAudit(sessionId);
    addRemote({ url: sink.url, token: "test-token", content: true }, home);
    setRepositoryExcluded("remote:example.com/team/other", true, home);
    await outbox().sync({ force: true });
    expect(storedRecords(sink).filter(item => item.type === "audit").map(item => item.id)).toContain(owned);
  });

  it("sends an audit the repository owns whose stored paths do not read with no session, its repository and a failure that names them, and again whole once they are mended", async () => {
    const { sessionId } = seed();
    const owned = seedRepositoryAudit(sessionId);
    setPathsText(owned, '["a.ts"');
    addRemote({ url: sink.url, token: "test-token", content: true }, home);
    await outbox().sync({ force: true });
    setPathsText(owned, '["a.ts"]');
    await outbox().sync({ force: true });
    const sent = (request: number): unknown[] => sink.requests[request]!.records.filter(item => item.type === "audit" && item.id === owned);
    expect(sent(0)).toEqual([expect.objectContaining({
      sessionId: null, repositoryKey: REPOSITORY, paths: [], failures: [expect.objectContaining({ clauseId: "paths", code: "adapter-error" })],
    })]);
    expect(sent(1)).toEqual([expect.objectContaining({ sessionId: null, repositoryKey: REPOSITORY, paths: ["a.ts"], failures: [] })]);
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
    const acl = { restrictToOwner: (path: string, folder: boolean): void => { restricted.push([path, folder]); }, otherReaders: (): string[] => [], inherits: () => Promise.resolve(true), restrictFolderToOwner: () => Promise.resolve(),
      writers: () => Promise.resolve([]), inheritedWriters: () => Promise.resolve([]), keepEachOwnersOwn: () => Promise.resolve(), letAddFiles: () => Promise.resolve() };
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

describe("event numbers a remote has taken", () => {
  const sent = () => sink.requests.flatMap(request => request.records).filter(record => record.type === "event").map(record => record.eventId);
  const session = { host: "claude-code" as const, sessionId: "fresh", cwd: "/work/app" };
  const sequenceOf = (path: string, eventId: string): number => {
    const database = new Database(path, { readonly: true });
    try {
      return (database.prepare("SELECT sequence FROM events WHERE event_id = ?").get(eventId) as { sequence: number }).sequence;
    } finally {
      database.close();
    }
  };
  /** Moves the home's database aside, as Rerail's refusal of an unreadable file tells a person to: the outbox stays. */
  const moveAside = (): void => {
    for (const suffix of ["", "-wal", "-shm"]) if (real.existsSync(`${databasePath()}${suffix}`)) real.renameSync(`${databasePath()}${suffix}`, `${databasePath()}.aside${suffix}`);
  };
  /** The store as a power cut that took the commits of every event leaves it: their rows gone, the counter back. */
  const cutEvents = (): void => {
    const database = new Database(databasePath());
    try {
      database.prepare("DELETE FROM events").run();
      database.prepare("UPDATE sqlite_sequence SET seq = 0 WHERE name = 'events'").run();
    } finally {
      database.close();
    }
  };

  // Its first events took the numbers the remote's cursor had passed, and were never sent.
  it("numbers the first event of a database started fresh past the remote's cursor, so the outbox sends it", async () => {
    seed();
    addRemote({ url: sink.url, token: "test-token", content: true }, home);
    await outbox().sync({ force: true });
    moveAside();
    const store = new SqliteSessionStore(databasePath());
    store.openSession(session);
    store.append({ sessionId: "fresh", eventId: "first", kind: "user-prompt", payload: { text: "again" } }, session);
    store.close();
    await outbox().sync({ force: true });
    expect(sent()).toContain("first");
  });

  // The remote took the batch while `rerail remote remove` ran, and the removal deleted the batch's record with the rest:
  // the event after the cut took number 1 and went to the remote added again under the id of the event it held as 1.
  it("keeps a batch's event numbers taken when its remote is removed while the batch is out", async () => {
    seed();
    const quiet = { stdout: { write: () => true } };
    addRemote({ url: sink.url, token: "test-token", content: true, name: "team" }, home);
    sink.duringNextRequest(() => void runRemoteCommand(["remove", "team"], { home, ...quiet }));
    await outbox().sync({ force: true });
    const held = storedRecords(sink).filter(record => record.type === "event").map(record => record.id);
    cutEvents();
    addRemote({ url: sink.url, token: "test-token", content: true, name: "team" }, home);
    const store = new SqliteSessionStore(databasePath());
    store.openSession(session);
    store.append({ sessionId: "fresh", eventId: "first", kind: "user-prompt", payload: { text: "again" } }, session);
    store.close();
    await outbox().sync({ force: true });
    const first = storedRecords(sink).find(record => record.type === "event" && record.eventId === "first");
    expect(held).toHaveLength(4);
    expect(first?.id).toEqual(expect.any(String));
    expect(held).not.toContain(first?.id);
  });

  // An audit sent marked, its stored paths unreadable, in a batch out while its remote is removed and added again: the
  // removal forgets the hash the audit is held under, as it forgets every sent record's, and keeps the batch's numbers.
  it("sends a remote added again while a batch was out an audit whose stored paths do not read once marked, whole once mended, and later events past the batch's numbers", async () => {
    const { auditId } = seed();
    setPathsText(auditId, '["a.ts"');
    const quiet = { stdout: { write: () => true } };
    addRemote({ url: sink.url, token: "test-token", content: true, name: "team" }, home);
    sink.duringNextRequest(() => {
      void runRemoteCommand(["remove", "team"], { home, ...quiet });
      addRemote({ url: sink.url, token: "test-token", content: true, name: "team" }, home);
    });
    await outbox().sync({ force: true });
    const held = storedRecords(sink).filter(record => record.type === "event").map(record => record.id);
    cutEvents();
    const store = new SqliteSessionStore(databasePath());
    store.openSession(session);
    store.append({ sessionId: "fresh", eventId: "first", kind: "user-prompt", payload: { text: "again" } }, session);
    store.close();
    await outbox().sync({ force: true });
    await outbox().sync({ force: true });
    setPathsText(auditId, '["a.ts"]');
    await outbox().sync({ force: true });
    const audits = sink.requests.map(request => request.records.filter(item => item.type === "audit").map(item => item.paths));
    expect(audits).toEqual([[[]], [[]], [["a.ts"]]]);
    const first = storedRecords(sink).find(record => record.type === "event" && record.eventId === "first");
    expect(held).toHaveLength(4);
    expect(first?.id).toEqual(expect.any(String));
    expect(held).not.toContain(first?.id);
  });

  // A hook's steer receipt opens a writer of its own and no session (`hook-host.ts`), and its session opened in the
  // daemon first, which moved the numbers: the receipt's writer opened the outbox at every receipt all the same.
  it("reads no outbox for a writer that opens no session, as a hook's steer receipt", async () => {
    seed();
    addRemote({ url: sink.url, token: "test-token", content: true }, home);
    await outbox().sync({ force: true });
    moveAside();
    vi.mocked(existsSync).mockClear();
    const store = new SqliteSessionStore(databasePath());
    store.recordSteer(session, "r1", ["a-rule"]);
    store.close();
    expect(vi.mocked(existsSync).mock.calls.filter(([path]) => String(path).endsWith("remote-outbox.sqlite"))).toEqual([]);
    expect(sequenceOf(databasePath(), "steering-delivered:r1")).toBe(1);
  });

  it("reads the outbox at a store's first session open only", async () => {
    seed();
    addRemote({ url: sink.url, token: "test-token", content: true }, home);
    await outbox().sync({ force: true });
    const store = new SqliteSessionStore(databasePath());
    store.openSession(session);
    vi.mocked(existsSync).mockClear();
    store.openSession({ ...session, sessionId: "second" });
    store.close();
    expect(vi.mocked(existsSync).mock.calls.filter(([path]) => String(path).endsWith("remote-outbox.sqlite"))).toEqual([]);
  });

  // A batch with no event (a queued scan) has no number to keep taken: its sync makes no commit that waits for the disk.
  it("records no event number for a batch that carries no event", async () => {
    addRemote({ url: sink.url, token: "test-token", content: true }, home);
    const report = { schema: "rerail.scan/1", scanId: "scan-1", repository: { key: REPOSITORY, displayName: "app" }, checkout: { path: home }, rules: [], violations: [] };
    expect(outbox().queueScan(report)).toBe(true);
    await outbox().sync({ force: true });
    expect(storedRecords(sink)).toEqual([expect.objectContaining({ type: "scan", id: "scan-1" })]);
    const state = new Database(join(home, "remote-outbox.sqlite"), { readonly: true });
    try {
      expect(state.prepare("SELECT value FROM cursors WHERE stream = 'event-posted'").all()).toEqual([]);
    } finally {
      state.close();
    }
  });

  it("leaves the numbering of a database the outbox does not read as it is", async () => {
    seed();
    addRemote({ url: sink.url, token: "test-token", content: true }, home);
    await outbox().sync({ force: true });
    const other = join(home, "other.sqlite");
    const store = new SqliteSessionStore(other);
    store.openSession(session);
    store.append({ sessionId: "fresh", eventId: "first", kind: "user-prompt", payload: { text: "elsewhere" } }, session);
    store.close();
    expect(sequenceOf(other, "first")).toBe(1);
  });

  it("numbers events as before beside an outbox it cannot read", () => {
    real.writeFileSync(join(home, "remote-outbox.sqlite"), "not a database");
    const store = new SqliteSessionStore(databasePath());
    store.openSession(session);
    store.append({ sessionId: "fresh", eventId: "first", kind: "user-prompt", payload: { text: "hello" } }, session);
    store.close();
    expect(sequenceOf(databasePath(), "first")).toBe(1);
  });
});
