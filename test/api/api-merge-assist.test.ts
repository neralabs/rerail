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
import { CALLER_CWD_HEADER, mergeAssistSettingsSchema, mergeAssistStatusSchema, recordCheckout } from "../../src/engine/index.js";
import { MERGE_ASSIST_ERROR_CODES, MergeAssistError } from "../../src/runtime/index.js";
import { RecordingMergeAssistService } from "../support/merge-assist-status.js";
import { candidate, candidateFiles, commit, forbidRule, mergeAssistWorkspaces, writeFiles } from "../runtime/merge-assist/support/merge-assist-workspace.js";

const CANDIDATE = "candidate:retries-in-transport-3f9a1c";

function git(cwd: string, ...args: string[]): void {
  execFileSync("git", args, {
    cwd,
    stdio: "ignore",
    env: { ...process.env, GIT_AUTHOR_NAME: "t", GIT_AUTHOR_EMAIL: "t@example.com", GIT_COMMITTER_NAME: "t", GIT_COMMITTER_EMAIL: "t@example.com" },
  });
}

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

interface ToolAnswer {
  /** The tool's structured result, or the refusal text of an error result. */
  result: Record<string, unknown> | string;
}

/** An MCP tool call from a coding agent whose bridge started in `cwd`. */
async function toolCall(mcp: (request: Request) => Promise<Response>, name: string, args: Record<string, unknown>, cwd: string): Promise<ToolAnswer> {
  const response = await mcp(new Request("http://localhost:4737/mcp", {
    method: "POST",
    headers: {
      host: "localhost:4737", "content-type": "application/json", accept: "application/json, text/event-stream",
      [CALLER_CWD_HEADER]: encodeURIComponent(cwd),
    },
    body: JSON.stringify({ jsonrpc: "2.0", id: 1, method: "tools/call", params: { name, arguments: args } }),
  }));
  const body = await response.json() as { result: { isError?: boolean; structuredContent?: Record<string, unknown>; content: Array<{ text: string }> } };
  return { result: body.result.isError === true ? body.result.content[0]!.text : body.result.structuredContent! };
}

describe("the Merge Assist operations", () => {
  let root: string;
  let checkout: string;
  let worktree: string;
  let other: string;
  let databasePath: string;
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
    databasePath = join(root, "rerail.sqlite");
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
    await rm(root, { recursive: true, force: true });
  });

  async function request(method: string, path: string, body?: Record<string, unknown>): Promise<Answer> {
    const response = await app.request(path, body === undefined ? { method } : {
      method, headers: { "content-type": "application/json" }, body: JSON.stringify(body),
    });
    return { status: response.status, body: await response.json() as Record<string, unknown> };
  }

  const call = (name: string, args: Record<string, unknown>, cwd: string): Promise<ToolAnswer> => toolCall(mcp, name, args, cwd);

  it("answers a person with the status of the checkout the target names", async () => {
    const byRepository = await request("GET", `/api/merge-assist?${new URLSearchParams({ repository: repositoryKey })}`);
    expect(byRepository.status).toBe(200);
    expect(mergeAssistStatusSchema.parse(byRepository.body).checkout).toBe(checkout);

    // A folder inside a checkout names that checkout: the linked worktree, not the repository's main checkout.
    const byFolder = await request("GET", `/api/merge-assist?${new URLSearchParams({ checkout: join(worktree, ".") })}`);
    expect(mergeAssistStatusSchema.parse(byFolder.body).checkout).toBe(worktree);
    const inside = await request("GET", `/api/merge-assist?${new URLSearchParams({ checkout: join(checkout, "src", "api") })}`);
    expect(mergeAssistStatusSchema.parse(inside.body).checkout).toBe(checkout);

    expect(service.calls).toEqual([
      { method: "status", checkout },
      { method: "status", checkout: worktree },
      { method: "status", checkout },
    ]);
  });

  it("refuses a target it cannot resolve without calling the service", async () => {
    expect(await request("GET", "/api/merge-assist")).toMatchObject({ status: 400, body: { error: "target-required" } });
    expect(await request("GET", `/api/merge-assist?${new URLSearchParams({ repository: "remote:github.com/acme/unknown" })}`))
      .toMatchObject({ status: 404, body: { error: "repository-not-found" } });
    const unseen = join(root, "unseen");
    mkdirSync(unseen);
    expect(await request("POST", "/api/merge-assist/scan", { checkout: unseen })).toMatchObject({ status: 404, body: { error: "checkout-not-found" } });
    expect(await request("POST", "/api/merge-assist/scan", { checkout, repository: "remote:github.com/acme/lib" }))
      .toMatchObject({ status: 400, body: { error: "target-mismatch" } });
    expect(service.calls).toEqual([]);
  });

  it("passes a person's changes to the service and answers with the new status", async () => {
    for (const [method, path, body] of [
      ["POST", "/api/merge-assist/scan", { checkout }],
      ["PUT", "/api/merge-assist/settings", { checkout, enabled: true }],
      ["PUT", "/api/merge-assist/settings", { repository: repositoryKey, enabled: false }],
      ["PUT", "/api/merge-assist/candidates", { checkout: worktree, candidateId: CANDIDATE, state: "rejected" }],
      ["PUT", "/api/merge-assist/candidates", { checkout: worktree, candidateId: CANDIDATE, state: "active" }],
      ["PUT", "/api/merge-assist/findings", { checkout: worktree, findingId: "finding-1", action: "adopt" }],
      ["PUT", "/api/merge-assist/findings", { checkout: worktree, findingId: "finding-1", action: "undo" }],
      ["PUT", "/api/merge-assist/findings", { checkout: worktree, findingId: "finding-2", action: "dismiss" }],
      ["PUT", "/api/merge-assist/conflicts", { checkout, conflictId: "conflict-1", action: "theirs" }],
      ["PUT", "/api/merge-assist/conflicts", { checkout, conflictId: "conflict-1", action: "new", text: "Share matchers." }],
    ] as const) {
      const answer = await request(method, path, body);
      expect(answer.status, `${method} ${path} ${JSON.stringify(body)}`).toBe(200);
      mergeAssistStatusSchema.parse(answer.body);
    }
    expect(service.calls).toEqual([
      { method: "scan", checkout },
      { method: "setEnabled", checkout, enabled: true },
      { method: "setEnabled", checkout, enabled: false },
      { method: "setCandidateState", checkout: worktree, candidateId: CANDIDATE, state: "rejected" },
      { method: "setCandidateState", checkout: worktree, candidateId: CANDIDATE, state: "active" },
      { method: "resolveFinding", checkout: worktree, findingId: "finding-1", action: "adopt", actor: "person" },
      { method: "resolveFinding", checkout: worktree, findingId: "finding-1", action: "undo", actor: "person" },
      { method: "resolveFinding", checkout: worktree, findingId: "finding-2", action: "dismiss", actor: "person" },
      { method: "resolveConflict", checkout, conflictId: "conflict-1", action: "theirs", text: undefined, actor: "person" },
      { method: "resolveConflict", checkout, conflictId: "conflict-1", action: "new", text: "Share matchers.", actor: "person" },
    ]);
  });

  it("reads the person's settings and changes them with on or off, the delivery and the hook", async () => {
    const read = await request("GET", `/api/merge-assist/settings?${new URLSearchParams({ checkout })}`);
    expect(read.status).toBe(200);
    expect(mergeAssistSettingsSchema.parse(read.body)).toEqual({ delivery: "both", hook: null });

    const hooked = await request("PUT", "/api/merge-assist/settings", { checkout, delivery: "terminal", hook: { command: ["./notify.sh", "--json"] } });
    expect(hooked.status).toBe(200);
    // A hook without a timeout gets the default, 10 seconds; the answer is the status, with the saved settings.
    expect(mergeAssistStatusSchema.parse(hooked.body).settings).toEqual({ delivery: "terminal", hook: { command: ["./notify.sh", "--json"], timeoutSeconds: 10 } });
    expect(mergeAssistSettingsSchema.parse((await request("GET", `/api/merge-assist/settings?${new URLSearchParams({ repository: repositoryKey })}`)).body))
      .toEqual({ delivery: "terminal", hook: { command: ["./notify.sh", "--json"], timeoutSeconds: 10 } });

    const both = await request("PUT", "/api/merge-assist/settings", { checkout, enabled: true, hook: null });
    expect(mergeAssistStatusSchema.parse(both.body).settings).toEqual({ delivery: "terminal", hook: null });
    expect((await request("PUT", "/api/merge-assist/settings", { checkout, hook: { command: ["notify"], timeoutSeconds: 60 } })).status).toBe(200);

    expect(service.calls).toEqual([
      { method: "settings", checkout },
      { method: "updateSettings", checkout, change: { delivery: "terminal", hook: { command: ["./notify.sh", "--json"], timeoutSeconds: 10 } } },
      { method: "settings", checkout },
      { method: "setEnabled", checkout, enabled: true },
      { method: "updateSettings", checkout, change: { hook: null } },
      { method: "updateSettings", checkout, change: { hook: { command: ["notify"], timeoutSeconds: 60 } } },
    ]);
  });

  it("validates bodies before anything runs", async () => {
    for (const [path, body] of [
      ["/api/merge-assist/settings", { checkout, enabled: "yes" }],
      ["/api/merge-assist/settings", { checkout }],
      ["/api/merge-assist/settings", { checkout, delivery: "email" }],
      ["/api/merge-assist/settings", { checkout, hook: { command: [] } }],
      ["/api/merge-assist/settings", { checkout, hook: { command: [""] } }],
      ["/api/merge-assist/settings", { checkout, hook: { command: "./notify.sh" } }],
      ["/api/merge-assist/settings", { checkout, hook: { command: ["./notify.sh"], timeoutSeconds: 61 } }],
      ["/api/merge-assist/settings", { checkout, hook: { command: ["./notify.sh"], timeoutSeconds: 0 } }],
      ["/api/merge-assist/settings", { checkout, hook: { command: ["./notify.sh"], shell: true } }],
      ["/api/merge-assist/candidates", { checkout, candidateId: "candidate:../../etc", state: "rejected" }],
      ["/api/merge-assist/candidates", { checkout, candidateId: CANDIDATE, state: "deleted" }],
      ["/api/merge-assist/findings", { checkout, findingId: "", action: "dismiss" }],
      ["/api/merge-assist/findings", { checkout, findingId: "finding-1", action: "apply" }],
      ["/api/merge-assist/findings", { checkout, findingId: "finding-1", action: "dismiss", extra: true }],
      ["/api/merge-assist/conflicts", { checkout, conflictId: "conflict-1", action: "new" }],
      ["/api/merge-assist/conflicts", { checkout, conflictId: "conflict-1", action: "merge" }],
      ["/api/merge-assist/conflicts", { checkout, conflictId: "conflict-1", action: "mine", text: "x".repeat(601) }],
      ["/api/merge-assist/settings", { checkout: "relative/app", enabled: true }],
    ] as const) {
      expect((await request("PUT", path, body)).status, JSON.stringify(body)).toBe(400);
    }
    expect(service.calls).toEqual([]);
  });

  it("maps the service's refusals to HTTP statuses under the service's code", async () => {
    const statuses: Record<string, number> = { "not-a-checkout": 404, "not-found": 404, "invalid-state": 409, disabled: 409 };
    for (const code of MERGE_ASSIST_ERROR_CODES) {
      service.failure = new MergeAssistError(code, `refused: ${code}`);
      expect(await request("PUT", "/api/merge-assist/findings", { checkout, findingId: "finding-1", action: "adopt" }))
        .toEqual({ status: statuses[code], body: { error: code, message: `refused: ${code}` } });
    }
    service.failure = new MergeAssistError("disabled", "Merge Assist is off for app.");
    expect(await call("merge_assist_scan", {}, checkout)).toEqual({ result: "disabled: Merge Assist is off for app." });
  });

  it("lets a coding agent read, scan, dismiss and reject in its own checkout, and nothing a person decides", async () => {
    const refused = expect.stringMatching(/^agent-not-permitted: /u);
    // No target: the checkout the agent's bridge started in, from any folder inside it.
    const read = await call("merge_assist_get", {}, join(checkout, "src", "api"));
    expect(mergeAssistStatusSchema.parse(read.result).checkout).toBe(checkout);
    expect(mergeAssistStatusSchema.safeParse((await call("merge_assist_scan", {}, checkout)).result).success).toBe(true);
    expect((await call("merge_assist_candidate_set", { candidateId: CANDIDATE, state: "rejected" }, checkout)).result).toMatchObject({ checkout });
    expect((await call("merge_assist_finding_set", { findingId: "finding-2", action: "dismiss" }, checkout)).result).toMatchObject({ checkout });
    // A verdict on a conflict reaches the service as the agent's, and the service refuses it (below, for real).
    expect((await call("merge_assist_conflict_set", { conflictId: "conflict-1", action: "theirs" }, checkout)).result).toMatchObject({ checkout });
    expect((await call("merge_assist_settings_get", {}, checkout)).result).toEqual({ delivery: "both", hook: null });

    // Turning it on, restoring, adopting and undoing widen what steers the agent: the person's to do.
    expect((await call("merge_assist_settings_set", { enabled: true }, checkout)).result).toEqual(refused);
    expect((await call("merge_assist_settings_set", { enabled: false }, checkout)).result).toEqual(refused);
    // The delivery and the hook command are the person's too: the hook runs a program as them.
    expect((await call("merge_assist_settings_set", { delivery: "chat" }, checkout)).result).toEqual(refused);
    expect((await call("merge_assist_settings_set", { hook: { command: ["./notify.sh"] } }, checkout)).result).toEqual(refused);
    expect((await call("merge_assist_settings_set", { hook: null }, checkout)).result).toEqual(refused);
    expect((await call("merge_assist_candidate_set", { candidateId: CANDIDATE, state: "active" }, checkout)).result).toEqual(refused);
    expect((await call("merge_assist_finding_set", { findingId: "finding-1", action: "adopt" }, checkout)).result).toEqual(refused);
    expect((await call("merge_assist_finding_set", { findingId: "finding-1", action: "undo" }, checkout)).result).toEqual(refused);

    expect(service.calls).toEqual([
      { method: "status", checkout },
      { method: "scan", checkout },
      { method: "setCandidateState", checkout, candidateId: CANDIDATE, state: "rejected" },
      { method: "resolveFinding", checkout, findingId: "finding-2", action: "dismiss", actor: "agent" },
      { method: "resolveConflict", checkout, conflictId: "conflict-1", action: "theirs", text: undefined, actor: "agent" },
      { method: "settings", checkout },
    ]);
  });

  it("keeps a coding agent's changes in its own checkout and its reads in its own repository", async () => {
    const refused = expect.stringMatching(/^agent-not-permitted: /u);
    // Another checkout of the same repository: the agent may read it, not change it.
    expect((await call("merge_assist_get", { checkout: worktree }, checkout)).result).toMatchObject({ checkout: worktree });
    expect((await call("merge_assist_finding_set", { checkout: worktree, findingId: "finding-2", action: "dismiss" }, checkout)).result).toEqual(refused);
    expect((await call("merge_assist_scan", { checkout: worktree }, checkout)).result).toEqual(refused);
    expect((await call("merge_assist_candidate_set", { checkout: worktree, candidateId: CANDIDATE, state: "rejected" }, checkout)).result).toEqual(refused);
    // Naming the repository picks its main checkout, which is not the worktree the agent works in.
    expect((await call("merge_assist_scan", { repository: repositoryKey }, worktree)).result).toEqual(refused);
    // Another repository is not its business at all.
    expect((await call("merge_assist_get", { checkout }, other)).result).toEqual(refused);
    expect((await call("merge_assist_get", { repository: repositoryKey }, other)).result).toEqual(refused);

    expect(service.calls).toEqual([{ method: "status", checkout: worktree }]);
  });

  it("lists every Merge Assist operation as an internal-testing MCP tool", async () => {
    const response = await mcp(new Request("http://localhost:4737/mcp", {
      method: "POST",
      headers: { host: "localhost:4737", "content-type": "application/json", accept: "application/json, text/event-stream" },
      body: JSON.stringify({ jsonrpc: "2.0", id: 1, method: "tools/list" }),
    }));
    const tools = (await response.json() as { result: { tools: Array<{ name: string; description: string }> } }).result.tools
      .filter(tool => tool.name.startsWith("merge_assist_"));
    expect(tools.map(tool => tool.name).sort()).toEqual([
      "merge_assist_candidate_set", "merge_assist_conflict_set", "merge_assist_finding_set", "merge_assist_get", "merge_assist_scan",
      "merge_assist_settings_get", "merge_assist_settings_set",
    ]);
    for (const tool of tools) expect(tool.description).toMatch(/^\[Internal testing\] /u);
    expect(tools.find(tool => tool.name === "merge_assist_settings_set")?.description).toContain("People only");
  });

  it("documents reading and changing the settings on one OpenAPI path", async () => {
    const document = (await request("GET", "/openapi.json")).body as { paths: Record<string, Record<string, { operationId: string; requestBody?: { content: Record<string, { schema: { properties: Record<string, unknown> } }> } }>> };
    const settings = document.paths["/api/merge-assist/settings"]!;
    expect(settings.get?.operationId).toBe("merge_assist_settings_get");
    expect(settings.put?.operationId).toBe("merge_assist_settings_set");
    expect(Object.keys(settings.put!.requestBody!.content["application/json"]!.schema.properties).sort())
      .toEqual(["checkout", "delivery", "enabled", "hook", "repository"]);
  });
});

// Each test builds Git repositories and compiles rules in the Engine; under a full run that outlasts the default 5 s.
describe("a coding agent's dismissal, through the real service", { timeout: 30_000 }, () => {
  const workspace = mergeAssistWorkspaces();
  const RETRY = candidate({ id: CANDIDATE, rule: forbidRule(CANDIDATE, String.raw`\bretry\(`) });

  it("never undoes a decision that applies here; only a person does", async () => {
    const ws = await workspace();
    const service = ws.service();
    await service.setEnabled(ws.repo, true);
    commit(ws.worktree("jbarnes-retry"), candidateFiles(RETRY));
    const [absorbed] = (await service.scan(ws.repo)).findings;
    expect(absorbed).toMatchObject({ tier: "absorb", status: "applied" });
    const steering = ws.rulesOn(ws.repo);
    expect(steering).not.toEqual([]);

    const stores = new DatabaseApiStores(ws.databasePath, {
      health: () => ({ pid: process.pid, database: { path: ws.databasePath, exists: true }, native: { ok: true } }),
      origin: () => "http://127.0.0.1:4737",
      mergeAssist: service,
    });
    try {
      const mcp = createRerailMcpHandler(stores, operations, { version: "0.0.0-test" });
      // Dismissing an applied finding has undo's effect, which is the person's.
      expect((await toolCall(mcp, "merge_assist_finding_set", { findingId: absorbed!.id, action: "dismiss" }, ws.repo)).result)
        .toEqual(expect.stringMatching(/^invalid-state: /u));
      expect(ws.rulesOn(ws.repo)).toEqual(steering);
      expect((await service.status(ws.repo)).findings[0]).toMatchObject({ id: absorbed!.id, status: "applied" });

      const app = createRerailApi(stores, operations, { version: "0.0.0-test" });
      const response = await app.request("/api/merge-assist/findings", {
        method: "PUT", headers: { "content-type": "application/json" },
        body: JSON.stringify({ checkout: ws.repo, findingId: absorbed!.id, action: "dismiss" }),
      });
      expect(response.status).toBe(200);
      expect(mergeAssistStatusSchema.parse(await response.json()).findings[0]).toMatchObject({ id: absorbed!.id, status: "dismissed" });
      expect(ws.rulesOn(ws.repo)).toEqual([]);
    } finally {
      stores.close();
    }
  });

  /** A conflict open in the workspace's checkout, and the API over a Merge Assist with the verdicts on or off. */
  async function openConflict(conflictVerdicts: boolean) {
    const ws = await workspace({ files: { "src/api/model.ts": "a\nb\nc\nd\ne\n" } });
    const service = ws.service({ conflictVerdicts });
    await service.setEnabled(ws.repo, true);
    commit(ws.worktree("jbarnes-model"), { "src/api/model.ts": "a\nB\nc\nd\ne\n" }, "Capitalise b");
    writeFiles(ws.repo, { "src/api/model.ts": "a\nb\nC\nd\ne\n" });
    const [conflict] = (await service.scan(ws.repo)).conflicts;
    expect(conflict).toMatchObject({ status: "open" });
    const stores = new DatabaseApiStores(ws.databasePath, {
      health: () => ({ pid: process.pid, database: { path: ws.databasePath, exists: true }, native: { ok: true } }),
      origin: () => "http://127.0.0.1:4737",
      mergeAssist: service,
    });
    const app = createRerailApi(stores, operations, { version: "0.0.0-test" });
    const resolve = async (action: string): Promise<Answer> => {
      const response = await app.request("/api/merge-assist/conflicts", {
        method: "PUT", headers: { "content-type": "application/json" },
        body: JSON.stringify({ checkout: ws.repo, conflictId: conflict!.id, action }),
      });
      return { status: response.status, body: await response.json() as Record<string, unknown> };
    };
    return { ws, service, conflict: conflict!, stores, resolve };
  }

  it("refuses a coding agent's verdict, and a person's while the verdicts are off, and takes the person's dismissal", async () => {
    const { ws, service, conflict, stores, resolve } = await openConflict(false);
    try {
      const mcp = createRerailMcpHandler(stores, operations, { version: "0.0.0-test" });
      expect(mergeAssistStatusSchema.parse((await toolCall(mcp, "merge_assist_get", {}, ws.repo)).result).conflicts).toEqual([conflict]);
      expect((await toolCall(mcp, "merge_assist_conflict_set", { conflictId: conflict.id, action: "mine" }, ws.repo)).result)
        .toEqual(expect.stringMatching(/^invalid-state: The person decides in-flight conflicts/u));

      for (const action of ["mine", "theirs"]) {
        expect(await resolve(action)).toEqual({ status: 409, body: { error: "invalid-state", message: expect.stringContaining("experimental: set RERAIL_EXPERIMENTAL_CONFLICT_VERDICTS=1") } });
      }
      expect((await service.status(ws.repo)).conflicts[0]).toMatchObject({ status: "open" });

      const dismissed = await resolve("dismiss");
      expect(dismissed.status).toBe(200);
      expect(mergeAssistStatusSchema.parse(dismissed.body).conflicts[0]).toMatchObject({ status: "dismissed", resolution: { action: "dismiss" } });
    } finally {
      stores.close();
    }
  });

  it("takes a person's verdict over HTTP while the verdicts are on", async () => {
    const { stores, resolve } = await openConflict(true);
    try {
      const resolved = await resolve("theirs");
      expect(resolved.status).toBe(200);
      expect(mergeAssistStatusSchema.parse(resolved.body).conflicts[0]).toMatchObject({ status: "resolved", resolution: { action: "theirs", text: "Capitalise b (src/api/model.ts:2)" } });
    } finally {
      stores.close();
    }
  });
});
