import { describe, expect, it } from "vitest";
import type { TreeNotice, TreeWatch } from "../../../../src/engine/code-rule/tree-watch.js";
import { noticeCounts, REWATCH_MS, SharedTreeWatch, WATCH_RETRY_MS, WorktreeNotices } from "../../../../src/engine/code-rule/worktree-notices.js";

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
  closed = false;

  watch(root: string, folders: readonly string[]): boolean {
    this.calls.push(`watch ${root} [${folders.join(",")}]`);
    return !this.refused.has(root);
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

/** `WorktreeNotices` on `platform` over `watch`, at a clock the test moves. */
function notices(watch: TreeWatch | undefined, platform: NodeJS.Platform = "darwin"): { notices: WorktreeNotices; clock: { now: number } } {
  const clock = { now: 0 };
  return { notices: new WorktreeNotices({ treeWatch: watch, platform, now: () => clock.now }), clock };
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
});
