/**
 * Splits the unit suite into CI's shards by how long each test file takes, not by how many files each shard holds.
 * vitest's own `--shard` sorts the files by a hash of their path and gives each shard an equal count, so one shard
 * could hold twice another's test time, and CI result waited for the slowest. vitest.config.ts uses this sequencer.
 *
 * The times are the medians of CI's own runs, per platform, in `unit-test-times.generated.ts`
 * (`scripts/generate/tests/unit-test-times.mjs` writes it from CI's logs; scripts/generate/README.md has the command).
 * Each shard computes the same split from the same files and times: the files, longest first, each to the shard that
 * holds the least time so far (longest processing time first). A file the times do not name yet, a new one, counts as
 * the median file. Stale times only make the split less even; every file still runs in exactly one shard. The split
 * and the weights are in shard-split.ts, which CI's select-tests job also reads to count a selection's shards.
 *
 * The first shard of each platform also runs a step after the suite (the lint config coverage gate on Linux, the
 * Claude Code hook integration test on Windows), so it starts with that step's time, as test-file time, already held.
 *
 * Within a shard, and in a run without shards, a file that failed in the last run here starts first, then the rest
 * longest first, so the slowest file does not start last. vitest's own order does the same by the times in its cache,
 * but CI keeps no cache, so there it orders by file size. A file the cache has a time for keeps that time here.
 */
import { relative } from "node:path";
import { BaseSequencer, type TestSpecification } from "vitest/node";
import { assignShards, platformTimes, weigher } from "./shard-split.js";
import { UNIT_TEST_TIMES } from "./unit-test-times.generated.js";

// The times file names its type from here.
export type { PlatformTimes, TestTimes } from "./shard-split.js";

export class TimeBalancedSequencer extends BaseSequencer {
  readonly #times = platformTimes(UNIT_TEST_TIMES, process.platform);

  /** A test file's path from the repository root, with `/` between folders, as the times file names it. */
  private pathOf(spec: TestSpecification): string {
    return relative(this.ctx.config.root, spec.moduleId).replaceAll("\\", "/");
  }

  override async shard(files: TestSpecification[]): Promise<TestSpecification[]> {
    const { index, count } = this.ctx.config.shard!;
    const shards = assignShards([...new Set(files.map((spec) => this.pathOf(spec)))], this.#times, count);
    return files.filter((spec) => shards.get(this.pathOf(spec)) === index - 1);
  }

  override async sort(files: TestSpecification[]): Promise<TestSpecification[]> {
    const weight = weigher(this.#times.files);
    const order = new Map(
      files.map((spec) => {
        const result = this.ctx.cache.getTestSpecificationResult(spec);
        return [spec, { failed: result?.failed ?? false, weight: result?.duration ?? weight(this.pathOf(spec)) }];
      }),
    );
    return [...files].sort((a, b) => {
      const [first, second] = [order.get(a)!, order.get(b)!];
      return Number(second.failed) - Number(first.failed) || second.weight - first.weight;
    });
  }
}
