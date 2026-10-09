import { execFileSync } from "node:child_process";
import { mkdirSync, realpathSync } from "node:fs";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { createRerailApi } from "../src/api/http.js";
import { createRerailMcpHandler } from "../src/api/mcp.js";
import { operations } from "../src/api/operations.js";
import { DatabaseApiStores } from "../src/api/stores.js";
import { loadBundledCatalog } from "../src/bundled-rules.js";
import type { CodeRule } from "../src/code-rule-schema.js";
import { CALLER_CWD_HEADER } from "../src/repository-model.js";
import { listRepositories, recordCheckout } from "../src/repository-registry.js";
import { addAttachment, setUpRepository } from "../src/rule-configuration.js";
import { upsertLibraryRule, writeLibraryRule } from "../src/rule-library.js";
import { ruleSettings } from "../src/schema.js";
import { MOCK_CODE_RULES } from "../src/seed.js";

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
    const databasePath = join(root, "rerail.sqlite");
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
    // Without a header, a direct MCP client must name its target.
    expect((await call("repository_context_get", {})).result).toMatch(/^target-required/u);
    expect(await call("repository_context_get", { repository: nestedKey })).toMatchObject({ result: { checkout: null } });
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
    // The checkout's own settings are the agent's to change.
    expect((await call("repository_settings_set", { ...repositoryOff, owner: "checkout" }, caller)).result)
      .toMatchObject({ settings: [{ owner: "checkout", ownerId: parent }] });

    // A rule the agent creates in the same call may be added; an existing one may not.
    const created = MOCK_CODE_RULES[0]!;
    expect((await call("library_rules_upsert", { id: created.id, kind: "file", rule: created, attach: true, folder: "web" }, caller)).result)
      .toMatchObject({ attached: { repositoryKey: parentKey, folders: ["web"] } });
    expect((await call("library_rules_upsert", { id: created.id, kind: "file", rule: created, attach: true }, caller)).result)
      .toMatch(/^agent-not-permitted/u);
    // Editing an existing rule a set-up repository runs is refused too: it would change that repository's rules.
    expect((await call("library_rules_upsert", { id: created.id, kind: "file", rule: created }, caller)).result)
      .toMatch(/^agent-not-permitted/u);
    expect((await call("library_rules_remove", { id: created.id }, caller)).result).toMatch(/^agent-not-permitted/u);

    // Editing a By Rerail rule stores a copy, added where the original is, and switches the original off there.
    const bundle = loadBundledCatalog();
    const [ruleId, packId] = [...bundle.packOf].find(([id]) => bundle.documents.get(id)?.kind === "file")!;
    addAttachment(stores.database(), { repositoryKey: parentKey, folder: "web", sourceKind: "pack", sourceId: packId });
    const original = bundle.documents.get(ruleId)!.rule as CodeRule;
    const copied = await call("library_rules_upsert", { id: ruleId, kind: "file", rule: { ...original, revision: original.revision + 1 } }, caller);
    expect(copied.result).toMatchObject({ copiedFromRuleId: ruleId, attached: { repositoryKey: parentKey, folders: ["web"] } });
    // Editing its own copy later is allowed, as making it was.
    const copy = (copied.result as { rule: { rule: CodeRule } }).rule.rule;
    expect((await call("library_rules_upsert", { id: copy.id, kind: "file", rule: { ...copy, revision: copy.revision + 1 } }, caller)).result)
      .toMatchObject({ rule: { revision: copy.revision + 1 } });
  });

  it("folds attachment settings for one target and folder before writing them, and writes them with the attachments", async () => {
    const app = createRerailApi(stores, operations, { version: "0.0.0-test" });
    const response = await app.request("/api/repository/attachments", {
      method: "PUT",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        repository: parentKey,
        add: [{ sourceKind: "pack", sourceId: "tslint", folder: "web" }],
        settings: [
          { folder: "web", targetKind: "pack", targetId: "tslint", on: false },
          { folder: "web", targetKind: "pack", targetId: "tslint", mode: "log" },
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
      body: JSON.stringify({ kind: "file", rule, repository: parentKey, folder: "web", attach: true }),
    });
    expect(response.status).toBe(200);
    const repository = listRepositories(stores.database()).find(candidate => candidate.repositoryKey === parentKey)!;
    expect(repository.configuredAt).not.toBeNull();
  });

  it("edits a library rule whose id a later release bundles, rather than copying the By Rerail rule", async () => {
    const bundle = loadBundledCatalog();
    const [bundledId, document] = [...bundle.documents].find(([, candidate]) => candidate.kind === "file")!;
    // The library rule came first; this release then bundled the same id, and the library rule wins.
    writeLibraryRule(stores.database(), { ...bundle, documents: new Map() }, { kind: "file", rule: { ...(document.rule as CodeRule), mode: "log" } });
    const app = createRerailApi(stores, operations, { version: "0.0.0-test" });
    const response = await app.request(`/api/library-rules/${bundledId}`, {
      method: "PUT",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ kind: "file", rule: { ...(document.rule as CodeRule), mode: "steer" } }),
    });
    expect(response.status).toBe(200);
    expect(await response.json()).toMatchObject({ copiedFromRuleId: null, rule: { rule: { id: bundledId, mode: "steer" } } });
  });

  it("lets a person through /api make the same changes an agent is refused", async () => {
    setUpRepository(stores.database(), parentKey, { packs: [] });
    upsertLibraryRule(stores.database(), loadBundledCatalog(), MOCK_CODE_RULES[1]!, "file");
    const app = createRerailApi(stores, operations, { version: "0.0.0-test" });
    const put = (path: string, body: unknown) => app.request(path, {
      method: "PUT",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(body),
    });
    expect((await put("/api/repository/settings", { checkout: parent, owner: "repository", targets: [{ kind: "pack", id: "tslint" }], on: false })).status).toBe(200);
    expect((await put("/api/repository/attachments", {
      repository: parentKey, add: [{ sourceKind: "library_rule", sourceId: MOCK_CODE_RULES[1]!.id, folder: "web" }],
    })).status).toBe(200);
    // A checkout and a repository that disagree are refused rather than guessed between.
    const mismatch = await put("/api/repository/settings", { checkout: nested, repository: parentKey, owner: "repository", targets: [{ kind: "pack", id: "tslint" }], on: false });
    expect(mismatch.status).toBe(400);
    expect(await mismatch.json()).toMatchObject({ error: "target-mismatch" });

    // The API never records a checkout: an unseen one and a missing path get the same answer, and nothing is written.
    const unseen = gitRepository(join(root, "unseen"), "https://github.com/acme/unseen.git");
    const answers = await Promise.all([unseen, join(root, "missing")].map(async checkout => {
      const response = await put("/api/repository/settings", { checkout, owner: "checkout", targets: [{ kind: "pack", id: "tslint" }], on: false });
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
    const off = { owner: "checkout", targets: [{ kind: "pack", id: "tslint" }], on: false };
    expect((await call("repository_settings_set", { ...off, checkout: nested }, caller)).result).toMatchObject({ settings: [{ ownerId: nested }] });
    expect((await call("repository_settings_set", { ...off, checkout: sibling }, caller)).result).toMatch(/^agent-not-permitted/u);
    expect((await call("repository_stop", { checkout: sibling, scope: "checkout" }, caller)).result).toMatch(/^agent-not-permitted/u);
    expect((await call("repository_stop", { checkout: sibling, scope: "checkout" })).result).toMatch(/^agent-not-permitted/u);
    expect((await call("repository_stop", { scope: "checkout" }, caller)).result).toMatchObject({ checkout: { checkoutPath: parent, state: "stopped" } });
  });

  it("reports where each rule's on and mode come from, for this worktree and for the repository (the worked example)", async () => {
    const bundle = loadBundledCatalog();
    const pack = [...bundle.packs.values()].find(candidate => candidate.rules.some(rule => rule.on && bundle.documents.get(rule.id)?.kind === "file"))!;
    const rule = pack.rules.find(candidate => candidate.on && bundle.documents.get(candidate.id)?.kind === "file")!;
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
