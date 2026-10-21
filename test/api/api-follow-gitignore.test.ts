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
import { CALLER_CWD_HEADER, recordCheckout, setUpRepository } from "../../src/engine/index.js";

describe("a repository's Follow .gitignore setting", () => {
  let root: string;
  let checkout: string;
  let databasePath: string;
  let repositoryKey: string;
  let stores: DatabaseApiStores;
  let app: Pick<OpenAPIHono, "request">;

  const openStores = (): DatabaseApiStores => new DatabaseApiStores(databasePath, {
    health: () => ({ pid: process.pid, database: { path: databasePath, exists: true }, native: { ok: true } }),
    origin: () => "http://127.0.0.1:4737",
  });

  beforeEach(async () => {
    root = realpathSync(await mkdtemp(join(tmpdir(), "rerail-follow-gitignore-")));
    checkout = join(root, "app");
    mkdirSync(checkout, { recursive: true });
    execFileSync("git", ["init", "-q", "-b", "main"], { cwd: checkout, stdio: "ignore" });
    execFileSync("git", ["remote", "add", "origin", "git@github.com:acme/app.git"], { cwd: checkout, stdio: "ignore" });
    databasePath = join(root, "rerail.sqlite");
    stores = openStores();
    repositoryKey = recordCheckout(stores.database(), checkout).repositoryKey;
    app = createRerailApi(stores, operations, { version: "0.0.0-test" });
  });

  afterEach(async () => {
    stores.close();
    await rm(root, { recursive: true, force: true });
  });

  const context = async (): Promise<{ repository: { followGitignore: boolean } }> =>
    await (await app.request(`/api/repository?${new URLSearchParams({ repository: repositoryKey })}`)).json() as { repository: { followGitignore: boolean } };
  const set = async (body: Record<string, unknown>): Promise<{ status: number; body: Record<string, unknown> }> => {
    const response = await app.request("/api/repository/follow-gitignore", {
      method: "PUT", headers: { "content-type": "application/json" }, body: JSON.stringify(body),
    });
    return { status: response.status, body: await response.json() as Record<string, unknown> };
  };

  it("is on for a repository Rerail just recorded, and a change persists in the repository summaries", async () => {
    expect(await context()).toMatchObject({ repository: { repositoryKey, followGitignore: true } });

    expect(await set({ repository: repositoryKey, followGitignore: false }))
      .toMatchObject({ status: 200, body: { repository: { repositoryKey, followGitignore: false } } });
    stores.close();
    stores = openStores();
    app = createRerailApi(stores, operations, { version: "0.0.0-test" });
    expect(await context()).toMatchObject({ repository: { followGitignore: false } });
    const listed = await (await app.request("/api/repositories")).json() as { repositories: Array<{ repositoryKey: string; followGitignore: boolean }> };
    expect(listed.repositories).toEqual([expect.objectContaining({ repositoryKey, followGitignore: false })]);

    // A checkout names its repository; the setting is the repository's, for every checkout.
    expect(await set({ checkout, followGitignore: true })).toMatchObject({ status: 200, body: { repository: { followGitignore: true } } });
    expect(await context()).toMatchObject({ repository: { followGitignore: true } });
  });

  it("refuses a body without a boolean and a repository Rerail does not know", async () => {
    expect((await set({ repository: repositoryKey, followGitignore: "no" })).status).toBe(400);
    expect((await set({ repository: repositoryKey })).status).toBe(400);
    expect(await set({ repository: "remote:example.com/unknown", followGitignore: false })).toMatchObject({ status: 404 });
    expect(await context()).toMatchObject({ repository: { followGitignore: true } });
  });

  it("lets a coding agent change it only before the repository is set up", async () => {
    const mcp = createRerailMcpHandler(stores, operations, { version: "0.0.0-test" });
    const call = async (followGitignore: boolean): Promise<{ isError?: boolean; content: Array<{ text: string }> }> => {
      const response = await mcp(new Request("http://localhost:4737/mcp", {
        method: "POST",
        headers: {
          host: "localhost:4737", "content-type": "application/json", accept: "application/json, text/event-stream",
          [CALLER_CWD_HEADER]: encodeURIComponent(checkout),
        },
        body: JSON.stringify({ jsonrpc: "2.0", id: 1, method: "tools/call", params: { name: "repository_follow_gitignore_set", arguments: { followGitignore } } }),
      }));
      return (await response.json() as { result: { isError?: boolean; content: Array<{ text: string }> } }).result;
    };
    expect((await call(false)).isError).not.toBe(true);
    expect(await context()).toMatchObject({ repository: { followGitignore: false } });

    setUpRepository(stores.database(), repositoryKey, { packs: [] });
    const refused = await call(true);
    expect(refused).toMatchObject({ isError: true, content: [{ text: expect.stringMatching(/^agent-not-permitted/u) }] });
    expect(await context()).toMatchObject({ repository: { followGitignore: false } });
  });
});
