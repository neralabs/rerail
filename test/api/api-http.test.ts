import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { OpenAPIHono } from "@hono/zod-openapi";
import Database from "better-sqlite3";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { createRerailApi } from "../../src/api/http.js";
import { operations, type RerailApiStores } from "../../src/api/operations.js";
import {
  openRerailDatabase,
  type RerailDatabase,
  SqliteAuditReader,
  SqliteAuditStore,
  SqliteAuditControlStore,
  ScopedRuleCatalog,
} from "../../src/engine/index.js";
import {
  startStateReader,
  SqlitePresentationPreferenceStore,
  SqliteSessionReader,
  SqliteSessionStore,
} from "../../src/runtime/index.js";
import type { CodeRuleViolation } from "../../src/engine/index.js";
import { MOCK_CONVERSATION_RULES, MOCK_CODE_RULES } from "../../src/cli/index.js";

/** The host a browser addressed the page to through a container port mapping; the fixture never binds it. */
const PAGE_HOST = "localhost:35831";
const SESSION = { host: "codex" as const, sessionId: "session-a", cwd: "/workspace" };

const flagged: CodeRuleViolation = {
  id: "violation-1",
  ruleId: MOCK_CODE_RULES[0]!.id,
  clauseId: "console-call",
  code: "forbidden-call",
  mode: "block",
  message: "console.log is not allowed",
  path: "web/src/x.ts",
  span: { startByte: 0, endByte: 9, startLine: 1, startColumn: 1, endLine: 1, endColumn: 9 },
  captures: {},
  variables: {},
};

describe("Rerail API over HTTP", () => {
  let root: string;
  let databasePath: string;
  let app: OpenAPIHono;
  let stores: RerailApiStores;
  let catalog: ScopedRuleCatalog;
  let audits: SqliteAuditStore;
  let sessionStore: SqliteSessionStore;
  let raw: Database.Database;
  let database: RerailDatabase;
  let closeables: Array<{ close(): void }>;
  const originalRerailHome = process.env.RERAIL_HOME;

  beforeEach(async () => {
    root = await mkdtemp(join(tmpdir(), "rerail-api-"));
    process.env.RERAIL_HOME = join(root, "home");
    databasePath = join(root, ".rerail", "rerail.sqlite");
    catalog = ScopedRuleCatalog.open(join(root, "rules.sqlite"), { repositoryId: "test-repository", workspaceRoot: root });
    audits = new SqliteAuditStore(databasePath);
    sessionStore = new SqliteSessionStore(databasePath);
    const reader = new SqliteSessionReader(databasePath);
    const auditReader = new SqliteAuditReader(databasePath);
    const preferences = new SqlitePresentationPreferenceStore(databasePath);
    const auditControls = new SqliteAuditControlStore(databasePath);
    raw = new Database(databasePath);
    database = openRerailDatabase(databasePath);
    closeables = [catalog, audits, sessionStore, reader, auditReader, preferences, auditControls, raw, database.$client];
    stores = {
      reader,
      audits: auditReader,
      rules: () => catalog,
      sessionStore: () => sessionStore,
      auditControls: () => auditControls,
      preferences: () => preferences,
      database: () => database,
      repositoryState: startStateReader(database),
      health: () => ({ pid: process.pid, database: { path: databasePath, exists: true }, native: { ok: true } }),
    };
    app = createRerailApi(stores, operations, { version: "0.0.0-test" });
    sessionStore.openSession(SESSION);
  });

  afterEach(async () => {
    for (const closeable of closeables) closeable.close();
    if (originalRerailHome === undefined) delete process.env.RERAIL_HOME;
    else process.env.RERAIL_HOME = originalRerailHome;
    await rm(root, { recursive: true, force: true });
  });

  /** A completed run with an explicit start time, which is what every range query is bounded by. */
  function seedAudit(audit: { sessionId?: string; at: number; violations?: CodeRuleViolation[]; audited?: string[] }): string {
    const auditId = audits.start({
      sessionId: audit.sessionId ?? SESSION.sessionId,
      workspaceRoot: root,
      scope: "touched-files",
      mode: "steer",
      revision: 1,
      catalogRevision: null,
      ruleRevisions: (audit.audited ?? []).map(ruleId => ({ ruleId, revision: 1 })),
      paths: ["web/src/x.ts"],
      ruleProvenance: [],
    });
    audits.finish(auditId, { status: "completed", violations: audit.violations ?? [], failures: [] });
    raw.prepare("UPDATE audits SET started_at = ? WHERE audit_id = ?").run(audit.at, auditId);
    return auditId;
  }

  async function getJson<T>(path: string): Promise<T> {
    const response = await app.request(path);
    expect(response.status).toBe(200);
    // Test boundary: the response is asserted against an expectation on the next line.
    return await response.json() as T;
  }

  it("counts one unfixed violation once however many audits report it", async () => {
    const [codeRule, otherCodeRule] = [MOCK_CODE_RULES[0]!, MOCK_CODE_RULES[1]!];
    const conversationRule = MOCK_CONVERSATION_RULES[0]!;
    catalog.upsertCodeRule(codeRule);
    catalog.upsertCodeRule(otherCodeRule);
    catalog.upsertConversationRule(conversationRule, { revision: 1 });
    seedAudit({ at: 500, violations: [flagged], audited: [codeRule.id] });
    seedAudit({ at: 2_000, violations: [flagged], audited: [codeRule.id, otherCodeRule.id] });
    seedAudit({ at: 3_000, violations: [{ ...flagged, id: "violation-2" }], audited: [codeRule.id] });

    const body = await getJson<{ since: number; rules: Record<string, Record<string, unknown>> }>("/api/rules/activity?since=1000");

    expect(body.since).toBe(1000);
    // Evaluated by the two runs in range; the run at 500 is before the range and counts for nothing.
    expect(body.rules[codeRule.id]).toEqual({
      measured: true,
      distinctViolations: 1,
      sessions: 1,
      lastSeenAt: 3_000,
      audits: 2,
      rerails: 0,
    });
    // Compiled into a run but never flagged: a zero backed by evidence rather than by silence.
    expect(body.rules[otherCodeRule.id]).toMatchObject({ measured: true, distinctViolations: 0, audits: 1, lastSeenAt: null });
    // Conversation rules never write an audit, so there is nothing to report either way.
    expect(body.rules[conversationRule.id]).toEqual({
      measured: false,
      distinctViolations: 0,
      sessions: 0,
      lastSeenAt: null,
      audits: 0,
      rerails: 0,
    });
  });

  it("separates violations by path, span, and session, and excludes audits before the range", async () => {
    const codeRule = MOCK_CODE_RULES[0]!;
    catalog.upsertCodeRule(codeRule);
    seedAudit({ at: 500, violations: [{ ...flagged, path: "web/src/old.ts" }] });
    seedAudit({
      at: 2_000,
      sessionId: "session-b",
      violations: [flagged, { ...flagged, id: "violation-2", path: "web/src/y.ts" }],
    });
    sessionStore.openSession({ ...SESSION, sessionId: "session-b" });
    seedAudit({ at: 2_500, violations: [flagged] });

    const body = await getJson<{ rules: Record<string, Record<string, unknown>> }>("/api/rules/activity?since=1000");

    // Violations alone do not make a run an evaluation of the rule; only the compiled rule set does.
    expect(body.rules[codeRule.id]).toMatchObject({ distinctViolations: 2, sessions: 2, lastSeenAt: 2_500, audits: 0 });
    expect((await getJson<{ rules: Record<string, Record<string, unknown>> }>("/api/rules/activity?since=0"))
      .rules[codeRule.id]).toMatchObject({ distinctViolations: 3, lastSeenAt: 2_500 });
  });

  it("attributes steers to the rules each steer named", async () => {
    const codeRule = MOCK_CODE_RULES[0]!;
    catalog.upsertCodeRule(codeRule);
    sessionStore.append({ sessionId: SESSION.sessionId, kind: "steering-delivered", observedAt: 2_000, payload: { ruleIds: [codeRule.id] } });
    // Deliveries recorded before rule attribution existed carry an empty payload and count for nothing.
    sessionStore.append({ sessionId: SESSION.sessionId, kind: "steering-delivered", observedAt: 2_100, payload: {} });
    sessionStore.append({ sessionId: SESSION.sessionId, kind: "steering-delivered", observedAt: 500, payload: { ruleIds: [codeRule.id] } });

    const body = await getJson<{ rules: Record<string, { rerails: number }> }>("/api/rules/activity?since=1000");

    expect(body.rules[codeRule.id]!.rerails).toBe(1);
    expect((await getJson<{ rules: Record<string, { rerails: number }> }>("/api/rules/activity?since=0"))
      .rules[codeRule.id]!.rerails).toBe(2);
  });

  it("lists violations newest first, paged, and filtered by rule", async () => {
    const other = { ...flagged, id: "violation-other", ruleId: MOCK_CODE_RULES[1]!.id, path: "web/src/z.ts" };
    seedAudit({ at: 1_000, violations: [flagged] });
    // The same violation twice in one run is one row; across runs it is history.
    const newest = seedAudit({ at: 4_000, violations: [flagged, { ...flagged, id: "violation-copy" }, other] });

    const page = await getJson<{ total: number; violations: Array<Record<string, unknown>> }>("/api/violations?since=0&limit=2");
    expect(page.total).toBe(3);
    expect(page.violations).toHaveLength(2);
    expect(page.violations.map(violation => violation.at)).toEqual([4_000, 4_000]);
    expect(page.violations[0]).toMatchObject({
      id: flagged.id,
      ruleId: flagged.ruleId,
      clauseId: flagged.clauseId,
      code: flagged.code,
      mode: "block",
      path: "web/src/x.ts",
      span: flagged.span,
      sessionId: SESSION.sessionId,
      auditId: newest,
      at: 4_000,
    });
    expect((await getJson<{ violations: Array<{ at: number }> }>("/api/violations?since=0&limit=2&offset=2")).violations)
      .toMatchObject([{ at: 1_000 }]);
    // A page past the end still reports how many violations exist.
    expect(await getJson<{ total: number; violations: unknown[] }>("/api/violations?since=0&limit=2&offset=3"))
      .toEqual({ total: 3, violations: [] });

    const filtered = await getJson<{ total: number; violations: Array<{ ruleId: string }> }>(`/api/violations?since=0&ruleId=${other.ruleId}`);
    expect(filtered).toMatchObject({ total: 1, violations: [{ ruleId: other.ruleId }] });
    expect((await getJson<{ total: number }>("/api/violations?since=2000")).total).toBe(2);
  });

  it("lists only the sessions started in range", async () => {
    sessionStore.openSession({ ...SESSION, sessionId: "session-old" }, 500);
    sessionStore.openSession({ ...SESSION, sessionId: "session-new" }, 2_000);

    // The fixture's own session opened at the current clock, so only `session-old` falls outside.
    expect((await getJson<{ sessions: Array<{ sessionId: string }> }>("/api/sessions?since=1000")).sessions
      .map(session => session.sessionId)).toEqual(["session-new", SESSION.sessionId]);
    expect((await getJson<{ sessions: unknown[] }>("/api/sessions")).sessions).toHaveLength(3);
  });

  it("answers an unchanged catalog read with 304 and mints a new tag after a write", async () => {
    catalog.upsertCodeRule(MOCK_CODE_RULES[0]!);
    const first = await app.request("/api/rules");
    const tag = first.headers.get("etag");
    expect(tag).toMatch(/^"[0-9a-f]{32}"$/);

    const repeated = await app.request("/api/rules", { headers: { "if-none-match": tag! } });
    expect(repeated.status).toBe(304);
    expect(repeated.headers.get("etag")).toBe(tag);
    expect(await repeated.text()).toBe("");
    // A different filter is a different response, so it must not reuse the tag.
    expect((await app.request("/api/rules?kind=conversation", { headers: { "if-none-match": tag! } })).status).toBe(200);

    const upsert = await app.request(`/api/rules/${MOCK_CODE_RULES[1]!.id}`, {
      method: "PUT",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ kind: "file", rule: MOCK_CODE_RULES[1]! }),
    });
    expect(upsert.status).toBe(200);
    const refreshed = await app.request("/api/rules", { headers: { "if-none-match": tag! } });
    expect(refreshed.status).toBe(200);
    expect(refreshed.headers.get("etag")).not.toBe(tag);
  });

  it("stores a structural tool-call rule and rejects a regex the engine cannot compile", async () => {
    const id = "no-force-push";
    const rule = (regex: string) => ({
      id,
      mode: "steer",
      match: { all: [{ field: "tool.name", eq: "Bash" }, { field: "input.command", regex }] },
      sample: { source: "tool:Bash", boundary: "tool-call" },
      action: { content: "Do not force-push." },
    });
    const put = (regex: string) => app.request(`/api/rules/${id}`, {
      method: "PUT",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ kind: "conversation", rule: rule(regex) }),
    });

    const rejected = await put("(?!main)");
    expect(rejected.status).toBe(400);
    expect(await rejected.json()).toMatchObject({
      error: "invalid-request",
      issues: [{ path: ["rule", "match", "all", 1, "regex"], message: expect.stringMatching(/^is not a valid regex: .*look-around/su) }],
    });
    expect(catalog.get(id)).toBeUndefined();

    expect((await put("\\bgit\\s+push\\b[^\\n]*--force\\b")).status).toBe(200);
    expect(catalog.get(id)?.rule).toEqual(rule("\\bgit\\s+push\\b[^\\n]*--force\\b"));
  });

  it("reports a quarantined inherited global toggle as a rule conflict", async () => {
    const id = "quarantined-global";
    catalog.upsertConversationRule({ ...MOCK_CONVERSATION_RULES[0]!, id }, { applicability: "global" });
    const catalogDatabase = new Database(catalog.path);
    catalogDatabase.prepare("UPDATE catalog_rules SET document_json = ? WHERE rule_id = ?").run(
      JSON.stringify({ kind: "conversation", rule: { id, unknown: true } }),
      id,
    );
    catalogDatabase.close();

    const response = await app.request("/api/rules", {
      method: "PATCH",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ enabled: false, ids: [id] }),
    });

    expect(response.status).toBe(409);
    expect(await response.json()).toEqual({
      error: "rule-conflict",
      message: `cannot toggle quarantined global rule ${id}`,
    });
  });

  it("dates activity, violations, and sessions by their own result", async () => {
    catalog.upsertCodeRule(MOCK_CODE_RULES[0]!);
    for (const path of ["/api/rules/activity?since=0", "/api/violations?since=0", "/api/sessions"]) {
      const first = await app.request(path);
      const tag = first.headers.get("etag")!;
      expect(first.status).toBe(200);
      const repeated = await app.request(path, { headers: { "if-none-match": tag } });
      expect(repeated.status).toBe(304);
      expect(await repeated.text()).toBe("");
      seedAudit({ at: 5_000, violations: [flagged] });
      sessionStore.openSession({ ...SESSION, sessionId: `session-${path}` });
      expect((await app.request(path, { headers: { "if-none-match": tag } })).status).toBe(200);
    }
  });

  it("admits a write whose Origin is an allowlisted host on the requested port and rejects every other", async () => {
    const patch = async (headers: Record<string, string>, api = app): Promise<Response> => await api.request("/api/rules", {
      method: "PATCH",
      headers: { "content-type": "application/json", ...headers },
      body: JSON.stringify({ enabled: true, ids: ["mock-no-console-log"] }),
    });

    // The page is reached through a container port mapping: Host names a port the daemon never bound.
    expect((await patch({ origin: "http://localhost:35831", host: PAGE_HOST })).status).toBe(200);
    expect((await patch({ origin: "http://127.0.0.1:35831", host: "127.0.0.1:35831" })).status).toBe(200);
    // `URL` keeps the brackets around an IPv6 literal, exactly as `Host` carries them.
    expect((await patch({ origin: "http://[::1]:35831", host: "[::1]:35831" })).status).toBe(200);
    expect((await patch({ origin: "http://LOCALHOST:35831", host: PAGE_HOST })).status).toBe(200);
    expect((await patch({ origin: "http://localhost", host: "localhost" })).status).toBe(200);
    expect((await patch({ origin: "http://localhost", host: "localhost:80" })).status).toBe(200);

    const forbidden = await patch({ origin: "http://127.0.0.1:9999", host: "127.0.0.1:53831" });
    expect(forbidden.status).toBe(403);
    expect(await forbidden.json()).toEqual({ error: "api-origin-forbidden" });
    expect((await patch({ origin: "https://evil.example", host: PAGE_HOST })).status).toBe(403);
    // A rebinding page's own name is the only host it can put in both headers; agreeing headers are not enough.
    expect((await patch({ origin: "https://rerail.example", host: "rerail.example:443" })).status).toBe(403);
    expect((await patch({ origin: "http://[::1]:35831", host: "::1:35831" })).status).toBe(403);
    expect((await patch({ origin: "http://localhost", host: "localhost:443" })).status).toBe(403);
    expect((await patch({ origin: "null", host: PAGE_HOST })).status).toBe(403);
    expect((await patch({ origin: "file:///index.html", host: "" })).status).toBe(403);
    expect((await patch({ origin: "http://localhost:35831" })).status).toBe(403);

    // `--public-host` admits that name, on the requested port, alongside loopback.
    const published = createRerailApi(stores, operations, { version: "0.0.0-test", publicHosts: new Set(["rerail.example"]) });
    expect((await patch({ origin: "https://rerail.example", host: "rerail.example:443" }, published)).status).toBe(200);
    expect((await patch({ origin: "https://Rerail.Example:8443", host: "rerail.example:8443" }, published)).status).toBe(200);
    expect((await patch({ origin: "http://localhost:35831", host: PAGE_HOST }, published)).status).toBe(200);
    expect((await patch({ origin: "https://rerail.example", host: "rerail.example:8443" }, published)).status).toBe(403);
    expect((await patch({ origin: "https://evil.example", host: "rerail.example:443" }, published)).status).toBe(403);

    // CLI, curl, and MCP clients send no Origin at all.
    expect((await patch({ host: PAGE_HOST })).status).toBe(200);
    expect((await patch({})).status).toBe(200);
    // Reads are not writes: a foreign page can already only read what its own daemon serves.
    expect((await app.request("/api/rules", { headers: { origin: "https://evil.example", host: PAGE_HOST } })).status).toBe(200);
  });

  it("documents the new operations and their conditional responses", async () => {
    const document = await getJson<{ paths: Record<string, Record<string, { operationId: string; responses: Record<string, unknown> }>> }>("/openapi.json");

    expect(document.paths["/api/rules/activity"]?.get?.operationId).toBe("rules_activity");
    expect(document.paths["/api/violations"]?.get?.operationId).toBe("violations_list");
    for (const path of ["/api/rules", "/api/rules/{id}", "/api/rules/activity", "/api/violations", "/api/sessions"]) {
      expect(document.paths[path]?.get?.responses["304"]).toBeDefined();
    }
    expect(document.paths["/api/rules"]?.patch?.responses["304"]).toBeUndefined();
  });

  it("lists repositories by start state and starts or stops only ones that were set up", async () => {
    const post = (path: string): Promise<Response> => Promise.resolve(app.request(path, { method: "POST" }));
    raw.prepare("INSERT INTO rule_repositories (repository_id, last_known_path, created_at, updated_at) VALUES (?, ?, 1, 1)")
      .run("repo-a", "/code/acme-web");
    raw.prepare("INSERT INTO rule_repositories (repository_id, last_known_path, created_at, updated_at) VALUES (?, ?, 1, 1)")
      .run("repo-b", "/code/landing-page");
    raw.prepare("INSERT INTO repository_starts (repository_id, state, started_at, updated_at) VALUES ('repo-a', 'started', 10, 10)").run();

    expect((await getJson<{ repositories: unknown[] }>("/api/repositories")).repositories).toEqual([
      { repositoryId: "repo-a", name: "acme-web", root: "/code/acme-web", state: "started", startedAt: 10, stoppedAt: null },
      { repositoryId: "repo-b", name: "landing-page", root: "/code/landing-page", state: "never", startedAt: null, stoppedAt: null },
    ]);

    const stopped = await post("/api/repositories/repo-a/stop");
    expect(stopped.status).toBe(200);
    expect(await stopped.json()).toMatchObject({ repository: { state: "stopped", startedAt: 10 } });
    const started = await post("/api/repositories/repo-a/start");
    expect(await started.json()).toMatchObject({ repository: { state: "started" } });

    // Set-up needs a session in the repository; the dashboard cannot do it.
    const unset = await post("/api/repositories/repo-b/start");
    expect(unset.status).toBe(409);
    expect((await post("/api/repositories/repo-b/stop")).status).toBe(200);
    expect((await getJson<{ repositories: Array<{ state: string }> }>("/api/repositories")).repositories[1]?.state).toBe("never");
    expect((await post("/api/repositories/unknown/stop")).status).toBe(404);
  });

  it("lists a repository set up only in a worktree as stopped, and starts and stops its main checkout", async () => {
    const post = (path: string): Promise<Response> => Promise.resolve(app.request(path, { method: "POST" }));
    raw.prepare("INSERT INTO rule_repositories (repository_id, last_known_path, created_at, updated_at) VALUES (?, ?, 1, 1)")
      .run("repo-c", "/code/acme-web");
    raw.prepare("INSERT INTO checkout_starts (repository_id, checkout_path, state, started_at, updated_at) VALUES ('repo-c', '/code/acme-web-feature', 'started', 5, 5)").run();

    expect((await getJson<{ repositories: unknown[] }>("/api/repositories")).repositories).toEqual([
      { repositoryId: "repo-c", name: "acme-web", root: "/code/acme-web", state: "stopped", startedAt: null, stoppedAt: null },
    ]);
    const started = await post("/api/repositories/repo-c/start");
    expect(started.status).toBe(200);
    expect(await started.json()).toMatchObject({ repository: { state: "started" } });
    const stopped = await post("/api/repositories/repo-c/stop");
    expect(await stopped.json()).toMatchObject({ repository: { state: "stopped" } });
    // The worktree's own setting is untouched by the main checkout's.
    expect(raw.prepare("SELECT state FROM checkout_starts WHERE repository_id = 'repo-c'").pluck().get()).toBe("started");
  });
});
