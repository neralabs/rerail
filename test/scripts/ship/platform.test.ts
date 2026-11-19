// scripts/ship/platform.ts and platform-watch.mjs: how each platform's measurement is read back, which CPUs a pinned
// run must leave idle, the memory cap in bytes, the process tree a macOS cap kills, and the methods a report counts.

import { describe, expect, it } from "vitest";
import { descendants } from "../../../scripts/ship/platform-watch.mjs";
import { busiest, capBytes, cpuList, fromLauncher, idleSiblingsOf, LINUX_METHOD, methodCounts, methodGaps, parseBsdTime, parseGnuTime, parseProcStat, type RunMethod } from "../../../scripts/ship/platform.js";

describe("CPU lists and siblings", () => {
  it("reads taskset's and sysfs's lists, and refuses what is not one", () => {
    expect(cpuList("8-11")).toEqual([8, 9, 10, 11]);
    expect(cpuList("6,0,2-3,2")).toEqual([0, 2, 3, 6]);
    expect(() => cpuList("3-1")).toThrow();
    expect(() => cpuList("a")).toThrow();
    expect(() => cpuList("1-")).toThrow();
  });

  it("leaves each pinned CPU's other threads idle and refuses two threads of one core", () => {
    const cores = [[0, 8], [1, 9], [2, 10], [3, 11]];
    expect(idleSiblingsOf([0, 1, 2], cores)).toEqual([8, 9, 10]);
    expect(idleSiblingsOf([0, 1], [[0], [1]])).toEqual([]);
    expect(() => idleSiblingsOf([0, 8], cores)).toThrow(/two threads of one core/);
    expect(() => idleSiblingsOf([4], cores)).toThrow(/no CPU 4/);
  });

  it("takes the busiest of the named CPUs between two samples", () => {
    const before = parseProcStat("cpu  1 1 1 1\ncpu0 100 0 0 900 0 0 0 0\ncpu1 0 0 0 1000 0 0 0 0\n");
    const after = parseProcStat("cpu0 600 0 0 1400 0 0 0 0\ncpu1 100 0 0 1900 0 0 0 0\n");
    expect(busiest(before, after, [0, 1])).toBe(0.5);
    expect(busiest(before, after, [1])).toBe(0.1);
    expect(busiest(before, after, [])).toBe(0);
  });
});

describe("memory caps", () => {
  it("reads systemd's sizes in powers of 1024", () => {
    expect(capBytes("24G")).toBe(24 * 1024 ** 3);
    expect(capBytes("512m")).toBe(512 * 1024 ** 2);
    expect(capBytes("1.5K")).toBe(1536);
    expect(capBytes("4096")).toBe(4096);
    expect(() => capBytes("24GB")).toThrow();
  });
});

describe("reading a run's measurement back", () => {
  it("reads GNU time's line, and zeros for a run that left none", () => {
    expect(parseGnuTime("Command exited with non-zero status 1\n12.34 10.50 1.25 2048000\n")).toEqual({ wallS: 12.34, userS: 10.5, sysS: 1.25, peakKB: 2048000 });
    expect(parseGnuTime("")).toEqual({ wallS: 0, userS: 0, sysS: 0, peakKB: 0 });
  });

  it("reads macOS's time -l report, whose peak RSS is in bytes", () => {
    const report = "        3.21 real         2.50 user         0.40 sys\n          1073741824  maximum resident set size\n                   0  average shared memory size\n";
    expect(parseBsdTime(report)).toEqual({ wallS: 3.21, userS: 2.5, sysS: 0.4, peakKB: 1048576 });
    expect(parseBsdTime("time: command terminated abnormally\n")).toBeNull();
  });

  it("marks a run a Windows job cap ended as killed, and falls back to committed memory without a working set", () => {
    const base = { exit: 0, killedBy: null, wallMs: 1500, userMs: 1000, sysMs: 250, peakWorkingSetKB: 2048, peakCommitKB: 4096, pinnedBusy: 0.9, siblingBusy: 0.01 } as const;
    expect(fromLauncher(base)).toMatchObject({ exit: 0, killedBy: null, wallS: 1.5, userS: 1, sysS: 0.25, peakKB: 2048 });
    expect(fromLauncher({ ...base, exit: 3221225495, killedBy: "memory" })).toMatchObject({ exit: 137, killedBy: "memory" });
    expect(fromLauncher({ ...base, peakWorkingSetKB: 0 }).peakKB).toBe(4096);
  });
});

describe("the process tree a macOS cap kills", () => {
  it("finds every process below the measuring time, not time itself, with each one's RSS", () => {
    const ps = ["  1     0   100", " 50     1  1000", " 51    50  2000", " 52    51  3000", " 60     1  9000", ""].join("\n");
    expect(descendants(ps, 50)).toEqual([
      { pid: 51, rssKB: 2000 },
      { pid: 52, rssKB: 3000 },
    ]);
    expect(descendants(ps, 99)).toEqual([]);
  });
});

describe("methods in the report", () => {
  const darwin: RunMethod = { platform: "darwin", pinning: "none", measured: "bsd-time", peak: "max-rss", memoryCap: "polled-rss", timeCap: "watchdog" };

  it("counts the runs each method measured, most first", () => {
    const rows = [{ method: LINUX_METHOD }, { method: darwin }, { method: { ...darwin } }];
    expect(methodCounts(rows)).toEqual([
      { method: darwin, runs: 2 },
      { method: LINUX_METHOD, runs: 1 },
    ]);
  });

  it("names no gap for Linux's pinned, kernel-capped runs and each one for macOS's", () => {
    expect(methodGaps(LINUX_METHOD)).toEqual([]);
    expect(methodGaps(darwin).map((gap) => gap.split(":")[0])).toEqual(["not pinned", "memory cap polled, not enforced by the kernel"]);
  });
});
