/**
 * The conformance check spread over shards (conformance/languages/shards.ts): every job runs in exactly one shard, and a
 * report is made only from a result for every job, each from one shard of the same plan.
 */
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";

import type { JobResult } from "../../conformance/languages/runner.js";
import { parseShard, readResults, shardJobs, writeResults, type JobOutline } from "../../conformance/languages/shards.js";
import { removeFolderSync } from "../support/platform/remove-folder.js";

/** Fixture units of three languages, interleaved as test files interleave them, then sample rules of uneven weight. */
const JOBS: JobOutline[] = [
  ...Array.from({ length: 97 }, (_, index): JobOutline => ({ source: "fixtures", weight: 1 + (index % 7), group: ["", "es5", "module"][index % 3]! })),
  ...Array.from({ length: 23 }, (_, index): JobOutline => ({ source: "samples", weight: index === 5 ? 500 : 3 + index, group: "" })),
];

describe("shardJobs", () => {
  it("runs every job in exactly one shard, whatever the number of shards", () => {
    for (let count = 1; count <= 9; count += 1) {
      const ran = Array.from({ length: count }, (_, index) => shardJobs(JOBS, { index: index + 1, count }).flat()).flat();
      expect(ran.toSorted((a, b) => a - b), `${count} shard(s)`).toEqual(JOBS.map((_, index) => index));
    }
  });

  it("runs a source's jobs grouped by group, groups in order of first appearance", () => {
    const rank = new Map<string, number>();
    for (const job of JOBS.filter(entry => entry.source === "fixtures")) if (!rank.has(job.group)) rank.set(job.group, rank.size);
    for (let index = 1; index <= 3; index += 1) {
      const [fixtures] = shardJobs(JOBS, { index, count: 3 });
      const ranks = fixtures!.map(job => rank.get(JOBS[job]!.group)!);
      expect(ranks, `shard ${index}`).toEqual(ranks.toSorted((a, b) => a - b));
    }
  });
});

describe("parseShard", () => {
  it("takes k/n with 1 ≤ k ≤ n and refuses anything else", () => {
    expect(parseShard("3/8")).toEqual({ index: 3, count: 8 });
    for (const text of ["0/2", "3/2", "2", "1/0", "a/b"]) expect(() => parseShard(text), text).toThrow(/--shard takes/u);
  });
});

describe("readResults", () => {
  let directory: string | undefined;
  afterEach(() => {
    if (directory !== undefined) removeFolderSync(directory);
  });
  const result = (job: number): JobResult => ({ errors: [`job ${job}`], workspaces: [] });

  it("puts every shard's results back in job order", () => {
    directory = mkdtempSync(join(tmpdir(), "rerail-shards-"));
    writeResults(directory, "pack", { index: 1, count: 2 }, 4, [3, 0], [result(3), result(0)]);
    writeResults(directory, "pack", { index: 2, count: 2 }, 4, [1, 2], [result(1), result(2)]);
    expect(readResults(directory, "pack", 4).map(entry => entry.errors[0])).toEqual(["job 0", "job 1", "job 2", "job 3"]);
  });

  it("refuses a job no shard ran, a job two shards ran, and results of another plan", () => {
    directory = mkdtempSync(join(tmpdir(), "rerail-shards-"));
    writeResults(directory, "pack", { index: 1, count: 2 }, 3, [0, 1], [result(0), result(1)]);
    expect(() => readResults(directory!, "pack", 3)).toThrow(/no shard .* has results for job 2 of 3/u);
    writeResults(directory, "pack", { index: 2, count: 2 }, 3, [1, 2], [result(1), result(2)]);
    expect(() => readResults(directory!, "pack", 3)).toThrow(/job 1 has results from another shard too/u);
    expect(() => readResults(directory!, "pack", 4)).toThrow(/results of a plan of 3 job\(s\); this one has 4/u);
  });
});
