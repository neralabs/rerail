import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { createRerailApi } from "../src/api/http.js";
import { createRerailMcpHandler } from "../src/api/mcp.js";
import { operations, type RerailApiStores } from "../src/api/operations.js";
import { findDuplicateJsonKeys, parseJsonWithoutDuplicateKeys } from "../src/json-duplicate-keys.js";
import { exportScopedRules, importScopedRules, loadBundledRules } from "../src/rule-bootstrap.js";
import { ScopedRuleCatalog } from "../src/scoped-rule-catalog.js";
import { MOCK_CODE_RULES } from "../src/seed.js";

describe("findDuplicateJsonKeys", () => {
  it("finds repeated keys at any depth with their JSON Pointer", () => {
    const text = '{"a":1,"b":{"kind":"x","kind":"y"},"c":[{"k":1},{"k":2,"k":3}],"a":4}';
    expect(findDuplicateJsonKeys(text)).toEqual([
      { pointer: "/b", key: "kind" },
      { pointer: "/c/1", key: "k" },
      { pointer: "", key: "a" },
    ]);
  });

  it("compares keys after unescaping and ignores braces and quotes inside strings", () => {
    expect(findDuplicateJsonKeys('{"a":1,"\\u0061":2}')).toEqual([{ pointer: "", key: "a" }]);
    expect(findDuplicateJsonKeys('{"s":"{\\"s\\":1,\\"s\\":2}","t":["}",",",{"x/y":{"z":1,"z":2}}]}')).toEqual([
      { pointer: "/t/2/x~1y", key: "z" },
    ]);
    expect(findDuplicateJsonKeys(' [1, -2.5e3, true, false, null, {}, []] ')).toEqual([]);
  });

  it("parses like JSON.parse but names every repeated key", () => {
    expect(parseJsonWithoutDuplicateKeys('{"a":[1]}', "doc")).toEqual({ a: [1] });
    expect(() => parseJsonWithoutDuplicateKeys('{"rule":{"id":"x","id":"y"}}', "doc")).toThrow("doc: duplicate key `id` at /rule");
    expect(() => parseJsonWithoutDuplicateKeys('{"a":', "doc")).toThrow(SyntaxError);
  });
});

describe("rule documents with repeated keys are rejected on write", () => {
  let root: string;
  let catalog: ScopedRuleCatalog;
  let stores: RerailApiStores;
  const codeRule = MOCK_CODE_RULES[0]!;
  /** The mock code rule's upsert body with its clause's `id` given twice; JSON.parse would keep the second. */
  const repeatedBody = JSON.stringify({ kind: "file", rule: codeRule })
    .replace('"id":"console-call"', '"id":"first","id":"console-call"');

  beforeEach(async () => {
    root = await mkdtemp(join(tmpdir(), "rerail-duplicate-keys-"));
    catalog = ScopedRuleCatalog.open(join(root, "catalog.sqlite"), { repositoryId: "repo", workspaceRoot: root });
    stores = { rules: () => catalog } as unknown as RerailApiStores;
  });

  afterEach(async () => {
    catalog.close();
    await rm(root, { recursive: true, force: true });
  });

  it("answers an HTTP rule write with a repeated key with 400 and its path, and stores nothing", async () => {
    const app = createRerailApi(stores, operations, { version: "0.0.0-test" });
    const rejected = await app.request(`/api/rules/${codeRule.id}`, {
      method: "PUT",
      headers: { "content-type": "application/json" },
      body: repeatedBody,
    });
    expect(rejected.status).toBe(400);
    expect(await rejected.json()).toEqual({
      error: "duplicate-key",
      message: "request body: duplicate key `id` at /rule/clauses/0",
    });
    expect(catalog.get(codeRule.id)).toBeUndefined();

    const accepted = await app.request(`/api/rules/${codeRule.id}`, {
      method: "PUT",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ kind: "file", rule: codeRule }),
    });
    expect(accepted.status).toBe(200);
  });

  it("rejects a deeply nested discarded duplicate key instead of applying last-key-wins", async () => {
    const app = createRerailApi(stores, operations, { version: "0.0.0-test" });
    const nested = `${"[".repeat(10_000)}0${"]".repeat(10_000)}`;
    const body = JSON.stringify({ kind: "file", rule: codeRule }).replace('"kind":"file"', `"kind":${nested},"kind":"file"`);

    const rejected = await app.request(`/api/rules/${codeRule.id}`, {
      method: "PUT",
      headers: { "content-type": "application/json" },
      body,
    });

    expect(rejected.status).toBe(400);
    expect(await rejected.json()).toEqual({
      error: "duplicate-key",
      message: "request body: JSON nesting exceeds duplicate-key validation limit",
    });
    expect(catalog.get(codeRule.id)).toBeUndefined();
  });

  it("answers an MCP tool call whose arguments repeat a key with a JSON-RPC error naming the path", async () => {
    const handler = createRerailMcpHandler(stores, operations, { version: "0.0.0-test" });
    const argumentsText = repeatedBody.replace(/^\{/u, `{"id":${JSON.stringify(codeRule.id)},`);
    const response = await handler(new Request("http://localhost:4737/mcp", {
      method: "POST",
      headers: { host: "localhost:4737", "content-type": "application/json", accept: "application/json, text/event-stream" },
      body: `{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"rules_upsert","arguments":${argumentsText}}}`,
    }));
    expect(response.status).toBe(400);
    expect(await response.json()).toEqual({
      jsonrpc: "2.0",
      id: 7,
      error: { code: -32602, message: "request body: duplicate key `id` at /params/arguments/rule/clauses/0" },
    });
    expect(catalog.get(codeRule.id)).toBeUndefined();
  });

  it("refuses to import a dump with a repeated key, naming its path", () => {
    const dumpPath = join(root, "rules.json");
    catalog.upsertCodeRule(codeRule);
    exportScopedRules(catalog, dumpPath);
    catalog.remove([codeRule.id]);
    const text = readFileSync(dumpPath, "utf8").replace('"id": "console-call"', '"id": "first", "id": "console-call"');
    writeFileSync(dumpPath, text);
    expect(() => importScopedRules(catalog, dumpPath)).toThrow(`${dumpPath}: duplicate key \`id\` at /rules/0/rule/clauses/0`);
    expect(catalog.get(codeRule.id)).toBeUndefined();
  });

  it("refuses a bundled rule file with a repeated key", () => {
    const directory = join(root, "pack", "rules");
    mkdirSync(directory, { recursive: true });
    writeFileSync(join(directory, "a.json"), JSON.stringify(codeRule).replace('"format":', '"format":"rerail.policy/v1","format":'));
    expect(() => loadBundledRules([join(root, "pack")])).toThrow(`${join(directory, "a.json")}: duplicate key \`format\` at /`);
  });
});
