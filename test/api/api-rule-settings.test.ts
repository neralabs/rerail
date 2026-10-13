import { execFileSync } from "node:child_process";
import { mkdirSync, realpathSync } from "node:fs";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import type { OpenAPIHono } from "@hono/zod-openapi";
import { createRerailApi } from "../../src/api/http.js";
import { createRerailMcpHandler } from "../../src/api/mcp.js";
import { operations } from "../../src/api/operations.js";
import { DatabaseApiStores } from "../../src/api/stores.js";
import {
  type BundledCatalog,
  CALLER_CWD_HEADER,
  codeRuleSchema,
  PACKAGE_ROOT,
  readBundledCatalog,
  recordCheckout,
  setUpRepository,
  ruleSettings,
  upsertLibraryRule,
} from "../../src/engine/index.js";

const RULE = "typescript-eslint-consistent-type-definitions";
const typescriptPack = readBundledCatalog([join(PACKAGE_ROOT, "packs", "typescript"), join(PACKAGE_ROOT, "packs", "python")]);

/** The API over a machine whose bundle is the TypeScript and Python packs, whose rules declare options. */
class TypeScriptPackStores extends DatabaseApiStores {
  override bundle(): BundledCatalog {
    return typescriptPack;
  }
}

describe("rule settings: suppressions and config", () => {
  let root: string;
  let checkout: string;
  let repositoryKey: string;
  let stores: TypeScriptPackStores;
  let app: Pick<OpenAPIHono, "request">;

  beforeEach(async () => {
    root = realpathSync(await mkdtemp(join(tmpdir(), "rerail-rule-settings-")));
    checkout = join(root, "app");
    mkdirSync(checkout, { recursive: true });
    execFileSync("git", ["init", "-q", "-b", "main"], { cwd: checkout, stdio: "ignore" });
    execFileSync("git", ["remote", "add", "origin", "git@github.com:acme/app.git"], { cwd: checkout, stdio: "ignore" });
    const databasePath = join(root, "rerail.sqlite");
    stores = new TypeScriptPackStores(databasePath, {
      health: () => ({ pid: process.pid, database: { path: databasePath, exists: true }, native: { ok: true } }),
      origin: () => "http://127.0.0.1:4737",
    });
    repositoryKey = recordCheckout(stores.database(), checkout).repositoryKey;
    setUpRepository(stores.database(), repositoryKey, { packs: [{ packId: "typescript", folder: "" }] });
    app = createRerailApi(stores, operations, { version: "0.0.0-test" });
  });

  afterEach(async () => {
    stores.close();
    await rm(root, { recursive: true, force: true });
  });

  const put = async (path: string, body: unknown): Promise<{ status: number; body: Record<string, unknown> }> => {
    const response = await app.request(path, { method: "PUT", headers: { "content-type": "application/json" }, body: JSON.stringify(body) });
    return { status: response.status, body: await response.json() as Record<string, unknown> };
  };
  const setting = (values: Record<string, unknown>, target: { kind: string; id: string } = { kind: "rule", id: RULE }) =>
    put("/api/repository/settings", { repository: repositoryKey, owner: "repository", targets: [target], ...values });
  const ruleView = async (): Promise<Record<string, unknown>> => {
    const body = await (await app.request(`/api/repository/rules?${new URLSearchParams({ repository: repositoryKey })}`)).json() as { rules: Array<{ rule: { id: string } }> };
    return body.rules.find(entry => entry.rule.id === RULE) as unknown as Record<string, unknown>;
  };

  it("reports the pack's start for suppressions, then a setting's choice, and keeps an omitted field", async () => {
    expect(await ruleView()).toMatchObject({ suppressions: "honour", suppressionsSource: { by: "pack-start", packId: "typescript" }, config: null, configSource: {} });

    expect(await setting({ suppressions: "ignore" }, { kind: "pack", id: "typescript" })).toMatchObject({
      status: 200,
      body: { settings: [{ targetKind: "pack", targetId: "typescript", on: null, mode: null, suppressions: "ignore", config: null }] },
    });
    expect(await ruleView()).toMatchObject({ suppressions: "ignore", suppressionsSource: { by: "repository", folder: "", target: "pack" } });

    expect(await setting({ mode: "log" }, { kind: "pack", id: "typescript" })).toMatchObject({ body: { settings: [{ mode: "log", suppressions: "ignore" }] } });
    expect(await setting({ suppressions: null }, { kind: "pack", id: "typescript" })).toMatchObject({ body: { settings: [{ mode: "log", suppressions: null }] } });
    expect(await ruleView()).toMatchObject({ suppressions: "honour", suppressionsSource: { by: "pack-start", packId: "typescript" } });
  });

  it("stores a rule's option values and lists the resolved rule with them", async () => {
    expect(await setting({ config: { option: "type" } })).toMatchObject({ status: 200, body: { settings: [{ config: { option: "type" } }] } });
    const view = await ruleView();
    expect(view).toMatchObject({ config: { option: "type" }, configSource: { option: { by: "repository", folder: "", target: "rule" } }, configError: null });
    expect(JSON.stringify(view.rule)).toContain("\"config\":{\"option\":\"type\"}");
    expect(await setting({ config: null })).toMatchObject({ status: 200, body: { settings: [{ config: null }] } });
    expect(await ruleView()).toMatchObject({ config: null });
  });

  it("refuses a config the rule's document does not declare, or a value its option does not allow", async () => {
    const unknown = await setting({ config: { opton: "type" } });
    expect(unknown.status).toBe(400);
    expect(unknown.body).toMatchObject({ error: "invalid-setting", message: expect.stringMatching(/`opton` is not an option this document declares/u) });
    const wrongType = await setting({ config: { option: 1 } });
    expect(wrongType.status).toBe(400);
    expect(wrongType.body).toMatchObject({ error: "invalid-setting", message: expect.stringMatching(/config\.option: is not a value of the option's type/u) });
    expect(await ruleView()).toMatchObject({ config: null });
  });

  // A value the option's declaration allows can still make a clause the engine refuses: F841's dummy-variable-rgx
  // is any string, but `[` is no regex. The update is refused and the stored setting stays as it was.
  it("refuses a config the engine cannot compile, keeping the setting before it", async () => {
    const f841 = { kind: "rule", id: "ruff-f841-unused-variable" };
    expect(await setting({ config: { "dummy-variable-rgx": "^_$" } }, f841)).toMatchObject({ status: 200 });
    const refused = await setting({ config: { "dummy-variable-rgx": "[" } }, f841);
    expect(refused.status).toBe(400);
    expect(refused.body).toMatchObject({ error: "invalid-setting", message: expect.stringMatching(/config: the engine refuses the rule with these values: .*regex/u) });
    const rows = stores.database().select().from(ruleSettings).all().filter(row => row.targetId === f841.id);
    expect(rows.map(row => row.config)).toEqual([{ "dummy-variable-rgx": "^_$" }]);
  });

  it("refuses config on a pack, in a folder, and on a library rule", async () => {
    expect(await setting({ config: { option: "type" } }, { kind: "pack", id: "typescript" })).toMatchObject({
      status: 400, body: { error: "invalid-setting", message: expect.stringMatching(/config applies to one rule, not to pack typescript/u) },
    });
    expect(await put("/api/repository/settings", { repository: repositoryKey, owner: "repository", folder: "web", targets: [{ kind: "rule", id: RULE }], config: { option: "type" } }))
      .toMatchObject({ status: 400, body: { message: expect.stringMatching(/whole repository/u) } });

    const mine = codeRuleSchema.parse({
      format: "rerail.policy/v1", id: "mine", revision: 1, mode: "log",
      clauses: [{ id: "c", violation: { anchor: "witness.span", code: "X", message: "x" }, forbid: { regex: { pattern: "x", source: "file.content" } } }],
    });
    upsertLibraryRule(stores.database(), typescriptPack, mine, "code");
    expect(await setting({ config: { option: "type" } }, { kind: "rule", id: "mine" })).toMatchObject({
      status: 400, body: { error: "invalid-setting", message: expect.stringMatching(/config applies to rules by Rerail; mine is your library rule/u) },
    });
  });

  it("checks config in an attachments call before anything is written", async () => {
    const answer = await put("/api/repository/attachments", {
      repository: repositoryKey,
      add: [{ sourceKind: "rerail_rule", sourceId: RULE, folder: "web" }],
      settings: [{ targetKind: "rule", targetId: RULE, config: { opton: "type" } }],
    });
    expect(answer.status).toBe(400);
    const attachments = await (await app.request(`/api/repository/rules?${new URLSearchParams({ repository: repositoryKey })}`)).json() as { rules: Array<{ folder: string }> };
    expect(attachments.rules.some(entry => entry.folder === "web")).toBe(false);

    expect(await put("/api/repository/attachments", {
      repository: repositoryKey,
      settings: [{ targetKind: "rule", targetId: RULE, suppressions: "ignore" }, { targetKind: "rule", targetId: RULE, config: { option: "type" } }],
    })).toMatchObject({ status: 200 });
    expect(await ruleView()).toMatchObject({ suppressions: "ignore", config: { option: "type" } });
  });

  it("takes suppressions from a coding agent's MCP call", async () => {
    const mcp = createRerailMcpHandler(stores, operations, { version: "0.0.0-test" });
    const response = await mcp(new Request("http://localhost:4737/mcp", {
      method: "POST",
      headers: {
        host: "localhost:4737", "content-type": "application/json", accept: "application/json, text/event-stream",
        [CALLER_CWD_HEADER]: encodeURIComponent(checkout),
      },
      body: JSON.stringify({
        jsonrpc: "2.0", id: 1, method: "tools/call",
        params: { name: "repository_settings_set", arguments: { owner: "checkout", targets: [{ kind: "rule", id: RULE }], suppressions: "ignore" } },
      }),
    }));
    const body = await response.json() as { result: { structuredContent?: unknown; content: unknown } };
    expect(body.result.structuredContent, JSON.stringify(body.result.content)).toMatchObject({ settings: [{ owner: "checkout", ownerId: checkout, suppressions: "ignore" }] });
  });
});
