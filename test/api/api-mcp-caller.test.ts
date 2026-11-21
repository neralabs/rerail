import { execFileSync } from "node:child_process";
import { type EncodingOption, mkdirSync, type PathLike, realpathSync, writeFileSync } from "node:fs";
import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { basename, join } from "node:path";
import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { StreamableHTTPClientTransport } from "@modelcontextprotocol/sdk/client/streamableHttp.js";
import { eq } from "drizzle-orm";
import { afterEach, beforeEach, describe, expect, it, onTestFinished, vi } from "vitest";
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
  listLibraryRules,
} from "../../src/engine/index.js";
import { MOCK_CODE_RULES, MOCK_CONVERSATION_RULES } from "../../src/cli/index.js";
import { installPack } from "../support/installed-packs.js";
import { shortSpelling } from "../engine/support/short-path.js";
import { linkFolder } from "../support/platform/links.js";
import { removeFolder } from "../support/platform/remove-folder.js";
import { withExperiments } from "../support/environment/experiments.js";

withExperiments("REPOSITORY_SCAN");

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
    // Spelt as Rerail records a checkout: on Windows the native realpath gives each name's long form, where a temp
    // folder can be spelt with 8.3 short names (`C:\Users\RUNNER~1\...`).
    root = realpathSync.native(await mkdtemp(join(tmpdir(), "rerail-mcp-caller-")));
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
    vi.unstubAllEnvs();
    stores.close();
    await removeFolder(root);
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

  /** The names of the MCP tools a coding agent in `cwd` is offered. */
  async function toolNames(cwd: string): Promise<string[]> {
    const response = await mcp(new Request("http://localhost:4737/mcp", {
      method: "POST",
      headers: { host: "localhost:4737", "content-type": "application/json", accept: "application/json, text/event-stream", [CALLER_CWD_HEADER]: cwd },
      body: JSON.stringify({ jsonrpc: "2.0", id: 1, method: "tools/list", params: {} }),
    }));
    return (await response.json() as { result: { tools: Array<{ name: string }> } }).result.tools.map(tool => tool.name);
  }

  it("offers an operation of an experimental feature as a tool only while the daemon runs that feature", async () => {
    const experimental = operations.filter(operation => operation.experiment !== undefined).map(operation => operation.id);
    expect(experimental).toEqual(expect.arrayContaining(["audit_requests_create", "repository_audits_list", "merge_assist_get"]));
    vi.stubEnv("RERAIL_EXPERIMENTAL_REPOSITORY_SCAN", "0");
    vi.stubEnv("RERAIL_EXPERIMENTAL_MERGE_ASSIST", "0");

    const offered = await toolNames(from(parent));
    expect(offered).toContain("repository_context_get");
    expect(offered.filter(name => experimental.includes(name))).toEqual([]);
    expect((await call("repository_audits_list", {}, from(parent))).result).toMatch(/Tool repository_audits_list not found/u);

    vi.stubEnv("RERAIL_EXPERIMENTAL", "1");
    vi.stubEnv("RERAIL_EXPERIMENTAL_REPOSITORY_SCAN", "");
    vi.stubEnv("RERAIL_EXPERIMENTAL_MERGE_ASSIST", "");
    try {
      expect(await toolNames(from(parent))).toEqual(expect.arrayContaining(experimental));
    } finally {
      vi.unstubAllEnvs();
    }
  });

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

  it("resolves a caller folder spelt with 8.3 short names to the checkout and folder Rerail records", async context => {
    const short = shortSpelling(join(parent, "web", "src"));
    if (short === undefined) {
      // Not Windows, or a volume with 8.3 names turned off: there is no short spelling to send.
      context.skip();
      return;
    }
    expect(await call("repository_context_get", {}, from(short))).toMatchObject({
      status: 200,
      result: { repository: { repositoryKey: parentKey }, checkout: { checkoutPath: parent, folder: "web/src" } },
    });
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

    // Editing a By Rerail rule stores a copy, added where the original is. The copy runs in Log until a person approves
    // it, and only that approval switches the original off: an agent's copy weakens nothing.
    const ruleCatalog = loadRuleCatalog();
    const [ruleId, packId] = [...ruleCatalog.packOf].find(([id]) => ruleCatalog.documents.get(id)?.kind === "code")!;
    addAttachment(stores.database(), { repositoryKey: parentKey, folder: "web", sourceKind: "pack", sourceId: packId });
    const original = ruleCatalog.documents.get(ruleId)!.rule as CodeRule;
    const copied = await call("library_rules_upsert", { id: ruleId, kind: "code", rule: { ...original, title: "Our copy", revision: original.revision + 1 } }, caller);
    expect(copied.result).toMatchObject({ copiedFromRuleId: ruleId, attached: { repositoryKey: parentKey, folders: ["web"] } });
    expect(stores.database().select().from(ruleSettings).where(eq(ruleSettings.targetId, ruleId)).all()).toEqual([]);
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
      body: JSON.stringify({ kind: "code", rule: { ...(document.rule as CodeRule), title: "Ours, from before the release", mode: "steer" } }),
    });
    expect(response.status).toBe(200);
    expect(await response.json()).toMatchObject({ copiedFromRuleId: null, rule: { rule: { id: bundledId, mode: "steer" } } });
  });

  it("refuses a rule written without a title, over /api and MCP and for a copy of a By Rerail rule, and stores nothing", async () => {
    vi.stubEnv("RERAIL_EXPERIMENTAL_CONVERSATION_RULES", "1");
    const app = createRerailApi(stores, operations, { version: "0.0.0-test" });
    const put = (id: string, body: unknown) => app.request(`/api/library-rules/${id}`, {
      method: "PUT",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(body),
    });
    const { title: _code, ...code } = MOCK_CODE_RULES[0]!;
    const { title: _conversation, ...conversation } = MOCK_CONVERSATION_RULES[0]!;
    const [bundledId, bundled] = [...loadRuleCatalog().documents].find(([, candidate]) => candidate.kind === "code")!;
    for (const [id, body] of [
      [code.id, { kind: "code", rule: code }],
      [conversation.id, { kind: "conversation", rule: conversation }],
      [bundledId, { kind: "code", rule: bundled.rule }],
    ] as const) {
      const response = await put(id, body);
      expect(response.status).toBe(400);
      expect(await response.json()).toMatchObject({ error: "rule-title-required", message: expect.stringMatching(/^rule\.title is required/u) });
    }
    expect((await call("library_rules_upsert", { id: code.id, kind: "code", rule: code })).result).toMatch(/^rule-title-required: rule\.title is required/u);
    expect(listLibraryRules(stores.database())).toEqual([]);
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
    upsertLibraryRule(stores.database(), loadRuleCatalog(), elsewhere, "code");
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
    // A place a turn end's walk could not check: the agent gets the count, never the place, as it gets no payload of it.
    const place = { path: ".secrets", reason: "EACCES: permission denied, opendir '/repo/.secrets'" };
    sessions.recordDiscoveryGap({ host: "codex", sessionId: "in-app", cwd: parent }, { boundary: "turn", ...place, key: JSON.stringify([".secrets", "EACCES"]) });
    const agentGaps = ((await call("sessions_get", { sessionId: "in-app" }, caller)).result as { gaps: Record<string, unknown> }).gaps;
    expect(agentGaps).toMatchObject({ discoveryGaps: 1 });
    expect(agentGaps).not.toHaveProperty("discoveryGapPlaces");
    expect(((await (await person.request("/api/sessions/in-app")).json()) as { gaps: unknown }).gaps)
      .toMatchObject({ discoveryGaps: 1, discoveryGapPlaces: [{ ...place, boundary: "turn", observedAt: expect.any(Number) }] });

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

  it("answers a coding agent's MCP client sessions_get within the output its tool declares, whether the walks met more among it", async () => {
    const sessions = stores.sessionStore();
    const session = { host: "codex" as const, sessionId: "in-app", cwd: parent };
    sessions.openSession(session, 1_000, parentKey);
    const place = { path: ".secrets", reason: "EACCES: permission denied, opendir '/repo/.secrets'" };
    sessions.recordDiscoveryGap(session, { boundary: "turn", ...place, key: JSON.stringify([".secrets", "EACCES"]) });
    const more = { boundary: "turn" as const, reason: "The walks could not check more than 1000 places; the rest are not recorded", more: true as const };
    sessions.recordDiscoveryGap(session, { ...more, key: JSON.stringify(["", more.reason]) });
    // The SDK's client checks each answer against the output schema its tool declared in `tools/list`, which allows no
    // field it does not name, as an MCP client that checks answers does.
    const client = new Client({ name: "rerail-test", version: "0.0.0" });
    const transport = new StreamableHTTPClientTransport(new URL("http://localhost:4737/mcp"), {
      fetch: async (url, init) => {
        const headers = new Headers(init?.headers);
        headers.set("host", "localhost:4737");
        headers.set(CALLER_CWD_HEADER, from(parent));
        return await mcp(new Request(url, { ...init, headers }));
      },
    });
    await client.connect(transport as Parameters<Client["connect"]>[0]);
    try {
      await client.listTools();
      const answer = await client.callTool({ name: "sessions_get", arguments: { sessionId: "in-app" } });
      expect(answer.structuredContent).toMatchObject({ gaps: { discoveryGaps: 1, discoveryGapsMore: true } });
      expect(answer.structuredContent).not.toHaveProperty(["gaps", "discoveryGapPlaces"]);
    } finally {
      await client.close();
    }
  });

  it("shows an agent a stored call that names a web URL without looking its host and path up on another machine", async () => {
    const command = "curl -s https://net/192.0.2.1/api/v1 | jq .";
    const sessions = stores.sessionStore();
    sessions.openSession({ host: "codex", sessionId: "in-app", cwd: parent }, 1_000, parentKey);
    sessions.append({ sessionId: "in-app", eventId: "url-call", kind: "call", observedAt: 2_000, payload: { kind: "call", toolName: "Bash", input: { command } } });
    // `//net/192.0.2.1/api/v1`, cut from the URL at its `:`, names the share `\\net\192.0.2.1\api\v1` on Windows and a
    // path under the automount root `/net` on Linux and macOS (192.0.2.1 is a documentation address, RFC 5737). Each
    // such lookup is recorded and refused here, not made, so the test reaches no host.
    const lookUp = realpathSync.native;
    const remote: string[] = [];
    onTestFinished(() => {
      vi.restoreAllMocks();
    });
    vi.spyOn(realpathSync, "native").mockImplementation((path: PathLike, options?: EncodingOption) => {
      if (!/^(?:\\\\[^.?\\]|\/net\/)/u.test(String(path))) return lookUp(path, options);
      remote.push(String(path));
      throw Object.assign(new Error(`ENOENT: no such file or directory, realpath '${String(path)}'`), { code: "ENOENT" });
    });

    const view = (await call("session_events", { sessionId: "in-app" }, from(parent))).result as { events: Array<{ payload: Record<string, unknown> }> };
    expect(view.events.map(event => event.payload.input)).toEqual([{ command }]);
    expect(remote).toEqual([]);
  });

  /**
   * Records a finished repository audit of `checkout` with one finding (text, variables and fix) per path, by a session
   * in `folder`. As the runtime keeps them, the paths are relative to the folder the audit ran in, its `workspaceRoot`.
   */
  function auditFinding(paths: readonly string[], options: { sessionId?: string; folder?: string; workspaceRoot?: string; checkout?: string } = {}): string {
    const { sessionId = "linked", folder = parent, workspaceRoot = folder, checkout = parent } = options;
    stores.sessionStore().openSession({ host: "codex", sessionId, cwd: folder }, 1_000, parentKey);
    const auditStore = new SqliteAuditStore(databasePath);
    try {
      stores.auditControls().claimRepositoryAudit(sessionId, checkout);
      const auditId = auditStore.start({
        sessionId, workspaceRoot, scope: "repository", mode: "log", revision: 1, catalogRevision: null, ruleRevisions: [], paths: [],
      });
      auditStore.finish(auditId, {
        status: "completed",
        violations: paths.map(path => ({
          id: path, ruleId: "no-plaintext", clauseId: "clause", code: "SECRET", mode: "log" as const, message: "`KEY = 'hunter2'` is plaintext",
          path, captures: {}, variables: { VALUE: "hunter2" },
          fix: { span: { startLine: 0, startColumn: 0, endLine: 0, endColumn: 3, startByte: 0, endByte: 3 }, text: "KEY = environ['KEY']" },
        })),
      });
      stores.auditControls().finishRepositoryAudit(sessionId, checkout, auditId, "completed");
      return auditId;
    } finally {
      auditStore.close();
    }
  }

  /**
   * How a finding {@link auditFinding} recorded reads: `whole`, or `withheld` (rule and place only). `violations_list`
   * gives no variables, and a missing fix as null.
   */
  function findingView({ message, fix = null, variables = {} }: Record<string, unknown>): string {
    if (typeof message === "string" && message.includes("withheld from this conversation") && fix === null && JSON.stringify(variables) === "{}") return "withheld";
    return message === "`KEY = 'hunter2'` is plaintext" && fix !== null ? "whole" : JSON.stringify({ message, fix, variables });
  }

  /**
   * How each finding of the audits `auditIds` reads, by path ({@link findingView}), in each list that serves findings.
   */
  async function findingViews(auditIds: readonly string[], caller: string | undefined): Promise<Array<Record<string, string>>> {
    const read = async (name: string, args: Record<string, unknown>, path: string): Promise<Record<string, unknown>> => {
      if (caller !== undefined) return (await call(name, args, caller)).result as Record<string, unknown>;
      return await (await createRerailApi(stores, operations, { version: "0.0.0-test" }).request(path)).json() as Record<string, unknown>;
    };
    const byPath = (rows: Array<Record<string, unknown>>): Record<string, string> => Object.fromEntries(rows.map(row => [String(row.path), findingView(row)]));
    const listed = await read("violations_list", { since: 0 }, "/api/violations?since=0") as { violations: Array<Record<string, unknown>> };
    const audits = await read("audits_list", {}, "/api/audits") as { audits: Array<{ violations: Array<Record<string, unknown>> }> };
    const entries: Array<Record<string, unknown>> = [];
    for (const auditId of auditIds) {
      const page = await read("repository_audit_entries_list", { auditId }, `/api/repository-audits/${auditId}/entries`) as { entries: Array<{ violation: Record<string, unknown> }> };
      entries.push(...page.entries.map(entry => entry.violation));
    }
    return [byPath(listed.violations), byPath(audits.audits.flatMap(audit => audit.violations)), byPath(entries)];
  }

  // A finding is kept under the path its file was read by, which can reach a folder holding secrets through a link:
  // `conf/config.py`, with `conf` a link (a junction on Windows) to `.aws`, is `.aws/config.py`. A coding agent reads it
  // by rule and place only, as it reads one in `.aws/` itself and as its steer said it; a link to an ordinary folder
  // changes nothing, and a person reads every finding whole.
  it("serves an agent a finding in a file a folder link leads into a folder holding secrets by rule and place only", async () => {
    mkdirSync(join(parent, ".aws"));
    writeFileSync(join(parent, ".aws", "config.py"), "KEY = 'hunter2'\n");
    linkFolder(".aws", join(parent, "conf"));
    writeFileSync(join(parent, "web", "src", "app.py"), "KEY = 'hunter2'\n");
    linkFolder(join("web", "src"), join(parent, "docs"));
    const auditIds = [auditFinding(["conf/config.py", ".aws/config.py", "docs/app.py", "web/src/app.py"])];
    const agent = { "conf/config.py": "withheld", ".aws/config.py": "withheld", "docs/app.py": "whole", "web/src/app.py": "whole" };
    expect(await findingViews(auditIds, from(join(parent, "web")))).toEqual([agent, agent, agent]);
    const person = { "conf/config.py": "whole", ".aws/config.py": "whole", "docs/app.py": "whole", "web/src/app.py": "whole" };
    expect(await findingViews(auditIds, undefined)).toEqual([person, person, person]);
  });

  // The runtime keeps a finding's path relative to the folder its session ran in, and its steer judged it there: from a
  // session in `web`, `conf/config.py` with `web/conf` a link to `../.aws` is `.aws/config.py`. The lists judge it in
  // that folder too, not from the top of the agent's checkout, where `conf` does not exist.
  it("serves an agent a finding kept relative to its session's folder by rule and place only where a link there leads into a folder holding secrets", async () => {
    mkdirSync(join(parent, ".aws"));
    writeFileSync(join(parent, ".aws", "config.py"), "KEY = 'hunter2'\n");
    const web = join(parent, "web");
    linkFolder("../.aws", join(web, "conf"));
    writeFileSync(join(web, "src", "app.py"), "KEY = 'hunter2'\n");
    const auditIds = [auditFinding(["conf/config.py", "src/app.py"], { folder: web })];
    const agent = { "conf/config.py": "withheld", "src/app.py": "whole" };
    expect(await findingViews(auditIds, from(web))).toEqual([agent, agent, agent]);
    expect(await findingViews(auditIds, from(parent))).toEqual([agent, agent, agent]);
    const person = { "conf/config.py": "whole", "src/app.py": "whole" };
    expect(await findingViews(auditIds, undefined)).toEqual([person, person, person]);
  });

  /** Makes `.aws/config.py`, a file holding secrets, in `folder`, and `<folder>/<link>` a link to that `.aws`. */
  function secretsBehind(folder: string, link: string): void {
    mkdirSync(join(folder, ".aws"), { recursive: true });
    writeFileSync(join(folder, ".aws", "config.py"), "KEY = 'hunter2'\n");
    linkFolder(".aws", join(folder, link));
  }

  /** Adds a worktree of the agent's checkout at `path`, detached at a first commit, and returns `path`. */
  function worktree(path: string): string {
    git(parent, "-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--allow-empty", "-m", "first");
    git(parent, "worktree", "add", "-q", "--detach", path);
    return path;
  }

  // A repository's history holds its other worktrees' findings, each relative to its own folder. One in a worktree inside
  // the agent's checkout is judged in that worktree, where `keys` leads into `.aws`.
  it("judges a finding from another worktree inside the agent's checkout in that worktree", async () => {
    const inner = worktree(join(parent, "worktrees", "inner"));
    secretsBehind(inner, "keys");
    const auditIds = [auditFinding(["keys/config.py"], { sessionId: "inner", folder: inner, checkout: inner })];
    const agent = { "keys/config.py": "withheld" };
    expect(await findingViews(auditIds, from(parent))).toEqual([agent, agent, agent]);
    const person = { "keys/config.py": "whole" };
    expect(await findingViews(auditIds, undefined)).toEqual([person, person, person]);
  });

  // A worktree beside the agent's checkout is outside it, where the lists read nothing on disk: its findings are judged
  // by name, though `conf` leads into `.aws` both in that worktree and in the agent's checkout.
  it("judges by name a finding from a worktree beside the agent's checkout", async () => {
    const beside = worktree(join(root, "beside"));
    secretsBehind(beside, "conf");
    secretsBehind(parent, "conf");
    const auditIds = [auditFinding(["conf/config.py", ".aws/config.py"], { sessionId: "beside", folder: beside, checkout: beside })];
    const agent = { "conf/config.py": "whole", ".aws/config.py": "withheld" };
    expect(await findingViews(auditIds, from(parent))).toEqual([agent, agent, agent]);
    const person = { "conf/config.py": "whole", ".aws/config.py": "whole" };
    expect(await findingViews(auditIds, undefined)).toEqual([person, person, person]);
  });

  // The folder an audit ran in may be gone since: the lists read nothing there, judge its findings by name and still
  // answer.
  it("judges by name a finding whose folder is gone since its audit ran", async () => {
    const gone = join(parent, "gone");
    const auditIds = [auditFinding([".aws/config.py", "config.py"], { sessionId: "gone", folder: gone, checkout: gone })];
    const agent = { ".aws/config.py": "withheld", "config.py": "whole" };
    expect(await findingViews(auditIds, from(parent))).toEqual([agent, agent, agent]);
  });

  // A folder of the agent's checkout may be a link that leads out of it: the lists read nothing where it leads, so a
  // finding there is judged by name, though in `elsewhere`, where the session ran, `keys` leads into `.aws`.
  it("judges by name a finding whose folder a link leads out of the agent's checkout", async () => {
    const elsewhere = join(root, "elsewhere");
    mkdirSync(elsewhere);
    secretsBehind(elsewhere, "keys");
    const out = join(parent, "web", "out");
    linkFolder(elsewhere, out);
    const auditIds = [auditFinding(["keys/config.py", ".aws/config.py"], { sessionId: "out", folder: out, checkout: out })];
    const agent = { "keys/config.py": "whole", ".aws/config.py": "withheld" };
    expect(await findingViews(auditIds, from(parent))).toEqual([agent, agent, agent]);
  });

  // An audit with no folder recorded has its findings judged by name, never against the daemon's own working folder,
  // which a relative path resolves against: here the agent's checkout, where `keys` leads into `.aws`.
  it("judges by name a finding whose folder was not recorded, where the daemon works in the agent's checkout", async () => {
    secretsBehind(parent, "keys");
    const auditIds = [auditFinding(["keys/config.py", ".aws/config.py"], { sessionId: "none", workspaceRoot: "", checkout: join(parent, "web") })];
    const agent = { "keys/config.py": "whole", ".aws/config.py": "withheld" };
    const started = process.cwd();
    process.chdir(parent);
    try {
      expect(await findingViews(auditIds, from(parent))).toEqual([agent, agent, agent]);
    } finally {
      process.chdir(started);
    }
  });

  // One answer resolves at most 256 distinct paths on disk (CHECKOUT_PATH_RESOLUTIONS in src/engine/path-boundary.ts)
  // across every folder its findings were recorded in, and judges the rest by name. Sessions in `web` and `api` recorded
  // 256 findings each, the first in `conf/config.py` and the last in `conf2/config.py`, with `conf` and `conf2` links to
  // `.aws`. `audits_list` judges every finding of both audits in one answer: it withholds both in the audit it judges
  // first, where `conf2/config.py` is the 256th path, and serves both whole in the other, from the 257th path on. A
  // lower limit fails the first audit, a higher one the second. `resolutions` repeats the limit on purpose: the boundary
  // check keeps this file from importing path-boundary.ts, and a new limit fails this case until the case changes too.
  it("resolves at most 256 paths per answer across the folders of its findings, past which it judges by name", async () => {
    const resolutions = 256;
    const paths = ["conf/config.py", ...Array.from({ length: resolutions - 2 }, (_, n) => `src/f${n}.py`), "conf2/config.py"];
    for (const name of ["web", "api"]) {
      const folder = join(parent, name);
      mkdirSync(folder, { recursive: true });
      secretsBehind(folder, "conf");
      linkFolder(".aws", join(folder, "conf2"));
      auditFinding(paths, { sessionId: name, folder, checkout: folder });
    }
    const { audits } = (await call("audits_list", {}, from(parent))).result as { audits: Array<{ violations: Array<Record<string, unknown>> }> };
    expect(audits.map(audit => audit.violations.length)).toEqual([resolutions, resolutions]);
    const ends = audits.map(audit => [audit.violations[0]!, audit.violations.at(-1)!].map(violation => [violation.path, findingView(violation)]));
    expect(ends).toEqual([
      [["conf/config.py", "withheld"], ["conf2/config.py", "withheld"]],
      [["conf/config.py", "whole"], ["conf2/config.py", "whole"]],
    ]);
  });

  // Windows opens a folder or a file by its 8.3 short name too (`AWS~1` for `.aws`, `WP-CON~1.PHP` for `wp-config.php`),
  // which no name check knows: a finding kept under one reaches an agent as the file it is.
  it("serves an agent a finding in a file holding secrets, named by a Windows short name, by rule and place only", async ({ skip }) => {
    skip(process.platform !== "win32", "only Windows gives a file or folder a short name");
    mkdirSync(join(parent, ".aws"));
    writeFileSync(join(parent, ".aws", "config.py"), "KEY = 'hunter2'\n");
    writeFileSync(join(parent, "web", "wp-config.php"), "<?php $key = 'hunter2';\n");
    const folder = basename(shortSpelling(join(parent, ".aws")) ?? ".aws");
    const file = basename(shortSpelling(join(parent, "web", "wp-config.php")) ?? "wp-config.php");
    if (folder === ".aws" || file === "wp-config.php") return skip("this volume makes no short names");
    const auditIds = [auditFinding([`${folder}/config.py`, `web/${file}`])];
    const agent = { [`${folder}/config.py`]: "withheld", [`web/${file}`]: "withheld" };
    expect(await findingViews(auditIds, from(parent))).toEqual([agent, agent, agent]);
    const person = { [`${folder}/config.py`]: "whole", [`web/${file}`]: "whole" };
    expect(await findingViews(auditIds, undefined)).toEqual([person, person, person]);
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
