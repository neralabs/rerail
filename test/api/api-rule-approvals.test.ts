import { execFileSync } from "node:child_process";
import { mkdirSync, realpathSync } from "node:fs";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import type { OpenAPIHono } from "@hono/zod-openapi";
import { createRerailApi } from "../../src/api/http.js";
import { ApiError, invokeOperation } from "../../src/api/operation.js";
import { operations } from "../../src/api/operations.js";
import { DatabaseApiStores } from "../../src/api/stores.js";
import { type RuleCatalog, PACKAGE_ROOT, readRuleCatalog, recordCheckout, setUpRepository } from "../../src/engine/index.js";
import { thirdPartyPack } from "../support/installed-packs.js";

type Json = Record<string, unknown>;
type RuleEntry = { rule: { id: string }; mode: string; modeSource: Json; approval?: { subject: Json; approved: boolean } };

describe("rule approvals", () => {
  let root: string;
  let checkout: string;
  let repositoryKey: string;
  let stores: DatabaseApiStores;
  let app: Pick<OpenAPIHono, "request">;

  beforeEach(async () => {
    root = realpathSync(await mkdtemp(join(tmpdir(), "rerail-rule-approvals-")));
    checkout = join(root, "app");
    mkdirSync(checkout, { recursive: true });
    execFileSync("git", ["init", "-q", "-b", "main"], { cwd: checkout, stdio: "ignore" });
    // Rerail's own TypeScript pack, and a third party's pack, as the registry installed them.
    const catalog = readRuleCatalog([join(PACKAGE_ROOT, "packs", "typescript"), thirdPartyPack(join(root, "copies"), "@acme/rerail-pack-lint", "lint")]);
    const databasePath = join(root, "rerail.sqlite");
    stores = new (class extends DatabaseApiStores {
      override ruleCatalog(): RuleCatalog {
        return catalog;
      }
    })(databasePath, {
      health: () => ({ pid: process.pid, database: { path: databasePath, exists: true }, native: { ok: true } }),
      origin: () => "http://127.0.0.1:4737",
    });
    repositoryKey = recordCheckout(stores.database(), checkout).repositoryKey;
    setUpRepository(stores.database(), repositoryKey, { packs: [{ packId: "typescript", folder: "" }, { packId: "lint", folder: "" }] });
    app = createRerailApi(stores, operations, { version: "0.0.0-test" });
  });

  afterEach(async () => {
    stores.close();
    await rm(root, { recursive: true, force: true });
  });

  const send = async (method: string, path: string, body?: unknown): Promise<{ status: number; body: Json }> => {
    const response = await app.request(path, body === undefined ? { method } : { method, headers: { "content-type": "application/json" }, body: JSON.stringify(body) });
    return { status: response.status, body: await response.json() as Json };
  };
  const rules = async (): Promise<Map<string, RuleEntry>> => {
    const { body } = await send("GET", `/api/repository/rules?${new URLSearchParams({ repository: repositoryKey })}`);
    return new Map((body.rules as RuleEntry[]).map(entry => [entry.rule.id, entry]));
  };
  const asAgent = (id: string, input: Json) => invokeOperation(operations.find(operation => operation.id === id)!, stores, input, { kind: "agent", checkout, folder: "" });

  it("runs a third party's pack in Log until a person approves what it ships, and Rerail's own pack as it is", async () => {
    const before = await rules();
    const lint = before.get("lint-assert-used")!;
    expect(lint).toMatchObject({ mode: "log", modeSource: { by: "awaiting-approval" }, approval: { subject: { kind: "pack", id: "@acme/rerail-pack-lint" }, approved: false } });
    expect(before.get("typescript-eslint-consistent-type-definitions")).not.toHaveProperty("approval");
    const { body: packs } = await send("GET", "/api/rule-packs");
    const listed = (packs.packs as Array<{ id: string; approval?: Json }>).find(pack => pack.id === "lint");
    expect(listed?.approval).toEqual(lint.approval);

    expect(await send("POST", "/api/rule-approvals", lint.approval!.subject)).toMatchObject({ status: 200, body: { approved: lint.approval!.subject } });

    expect((await rules()).get("lint-assert-used")).toMatchObject({ mode: "steer", modeSource: { by: "pack-start", packId: "lint" }, approval: { approved: true } });
  });

  it("refuses a coding agent, content that changed since it was read, and a name nothing waits on", async () => {
    const subject = (await rules()).get("lint-assert-used")!.approval!.subject;

    await expect(asAgent("rule_approvals_set", subject)).rejects.toMatchObject({ status: 403, code: "agent-not-permitted" } satisfies Partial<ApiError>);
    expect(await send("POST", "/api/rule-approvals", { ...subject, digest: "sha256:other" })).toMatchObject({ status: 409, body: { error: "approval-stale" } });
    expect(await send("POST", "/api/rule-approvals", { kind: "pack", id: "typescript", digest: "sha256:x" })).toMatchObject({ status: 404 });
    expect((await rules()).get("lint-assert-used")).toMatchObject({ mode: "log", approval: { approved: false } });
  });

  it("approves a person's own write of a rule created by you, and leaves a coding agent's new rule waiting", async () => {
    const rule = (id: string, message: string) => ({
      kind: "code", attach: true, repository: repositoryKey,
      rule: { format: "rerail.policy/v1", id, revision: 1, mode: "steer", clauses: [{ id: "c", violation: { anchor: "witness.span", code: "X", message }, forbid: { regex: { pattern: "x", source: "file.content" } } }] },
    });
    expect(await send("PUT", "/api/library-rules/mine", rule("mine", "Remove x."))).toMatchObject({ status: 200 });
    await asAgent("library_rules_upsert", { id: "agents", ...rule("agents", "Remove x.") });

    const after = await rules();
    expect(after.get("mine")).toMatchObject({ mode: "steer", approval: { subject: { kind: "rule", id: "mine" }, approved: true } });
    expect(after.get("agents")).toMatchObject({ mode: "log", approval: { subject: { kind: "rule", id: "agents" }, approved: false } });
  });
});
