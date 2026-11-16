/**
 * A pruning pass that never ends: a checkout on a share whose server is down, where a stat never answers. The daemon's
 * passes must go on after it, so the pass gives up after its limit and the next one starts; that pass skips the checkout
 * the filesystem has not answered for, so a dead share holds one pool thread however many passes run.
 */
import { execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, realpathSync } from "node:fs";
import type * as FsPromises from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { startCheckoutPruning } from "../../src/daemon/checkout-pruning.js";
import { openRerailDatabase, recordCheckout } from "../../src/engine/index.js";
import { removeFolder } from "../support/platform/remove-folder.js";

const share = vi.hoisted(() => ({ root: "\0", waiting: 0, local: "\0", localStats: 0 }));

vi.mock("node:fs/promises", async importOriginal => {
  const actual = await importOriginal<typeof FsPromises>();
  return {
    ...actual,
    // A stat on the share never answers, as on a hard mount whose server is down.
    stat: (path: Parameters<typeof actual.stat>[0], ...rest: unknown[]) => {
      if (String(path).startsWith(share.local)) share.localStats += 1;
      if (!String(path).startsWith(share.root)) return (actual.stat as (...args: unknown[]) => Promise<unknown>)(path, ...rest);
      share.waiting += 1;
      return Promise.withResolvers<never>().promise;
    },
  };
});

const cleanups: Array<() => Promise<void> | void> = [];
afterEach(async () => {
  share.root = "\0";
  share.waiting = 0;
  share.local = "\0";
  share.localStats = 0;
  for (const cleanup of cleanups.splice(0).reverse()) await cleanup();
  vi.useRealTimers();
});

describe("the daemon's checkout pruning over a share that never answers", () => {
  it("gives up on a pass past its limit, says so once, and runs the next pass without asking the share again", async () => {
    const home = realpathSync(mkdtempSync(join(tmpdir(), "rerail-prune-limit-")));
    cleanups.push(() => removeFolder(home));
    const away = join(home, "share", "away");
    mkdirSync(away, { recursive: true });
    execFileSync("git", ["init", "-q", "-b", "main", away], { stdio: "ignore" });
    const databasePath = join(home, "rerail.sqlite");
    const database = openRerailDatabase(databasePath);
    cleanups.push(() => { database.$client.close(); });
    recordCheckout(database, away);
    const local = join(home, "local");
    mkdirSync(local, { recursive: true });
    execFileSync("git", ["init", "-q", "-b", "main", local], { stdio: "ignore" });
    recordCheckout(database, local);
    share.root = join(home, "share");
    share.local = local;
    const lines: string[] = [];
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "setInterval", "clearInterval"] });

    const pruning = startCheckoutPruning(databasePath, { intervalMs: 1_000, passLimitMs: 300, log: line => lines.push(line) });
    cleanups.push(() => pruning.stop());
    // The pass reaches the share in promise jobs alone; advancing by nothing runs them.
    await vi.advanceTimersByTimeAsync(0);
    expect(share.waiting).toBe(1);
    await vi.advanceTimersByTimeAsync(299);
    expect(lines).toEqual([]);

    await vi.advanceTimersByTimeAsync(1);
    expect(lines).toHaveLength(1);
    expect(lines[0]).toMatch(/^rerail daemon: forgetting removed checkouts: gave up on a pass .* the next pass skips the checkout it waited on\n$/u);
    await vi.advanceTimersByTimeAsync(699);
    expect(share.waiting).toBe(1);

    // The hourly tick (here every second) starts the next pass: it looks at the local checkout again and asks the share
    // nothing, so the stat still waiting there is the only pool thread the share holds.
    const localStats = share.localStats;
    await vi.advanceTimersByTimeAsync(1);
    await vi.waitFor(() => expect(share.localStats).toBeGreaterThan(localStats));
    expect(share.waiting).toBe(1);
    expect(lines).toHaveLength(1);
  });
});
