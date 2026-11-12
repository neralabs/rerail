import { execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, realpathSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { RerailDaemon } from "../../src/daemon/index.js";
import { listRepositories, openRerailDatabase, recordCheckout } from "../../src/engine/index.js";

const cleanups: Array<() => Promise<void> | void> = [];
afterEach(async () => {
  for (const cleanup of cleanups.splice(0).reverse()) await cleanup();
});

function git(cwd: string, ...args: string[]): string {
  return execFileSync("git", args, { cwd, encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] });
}

function gitRepository(path: string): string {
  mkdirSync(path, { recursive: true });
  git(path, "init", "-q", "-b", "main");
  git(path, "-c", "user.name=Rerail", "-c", "user.email=rerail@example.invalid", "commit", "-q", "--allow-empty", "-m", "base");
  return path;
}

async function until(condition: () => boolean, timeoutMs = 10_000): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  while (!condition() && Date.now() < deadline) await new Promise(resolve => setTimeout(resolve, 50));
}

describe("the daemon's checkout pruning", () => {
  it("forgets a checkout whose worktree is removed, before and while it runs, and keeps the others", async () => {
    const home = realpathSync(mkdtempSync(join(tmpdir(), "rerail-prune-daemon-")));
    cleanups.push(() => rmSync(home, { recursive: true, force: true }));
    const databasePath = join(home, "rerail.sqlite");
    const main = gitRepository(join(home, "main"));
    git(main, "worktree", "add", "-q", "-b", "before", join(home, "before"));
    git(main, "worktree", "add", "-q", "-b", "after", join(home, "after"));
    const database = openRerailDatabase(databasePath);
    cleanups.push(() => { database.$client.close(); });
    const [kept, before, after] = [main, join(home, "before"), join(home, "after")].map(path => recordCheckout(database, path).checkoutPath);
    const recorded = (): string[] => listRepositories(database).flatMap(repository => repository.checkouts.map(row => row.checkoutPath));
    git(main, "worktree", "remove", before!);

    const daemon = await RerailDaemon.start({
      version: "test", databasePath, workspaceRoot: home, port: 0, remoteSink: false, checkoutPruning: { intervalMs: 100 },
    });
    cleanups.push(() => daemon.close());
    await until(() => !recorded().includes(before!));
    expect(recorded().sort()).toEqual([after!, kept!].sort());

    git(main, "worktree", "remove", after!);
    await until(() => !recorded().includes(after!));
    expect(recorded()).toEqual([kept]);
  }, 30_000);
});
