import { EventEmitter } from "node:events";
import type { FSWatcher, watch as watchFolder } from "node:fs";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { databaseChangeFeed, type ChangeNotice } from "../../src/api/change-feed.js";

const DATABASE = join("/rerail-home", "rerail.sqlite");

/** A folder watch the test drives: `report(name)` is the OS saying `name` in the folder changed. */
function fakeWatch(): { watch: typeof watchFolder; report: (name: string | null) => void; watched: () => number; fail: (on: boolean) => void } {
  let listener: ((event: string, name: string | null) => void) | undefined;
  let open = 0;
  let failing = false;
  const watch = ((_folder: string, _options: unknown, onChange: (event: string, name: string | null) => void) => {
    if (failing) throw Object.assign(new Error("ENOSPC: no watches left"), { code: "ENOSPC" });
    listener = onChange;
    open += 1;
    const watcher = new EventEmitter() as FSWatcher;
    watcher.close = () => { open -= 1; listener = undefined; };
    return watcher;
  }) as unknown as typeof watchFolder;
  return { watch, report: name => listener?.("change", name), watched: () => open, fail: on => { failing = on; } };
}

/** The notices heard so far, once the turn that reported them has ended. */
async function heardAfterTurn(heard: ChangeNotice[]): Promise<ChangeNotice[]> {
  await vi.advanceTimersToNextTimerAsync();
  return heard;
}

describe("the database's change feed", () => {
  beforeEach(() => {
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "setInterval", "clearInterval", "setImmediate"] });
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("tells a commit to the database's -wal file once per turn, and nothing for another file in its folder", async () => {
    const folder = fakeWatch();
    const feed = databaseChangeFeed(DATABASE, { watch: folder.watch });
    const heard: ChangeNotice[] = [];
    feed.listen(notice => heard.push(notice));

    folder.report("daemon.log");
    folder.report("rerail.sqlite-wal");
    folder.report("rerail.sqlite-wal");
    folder.report("rerail.sqlite-shm");

    expect(await heardAfterTurn(heard)).toEqual(["data"]);
  });

  it("takes a notice that names no file as the database's, since some systems name none", async () => {
    const folder = fakeWatch();
    const feed = databaseChangeFeed(DATABASE, { watch: folder.watch });
    const heard: ChangeNotice[] = [];
    feed.listen(notice => heard.push(notice));

    folder.report(null);

    expect(await heardAfterTurn(heard)).toEqual(["data"]);
  });

  it("watches and beats only while someone listens, and leaves no timer behind once nobody does", () => {
    const folder = fakeWatch();
    const feed = databaseChangeFeed(DATABASE, { watch: folder.watch, beatMs: 5_000 });
    expect([folder.watched(), vi.getTimerCount()]).toEqual([0, 0]);

    const stop = feed.listen(() => {});
    expect([folder.watched(), vi.getTimerCount()]).toEqual([1, 1]);

    stop();
    expect([folder.watched(), vi.getTimerCount()]).toEqual([0, 0]);
  });

  it("beats while someone listens, with nothing else to tell", async () => {
    const feed = databaseChangeFeed(DATABASE, { watch: fakeWatch().watch, beatMs: 5_000 });
    const heard: ChangeNotice[] = [];
    feed.listen(notice => heard.push(notice));

    await vi.advanceTimersByTimeAsync(10_000);

    expect(heard).toEqual(["beat", "beat"]);
  });

  it("tries a watch that could not start again at the next beat, and tells commits once it runs", async () => {
    const folder = fakeWatch();
    folder.fail(true);
    const feed = databaseChangeFeed(DATABASE, { watch: folder.watch, beatMs: 5_000 });
    const heard: ChangeNotice[] = [];
    feed.listen(notice => heard.push(notice));
    expect(folder.watched()).toBe(0);

    folder.fail(false);
    await vi.advanceTimersByTimeAsync(5_000);
    expect(folder.watched()).toBe(1);
    folder.report("rerail.sqlite-wal");

    expect(await heardAfterTurn(heard)).toEqual(["beat", "data"]);
  });

  it("tells what this process says changed, and nothing once closed", async () => {
    const feed = databaseChangeFeed(DATABASE, { watch: fakeWatch().watch });
    const heard: ChangeNotice[] = [];
    feed.listen(notice => heard.push(notice));

    feed.notify("conflicts");
    expect(await heardAfterTurn(heard)).toEqual(["conflicts"]);

    feed.close();
    feed.notify("data");
    expect(vi.getTimerCount()).toBe(0);
    expect(heard).toEqual(["conflicts"]);
  });
});
