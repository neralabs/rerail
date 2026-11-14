import { mkdirSync, realpathSync } from "node:fs";
import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { removeFolder } from "../support/platform/remove-folder.js";
import type { OpenAPIHono } from "@hono/zod-openapi";
import { createRerailApi } from "../../src/api/http.js";
import { createRerailMcpHandler } from "../../src/api/mcp.js";
import { operations } from "../../src/api/operations.js";
import { DatabaseApiStores } from "../../src/api/stores.js";
import { CALLER_CWD_HEADER, mergeAssistStatusSchema, recordCheckout } from "../../src/engine/index.js";
import { MERGE_ASSIST_ERROR_CODES, MergeAssistError } from "../../src/runtime/index.js";
import { git } from "../support/merge-assist/git.js";
import { RecordingMergeAssistService } from "../support/merge-assist/status.js";

// Merge Assist is experimental: every test here runs with it on, as a daemon started with the variable does.
beforeEach(() => {
  vi.stubEnv("RERAIL_EXPERIMENTAL_MERGE_ASSIST", "1");
});

const CONFLICT = "conflict:overlap:0123456789abcdef01234567:89abcdef";

function gitRepository(path: string, origin: string): string {
  mkdirSync(path, { recursive: true });
  git(path, "init", "-q", "-b", "main");
  git(path, "remote", "add", "origin", origin);
  git(path, "commit", "-q", "--allow-empty", "-m", "start");
  return path;
}

interface Answer {
  status: number;
  body: Record<string, unknown>;
}

/** The Merge Assist operations over a recording service: what each surface hands the service, and what it refuses. */
describe("the Merge Assist operations", () => {
  let root: string;
  let checkout: string;
  let worktree: string;
  let other: string;
  let repositoryKey: string;
  let service: RecordingMergeAssistService;
  let stores: DatabaseApiStores;
  let app: Pick<OpenAPIHono, "request">;
  let mcp: (request: Request) => Promise<Response>;

  beforeEach(async () => {
    root = realpathSync(await mkdtemp(join(tmpdir(), "rerail-merge-assist-api-")));
    checkout = gitRepository(join(root, "app"), "git@github.com:acme/app.git");
    mkdirSync(join(checkout, "src", "api"), { recursive: true });
    worktree = join(root, "app-export");
    git(checkout, "worktree", "add", "-q", "-b", "tbelanger/export-api", worktree);
    other = gitRepository(join(root, "lib"), "git@github.com:acme/lib.git");
    const databasePath = join(root, "rerail.sqlite");
    service = new RecordingMergeAssistService();
    stores = new DatabaseApiStores(databasePath, {
      health: () => ({ pid: process.pid, database: { path: databasePath, exists: true }, native: { ok: true } }),
      origin: () => "http://127.0.0.1:4737",
      mergeAssist: service,
    });
    repositoryKey = recordCheckout(stores.database(), checkout).repositoryKey;
    recordCheckout(stores.database(), worktree);
    recordCheckout(stores.database(), other);
    app = createRerailApi(stores, operations, { version: "0.0.0-test" });
    mcp = createRerailMcpHandler(stores, operations, { version: "0.0.0-test" });
  });

  afterEach(async () => {
    stores.close();
    await removeFolder(root);
  });

  async function request(method: string, path: string, body?: Record<string, unknown>): Promise<Answer> {
    const response = await app.request(path, body === undefined ? { method } : {
      method, headers: { "content-type": "application/json" }, body: JSON.stringify(body),
    });
    return { status: response.status, body: await response.json() as Record<string, unknown> };
  }

  /** An MCP tool call from a coding agent whose bridge started in `cwd`: its structured result, or its refusal text. */
  async function call(name: string, args: Record<string, unknown>, cwd: string): Promise<Record<string, unknown> | string> {
    const response = await mcp(new Request("http://localhost:4737/mcp", {
      method: "POST",
      headers: {
        host: "localhost:4737", "content-type": "application/json", accept: "application/json, text/event-stream",
        [CALLER_CWD_HEADER]: encodeURIComponent(cwd),
      },
      body: JSON.stringify({ jsonrpc: "2.0", id: 1, method: "tools/call", params: { name, arguments: args } }),
    }));
    const body = await response.json() as { result: { isError?: boolean; structuredContent?: Record<string, unknown>; content: Array<{ text: string }> } };
    return body.result.isError === true ? body.result.content[0]!.text : body.result.structuredContent!;
  }

  it("hands a person's request to the service for the checkout its target names", async () => {
    for (const [method, path, body] of [
      ["GET", `/api/merge-assist?${new URLSearchParams({ repository: repositoryKey })}`],
      // A folder inside a checkout names that checkout: the linked worktree, not the repository's main checkout.
      ["GET", `/api/merge-assist?${new URLSearchParams({ checkout: join(worktree, ".") })}`],
      ["POST", "/api/merge-assist/scan", { checkout: join(checkout, "src", "api") }],
      ["GET", `/api/merge-assist/settings?${new URLSearchParams({ checkout })}`],
      // A hook without a timeout gets the default, 10 seconds, after the switch.
      ["PUT", "/api/merge-assist/settings", { checkout, enabled: true, hook: { command: ["./notify.sh", "--json"] } }],
      ["PUT", "/api/merge-assist/settings", { repository: repositoryKey, enabled: false }],
      ["POST", "/api/merge-assist/conflicts/dismiss", { checkout: worktree, conflictId: CONFLICT }],
    ] as const) {
      expect((await request(method, path, body)).status, `${method} ${path}`).toBe(200);
    }
    expect(service.calls).toEqual([
      { method: "status", checkout },
      { method: "status", checkout: worktree },
      { method: "scan", checkout },
      { method: "settings", checkout },
      { method: "setEnabled", checkout, enabled: true },
      { method: "updateSettings", checkout, change: { hook: { command: ["./notify.sh", "--json"], timeoutSeconds: 10 } } },
      { method: "setEnabled", checkout, enabled: false },
      { method: "dismissConflict", checkout: worktree, conflictId: CONFLICT },
    ]);
  });

  it("answers every operation 404 experimental-off while the experiment is off, over HTTP and MCP, before the service runs", async () => {
    vi.stubEnv("RERAIL_EXPERIMENTAL_MERGE_ASSIST", "");
    for (const [method, path, body] of [
      ["GET", `/api/merge-assist?${new URLSearchParams({ checkout })}`],
      ["POST", "/api/merge-assist/scan", { checkout }],
      ["GET", `/api/merge-assist/settings?${new URLSearchParams({ checkout })}`],
      ["PUT", "/api/merge-assist/settings", { checkout, enabled: true }],
      ["POST", "/api/merge-assist/conflicts/dismiss", { checkout, conflictId: CONFLICT }],
    ] as const) {
      expect(await request(method, path, body), `${method} ${path}`).toMatchObject({ status: 404, body: { error: "experimental-off" } });
    }
    expect(await call("merge_assist_get", {}, checkout)).toMatch(/^experimental-off: /u);
    expect(service.calls).toEqual([]);
  });

  it("refuses a target it cannot resolve and a body it does not take, before the service runs", async () => {
    const unseen = join(root, "unseen");
    mkdirSync(unseen);
    expect(await request("GET", "/api/merge-assist")).toMatchObject({ status: 400, body: { error: "target-required" } });
    expect(await request("GET", `/api/merge-assist?${new URLSearchParams({ repository: "remote:github.com/acme/unknown" })}`))
      .toMatchObject({ status: 404, body: { error: "repository-not-found" } });
    expect(await request("POST", "/api/merge-assist/scan", { checkout: unseen })).toMatchObject({ status: 404, body: { error: "checkout-not-found" } });
    expect(await request("POST", "/api/merge-assist/scan", { checkout, repository: "remote:github.com/acme/lib" }))
      .toMatchObject({ status: 400, body: { error: "target-mismatch" } });
    for (const [method, path, body] of [
      ["PUT", "/api/merge-assist/settings", { checkout }],
      ["PUT", "/api/merge-assist/settings", { checkout, enabled: "yes" }],
      ["PUT", "/api/merge-assist/settings", { checkout, hook: { command: "./notify.sh" } }],
      ["PUT", "/api/merge-assist/settings", { checkout, hook: { command: ["./notify.sh"], timeoutSeconds: 61 } }],
      ["PUT", "/api/merge-assist/settings", { checkout, hook: { command: ["./notify.sh"], shell: true } }],
      ["PUT", "/api/merge-assist/settings", { checkout: "relative/app", enabled: true }],
      ["POST", "/api/merge-assist/conflicts/dismiss", { checkout, conflictId: "" }],
      ["POST", "/api/merge-assist/conflicts/dismiss", { checkout, conflictId: CONFLICT, action: "theirs" }],
    ] as const) {
      expect((await request(method, path, body)).status, `${method} ${path} ${JSON.stringify(body)}`).toBe(400);
    }
    expect(service.calls).toEqual([]);
  });

  it("answers the service's refusals with an HTTP status under the service's code", async () => {
    const statuses: Record<string, number> = { "not-a-checkout": 404, "not-found": 404, "invalid-state": 409, disabled: 409 };
    for (const code of MERGE_ASSIST_ERROR_CODES) {
      service.failure = new MergeAssistError(code, `refused: ${code}`);
      expect(await request("POST", "/api/merge-assist/conflicts/dismiss", { checkout, conflictId: CONFLICT }))
        .toEqual({ status: statuses[code], body: { error: code, message: `refused: ${code}` } });
    }
    service.failure = new MergeAssistError("disabled", "Merge Assist is off for app.");
    expect(await call("merge_assist_scan", {}, checkout)).toBe("disabled: Merge Assist is off for app.");
  });

  it("lets a coding agent read and scan in its own checkout, and change nothing a person decides", async () => {
    const refused = expect.stringMatching(/^agent-not-permitted: /u);
    // No target: the checkout the agent's bridge started in, from any folder inside it.
    expect(mergeAssistStatusSchema.parse(await call("merge_assist_get", {}, join(checkout, "src", "api"))).checkout).toBe(checkout);
    expect(await call("merge_assist_scan", {}, checkout)).toMatchObject({ checkout });
    expect(await call("merge_assist_settings_get", {}, checkout)).toEqual({ hook: null });
    // Dismissing a conflict is the person's: refused before the service hears of it, even in the agent's own checkout.
    expect(await call("merge_assist_conflict_dismiss", { conflictId: CONFLICT }, checkout)).toEqual(refused);
    // Turning it on or off, and the hook command, which runs a program as the person, are the person's.
    expect(await call("merge_assist_settings_set", { enabled: true }, checkout)).toEqual(refused);
    expect(await call("merge_assist_settings_set", { hook: null }, checkout)).toEqual(refused);
    // Another checkout of the same repository: the agent may read it, not change it; another repository not at all.
    expect(await call("merge_assist_get", { checkout: worktree }, checkout)).toMatchObject({ checkout: worktree });
    expect(await call("merge_assist_conflict_dismiss", { checkout: worktree, conflictId: CONFLICT }, checkout)).toEqual(refused);
    // Naming the repository picks its main checkout, which is not the worktree the agent works in.
    expect(await call("merge_assist_scan", { repository: repositoryKey }, worktree)).toEqual(refused);
    expect(await call("merge_assist_get", { repository: repositoryKey }, other)).toEqual(refused);

    expect(service.calls).toEqual([
      { method: "status", checkout },
      { method: "scan", checkout },
      { method: "settings", checkout },
      { method: "status", checkout: worktree },
    ]);
  });

  it("lists every operation as an internal-testing MCP tool, the settings read and change on one OpenAPI path", async () => {
    const response = await mcp(new Request("http://localhost:4737/mcp", {
      method: "POST",
      headers: { host: "localhost:4737", "content-type": "application/json", accept: "application/json, text/event-stream" },
      body: JSON.stringify({ jsonrpc: "2.0", id: 1, method: "tools/list" }),
    }));
    const tools = (await response.json() as { result: { tools: Array<{ name: string; description: string }> } }).result.tools
      .filter(tool => tool.name.startsWith("merge_assist_"));
    expect(tools.map(tool => tool.name).sort()).toEqual([
      "merge_assist_conflict_dismiss", "merge_assist_get", "merge_assist_scan", "merge_assist_settings_get", "merge_assist_settings_set",
    ]);
    for (const tool of tools) expect(tool.description).toMatch(/^\[Internal testing\] /u);
    expect(tools.find(tool => tool.name === "merge_assist_settings_set")?.description).toContain("People only");

    const { paths } = (await request("GET", "/openapi.json")).body as { paths: Record<string, Record<string, { operationId: string }>> };
    expect([paths["/api/merge-assist/settings"]?.get?.operationId, paths["/api/merge-assist/settings"]?.put?.operationId])
      .toEqual(["merge_assist_settings_get", "merge_assist_settings_set"]);
  });
});
