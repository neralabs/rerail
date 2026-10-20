import { realpathSync } from "node:fs";
import { chmod, mkdtemp, mkdir, rm, stat } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import type { ExtensionAPI, ExtensionContext } from "@oh-my-pi/pi-coding-agent";
import Database from "better-sqlite3";
import { is } from "drizzle-orm";
import { getTableConfig, SQLiteTable } from "drizzle-orm/sqlite-core";
import { afterEach, describe, expect, it } from "vitest";
import { createConversationRulesExtension } from "../../src/hosts/omp/index.js";
import { RerailRuntime } from "../../src/runtime/runtime.js";
import {
  defaultRerailDatabasePath,
  openRerailDatabase,
  resetRepositoryHistory,
  loadRuleCatalog,
  addAttachment,
  removeAttachment,
  ruleConfigurationSignature,
  setRuleSetting,
  listLibraryRules,
  upsertLibraryRule,
} from "../../src/engine/index.js";
import { SqlitePresentationPreferenceStore } from "../../src/runtime/presentation-preference-store.js";
import { MOCK_CONVERSATION_RULES, MOCK_CODE_RULES } from "../../src/cli/index.js";
import { decodeEventPayload, SqliteSessionReader, SqliteSessionStore } from "../../src/runtime/session-store.js";
import { NativeSessionClient } from "../../src/harness/index.js";
import { schema } from "../../src/engine/index.js";
import { testEmbedder } from "../support/hook-events.js";
import { closeNativeTestDaemons, startNativeTestDaemon } from "../support/native-daemon.js";
import { startTestRepository } from "../support/started-repository.js";
import { setUpTestRepository } from "../support/repository-rules.js";

const temporaryDirectories: string[] = [];
const temporaryConnections: Array<{ close(): void }> = [];

const originalRerailHome = process.env.RERAIL_HOME;

afterEach(async () => {
  await closeNativeTestDaemons();
  if (originalRerailHome === undefined) delete process.env.RERAIL_HOME;
  else process.env.RERAIL_HOME = originalRerailHome;
  await Promise.all(temporaryConnections.splice(0).reverse().map(async connection => connection.close()));
  await Promise.all(temporaryDirectories.splice(0).map(path => rm(path, { recursive: true, force: true })));
});

async function temporaryDirectory(): Promise<string> {
  const path = await mkdtemp(join(tmpdir(), "rerail-transcripts-"));
  process.env.RERAIL_HOME = join(path, "home");
  temporaryDirectories.push(path);
  startTestRepository(path);
  return path;
}

function closeAfterTest<T extends { close(): void }>(connection: T): T {
  temporaryConnections.push(connection);
  return connection;
}

async function hudFixture(sessionId: string) {
  const root = await temporaryDirectory();
  const path = defaultRerailDatabasePath();
  const repositoryId = "hud-repository";
  const store = closeAfterTest(new SqliteSessionStore(path, { repositoryId }));
  const reader = closeAfterTest(new SqliteSessionReader(path));
  const session = { host: "omp" as const, sessionId, cwd: "/workspace" };
  return { root, path, repositoryId, store, reader, session };
}

describe("SQLite session persistence", () => {
  it("stores ordered full-fidelity events idempotently in a private database", async () => {
    const root = await temporaryDirectory();
    const path = join(root, "private", "rerail.sqlite");
    const store = new SqliteSessionStore(path);
    store.openSession({ host: "omp", sessionId: "session-1", cwd: "/workspace" }, 100);
    store.append({ sessionId: "session-1", eventId: "event-1", kind: "user-prompt", observedAt: 110, payload: { text: "hello" } });
    store.append({ sessionId: "session-1", eventId: "event-1", kind: "user-prompt", observedAt: 120, payload: { text: "duplicate" } });
    store.append({ sessionId: "session-1", eventId: "event-2", kind: "assistant-message", observedAt: 130, payload: { text: "world" } });
    const circular: { count: bigint; self?: unknown } = { count: 1n };
    circular.self = circular;
    store.append({ sessionId: "session-1", eventId: "event-3", kind: "result", observedAt: 135, payload: circular });
    store.closeSession("session-1", 140);
    expect(store.sessions()).toEqual([{
      repositoryId: null,
      sessionId: "session-1",
      host: "omp",
      cwd: "/workspace",
      startedAt: 100,
      endedAt: 140,
      eventCount: 3,
    }]);
    store.close();

    expect((await stat(path)).mode & 0o777).toBe(0o600);
    expect((await stat(join(root, "private"))).mode & 0o777).toBe(0o700);
    const database = new Database(path, { readonly: true });
    expect(database.prepare("SELECT kind, observed_at AS observedAt, payload_json AS payloadJson FROM events WHERE event_id != 'event-3' ORDER BY sequence").all()).toEqual([
      { kind: "user-prompt", observedAt: 110, payloadJson: JSON.stringify({ text: "hello" }) },
      { kind: "assistant-message", observedAt: 130, payloadJson: JSON.stringify({ text: "world" }) },
    ]);
    const encoded = database.prepare("SELECT payload_json AS payloadJson, payload_v8 AS payloadV8 FROM events WHERE event_id = 'event-3'").get() as {
      payloadJson: string;
      payloadV8: Buffer;
    };
    expect(JSON.parse(encoded.payloadJson)).toEqual({ encoding: "node:v8" });
    const decoded = decodeEventPayload(encoded.payloadV8) as { count: bigint; self: unknown };
    expect(decoded.count).toBe(1n);
    expect(decoded.self).toBe(decoded);
    database.close();
  });

  it("creates a private session store directory without changing caller-owned parent permissions", async () => {
    const root = await temporaryDirectory();
    const callerDirectory = join(root, "configured");
    const databaseDirectory = join(callerDirectory, "transcripts");
    const path = join(databaseDirectory, "events.sqlite");
    await mkdir(callerDirectory, { recursive: true });
    await chmod(callerDirectory, 0o755);

    const store = new SqliteSessionStore(path);
    store.close();

    expect((await stat(callerDirectory)).mode & 0o777).toBe(0o755);
    expect((await stat(databaseDirectory)).mode & 0o777).toBe(0o700);
    expect((await stat(path)).mode & 0o777).toBe(0o600);
  });

  it("captures the complete normalized MCP event stream as it is ingested", async () => {
    const root = await temporaryDirectory();
    const databasePath = defaultRerailDatabasePath();
    const runtime = new RerailRuntime({
      storageRoot: join(root, "sessions"),
      databasePath,
      workspaceRoot: root,
      embedder: testEmbedder,
    });
    const session = { host: "claude-code" as const, sessionId: "live-session", cwd: root };
    await runtime.ingest({ kind: "session-start", eventId: "start", session });
    await runtime.ingest({ kind: "user-prompt", eventId: "user", sessionId: session.sessionId, text: "inspect this" });
    await runtime.ingest({ kind: "assistant-message", eventId: "assistant", sessionId: session.sessionId, messageId: "message-1", text: "checking" });
    await runtime.ingest({ kind: "call", eventId: "call", toolCallId: "call-1", sessionId: session.sessionId, toolName: "read", input: { path: "src/a.ts" } });
    await runtime.ingest({ kind: "result", eventId: "result", toolCallId: "call-1", sessionId: session.sessionId, toolName: "read", output: { text: "contents" } });
    await runtime.ingest({ kind: "turn-end", eventId: "turn", sessionId: session.sessionId, text: "done" });
    await runtime.ingest({ kind: "session-end", eventId: "end", sessionId: session.sessionId });
    await runtime.dispose();

    const database = new Database(databasePath, { readonly: true });
    const events = database.prepare("SELECT event_id AS eventId, kind, payload_json AS payloadJson FROM events WHERE kind NOT IN ('hud-tracking-start', 'rule-coverage') ORDER BY sequence").all() as Array<{
      eventId: string;
      kind: string;
      payloadJson: string;
    }>;
    expect(events.map(event => [event.eventId, event.kind])).toEqual([
      ["start", "session-start"],
      ["user", "user-prompt"],
      ["assistant", "assistant-message"],
      ["call", "call"],
      ["result", "result"],
      ["turn", "turn-end"],
      ["end", "session-end"],
    ]);
    expect(JSON.parse(events[3]!.payloadJson)).toMatchObject({ toolCallId: "call-1", toolName: "read", input: { path: "src/a.ts" } });
    expect(JSON.parse(events[4]!.payloadJson)).toMatchObject({ toolCallId: "call-1", toolName: "read", output: { text: "contents" } });
    expect(database.prepare("SELECT ended_at AS endedAt FROM sessions WHERE session_id = ?").get(session.sessionId)).toEqual({
      endedAt: expect.any(Number),
    });
    database.close();
  });

  it("preserves circular and BigInt call payloads across the native daemon boundary", async () => {
    const root = await temporaryDirectory();
    const daemon = await startNativeTestDaemon(root);
    const client = new NativeSessionClient({
      instance: { workspaceRoot: root, url: new URL(daemon.url) },
      token: daemon.state().nativeToken!,
      host: "omp", sessionId: "non-json", entries: () => [],
      appendEntry() {}, deliver() {},
    });
    const input: { count: bigint; self?: unknown } = { count: 7n };
    input.self = input;
    try {
      await client.initialize({ cwd: root, modelPath: resolve("model"), rules: [] });
      client.activate();
      await client.ingest({
        kind: "call", eventId: "non-json-call", sessionId: "non-json",
        toolCallId: "call", toolName: "submit", input,
      });
      const reader = new Database(defaultRerailDatabasePath(), { readonly: true });
      try {
        const event = reader.prepare("SELECT payload_v8 AS payloadV8 FROM events WHERE event_id = 'non-json-call'").get() as { payloadV8: Buffer };
        const payload = decodeEventPayload(event.payloadV8) as { input: typeof input };
        expect(payload.input.count).toBe(7n);
        expect(payload.input.self).toBe(payload.input);
      } finally {
        reader.close();
      }
    } finally {
      await client.close();
    }
  });

  it("returns the host-assigned revision for a remotely committed watermark", async () => {
    const root = await temporaryDirectory();
    const daemon = await startNativeTestDaemon(root);
    const branch: Array<{ type: string; id: string; timestamp: string; customType: string; data: unknown }> = [];
    let nextEntry = 1;
    const client = new NativeSessionClient({
      instance: { workspaceRoot: root, url: new URL(daemon.url) },
      token: daemon.state().nativeToken!,
      host: "omp",
      sessionId: "watermark-receipt",
      entries: () => branch,
      appendEntry(customType, data) {
        branch.push({
          type: "custom",
          id: `custom-${nextEntry++}`,
          timestamp: new Date().toISOString(),
          customType,
          data,
        });
      },
      deliver() {},
    });
    try {
      await client.initialize({ cwd: root, modelPath: resolve("model"), rules: [] });
      client.activate();
      const receipt = await client.mark({
        kind: "attach",
        targets: [{ source: "omp:message", id: "message-1" }],
        watermarks: [{
          namespace: "review",
          key: "approved",
          value: true,
          owner: "reviewer",
          visibility: "project",
          createdAt: 1,
        }],
      });

      expect(receipt).toMatchObject({
        revision: branch.at(-1)?.id,
        assignmentIds: [expect.any(String)],
      });
    } finally {
      await client.close();
    }
  });

  it("persists OMP events automatically through the live extension surface", async () => {
    const root = await temporaryDirectory();
    await startNativeTestDaemon(root);
    const databasePath = defaultRerailDatabasePath();
    const handlers = new Map<string, (event: unknown, context: ExtensionContext) => unknown>();
    const extensionApi = {
      on(event: string, handler: (event: unknown, context: ExtensionContext) => unknown) {
        handlers.set(event, handler);
      },
      setLabel() {},
      registerTool() {},
      registerCommand() {},
      registerMessageRenderer() {},
      appendEntry() {},
      sendMessage() {},
    } as unknown as ExtensionAPI;
    const context = {
      cwd: root,
      sessionManager: { getSessionId: () => "omp-live", getBranch: () => [] },
    } as unknown as ExtensionContext;
    // Only rules the repository adds run: one conversation rule now, a code rule added mid-session.
    const repositoryKey = setUpTestRepository(root, {
      library: [{ kind: "conversation", rule: { ...MOCK_CONVERSATION_RULES[0]!, id: "host-only", mode: "log", action: { content: "Host rule" } } }],
    });
    createConversationRulesExtension({
      modelPath: resolve("model"),
      registerTool: false,
      registerCommand: false,
    })(extensionApi);

    await handlers.get("session_start")?.({ type: "session_start", reason: "startup" }, context);
    const reader = new SqliteSessionReader(databasePath);
    await handlers.get("before_agent_start")?.({ type: "before_agent_start", prompt: "hello" }, context);
    expect(reader.hud("omp-live")).toEqual({ activeRuleCount: 1, steeringCount: 0 });
    const configuration = openRerailDatabase(databasePath);
    upsertLibraryRule(configuration, loadRuleCatalog(), MOCK_CODE_RULES[0]!, "code");
    addAttachment(configuration, { repositoryKey, folder: "", sourceKind: "library_rule", sourceId: MOCK_CODE_RULES[0]!.id });
    configuration.$client.close();
    expect(reader.hud("omp-live").activeRuleCount).toBeNull();
    await handlers.get("message_end")?.({
      type: "message_end",
      message: {
        role: "assistant",
        timestamp: 1,
        responseId: "response-1",
        provider: "test",
        model: "test",
        stopReason: "stop",
        content: [{ type: "text", text: "world" }],
      },
    }, context);
    expect(reader.hud("omp-live").activeRuleCount).toBe(2);
    reader.close();
    await Promise.all([
      handlers.get("tool_call")?.({
        type: "tool_call",
        toolCallId: "call-1",
        toolName: "read",
        input: { path: "src/a.ts" },
      }, context),
      handlers.get("tool_result")?.({
        type: "tool_result",
        toolCallId: "call-1",
        toolName: "read",
        input: { path: "src/a.ts" },
        content: [{ type: "text", text: "contents" }],
        details: undefined,
        isError: false,
      }, context),
      handlers.get("tool_call")?.({
        type: "tool_call",
        toolCallId: "call-2",
        toolName: "read",
        input: { path: "src/b.ts" },
      }, context),
      handlers.get("tool_result")?.({
        type: "tool_result",
        toolCallId: "call-2",
        toolName: "read",
        input: { path: "src/b.ts" },
        content: [{ type: "text", text: "other contents" }],
        details: undefined,
        isError: false,
      }, context),
    ]);
    await handlers.get("session_shutdown")?.({ type: "session_shutdown" }, context);

    const database = new Database(databasePath, { readonly: true });
    const events = database.prepare(
      "SELECT event_id AS eventId, kind FROM events WHERE kind NOT IN ('hud-tracking-start', 'rule-coverage') ORDER BY sequence",
    ).all() as Array<{ eventId: string; kind: string }>;
    expect(events.map(event => event.kind)).toEqual([
      "session-start",
      "user-prompt",
      "assistant-delta",
      "call",
      "result",
      "call",
      "result",
      "session-end",
    ]);
    database.close();
  });

  // contract: OMP fires session_switch / session_branch / session_tree for navigation inside the SAME session
  // (and for print-mode responses). Those must not tear the runtime down, and a streaming delta that lands
  // during a genuine switch must meet a session, never a gap.
  it("keeps recording through same-session navigation and across a real switch with a delta in flight", async () => {
    const root = await temporaryDirectory();
    await startNativeTestDaemon(root);
    const databasePath = defaultRerailDatabasePath();
    const handlers = new Map<string, (event: unknown, context: ExtensionContext) => unknown>();
    const extensionApi = {
      on(event: string, handler: (event: unknown, context: ExtensionContext) => unknown) { handlers.set(event, handler); },
      setLabel() {}, registerTool() {}, registerCommand() {}, registerMessageRenderer() {}, appendEntry() {}, sendMessage() {},
    } as unknown as ExtensionAPI;
    const contextFor = (sessionId: string) => ({ cwd: root, sessionManager: { getSessionId: () => sessionId, getBranch: () => [] } }) as unknown as ExtensionContext;
    createConversationRulesExtension({
      modelPath: resolve("model"),
      registerTool: false,
      registerCommand: false,
    })(extensionApi);
    const delta = (text: string, context: ExtensionContext) =>
      handlers.get("message_update")?.({
        type: "message_update",
        message: { role: "assistant", timestamp: 1, responseId: "r", provider: "test", model: "test", content: [{ type: "text", text }] },
        assistantMessageEvent: { type: "text_delta", delta: text },
      }, context);
    const kinds = (sessionId: string) => {
      const database = new Database(databasePath, { readonly: true });
      try {
        return (database.prepare("SELECT kind FROM events WHERE session_id = ? AND kind NOT IN ('hud-tracking-start', 'rule-coverage') ORDER BY sequence").all(sessionId) as Array<{ kind: string }>).map(row => row.kind);
      } finally {
        database.close();
      }
    };

    const first = contextFor("omp-first");
    // An instance that owns no session (OMP's secondary agent in print mode) ignores stream events: no throw,
    // nothing recorded. Once session_start is dispatched, a delta racing its completion waits and is recorded.
    await expect(delta("orphan", first)).resolves.toBeUndefined();
    const starting = handlers.get("session_start")?.({ type: "session_start" }, first);
    await delta("one", first);
    await starting;
    // Same session: branch / tree / switch(resume) must be no-ops for the runtime — no session-end, no restart.
    await handlers.get("session_branch")?.({ type: "session_branch", previousSessionFile: undefined }, first);
    await handlers.get("session_tree")?.({ type: "session_tree", newLeafId: "x", oldLeafId: null }, first);
    await handlers.get("session_switch")?.({ type: "session_switch", reason: "resume", previousSessionFile: undefined }, first);
    await delta("two", first);
    expect(kinds("omp-first")).toEqual(["session-start", "assistant-delta", "assistant-delta"]);

    // A real switch: a delta fired after the switch began belongs to the session being switched to, and is
    // recorded there once that session is up — never dropped, never an error, never on the old session.
    const second = contextFor("omp-second");
    const switching = handlers.get("session_switch")?.({ type: "session_switch", reason: "new", previousSessionFile: "first.jsonl" }, second);
    await delta("during", second);
    await switching;
    await delta("after", second);
    await handlers.get("session_shutdown")?.({ type: "session_shutdown" }, second);

    expect(kinds("omp-first")).toEqual(["session-start", "assistant-delta", "assistant-delta", "session-end"]);
    expect(kinds("omp-second")).toEqual(["session-start", "assistant-delta", "assistant-delta", "session-end"]);
  });

  it("lets a viewer open before recording begins and page decoded events after the writer closes", async () => {
    const root = await temporaryDirectory();
    const path = defaultRerailDatabasePath();
    const reader = new SqliteSessionReader(path);
    expect(reader.sessions()).toEqual([]);
    expect(reader.events("session-1")).toEqual([]);
    expect(reader.hud("session-1")).toEqual({ activeRuleCount: null, steeringCount: null });

    const store = new SqliteSessionStore(path);
    store.openSession({ host: "codex", sessionId: "session-1", cwd: "/workspace" }, 10);
    store.append({ sessionId: "session-1", eventId: "event-1", kind: "user-prompt", observedAt: 11, payload: { text: "first" } });
    store.append({ sessionId: "session-1", eventId: "event-2", kind: "turn-end", observedAt: 12, payload: { text: "second" } });
    store.closeSession("session-1", 13);
    store.close();

    expect(reader.sessions()).toEqual([{
      repositoryId: null,
      sessionId: "session-1",
      host: "codex",
      cwd: "/workspace",
      startedAt: 10,
      endedAt: 13,
      eventCount: 2,
    }]);
    expect(reader.events("session-1", { limit: 1 })).toEqual([{
      sequence: 1,
      sessionId: "session-1",
      eventId: "event-1",
      kind: "user-prompt",
      observedAt: 11,
      payload: { text: "first" },
    }]);
    expect(reader.events("session-1", { after: 1, limit: 1 })).toEqual([{
      sequence: 2,
      sessionId: "session-1",
      eventId: "event-2",
      kind: "turn-end",
      observedAt: 12,
      payload: { text: "second" },
    }]);
    reader.close();
  });

  it("counts successful steers across the full session history, deduplicating only receipt retries", async () => {
    const { store, reader, session: first } = await hudFixture("first");
    const second = { ...first, sessionId: "second" };
    for (const session of [first, second]) {
      store.openSession(session);
      store.startHudTracking(session, true);
    }
    expect(reader.hud(first.sessionId).steeringCount).toBe(0);
    for (let index = 0; index < 501; index += 1) {
      store.recordSteer(first, `receipt-${index}`);
    }
    store.recordSteer(first, "receipt-0");
    store.recordSteer(second, "receipt-0");
    store.append({ sessionId: first.sessionId, kind: "delivery", payload: { requested: "steer" } });
    store.append({ sessionId: first.sessionId, kind: "rule-match", payload: { ruleId: "matched" } });
    expect(reader.hud(first.sessionId).steeringCount).toBe(501);
    expect(reader.hud(second.sessionId).steeringCount).toBe(1);
    store.closeSession(first.sessionId);
    store.openSession(first);
    store.startHudTracking(first, false);
    expect(reader.hud(first.sessionId).steeringCount).toBe(501);
  });

  it("counts one steer per rule a receipt carried, so two rules in one edit read twice", async () => {
    const { store, reader, session } = await hudFixture("per-rule");
    store.openSession(session);
    store.startHudTracking(session, true);
    store.recordSteer(session, "two-rules", ["a:one", "a:two"]);
    expect(reader.hud(session.sessionId).steeringCount).toBe(2);
    store.recordSteer(session, "one-rule", ["a:one"]);
    expect(reader.hud(session.sessionId).steeringCount).toBe(3);
  });

  it("keeps untracked and reset totals unknown rather than counting only surviving receipts", async () => {
    const { path, repositoryId, store, reader, session } = await hudFixture("untracked");
    store.openSession(session);
    store.recordSteer(session, "new-receipt");
    expect(reader.hud(session.sessionId)).toEqual({ activeRuleCount: null, steeringCount: null });
    store.startHudTracking(session, false);
    store.startHudTracking(session, true);
    expect(reader.hud(session.sessionId).steeringCount).toBeNull();
    const fresh = { ...session, sessionId: "fresh" };
    store.openSession(fresh);
    store.startHudTracking(fresh, true);
    store.recordSteer(fresh, "before-reset");
    expect(reader.hud(fresh.sessionId).steeringCount).toBe(1);
    resetRepositoryHistory(path, repositoryId);
    store.recordSteer(fresh, "after-reset");
    expect(reader.hud(fresh.sessionId).steeringCount).toBeNull();
  });

  it("credits a steer receipt to the rules whose violations it carried", async () => {
    const { path, store, reader, session } = await hudFixture("attributed");
    store.openSession(session);
    store.startHudTracking(session, true);
    store.recordSteer(session, "attributed-receipt", ["rerail:no-raw-sql", "team:api-auth"]);
    store.recordSteer(session, "unattributed-receipt");

    expect(reader.events(session.sessionId).filter(event => event.kind === "steering-delivered")).toEqual([
      expect.objectContaining({
        eventId: "steering-delivered:attributed-receipt",
        payload: { ruleIds: ["rerail:no-raw-sql", "team:api-auth"] },
      }),
      expect.objectContaining({ eventId: "steering-delivered:unattributed-receipt", payload: { ruleIds: [] } }),
    ]);
    const database = closeAfterTest(new Database(path, { readonly: true }));
    expect(database.prepare("SELECT payload_json AS payloadJson FROM events WHERE event_id = ?").get("steering-delivered:attributed-receipt"))
      .toEqual({ payloadJson: '{"ruleIds":["rerail:no-raw-sql","team:api-auth"]}' });
  });

  it("serves the latest effective coverage only while its repository configuration is unchanged", async () => {
    const { root, store, reader, session } = await hudFixture("covered");
    const repositoryKey = setUpTestRepository(root, { library: [{ kind: "conversation", rule: MOCK_CONVERSATION_RULES[0]! }] });
    const database = openRerailDatabase(defaultRerailDatabasePath());
    closeAfterTest({ close: () => database.$client.close() });
    const ownerCheckoutPaths = [realpathSync(root)];
    const signature = () => ruleConfigurationSignature(database, loadRuleCatalog(), repositoryKey, ownerCheckoutPaths);
    store.openSession(session);
    store.startHudTracking(session, true);
    const coverage = { repositoryId: repositoryKey, ownerCheckoutPaths, activeRuleCount: 3, ruleSignature: signature() };
    store.recordRuleCoverage(session, coverage);
    expect(reader.hud(session.sessionId)).toEqual({ activeRuleCount: 3, steeringCount: 0 });
    setRuleSetting(database, {
      owner: { kind: "repository", repositoryKey }, folder: "", targetKind: "rule", targetId: MOCK_CONVERSATION_RULES[0]!.id, on: false, mode: null,
    });
    expect(reader.hud(session.sessionId).activeRuleCount).toBeNull();
    // A local registry mutation does not establish synchronization with a changed configuration.
    store.recordRuleCoverage(session, { ...coverage, activeRuleCount: 4 });
    expect(reader.hud(session.sessionId).activeRuleCount).toBeNull();
    store.recordRuleCoverage(session, { ...coverage, activeRuleCount: 2, ruleSignature: signature() });
    expect(reader.hud(session.sessionId).activeRuleCount).toBe(2);
    removeAttachment(database, { repositoryKey, folder: "", sourceKind: "library_rule", sourceId: MOCK_CONVERSATION_RULES[0]!.id });
    expect(reader.hud(session.sessionId).activeRuleCount).toBeNull();
  });

  it("preserves a runtime's complete baseline across reopen but not across a database reset", async () => {
    const root = await temporaryDirectory();
    const path = defaultRerailDatabasePath();
    const repositoryKey = setUpTestRepository(root);
    const options = { storageRoot: join(root, "sessions"), workspaceRoot: root, databasePath: path, modelPath: resolve("model"), autostart: false };
    const reader = new SqliteSessionReader(path);
    const session = { host: "claude-code" as const, sessionId: "runtime-hud", cwd: root };
    const legacy = { ...session, sessionId: "pre-tracking" };
    const oldStore = new SqliteSessionStore(path);
    oldStore.openSession(legacy);
    oldStore.append({ sessionId: legacy.sessionId, kind: "delivery", payload: { requested: "steer" } });
    oldStore.close();
    let runtime = new RerailRuntime(options);
    try {
      await runtime.ingest({ kind: "session-start", session: legacy, eventId: "legacy-resume" });
      runtime.recordSteer(legacy.sessionId, "new-only");
      expect(reader.hud(legacy.sessionId).steeringCount).toBeNull();
      await runtime.ingest({ kind: "session-start", session, eventId: "start" });
      // The evaluator, whose catalog the coverage reads, opens behind the session start; a turn boundary waits for it.
      await runtime.ingest({ kind: "turn-end", sessionId: session.sessionId, eventId: "opened", text: "" });
      expect(reader.hud(session.sessionId).steeringCount).toBe(0);
      const baseline = reader.hud(session.sessionId).activeRuleCount;
      expect(baseline).not.toBeNull();
      const database = openRerailDatabase(path);
      try {
        upsertLibraryRule(database, loadRuleCatalog(), { ...MOCK_CONVERSATION_RULES[0]!, id: "late-source" }, "conversation");
        addAttachment(database, { repositoryKey, folder: "", sourceKind: "library_rule", sourceId: "late-source" });
      } finally {
        database.$client.close();
      }
      // Accounting cannot claim a mutation until the source actually loads it.
      runtime.recordRuleCoverage(session.sessionId);
      expect(reader.hud(session.sessionId).activeRuleCount).toBeNull();
      await runtime.ingest({ kind: "assistant-delta", sessionId: session.sessionId, eventId: "sync", messageId: "stream", delta: "Continue", final: false });
      expect(reader.hud(session.sessionId).activeRuleCount).toBe(baseline! + 1);
      runtime.recordSteer(session.sessionId, "sent");
      await runtime.dispose();
      const endedSession = reader.session(session.sessionId)!;
      runtime.recordSteer(session.sessionId, "late-send");
      expect(reader.hud(session.sessionId).steeringCount).toBe(2);
      expect(reader.session(session.sessionId)).toMatchObject({ startedAt: endedSession.startedAt, endedAt: endedSession.endedAt });
      runtime = new RerailRuntime(options);
      await runtime.ingest({ kind: "session-start", session, eventId: "resume" });
      expect(reader.hud(session.sessionId).steeringCount).toBe(2);
      runtime.recordSteer(session.sessionId, "sent");
      expect(reader.hud(session.sessionId).steeringCount).toBe(2);
      await runtime.dispose();
      const closedSession = reader.session(session.sessionId)!;
      resetRepositoryHistory(path, repositoryKey);
      runtime.recordSteer(session.sessionId, "late-after-reset");
      expect(reader.session(session.sessionId)).toMatchObject({
        startedAt: closedSession.startedAt, endedAt: closedSession.endedAt,
      });
      expect(reader.hud(session.sessionId).steeringCount).toBeNull();
      runtime = new RerailRuntime(options);
      await runtime.ingest({ kind: "session-start", session, eventId: "resume-after-reset" });
      runtime.recordSteer(session.sessionId, "after-reset");
      expect(reader.hud(session.sessionId).steeringCount).toBeNull();
    } finally {
      await runtime.dispose();
      reader.close();
    }
  });

});

describe("one database for every repository", () => {
  it("lists and aggregates only the scoped repository while by-id reads find any session", async () => {
    const root = await temporaryDirectory();
    const path = defaultRerailDatabasePath();
    const alpha = closeAfterTest(new SqliteSessionStore(path, { repositoryId: "alpha" }));
    const beta = closeAfterTest(new SqliteSessionStore(path, { repositoryId: "beta" }));
    alpha.openSession({ host: "omp", sessionId: "alpha-session", cwd: join(root, "alpha") }, 100);
    beta.openSession({ host: "pi", sessionId: "beta-session", cwd: join(root, "beta") }, 200);
    alpha.recordSteer({ host: "omp", sessionId: "alpha-session", cwd: join(root, "alpha") }, "a", ["shared-rule"]);
    beta.recordSteer({ host: "pi", sessionId: "beta-session", cwd: join(root, "beta") }, "b", ["shared-rule"]);
    beta.recordSteer({ host: "pi", sessionId: "beta-session", cwd: join(root, "beta") }, "c", ["shared-rule"]);

    const alphaView = closeAfterTest(new SqliteSessionReader(path, { repositoryId: "alpha" }));
    const machineView = closeAfterTest(new SqliteSessionReader(path));
    expect(alphaView.sessions().map(session => [session.sessionId, session.repositoryId])).toEqual([["alpha-session", "alpha"]]);
    expect(alphaView.sessionPage({ since: 0, limit: 50 }).sessions.map(session => session.sessionId)).toEqual(["alpha-session"]);
    expect(alphaView.steersByRule(0)).toEqual(new Map([["shared-rule", 1]]));
    expect(machineView.sessions().map(session => session.sessionId)).toEqual(["alpha-session", "beta-session"]);
    expect(machineView.steersByRule(0)).toEqual(new Map([["shared-rule", 3]]));
    expect(alphaView.session("beta-session")).toMatchObject({ repositoryId: "beta" });
    expect(alpha.sessions().map(session => session.sessionId)).toEqual(["alpha-session"]);
  });
});

describe("resetRepositoryHistory", () => {
  it("clears or explicitly keeps every session-keyed table, so a new one cannot slip past a reset", () => {
    const reset = new Set<SQLiteTable>(schema.SESSION_HISTORY_TABLES);
    // Handled by hand: ended sessions and their runtime ownership go, running ones stay. The derived audit history
    // goes with the audits it came from, through the triggers on `audits`.
    const handled = new Set<SQLiteTable>([schema.sessions, schema.policySessionOwners, ...schema.DERIVED_AUDIT_HISTORY_TABLES]);
    const exported: unknown[] = Object.values(schema);
    const tables = exported.filter((value): value is SQLiteTable => is(value, SQLiteTable));
    for (const table of tables) {
      const config = getTableConfig(table);
      if (config.columns.some(column => column.name === "session_id")) {
        expect(reset.has(table) || handled.has(table), `${config.name} is keyed by session but reset ignores it`).toBe(true);
      }
      for (const foreignKey of config.foreignKeys) {
        if (reset.has(foreignKey.reference().foreignTable)) {
          expect(foreignKey.onDelete, `${config.name} must cascade with the history it references`).toBe("cascade");
        }
      }
    }
    expect(tables).toContain(schema.auditResultPages);
  });

  // contract (`rerail reset`): one repository's history goes; its still-running sessions keep their rows; rules,
  // settings, and other repositories stay; the file is never unlinked.
  it("removes one repository's history under live connections and leaves rules and other repositories alone", async () => {
    const root = await temporaryDirectory();
    const path = defaultRerailDatabasePath();
    const running = { host: "omp" as const, sessionId: "running", cwd: root };
    const writer = new SqliteSessionStore(path, { repositoryId: "alpha" });
    const other = new SqliteSessionStore(path, { repositoryId: "beta" });
    const rules = openRerailDatabase(path);
    const preferences = new SqlitePresentationPreferenceStore(path);
    try {
      writer.openSession(running, 100);
      writer.append({ sessionId: "running", eventId: "before", kind: "user-prompt", observedAt: 110, payload: { text: "before" } }, running);
      writer.openSession({ host: "pi", sessionId: "finished", cwd: root }, 50);
      writer.append({ sessionId: "finished", eventId: "done", kind: "turn-end", observedAt: 60, payload: {} });
      writer.linkWork({ sessionId: "finished", kind: "git-branch", reference: "main" });
      writer.closeSession("finished", 90);
      other.openSession({ host: "codex", sessionId: "elsewhere", cwd: join(root, "beta") }, 70);
      other.append({ sessionId: "elsewhere", eventId: "kept", kind: "user-prompt", observedAt: 75, payload: {} });
      other.closeSession("elsewhere", 80);
      upsertLibraryRule(rules, loadRuleCatalog(), MOCK_CONVERSATION_RULES[0], "conversation");
      preferences.upsert({ repository: "", host: "", presence: "silent" });
      const inode = (await stat(path)).ino;

      expect(resetRepositoryHistory(path, "alpha")).toEqual({ sessions: 1, openSessions: 1 });
      expect((await stat(path)).ino).toBe(inode);
      expect(listLibraryRules(rules).map(stored => stored.document.rule.id)).toEqual([MOCK_CONVERSATION_RULES[0]?.id]);
      expect(preferences.effective(root, "omp")).toBe("silent");
      const reader = closeAfterTest(new SqliteSessionReader(path));
      expect(reader.sessions()).toEqual([
        expect.objectContaining({ sessionId: "elsewhere", repositoryId: "beta", eventCount: 1 }),
        expect.objectContaining({ sessionId: "running", repositoryId: "alpha", startedAt: 100, endedAt: null, eventCount: 0 }),
      ]);
      expect(reader.workLinks("finished")).toEqual([]);

      // The running session keeps recording through its existing connection.
      writer.append({ sessionId: "running", eventId: "before", kind: "user-prompt", observedAt: 130, payload: { text: "again" } });
      expect(reader.events("running").map(event => [event.eventId, event.observedAt])).toEqual([["before", 130]]);

      // Once the session has ended its row goes with the next reset; a writer holding the descriptor heals the row.
      writer.closeSession("running", 135);
      expect(resetRepositoryHistory(path, "alpha")).toEqual({ sessions: 1, openSessions: 0 });
      expect(() =>
        writer.append({ sessionId: "running", eventId: "orphan", kind: "turn-end", observedAt: 140, payload: {} }),
      ).toThrow(/FOREIGN KEY/u);
      writer.append({ sessionId: "running", eventId: "healed", kind: "turn-end", observedAt: 141, payload: {} }, running);
      expect(reader.session("running")).toMatchObject({ repositoryId: "alpha" });
      expect(reader.events("running").map(event => event.eventId)).toEqual(["healed"]);
      expect(reader.session("elsewhere")).toMatchObject({ eventCount: 1 });
    } finally {
      rules.$client.close();
      preferences.close();
      writer.close();
      other.close();
    }
  });
});
