import { mkdirSync, mkdtempSync, realpathSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { setTimeout as sleep } from "node:timers/promises";
import { afterEach, describe, expect, it, vi } from "vitest";
import { nativeTreeWatch, type TreeNotice, type TreeWatch } from "../../../src/engine/code-rule/tree-watch.js";
import { loadNativeBinding } from "../../../src/engine/native/native.js";
import { removeFolder } from "../../support/platform/remove-folder.js";

vi.mock("../../../src/engine/native/native.js", { spy: true });

const roots: string[] = [];
const watches: TreeWatch[] = [];
afterEach(async () => {
  vi.mocked(loadNativeBinding).mockReset();
  for (const watch of watches.splice(0)) watch.close();
  await Promise.all(roots.splice(0).map(root => removeFolder(root)));
});

/** A git-free folder of its own, by its physical path, as a host watches it. */
function scratch(): string {
  const root = realpathSync(mkdtempSync(join(tmpdir(), "rerail-tree-watch-")));
  roots.push(root);
  return root;
}

function engineWatch(): TreeWatch {
  const watch = nativeTreeWatch();
  if (watch === undefined) throw new Error("the engine has no TreeWatcher");
  watches.push(watch);
  return watch;
}

/** Takes until a notice of `root` names `path` as changed; fails after ten seconds. */
async function noticeNaming(watch: TreeWatch, root: string, path: string): Promise<TreeNotice[]> {
  const taken: TreeNotice[] = [];
  for (const started = Date.now(); Date.now() - started < 10_000; await sleep(20)) {
    taken.push(...watch.take().filter(notice => notice.root === root));
    if (taken.some(notice => notice.changed.includes(path))) return taken;
  }
  throw new Error(`no notice named ${path}: ${JSON.stringify(taken)}`);
}

describe("the engine's tree watch", () => {
  it("names a file written under a watched root, '/'-separated, and nothing once the root is unwatched", async () => {
    const root = scratch();
    mkdirSync(join(root, "src"));
    writeFileSync(join(root, "src", "a.txt"), "a");
    const watch = engineWatch();
    expect(watch.watch(root, ["", "src"])).toBe(true);

    writeFileSync(join(root, "src", "a.txt"), "written");
    const taken = await noticeNaming(watch, root, "src/a.txt");
    expect(taken.some(notice => notice.ended)).toBe(false);

    watch.unwatch(root);
    writeFileSync(join(root, "src", "a.txt"), "after");
    writeFileSync(join(root, "src", "b.txt"), "created");
    await sleep(500);
    expect(watch.take()).toEqual([]);
  });

  it("watches a spare root as any root while the watch budget has room", async () => {
    const root = scratch();
    mkdirSync(join(root, "src"));
    const watch = engineWatch();
    expect(watch.watch(root, ["", "src"], true)).toBe(true);

    writeFileSync(join(root, "src", "a.txt"), "written");
    const taken = await noticeNaming(watch, root, "src/a.txt");
    expect(taken.some(notice => notice.ended)).toBe(false);
  });

  it("watches a spare root through the engine's spare watch, and not at all with an engine built before it", () => {
    const calls: string[] = [];
    class Spare {
      watch(root: string): boolean {
        calls.push(`watch ${root}`);
        return true;
      }
      watchSpare(root: string): boolean {
        calls.push(`spare ${root}`);
        return true;
      }
      unwatch(): void {}
      take(): TreeNotice[] {
        return [];
      }
      close(): void {}
    }
    class Older {
      watch = Spare.prototype.watch;
      unwatch(): void {}
      take(): TreeNotice[] {
        return [];
      }
      close(): void {}
    }
    for (const TreeWatcher of [Spare, Older]) {
      vi.mocked(loadNativeBinding).mockReturnValueOnce({ ...loadNativeBinding(), TreeWatcher });
      const watch = nativeTreeWatch()!;
      calls.push(`${watch.watch("/r", [""], true)} ${watch.watch("/w", [""])}`);
    }
    expect(calls).toEqual(["spare /r", "watch /w", "true true", "watch /w", "false true"]);
  });

  it("is undefined when the loaded engine was built before it", () => {
    const { TreeWatcher: _, ...older } = loadNativeBinding();
    vi.mocked(loadNativeBinding).mockReturnValueOnce(older);
    expect(nativeTreeWatch()).toBeUndefined();
  });
});
