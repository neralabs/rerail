import { mkdtemp, readdir, readFile, realpath, rm, stat, utimes, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import type { ObservedSource } from "../../../../src/engine/code-rule/code-rule-bridge.js";
import { keptRead, stampTellsChanges, WalkDigests } from "../../../../src/engine/code-rule/walk-digests.js";

const directories: string[] = [];
afterEach(async () => {
  await Promise.all(directories.splice(0).map(path => rm(path, { recursive: true, force: true })));
});

async function scratch(): Promise<string> {
  const root = await realpath(await mkdtemp(join(tmpdir(), "rerail-walk-digests-")));
  directories.push(root);
  return root;
}

/** A stamp as `fileStamp` writes it: device, file id, size, modification and change times. */
function stamp({ ino = 7, size = 6, mtimeMs = 1_000.25, ctimeMs = 2_000.5 } = {}): string {
  return `66:${ino}:${size}:${mtimeMs}:${ctimeMs}`;
}

const DIGEST = "a".repeat(64);

/** A file a walk read whole at `observedAt`, after its stamp's tick. */
function read(fields: Partial<ObservedSource> = {}): ObservedSource {
  return { path: "src/a.py", language: "python", stamp: stamp(), modifiedAt: 2_000.5, observedAt: 3_000, digest: DIGEST, ...fields };
}

/** `source` without `keys`: an unlisted binary file has no digest, and a file no rule reads was never read. */
function without(source: ObservedSource, ...keys: Array<"digest" | "modifiedAt">): ObservedSource {
  const copy = { ...source };
  for (const key of keys) delete copy[key];
  return copy;
}

describe("keptRead", () => {
  it("keeps a file read whole after its stamp's tick, and an unlisted file found binary", () => {
    expect(keptRead("src/a.py", read())).toEqual({ kind: "text", stamp: stamp(), digest: DIGEST });
    expect(keptRead("logo.png", without(read(), "digest"))).toEqual({ kind: "binary", stamp: stamp() });
  });

  it("keeps nothing of a file it would have to read again to be sure of", () => {
    expect({
      // Read within the stamp's own clock tick: a write in that tick leaves the same stamp.
      racy: keptRead("src/a.py", read({ observedAt: 2_000.5 })),
      // A write while the walk read it moved `modifiedAt` past the stamp: the bytes read may be neither version.
      writtenWhileRead: keptRead("src/a.py", read({ modifiedAt: 2_500 })),
      unreadable: keptRead("src/a.py", read({ readError: "EACCES" })),
      // No rule reads it, so the walk never read it.
      notRead: keptRead("src/a.py", without(read(), "modifiedAt", "digest")),
      secretByName: keptRead(".env", read()),
      secretFolder: keptRead(".ssh/config", read()),
      outsideTheRoot: keptRead("../w2/src/a.py", read()),
    }).toEqual({
      racy: undefined, writtenWhileRead: undefined, unreadable: undefined, notRead: undefined, secretByName: undefined,
      secretFolder: undefined, outsideTheRoot: undefined,
    });
  });

  it("trusts a stamp only when it moves with every write, whatever the modification time says", () => {
    expect({
      ntfsOrExt4: stampTellsChanges(stamp()),
      // The change time is the file system's own: a copy that puts the modification time back still moves it.
      modificationTimeOnAGrid: stampTellsChanges(stamp({ mtimeMs: 1_000_000, ctimeMs: 2_000.5 })),
      noFileId: stampTellsChanges(stamp({ ino: 0 })),
      noChangeTime: stampTellsChanges(stamp({ ctimeMs: 0 })),
      // FAT and exFAT report the modification time as the change time, on a 2 s or 10 ms grid.
      fat: stampTellsChanges(stamp({ mtimeMs: 1_731_400_000_000, ctimeMs: 1_731_400_000_000 })),
      exFat: stampTellsChanges(stamp({ mtimeMs: 1_731_400_000_010, ctimeMs: 1_731_400_000_010 })),
      notAStamp: stampTellsChanges("66:7:6"),
    }).toEqual({ ntfsOrExt4: true, modificationTimeOnAGrid: true, noFileId: false, noChangeTime: false, fat: false, exFat: false, notAStamp: false });
    expect(keptRead("src/a.py", read({ stamp: stamp({ ctimeMs: 2_000 }), modifiedAt: 2_000 }))).toBeUndefined();
  });
});

describe("WalkDigests", () => {
  it("gives the next daemon what a walk of a root read, and none of it to a walk on a clock behind that walk", async () => {
    const [folder, root] = [await scratch(), await scratch()];
    const sources = new Map([["src/a.py", read()], ["logo.png", without(read({ path: "logo.png" }), "digest")], [".env", read({ path: ".env" })]]);
    const first = new WalkDigests(folder);
    first.keep(root, { sources, previous: new Map(), kept: undefined, observedAt: 3_000 });
    await first.close();

    const next = new WalkDigests(folder);
    expect(Object.fromEntries(await next.load(root, 4_000))).toEqual({
      "src/a.py": { kind: "text", stamp: stamp(), digest: DIGEST },
      "logo.png": { kind: "binary", stamp: stamp() },
    });
    // The clock went back while no daemon ran: a file written since may carry a stamp it had before.
    expect((await next.load(root, 2_999)).size).toBe(0);
    expect((await next.load(await scratch(), 4_000)).size).toBe(0);
    await next.close();
  });

  it("writes nothing for a walk that found every file as it started or as the last daemon kept it", async () => {
    const [folder, root] = [await scratch(), await scratch()];
    const unchanged = read();
    const digests = new WalkDigests(folder);
    digests.keep(root, { sources: new Map([["src/a.py", unchanged]]), previous: new Map([["src/a.py", unchanged]]), kept: undefined, observedAt: 3_000 });
    digests.keep(root, {
      sources: new Map([["src/a.py", read()]]), previous: new Map(), kept: new Map([["src/a.py", { kind: "text", stamp: stamp(), digest: DIGEST }]]), observedAt: 3_000,
    });
    await digests.close();
    expect(await readdir(folder).catch(() => [])).toEqual([]);
  });

  it("reads a file it cannot decode as no kept reads", async () => {
    const [folder, root] = [await scratch(), await scratch()];
    const first = new WalkDigests(folder);
    first.keep(root, { sources: new Map([["src/a.py", read()]]), previous: new Map(), kept: undefined, observedAt: 3_000 });
    await first.close();
    const [name] = await readdir(folder);
    await writeFile(join(folder, name!), `${JSON.stringify({ format: 1, root, observedAt: 3_000 })}\nnot gzip`);
    const next = new WalkDigests(folder);
    expect((await next.load(root, 4_000)).size).toBe(0);
    await next.close();
  });

  it("drops a root whose folder is gone, a worktree it is told is gone, and past its bound the roots used longest ago", async () => {
    const folder = await scratch();
    const roots = await Promise.all([0, 1, 2, 3, 4, 5, 6].map(async () => await scratch()));
    const first = new WalkDigests(folder);
    for (const root of roots) first.keep(root, { sources: new Map([["src/a.py", read()]]), previous: new Map(), kept: undefined, observedAt: 3_000 });
    await first.close();
    const files = await keptFiles(folder);
    expect(files.size).toBe(7);
    // Every root's file is the same size. Its time is when a walk last used it: root 0 longest ago, root 6 last.
    const size = (await stat(files.get(roots[0]!)!)).size;
    for (const [index, root] of roots.entries()) {
      const usedAt = new Date(Date.now() - (7 - index) * 60_000);
      await utimes(files.get(root)!, usedAt, usedAt);
    }
    await rm(roots[6]!, { recursive: true });

    const bounded = new WalkDigests(folder, { maxBytes: size * 4 + size / 2 });
    bounded.forget(roots[5]!);
    await bounded.close();
    // Root 6 is gone from disk and root 5 was forgotten; of the five left, room for four, so root 0 went too.
    expect([...(await keptFiles(folder)).keys()].map(root => roots.indexOf(root)).sort()).toEqual([1, 2, 3, 4]);
  });

  it("keeps nothing, and fails no walk, where it cannot write", async () => {
    const root = await scratch();
    const blocked = join(root, "file");
    await writeFile(blocked, "");
    const digests = new WalkDigests(join(blocked, "walk-digests"));
    digests.keep(root, { sources: new Map([["src/a.py", read()]]), previous: new Map(), kept: undefined, observedAt: 3_000 });
    await expect(digests.close()).resolves.toBeUndefined();
    expect((await new WalkDigests(join(blocked, "walk-digests")).load(root, 4_000)).size).toBe(0);
  });
});

/** Each root's file in `folder`, by the root its first line names. */
async function keptFiles(folder: string): Promise<Map<string, string>> {
  const files = new Map<string, string>();
  for (const name of await readdir(folder)) {
    const header: unknown = JSON.parse((await readFile(join(folder, name), "latin1")).split("\n")[0]!);
    files.set((header as { root: string }).root, join(folder, name));
  }
  return files;
}
