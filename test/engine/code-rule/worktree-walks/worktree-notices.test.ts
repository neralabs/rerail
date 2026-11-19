import { mkdir, mkdtemp, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import type { TreeNotice, TreeWatch } from "../../../../src/engine/code-rule/tree-watch.js";
import {
  folderAt, NOTICE_PATHS_CAP, noticeCounts, REWATCH_MS, SharedTreeWatch, WATCH_RETRY_MS, WorktreeNotices,
} from "../../../../src/engine/code-rule/worktree-notices.js";
import { removeFolder } from "../../../support/platform/remove-folder.js";

/** A read that walked the worktree, `src` and `src/deep`, and left out an ignored folder, an ignored file and a worktree. */
const seen = { folders: new Set(["", "src", "src/deep"]), leftOut: new Set(["node_modules", "src/cache.tmp", "nested"]) };

describe("noticeCounts", () => {
  it.each<["change" | "rename", string, boolean, string]>([
    ["change", "src/a.py", true, "a file the read observed changed"],
    ["rename", "src/new.py", true, "a file came or went in a folder the read walked"],
    ["rename", "src/fresh", true, "a folder came"],
    ["change", "src/fresh/n.py", true, "a file changed in a folder made since the read"],
    ["change", "PKG_0~1/SUB_3~1/M0.PY", true, "a short name the read never saw"],
    ["rename", ".git", true, "a checkout came or went"],
    ["rename", "src/deep/.git", true, "a checkout came or went inside"],
    ["rename", "node_modules", true, "what the read left out came or went"],
    ["change", "src", false, "the times of a folder the read walked, whose entries' changes come on their own"],
    ["change", ".git", false, "the times of a .git folder"],
    ["change", ".git/index", false, "Git's own files"],
    ["rename", ".git/index.lock", false, "Git's own files"],
    ["change", "node_modules", false, "the times of an ignored folder"],
    ["rename", "node_modules/pkg/index.js", false, "a file in an ignored folder"],
    ["change", "src/cache.tmp", false, "an ignored file"],
    ["change", "nested/src/a.py", false, "a file in another worktree inside"],
  ])("%s of %s counts: %s (%s)", (event, path, counts) => {
    expect(noticeCounts(seen, event, path)).toBe(counts);
  });
});

/** A `TreeWatch` a test feeds notices to, recording what it was asked to watch. */
class ScriptedWatch implements TreeWatch {
  readonly calls: string[] = [];
  readonly queued: TreeNotice[] = [];
  /** Roots it refuses to watch. */
  readonly refused = new Set<string>();
  /** Folders past the watch budget: it refuses a watch that follows any of them. */
  readonly overBudget = new Set<string>();
  closed = false;

  watch(root: string, folders: readonly string[], spare?: boolean): boolean {
    this.calls.push(`watch ${root} [${folders.join(",")}]${spare === true ? " spare" : ""}`);
    if (!this.refused.has(root) && !folders.some(folder => this.overBudget.has(folder))) return true;
    // As the engine does, a refused root's queued notices go with it.
    this.queued.splice(0, this.queued.length, ...this.queued.filter(notice => notice.root !== root));
    return false;
  }

  unwatch(root: string): void {
    this.calls.push(`unwatch ${root}`);
  }

  take(): TreeNotice[] {
    return this.queued.splice(0);
  }

  close(): void {
    this.closed = true;
  }

  notice(root: string, notice: Partial<Omit<TreeNotice, "root">>): void {
    this.queued.push({ root, changed: [], renamed: [], lost: false, ended: false, ...notice });
  }
}

/** `WorktreeNotices` on `platform` over `watch`, at a clock the test moves, where every worktree has the same folder. */
function notices(watch: TreeWatch | undefined, platform: NodeJS.Platform = "darwin"): { notices: WorktreeNotices; clock: { now: number } } {
  const clock = { now: 0 };
  return { notices: new WorktreeNotices({ treeWatch: watch, platform, now: () => clock.now, folderAt: () => "folder" }), clock };
}

/** Reads `root` as a walk does: it was changed, the read begins and ends having seen `read`. */
function read(subject: WorktreeNotices, root: string, read = seen): void {
  subject.changed(root);
  subject.reading(root);
  subject.read(root, read);
}

describe("WorktreeNotices", () => {
  it("counts every worktree changed on every walk with no engine watch", () => {
    const { notices: subject } = notices(undefined);
    read(subject, "/w");
    expect([subject.changed("/w"), subject.watching("/w")]).toEqual([true, false]);
  });

  it("watches a worktree before its first read, and holds it changed until a notice counts", () => {
    const watch = new ScriptedWatch();
    const { notices: subject } = notices(watch);
    expect(subject.changed("/w")).toBe(true);
    expect(watch.calls).toEqual(["watch /w []"]);
    subject.reading("/w");
    subject.read("/w", seen);
    expect(subject.changed("/w")).toBe(false);
    watch.notice("/w", { changed: ["node_modules/x.js", "src"] });
    subject.collect();
    expect(subject.changed("/w")).toBe(false);
    watch.notice("/w", { changed: ["src/a.py"] });
    subject.collect();
    expect(subject.changed("/w")).toBe(true);
  });

  it("counts a notice that arrives while a read runs, whatever it names", () => {
    const watch = new ScriptedWatch();
    const { notices: subject } = notices(watch);
    subject.changed("/w");
    subject.reading("/w");
    watch.notice("/w", { changed: ["node_modules/x.js"] });
    subject.read("/w", seen);
    expect(subject.changed("/w")).toBe(true);
  });

  it("counts a worktree changed when the system lost its notices, and watches it again a while after its watch ended", () => {
    const watch = new ScriptedWatch();
    const { notices: subject, clock } = notices(watch);
    read(subject, "/w");
    watch.notice("/w", { lost: true });
    subject.collect();
    expect(subject.changed("/w")).toBe(true);
    read(subject, "/w");
    watch.notice("/w", { ended: true });
    subject.collect();
    expect(subject.watching("/w")).toBe(false);
    // Read whole at once, watched again only once `REWATCH_MS` passed.
    expect(subject.changed("/w")).toBe(true);
    expect(watch.calls.filter(call => call.startsWith("watch"))).toEqual(["watch /w []"]);
    clock.now = REWATCH_MS;
    expect(subject.changed("/w")).toBe(true);
    expect(watch.calls.filter(call => call.startsWith("watch"))).toEqual(["watch /w []", "watch /w []"]);
  });

  it("reads a worktree it cannot watch on every walk, and tries to watch it again after the retry", () => {
    const watch = new ScriptedWatch();
    watch.refused.add("/w");
    const { notices: subject, clock } = notices(watch);
    read(subject, "/w");
    expect(subject.changed("/w")).toBe(true);
    clock.now = WATCH_RETRY_MS - 1;
    expect(subject.changed("/w")).toBe(true);
    watch.refused.delete("/w");
    clock.now = WATCH_RETRY_MS;
    read(subject, "/w");
    expect(subject.changed("/w")).toBe(false);
    expect(watch.calls).toEqual(["watch /w []", "watch /w []"]);
  });

  it("follows the folders each read walked into on Linux, and reads again after one is watched for the first time", () => {
    const watch = new ScriptedWatch();
    const { notices: subject } = notices(watch, "linux");
    read(subject, "/w");
    expect(watch.calls).toEqual(["watch /w []", "watch /w [,src,src/deep]"]);
    // `src/deep` was not watched while the first read ran.
    expect(subject.changed("/w")).toBe(true);
    read(subject, "/w");
    expect(subject.changed("/w")).toBe(false);
    read(subject, "/w", { folders: new Set(["", "src"]), leftOut: seen.leftOut });
    expect(watch.calls.at(-1)).toBe("watch /w [,src]");
    expect(subject.changed("/w")).toBe(false);
  });

  it("watches again on Linux a folder that went and came back at the same path, and reads the worktree again", () => {
    const watch = new ScriptedWatch();
    const { notices: subject } = notices(watch, "linux");
    const walked = { folders: new Set(["", "a"]), leftOut: new Set<string>() };
    read(subject, "/w", walked);
    read(subject, "/w", walked);
    expect(subject.changed("/w")).toBe(false);
    // `rm -rf a && mkdir a`: the watch of the old `a` tells of nothing in the new one.
    watch.notice("/w", { renamed: ["a"] });
    subject.collect();
    read(subject, "/w", walked);
    expect(watch.calls.filter(call => call.startsWith("watch"))).toEqual(["watch /w []", "watch /w [,a]", "watch /w [,a]"]);
    expect(subject.changed("/w")).toBe(true);
  });

  it("watches every folder again on Linux after a notice that lost its paths, which may have hidden a folder put back", () => {
    const watch = new ScriptedWatch();
    const { notices: subject } = notices(watch, "linux");
    const walked = { folders: new Set(["", "a"]), leftOut: new Set<string>() };
    read(subject, "/w", walked);
    read(subject, "/w", walked);
    // A burst past the engine's cap: which folders went and came back is no longer known.
    watch.notice("/w", { lost: true });
    subject.collect();
    read(subject, "/w", walked);
    expect(watch.calls.filter(call => call.startsWith("watch"))).toEqual(["watch /w []", "watch /w [,a]", "watch /w [,a]"]);
  });

  it("hands on each worktree a notice counted in since a read of it began, once", () => {
    const watch = new ScriptedWatch();
    const { notices: subject } = notices(watch);
    read(subject, "/a");
    read(subject, "/b");
    expect(subject.noticed()).toEqual(new Set());
    watch.notice("/a", { changed: ["src/a.py"] });
    watch.notice("/b", { changed: ["src/b.py"] });
    subject.collect();
    // The read of `/b` that begins now answers its notice.
    subject.reading("/b");
    expect([subject.noticed(), subject.noticed()]).toEqual([new Set(["/a"]), new Set()]);
  });

  it("reads on every walk a Linux worktree whose folders the watch budget cannot hold, until the retry", () => {
    const watch = new ScriptedWatch();
    const { notices: subject } = notices(watch, "linux");
    subject.changed("/w");
    subject.reading("/w");
    watch.refused.add("/w");
    subject.read("/w", seen);
    expect(watch.calls.at(-1)).toBe("unwatch /w");
    expect([subject.changed("/w"), subject.watching("/w")]).toEqual([true, false]);
  });

  it("watches again at once a worktree whose watch ended once another folder stands at its path, and waits while the same folder, or none, does", () => {
    const watch = new ScriptedWatch();
    const clock = { now: 0 };
    const folders = new Map([["/remade", "a"], ["/same", "b"], ["/gone", "c"]]);
    const subject = new WorktreeNotices({ treeWatch: watch, platform: "darwin", now: () => clock.now, folderAt: root => folders.get(root) ?? "" });
    for (const root of folders.keys()) {
      read(subject, root);
      watch.notice(root, { ended: true });
    }
    subject.collect();
    folders.set("/remade", "a2");
    folders.delete("/gone");
    expect([...folders.keys(), "/gone"].map(root => [subject.changed(root), subject.watching(root)])).toEqual([[true, true], [true, false], [true, false]]);
    clock.now = REWATCH_MS;
    folders.set("/gone", "c2");
    expect(["/same", "/gone"].map(root => [subject.changed(root), subject.watching(root)])).toEqual([[true, true], [true, true]]);
  });

  it("tries to watch at the next walk a worktree whose folder was gone as its watch would begin, not after the retry", () => {
    const watch = new ScriptedWatch();
    const folders = new Map<string, string>();
    const subject = new WorktreeNotices({ treeWatch: watch, platform: "darwin", now: () => 0, folderAt: root => folders.get(root) ?? "" });
    // A walk listed it, and its folder went before the walk came to it: the engine refuses a path with no folder.
    watch.refused.add("/w");
    expect([subject.changed("/w"), subject.watching("/w")]).toEqual([true, false]);
    // Made again: the next walk watches it, though `WATCH_RETRY_MS` has not passed.
    folders.set("/w", "a");
    watch.refused.delete("/w");
    expect([subject.changed("/w"), subject.watching("/w")]).toEqual([true, true]);
  });

  it("hands on the folder where Git registers the worktrees once an entry there came or went, or its gitdir or locked file did, and not for Git's work in an entry", () => {
    const watch = new ScriptedWatch();
    const { notices: subject } = notices(watch);
    subject.watchRegistry("/r");
    subject.watchRegistry("/r");
    expect([watch.calls, subject.watchingAny()]).toEqual([["watch /r [] spare"], true]);
    watch.notice("/r", { changed: ["w1", "w1/index", "w1/HEAD"], renamed: ["w1/index.lock", "w1/logs/HEAD"] });
    subject.collect();
    expect(subject.noticed()).toEqual(new Set());
    for (const notice of [{ renamed: ["w2"] }, { changed: ["w1/gitdir"] }, { renamed: ["w1/locked"] }, { lost: true }, { ended: true }]) {
      watch.notice("/r", notice);
      subject.collect();
      expect(subject.noticed()).toEqual(new Set(["/r"]));
    }
    // Its watch ended: watched again as the next walk names it.
    subject.watchRegistry("/r");
    expect(watch.calls).toEqual(["watch /r [] spare", "watch /r [] spare"]);
    subject.retain(new Set());
    expect([watch.calls.at(-1), subject.watchingAny()]).toEqual(["unwatch /r", false]);
  });

  it("follows each worktree's entry where Git registers the worktrees on Linux, and the entries there once one came", async () => {
    const folder = await mkdtemp(join(tmpdir(), "rerail-registry-"));
    try {
      await mkdir(join(folder, "w1"));
      await writeFile(join(folder, "stray"), "");
      const watch = new ScriptedWatch();
      const { notices: subject } = notices(watch, "linux");
      subject.watchRegistry(folder);
      await mkdir(join(folder, "w2"));
      watch.notice(folder, { changed: ["w1/index"] });
      subject.collect();
      watch.notice(folder, { renamed: ["w2"] });
      subject.collect();
      expect(watch.calls).toEqual([`watch ${folder} [,w1] spare`, `watch ${folder} [,w1,w2] spare`]);
      // One the watch refuses to follow is let go, and watched again as the next walk names it.
      watch.refused.add(folder);
      watch.notice(folder, { lost: true });
      subject.collect();
      expect(subject.watchingAny()).toBe(false);
      watch.refused.delete(folder);
      subject.watchRegistry(folder);
      expect([watch.calls.at(-1), subject.watchingAny()]).toEqual([`watch ${folder} [,w1,w2] spare`, true]);
    } finally {
      await removeFolder(folder);
    }
  });

  it("lets go of each worktree no walk watches any more", () => {
    const watch = new ScriptedWatch();
    const { notices: subject } = notices(watch);
    read(subject, "/a");
    read(subject, "/b");
    subject.retain(new Set(["/a"]));
    expect(watch.calls.filter(call => call.startsWith("unwatch"))).toEqual(["unwatch /b"]);
    expect(subject.watching("/b")).toBe(false);
  });
});

describe("folderAt", () => {
  it("tells a folder made again at a path from the one removed there, and gives none for a file or nothing there", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-folder-at-"));
    try {
      const path = join(root, "w");
      await mkdir(path);
      const first = folderAt(path);
      expect([first === "", folderAt(path) === first]).toEqual([false, true]);
      await removeFolder(path);
      expect(folderAt(path)).toBe("");
      // A folder made right after another went may take its file number on Linux.
      await mkdir(path);
      expect([folderAt(path) === "", folderAt(path) === first]).toEqual([false, false]);
      await removeFolder(path);
      await writeFile(path, "");
      expect(folderAt(path)).toBe("");
    } finally {
      await removeFolder(root);
    }
  });
});

describe("SharedTreeWatch", () => {
  it("hands each walk only the notices of its roots, and watches a root two walks share until both let go", () => {
    const watch = new ScriptedWatch();
    const shared = new SharedTreeWatch(watch);
    const [first, second] = [shared.user(), shared.user()];
    first.watch("/a", [""]);
    first.watch("/both", ["", "x"]);
    second.watch("/both", ["", "y"]);
    expect(watch.calls.at(-1)).toBe("watch /both [,x,y]");
    watch.notice("/a", { changed: ["f"] });
    watch.notice("/both", { changed: ["g"] });
    expect(first.take().map(notice => notice.root)).toEqual(["/a", "/both"]);
    expect(second.take().map(notice => notice.root)).toEqual(["/both"]);
    first.close();
    expect(watch.calls.slice(-2)).toEqual(["unwatch /a", "watch /both [,y]"]);
    second.unwatch("/both");
    expect(watch.calls.at(-1)).toBe("unwatch /both");
    shared.close();
    expect(watch.closed).toBe(true);
  });

  it("watches a root spare while each walk on it watches it spare, also once one of them lets go", () => {
    const watch = new ScriptedWatch();
    const shared = new SharedTreeWatch(watch);
    const [first, second] = [shared.user(), shared.user()];
    first.watch("/r", [""], true);
    second.watch("/r", ["", "x"], true);
    second.unwatch("/r");
    first.watch("/w", [""]);
    expect(watch.calls).toEqual(["watch /r [] spare", "watch /r [,x] spare", "watch /r [] spare", "watch /w []"]);
  });

  it("lets go of a root a walk was refused, so another walk's watch of it ends as that walk lets go", () => {
    const watch = new ScriptedWatch();
    const shared = new SharedTreeWatch(watch);
    const [first, second] = [shared.user(), shared.user()];
    watch.refused.add("/r");
    expect(first.watch("/r", [""])).toBe(false);
    watch.refused.delete("/r");
    expect(second.watch("/r", [""])).toBe(true);
    second.unwatch("/r");
    expect(watch.calls.at(-1)).toBe("unwatch /r");
  });

  it("holds one notice per root for a walk that takes its notices rarely, losing its paths past the engine's cap", () => {
    const watch = new ScriptedWatch();
    const shared = new SharedTreeWatch(watch);
    const [busy, idle] = [shared.user(), shared.user()];
    busy.watch("/r", [""]);
    idle.watch("/r", [""]);
    for (const name of ["a", "b", "a"]) {
      watch.notice("/r", { changed: [`${name}.py`] });
      busy.take();
    }
    watch.notice("/r", { renamed: ["c.py"] });
    busy.take();
    expect(idle.take()).toEqual([{ root: "/r", changed: ["a.py", "b.py"], renamed: ["c.py"], lost: false, ended: false }]);
    watch.notice("/r", { changed: Array.from({ length: NOTICE_PATHS_CAP }, (_, index) => `f${index}.py`) });
    busy.take();
    watch.notice("/r", { changed: ["g.py"] });
    busy.take();
    expect(idle.take()).toEqual([{ root: "/r", changed: [], renamed: [], lost: true, ended: false }]);
  });

  it("tells the walks left on a root their notices were lost when another walk's watch of it was refused, and that it ended when theirs is too", () => {
    const watch = new ScriptedWatch();
    const shared = new SharedTreeWatch(watch);
    const [first, second, third] = [shared.user(), shared.user(), shared.user()];
    second.watch("/r", [""]);
    watch.notice("/r", { changed: ["a.py"] });
    watch.overBudget.add("big");
    expect(first.watch("/r", ["", "big"])).toBe(false);
    expect(watch.calls.at(-1)).toBe("watch /r []");
    expect(second.take()).toEqual([{ root: "/r", changed: [], renamed: [], lost: true, ended: false }]);
    watch.refused.add("/r");
    expect(first.watch("/r", [""])).toBe(false);
    expect(second.take()).toEqual([{ root: "/r", changed: [], renamed: [], lost: true, ended: true }]);
    second.unwatch("/r");
    expect(watch.calls.at(-1)).toBe("watch /r []");
    second.watch("/s", [""]);
    third.watch("/s", [""]);
    watch.refused.add("/s");
    second.unwatch("/s");
    expect(third.take()).toEqual([{ root: "/s", changed: [], renamed: [], lost: true, ended: true }]);
  });

  it("keeps an ended watch's notice for a walk that lets go of its last other root before it takes it", () => {
    const watch = new ScriptedWatch();
    const shared = new SharedTreeWatch(watch);
    const [first, second] = [shared.user(), shared.user()];
    first.watch("/x", [""]);
    first.watch("/y", [""]);
    second.watch("/x", [""]);
    watch.notice("/x", { ended: true });
    expect(second.take()).toEqual([{ root: "/x", changed: [], renamed: [], lost: false, ended: true }]);
    first.unwatch("/y");
    expect(first.take()).toEqual([{ root: "/x", changed: [], renamed: [], lost: false, ended: true }]);
    expect(first.take()).toEqual([]);
  });

  it("drops what was queued of a root for a walk that lets go of it, its ended watch's notice included", () => {
    const watch = new ScriptedWatch();
    const shared = new SharedTreeWatch(watch);
    const [first, second] = [shared.user(), shared.user()];
    first.watch("/x", [""]);
    first.watch("/y", [""]);
    second.watch("/x", [""]);
    second.watch("/y", [""]);
    watch.notice("/x", { changed: ["a.py"] });
    watch.notice("/y", { ended: true });
    second.take();
    first.unwatch("/x");
    first.unwatch("/y");
    expect(first.take()).toEqual([]);
  });
});
