import { mkdir, mkdtemp, readdir, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { OpenAPIHono } from "@hono/zod-openapi";
import { eq } from "drizzle-orm";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createRerailApi } from "../../src/api/http.js";
import { operations } from "../../src/api/operations.js";
import { DatabaseApiStores } from "../../src/api/stores.js";
import {
  loadBundledCatalog,
  openRerailDatabase,
  type RerailDatabase,
  type CodeRuleViolation,
  SqliteAuditStore,
  upsertLibraryRule,
  audits,
  checkouts,
  repositories,
  PACKAGE_ROOT,
  defaultRerailHome,
} from "../../src/engine/index.js";
import { MOCK_CODE_RULES, MOCK_CONVERSATION_RULES } from "../../src/cli/index.js";
import { SqliteSessionStore } from "../../src/runtime/index.js";
import { codingAgentHome, codingAgentStates, registerHarnesses, registrationRoot } from "../../src/setup/index.js";

/** The host a browser addressed the page to through a container port mapping; the fixture never binds it. */
const PAGE_HOST = "localhost:35831";
const SESSION = { host: "codex" as const, sessionId: "session-a", cwd: "/workspace" };
const WEB = "remote:github.com/acme/web";

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
  let stores: DatabaseApiStores;
  let auditStore: SqliteAuditStore;
  let sessionStore: SqliteSessionStore;
  let database: RerailDatabase;
  const originalRerailHome = process.env.RERAIL_HOME;

  beforeEach(async () => {
    root = await mkdtemp(join(tmpdir(), "rerail-api-"));
    process.env.RERAIL_HOME = join(root, "home");
    databasePath = join(root, "rerail.sqlite");
    database = openRerailDatabase(databasePath);
    auditStore = new SqliteAuditStore(databasePath);
    sessionStore = new SqliteSessionStore(databasePath);
    stores = new DatabaseApiStores(databasePath, {
      health: () => ({ pid: process.pid, database: { path: databasePath, exists: true }, native: { ok: true } }),
      origin: () => "http://127.0.0.1:4737",
      codingAgents: () => codingAgentStates(registrationRoot(defaultRerailHome(), PACKAGE_ROOT), codingAgentHome()),
    });
    app = createRerailApi(stores, operations, { version: "0.0.0-test" });
    sessionStore.openSession(SESSION);
  });

  afterEach(async () => {
    for (const closeable of [stores, auditStore, sessionStore]) closeable.close();
    database.$client.close();
    if (originalRerailHome === undefined) delete process.env.RERAIL_HOME;
    else process.env.RERAIL_HOME = originalRerailHome;
    await rm(root, { recursive: true, force: true });
  });

  /** A completed run with an explicit start time, which is what every range query is bounded by. */
  function seedAudit(audit: { sessionId?: string; at: number; violations?: CodeRuleViolation[]; audited?: string[] }): string {
    const auditId = auditStore.start({
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
    auditStore.finish(auditId, { status: "completed", violations: audit.violations ?? [], failures: [] });
    database.update(audits).set({ startedAt: audit.at }).where(eq(audits.auditId, auditId)).run();
    return auditId;
  }

  function addLibraryRules(): void {
    const bundle = loadBundledCatalog();
    for (const rule of MOCK_CODE_RULES) upsertLibraryRule(database, bundle, rule, "code");
    for (const rule of MOCK_CONVERSATION_RULES) upsertLibraryRule(database, bundle, rule, "conversation", { revision: 1 });
  }

  function addRepository(key: string, options: { name?: string; configuredAt?: number | null; started?: boolean | null } = {}): void {
    database.insert(repositories).values({
      repositoryKey: key,
      displayName: options.name ?? key,
      configuredAt: options.configuredAt ?? null,
      started: options.started ?? null,
      createdAt: 1,
      updatedAt: 1,
    }).run();
  }

  function addCheckout(checkoutPath: string, repositoryKey: string, cloneId: string, started: boolean | null = null): void {
    database.insert(checkouts).values({ checkoutPath, repositoryKey, cloneId, started, createdAt: 1, lastSeenAt: 1 }).run();
  }

  async function getJson<T>(path: string): Promise<T> {
    const response = await app.request(path);
    expect(response.status).toBe(200);
    // Test boundary: the response is asserted against an expectation on the next line.
    return await response.json() as T;
  }

  async function putJson(path: string, body: unknown, headers: Record<string, string> = {}): Promise<Response> {
    return await app.request(path, { method: "PUT", headers: { "content-type": "application/json", ...headers }, body: JSON.stringify(body) });
  }

  it("counts one unfixed violation once however many audits report it", async () => {
    addLibraryRules();
    const [codeRule, otherCodeRule] = [MOCK_CODE_RULES[0]!, MOCK_CODE_RULES[1]!];
    const conversationRule = MOCK_CONVERSATION_RULES[0]!;
    seedAudit({ at: 500, violations: [flagged], audited: [codeRule.id] });
    seedAudit({ at: 2_000, violations: [flagged], audited: [codeRule.id, otherCodeRule.id] });
    seedAudit({ at: 3_000, violations: [{ ...flagged, id: "violation-2" }], audited: [codeRule.id] });

    const body = await getJson<{ since: number; rules: Record<string, Record<string, unknown>> }>("/api/rules/activity?since=1000");

    expect(body.since).toBe(1000);
    // Evaluated by the two runs in range; the run at 500 is before the range and counts for nothing.
    expect(body.rules[codeRule.id]).toEqual({ measured: true, distinctViolations: 1, sessions: 1, lastSeenAt: 3_000, audits: 2, loggedViolations: 0, rerails: 0 });
    // Compiled into a run but never flagged: a zero backed by evidence rather than by silence.
    expect(body.rules[otherCodeRule.id]).toMatchObject({ measured: true, distinctViolations: 0, audits: 1, lastSeenAt: null });
    // Conversation rules never write an audit, so there is nothing to report either way.
    expect(body.rules[conversationRule.id]).toEqual({ measured: false, distinctViolations: 0, sessions: 0, lastSeenAt: null, audits: 0, loggedViolations: 0, rerails: 0 });
  });

  it("counts logged-only violations by the mode recorded with each, not the rule's mode now", async () => {
    addLibraryRules();
    const codeRule = MOCK_CODE_RULES[0]!;
    seedAudit({ at: 2_000, violations: [{ ...flagged, mode: "steer" }], audited: [codeRule.id] });
    seedAudit({ at: 3_000, violations: [{ ...flagged, id: "violation-2", path: "web/src/y.ts", mode: "log" }], audited: [codeRule.id] });

    const body = await getJson<{ rules: Record<string, Record<string, unknown>> }>("/api/rules/activity?since=1000");

    expect(body.rules[codeRule.id]).toMatchObject({ distinctViolations: 2, loggedViolations: 1 });
  });

  it("separates violations by path, span, and session, and excludes audits before the range", async () => {
    addLibraryRules();
    const codeRule = MOCK_CODE_RULES[0]!;
    seedAudit({ at: 500, violations: [{ ...flagged, path: "web/src/old.ts" }] });
    seedAudit({ at: 2_000, sessionId: "session-b", violations: [flagged, { ...flagged, id: "violation-2", path: "web/src/y.ts" }] });
    sessionStore.openSession({ ...SESSION, sessionId: "session-b" });
    seedAudit({ at: 2_500, violations: [flagged] });

    const body = await getJson<{ rules: Record<string, Record<string, unknown>> }>("/api/rules/activity?since=1000");

    // Violations alone do not make a run an evaluation of the rule; only the compiled rule set does.
    expect(body.rules[codeRule.id]).toMatchObject({ distinctViolations: 2, sessions: 2, lastSeenAt: 2_500, audits: 0 });
    expect((await getJson<{ rules: Record<string, Record<string, unknown>> }>("/api/rules/activity?since=0"))
      .rules[codeRule.id]).toMatchObject({ distinctViolations: 3, lastSeenAt: 2_500 });
  });

  it("attributes steers to the rules each steer named", async () => {
    addLibraryRules();
    const codeRule = MOCK_CODE_RULES[0]!;
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

    const page = await getJson<{ total: number; violations: Array<Record<string, unknown>>; nextCursor: string | null }>("/api/violations?since=0&limit=2");
    expect(page.total).toBe(3);
    expect(page.violations.map(violation => violation.at)).toEqual([4_000, 4_000]);
    expect(page.violations[0]).toMatchObject({
      id: flagged.id, ruleId: flagged.ruleId, clauseId: flagged.clauseId, code: flagged.code, mode: "block",
      path: "web/src/x.ts", span: flagged.span, sessionId: SESSION.sessionId, auditId: newest, at: 4_000, host: SESSION.host,
    });
    // The last page says so, and still reports how many violations exist.
    expect(await getJson(`/api/violations?since=0&limit=2&cursor=${encodeURIComponent(page.nextCursor!)}`))
      .toMatchObject({ total: 3, violations: [{ at: 1_000 }], nextCursor: null });
    expect((await app.request("/api/violations?since=0&cursor=not-a-cursor")).status).toBe(400);
    const filtered = await getJson<{ total: number; violations: Array<{ ruleId: string }> }>(`/api/violations?since=0&ruleId=${other.ruleId}`);
    expect(filtered).toMatchObject({ total: 1, violations: [{ ruleId: other.ruleId }] });
    expect((await getJson<{ total: number }>(`/api/violations?since=0&sessionId=${SESSION.sessionId}`)).total).toBe(3);
    expect((await getJson<{ total: number }>("/api/violations?since=0&sessionId=another-session")).total).toBe(0);
    expect((await getJson<{ total: number }>("/api/violations?since=2000")).total).toBe(2);
  });

  it("counts a session's gaps over its whole transcript, however many events follow them", async () => {
    const dropped = { id: "gap-1", count: 7, kinds: { call: 7 }, firstEventId: null, lastEventId: null, firstDroppedAt: 1, lastDroppedAt: 2 };
    sessionStore.recordDroppedEvents(SESSION, dropped);
    sessionStore.recordDroppedEvents(SESSION, { ...dropped, id: "gap-2", count: 2 });
    sessionStore.recordDiscoveryPending(SESSION, "turn");
    sessionStore.recordCodeRulesPending(SESSION, "call");
    sessionStore.recordCodeRulesPending(SESSION, "result");
    // More than a transcript page of later events: the dashboard opens on those and never loads the gaps.
    for (let index = 0; index < 600; index += 1) {
      sessionStore.append({ sessionId: SESSION.sessionId, eventId: `later-${index}`, kind: "turn-end", observedAt: 10_000 + index, payload: {} });
    }

    expect((await getJson<{ gaps: unknown }>(`/api/sessions/${SESSION.sessionId}`)).gaps)
      .toEqual({ droppedEvents: 9, uncheckedBoundaries: 1, codeRulesPendingBoundaries: 2 });
  });

  it("lists sessions newest first, a page at a time, with their violation counts", async () => {
    sessionStore.openSession({ ...SESSION, sessionId: "session-old" }, 500);
    sessionStore.openSession({ ...SESSION, sessionId: "session-new" }, 2_000);
    sessionStore.openSession({ ...SESSION, sessionId: "session-tied" }, 2_000);
    seedAudit({ sessionId: "session-new", at: 2_100, violations: [flagged, { ...flagged, id: "violation-2", path: "web/src/y.ts" }] });

    const seen: string[] = [];
    let cursor: string | null = null;
    do {
      const page: { sessions: Array<{ sessionId: string; violationCount: number }>; total: number; nextCursor: string | null } =
        await getJson(`/api/sessions?limit=2${cursor === null ? "" : `&cursor=${encodeURIComponent(cursor)}`}`);
      expect(page.total).toBe(4);
      seen.push(...page.sessions.map(session => `${session.sessionId}:${session.violationCount}`));
      cursor = page.nextCursor;
    } while (cursor !== null);
    // The fixture's own session opened at the current clock; sessions started together keep id order.
    expect(seen).toEqual([`${SESSION.sessionId}:0`, "session-new:2", "session-tied:0", "session-old:0"]);
  });

  it("lists only the sessions started in range", async () => {
    sessionStore.openSession({ ...SESSION, sessionId: "session-old" }, 500);
    sessionStore.openSession({ ...SESSION, sessionId: "session-new" }, 2_000);

    // The fixture's own session opened at the current clock, so only `session-old` falls outside.
    expect((await getJson<{ sessions: Array<{ sessionId: string }> }>("/api/sessions?since=1000")).sessions
      .map(session => session.sessionId)).toEqual([SESSION.sessionId, "session-new"]);
    expect((await getJson<{ sessions: unknown[] }>("/api/sessions")).sessions).toHaveLength(3);
    expect((await getJson<{ sessions: { started: number } }>("/api/rules/activity?since=1000")).sessions.started).toBe(2);
  });

  it("narrows history to one repository: its key, its aliases, and its checkouts' clone ids", async () => {
    database.insert(repositories).values({ repositoryKey: WEB, displayName: "web", aliases: ["clone:adopted"], createdAt: 1, updatedAt: 1 }).run();
    addCheckout("/code/web", WEB, "clone-web");
    sessionStore.openSession({ ...SESSION, sessionId: "by-key" }, 1_000, WEB);
    sessionStore.openSession({ ...SESSION, sessionId: "by-clone-id" }, 1_100, "clone-web");
    sessionStore.openSession({ ...SESSION, sessionId: "by-alias" }, 1_200, "clone:adopted");
    sessionStore.openSession({ ...SESSION, sessionId: "elsewhere" }, 1_300, "remote:github.com/acme/other");
    seedAudit({ sessionId: "by-clone-id", at: 1_500, violations: [flagged] });
    seedAudit({ sessionId: "elsewhere", at: 1_600, violations: [{ ...flagged, id: "violation-elsewhere" }] });

    const query = `repository=${encodeURIComponent(WEB)}`;
    expect((await getJson<{ sessions: Array<{ sessionId: string }> }>(`/api/sessions?${query}`)).sessions.map(session => session.sessionId))
      .toEqual(["by-alias", "by-clone-id", "by-key"]);
    expect((await getJson<{ violations: Array<{ sessionId: string }> }>(`/api/violations?since=0&${query}`)).violations)
      .toMatchObject([{ sessionId: "by-clone-id" }]);
    expect((await getJson<{ audits: Array<{ sessionId: string }> }>(`/api/audits?${query}`)).audits).toMatchObject([{ sessionId: "by-clone-id" }]);
    // Without a repository the listing is machine-wide.
    expect((await getJson<{ total: number }>("/api/violations?since=0")).total).toBe(2);
  });

  it("answers an unchanged rules read with 304 and mints a new tag after a settings write", async () => {
    addRepository(WEB, { configuredAt: 5, started: true });
    const path = `/api/repository/rules?repository=${encodeURIComponent(WEB)}`;
    const first = await app.request(path);
    const tag = first.headers.get("etag");
    expect(tag).toMatch(/^"[0-9a-f]{32}"$/);

    const repeated = await app.request(path, { headers: { "if-none-match": tag! } });
    expect(repeated.status).toBe(304);
    expect(await repeated.text()).toBe("");

    const written = await putJson("/api/repository/settings", { repository: WEB, owner: "repository", targets: [{ kind: "pack", id: "complexity" }], on: false });
    expect(written.status).toBe(200);
    const refreshed = await app.request(path, { headers: { "if-none-match": tag! } });
    expect(refreshed.status).toBe(200);
    expect(refreshed.headers.get("etag")).not.toBe(tag);
  });

  it("stores a structural tool-call rule in the library and rejects a regex the engine cannot compile", async () => {
    const id = "no-force-push";
    const rule = (regex: string) => ({
      id,
      mode: "steer",
      match: { all: [{ field: "tool.name", eq: "Bash" }, { field: "input.command", regex }] },
      sample: { source: "tool:Bash", boundary: "tool-call" },
      action: { content: "Do not force-push." },
    });
    const put = (regex: string) => putJson(`/api/library-rules/${id}`, { kind: "conversation", rule: rule(regex) });

    const rejected = await put("(?!main)");
    expect(rejected.status).toBe(400);
    expect(await rejected.json()).toMatchObject({
      error: "invalid-request",
      issues: [{ path: ["rule", "match", "all", 1, "regex"], message: expect.stringMatching(/^is not a valid regex: .*look-around/su) }],
    });
    expect((await app.request(`/api/library-rules/${id}`)).status).toBe(404);

    expect((await put("\\bgit\\s+push\\b[^\\n]*--force\\b")).status).toBe(200);
    expect((await getJson<{ rule: { rule: unknown } }>(`/api/library-rules/${id}`)).rule.rule).toEqual(rule("\\bgit\\s+push\\b[^\\n]*--force\\b"));
  });

  it("dates activity, violations, and sessions by their own result", async () => {
    addLibraryRules();
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

  it("computes a history read once per change to the database, however often the dashboard polls it", async () => {
    addLibraryRules();
    addRepository(WEB, { configuredAt: 5, started: true });
    const activity = vi.spyOn(stores.audits, "ruleActivity");
    const first = await getJson("/api/rules/activity?since=0");
    expect(await getJson("/api/rules/activity?since=0")).toEqual(first);
    expect(activity).toHaveBeenCalledTimes(1);
    // Another connection commits: the next read computes again and sees it.
    seedAudit({ at: 5_000, violations: [flagged] });
    expect(await getJson("/api/rules/activity?since=0")).not.toEqual(first);
    expect(activity).toHaveBeenCalledTimes(2);
    // A write through the API's own connection counts too, which `PRAGMA data_version` does not see (only the stores'
    // local revision does), and another range is its own read.
    expect((await putJson("/api/repository/settings", { repository: WEB, owner: "repository", targets: [{ kind: "pack", id: "complexity" }], on: false })).status).toBe(200);
    await getJson("/api/rules/activity?since=0");
    await getJson("/api/rules/activity?since=1");
    expect(activity).toHaveBeenCalledTimes(4);
  });

  it("admits a write whose Origin is an allowlisted host on the requested port and rejects every other", async () => {
    addRepository(WEB);
    const write = async (headers: Record<string, string>, api = app): Promise<Response> => await api.request("/api/repository/settings", {
      method: "PUT",
      headers: { "content-type": "application/json", ...headers },
      body: JSON.stringify({ repository: WEB, owner: "repository", targets: [{ kind: "rule", id: "mock-no-console-log" }], on: true }),
    });

    // The page is reached through a container port mapping: Host names a port the daemon never bound.
    expect((await write({ origin: "http://localhost:35831", host: PAGE_HOST })).status).toBe(200);
    expect((await write({ origin: "http://127.0.0.1:35831", host: "127.0.0.1:35831" })).status).toBe(200);
    // `URL` keeps the brackets around an IPv6 literal, exactly as `Host` carries them.
    expect((await write({ origin: "http://[::1]:35831", host: "[::1]:35831" })).status).toBe(200);
    expect((await write({ origin: "http://LOCALHOST:35831", host: PAGE_HOST })).status).toBe(200);
    expect((await write({ origin: "http://localhost", host: "localhost" })).status).toBe(200);
    expect((await write({ origin: "http://localhost", host: "localhost:80" })).status).toBe(200);

    const forbidden = await write({ origin: "http://127.0.0.1:9999", host: "127.0.0.1:53831" });
    expect(forbidden.status).toBe(403);
    expect(await forbidden.json()).toEqual({ error: "api-origin-forbidden" });
    expect((await write({ origin: "https://evil.example", host: PAGE_HOST })).status).toBe(403);
    // A rebinding page's own name is the only host it can put in both headers; agreeing headers are not enough.
    expect((await write({ origin: "https://rerail.example", host: "rerail.example:443" })).status).toBe(403);
    expect((await write({ origin: "http://[::1]:35831", host: "::1:35831" })).status).toBe(403);
    expect((await write({ origin: "http://localhost", host: "localhost:443" })).status).toBe(403);
    expect((await write({ origin: "null", host: PAGE_HOST })).status).toBe(403);
    expect((await write({ origin: "file:///index.html", host: "" })).status).toBe(403);
    expect((await write({ origin: "http://localhost:35831" })).status).toBe(403);

    // `--public-host` admits that name, on the requested port, alongside loopback.
    const published = createRerailApi(stores, operations, { version: "0.0.0-test", publicHosts: new Set(["rerail.example"]) });
    expect((await write({ origin: "https://rerail.example", host: "rerail.example:443" }, published)).status).toBe(200);
    expect((await write({ origin: "https://Rerail.Example:8443", host: "rerail.example:8443" }, published)).status).toBe(200);
    expect((await write({ origin: "http://localhost:35831", host: PAGE_HOST }, published)).status).toBe(200);
    expect((await write({ origin: "https://rerail.example", host: "rerail.example:8443" }, published)).status).toBe(403);
    expect((await write({ origin: "https://evil.example", host: "rerail.example:443" }, published)).status).toBe(403);

    // CLI, curl, and MCP clients send no Origin at all.
    expect((await write({ host: PAGE_HOST })).status).toBe(200);
    expect((await write({})).status).toBe(200);
    // Reads are not writes: a foreign page can already only read what its own daemon serves.
    expect((await app.request("/api/repositories", { headers: { origin: "https://evil.example", host: PAGE_HOST } })).status).toBe(200);
  });

  it("documents conditional responses and the agent refusal", async () => {
    const document = await getJson<{ paths: Record<string, Record<string, { operationId: string; responses: Record<string, unknown> }>> }>("/openapi.json");

    expect(document.paths["/api/repository/rules"]?.get?.operationId).toBe("repository_rules_get");
    for (const path of ["/api/repositories", "/api/repository/rules", "/api/library-rules", "/api/library-rules/{id}", "/api/rules/activity", "/api/sessions"]) {
      expect(document.paths[path]?.get?.responses["304"]).toBeDefined();
    }
    expect(document.paths["/api/repository/settings"]?.put?.responses["304"]).toBeUndefined();
    expect(document.paths["/api/repository/settings"]?.put?.responses["403"]).toBeDefined();
    for (const removed of ["/api/rules", "/api/rules/{id}", "/api/rule-context"]) expect(document.paths[removed]).toBeUndefined();
  });

  it("lists repositories, and starts or stops one as a whole, set up or not", async () => {
    const post = (repository: string, action: string): Promise<Response> => Promise.resolve(app.request(`/api/repository/${action}`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ repository, scope: "repository" }),
    }));
    addRepository(WEB, { name: "acme-web", configuredAt: 5, started: true });
    addRepository("clone:landing", { name: "landing-page", started: true });
    addCheckout("/code/acme-web", WEB, "a");
    addCheckout("/code/acme-web-feature", WEB, "a", false);

    expect((await getJson<{ repositories: unknown[] }>("/api/repositories")).repositories).toEqual([
      { repositoryKey: WEB, name: "acme-web", aliases: [], setUp: true, configuredAt: 5, started: true, state: "started", followGitignore: true, checkoutCount: 2 },
      { repositoryKey: "clone:landing", name: "landing-page", aliases: [], setUp: false, configuredAt: null, started: true, state: "not-set-up", followGitignore: true, checkoutCount: 0 },
    ]);

    // Stopping and starting act on the repository as a whole: a checkout's own value follows it again.
    const stopped = await post(WEB, "stop");
    expect(stopped.status).toBe(200);
    expect(await stopped.json()).toMatchObject({ repository: { state: "stopped" }, checkout: null });
    expect(database.select({ started: checkouts.started }).from(checkouts).all()).toEqual([{ started: null }, { started: null }]);
    expect(await (await post(WEB, "start")).json()).toMatchObject({ repository: { state: "started" } });

    // A repository that is not set up is started and stopped the same way.
    expect(await (await post("clone:landing", "stop")).json()).toMatchObject({ repository: { state: "stopped" } });
    expect(await (await post("clone:landing", "start")).json()).toMatchObject({ repository: { state: "not-set-up" } });
    expect((await post("clone:unknown", "stop")).status).toBe(404);
  });

  it("sets a repository up with its first attachments, and refuses an unknown source", async () => {
    addRepository(WEB);
    const missing = await putJson("/api/repository/attachments", { repository: WEB, add: [{ sourceKind: "pack", sourceId: "no-such-pack" }] });
    expect(missing.status).toBe(404);
    expect((await getJson<{ repositories: Array<{ setUp: boolean }> }>("/api/repositories")).repositories[0]!.setUp).toBe(false);

    // "I'll pick rule packs myself": set up, nothing added.
    const configured = await putJson("/api/repository/attachments", { repository: WEB, configure: true });
    expect(await configured.json()).toMatchObject({ repository: { setUp: true }, attachments: [] });
    const pack = [...loadBundledCatalog().packs.keys()][0]!;
    const added = await putJson("/api/repository/attachments", { repository: WEB, add: [{ sourceKind: "pack", sourceId: pack, folder: "web/" }] });
    expect(await added.json()).toMatchObject({ attachments: [{ folder: "web", sourceKind: "pack", sourceId: pack }] });
    expect((await putJson("/api/repository/attachments", { repository: WEB, add: [{ sourceKind: "pack", sourceId: pack, folder: "../x" }] })).status).toBe(400);
  });

  it("tells a coding agent only found on this computer from one Rerail is connected to, and changes nothing", async () => {
    const home = join(root, "user");
    await mkdir(join(home, ".claude"), { recursive: true });
    await mkdir(join(home, ".cursor"), { recursive: true });
    await mkdir(join(home, ".codex"), { recursive: true });
    const previousHome = process.env.HOME;
    process.env.HOME = home;
    try {
      await registerHarnesses({ root: PACKAGE_ROOT, home, harnesses: ["cursor"] });
      // One coding agent whose config cannot be read must not hide the others.
      await writeFile(join(home, ".codex", "hooks.json"), "{ not json");
      const before = await readdir(join(home, ".claude"));
      const { agents } = await getJson<{ agents: Array<{ id: string; found: boolean; connected: boolean; problem: string | null }> }>("/api/coding-agents");
      expect(agents.find(agent => agent.id === "cursor")).toMatchObject({ found: true, connected: true, problem: null });
      expect(agents.find(agent => agent.id === "claude-code")).toMatchObject({ found: true, connected: false, problem: null });
      expect(agents.find(agent => agent.id === "codex")).toMatchObject({ found: true, connected: false, problem: expect.stringContaining("hooks.json") });
      expect(agents.find(agent => agent.id === "omp")).toMatchObject({ found: false, connected: false });
      expect(await readdir(join(home, ".claude"))).toEqual(before);
    } finally {
      if (previousHome === undefined) delete process.env.HOME;
      else process.env.HOME = previousHome;
    }
  });
});
