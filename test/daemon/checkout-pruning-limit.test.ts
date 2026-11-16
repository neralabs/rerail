/**
 * A pruning pass that never ends: checkouts on a share whose server is down, where a stat never answers. The daemon's
 * passes must go on after it, so the pass gives up after its limit and the next one starts; while the stat it gave up on
 * is unanswered, the later passes look at nothing, so a dead share holds one pool thread however many of its checkouts
 * are recorded and however many passes run.
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

const share = vi.hoisted(() => ({ root: "\0", waiting: 0, local: "\0", localStats: 0, answer: [] as Array<() => void> }));

vi.mock("node:fs/promises", async importOriginal => {
  const actual = await importOriginal<typeof FsPromises>();
  return {
    ...actual,
    // A stat on the share never answers, as on a hard mount whose server is down, until the test answers it.
    stat: (path: Parameters<typeof actual.stat>[0], ...rest: unknown[]) => {
      if (String(path).startsWith(share.local)) share.localStats += 1;
      if (!String(path).startsWith(share.root)) return (actual.stat as (...args: unknown[]) => Promise<unknown>)(path, ...rest);
      share.waiting += 1;
      const { promise, reject } = Promise.withResolvers<never>();
      share.answer.push(() => reject(Object.assign(new Error("share timed out"), { code: "ETIMEDOUT" })));
      return promise;
    },
  };
});

const cleanups: Array<() => Promise<void> | void> = [];
afterEach(async () => {
  share.root = "\0";
  share.waiting = 0;
  share.local = "\0";
  share.localStats = 0;
  share.answer = [];
  for (const cleanup of cleanups.splice(0).reverse()) await cleanup();
  vi.useRealTimers();
});

describe("the daemon's checkout pruning over a share that never answers", () => {
  it("gives up on a pass past its limit, says so once, and asks nothing more until the share answers", async () => {
    const home = realpathSync(mkdtempSync(join(tmpdir(), "rerail-prune-limit-")));
    cleanups.push(() => removeFolder(home));
    const databasePath = join(home, "rerail.sqlite");
    const database = openRerailDatabase(databasePath);
    cleanups.push(() => { database.$client.close(); });
    // Several clones on the share, each a checkout a pass would look at in turn.
    for (const name of ["one", "two", "three"]) {
      const away = join(home, "share", name);
      mkdirSync(away, { recursive: true });
      execFileSync("git", ["init", "-q", "-b", "main", away], { stdio: "ignore" });
      recordCheckout(database, away);
    }
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
    expect(lines[0]).toMatch(/^rerail daemon: forgetting removed checkouts: gave up on a pass .* later passes look at nothing until it answers\n$/u);

    // Five more hourly ticks (here every second): each pass looks at nothing while the share's stat is unanswered, so
    // that stat is the only pool thread the share holds, and none of them gives up on anything.
    const localStats = share.localStats;
    await vi.advanceTimersByTimeAsync(5_000);
    expect(share.waiting).toBe(1);
    expect(share.localStats).toBe(localStats);
    expect(lines).toHaveLength(1);

    // Once the share answers, the next pass looks at every checkout again.
    share.root = "\0";
    for (const answer of share.answer.splice(0)) answer();
    await vi.advanceTimersByTimeAsync(1_000);
    await vi.waitFor(() => expect(share.localStats).toBeGreaterThan(localStats));
    expect(share.waiting).toBe(1);
  });
});
