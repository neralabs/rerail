import { execFileSync, spawnSync } from "node:child_process";
import { mkdirSync, realpathSync } from "node:fs";
import { mkdtemp, rename } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { createRerailApi } from "../../src/api/http.js";
import { operations } from "../../src/api/operations.js";
import { DatabaseApiStores } from "../../src/api/stores.js";
import { recordCheckout } from "../../src/engine/index.js";
import { withExperiments } from "../support/environment/experiments.js";
import { git } from "../support/merge-assist/git.js";
import { RecordingMergeAssistService } from "../support/merge-assist/status.js";
import { removeFolder } from "../support/platform/remove-folder.js";

// Every process Rerail starts goes through these; one started with spawnSync or execFileSync holds the thread until it
// exits, and a daemon answers no hook meanwhile.
vi.mock("node:child_process", { spy: true });

// Merge Assist is experimental: every test here runs with it on, as a daemon started with the variable does.
withExperiments("MERGE_ASSIST");

const roots: string[] = [];
afterEach(async () => {
  for (const root of roots.splice(0)) await removeFolder(root);
});

describe("the Merge Assist operations for a repository", () => {
  it("find its main checkout with no process that holds the thread, as the dashboard's page reads it with no checkout in scope", async () => {
    const root = realpathSync(await mkdtemp(join(tmpdir(), "rerail-merge-assist-thread-")));
    roots.push(root);
    const checkout = join(root, "app");
    mkdirSync(checkout);
    git(checkout, "init", "-q", "-b", "main");
    git(checkout, "remote", "add", "origin", "git@github.com:acme/app.git");
    git(checkout, "commit", "-q", "--allow-empty", "-m", "start");
    // A linked worktree whose path sorts first: the main checkout is the second one asked about.
    const worktree = join(root, "a-export");
    git(checkout, "worktree", "add", "-q", "-b", "akim/export-api", worktree);
    const databasePath = join(root, "rerail.sqlite");
    const service = new RecordingMergeAssistService();
    const stores = new DatabaseApiStores(databasePath, {
      health: () => ({ pid: process.pid, database: { path: databasePath, exists: true }, native: { ok: true } }),
      origin: () => "http://127.0.0.1:4737",
      mergeAssist: service,
    });
    try {
      const { repositoryKey } = recordCheckout(stores.database(), worktree);
      recordCheckout(stores.database(), checkout);
      const app = createRerailApi(stores, operations, { version: "0.0.0-test" });
      const starters = [vi.mocked(spawnSync), vi.mocked(execFileSync)];
      for (const starter of starters) starter.mockClear();

      const response = await app.request(`/api/merge-assist?${new URLSearchParams({ repository: repositoryKey })}`);

      expect(response.status).toBe(200);
      expect(service.calls).toEqual([{ method: "status", checkout }]);
      expect(starters.flatMap(starter => starter.mock.calls.map(([command, args]) => [command, ...(Array.isArray(args) ? args : [])].join(" ")))).toEqual([]);
    } finally {
      stores.close();
    }
  });

  it("find its main checkout moved and recorded again at its new path, not the worktree Git refuses whose path sorts first", async () => {
    const root = realpathSync(await mkdtemp(join(tmpdir(), "rerail-merge-assist-thread-")));
    roots.push(root);
    const checkout = join(root, "app");
    mkdirSync(checkout);
    git(checkout, "init", "-q", "-b", "main");
    git(checkout, "commit", "-q", "--allow-empty", "-m", "start");
    const worktree = join(root, "a-export");
    git(checkout, "worktree", "add", "-q", "-b", "akim/export-api", worktree);
    const databasePath = join(root, "rerail.sqlite");
    const service = new RecordingMergeAssistService();
    const stores = new DatabaseApiStores(databasePath, {
      health: () => ({ pid: process.pid, database: { path: databasePath, exists: true }, native: { ok: true } }),
      origin: () => "http://127.0.0.1:4737",
      mergeAssist: service,
    });
    try {
      const { repositoryKey } = recordCheckout(stores.database(), worktree);
      recordCheckout(stores.database(), checkout);
      // The worktree's Git folder moves with the main checkout, so Git refuses it; a session there records it again.
      const moved = join(root, "app-moved");
      await rename(checkout, moved);
      recordCheckout(stores.database(), moved);
      expect(recordCheckout(stores.database(), worktree).repositoryKey).toBe(repositoryKey);
      const app = createRerailApi(stores, operations, { version: "0.0.0-test" });

      const response = await app.request(`/api/merge-assist?${new URLSearchParams({ repository: repositoryKey })}`);

      expect(response.status).toBe(200);
      expect(service.calls).toEqual([{ method: "status", checkout: moved }]);
    } finally {
      stores.close();
    }
  });
});
