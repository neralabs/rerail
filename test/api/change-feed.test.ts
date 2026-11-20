import { EventEmitter } from "node:events";
import type { FSWatcher, watch as watchFolder } from "node:fs";
import { appendFileSync, mkdtempSync, rmSync, statSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { databaseChangeFeed, LOOK_AGAIN_MS, type ChangeNotice } from "../../src/api/change-feed.js";
import { removeFolder } from "../support/platform/remove-folder.js";

/**
 * Puts a link that leads to itself in the place of the file at `path`, so `stat` of it fails (ELOOP) for every account
 * on every system: on Windows a junction, which needs no privilege. Rights taken away would not bind every account:
 * root reads past a folder's mode, and on Windows a token whose backup privilege is on (an elevated administrator's in
 * Git Bash, as GitHub's Windows runner runs the tests) reads past a deny rule.
 */
function loopInPlace(path: string): void {
  rmSync(path);
  symlinkSync(path, path, process.platform === "win32" ? "junction" : null);
}

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

/** A database version that has moved at every look, as one a commit moved each time would. */
function moving(): () => string {
  let version = 0;
  return () => {
    version += 1;
    return String(version);
  };
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
    const feed = databaseChangeFeed(DATABASE, moving(), { watch: folder.watch });
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
    const feed = databaseChangeFeed(DATABASE, moving(), { watch: folder.watch });
    const heard: ChangeNotice[] = [];
    feed.listen(notice => heard.push(notice));

    folder.report(null);

    expect(await heardAfterTurn(heard)).toEqual(["data"]);
  });

  it("watches and beats only while someone listens, and leaves no timer behind once nobody does", () => {
    const folder = fakeWatch();
    const feed = databaseChangeFeed(DATABASE, moving(), { watch: folder.watch, beatMs: 5_000 });
    expect([folder.watched(), vi.getTimerCount()]).toEqual([0, 0]);

    const stop = feed.listen(() => {});
    expect([folder.watched(), vi.getTimerCount()]).toEqual([1, 1]);

    stop();
    expect([folder.watched(), vi.getTimerCount()]).toEqual([0, 0]);
  });

  it("beats while someone listens, with nothing else to tell", async () => {
    const feed = databaseChangeFeed(DATABASE, moving(), { watch: fakeWatch().watch, beatMs: 5_000 });
    const heard: ChangeNotice[] = [];
    feed.listen(notice => heard.push(notice));

    await vi.advanceTimersByTimeAsync(10_000);

    // The second beat's turn may end just past the time moved to: the next timer is either that turn or a third beat.
    expect(await heardAfterTurn(heard)).toEqual(["beat", "beat"]);
  });

  it("tries a watch that could not start again at the next beat, and tells commits once it runs", async () => {
    const folder = fakeWatch();
    folder.fail(true);
    const feed = databaseChangeFeed(DATABASE, moving(), { watch: folder.watch, beatMs: 5_000 });
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
    const feed = databaseChangeFeed(DATABASE, moving(), { watch: fakeWatch().watch });
    const heard: ChangeNotice[] = [];
    feed.listen(notice => heard.push(notice));

    feed.notify("conflicts");
    expect(await heardAfterTurn(heard)).toEqual(["conflicts"]);

    feed.close();
    feed.notify("data");
    expect(vi.getTimerCount()).toBe(0);
    expect(heard).toEqual(["conflicts"]);
  });

  it("tells another process's commit once readers can see it, looking again after the notice that came before it", async () => {
    // The writer's -wal write is heard before its sync ends; its commit shows to readers 7 ms later.
    const folder = fakeWatch();
    let version = "1";
    const feed = databaseChangeFeed(DATABASE, () => version, { watch: folder.watch });
    const heard: ChangeNotice[] = [];
    feed.listen(notice => heard.push(notice));

    folder.report("rerail.sqlite-wal");
    expect(await heardAfterTurn(heard)).toEqual([]);
    setTimeout(() => { version = "2"; }, 7);
    await vi.advanceTimersByTimeAsync(LOOK_AGAIN_MS.at(-1)!);

    expect(heard).toEqual(["data"]);
    // The looks ended once it showed: only the beat is left.
    expect(vi.getTimerCount()).toBe(1);
  });

  it("stops looking again past its last step, leaving the beat to read it", async () => {
    const folder = fakeWatch();
    const feed = databaseChangeFeed(DATABASE, () => "1", { watch: folder.watch, beatMs: 60_000 });
    const heard: ChangeNotice[] = [];
    feed.listen(notice => heard.push(notice));

    folder.report("rerail.sqlite-wal");
    await vi.advanceTimersByTimeAsync(LOOK_AGAIN_MS.at(-1)!);

    expect(heard).toEqual([]);
    expect(vi.getTimerCount()).toBe(1);
  });

  it("tells a notice whose commit already shows at once, as this process's commits always do, and looks no more", async () => {
    const folder = fakeWatch();
    let version = "1";
    const feed = databaseChangeFeed(DATABASE, () => version, { watch: folder.watch });
    const heard: ChangeNotice[] = [];
    feed.listen(notice => heard.push(notice));

    version = "2";
    folder.report("rerail.sqlite-wal");

    expect(await heardAfterTurn(heard)).toEqual(["data"]);
    expect(vi.getTimerCount()).toBe(1);
  });

  it("looks no more after a notice of writes already told, as its own commit's notice heard after it was told, and looks after a new write", async () => {
    const folder = fakeWatch();
    const home = mkdtempSync(join(tmpdir(), "rerail-feed-"));
    try {
      const database = join(home, "rerail.sqlite");
      writeFileSync(`${database}-wal`, "commit 1");
      let version = "1";
      let reads = 0;
      const feed = databaseChangeFeed(database, () => { reads += 1; return version; }, { watch: folder.watch, beatMs: 60_000 });
      const heard: ChangeNotice[] = [];
      feed.listen(notice => heard.push(notice));
      version = "2";
      feed.notify("data");
      expect(await heardAfterTurn(heard)).toEqual(["data"]);

      folder.report("rerail.sqlite-wal");
      await vi.advanceTimersByTimeAsync(LOOK_AGAIN_MS.at(-1)!);
      const afterEcho = reads;
      expect(vi.getTimerCount()).toBe(1);

      appendFileSync(`${database}-wal`, ", commit 2 in flight");
      folder.report("rerail.sqlite-wal");
      await vi.advanceTimersByTimeAsync(LOOK_AGAIN_MS.at(-1)!);

      expect([heard, afterEcho, reads - afterEcho]).toEqual([["data"], 3, LOOK_AGAIN_MS.length + 1]);
    } finally {
      await removeFolder(home);
    }
  });

  it("looks again, and throws nowhere, when the database's files cannot be read: a link that leads to itself in the -wal file's place", async () => {
    const folder = fakeWatch();
    const home = mkdtempSync(join(tmpdir(), "rerail-feed-"));
    const database = join(home, "rerail.sqlite");
    writeFileSync(database, "");
    writeFileSync(`${database}-wal`, "commit 1");
    let version = "1";
    const feed = databaseChangeFeed(database, () => version, { watch: folder.watch, beatMs: 60_000 });
    const heard: ChangeNotice[] = [];
    feed.listen(notice => heard.push(notice));
    loopInPlace(`${database}-wal`);
    try {
      expect(() => statSync(`${database}-wal`)).toThrow(/ELOOP/u);
      folder.report("rerail.sqlite-wal");
      await vi.advanceTimersByTimeAsync(LOOK_AGAIN_MS[2]);
      version = "2";
      await vi.advanceTimersByTimeAsync(LOOK_AGAIN_MS.at(-1)!);

      expect(heard).toEqual(["data"]);
    } finally {
      feed.close();
      await removeFolder(home);
    }
  });
});
