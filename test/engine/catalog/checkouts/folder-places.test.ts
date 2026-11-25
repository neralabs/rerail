import { chmodSync, existsSync, mkdirSync, mkdtempSync, realpathSync, renameSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { physicalPath, storedPath } from "../../../../src/engine/catalog/checkout-control.js";
import { FolderPlaces, type PlaceWatch } from "../../../../src/engine/catalog/checkouts/folder-places.js";
import { nativeTreeWatch, type TreeWatch } from "../../../../src/engine/code-rule/tree-watch.js";
import { linkFolder } from "../../../support/platform/links.js";
import { removeFolderSync } from "../../../support/platform/remove-folder.js";

const cleanups: Array<() => void> = [];
afterEach(() => {
  for (const cleanup of cleanups.splice(0).reverse()) cleanup();
});

function scratch(): string {
  const root = realpathSync.native(mkdtempSync(join(tmpdir(), "rerail-places-")));
  cleanups.push(() => removeFolderSync(root));
  return root;
}

/** A folder holding a file, made at `path`. */
function folder(path: string): string {
  mkdirSync(path, { recursive: true });
  writeFileSync(join(path, "a.txt"), "a");
  return path;
}

/** A watch whose notices the test hands over, recording what it watches. */
class FakeWatch implements PlaceWatch {
  readonly watched = new Set<string>();
  readonly queued: Array<{ root: string; lost: boolean; ended: boolean }> = [];
  refuse = false;
  watch(root: string): boolean {
    if (this.refuse) return false;
    this.watched.add(root);
    return true;
  }
  unwatch(root: string): void {
    this.watched.delete(root);
  }
  take(): Array<{ root: string; lost: boolean; ended: boolean }> {
    return this.queued.splice(0);
  }
}

/** The answers the disk gives for `path`, as main asks them at every look-up. */
function disk(path: string): [string, string, boolean] {
  return [storedPath(path), physicalPath(path), existsSync(path)];
}

function answers(places: FolderPlaces, path: string): [string, string, boolean] {
  return [places.worktreeRoot(path), places.physical(path), places.exists(path)];
}

/** The next stretch of work, as the next event's: the places take the watch's notices once per stretch. */
const nextUse = (): Promise<void> => new Promise(resolve => setImmediate(resolve));

describe("FolderPlaces with the watch's notices handed over", () => {
  it("keeps a watched worktree folder's place until a notice that its watch ended or lost its notices", async () => {
    const root = scratch();
    const worktree = folder(join(root, "w1"));
    const watch = new FakeWatch();
    const places = new FolderPlaces({ watch });
    expect(answers(places, worktree)).toEqual(disk(worktree));
    expect([...watch.watched]).toEqual([worktree]);

    // Moved with no notice yet: the kept answer stands, which is what the notice exists for.
    renameSync(worktree, join(root, "moved"));
    await nextUse();
    expect(places.worktreeRoot(worktree)).toBe(worktree);
    expect(places.exists(worktree)).toBe(true);

    for (const notice of [{ ended: true, lost: false }, { ended: false, lost: true }]) {
      watch.queued.push({ root: worktree, ...notice });
      await nextUse();
      expect(answers(places, worktree)).toEqual(disk(worktree));
      expect(watch.watched.has(worktree)).toBe(false);
      folder(worktree);
      expect(answers(places, worktree)).toEqual(disk(worktree));
      removeFolderSync(worktree);
    }
  });

  it("keeps nothing on the way through a link, for a missing folder, where the watch refuses, or on macOS", async () => {
    const root = scratch();
    const target = folder(join(root, "target"));
    const linked = join(root, "linked");
    linkFolder(target, linked);
    const watch = new FakeWatch();
    const places = new FolderPlaces({ watch });
    expect(answers(places, linked)).toEqual(disk(linked));
    expect(answers(places, join(root, "missing"))).toEqual(disk(join(root, "missing")));
    expect(watch.watched.size).toBe(0);

    const other = folder(join(root, "other"));
    removeFolderSync(linked);
    linkFolder(other, linked);
    await nextUse();
    expect(answers(places, linked)).toEqual(disk(linked));

    watch.refuse = true;
    const plain = folder(join(root, "plain"));
    places.worktreeRoot(plain);
    renameSync(plain, join(root, "plain-moved"));
    expect(answers(places, plain)).toEqual(disk(plain));

    const mac = new FolderPlaces({ watch: new FakeWatch(), platform: "darwin" });
    const kept = folder(join(root, "mac"));
    mac.worktreeRoot(kept);
    renameSync(kept, join(root, "mac-moved"));
    expect(answers(mac, kept)).toEqual(disk(kept));
  });

  it("gives every place from the disk with no watch", () => {
    const root = scratch();
    const worktree = folder(join(root, "w1"));
    const places = new FolderPlaces();
    places.worktreeRoot(worktree);
    renameSync(worktree, join(root, "moved"));
    expect(answers(places, worktree)).toEqual(disk(worktree));
  });
});

/** The engine's watch, closed when the test ends; undefined with an engine built before it. */
function engineWatch(): TreeWatch | undefined {
  const watch = nativeTreeWatch();
  if (watch !== undefined) cleanups.push(() => watch.close());
  return watch;
}

/** Where a move of a watched folder is a notice before the next look-up; the places keep nothing elsewhere. */
const exact = process.platform === "linux" || process.platform === "win32";

describe.skipIf(!exact || nativeTreeWatch() === undefined)("FolderPlaces with the engine's watch: answers equal the disk's at the next use", () => {
  it("a worktree folder replaced by a link and back, and moved by a rename of a folder above it", async () => {
    const root = scratch();
    const worktree = folder(join(root, "wt", "w1"));
    const places = new FolderPlaces({ watch: engineWatch() });
    expect(answers(places, worktree)).toEqual(disk(worktree));

    const elsewhere = folder(join(root, "elsewhere"));
    renameSync(worktree, join(root, "w1-old"));
    linkFolder(elsewhere, worktree);
    await nextUse();
    expect(answers(places, worktree)).toEqual(disk(worktree));

    removeFolderSync(worktree);
    renameSync(join(root, "w1-old"), worktree);
    await nextUse();
    expect(answers(places, worktree)).toEqual(disk(worktree));
    expect(answers(places, worktree)).toEqual(disk(worktree));

    // The folder above renamed, and a link to where it went put in its place: the same folder, another path.
    renameSync(join(root, "wt"), join(root, "wt-moved"));
    linkFolder(join(root, "wt-moved"), join(root, "wt"));
    await nextUse();
    expect(answers(places, worktree)).toEqual(disk(worktree));
  });

  it("a worktree folder deleted and made again, and one moved away", async () => {
    const root = scratch();
    const worktree = folder(join(root, "w1"));
    const places = new FolderPlaces({ watch: engineWatch() });
    expect(answers(places, worktree)).toEqual(disk(worktree));
    removeFolderSync(worktree);
    await nextUse();
    expect(answers(places, worktree)).toEqual(disk(worktree));
    folder(worktree);
    await nextUse();
    expect(answers(places, worktree)).toEqual(disk(worktree));
    expect(answers(places, worktree)).toEqual(disk(worktree));
    renameSync(worktree, join(root, "w1-away"));
    await nextUse();
    expect(answers(places, worktree)).toEqual(disk(worktree));
  });

  it("a link at a worktree's path retargeted", async () => {
    const root = scratch();
    const first = folder(join(root, "first"));
    const second = folder(join(root, "second"));
    const worktree = join(root, "w1");
    linkFolder(first, worktree);
    const places = new FolderPlaces({ watch: engineWatch() });
    expect(answers(places, worktree)).toEqual(disk(worktree));
    removeFolderSync(worktree);
    linkFolder(second, worktree);
    await nextUse();
    expect(answers(places, worktree)).toEqual(disk(worktree));
  });

  it.skipIf(process.platform !== "linux" || process.getuid?.() === 0)("a folder above a worktree that the engine cannot follow, renamed", async () => {
    const root = scratch();
    const worktree = folder(join(root, "locked", "w1"));
    // A folder this process may enter but not list: the engine's watch cannot follow it, so its rename is no notice.
    chmodSync(join(root, "locked"), 0o311);
    cleanups.push(() => {
      for (const name of ["locked", "locked-moved"]) if (existsSync(join(root, name))) chmodSync(join(root, name), 0o755);
    });
    const places = new FolderPlaces({ watch: engineWatch() });
    expect(answers(places, worktree)).toEqual(disk(worktree));
    renameSync(join(root, "locked"), join(root, "locked-moved"));
    await nextUse();
    expect(answers(places, worktree)).toEqual(disk(worktree));
  });

  it.skipIf(process.platform !== "win32")("a worktree folder renamed in letter case only", async () => {
    const root = scratch();
    const worktree = folder(join(root, "w1"));
    const places = new FolderPlaces({ watch: engineWatch() });
    expect(answers(places, worktree)).toEqual(disk(worktree));
    renameSync(worktree, join(root, "W1"));
    await nextUse();
    expect(answers(places, worktree)).toEqual(disk(worktree));
    expect(places.worktreeRoot(worktree)).toBe(join(root, "W1"));
  });
});
