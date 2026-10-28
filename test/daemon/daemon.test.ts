import { existsSync } from "node:fs";
import { createServer, request as httpRequest, type IncomingMessage } from "node:http";
import { mkdtemp, mkdir, readFile, realpath, rm, stat, symlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { StreamableHTTPClientTransport } from "@modelcontextprotocol/sdk/client/streamableHttp.js";
import { afterEach, describe, expect, it, vi } from "vitest";
import { publicHostSet, operations, presentationResponseSchema } from "../../src/api/index.js";
import {
  assertHostBindable,
  DEFAULT_DAEMON_PORT,
  defaultEngineSocketPath,
  parsePublicHosts,
  RerailDaemon,
  type RerailDaemonOptions,
} from "../../src/daemon/daemon.js";
import { NativeSessionServer } from "../../src/daemon/native-session-server.js";
import {
  type DaemonHealth,
  ingestEngineEvent,
  SqliteSessionStore,
  SESSION_EVENT_PAGE_LIMIT,
} from "../../src/runtime/index.js";
import { MOCK_CONVERSATION_RULES, MOCK_CODE_RULES } from "../../src/cli/index.js";
import { openRerailDatabase, SqliteAuditStore, SqliteAuditControlStore } from "../../src/engine/index.js";
import { startTestRepository, stopTestRepository } from "../support/started-repository.js";

const healthyNative = () => ({ ok: true, kernelVersion: "1" });

const originalRerailHome = process.env.RERAIL_HOME;

describe("daemon bind host", () => {
  it("binds loopback spellings freely and anything else only with --allow-remote", () => {
    for (const host of ["127.0.0.1", "localhost", "LOCALHOST", "::1"]) expect(() => assertHostBindable(host, false)).not.toThrow();
    for (const host of ["0.0.0.0", "::", "192.168.1.5", "rerail.example"]) {
      expect(() => assertHostBindable(host, false)).toThrow(/--allow-remote/u);
      expect(() => assertHostBindable(host, true)).not.toThrow();
    }
  });

  it("reads --public-host as a comma-separated list of bare names and refuses a port in one", () => {
    expect(parsePublicHosts(undefined)).toEqual([]);
    expect(parsePublicHosts("rerail.example, Rerail.Internal ,")).toEqual(["rerail.example", "Rerail.Internal"]);
    expect(publicHostSet(["Rerail.Example", "[fd00::1]"])).toEqual(new Set(["rerail.example", "[fd00::1]"]));
    expect(() => publicHostSet(["rerail.example:443"])).toThrow(/Invalid public host/u);
    expect(() => publicHostSet(["::1"])).toThrow(/Invalid public host/u);
  });
});

/** The capability every test daemon requires; the helpers below send it unless a test names another. */
const ACCESS = "test-access-capability";
const ACCESS_HEADER = "x-rerail-access";

describe("Rerail daemon", () => {
  let root: string;
  const daemons: RerailDaemon[] = [];

  /** `fetch` against a test daemon carries its capability, as every Rerail client and a browser holding the cookie does. */
  const fetch = (input: string | URL, init: RequestInit & { headers?: Record<string, string> } = {}): Promise<Response> =>
    globalThis.fetch(input, { ...init, headers: { [ACCESS_HEADER]: ACCESS, ...init.headers } });

  /** A daemon requiring `ACCESS`, unless `minted`: then it records or mints its own, as a real start does. */
  async function start(overrides: Partial<RerailDaemonOptions> = {}, minted = false): Promise<RerailDaemon> {
    process.env.RERAIL_HOME = join(root, "home");
    const daemon = await RerailDaemon.start({
      version: "0.0.0-test",
      databasePath: join(root, "rerail.sqlite"),
      workspaceRoot: root,
      port: 0,
      nativeHealth: healthyNative,
      ...(minted ? {} : { accessToken: ACCESS }),
      ...overrides,
    });
    daemons.push(daemon);
    return daemon;
  }

  async function getJson<T = unknown>(daemon: RerailDaemon, path: string): Promise<{ status: number; body: T }> {
    return await sendJson<T>(daemon, path, "GET");
  }

  async function sendJson<T = unknown>(daemon: RerailDaemon, path: string, method: string, body?: unknown): Promise<{ status: number; body: T }> {
    const response = await fetch(new URL(path, daemon.url), {
      method,
      ...(body === undefined ? {} : { headers: { "content-type": "application/json" }, body: JSON.stringify(body) }),
    });
    // Test boundary: the daemon's JSON is asserted against expectations right after.
    const parsed = await response.json() as T;
    return { status: response.status, body: parsed };
  }

  afterEach(async () => {
    await Promise.all(daemons.splice(0).map(daemon => daemon.close()));
    if (originalRerailHome === undefined) delete process.env.RERAIL_HOME;
    else process.env.RERAIL_HOME = originalRerailHome;
    await rm(root, { recursive: true, force: true });
  });

  /** `fetch` refuses a caller-supplied `Host`; only a raw request can imitate a rebinding page or a port mapping. */
  function rawRequest(
    daemon: RerailDaemon,
    path: string,
    { method = "GET", headers = {}, body }: { method?: string; headers?: Record<string, string>; body?: string } = {},
  ): Promise<{ status: number; body: unknown }> {
    const { promise, resolve, reject } = Promise.withResolvers<{ status: number; body: unknown }>();
    const outgoing = httpRequest(new URL(path, daemon.url), {
      method,
      setHost: false,
      headers: {
        [ACCESS_HEADER]: ACCESS,
        ...(body === undefined ? {} : { "content-type": "application/json", "content-length": Buffer.byteLength(body) }),
        ...headers,
      },
    }, (incoming) => {
      const chunks: Buffer[] = [];
      incoming.on("data", chunk => chunks.push(chunk));
      incoming.on("end", () => {
        const text = Buffer.concat(chunks).toString();
        const json = incoming.headers["content-type"]?.startsWith("application/json") === true;
        resolve({ status: incoming.statusCode!, body: json ? JSON.parse(text) : text });
      });
      incoming.on("error", reject);
    });
    outgoing.on("error", reject);
    outgoing.end(body);
    return promise;
  }

  it("reports health with the native probe and database presence", async () => {
    root = await mkdtemp(join(tmpdir(), "rerail-daemon-"));
    const daemon = await start({ nativeHealth: () => { throw new Error("addon missing"); } });
    const { status, body: health } = await getJson<DaemonHealth>(daemon, "/healthz");
    expect(status).toBe(200);
    expect(health).toMatchObject({
      status: "ok",
      version: "0.0.0-test",
      pid: process.pid,
      database: { path: join(root, "rerail.sqlite"), exists: false },
      native: { ok: false, error: "addon missing" },
      ui: "dev",
      installedAt: null,
    });
    const query = new URLSearchParams({ repository: root, host: "omp", sessionId: "missing" });
    expect((await getJson(daemon, `/api/presentation?${query}`)).body).toMatchObject({
      presence: "compact", context: { daemon: { pid: process.pid, native: { ok: false } }, session: null },
    });
    expect(existsSync(join(root, "rerail.sqlite"))).toBe(false);
  });

  it("requires the private capability for every native session and protects its ownership", async () => {
    root = await mkdtemp(join(tmpdir(), "rerail-daemon-"));
    const workspaceRoot = join(root, "daemon-workspace");
    const cwd = join(root, "session-workspace");
    await Promise.all([mkdir(workspaceRoot), mkdir(cwd)]);
    const daemon = await start({ workspaceRoot });
    const input = {
      workspaceRoot, sessionId: "private-native", host: "omp", operation: "initialize", entries: [],
      input: { cwd, modelPath: resolve("model"), databasePath: join(cwd, "custom.sqlite"), rules: [] },
    };
    const request = (body: unknown, headers: Record<string, string> = {}) => fetch(new URL("/native/session", daemon.url), {
      method: "POST", headers: { "content-type": "application/json", ...headers }, body: JSON.stringify(body),
    });
    const unauthorized = await request(input);
    expect(unauthorized.status).toBe(403);
    expect(await unauthorized.json()).toEqual({ error: "native-authorization-invalid" });
    expect(existsSync(join(cwd, "custom.sqlite"))).toBe(false);
    const authorization = `Bearer ${daemon.state().nativeToken!}`;
    expect((await request(input, { authorization: "Bearer invalid" })).status).toBe(403);
    const browser = await request(input, { authorization, origin: "http://attacker.test" });
    expect(browser.status).toBe(403);
    expect(await browser.json()).toEqual({ error: "native-origin-forbidden" });
    const initialized = await request(input, { authorization });
    expect(initialized.status).toBe(200);
    expect(existsSync(join(cwd, "custom.sqlite"))).toBe(true);
    const close = { workspaceRoot, sessionId: input.sessionId, host: "omp", operation: "close" };
    expect((await request(close)).status).toBe(403);
    expect((await request({ ...close, operation: "cosine", input: { action: "list" } }, { authorization })).status).toBe(200);
    expect((await request(close, { authorization })).status).toBe(200);
    expect((await getJson(daemon, "/healthz")).body).not.toHaveProperty("nativeToken");
  });

  it("serves sessions and paged events read from the session store", async () => {
    root = await mkdtemp(join(tmpdir(), "rerail-daemon-"));
    const daemon = await start();
    expect(await getJson(daemon, "/api/sessions")).toEqual({ status: 200, body: { sessions: [], total: 0, nextCursor: null } });
    // Before anything is recorded, a malformed cursor is refused just as it is once history exists.
    expect(existsSync(join(root, "rerail.sqlite"))).toBe(false);
    for (const path of ["/api/sessions?cursor=not-a-cursor", "/api/violations?since=0&cursor=not-a-cursor"]) {
      expect(await getJson(daemon, path), path).toMatchObject({ status: 400, body: { error: "invalid-cursor" } });
    }

    const store = new SqliteSessionStore(join(root, "rerail.sqlite"), { repositoryId: "test-repository" });
    store.openSession({ host: "claude-code", sessionId: "s1", cwd: "/workspace" }, 1_000);
    store.append({ sessionId: "s1", kind: "user-prompt", eventId: "e1", observedAt: 1_001, payload: { text: "hello" } });
    store.append({ sessionId: "s1", kind: "call", eventId: "e2", observedAt: 1_002, payload: { toolName: "read" } });
    store.append({ sessionId: "s1", kind: "turn-end", eventId: "e3", observedAt: 1_003, payload: { text: "done" } });
    store.closeSession("s1", 2_000);
    store.close();

    expect(await getJson(daemon, "/api/sessions")).toEqual({
      status: 200,
      body: {
        sessions: [{
          sessionId: "s1", host: "claude-code", cwd: "/workspace", repositoryId: "test-repository", startedAt: 1_000, endedAt: 2_000,
          eventCount: 3, violationCount: 0, steeredViolations: 0, lastEventAt: 1_003,
        }],
        total: 1,
        nextCursor: null,
      },
    });

    const firstPage = await getJson(daemon, "/api/sessions/s1/events?limit=2");
    expect(firstPage).toEqual({
      status: 200,
      body: {
        events: [
          { sequence: 1, sessionId: "s1", eventId: "e1", kind: "user-prompt", observedAt: 1_001, payload: { text: "hello" } },
          { sequence: 2, sessionId: "s1", eventId: "e2", kind: "call", observedAt: 1_002, payload: { toolName: "read" } },
        ],
        nextAfter: 2,
        pageLimit: SESSION_EVENT_PAGE_LIMIT,
      },
    });
    const secondPage = await getJson<{ events: Array<{ eventId: string }>; nextAfter: number }>(daemon, "/api/sessions/s1/events?after=2");
    expect(secondPage.body.nextAfter).toBe(3);
    expect(secondPage.body.events.map(event => event.eventId)).toEqual(["e3"]);
    expect(await getJson(daemon, "/api/sessions/s1/events?after=3")).toMatchObject({ body: { events: [], nextAfter: 3 } });
    // Backward: the newest page, then the page before it, each in sequence order with the tail as `nextAfter`.
    const newest = await getJson<{ events: Array<{ eventId: string }>; nextAfter: number }>(daemon, "/api/sessions/s1/events?before=9007199254740991&limit=2");
    expect(newest.body).toMatchObject({ events: [{ eventId: "e2" }, { eventId: "e3" }], nextAfter: 3 });
    expect((await getJson<{ events: Array<{ eventId: string }> }>(daemon, "/api/sessions/s1/events?before=2&limit=2")).body.events.map(event => event.eventId))
      .toEqual(["e1"]);
    expect(await getJson(daemon, "/api/sessions/missing/events")).toMatchObject({ body: { events: [] } });
    expect(await getJson(daemon, "/api/sessions/s1")).toMatchObject({
      status: 200,
      body: { session: { sessionId: "s1", eventCount: 3 }, hud: { activeRuleCount: null, steeringCount: null } },
    });
    expect(await getJson(daemon, "/api/sessions/missing")).toMatchObject({ status: 404, body: { error: "session-not-found" } });
    // The database this test created dates the install; it is read once and never changes.
    const { installedAt } = (await getJson<DaemonHealth>(daemon, "/healthz")).body;
    expect(installedAt).toBeGreaterThan(0);
    expect(installedAt).toBeLessThanOrEqual(Date.now());
    expect((await getJson(daemon, "/healthz")).body).toMatchObject({ database: { exists: true }, installedAt });
  });

  it("resolves canonical presentation overrides in precedence order and falls through after deletion", async () => {
    root = await mkdtemp(join(tmpdir(), "rerail-daemon-"));
    const alias = join(root, "alias");
    await symlink(root, alias);
    const daemon = await start();
    const repository = await realpath(root);
    const query = new URLSearchParams({ repository: alias, host: "omp" });
    const overrides = [
      { repository: "", host: "", presence: "silent" },
      { repository: "", host: "omp", presence: "compact" },
      { repository: alias, host: "", presence: "silent" },
      { repository: alias, host: "omp", presence: "compact" },
    ];
    // The test repository was never started; presence resolves the same either way.
    expect((await getJson(daemon, `/api/presentation?${query}`)).body).toEqual({ presence: "compact", repositoryState: "stopped", scope: null });
    for (const preference of overrides) {
      const result = await sendJson(daemon, "/api/presentation-preferences", "PUT", preference);
      expect(result).toMatchObject({ status: 200, body: {
        preference: { ...preference, repository: preference.repository === "" ? "" : repository },
      } });
      expect((await getJson(daemon, `/api/presentation?${query}`)).body).toEqual({ presence: preference.presence, repositoryState: "stopped", scope: null });
    }
    for (let index = overrides.length - 1; index >= 0; index--) {
      const preference = overrides[index]!;
      const key = new URLSearchParams({ repository: preference.repository === "" ? "" : repository, host: preference.host });
      expect(await sendJson(daemon, `/api/presentation-preferences?${key}`, "DELETE")).toEqual({ status: 200, body: { removed: 1 } });
      expect((await getJson(daemon, `/api/presentation?${query}`)).body).toEqual({ presence: overrides[index - 1]?.presence ?? "compact", repositoryState: "stopped", scope: null });
    }
  });

  it("keeps session status bounded even when an audit contains multi-megabyte violations", async () => {
    root = await mkdtemp(join(tmpdir(), "rerail-daemon-"));
    const path = join(root, "rerail.sqlite");
    const sessionStore = new SqliteSessionStore(path, { repositoryId: "test-repository" });
    sessionStore.openSession({ host: "omp", sessionId: "large-run", cwd: root }, 1);
    sessionStore.startHudTracking({ host: "omp", sessionId: "large-run", cwd: root }, true);
    sessionStore.close();
    const audits = new SqliteAuditStore(path);
    const audit = audits.start({ sessionId: "large-run", workspaceRoot: root, scope: "repository", mode: "log", revision: 0, paths: [], catalogRevision: "test", ruleRevisions: [] });
    audits.finish(audit, {
      status: "failed",
      violations: [{
        id: "large-violation", ruleId: "policy", clauseId: "check", code: "code",
        mode: "block", message: "x".repeat(2 * 1024 * 1024),
        path: "source.ts", captures: {}, variables: {},
      }],
      failures: [
        { clauseId: "scan", code: "budget-exceeded", message: "scan exceeded its budget", stage: "scan" },
        { clauseId: "source", code: "budget-exceeded", message: "Source exceeds the per-file byte budget", path: "big.ts", stage: "read" },
        // A file blind for its syntax fails the audit; one blind for its work is a gap.
        { clauseId: "file", code: "source-incomplete", message: "file has syntax errors", path: "broken.ts", stage: "evaluate", blind: "syntax" },
        { clauseId: "file", code: "budget-exceeded", message: "file parsing time budget exceeded", path: "huge.ts", stage: "evaluate", blind: "budget" },
        // A rule that ran out of its own work is a gap, counted per rule.
        { ruleId: "slow", clauseId: "c", code: "budget-exceeded", message: "clause work budget exceeded", path: "a.ts", stage: "evaluate" },
        { ruleId: "slow", clauseId: "c", code: "budget-exceeded", message: "clause work budget exceeded", path: "b.ts", stage: "evaluate" },
        // A rule the file's work stopped is a gap, but did not run out itself.
        { ruleId: "late", clauseId: "c", code: "file-budget-exceeded", message: "file clause work budget exceeded", path: "a.ts", stage: "evaluate" },
        { ruleId: "late", clauseId: "c", code: "file-budget-exceeded", message: "file AST work budget exceeded", path: "b.ts", stage: "evaluate" },
      ],
      // The list holds eight of the run's failures; it had 140 more.
      unlistedFailureCount: 140,
    });
    audits.close();
    const daemon = await start();
    const query = new URLSearchParams({ repository: root, host: "omp", sessionId: "large-run" });
    const response = await fetch(new URL(`/api/presentation?${query}`, daemon.url));
    const text = await response.text();
    expect(response.status).toBe(200);
    expect(Buffer.byteLength(text)).toBeLessThan(1024);
    const compact = presentationResponseSchema.parse(JSON.parse(text));
    expect(compact.context?.daemon).toEqual({
      pid: process.pid, database: { path, exists: true }, native: { ok: true }, workspaceRoot: root,
    });
    // `sessions_get` adds the transcript's gaps, which the status line does not read.
    const { gaps, ...head } = (await getJson<Record<string, unknown>>(daemon, "/api/sessions/large-run")).body;
    expect(compact.context?.session).toEqual(head);
    expect(gaps).toEqual({ droppedEvents: 0, uncheckedBoundaries: 0, codeRulesPendingBoundaries: 0 });
    expect(compact.context?.session?.audit).toEqual({
      workspaceRoot: root, scope: "repository", mode: "log", status: "failed", violationCount: 1, failureCount: 148,
      unlistedFailureCount: 140, blindFileCount: 2, gapCount: 5, outOfWorkRuleCount: 1, fileWorkStoppedRuleCount: 1,
    });
    for (const mismatch of [{ host: "claude-code" }, { repository: tmpdir() }, { sessionId: "missing" }]) {
      const wrong = new URLSearchParams({ repository: root, host: "omp", sessionId: "large-run", ...mismatch });
      expect((await getJson(daemon, `/api/presentation?${wrong}`)).body).toMatchObject({ context: { session: null } });
    }
    await sendJson(daemon, "/api/presentation-preferences", "PUT", { repository: root, host: "omp", presence: "silent" });
    expect((await getJson(daemon, `/api/presentation?${query}`)).body).toMatchObject({ presence: "silent", context: { session: null } });
    query.set("onDemand", "true");
    expect((await getJson(daemon, `/api/presentation?${query}`)).body).toMatchObject({ presence: "silent", context: { session: compact.context!.session } });
    // A host below the launch root reads the root's session and its silent preference.
    await mkdir(join(root, "nested"));
    query.set("repository", join(root, "nested"));
    expect((await getJson(daemon, `/api/presentation?${query}`)).body).toMatchObject({ presence: "silent", context: { session: compact.context!.session } });

  });

  it("reads an encoded session id through the read-only API", async () => {
    root = await mkdtemp(join(tmpdir(), "rerail-daemon-"));
    const daemon = await start();
    const store = new SqliteSessionStore(join(root, "rerail.sqlite"), { repositoryId: "test-repository" });
    const sessionId = "project/a session";
    store.openSession({ host: "claude-code", sessionId, cwd: "/workspace" }, 1_000);
    store.append({ sessionId, kind: "user-prompt", eventId: "encoded-event", observedAt: 1_001, payload: { text: "hello" } });
    store.close();

    const response = await getJson<{ events: Array<{ sessionId: string; eventId: string }> }>(
      daemon,
      `/api/sessions/${encodeURIComponent(sessionId)}/events`,
    );
    expect(response).toEqual({
      status: 200,
      body: {
        events: [{ sequence: 1, sessionId, eventId: "encoded-event", kind: "user-prompt", observedAt: 1_001, payload: { text: "hello" } }],
        nextAfter: 1,
        pageLimit: SESSION_EVENT_PAGE_LIMIT,
      },
    });
  });

  it("rejects malformed queries, unknown API routes, and mutating methods outside the API", async () => {
    root = await mkdtemp(join(tmpdir(), "rerail-daemon-"));
    const daemon = await start();
    expect(await getJson(daemon, "/api/sessions/s1/events?after=-1")).toMatchObject({ status: 400, body: { error: "invalid-request" } });
    expect(await getJson(daemon, "/api/sessions/s1/events?limit=abc")).toMatchObject({ status: 400 });
    expect(await getJson(daemon, "/api/nope")).toEqual({ status: 404, body: { error: "not-found" } });
    expect(await sendJson(daemon, "/api/sessions", "POST")).toMatchObject({ status: 404 });
    expect(await sendJson(daemon, "/", "POST")).toMatchObject({ status: 405, body: { error: "method-not-allowed" } });
  });

  it("writes and reads rules created by you and work links over HTTP with kind-specific validation", async () => {
    root = await mkdtemp(join(tmpdir(), "rerail-daemon-"));
    const daemon = await start();
    const codeRule = MOCK_CODE_RULES[0]!;
    const conversationRule = MOCK_CONVERSATION_RULES[0]!;
    expect(await getJson(daemon, "/api/library-rules")).toEqual({ status: 200, body: { rules: [], quarantined: [] } });
    for (const removed of ["/api/rule-context", "/api/rules"]) expect((await getJson(daemon, removed)).status).toBe(404);

    const created = await sendJson(daemon, `/api/library-rules/${codeRule.id}`, "PUT", { kind: "code", rule: codeRule });
    expect(created).toMatchObject({ status: 200, body: { rule: { kind: "code", revision: codeRule.revision, rule: codeRule, copiedFromRuleId: null }, attached: null } });
    expect(await sendJson(daemon, `/api/library-rules/${conversationRule.id}`, "PUT", { kind: "conversation", rule: conversationRule })).toMatchObject({
      status: 200, body: { rule: { kind: "conversation", revision: 1, rule: conversationRule } },
    });

    expect(await sendJson(daemon, "/api/library-rules/other", "PUT", { kind: "code", rule: codeRule })).toMatchObject({ status: 400, body: { error: "rule-id-mismatch" } });
    expect(await sendJson(daemon, `/api/library-rules/${conversationRule.id}`, "PUT", { kind: "code", rule: conversationRule })).toMatchObject({ status: 400, body: { error: "invalid-request" } });
    expect(await sendJson(daemon, `/api/library-rules/${conversationRule.id}`, "PUT", { kind: "conversation", rule: { ...conversationRule, match: { operator: "gte", threshold: 2 } } }))
      .toMatchObject({ status: 400, body: { error: "invalid-request" } });
    // One id keeps one kind.
    expect(await sendJson(daemon, `/api/library-rules/${conversationRule.id}`, "PUT", { kind: "code", rule: { ...codeRule, id: conversationRule.id } })).toMatchObject({ status: 409, body: { error: "rule-conflict" } });
    // Attaching names a repository Rerail knows.
    expect(await sendJson(daemon, `/api/library-rules/${codeRule.id}`, "PUT", { kind: "code", rule: codeRule, attach: true })).toMatchObject({ status: 400, body: { error: "target-required" } });

    expect((await getJson<{ rules: Array<{ rule: { id: string } }> }>(daemon, "/api/library-rules?kind=conversation")).body.rules.map(stored => stored.rule.id)).toEqual([conversationRule.id]);
    expect(await getJson(daemon, "/api/library-rules?kind=nope")).toMatchObject({ status: 400 });
    expect(await sendJson(daemon, `/api/library-rules/${conversationRule.id}`, "DELETE")).toEqual({ status: 200, body: { removed: 1 } });
    expect(await sendJson(daemon, `/api/library-rules/${conversationRule.id}`, "DELETE")).toMatchObject({ status: 404, body: { error: "rule-not-found" } });

    expect(await sendJson(daemon, "/api/sessions/s1/work-links", "PUT", { kind: "linear-issue", reference: "ABC-1", url: "https://linear.app/example/issue/ABC-1" })).toMatchObject({
      status: 200,
      body: { workLinks: [{ sessionId: "s1", kind: "linear-issue", reference: "ABC-1", url: "https://linear.app/example/issue/ABC-1" }] },
    });
    expect(await sendJson(daemon, "/api/sessions/s1/work-links", "PUT", { kind: "git-branch" })).toMatchObject({ status: 400 });
    expect((await getJson<{ workLinks: unknown[] }>(daemon, "/api/sessions/s1/work-links")).body.workLinks).toHaveLength(1);
    expect((await getJson(daemon, "/healthz")).body).toMatchObject({ database: { exists: true } });
  });

  it("saves workspace defaults and rejects unsupported one-shot audit controls", async () => {
    root = await realpath(await mkdtemp(join(tmpdir(), "rerail-daemon-")));
    startTestRepository(root, join(root, "rerail.sqlite"));
    const daemon = await start();
    await ingestEngineEvent(daemon.kernelSocketPath, {
      kind: "session-start", eventId: "start-controls",
      session: { host: "claude-code", sessionId: "audit-session", cwd: root },
    });
    expect(await sendJson(daemon, "/api/sessions/audit-session/audit-settings", "PATCH", { scope: "repository", mode: "log" })).toMatchObject({
      status: 200,
      body: { workspaceRoot: root, active: true, delivery: "next-event", settings: { scope: "repository", mode: "log" } },
    });
    expect(await sendJson(daemon, "/api/sessions/audit-session/audit-settings", "PATCH", { initialMode: "resteer" })).toMatchObject({
      status: 400,
      body: { error: "invalid-request" },
    });
    expect(await sendJson(daemon, "/api/sessions/audit-session/audit-requests", "POST", { scope: "ai-output" })).toMatchObject({
      status: 400,
      body: { error: "invalid-request" },
    });
    expect(await sendJson(daemon, "/api/sessions/audit-session/audit-requests", "POST", { scope: "repository", mode: "steer" })).toMatchObject({
      status: 400,
      body: { error: "invalid-request" },
    });
    expect(await getJson(daemon, "/api/sessions/audit-session/audit-requests")).toMatchObject({
      status: 200,
      body: { auditRequests: [] },
    });
    expect(await getJson(daemon, "/api/sessions/audit-session/audit-settings")).toMatchObject({
      status: 200,
      body: { settings: { scope: "repository", mode: "log" } },
    });
    expect(await getJson(daemon, "/api/sessions/missing/audit-settings")).toMatchObject({
      status: 404,
      body: { error: "session-not-found" },
    });
  });

  it("lists machine-wide history by default and one repository's with a repository filter, and reads any session by id", async () => {
    root = await mkdtemp(join(tmpdir(), "rerail-daemon-"));
    const daemon = await start();
    const path = join(root, "rerail.sqlite");
    const own = new SqliteSessionStore(path, { repositoryId: "test-repository" });
    const other = new SqliteSessionStore(path, { repositoryId: "other-repository" });
    own.openSession({ host: "omp", sessionId: "own", cwd: join(root, "own") }, 1_000);
    other.openSession({ host: "omp", sessionId: "foreign", cwd: join(root, "foreign") }, 2_000);
    own.recordSteer({ host: "omp", sessionId: "own", cwd: join(root, "own") }, "own-receipt", ["shared"]);
    other.recordSteer({ host: "omp", sessionId: "foreign", cwd: join(root, "foreign") }, "foreign-receipt", ["shared"]);
    own.close();
    other.close();
    const auditStore = new SqliteAuditStore(path);
    const violation = {
      id: "violation", ruleId: "shared", clauseId: "clause", code: "CODE", mode: "steer" as const,
      message: "m", path: "a.ts", captures: {}, variables: {},
    };
    for (const [sessionId, workspaceRoot] of [["own", join(root, "own")], ["foreign", join(root, "foreign")]] as const) {
      const audit = auditStore.start({
        sessionId, workspaceRoot, scope: "repository", mode: "log", revision: 1, catalogRevision: null,
        ruleRevisions: [{ ruleId: "shared", revision: 1 }], paths: [],
      });
      auditStore.finish(audit, { status: "completed", violations: [{ ...violation, id: sessionId, path: `${sessionId}.ts` }] });
    }
    auditStore.close();

    const sessionIds = async (query: string) =>
      (await getJson<{ sessions: Array<{ sessionId: string }> }>(daemon, `/api/sessions${query}`)).body.sessions.map(session => session.sessionId);
    // Newest first: `foreign` opened after `own`.
    expect(await sessionIds("")).toEqual(["foreign", "own"]);
    expect(await sessionIds("?since=0&repository=test-repository")).toEqual(["own"]);
    expect((await getJson(daemon, "/api/audits")).body).toMatchObject({ total: 2 });
    expect((await getJson(daemon, "/api/audits?repository=test-repository")).body)
      .toMatchObject({ total: 1, audits: [expect.objectContaining({ sessionId: "own" })] });
    expect((await getJson(daemon, "/api/audits?sessionId=foreign&repository=test-repository")).body).toEqual({ audits: [], total: 0 });
    expect((await getJson(daemon, "/api/violations?since=0&repository=other-repository")).body)
      .toMatchObject({ total: 1, violations: [expect.objectContaining({ sessionId: "foreign" })] });
    const repositoryAudits = await getJson<{ repositories: Array<{ workspaceRoot: string }> }>(daemon, "/api/repository-audits?repository=test-repository");
    expect(repositoryAudits.body.repositories.map(repository => repository.workspaceRoot)).toEqual([join(root, "own")]);
    // Session-addressed reads find a session whichever repository recorded it.
    expect(await getJson(daemon, "/api/sessions/foreign")).toMatchObject({ status: 200, body: { session: { repositoryId: "other-repository" } } });
  });

  it("lists durable audit history through the read-only API", async () => {
    root = await mkdtemp(join(tmpdir(), "rerail-daemon-"));
    const daemon = await start();
    expect(await getJson(daemon, "/api/audits")).toEqual({ status: 200, body: { audits: [], total: 0 } });

    const store = new SqliteSessionStore(join(root, "rerail.sqlite"), { repositoryId: "test-repository" });
    store.openSession({ host: "omp", sessionId: "s1", cwd: "/workspace" });
    store.close();
    const audits = new SqliteAuditStore(join(root, "rerail.sqlite"));
    const auditId = audits.start({
      sessionId: "s1",
      workspaceRoot: "/workspace",
      scope: "touched-files",
      mode: "log",
      revision: 4,
      catalogRevision: null,
      ruleRevisions: [],
      paths: ["src/a.ts"],
      ruleProvenance: [],
    });
    audits.finish(auditId, {
      status: "failed",
      failures: [{ clauseId: "clause-a", code: "adapter-error", message: "native error", path: "src/a.ts", stage: "scan" }],
    });
    audits.close();

    expect(await getJson(daemon, "/api/audits?sessionId=s1&scope=touched-files&mode=log&limit=1")).toMatchObject({
      status: 200,
      body: {
        total: 1,
        audits: [expect.objectContaining({
          auditId: auditId,
          status: "failed",
          scope: "touched-files",
          mode: "log",
          paths: ["src/a.ts"],
          violations: [],
          failures: [{ clauseId: "clause-a", code: "adapter-error", message: "native error", path: "src/a.ts", stage: "scan" }],
          catalogRevision: null,
          ruleRevisions: [],
        })],
      },
    });
    expect(await getJson(daemon, "/api/audits?limit=201")).toMatchObject({ status: 400, body: { error: "invalid-request" } });
    expect(await getJson(daemon, "/api/audits?scope=delivery")).toMatchObject({ status: 400, body: { error: "invalid-request" } });
  });

  it("queries one full audit per historically audited repository", async () => {
    root = await mkdtemp(join(tmpdir(), "rerail-daemon-"));
    const path = join(root, "rerail.sqlite");
    const sessionStore = new SqliteSessionStore(path, { repositoryId: "test-repository" });
    sessionStore.openSession({ host: "omp", sessionId: "repo-a-session", cwd: "/workspace/repo-a" });
    sessionStore.openSession({ host: "omp", sessionId: "repo-b-session", cwd: "/workspace/repo-b" });
    sessionStore.openSession({ host: "omp", sessionId: "repo-c-session", cwd: "/workspace/repo-c" });
    const controls = new SqliteAuditControlStore(path);
    expect(controls.claimRepositoryAudit("repo-a-session", "/workspace/repo-a")).toBe(true);
    const audits = new SqliteAuditStore(path);
    const repositoryAuditId = audits.start({
      sessionId: "repo-a-session",
      workspaceRoot: "/workspace/repo-a",
      scope: "repository",
      mode: "log",
      revision: 1,
      catalogRevision: "catalog-a",
      ruleRevisions: [{ ruleId: "security", revision: 2 }],
      paths: [],
    });
    audits.finish(repositoryAuditId, {
      status: "failed",
      violations: [
        {
          id: "unsafe-redirect",
          ruleId: "security",
          clauseId: "redirect",
          code: "UNSAFE_REDIRECT",
          mode: "block",
          message: "Validate redirect targets.",
          path: "src/routes.ts",
          captures: {},
          variables: {},
        },
        {
          id: "debug-log",
          ruleId: "quality",
          clauseId: "logging",
          code: "DEBUG_LOG",
          mode: "steer",
          message: "Remove debug logging.",
          path: "src/logger.ts",
          captures: {},
          variables: {},
        },
      ],
      failures: [{
        clauseId: "parse",
        code: "budget-exceeded",
        message: "Source exceeds the per-file byte budget",
        path: "generated/client.ts",
        stage: "read",
      }],
      // The run had 30 more failures than it lists; the repository listing counts them.
      unlistedFailureCount: 30,
    });
    controls.finishRepositoryAudit("repo-a-session", "/workspace/repo-a", repositoryAuditId, "failed", "Source exceeds the per-file byte budget");
    const historicalAuditId = audits.start({
      sessionId: "repo-b-session",
      workspaceRoot: "/workspace/repo-b",
      scope: "repository",
      mode: "steer",
      revision: 1,
      catalogRevision: null,
      ruleRevisions: [],
      paths: [],
    });
    audits.finish(historicalAuditId, { status: "completed" });
    const touchedAuditId = audits.start({
      sessionId: "repo-c-session",
      workspaceRoot: "/workspace/repo-c",
      scope: "touched-files",
      mode: "log",
      revision: 1,
      catalogRevision: null,
      ruleRevisions: [],
      paths: ["src/file.ts"],
    });
    audits.finish(touchedAuditId, { status: "completed" });
    audits.close(); controls.close(); sessionStore.close();
    const daemon = await start();

    expect(await getJson(daemon, "/api/repository-audits?limit=10")).toEqual({
      status: 200,
      body: {
        repositories: [
          {
            workspaceRoot: "/workspace/repo-a",
            repositoryAudit: {
              sessionId: "repo-a-session",
              auditId: repositoryAuditId,
              status: "failed",
              startedAt: expect.any(Number),
              finishedAt: expect.any(Number),
              error: "Source exceeds the per-file byte budget",
            },
            audit: {
              auditId: repositoryAuditId,
              scope: "repository",
              status: "failed",
              startedAt: expect.any(Number),
              finishedAt: expect.any(Number),
              revision: 1,
              catalogRevision: "catalog-a",
              ruleRevisionCount: 1,
              violationCount: 2,
              failureCount: 31,
              unlistedFailureCount: 30,
            },
          },
          {
            workspaceRoot: "/workspace/repo-b",
            repositoryAudit: null,
            audit: null,
          },
          {
            workspaceRoot: "/workspace/repo-c",
            repositoryAudit: null,
            audit: null,
          },
        ],
        total: 3,
      },
    });
    expect(await getJson(daemon, `/api/repository-audits/${repositoryAuditId}/entries?kind=violation&mode=block&path=routes&q=redirect&limit=20`)).toEqual({
      status: 200,
      body: {
        entries: [{
          kind: "violation",
          violation: expect.objectContaining({
            id: "unsafe-redirect",
            code: "UNSAFE_REDIRECT",
            mode: "block",
            path: "src/routes.ts",
            message: "Validate redirect targets.",
          }),
        }],
        total: 1,
      },
    });
    expect(await getJson(daemon, `/api/repository-audits/${repositoryAuditId}/entries?mode=steer`)).toMatchObject({
      status: 200,
      body: { entries: [{ kind: "violation", violation: { id: "debug-log", mode: "steer" } }], total: 1 },
    });
    expect(await getJson(daemon, `/api/repository-audits/${repositoryAuditId}/entries?severity=error`)).toMatchObject({
      status: 400,
      body: { error: "invalid-request" },
    });
    expect(await getJson(daemon, `/api/repository-audits/${historicalAuditId}/entries`)).toMatchObject({
      status: 404,
      body: { error: "repository-audit-not-found" },
    });
  });

  it("takes a free port when another process holds the default one and no port was asked for", async () => {
    root = await mkdtemp(join(tmpdir(), "rerail-daemon-"));
    const blocker = createServer();
    // Either this test holds the default port or another process already does; both leave it taken.
    const held = await new Promise<boolean>((resolveHeld) => {
      blocker.once("error", () => resolveHeld(false));
      blocker.listen(DEFAULT_DAEMON_PORT, "127.0.0.1", () => resolveHeld(true));
    });
    try {
      process.env.RERAIL_HOME = join(root, "home");
      const daemon = await RerailDaemon.start({
        version: "0.0.0-test",
        databasePath: join(root, "rerail.sqlite"),
        workspaceRoot: root,
        nativeHealth: healthyNative,
        accessToken: ACCESS,
      });
      daemons.push(daemon);
      expect(daemon.port).not.toBe(DEFAULT_DAEMON_PORT);
      expect((await getJson(daemon, "/healthz")).status).toBe(200);
    } finally {
      if (held) await new Promise<void>((resolveClosed) => blocker.close(() => resolveClosed()));
    }
  });

  it("refuses an explicit port another process holds", async () => {
    root = await mkdtemp(join(tmpdir(), "rerail-daemon-"));
    const blocker = createServer();
    await new Promise<void>((resolveListening) => blocker.listen(0, "127.0.0.1", () => resolveListening()));
    try {
      const { port } = blocker.address() as { port: number };
      await expect(start({ port })).rejects.toMatchObject({ code: "EADDRINUSE" });
    } finally {
      await new Promise<void>((resolveClosed) => blocker.close(() => resolveClosed()));
    }
  });

  it("comes back on the port it last listened on, and moves only when another process holds it", async () => {
    root = await mkdtemp(join(tmpdir(), "rerail-daemon-"));
    process.env.RERAIL_HOME = join(root, "home");
    const portFile = join(root, "daemon-port.json");
    const options = { version: "0.0.0-test", databasePath: join(root, "rerail.sqlite"), workspaceRoot: root, nativeHealth: healthyNative, portFile };
    const probe = createServer();
    await new Promise<void>((resolveListening) => probe.listen(0, "127.0.0.1", () => resolveListening()));
    const { port } = probe.address() as { port: number };
    await new Promise<void>((resolveClosed) => probe.close(() => resolveClosed()));

    // An explicit port is remembered, and a later start without one comes back on it, so dashboard links still work.
    const first = await RerailDaemon.start({ ...options, port });
    daemons.push(first);
    await first.close();
    const second = await RerailDaemon.start(options);
    daemons.push(second);
    expect(second.port).toBe(port);
    await second.close();

    const blocker = createServer();
    await new Promise<void>((resolveListening) => blocker.listen(port, "127.0.0.1", () => resolveListening()));
    try {
      const third = await RerailDaemon.start(options);
      daemons.push(third);
      expect(third.port).not.toBe(port);
      expect(JSON.parse(await readFile(portFile, "utf8"))).toEqual({ port: third.port });
    } finally {
      await new Promise<void>((resolveClosed) => blocker.close(() => resolveClosed()));
    }
  });

  // A port below 1024 is refused (EACCES) to an unprivileged process on most systems, as a port Windows reserved is.
  it("moves on from a remembered port it is refused", async (context) => {
    const probe = createServer();
    const { promise: probed, resolve: refused } = Promise.withResolvers<boolean>();
    probe.once("error", (error: NodeJS.ErrnoException) => refused(error.code === "EACCES"));
    probe.listen(1, "127.0.0.1", () => probe.close(() => refused(false)));
    if (!(await probed)) context.skip();
    root = await mkdtemp(join(tmpdir(), "rerail-daemon-"));
    process.env.RERAIL_HOME = join(root, "home");
    const portFile = join(root, "daemon-port.json");
    await writeFile(portFile, `${JSON.stringify({ port: 1 })}\n`);
    const daemon = await RerailDaemon.start({ version: "0.0.0-test", databasePath: join(root, "rerail.sqlite"), workspaceRoot: root, nativeHealth: healthyNative, portFile });
    daemons.push(daemon);
    expect(daemon.port).not.toBe(1);
  });

  it("releases its engine socket when a rollback holds the database, so the next start succeeds", async () => {
    root = await mkdtemp(join(tmpdir(), "rerail-daemon-"));
    const databasePath = join(root, "rerail.sqlite");
    const restoring = `${databasePath}.restoring`;
    openRerailDatabase(databasePath).$client.close();
    await writeFile(restoring, "");
    await expect(start()).rejects.toThrow(/putting back a copy/u);
    expect(existsSync(defaultEngineSocketPath(databasePath))).toBe(false);
    await rm(restoring);
    const daemon = await start();
    expect((await getJson(daemon, "/healthz")).status).toBe(200);
  });

  // Review of #49 (N1): a resource that fails to close does not keep the next one open.
  it("releases its engine socket on a failed start even when the native session server fails to close", async () => {
    root = await mkdtemp(join(tmpdir(), "rerail-daemon-"));
    const databasePath = join(root, "rerail.sqlite");
    const stateFile = join(root, "daemon.json");
    await mkdir(stateFile);
    const close = vi.spyOn(NativeSessionServer.prototype, "close").mockRejectedValueOnce(new Error("close failed"));
    const stderr = vi.spyOn(process.stderr, "write").mockReturnValue(true);
    await expect(start({ stateFile })).rejects.toThrow(/EISDIR|illegal operation on a directory/u);
    expect(close).toHaveBeenCalledTimes(1);
    expect(existsSync(defaultEngineSocketPath(databasePath))).toBe(false);
    // Review of #49 (N5): the failed close is on record.
    expect(stderr).toHaveBeenCalledWith("rerail daemon: could not close the native session server after a failed start: close failed\n");
    stderr.mockRestore();
  });

  it("publishes every operation in the OpenAPI document", async () => {
    root = await mkdtemp(join(tmpdir(), "rerail-daemon-"));
    const daemon = await start();
    const { status, body } = await getJson<{ openapi: string; paths: Record<string, Record<string, { operationId: string }>> }>(daemon, "/openapi.json");
    expect(status).toBe(200);
    expect(body.openapi).toBe("3.1.0");
    const operationIds = Object.values(body.paths).flatMap(methods => Object.values(methods).map(route => route.operationId)).sort();
    expect(operationIds).toEqual(operations.map(operation => operation.id).sort());
    expect(body.paths["/api/library-rules/{id}"]?.put).toBeDefined();
  });

  it("declines standalone MCP event streams instead of advertising a connection that immediately closes", async () => {
    root = await mkdtemp(join(tmpdir(), "rerail-daemon-"));
    const daemon = await start();
    const response = await fetch(new URL("/mcp", daemon.url), {
      headers: { Accept: "text/event-stream" },
      signal: AbortSignal.timeout(3_000),
    });
    await response.body?.cancel();
    expect(response.status).toBe(405);
    expect(response.headers.get("Allow")).toBe("POST");
  });

  it("exposes the same operations as MCP tools over Streamable HTTP", async () => {
    root = await mkdtemp(join(tmpdir(), "rerail-daemon-"));
    const daemon = await start();
    const client = new Client({ name: "rerail-test", version: "0.0.0" });
    const transport = new StreamableHTTPClientTransport(new URL("/mcp", daemon.url), { requestInit: { headers: { [ACCESS_HEADER]: ACCESS } } });
    await client.connect(transport as Parameters<Client["connect"]>[0]);
    try {
      const tools = await client.listTools();
      expect(tools.tools.map(tool => tool.name).sort()).toEqual(operations.map(operation => operation.id).sort());
      const upsert = tools.tools.find(tool => tool.name === "library_rules_upsert")!;
      expect(Object.keys(upsert.inputSchema.properties ?? {}).sort())
        .toEqual(["attach", "checkout", "folder", "id", "kind", "repository", "revision", "rule"]);
      expect(upsert.annotations).toMatchObject({ readOnlyHint: false, destructiveHint: false });

      const conversationRule = MOCK_CONVERSATION_RULES[1]!;
      const created = await client.callTool({ name: "library_rules_upsert", arguments: { id: conversationRule.id, kind: "conversation", rule: conversationRule } });
      expect(created.isError).toBeFalsy();
      expect(created.structuredContent).toMatchObject({ rule: { kind: "conversation", revision: 1, rule: conversationRule }, attached: null });
      const listed = await client.callTool({ name: "library_rules_list", arguments: {} });
      expect(listed.structuredContent).toMatchObject({ rules: [{ rule: { id: conversationRule.id } }] });
      const missing = await client.callTool({ name: "library_rules_get", arguments: { id: "nope" } });
      expect(missing.isError).toBe(true);
      expect(missing.content).toEqual([{ type: "text", text: "rule-not-found: library rule nope does not exist" }]);
      expect(await getJson(daemon, `/api/library-rules/${conversationRule.id}`)).toMatchObject({ status: 200 });
    } finally {
      await client.close();
    }
  });

  it("answers page routes with a plain 404 when it was started without a UI root", async () => {
    root = await mkdtemp(join(tmpdir(), "rerail-daemon-"));
    const daemon = await start();
    expect(await getJson(daemon, "/")).toMatchObject({ status: 404, body: { error: "not-found" } });
    expect(await getJson(daemon, "/sessions/abc")).toMatchObject({ status: 404, body: { error: "not-found" } });
  });

  it("supports bodyless HEAD probes on its read-only health endpoint", async () => {
    root = await mkdtemp(join(tmpdir(), "rerail-daemon-"));
    const daemon = await start();

    const response = await fetch(new URL("/healthz", daemon.url), { method: "HEAD" });
    expect(response.status).toBe(200);
    expect(await response.text()).toBe("");
  });

  it("serves the bundled UI with an SPA fallback and no path escape", async () => {
    root = await mkdtemp(join(tmpdir(), "rerail-daemon-"));
    const uiRoot = join(root, "ui");
    await mkdir(join(uiRoot, "assets"), { recursive: true });
    await writeFile(join(uiRoot, "index.html"), "<!doctype html><title>Rerail</title>");
    await writeFile(join(uiRoot, "assets", "app.js"), "console.log('rerail')");
    await writeFile(join(root, "secret.txt"), "nope");
    const daemon = await start({ uiRoot });

    const index = await fetch(daemon.url);
    expect(index.status).toBe(200);
    expect(index.headers.get("content-type")).toBe("text/html; charset=utf-8");
    // The page is a write-capable origin: never framed by another site, never sniffed into a script.
    expect(index.headers.get("content-security-policy")).toBe("frame-ancestors 'none'");
    expect(index.headers.get("x-frame-options")).toBe("DENY");
    expect(index.headers.get("x-content-type-options")).toBe("nosniff");
    expect(await index.text()).toContain("<title>Rerail</title>");

    const assetUrl = new URL("/assets/app.js", daemon.url);
    const asset = await fetch(assetUrl);
    expect(asset.headers.get("content-type")).toBe("text/javascript; charset=utf-8");
    expect(asset.headers.get("x-content-type-options")).toBe("nosniff");
    expect(asset.headers.get("content-security-policy")).toBeNull();
    // Unversioned URLs: a browser must revalidate, or it keeps the previous release (or, in --dev, the previous edit).
    expect(asset.headers.get("cache-control")).toBe("no-cache");
    expect(await asset.text()).toBe("console.log('rerail')");
    const etag = asset.headers.get("etag")!;
    const unchanged = await fetch(assetUrl, { headers: { "if-none-match": etag } });
    expect(unchanged.status).toBe(304);
    expect(await unchanged.text()).toBe("");
    await writeFile(join(uiRoot, "assets", "app.js"), "console.log('rerail, edited')");
    const edited = await fetch(assetUrl, { headers: { "if-none-match": etag } });
    expect(edited.status).toBe(200);
    expect(await edited.text()).toBe("console.log('rerail, edited')");

    const route = await fetch(new URL("/sessions/abc", daemon.url));
    expect(route.status).toBe(200);
    expect(await route.text()).toContain("<title>Rerail</title>");

    expect((await fetch(new URL("/assets/missing.js", daemon.url))).status).toBe(404);
    expect((await fetch(new URL("/..%2Fsecret.txt", daemon.url))).status).toBe(404);
    expect((await fetch(`${daemon.url}assets/../../secret.txt`)).status).toBe(404);
    expect((await getJson(daemon, "/healthz")).body).toMatchObject({ ui: "bundled" });
  });

  it("writes its state file while listening and removes it on close", async () => {
    root = await mkdtemp(join(tmpdir(), "rerail-daemon-"));
    const stateFile = join(root, "daemon.json");
    const daemon = await start({ stateFile });
    expect(existsSync(stateFile)).toBe(true);
    expect(daemon.state()).toMatchObject({ pid: process.pid, port: daemon.port, url: daemon.url, ui: "dev", version: "0.0.0-test" });
    await daemon.close();
    expect(existsSync(stateFile)).toBe(false);
    await expect(fetch(daemon.url)).rejects.toThrow();
  });

  it("accepts API writes whose Origin is a known host on the requested port and refuses every other", async () => {
    root = await mkdtemp(join(tmpdir(), "rerail-daemon-"));
    const daemon = await start();
    const remove = (headers: Record<string, string>) => rawRequest(daemon, "/api/presentation-preferences?repository=&host=", { method: "DELETE", headers });
    const bound = `127.0.0.1:${daemon.port}`;

    expect(await remove({ origin: `http://${bound}`, host: bound })).toEqual({ status: 200, body: { removed: 0 } });

    // Reached through a container port mapping, the browser addresses a port the daemon never bound.
    for (const host of ["localhost:35831", "127.0.0.1:35831", "[::1]:35831"]) {
      expect(await remove({ origin: `http://${host}`, host })).toEqual({ status: 200, body: { removed: 0 } });
    }

    const forbidden = { status: 403, body: { error: "api-origin-forbidden" } };
    expect(await remove({ origin: "https://evil.example", host: bound })).toEqual(forbidden);
    expect(await remove({ origin: "http://localhost:35831", host: bound })).toEqual(forbidden);
  });

  it("answers 421 before any route when Host is not a name the daemon is known by", async () => {
    root = await mkdtemp(join(tmpdir(), "rerail-daemon-"));
    const uiRoot = join(root, "ui");
    await mkdir(uiRoot, { recursive: true });
    await writeFile(join(uiRoot, "index.html"), "<!doctype html><title>Rerail</title>");
    const daemon = await start({ uiRoot });
    const misdirected = { status: 421, body: { error: "host-not-allowed" } };
    const mcpCall = JSON.stringify({ jsonrpc: "2.0", id: 1, method: "tools/list" });

    // A DNS-rebinding page reaches the loopback socket under its own name; that name is the only tell.
    // (Node itself answers a missing, empty, or malformed `Host` with 400 before the app runs.)
    for (const host of ["evil.example:4737", `evil.example:${daemon.port}`, "evil.example"]) {
      expect(await rawRequest(daemon, "/api/sessions/x/events", { headers: { host } })).toEqual(misdirected);
      expect(await rawRequest(daemon, "/healthz", { headers: { host } })).toEqual(misdirected);
      expect(await rawRequest(daemon, "/", { headers: { host } })).toEqual(misdirected);
      expect(await rawRequest(daemon, "/mcp", { method: "POST", headers: { host, accept: "application/json, text/event-stream" }, body: mcpCall })).toEqual(misdirected);
      expect(await rawRequest(daemon, "/native/session", { method: "POST", headers: { host }, body: "{}" })).toEqual(misdirected);
    }

    // Every loopback spelling on any port, since a port mapping may present one the daemon never bound.
    for (const host of ["localhost:4737", "127.0.0.1:4737", "[::1]:4737", "LOCALHOST", `127.0.0.1:${daemon.port}`]) {
      expect(await rawRequest(daemon, "/healthz", { headers: { host } })).toMatchObject({ status: 200, body: { status: "ok" } });
      expect(await rawRequest(daemon, "/api/sessions", { headers: { host } })).toEqual({ status: 200, body: { sessions: [], total: 0, nextCursor: null } });
    }
    expect((await rawRequest(daemon, "/", { headers: { host: "localhost:4737" } })).status).toBe(200);
  });

  it("also answers to the hostnames named by --public-host", async () => {
    root = await mkdtemp(join(tmpdir(), "rerail-daemon-"));
    const daemon = await start({ publicHosts: ["Rerail.Example"] });
    expect(await rawRequest(daemon, "/healthz", { headers: { host: "rerail.example:8443" } })).toMatchObject({ status: 200 });
    expect(await rawRequest(daemon, "/healthz", { headers: { host: "127.0.0.1:4737" } })).toMatchObject({ status: 200 });
    expect(await rawRequest(daemon, "/healthz", { headers: { host: "evil.example" } })).toEqual({ status: 421, body: { error: "host-not-allowed" } });
    const remove = "/api/presentation-preferences?repository=&host=";
    expect(await rawRequest(daemon, remove, {
      method: "DELETE", headers: { origin: "https://rerail.example", host: "rerail.example" },
    })).toEqual({ status: 200, body: { removed: 0 } });
    expect(await rawRequest(daemon, remove, {
      method: "DELETE", headers: { origin: "https://evil.example", host: "rerail.example" },
    })).toEqual({ status: 403, body: { error: "api-origin-forbidden" } });
  });

  it("answers under its own bound address when started off loopback, so its state URL stays dialable", async () => {
    root = await mkdtemp(join(tmpdir(), "rerail-daemon-"));
    const daemon = await start({ host: "0.0.0.0" });
    expect(daemon.url).toBe(`http://0.0.0.0:${daemon.port}/`);
    // `lookupDaemon` and the container health check dial exactly this URL, sending `Host: 0.0.0.0:<port>`.
    const probed = await fetch(new URL("/healthz", daemon.url));
    expect(probed.status).toBe(200);
    expect(await rawRequest(daemon, "/healthz", { headers: { host: "0.0.0.0:4737" } })).toMatchObject({ status: 200 });
    expect(await rawRequest(daemon, "/healthz", { headers: { host: "evil.example" } })).toEqual({ status: 421, body: { error: "host-not-allowed" } });
  });

  it("requires its access capability on every request, on loopback too", async () => {
    root = await mkdtemp(join(tmpdir(), "rerail-daemon-"));
    const daemon = await start();
    const host = "127.0.0.1";
    const refused = { status: 401, body: { error: "access-required", message: expect.stringContaining("x-rerail-access") } };
    // `Host: 127.0.0.1` and a matching `Origin` are only claims; without the capability nothing answers, not even health.
    for (const [path, method] of [["/healthz", "GET"], ["/api/sessions", "GET"], ["/openapi.json", "GET"], ["/", "GET"], ["/mcp", "POST"], ["/native/session", "POST"]] as const) {
      const body = method === "POST" ? { body: "{}" } : {};
      expect(await rawRequest(daemon, path, { method, headers: { host, origin: `http://${host}`, [ACCESS_HEADER]: "" }, ...body })).toEqual(refused);
      expect(await rawRequest(daemon, path, { method, headers: { host, [ACCESS_HEADER]: `${ACCESS}x` }, ...body })).toEqual(refused);
    }
    expect(await rawRequest(daemon, "/api/sessions", { headers: { host } })).toEqual({ status: 200, body: { sessions: [], total: 0, nextCursor: null } });
    expect((await rawRequest(daemon, "/api/sessions", { headers: { host, [ACCESS_HEADER]: "", cookie: `other=1; rerail-access=${ACCESS}` } })).status).toBe(200);
    // The capability guards the native bridge too, on top of that bridge's own bearer token.
    expect((await rawRequest(daemon, "/native/session", { method: "POST", headers: { host }, body: "{}" })).body).toEqual({ error: "native-authorization-invalid" });

    // The printed link sets the cookie and drops the capability from the address, staying on this daemon.
    const linked = Promise.withResolvers<IncomingMessage>();
    httpRequest({ host, port: daemon.port, path: `//evil.example/session?repository=r&rerail-access=${ACCESS}` }, linked.resolve)
      .on("error", linked.reject).end();
    const link = await linked.promise;
    link.resume();
    expect(link.statusCode).toBe(303);
    expect(link.headers.location).toBe("/evil.example/session?repository=r");
    expect(link.headers["set-cookie"]).toEqual([`rerail-access=${ACCESS}; Path=/; Max-Age=2592000; HttpOnly; SameSite=Lax`]);
    // A browser that already holds the cookie still has the capability taken out of its address bar.
    expect((await rawRequest(daemon, `/?rerail-access=${ACCESS}`, { headers: { host, [ACCESS_HEADER]: "", cookie: `rerail-access=${ACCESS}` } })).status).toBe(303);
    // Behind an https proxy the cookie is marked Secure.
    const forwarded = Promise.withResolvers<IncomingMessage>();
    httpRequest({ host, port: daemon.port, path: `/?rerail-access=${ACCESS}`, headers: { "x-forwarded-proto": "https" } }, forwarded.resolve)
      .on("error", forwarded.reject).end();
    const secure = await forwarded.promise;
    secure.resume();
    expect(secure.headers["set-cookie"]).toEqual([`rerail-access=${ACCESS}; Path=/; Max-Age=2592000; HttpOnly; SameSite=Lax; Secure`]);
    // Only a page load takes it from the query; a write must hold the capability already.
    expect((await rawRequest(daemon, `/api/presentation-preferences?repository=&host=&rerail-access=${ACCESS}`, { method: "DELETE", headers: { host, [ACCESS_HEADER]: "" } })).status).toBe(401);
  });

  it("keeps a loopback daemon's capability in its home across restarts, and mints one per start off loopback", async () => {
    root = await mkdtemp(join(tmpdir(), "rerail-daemon-"));
    await mkdir(join(root, "home"), { recursive: true });
    const accessTokenFile = join(root, "home", "access-token");
    const first = await start({ accessTokenFile }, true);
    const token = first.state().accessToken!;
    expect(token).toMatch(/^[\w-]{43}$/u);
    expect((await readFile(accessTokenFile, "utf8")).trim()).toBe(token);
    if (process.platform !== "win32") expect((await stat(accessTokenFile)).mode & 0o777).toBe(0o600);
    await first.close();
    const second = await start({ accessTokenFile }, true);
    expect(second.state().accessToken).toBe(token);
    expect((await rawRequest(second, "/healthz", { headers: { host: "127.0.0.1", [ACCESS_HEADER]: token } })).status).toBe(200);
    await second.close();
    // Reachable from other machines, the capability is per launch and the kept one is left alone.
    const remote = await start({ accessTokenFile, host: "0.0.0.0" }, true);
    expect(remote.state().accessToken).not.toBe(token);
    expect((await readFile(accessTokenFile, "utf8")).trim()).toBe(token);
  });
});
