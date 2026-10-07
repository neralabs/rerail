import { existsSync } from "node:fs";
import { request as httpRequest } from "node:http";
import { mkdtemp, mkdir, realpath, rm, symlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { StreamableHTTPClientTransport } from "@modelcontextprotocol/sdk/client/streamableHttp.js";
import { afterEach, describe, expect, it } from "vitest";
import { publicHostSet } from "../src/api/http.js";
import { operations } from "../src/api/operations.js";
import { presentationResponseSchema } from "../src/api/session-contracts.js";
import { assertHostBindable, parsePublicHosts, RerailDaemon, type DaemonHealth, type RerailDaemonOptions } from "../src/daemon.js";
import { ingestKernelEvent } from "../src/kernel-service.js";
import { MOCK_COSINE_RULES, MOCK_POLICY_RULES } from "../src/seed.js";
import { SqlitePolicyExecutionReader, SqlitePolicyExecutionStore } from "../src/policy-execution-store.js";
import { SqliteScanControlStore } from "../src/scan-control.js";
import { SqliteTranscriptStore, TRANSCRIPT_EVENT_PAGE_LIMIT } from "../src/transcript-store.js";
import { startTestRepositoryById } from "./support/started-repository.js";

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

describe("Rerail daemon", () => {
  let root: string;
  const daemons: RerailDaemon[] = [];

  async function start(overrides: Partial<RerailDaemonOptions> = {}): Promise<RerailDaemon> {
    process.env.RERAIL_HOME = join(root, "home");
    const daemon = await RerailDaemon.start({
      version: "0.0.0-test",
      databasePath: join(root, "rerail.sqlite"),
      workspaceRoot: root,
      repositoryContext: { repositoryId: "test-repository", workspaceRoot: root },
      port: 0,
      nativeHealth: healthyNative,
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

  it("serves sessions and paged events read from the transcript store", async () => {
    root = await mkdtemp(join(tmpdir(), "rerail-daemon-"));
    const daemon = await start();
    expect(await getJson(daemon, "/api/sessions")).toEqual({ status: 200, body: { sessions: [] } });

    const store = new SqliteTranscriptStore(join(root, "rerail.sqlite"), { repositoryId: "test-repository" });
    store.openSession({ host: "claude-code", sessionId: "s1", cwd: "/workspace" }, 1_000);
    store.append({ sessionId: "s1", kind: "user-prompt", eventId: "e1", observedAt: 1_001, payload: { text: "hello" } });
    store.append({ sessionId: "s1", kind: "call", eventId: "e2", observedAt: 1_002, payload: { toolName: "read" } });
    store.append({ sessionId: "s1", kind: "turn-end", eventId: "e3", observedAt: 1_003, payload: { text: "done" } });
    store.closeSession("s1", 2_000);
    store.close();

    expect(await getJson(daemon, "/api/sessions")).toEqual({
      status: 200,
      body: { sessions: [{
        sessionId: "s1", host: "claude-code", cwd: "/workspace", repositoryId: "test-repository", startedAt: 1_000, endedAt: 2_000, eventCount: 3,
      }] },
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
        pageLimit: TRANSCRIPT_EVENT_PAGE_LIMIT,
      },
    });
    const secondPage = await getJson<{ events: Array<{ eventId: string }>; nextAfter: number }>(daemon, "/api/sessions/s1/events?after=2");
    expect(secondPage.body.nextAfter).toBe(3);
    expect(secondPage.body.events.map(event => event.eventId)).toEqual(["e3"]);
    expect(await getJson(daemon, "/api/sessions/s1/events?after=3")).toMatchObject({ body: { events: [], nextAfter: 3 } });
    expect(await getJson(daemon, "/api/sessions/missing/events")).toMatchObject({ body: { events: [] } });
    expect(await getJson(daemon, "/api/sessions/s1")).toMatchObject({
      status: 200,
      body: { session: { sessionId: "s1", eventCount: 3 }, hud: { activeRuleCount: null, steeringCount: null } },
    });
    expect(await getJson(daemon, "/api/sessions/missing")).toMatchObject({ status: 404, body: { error: "session-not-found" } });
    expect((await getJson(daemon, "/healthz")).body).toMatchObject({ database: { exists: true } });
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
    expect((await getJson(daemon, `/api/presentation?${query}`)).body).toEqual({ presence: "compact", repositoryState: "never" });
    for (const preference of overrides) {
      const result = await sendJson(daemon, "/api/presentation-preferences", "PUT", preference);
      expect(result).toMatchObject({ status: 200, body: {
        preference: { ...preference, repository: preference.repository === "" ? "" : repository },
      } });
      expect((await getJson(daemon, `/api/presentation?${query}`)).body).toEqual({ presence: preference.presence, repositoryState: "never" });
    }
    for (let index = overrides.length - 1; index >= 0; index--) {
      const preference = overrides[index]!;
      const key = new URLSearchParams({ repository: preference.repository === "" ? "" : repository, host: preference.host });
      expect(await sendJson(daemon, `/api/presentation-preferences?${key}`, "DELETE")).toEqual({ status: 200, body: { removed: 1 } });
      expect((await getJson(daemon, `/api/presentation?${query}`)).body).toEqual({ presence: overrides[index - 1]?.presence ?? "compact", repositoryState: "never" });
    }
  });

  it("keeps session status bounded even when an evaluation contains multi-megabyte findings", async () => {
    root = await mkdtemp(join(tmpdir(), "rerail-daemon-"));
    const path = join(root, "rerail.sqlite");
    const transcripts = new SqliteTranscriptStore(path, { repositoryId: "test-repository" });
    transcripts.openSession({ host: "omp", sessionId: "large-run", cwd: root }, 1);
    transcripts.startHudTracking({ host: "omp", sessionId: "large-run", cwd: root }, true);
    transcripts.close();
    const executions = new SqlitePolicyExecutionStore(path);
    const run = executions.start({ sessionId: "large-run", workspaceRoot: root, scope: "repository", mode: "log", revision: 0, paths: [], catalogRevision: "test", ruleRevisions: [] });
    executions.finish(run, {
      status: "failed",
      violations: [{
        id: "large-finding", ruleId: "policy", clauseId: "check", code: "code",
        mode: "block", message: "x".repeat(2 * 1024 * 1024),
        path: "source.ts", captures: {}, variables: {},
      }],
      failures: [{ clauseId: "scan", code: "budget-exceeded", message: "scan exceeded its budget", stage: "scan" }],
    });
    executions.close();
    const daemon = await start();
    const query = new URLSearchParams({ repository: root, host: "omp", sessionId: "large-run" });
    const response = await fetch(new URL(`/api/presentation?${query}`, daemon.url));
    const text = await response.text();
    expect(response.status).toBe(200);
    expect(Buffer.byteLength(text)).toBeLessThan(1024);
    const compact = presentationResponseSchema.parse(JSON.parse(text));
    expect(compact.context?.daemon).toEqual({
      pid: process.pid, database: { path, exists: true }, native: { ok: true }, workspaceRoot: root, repositoryId: "test-repository",
    });
    expect(compact.context?.session).toEqual((await getJson(daemon, "/api/sessions/large-run")).body);
    expect(compact.context?.session?.audit).toEqual({
      workspaceRoot: root, scope: "repository", mode: "log", status: "failed", violationCount: 1, failureCount: 1,
    });
    for (const mismatch of [{ host: "claude-code" }, { repository: tmpdir() }, { sessionId: "missing" }]) {
      const wrong = new URLSearchParams({ repository: root, host: "omp", sessionId: "large-run", ...mismatch });
      expect((await getJson(daemon, `/api/presentation?${wrong}`)).body).toMatchObject({ context: { session: null } });
    }
    await sendJson(daemon, "/api/presentation-preferences", "PUT", { repository: root, host: "omp", presence: "silent" });
    expect((await getJson(daemon, `/api/presentation?${query}`)).body).toMatchObject({ presence: "silent", context: { session: null } });
    query.set("onDemand", "true");
    expect((await getJson(daemon, `/api/presentation?${query}`)).body).toMatchObject({ presence: "silent", context: { session: compact.context!.session } });
    // A harness below the launch root reads the root's session and its silent preference.
    await mkdir(join(root, "nested"));
    query.set("repository", join(root, "nested"));
    expect((await getJson(daemon, `/api/presentation?${query}`)).body).toMatchObject({ presence: "silent", context: { session: compact.context!.session } });
  });

  it("reads an encoded transcript session id through the read-only API", async () => {
    root = await mkdtemp(join(tmpdir(), "rerail-daemon-"));
    const daemon = await start();
    const store = new SqliteTranscriptStore(join(root, "rerail.sqlite"), { repositoryId: "test-repository" });
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
        pageLimit: TRANSCRIPT_EVENT_PAGE_LIMIT,
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

  it("writes and reads rules and work links over HTTP with kind-specific validation", async () => {
    root = await mkdtemp(join(tmpdir(), "rerail-daemon-"));
    const daemon = await start();
    const policy = MOCK_POLICY_RULES[0]!;
    const cosine = MOCK_COSINE_RULES[0]!;
    expect(await getJson(daemon, "/api/rules")).toEqual({ status: 200, body: { rules: [], quarantined: [] } });
    expect(await getJson(daemon, "/api/rule-context")).toMatchObject({
      status: 200,
      body: { context: { repositoryId: "test-repository", projectId: "repository:test-repository" } },
    });
    expect(await sendJson(
      daemon,
      "/api/projects/project-shared/repositories/test-repository",
      "PUT",
      { projectName: "Shared project" },
    )).toEqual({ status: 200, body: { projectId: "project-shared", repositoryId: "test-repository" } });
    expect(await getJson(daemon, "/api/rule-context")).toMatchObject({
      body: { context: { projectId: "project-shared", projectName: "Shared project" } },
    });

    const created = await sendJson<{ rule: { kind: string; enabled: boolean; revision: number } }>(daemon, `/api/rules/${policy.id}`, "PUT", { kind: "file", rule: policy });
    expect(created.status).toBe(200);
    expect(created.body.rule).toMatchObject({ kind: "file", enabled: true, revision: policy.revision, rule: policy });
    expect(await sendJson(daemon, `/api/rules/${cosine.id}`, "PUT", { kind: "conversation", rule: cosine, enabled: false, applicability: "global" })).toMatchObject({
      status: 200,
      body: { rule: { kind: "conversation", enabled: false, applicability: "global", origin: { kind: "global" }, revision: 1, rule: cosine } },
    });

    expect(await sendJson(daemon, "/api/rules/other", "PUT", { kind: "file", rule: policy })).toMatchObject({ status: 400, body: { error: "rule-id-mismatch" } });
    expect(await sendJson(daemon, `/api/rules/${cosine.id}`, "PUT", { kind: "file", rule: cosine })).toMatchObject({ status: 400, body: { error: "invalid-request" } });
    expect(await sendJson(daemon, `/api/rules/${cosine.id}`, "PUT", { kind: "conversation", rule: { ...cosine, match: { operator: "gte", threshold: 2 } } })).toMatchObject({ status: 400, body: { error: "invalid-request" } });
    expect(await sendJson(daemon, `/api/rules/${cosine.id}`, "PUT", { kind: "file", rule: { ...policy, id: cosine.id } })).toMatchObject({ status: 409, body: { error: "rule-conflict" } });

    expect((await getJson<{ rules: Array<{ rule: { id: string } }> }>(daemon, "/api/rules?enabled=false")).body.rules.map(stored => stored.rule.id)).toEqual([cosine.id]);
    expect((await getJson<{ rules: unknown[] }>(daemon, "/api/rules?kind=file&applicability=project")).body.rules).toHaveLength(1);
    expect(await getJson(daemon, "/api/rules?kind=nope")).toMatchObject({ status: 400 });
    expect(await sendJson(daemon, "/api/rules", "PATCH", { ids: [cosine.id, "missing"], enabled: true })).toEqual({ status: 200, body: { changed: 1 } });
    expect((await getJson<{ rules: Array<{ rule: { id: string } }> }>(daemon, "/api/rules?q=console&mode=log")).body.rules).toEqual([]);
    expect((await getJson<{ rules: Array<{ rule: { id: string } }> }>(daemon, "/api/rules?mode=block")).body.rules).toEqual([]);
    expect((await getJson<{ rules: Array<{ rule: { id: string } }> }>(daemon, "/api/rules?mode=steer")).body.rules.map(stored => stored.rule.id)).toEqual([cosine.id, policy.id]);
    expect(await getJson(daemon, "/api/rules?delivery=steer")).toMatchObject({ status: 400, body: { error: "invalid-request" } });
    expect((await getJson<{ rules: Array<{ rule: { id: string } }> }>(daemon, "/api/rules?q=CONSOLE")).body.rules.map(stored => stored.rule.id)).toEqual([policy.id]);
    expect((await getJson<{ rules: Array<{ rule: { id: string } }> }>(daemon, "/api/rules?source=assistant-text&boundary=sentence")).body.rules.map(stored => stored.rule.id)).toEqual([cosine.id]);
    expect(await sendJson(daemon, "/api/rules", "PATCH", { filter: { q: "console" }, enabled: false })).toEqual({ status: 200, body: { changed: 1 } });
    expect(await sendJson(daemon, "/api/rules", "PATCH", { filter: {}, enabled: true })).toEqual({ status: 200, body: { changed: 1 } });
    expect(await sendJson(daemon, "/api/rules", "PATCH", { enabled: true })).toMatchObject({ status: 400, body: { error: "invalid-request" } });
    expect(await sendJson(daemon, "/api/rules", "PATCH", { ids: ["x"], filter: {}, enabled: true })).toMatchObject({ status: 400 });
    expect(await getJson(daemon, `/api/rules/${cosine.id}`)).toMatchObject({ body: { rule: { enabled: true, applicability: "project" } } });
    expect(await sendJson(daemon, `/api/rules/${cosine.id}?applicability=global`, "DELETE")).toEqual({ status: 200, body: { removed: 1 } });
    expect(await getJson(daemon, `/api/rules/${cosine.id}`)).toMatchObject({ body: { rule: { enabled: true, applicability: "project" } } });
    expect(await sendJson(daemon, `/api/rules/${cosine.id}`, "DELETE")).toEqual({ status: 200, body: { removed: 1 } });
    expect(await sendJson(daemon, `/api/rules/${cosine.id}`, "DELETE")).toMatchObject({ status: 404, body: { error: "rule-not-found" } });

    expect(await sendJson(daemon, "/api/sessions/s1/work-links", "PUT", { kind: "linear-issue", reference: "ABC-1", url: "https://linear.app/example/issue/ABC-1" })).toMatchObject({
      status: 200,
      body: { workLinks: [{ sessionId: "s1", kind: "linear-issue", reference: "ABC-1", url: "https://linear.app/example/issue/ABC-1" }] },
    });
    expect(await sendJson(daemon, "/api/sessions/s1/work-links", "PUT", { kind: "git-branch" })).toMatchObject({ status: 400 });
    expect((await getJson<{ workLinks: unknown[] }>(daemon, "/api/sessions/s1/work-links")).body.workLinks).toHaveLength(1);
    expect((await getJson(daemon, "/healthz")).body).toMatchObject({ database: { exists: true } });
  });

  it("saves workspace defaults and rejects unsupported one-shot scan controls", async () => {
    root = await realpath(await mkdtemp(join(tmpdir(), "rerail-daemon-")));
    startTestRepositoryById("test-repository", join(root, "rerail.sqlite"));
    const daemon = await start();
    await ingestKernelEvent(daemon.kernelSocketPath, {
      kind: "session-start", eventId: "start-controls",
      session: { host: "claude-code", sessionId: "scan-session", cwd: root },
    });
    expect(await sendJson(daemon, "/api/sessions/scan-session/audit-settings", "PATCH", { scope: "repository", mode: "log" })).toMatchObject({
      status: 200,
      body: { workspaceRoot: root, active: true, delivery: "next-event", settings: { scope: "repository", mode: "log" } },
    });
    expect(await sendJson(daemon, "/api/sessions/scan-session/audit-settings", "PATCH", { initialMode: "resteer" })).toMatchObject({
      status: 400,
      body: { error: "invalid-request" },
    });
    expect(await sendJson(daemon, "/api/sessions/scan-session/audit-requests", "POST", { scope: "ai-output" })).toMatchObject({
      status: 400,
      body: { error: "invalid-request" },
    });
    expect(await sendJson(daemon, "/api/sessions/scan-session/audit-requests", "POST", { scope: "repository", mode: "steer" })).toMatchObject({
      status: 400,
      body: { error: "invalid-request" },
    });
    expect(await getJson(daemon, "/api/sessions/scan-session/audit-requests")).toMatchObject({
      status: 200,
      body: { auditRequests: [] },
    });
    expect(await getJson(daemon, "/api/sessions/scan-session/audit-settings")).toMatchObject({
      status: 200,
      body: { settings: { scope: "repository", mode: "log" } },
    });
    expect(await getJson(daemon, "/api/sessions/missing/audit-settings")).toMatchObject({
      status: 404,
      body: { error: "session-not-found" },
    });
  });

  it("lists only its own repository's history from the shared database but reads any session by id", async () => {
    root = await mkdtemp(join(tmpdir(), "rerail-daemon-"));
    const daemon = await start();
    const path = join(root, "rerail.sqlite");
    const own = new SqliteTranscriptStore(path, { repositoryId: "test-repository" });
    const other = new SqliteTranscriptStore(path, { repositoryId: "other-repository" });
    own.openSession({ host: "omp", sessionId: "own", cwd: join(root, "own") }, 1_000);
    other.openSession({ host: "omp", sessionId: "foreign", cwd: join(root, "foreign") }, 2_000);
    own.recordSteeringDelivery({ host: "omp", sessionId: "own", cwd: join(root, "own") }, "own-receipt", ["shared"]);
    other.recordSteeringDelivery({ host: "omp", sessionId: "foreign", cwd: join(root, "foreign") }, "foreign-receipt", ["shared"]);
    own.close();
    other.close();
    const executions = new SqlitePolicyExecutionStore(path);
    const violation = {
      id: "violation", ruleId: "shared", clauseId: "clause", code: "CODE", mode: "steer" as const,
      message: "m", path: "a.ts", captures: {}, variables: {},
    };
    for (const [sessionId, workspaceRoot] of [["own", join(root, "own")], ["foreign", join(root, "foreign")]] as const) {
      const run = executions.start({
        sessionId, workspaceRoot, scope: "repository", mode: "log", revision: 1, catalogRevision: null,
        ruleRevisions: [{ ruleId: "shared", revision: 1 }], paths: [],
      });
      executions.finish(run, { status: "completed", violations: [{ ...violation, id: sessionId, path: `${sessionId}.ts` }] });
    }
    executions.close();

    const sessions = await getJson<{ sessions: Array<{ sessionId: string }> }>(daemon, "/api/sessions?since=0");
    expect(sessions.body.sessions.map(session => session.sessionId)).toEqual(["own"]);
    expect((await getJson<{ sessions: Array<{ sessionId: string }> }>(daemon, "/api/sessions")).body.sessions.map(session => session.sessionId))
      .toEqual(["own"]);
    const audits = await getJson<{ audits: Array<{ sessionId: string }>; total: number }>(daemon, "/api/audits");
    expect(audits.body).toMatchObject({ total: 1, audits: [expect.objectContaining({ sessionId: "own" })] });
    expect((await getJson(daemon, "/api/audits?sessionId=foreign")).body).toEqual({ audits: [], total: 0 });
    const violations = await getJson<{ total: number; violations: Array<{ sessionId: string }> }>(daemon, "/api/violations?since=0");
    expect(violations.body).toMatchObject({ total: 1, violations: [expect.objectContaining({ sessionId: "own" })] });
    const repositoryAudits = await getJson<{ repositories: Array<{ workspaceRoot: string }> }>(daemon, "/api/repository-audits");
    expect(repositoryAudits.body.repositories.map(repository => repository.workspaceRoot)).toEqual([join(root, "own")]);
    const scoped = new SqlitePolicyExecutionReader(path, { repositoryId: "test-repository" });
    const machine = new SqlitePolicyExecutionReader(path);
    expect(scoped.ruleActivity(0).get("shared")).toMatchObject({ sessions: 1, audits: 1 });
    expect(machine.ruleActivity(0).get("shared")).toMatchObject({ sessions: 2, audits: 2 });
    scoped.close();
    machine.close();
    // Session-addressed reads find a session whichever repository recorded it.
    expect(await getJson(daemon, "/api/sessions/foreign")).toMatchObject({ status: 200, body: { session: { repositoryId: "other-repository" } } });
  });

  it("lists durable policy-run history through the read-only API", async () => {
    root = await mkdtemp(join(tmpdir(), "rerail-daemon-"));
    const daemon = await start();
    expect(await getJson(daemon, "/api/audits")).toEqual({ status: 200, body: { audits: [], total: 0 } });

    const store = new SqliteTranscriptStore(join(root, "rerail.sqlite"), { repositoryId: "test-repository" });
    store.openSession({ host: "omp", sessionId: "s1", cwd: "/workspace" });
    store.close();
    const executions = new SqlitePolicyExecutionStore(join(root, "rerail.sqlite"));
    const runId = executions.start({
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
    executions.finish(runId, {
      status: "failed",
      failures: [{ clauseId: "check-a", code: "adapter-error", message: "native error", path: "src/a.ts", stage: "scan" }],
    });
    executions.close();

    expect(await getJson(daemon, "/api/audits?sessionId=s1&scope=touched-files&mode=log&limit=1")).toMatchObject({
      status: 200,
      body: {
        total: 1,
        audits: [expect.objectContaining({
          auditId: runId,
          status: "failed",
          scope: "touched-files",
          mode: "log",
          paths: ["src/a.ts"],
          violations: [],
          failures: [{ clauseId: "check-a", code: "adapter-error", message: "native error", path: "src/a.ts", stage: "scan" }],
          catalogRevision: null,
          ruleRevisions: [],
        })],
      },
    });
    expect(await getJson(daemon, "/api/audits?limit=201")).toMatchObject({ status: 400, body: { error: "invalid-request" } });
    expect(await getJson(daemon, "/api/audits?scope=delivery")).toMatchObject({ status: 400, body: { error: "invalid-request" } });
  });

  it("queries one full diagnostic per historically scanned repository", async () => {
    root = await mkdtemp(join(tmpdir(), "rerail-daemon-"));
    const path = join(root, "rerail.sqlite");
    const transcripts = new SqliteTranscriptStore(path, { repositoryId: "test-repository" });
    transcripts.openSession({ host: "omp", sessionId: "repo-a-session", cwd: "/workspace/repo-a" });
    transcripts.openSession({ host: "omp", sessionId: "repo-b-session", cwd: "/workspace/repo-b" });
    transcripts.openSession({ host: "omp", sessionId: "repo-c-session", cwd: "/workspace/repo-c" });
    const controls = new SqliteScanControlStore(path);
    expect(controls.claimRepositoryDiagnostic("repo-a-session")).toBe(true);
    const executions = new SqlitePolicyExecutionStore(path);
    const baselineRunId = executions.start({
      sessionId: "repo-a-session",
      workspaceRoot: "/workspace/repo-a",
      scope: "repository",
      mode: "log",
      revision: 1,
      catalogRevision: "catalog-a",
      ruleRevisions: [{ ruleId: "security", revision: 2 }],
      paths: [],
    });
    executions.finish(baselineRunId, {
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
    });
    controls.finishRepositoryDiagnostic("repo-a-session", baselineRunId, "failed", "Source exceeds the per-file byte budget");
    const historicalRunId = executions.start({
      sessionId: "repo-b-session",
      workspaceRoot: "/workspace/repo-b",
      scope: "repository",
      mode: "steer",
      revision: 1,
      catalogRevision: null,
      ruleRevisions: [],
      paths: [],
    });
    executions.finish(historicalRunId, { status: "completed" });
    const touchedRunId = executions.start({
      sessionId: "repo-c-session",
      workspaceRoot: "/workspace/repo-c",
      scope: "touched-files",
      mode: "log",
      revision: 1,
      catalogRevision: null,
      ruleRevisions: [],
      paths: ["src/file.ts"],
    });
    executions.finish(touchedRunId, { status: "completed" });
    executions.close(); controls.close(); transcripts.close();
    const daemon = await start();

    expect(await getJson(daemon, "/api/repository-audits?limit=10")).toEqual({
      status: 200,
      body: {
        repositories: [
          {
            workspaceRoot: "/workspace/repo-a",
            repositoryAudit: {
              sessionId: "repo-a-session",
              auditId: baselineRunId,
              status: "failed",
              startedAt: expect.any(Number),
              finishedAt: expect.any(Number),
              error: "Source exceeds the per-file byte budget",
            },
            audit: {
              auditId: baselineRunId,
              scope: "repository",
              status: "failed",
              startedAt: expect.any(Number),
              finishedAt: expect.any(Number),
              revision: 1,
              catalogRevision: "catalog-a",
              ruleRevisionCount: 1,
              violationCount: 2,
              failureCount: 1,
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
    expect(await getJson(daemon, `/api/repository-audits/${baselineRunId}/entries?kind=violation&mode=block&path=routes&q=redirect&limit=20`)).toEqual({
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
    expect(await getJson(daemon, `/api/repository-audits/${baselineRunId}/entries?mode=steer`)).toMatchObject({
      status: 200,
      body: { entries: [{ kind: "violation", violation: { id: "debug-log", mode: "steer" } }], total: 1 },
    });
    expect(await getJson(daemon, `/api/repository-audits/${baselineRunId}/entries?severity=error`)).toMatchObject({
      status: 400,
      body: { error: "invalid-request" },
    });
    expect(await getJson(daemon, `/api/repository-audits/${historicalRunId}/entries`)).toMatchObject({
      status: 404,
      body: { error: "repository-audit-not-found" },
    });
  });

  it("publishes every operation in the OpenAPI document", async () => {
    root = await mkdtemp(join(tmpdir(), "rerail-daemon-"));
    const daemon = await start();
    const { status, body } = await getJson<{ openapi: string; paths: Record<string, Record<string, { operationId: string }>> }>(daemon, "/openapi.json");
    expect(status).toBe(200);
    expect(body.openapi).toBe("3.1.0");
    const operationIds = Object.values(body.paths).flatMap(methods => Object.values(methods).map(route => route.operationId)).sort();
    expect(operationIds).toEqual(operations.map(operation => operation.id).sort());
    expect(body.paths["/api/rules/{id}"]?.put).toBeDefined();
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
    await client.connect(new StreamableHTTPClientTransport(new URL("/mcp", daemon.url)) as Parameters<Client["connect"]>[0]);
    try {
      const tools = await client.listTools();
      expect(tools.tools.map(tool => tool.name).sort()).toEqual(operations.map(operation => operation.id).sort());
      const upsert = tools.tools.find(tool => tool.name === "rules_upsert")!;
      expect(Object.keys(upsert.inputSchema.properties ?? {}).sort()).toEqual(["applicability", "enabled", "id", "kind", "revision", "rule"]);
      expect(upsert.annotations).toMatchObject({ readOnlyHint: false, destructiveHint: false });

      const cosine = MOCK_COSINE_RULES[1]!;
      const created = await client.callTool({ name: "rules_upsert", arguments: { id: cosine.id, kind: "conversation", rule: cosine } });
      expect(created.isError).toBeFalsy();
      expect(created.structuredContent).toMatchObject({ rule: { kind: "conversation", enabled: true, rule: cosine } });
      const listed = await client.callTool({ name: "rules_list", arguments: { enabled: true } });
      expect((listed.structuredContent as { rules: unknown[] }).rules).toHaveLength(1);
      const missing = await client.callTool({ name: "rules_get", arguments: { id: "nope" } });
      expect(missing.isError).toBe(true);
      expect(missing.content).toEqual([{ type: "text", text: "rule-not-found: rule nope does not exist" }]);
      expect(await getJson(daemon, `/api/rules/${cosine.id}`)).toMatchObject({ status: 200 });
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
    const body = JSON.stringify({ enabled: false, ids: ["nothing-here"] });
    const patch = (headers: Record<string, string>) => rawRequest(daemon, "/api/rules", { method: "PATCH", headers, body });
    const bound = `127.0.0.1:${daemon.port}`;

    expect(await patch({ origin: `http://${bound}`, host: bound })).toEqual({ status: 200, body: { changed: 0 } });

    // Reached through a container port mapping, the browser addresses a port the daemon never bound.
    for (const host of ["localhost:35831", "127.0.0.1:35831", "[::1]:35831"]) {
      expect(await patch({ origin: `http://${host}`, host })).toEqual({ status: 200, body: { changed: 0 } });
    }

    const forbidden = { status: 403, body: { error: "api-origin-forbidden" } };
    expect(await patch({ origin: "https://evil.example", host: bound })).toEqual(forbidden);
    expect(await patch({ origin: "http://localhost:35831", host: bound })).toEqual(forbidden);
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
      expect(await rawRequest(daemon, "/api/sessions", { headers: { host } })).toEqual({ status: 200, body: { sessions: [] } });
    }
    expect((await rawRequest(daemon, "/", { headers: { host: "localhost:4737" } })).status).toBe(200);
  });

  it("also answers to the hostnames named by --public-host", async () => {
    root = await mkdtemp(join(tmpdir(), "rerail-daemon-"));
    const daemon = await start({ publicHosts: ["Rerail.Example"] });
    expect(await rawRequest(daemon, "/healthz", { headers: { host: "rerail.example:8443" } })).toMatchObject({ status: 200 });
    expect(await rawRequest(daemon, "/healthz", { headers: { host: "127.0.0.1:4737" } })).toMatchObject({ status: 200 });
    expect(await rawRequest(daemon, "/healthz", { headers: { host: "evil.example" } })).toEqual({ status: 421, body: { error: "host-not-allowed" } });
    const body = JSON.stringify({ enabled: false, ids: ["nothing-here"] });
    expect(await rawRequest(daemon, "/api/rules", {
      method: "PATCH", headers: { origin: "https://rerail.example", host: "rerail.example" }, body,
    })).toEqual({ status: 200, body: { changed: 0 } });
    expect(await rawRequest(daemon, "/api/rules", {
      method: "PATCH", headers: { origin: "https://evil.example", host: "rerail.example" }, body,
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
});
