import { execFileSync } from "node:child_process";
import { mkdirSync, realpathSync } from "node:fs";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { createRerailApi } from "../../src/api/http.js";
import { createRerailMcpHandler } from "../../src/api/mcp.js";
import { operations } from "../../src/api/operations.js";
import { DatabaseApiStores } from "../../src/api/stores.js";
import {
  loadRuleCatalog,
  type CodeRule,
  CALLER_CWD_HEADER,
  listRepositories,
  recordCheckout,
  addAttachment,
  setUpRepository,
  SqliteAuditStore,
  upsertLibraryRule,
  writeLibraryRule,
  ruleSettings,
} from "../../src/engine/index.js";
import { MOCK_CODE_RULES } from "../../src/cli/index.js";
import { installPack } from "../support/installed-packs.js";

// Core ships no packs; these tests add, configure and copy the complexity pack's rules.
installPack("complexity");

function git(cwd: string, ...args: string[]): void {
  execFileSync("git", args, { cwd, stdio: "ignore" });
}

function gitRepository(path: string, origin: string): string {
  mkdirSync(path, { recursive: true });
  git(path, "init", "-q", "-b", "main");
  git(path, "remote", "add", "origin", origin);
  return path;
}

interface ToolAnswer {
  status: number;
  /** The tool's structured result, or the refusal text of an error result. */
  result: Record<string, unknown> | string;
}

describe("MCP callers and the agent guard", () => {
  let root: string;
  let parent: string;
  let nested: string;
  let databasePath: string;
  let stores: DatabaseApiStores;
  let mcp: (request: Request) => Promise<Response>;
  let parentKey: string;
  let nestedKey: string;

  beforeEach(async () => {
    root = realpathSync(await mkdtemp(join(tmpdir(), "rerail-mcp-caller-")));
    parent = gitRepository(join(root, "app"), "git@github.com:acme/app.git");
    mkdirSync(join(parent, "web", "src"), { recursive: true });
    // A repository inside another (a submodule's checkout) is its own repository.
    nested = gitRepository(join(parent, "vendor", "lib"), "https://github.com/acme/lib.git");
    databasePath = join(root, "rerail.sqlite");
    stores = new DatabaseApiStores(databasePath, {
      health: () => ({ pid: process.pid, database: { path: databasePath, exists: true }, native: { ok: true } }),
      origin: () => "http://127.0.0.1:4737",
    });
    parentKey = recordCheckout(stores.database(), parent).repositoryKey;
    nestedKey = recordCheckout(stores.database(), nested).repositoryKey;
    mcp = createRerailMcpHandler(stores, operations, { version: "0.0.0-test" });
  });

  afterEach(async () => {
    stores.close();
    await rm(root, { recursive: true, force: true });
  });

  async function call(name: string, args: Record<string, unknown>, cwd?: string | null): Promise<ToolAnswer> {
    const response = await mcp(new Request("http://localhost:4737/mcp", {
      method: "POST",
      headers: {
        host: "localhost:4737",
        "content-type": "application/json",
        accept: "application/json, text/event-stream",
        ...(cwd === undefined || cwd === null ? {} : { [CALLER_CWD_HEADER]: cwd }),
      },
      body: JSON.stringify({ jsonrpc: "2.0", id: 1, method: "tools/call", params: { name, arguments: args } }),
    }));
    const body = await response.json() as { result?: { isError?: boolean; structuredContent?: Record<string, unknown>; content: Array<{ text: string }> }; error?: { code: number; message: string } };
    if (body.error !== undefined) return { status: response.status, result: `${body.error.code}: ${body.error.message}` };
    return { status: response.status, result: body.result?.isError === true ? body.result.content[0]!.text : body.result!.structuredContent! };
  }

  const from = (path: string): string => encodeURIComponent(path);

  it("refuses a caller header that is not an absolute, existing folder with invalid params", async () => {
    for (const header of ["relative/path", encodeURIComponent(`${parent}\0x`), encodeURIComponent(join(root, "gone")), "%E0%A4%A"]) {
      const answer = await call("repository_context_get", {}, header);
      expect(answer.status).toBe(400);
      expect(answer.result).toMatch(/^-32602: x-rerail-caller-cwd /u);
    }
  });

  it("resolves a target-less call to the caller's checkout and folder, and a nested repository to itself", async () => {
    expect(await call("repository_context_get", {}, from(join(parent, "web", "src")))).toMatchObject({
      status: 200,
      result: { repository: { repositoryKey: parentKey }, checkout: { checkoutPath: parent, folder: "web/src" } },
    });
    expect(await call("repository_context_get", {}, from(nested))).toMatchObject({
      result: { repository: { repositoryKey: nestedKey }, checkout: { checkoutPath: nested, folder: "" } },
    });
    // Without a header, a direct MCP client must name its target, and then reaches nothing: it works in no repository.
    expect((await call("repository_context_get", {})).result).toMatch(/^target-required/u);
    expect((await call("repository_context_get", { repository: nestedKey })).result).toMatch(/^agent-not-permitted: a coding agent reads only/u);
    // A nested repository inside the caller's own tree is its work too; a sibling is not.
    expect(await call("repository_context_get", { checkout: nested }, from(parent))).toMatchObject({ result: { repository: { repositoryKey: nestedKey } } });
    const sibling = gitRepository(join(root, "sibling"), "https://github.com/acme/sibling.git");
    recordCheckout(stores.database(), sibling);
    for (const [name, args] of [["repository_context_get", { checkout: sibling }], ["repository_rules_get", { checkout: sibling }], ["rule_packs_recommend", { checkout: sibling }]] as const) {
      expect((await call(name, args, from(parent))).result, name).toMatch(/^agent-not-permitted: a coding agent reads only in the repository it works in/u);
    }
    expect((await call("repositories_list", {}, from(parent))).result).toMatchObject({ repositories: [{ repositoryKey: parentKey }] });
    expect((await call("repositories_list", {})).result).toEqual({ repositories: [] });
    expect((await call("presentation_get", { repository: sibling, host: "omp" }, from(parent))).result).toMatch(/^agent-not-permitted/u);
    expect((await call("presentation_get", { repository: nested, host: "omp" }, from(parent))).result).toMatchObject({ scope: { repositoryKey: nestedKey } });
    // A person sees every repository, as before.
    const app = createRerailApi(stores, operations, { version: "0.0.0-test" });
    const listed = await (await app.request("/api/repositories")).json() as { repositories: Array<{ repositoryKey: string }> };
    expect(listed.repositories.map(repository => repository.repositoryKey).sort()).toEqual([nestedKey, parentKey, "remote:github.com/acme/sibling"].sort());
  });

  it("resolves a caller folder to a nested checkout recorded after the folder was first seen", async () => {
    const tools = join(parent, "tools");
    mkdirSync(tools, { recursive: true });
    expect(await call("repository_context_get", {}, from(tools))).toMatchObject({
      result: { repository: { repositoryKey: parentKey }, checkout: { checkoutPath: parent, folder: "tools" } },
    });
    // `tools/` becomes a repository of its own, and Rerail records it.
    const toolsRoot = gitRepository(tools, "https://github.com/acme/tools.git");
    const toolsKey = recordCheckout(stores.database(), toolsRoot).repositoryKey;
    expect(await call("repository_context_get", {}, from(tools))).toMatchObject({
      result: { repository: { repositoryKey: toolsKey }, checkout: { checkoutPath: toolsRoot, folder: "" } },
    });
  });

  it("lets an agent change only a set-up repository's checkout settings, rules it just created, and copies it edits", async () => {
    const caller = from(join(parent, "web"));
    const repositoryOff = { owner: "repository", targets: [{ kind: "rule", id: "mock-no-console-log" }], on: false };
    // Before set-up, the agent may configure the repository.
    expect((await call("repository_settings_set", repositoryOff, caller)).result).toMatchObject({ settings: [{ owner: "repository", folder: "" }] });
    expect((await call("repository_attachments_set", { configure: true }, caller)).result).toMatchObject({ repository: { setUp: true } });

    // After set-up: repository settings and attachments are refused, naming the scoped dashboard.
    const refused = await call("repository_settings_set", repositoryOff, caller);
    expect(refused.result).toMatch(/^agent-not-permitted: .*http:\/\/127\.0\.0\.1:4737\/\?repository=.*&checkout=.*#rules$/u);
    expect((await call("repository_attachments_set", { add: [{ sourceKind: "library_rule", sourceId: "x" }] }, caller)).result)
      .toMatch(/^agent-not-permitted/u);
    expect((await call("repository_link", { address: "git@github.com:acme/app-renamed.git" }, caller)).result).toMatch(/^agent-not-permitted/u);
    // Clearing history is the person's, in its own checkout too.
    expect((await call("repository_reset", {}, caller)).result).toMatch(/^agent-not-permitted/u);
    // The checkout's own settings are the agent's to change.
    expect((await call("repository_settings_set", { ...repositoryOff, owner: "checkout" }, caller)).result)
      .toMatchObject({ settings: [{ owner: "checkout", ownerId: parent }] });

    // A rule the agent creates in the same call may be added; an existing one may not.
    const created = MOCK_CODE_RULES[0]!;
    expect((await call("library_rules_upsert", { id: created.id, kind: "code", rule: created, attach: true, folder: "web" }, caller)).result)
      .toMatchObject({ attached: { repositoryKey: parentKey, folders: ["web"] } });
    expect((await call("library_rules_upsert", { id: created.id, kind: "code", rule: created, attach: true }, caller)).result)
      .toMatch(/^agent-not-permitted/u);
    // Editing an existing rule a set-up repository runs is refused too: it would change that repository's rules.
    expect((await call("library_rules_upsert", { id: created.id, kind: "code", rule: created }, caller)).result)
      .toMatch(/^agent-not-permitted/u);
    expect((await call("library_rules_remove", { id: created.id }, caller)).result).toMatch(/^agent-not-permitted/u);

    // Editing a By Rerail rule stores a copy, added where the original is; it waits for a person's approval, and the
    // original keeps running there until then.
    const ruleCatalog = loadRuleCatalog();
    const [ruleId, packId] = [...ruleCatalog.packOf].find(([id]) => ruleCatalog.documents.get(id)?.kind === "code")!;
    addAttachment(stores.database(), { repositoryKey: parentKey, folder: "web", sourceKind: "pack", sourceId: packId });
    const original = ruleCatalog.documents.get(ruleId)!.rule as CodeRule;
    const copied = await call("library_rules_upsert", { id: ruleId, kind: "code", rule: { ...original, revision: original.revision + 1 } }, caller);
    expect(copied.result).toMatchObject({ copiedFromRuleId: ruleId, attached: { repositoryKey: parentKey, folders: ["web"] } });
    // Editing its own copy later is allowed, as making it was.
    const copy = (copied.result as { rule: { rule: CodeRule } }).rule.rule;
    expect((await call("library_rules_upsert", { id: copy.id, kind: "code", rule: { ...copy, revision: copy.revision + 1 } }, caller)).result)
      .toMatchObject({ rule: { revision: copy.revision + 1 } });
  });

  it("folds attachment settings for one target and folder before writing them, and writes them with the attachments", async () => {
    const app = createRerailApi(stores, operations, { version: "0.0.0-test" });
    const response = await app.request("/api/repository/attachments", {
      method: "PUT",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        repository: parentKey,
        add: [{ sourceKind: "pack", sourceId: "complexity", folder: "web" }],
        settings: [
          { folder: "web", targetKind: "pack", targetId: "complexity", on: false },
          { folder: "web", targetKind: "pack", targetId: "complexity", mode: "log" },
        ],
      }),
    });
    expect(response.status).toBe(200);
    expect(stores.database().select({ on: ruleSettings.on, mode: ruleSettings.mode }).from(ruleSettings).all()).toEqual([{ on: false, mode: "log" }]);
  });

  it("sets a repository up when a library write adds its first rule", async () => {
    const app = createRerailApi(stores, operations, { version: "0.0.0-test" });
    const rule = MOCK_CODE_RULES[1]!;
    const response = await app.request(`/api/library-rules/${rule.id}`, {
      method: "PUT",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ kind: "code", rule, repository: parentKey, folder: "web", attach: true }),
    });
    expect(response.status).toBe(200);
    const repository = listRepositories(stores.database()).find(candidate => candidate.repositoryKey === parentKey)!;
    expect(repository.configuredAt).not.toBeNull();
  });

  it("edits a library rule whose id a later release bundles, rather than copying the By Rerail rule", async () => {
    const ruleCatalog = loadRuleCatalog();
    const [bundledId, document] = [...ruleCatalog.documents].find(([, candidate]) => candidate.kind === "code")!;
    // The library rule came first; this release then bundled the same id, and the library rule wins.
    writeLibraryRule(stores.database(), { ...ruleCatalog, documents: new Map() }, { kind: "code", rule: { ...(document.rule as CodeRule), mode: "log" } });
    const app = createRerailApi(stores, operations, { version: "0.0.0-test" });
    const response = await app.request(`/api/library-rules/${bundledId}`, {
      method: "PUT",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ kind: "code", rule: { ...(document.rule as CodeRule), mode: "steer" } }),
    });
    expect(response.status).toBe(200);
    expect(await response.json()).toMatchObject({ copiedFromRuleId: null, rule: { rule: { id: bundledId, mode: "steer" } } });
  });

  it("lets a person through /api make the same changes an agent is refused", async () => {
    setUpRepository(stores.database(), parentKey, { packs: [] });
    upsertLibraryRule(stores.database(), loadRuleCatalog(), MOCK_CODE_RULES[1]!, "code");
    const app = createRerailApi(stores, operations, { version: "0.0.0-test" });
    const put = (path: string, body: unknown) => app.request(path, {
      method: "PUT",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(body),
    });
    expect((await put("/api/repository/settings", { checkout: parent, owner: "repository", targets: [{ kind: "pack", id: "complexity" }], on: false })).status).toBe(200);
    expect((await put("/api/repository/attachments", {
      repository: parentKey, add: [{ sourceKind: "library_rule", sourceId: MOCK_CODE_RULES[1]!.id, folder: "web" }],
    })).status).toBe(200);
    // A checkout and a repository that disagree are refused rather than guessed between.
    const mismatch = await put("/api/repository/settings", { checkout: nested, repository: parentKey, owner: "repository", targets: [{ kind: "pack", id: "complexity" }], on: false });
    expect(mismatch.status).toBe(400);
    expect(await mismatch.json()).toMatchObject({ error: "target-mismatch" });

    // The API never records a checkout: an unseen one and a missing path get the same answer, and nothing is written.
    const unseen = gitRepository(join(root, "unseen"), "https://github.com/acme/unseen.git");
    const answers = await Promise.all([unseen, join(root, "missing")].map(async checkout => {
      const response = await put("/api/repository/settings", { checkout, owner: "checkout", targets: [{ kind: "pack", id: "complexity" }], on: false });
      return { status: response.status, body: await response.json() as { error: string; message: string } };
    }));
    expect(answers.map(answer => [answer.status, answer.body.error])).toEqual([[404, "checkout-not-found"], [404, "checkout-not-found"]]);
    expect(answers[0]!.body.message).toMatch(/Start Rerail there first: rerail start$/u);
    expect(listRepositories(stores.database()).flatMap(repository => repository.checkouts.map(row => row.checkoutPath))).not.toContain(unseen);
  });

  it("lets an agent change checkout settings and start state only in its own checkout tree", async () => {
    const sibling = gitRepository(join(root, "sibling"), "https://github.com/acme/sibling.git");
    recordCheckout(stores.database(), sibling);
    const caller = from(parent);
    const off = { owner: "checkout", targets: [{ kind: "pack", id: "complexity" }], on: false };
    expect((await call("repository_settings_set", { ...off, checkout: nested }, caller)).result).toMatchObject({ settings: [{ ownerId: nested }] });
    expect((await call("repository_settings_set", { ...off, checkout: sibling }, caller)).result).toMatch(/^agent-not-permitted/u);
    expect((await call("repository_stop", { checkout: sibling, scope: "checkout" }, caller)).result).toMatch(/^agent-not-permitted/u);
    expect((await call("repository_stop", { checkout: sibling, scope: "checkout" })).result).toMatch(/^agent-not-permitted/u);
    expect((await call("repository_stop", { scope: "checkout" }, caller)).result).toMatchObject({ checkout: { checkoutPath: parent, state: "stopped" } });
  });

  it("lets an agent change rules and settings only in its own repository, set up or not", async () => {
    const sibling = gitRepository(join(root, "sibling"), "https://github.com/acme/sibling.git");
    recordCheckout(stores.database(), sibling);
    const caller = from(parent);
    const rule = MOCK_CODE_RULES[1]!;
    // The sibling is not set up, which is no licence for an agent at work elsewhere to set it up with rules of its choosing.
    const refused = /^agent-not-permitted: a coding agent changes rules and settings only in the repository it works in/u;
    expect((await call("repository_attachments_set", { checkout: sibling, add: [{ sourceKind: "rerail_rule", sourceId: "mock-no-console-log" }] }, caller)).result).toMatch(refused);
    expect((await call("repository_settings_set", { checkout: sibling, owner: "repository", targets: [{ kind: "rule", id: "mock-no-console-log" }], on: false }, caller)).result).toMatch(refused);
    expect((await call("repository_follow_gitignore_set", { checkout: sibling, followGitignore: true }, caller)).result).toMatch(refused);
    expect((await call("library_rules_upsert", { id: rule.id, kind: "code", rule, attach: true, checkout: sibling }, caller)).result).toMatch(refused);
    expect((await call("library_rules_upsert", { id: rule.id, kind: "code", rule, attach: true, checkout: sibling })).result).toMatch(refused);
    // Its own repository, not set up: allowed as before.
    expect((await call("library_rules_upsert", { id: rule.id, kind: "code", rule, attach: true }, caller)).result).toMatchObject({ attached: { repositoryKey: parentKey } });
    // A person adds that rule to the sibling too; the agent may then neither edit nor delete it.
    const app = createRerailApi(stores, operations, { version: "0.0.0-test" });
    const attach = await app.request("/api/repository/attachments", {
      method: "PUT", headers: { "content-type": "application/json" },
      body: JSON.stringify({ checkout: sibling, add: [{ sourceKind: "library_rule", sourceId: rule.id }] }),
    });
    expect(attach.status).toBe(200);
    expect((await call("library_rules_upsert", { id: rule.id, kind: "code", rule }, caller)).result).toMatch(/^agent-not-permitted/u);
    expect((await call("library_rules_remove", { id: rule.id }, caller)).result).toMatch(/^agent-not-permitted/u);
    // The status line is the person's.
    expect((await call("presentation_preferences_upsert", { repository: "", host: "", presence: "silent" }, caller)).result).toMatch(/^agent-not-permitted/u);
    expect((await call("presentation_preferences_remove", { repository: "", host: "" }, caller)).result).toMatch(/^agent-not-permitted/u);
  });

  it("reads only the history of the caller's own repository, and answers for another's as for what does not exist", async () => {
    const sessions = stores.sessionStore();
    const auditStore = new SqliteAuditStore(databasePath);
    const repositoryAuditIds = new Map<string, string>();
    try {
      for (const [sessionId, checkout, repositoryKey] of [["in-app", parent, parentKey], ["in-lib", nested, nestedKey]] as const) {
        sessions.openSession({ host: "codex", sessionId, cwd: checkout }, sessionId === "in-app" ? 1_000 : 2_000, repositoryKey);
        sessions.append({ sessionId, eventId: "prompt", kind: "user-prompt", observedAt: 3_000, payload: { text: `cat ${checkout}/.env` } });
        sessions.append({ sessionId, eventId: "read-env", kind: "call", observedAt: 3_001, payload: { kind: "call", toolName: "Bash", input: { command: `cat ${checkout}/.env` } } });
        sessions.append({ sessionId, eventId: "env-out", kind: "result", observedAt: 3_002, payload: { kind: "result", toolName: "Bash", output: "API_KEY=hunter2" } });
        sessions.append({ sessionId, eventId: "read-src", kind: "call", observedAt: 3_003, payload: { kind: "call", toolName: "Read", input: { file_path: `${checkout}/src/app.ts` } } });
        stores.auditControls().claimRepositoryAudit(sessionId, checkout);
        const auditId = auditStore.start({
          sessionId, workspaceRoot: checkout, scope: "repository", mode: "log", revision: 1, catalogRevision: null, ruleRevisions: [], paths: [],
        });
        auditStore.finish(auditId, {
          status: "completed",
          violations: [
            { id: sessionId, ruleId: "rule", clauseId: "clause", code: "CODE", mode: "log", message: "m", path: "a.ts", captures: {}, variables: {} },
            {
              id: `${sessionId}-secret`, ruleId: "no-plaintext", clauseId: "clause", code: "SECRET", mode: "log", message: "`API_KEY: hunter2` is plaintext",
              path: "config/secrets.yaml", captures: {}, variables: { VALUE: "hunter2" },
              fix: { span: { startLine: 0, startColumn: 0, endLine: 0, endColumn: 7, startByte: 0, endByte: 7 }, text: "API_KEY: ${SECRET}" },
            },
          ],
        });
        stores.auditControls().finishRepositoryAudit(sessionId, checkout, auditId, "completed");
        repositoryAuditIds.set(sessionId, auditId);
      }
    } finally {
      auditStore.close();
    }
    const caller = from(join(parent, "web"));
    const ids = (answer: ToolAnswer, list: string): unknown[] =>
      (answer.result as Record<string, Array<Record<string, unknown>>>)[list]!.map(row => row.sessionId ?? row.workspaceRoot);

    expect(ids(await call("sessions_list", {}, caller), "sessions")).toEqual(["in-app"]);
    expect(ids(await call("sessions_list", { repository: parentKey }, caller), "sessions")).toEqual(["in-app"]);
    const found = (answer: ToolAnswer, list: string): unknown[] =>
      (answer.result as Record<string, Array<Record<string, unknown>>>)[list]!.map(row => [row.sessionId ?? row.workspaceRoot, row.path]);
    expect(found(await call("violations_list", { since: 0 }, caller), "violations").sort()).toEqual([["in-app", "a.ts"], ["in-app", "config/secrets.yaml"]]);
    expect((await call("violations_list", { since: 0, sessionId: "in-lib" }, caller)).result).toMatchObject({ total: 0 });
    expect(ids(await call("audits_list", {}, caller), "audits")).toEqual(["in-app"]);
    expect(ids(await call("repository_audits_list", {}, caller), "repositories")).toEqual([parent]);
    // A finding in a file named like one holding secrets reaches the agent by rule and place only, everywhere it is listed.
    const secretView = { ruleId: "no-plaintext", path: "config/secrets.yaml", message: expect.stringContaining("withheld"), fix: null };
    expect((await call("violations_list", { since: 0, ruleId: "no-plaintext" }, caller)).result).toMatchObject({ violations: [secretView] });
    const audits = (await call("audits_list", {}, caller)).result as { audits: Array<{ violations: Array<Record<string, unknown>> }> };
    const audited = audits.audits[0]!.violations.find(violation => violation.ruleId === "no-plaintext")!;
    expect(audited).toMatchObject({ ruleId: "no-plaintext", path: "config/secrets.yaml", message: expect.stringContaining("withheld"), variables: {} });
    expect(audited).not.toHaveProperty("fix");
    expect((await call("repository_audit_entries_list", { auditId: repositoryAuditIds.get("in-app"), kind: "violation" }, caller)).result)
      .toMatchObject({ entries: expect.arrayContaining([{ kind: "violation", violation: expect.objectContaining({ ruleId: "no-plaintext", message: expect.stringContaining("withheld"), variables: {} }) }]) });
    // A person reads the finding whole.
    const person = createRerailApi(stores, operations, { version: "0.0.0-test" });
    expect(await (await person.request("/api/violations?since=0&ruleId=no-plaintext")).json())
      .toMatchObject({ total: 2, violations: expect.arrayContaining([expect.objectContaining({ message: "`API_KEY: hunter2` is plaintext", fix: { text: "API_KEY: ${SECRET}", span: expect.anything() } })]) });
    expect((await call("rules_activity", { since: 0 }, caller)).result).toMatchObject({ sessions: { started: 1 } });
    // Its rules are its own repository's too: a library rule only the nested repository runs is the person's to see.
    const elsewhere = MOCK_CODE_RULES[1]!;
    upsertLibraryRule(stores.database(), loadRuleCatalog(), elsewhere, "code", { approve: true });
    addAttachment(stores.database(), { repositoryKey: nestedKey, folder: "", sourceKind: "library_rule", sourceId: elsewhere.id });
    const agentActivity = (await call("rules_activity", { since: 0 }, caller)).result;
    expect(agentActivity).toHaveProperty("rules");
    expect(agentActivity).not.toHaveProperty(["rules", elsewhere.id]);
    expect(await (await person.request("/api/rules/activity?since=0")).json()).toHaveProperty(["rules", elsewhere.id]);
    // The agent's view of its own repository's transcript: tool outputs withheld, as is a call over a secret-like path.
    const view = (await call("session_events", { sessionId: "in-app" }, caller)).result as { events: Array<{ eventId: string; payload: Record<string, unknown> }> };
    expect(view.events.map(event => [event.eventId, event.payload.text ?? event.payload.input ?? event.payload.output])).toEqual([
      ["prompt", `cat ${parent}/.env`],
      ["read-env", { withheld: "tool input", reason: expect.stringContaining("secret-like path") }],
      ["env-out", { withheld: "tool output", reason: expect.stringContaining("file's contents") }],
      ["read-src", { file_path: `${parent}/src/app.ts` }],
    ]);
    expect((await call("repository_audit_entries_list", { auditId: repositoryAuditIds.get("in-app") }, caller)).result).toMatchObject({ total: 2 });
    // Ongoing audit settings outlast the session, so an agent cannot set its checkout to log-only for everyone after it.
    expect((await call("audit_settings_update", { sessionId: "in-app", mode: "log" }, caller)).result).toMatch(/^agent-not-permitted: .*rerail settings/u);
    expect((await call("audit_settings_get", { sessionId: "in-app" }, caller)).result).toMatchObject({ settings: { mode: "steer" } });

    // Naming another repository is refused; its sessions and audits answer exactly as ones that do not exist.
    for (const name of ["sessions_list", "violations_list", "audits_list", "repository_audits_list", "rules_activity"]) {
      expect((await call(name, { since: 0, repository: nestedKey }, caller)).result).toMatch(/^agent-not-permitted: /u);
    }
    for (const name of ["sessions_get", "session_events", "work_links_list", "audit_settings_get"]) {
      expect((await call(name, { sessionId: "in-lib" }, caller)).result).toBe("session-not-found: session in-lib does not exist");
      expect((await call(name, { sessionId: "missing" }, caller)).result).toBe("session-not-found: session missing does not exist");
    }
    expect((await call("work_links_add", { sessionId: "in-lib", kind: "branch", reference: "main" }, caller)).result).toMatch(/^session-not-found: /u);
    expect((await call("repository_audit_entries_list", { auditId: repositoryAuditIds.get("in-lib") }, caller)).result)
      .toBe(`repository-audit-not-found: repository audit ${repositoryAuditIds.get("in-lib")} does not exist`);

    // An agent whose call names no checkout reads no history at all.
    for (const [name, args] of [["sessions_list", {}], ["session_events", { sessionId: "in-app" }], ["violations_list", { since: 0, repository: parentKey }]] as const) {
      expect((await call(name, args)).result).toMatch(/^agent-not-permitted: .*x-rerail-caller-cwd/u);
    }

    // A person reads every repository's history, as before.
    const app = createRerailApi(stores, operations, { version: "0.0.0-test" });
    const listed = await (await app.request("/api/sessions")).json() as { sessions: Array<{ sessionId: string }> };
    expect(listed.sessions.map(session => session.sessionId)).toEqual(["in-lib", "in-app"]);
    expect(await (await app.request("/api/sessions/in-lib/events")).json()).toMatchObject({
      events: [{ payload: { text: `cat ${nested}/.env` } }, { payload: { input: { command: `cat ${nested}/.env` } } }, { payload: { output: "API_KEY=hunter2" } }, {}],
    });
    expect((await app.request(`/api/repository-audits/${repositoryAuditIds.get("in-lib")}/entries`)).status).toBe(200);
  });

  it("reports where each rule's on and mode come from, for this worktree and for the repository (the worked example)", async () => {
    const ruleCatalog = loadRuleCatalog();
    const pack = [...ruleCatalog.packs.values()].find(candidate => candidate.rules.some(rule => rule.on && ruleCatalog.documents.get(rule.id)?.kind === "code"))!;
    const rule = pack.rules.find(candidate => candidate.on && ruleCatalog.documents.get(candidate.id)?.kind === "code")!;
    setUpRepository(stores.database(), parentKey, { packs: [{ packId: pack.id, folder: "web" }] });
    const app = createRerailApi(stores, operations, { version: "0.0.0-test" });
    const put = (body: unknown) => app.request("/api/repository/settings", {
      method: "PUT",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(body),
    });
    expect((await put({ repository: parentKey, owner: "repository", targets: [{ kind: "pack", id: pack.id }], mode: "log" })).status).toBe(200);
    expect((await put({ repository: parentKey, owner: "repository", folder: "web/legacy", targets: [{ kind: "rule", id: rule.id }], on: false })).status).toBe(200);
    expect((await put({ checkout: parent, owner: "checkout", targets: [{ kind: "rule", id: rule.id }], mode: "steer" })).status).toBe(200);

    type Rules = { rules: Array<{ rule: { id: string }; folder: string; on: boolean; mode: string; onSource: unknown; modeSource: unknown }> };
    const rulesFor = async (query: string) => {
      const body = await (await app.request(`/api/repository/rules?${query}`)).json() as Rules;
      return body.rules.find(candidate => candidate.rule.id === rule.id);
    };
    expect(await rulesFor(new URLSearchParams({ checkout: parent }).toString())).toEqual(expect.objectContaining({
      folder: "web",
      on: true,
      onSource: { by: "pack-start", packId: pack.id },
      mode: "steer",
      modeSource: { by: "checkout", checkoutPath: parent, folder: "", target: "rule" },
    }));
    expect(await rulesFor(new URLSearchParams({ repository: parentKey }).toString())).toEqual(expect.objectContaining({
      on: true,
      mode: "log",
      modeSource: { by: "repository", folder: "", target: "pack" },
    }));
  });
});
