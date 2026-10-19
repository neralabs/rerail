import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { createRerailApi, createRerailMcpHandler, operations, DatabaseApiStores } from "../../../src/api/index.js";
import { findDuplicateJsonKeys, parseJsonWithoutDuplicateKeys } from "../../../src/engine/json-duplicate-keys.js";
import { loadPackRuleDocuments } from "../../../src/engine/catalog/rule-documents.js";
import { loadRuleCatalog } from "../../../src/engine/catalog/rule-catalog.js";
import { openRerailDatabase } from "../../../src/engine/storage/database.js";
import { runRulesCommand, MOCK_CODE_RULES } from "../../../src/cli/index.js";
import { getLibraryRule, removeLibraryRule, upsertLibraryRule } from "../../../src/engine/catalog/rule-library.js";

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
  let databasePath: string;
  let stores: DatabaseApiStores;
  const codeRule = MOCK_CODE_RULES[0]!;
  /** The mock code rule's upsert body with its clause's `id` given twice; JSON.parse would keep the second. */
  const repeatedBody = JSON.stringify({ kind: "code", rule: codeRule })
    .replace('"id":"console-call"', '"id":"first","id":"console-call"');
  const originalRerailHome = process.env.RERAIL_HOME;

  beforeEach(async () => {
    root = await mkdtemp(join(tmpdir(), "rerail-duplicate-keys-"));
    process.env.RERAIL_HOME = root;
    databasePath = join(root, "rerail.sqlite");
    stores = new DatabaseApiStores(databasePath, {
      health: () => ({ pid: process.pid, database: { path: databasePath, exists: true }, native: { ok: true } }),
      origin: () => "http://127.0.0.1:4737",
    });
  });

  afterEach(async () => {
    stores.close();
    if (originalRerailHome === undefined) delete process.env.RERAIL_HOME;
    else process.env.RERAIL_HOME = originalRerailHome;
    await rm(root, { recursive: true, force: true });
  });

  const stored = (): boolean => getLibraryRule(stores.database(), codeRule.id) !== undefined;

  it("answers an HTTP rule write with a repeated key with 400 and its path, and stores nothing", async () => {
    const app = createRerailApi(stores, operations, { version: "0.0.0-test" });
    const put = (body: string) => app.request(`/api/library-rules/${codeRule.id}`, {
      method: "PUT",
      headers: { "content-type": "application/json" },
      body,
    });
    const rejected = await put(repeatedBody);
    expect(rejected.status).toBe(400);
    expect(await rejected.json()).toEqual({ error: "duplicate-key", message: "request body: duplicate key `id` at /rule/clauses/0" });
    expect(stored()).toBe(false);

    expect((await put(JSON.stringify({ kind: "code", rule: codeRule }))).status).toBe(200);
  });

  it("rejects a deeply nested discarded duplicate key instead of applying last-key-wins", async () => {
    const app = createRerailApi(stores, operations, { version: "0.0.0-test" });
    const nested = `${"[".repeat(10_000)}0${"]".repeat(10_000)}`;
    const body = JSON.stringify({ kind: "code", rule: codeRule }).replace('"kind":"code"', `"kind":${nested},"kind":"code"`);

    const rejected = await app.request(`/api/library-rules/${codeRule.id}`, {
      method: "PUT",
      headers: { "content-type": "application/json" },
      body,
    });

    expect(rejected.status).toBe(400);
    expect(await rejected.json()).toEqual({ error: "duplicate-key", message: "request body: JSON nesting exceeds duplicate-key validation limit" });
    expect(stored()).toBe(false);
  });

  it("answers an MCP tool call whose arguments repeat a key with a JSON-RPC error naming the path", async () => {
    const handler = createRerailMcpHandler(stores, operations, { version: "0.0.0-test" });
    const argumentsText = repeatedBody.replace(/^\{/u, `{"id":${JSON.stringify(codeRule.id)},`);
    const response = await handler(new Request("http://localhost:4737/mcp", {
      method: "POST",
      headers: { host: "localhost:4737", "content-type": "application/json", accept: "application/json, text/event-stream" },
      body: `{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"library_rules_upsert","arguments":${argumentsText}}}`,
    }));
    expect(response.status).toBe(400);
    expect(await response.json()).toEqual({
      jsonrpc: "2.0",
      id: 7,
      error: { code: -32602, message: "request body: duplicate key `id` at /params/arguments/rule/clauses/0" },
    });
    expect(stored()).toBe(false);
  });

  it("refuses to import a dump with a repeated key, naming its path", () => {
    const dumpPath = join(root, "rules.json");
    upsertLibraryRule(stores.database(), loadRuleCatalog(), codeRule, "code");
    expect(runRulesCommand(["export", "--output", dumpPath], () => "usage")).toBe(0);
    removeLibraryRule(stores.database(), codeRule.id);
    writeFileSync(dumpPath, readFileSync(dumpPath, "utf8").replace('"id": "console-call"', '"id": "first", "id": "console-call"'));
    expect(() => runRulesCommand(["import", dumpPath], () => "usage")).toThrow(`${dumpPath}: duplicate key \`id\` at /library/0/rule/clauses/0`);
    const reopened = openRerailDatabase(databasePath);
    try {
      expect(getLibraryRule(reopened, codeRule.id)).toBeUndefined();
    } finally {
      reopened.$client.close();
    }
  });

  it("refuses a bundled rule file with a repeated key", () => {
    const directory = join(root, "pack", "rules");
    mkdirSync(directory, { recursive: true });
    writeFileSync(join(directory, "a.json"), JSON.stringify(codeRule).replace('"format":', '"format":"rerail.policy/v1","format":'));
    expect(() => loadPackRuleDocuments([join(root, "pack")])).toThrow(`${join(directory, "a.json")}: duplicate key \`format\` at /`);
  });
});
