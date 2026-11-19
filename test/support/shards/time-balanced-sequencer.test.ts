import { describe, expect, it } from "vitest";
import { assignShards, defaultWeight, platformTimes, type PlatformTimes } from "./time-balanced-sequencer.js";

/** Each shard's paths, in the order assignShards placed them, and each shard's total weight. */
function split(paths: readonly string[], times: PlatformTimes, count: number) {
  const shards = assignShards(paths, times, count);
  const fallback = defaultWeight(times.files);
  const members = Array.from({ length: count }, () => [] as string[]);
  const loads = Array.from({ length: count }, (_, shard) => (shard === 0 ? times.firstShardExtraMs : 0));
  for (const [path, shard] of shards) {
    members[shard]!.push(path);
    loads[shard]! += times.files[path] ?? fallback;
  }
  return { members, loads };
}

describe("assignShards", () => {
  it("puts every file in exactly one shard, and evens the shards' time where counting files would not", () => {
    // By count, two shards of three files each; the hash order vitest uses could put both long files in one.
    const files = { "a.test.ts": 90, "b.test.ts": 80, "c.test.ts": 30, "d.test.ts": 30, "e.test.ts": 20, "f.test.ts": 10 };
    const { members, loads } = split(Object.keys(files), { firstShardExtraMs: 0, files }, 2);
    expect(members.flat().sort()).toEqual(Object.keys(files));
    expect(loads).toEqual([130, 130]);
    expect(members[0]).toContain("a.test.ts");
    expect(members[1]).toContain("b.test.ts");
  });

  it("gives every shard the same split whatever order vitest lists the files in", () => {
    const files = { "a.test.ts": 5, "b.test.ts": 5, "c.test.ts": 5, "d.test.ts": 3, "e.test.ts": 3 };
    const times = { firstShardExtraMs: 0, files };
    const paths = Object.keys(files);
    expect(assignShards([...paths].reverse(), times, 3)).toEqual(assignShards(paths, times, 3));
  });

  it("counts a file the times do not name as the median file", () => {
    const files = { "a.test.ts": 100, "b.test.ts": 10, "c.test.ts": 40 };
    expect(defaultWeight(files)).toBe(40);
    expect(defaultWeight({ ...files, "d.test.ts": 20 })).toBe(30);
    const { loads } = split([...Object.keys(files), "new.test.ts"], { firstShardExtraMs: 0, files }, 2);
    expect(loads).toEqual([100, 90]);
  });

  it("starts the first shard holding the step it runs after the suite", () => {
    const files = { "a.test.ts": 60, "b.test.ts": 60, "c.test.ts": 60 };
    const { members } = split(Object.keys(files), { firstShardExtraMs: 100, files }, 2);
    expect(members).toEqual([["b.test.ts"], ["a.test.ts", "c.test.ts"]]);
  });

  it("leaves no shard without a file, even when the first shard's extra time outweighs every file", () => {
    const files = { "a.test.ts": 1000, "b.test.ts": 1, "c.test.ts": 1 };
    const { members } = split(Object.keys(files), { firstShardExtraMs: 5000, files }, 3);
    expect(members).toEqual([["c.test.ts"], ["a.test.ts"], ["b.test.ts"]]);
  });
});

describe("platformTimes", () => {
  const linux = { firstShardExtraMs: 1, files: { "a.test.ts": 1 } };
  const win32 = { firstShardExtraMs: 2, files: { "a.test.ts": 2 } };

  it("uses the platform's own times, and Linux's on a platform CI does not shard", () => {
    expect(platformTimes({ linux, win32 }, "win32")).toBe(win32);
    expect(platformTimes({ linux, win32 }, "darwin")).toBe(linux);
    expect(platformTimes({}, "linux")).toEqual({ firstShardExtraMs: 0, files: {} });
  });
});
