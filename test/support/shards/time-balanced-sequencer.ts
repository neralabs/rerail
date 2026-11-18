/**
 * Splits the unit suite into CI's shards by how long each test file takes, not by how many files each shard holds.
 * vitest's own `--shard` sorts the files by a hash of their path and gives each shard an equal count, so one shard
 * could hold twice another's test time, and CI result waited for the slowest. vitest.config.ts uses this sequencer.
 *
 * The times are the medians of CI's own runs, per platform, in `unit-test-times.generated.ts`
 * (`scripts/generate/tests/unit-test-times.mjs` writes it from CI's logs; scripts/generate/README.md has the command).
 * Each shard computes the same split from the same files and times: the files, longest first, each to the shard that
 * holds the least time so far (longest processing time first). A file the times do not name yet, a new one, counts as
 * the median file. Stale times only make the split less even; every file still runs in exactly one shard.
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
import { UNIT_TEST_TIMES } from "./unit-test-times.generated.js";

/** One platform's times: each test file's median milliseconds, and what the first shard's extra step adds. */
export interface PlatformTimes {
  /** The step the first shard runs after the suite, as test-file milliseconds. */
  readonly firstShardExtraMs: number;
  /** Each test file's median milliseconds, by its path from the repository root with `/` between folders. */
  readonly files: Readonly<Record<string, number>>;
}

/** The times file: one entry per platform CI runs the suite on, keyed as `process.platform` names it. */
export type TestTimes = Readonly<Record<string, PlatformTimes>>;

/** The platform whose times a platform without its own (macOS) uses. */
const FALLBACK_PLATFORM = "linux";

/** What a file the times do not name counts as: the median of the files they do. */
export function defaultWeight(files: Readonly<Record<string, number>>): number {
  const times = Object.values(files).sort((a, b) => a - b);
  if (times.length === 0) return 1;
  const middle = Math.floor(times.length / 2);
  return times.length % 2 === 1 ? times[middle]! : (times[middle - 1]! + times[middle]!) / 2;
}

/** The times this platform's runs use. */
export function platformTimes(times: TestTimes, platform: string): PlatformTimes {
  return times[platform] ?? times[FALLBACK_PLATFORM] ?? { firstShardExtraMs: 0, files: {} };
}

/**
 * Which shard (0-based) runs each path, longest processing time first: paths by weight, longest first and then by
 * path, each to the shard holding the least so far, the lowest such shard on a tie. The first shard starts holding
 * `firstShardExtraMs`. The `count` longest paths go one to each shard, so no shard is left without a file while there
 * are enough (vitest fails a run with none), even when the extra time outweighs the paths, as in a filtered run.
 * Deterministic, so every shard computes the same split.
 */
export function assignShards(paths: readonly string[], times: PlatformTimes, count: number): Map<string, number> {
  const fallback = defaultWeight(times.files);
  const weighted = paths.map((path) => ({ path, weight: times.files[path] ?? fallback }));
  weighted.sort((a, b) => b.weight - a.weight || (a.path < b.path ? -1 : a.path > b.path ? 1 : 0));
  const loads = Array.from({ length: count }, (_, shard) => (shard === 0 ? times.firstShardExtraMs : 0));
  const taken = new Set<number>();
  const shards = new Map<string, number>();
  for (const { path, weight } of weighted) {
    let lightest = -1;
    for (let shard = 0; shard < count; shard += 1) {
      if (taken.size < count && taken.has(shard)) continue;
      if (lightest === -1 || loads[shard]! < loads[lightest]!) lightest = shard;
    }
    taken.add(lightest);
    loads[lightest]! += weight;
    shards.set(path, lightest);
  }
  return shards;
}

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
    const { files: times } = this.#times;
    const fallback = defaultWeight(times);
    const order = new Map(
      files.map((spec) => {
        const result = this.ctx.cache.getTestSpecificationResult(spec);
        return [spec, { failed: result?.failed ?? false, weight: result?.duration ?? times[this.pathOf(spec)] ?? fallback }];
      }),
    );
    return [...files].sort((a, b) => {
      const [first, second] = [order.get(a)!, order.get(b)!];
      return Number(second.failed) - Number(first.failed) || second.weight - first.weight;
    });
  }
}
