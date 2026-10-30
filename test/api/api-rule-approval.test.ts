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
import { CALLER_CWD_HEADER, recordCheckout } from "../../src/engine/index.js";
import { MOCK_CODE_RULES } from "../../src/cli/index.js";

interface ToolAnswer {
  status: number;
  /** The tool's structured result, or the refusal text of an error result. */
  result: Record<string, unknown> | string;
}

describe("rule approval through the API", () => {
  let root: string;
  let app: string;
  let stores: DatabaseApiStores;
  let mcp: (request: Request) => Promise<Response>;
  let repositoryKey: string;

  beforeEach(async () => {
    root = realpathSync(await mkdtemp(join(tmpdir(), "rerail-rule-approval-")));
    app = join(root, "app");
    mkdirSync(app, { recursive: true });
    execFileSync("git", ["init", "-q", "-b", "main"], { cwd: app, stdio: "ignore" });
    execFileSync("git", ["remote", "add", "origin", "git@github.com:acme/app.git"], { cwd: app, stdio: "ignore" });
    const databasePath = join(root, "rerail.sqlite");
    stores = new DatabaseApiStores(databasePath, {
      health: () => ({ pid: process.pid, database: { path: databasePath, exists: true }, native: { ok: true } }),
      origin: () => "http://127.0.0.1:4737",
    });
    repositoryKey = recordCheckout(stores.database(), app).repositoryKey;
    mcp = createRerailMcpHandler(stores, operations, { version: "0.0.0-test" });
  });

  afterEach(async () => {
    stores.close();
    await rm(root, { recursive: true, force: true });
  });

  /** A tool call from a coding agent working in the repository. */
  async function agent(name: string, args: Record<string, unknown>): Promise<ToolAnswer> {
    const response = await mcp(new Request("http://localhost:4737/mcp", {
      method: "POST",
      headers: {
        host: "localhost:4737",
        "content-type": "application/json",
        accept: "application/json, text/event-stream",
        [CALLER_CWD_HEADER]: encodeURIComponent(app),
      },
      body: JSON.stringify({ jsonrpc: "2.0", id: 1, method: "tools/call", params: { name, arguments: args } }),
    }));
    const body = await response.json() as { result?: { isError?: boolean; structuredContent?: Record<string, unknown>; content: Array<{ text: string }> } };
    return { status: response.status, result: body.result?.isError === true ? body.result.content[0]!.text : body.result!.structuredContent! };
  }

  /** A request from the person, through the dashboard's `/api`. */
  async function person(path: string, init: { method: string; body: unknown } | undefined = undefined): Promise<{ status: number; body: Record<string, unknown> }> {
    const response = await createRerailApi(stores, operations, { version: "0.0.0-test" }).request(path, init === undefined ? undefined : {
      method: init.method,
      headers: { "content-type": "application/json" },
      body: JSON.stringify(init.body),
    });
    return { status: response.status, body: await response.json() as Record<string, unknown> };
  }

  const rulesOfRepository = async () => (await person(`/api/repository/rules?repository=${encodeURIComponent(repositoryKey)}`)).body as {
    rules: Array<{ rule: { id: string } }>;
    unavailable: Array<{ sourceId: string; reason: string }>;
    awaitingApproval: number;
  };

  it("holds a rule a coding agent writes until the person approves it, and refuses the agent's own approval", async () => {
    const rule = MOCK_CODE_RULES[0]!;
    const proposed = await agent("library_rules_upsert", { id: rule.id, kind: "code", rule, attach: true });
    expect(proposed.result).toMatchObject({ rule: { approval: { state: "pending", approvedAt: null } }, attached: { repositoryKey } });

    let view = await rulesOfRepository();
    expect(view.rules.map(entry => entry.rule.id)).not.toContain(rule.id);
    expect(view.unavailable).toContainEqual(expect.objectContaining({ sourceId: rule.id, reason: "awaiting-approval" }));
    expect(view.awaitingApproval).toBe(1);

    const listed = await agent("rule_approvals_list", {});
    const pending = (listed.result as { rules: Array<{ id: string; owner: string; contentHash: string; approved: unknown }> }).rules;
    expect(pending).toEqual([expect.objectContaining({ id: rule.id, owner: "library", approved: null })]);

    const refused = await agent("rule_approvals_approve", { owner: "library", id: rule.id, contentHash: pending[0]!.contentHash });
    expect(refused.result).toMatch(/^agent-not-permitted/u);
    expect((await rulesOfRepository()).awaitingApproval).toBe(1);

    const approved = await person("/api/rule-approvals", { method: "POST", body: { owner: "library", id: rule.id, contentHash: pending[0]!.contentHash } });
    expect(approved.status).toBe(200);
    view = await rulesOfRepository();
    expect(view.rules).toContainEqual(expect.objectContaining({ rule: expect.objectContaining({ id: rule.id }), approval: expect.objectContaining({ state: "approved" }) }));
    expect(view.awaitingApproval).toBe(0);
  });

  it("refuses an approval for content that changed since the person read it", async () => {
    const rule = MOCK_CODE_RULES[0]!;
    await agent("library_rules_upsert", { id: rule.id, kind: "code", rule });
    const stale = await person("/api/rule-approvals", { method: "POST", body: { owner: "library", id: rule.id, contentHash: "sha256:not-what-it-says" } });
    expect(stale.status).toBe(409);
    expect(stale.body).toMatchObject({ error: "rule-changed" });
  });

  it("approves what the person saves in the dashboard as it is saved", async () => {
    const rule = MOCK_CODE_RULES[0]!;
    const saved = await person(`/api/library-rules/${encodeURIComponent(rule.id)}`, { method: "PUT", body: { kind: "code", rule } });
    expect(saved.status).toBe(200);
    expect(saved.body).toMatchObject({ rule: { approval: { state: "approved" } } });
    expect((await person("/api/rule-approvals")).body).toEqual({ rules: [] });
  });
});
