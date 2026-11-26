/**
 * How CI's unit shards weigh a test file, split a run's files between them, and how many shards a run needs.
 *
 * The time-balanced sequencer splits each shard's files by these weights, and CI's select-tests job
 * (scripts/check/ci/test-selection.mjs) counts a selection's shards by the same weights, so the two cannot disagree on
 * what a file costs. It imports nothing at run time: that script is plain JavaScript, which Node runs with this file's
 * types stripped, and Node would not find a `.js` path TypeScript resolves to a `.ts` file.
 */

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

/** A file's weight by these times: its recorded median, or the median file's for a file they do not name. */
export function weigher(files: Readonly<Record<string, number>>): (path: string) => number {
  const fallback = defaultWeight(files);
  return (path) => files[path] ?? fallback;
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
  const weight = weigher(times.files);
  const weighted = paths.map((path) => ({ path, weight: weight(path) }));
  weighted.sort((a, b) => b.weight - a.weight || (a.path < b.path ? -1 : a.path > b.path ? 1 : 0));
  const loads = Array.from({ length: count }, (_, shard) => (shard === 0 ? times.firstShardExtraMs : 0));
  const taken = new Set<number>();
  const shards = new Map<string, number>();
  for (const { path, weight } of weighted) {
    const lightest = lightestShard(loads, taken);
    taken.add(lightest);
    loads[lightest]! += weight;
    shards.set(path, lightest);
  }
  return shards;
}

/** The shard holding the least so far, the lowest on a tie, among the shards still without a file while any is. */
function lightestShard(loads: readonly number[], taken: ReadonlySet<number>): number {
  const open = loads.map((_, shard) => shard).filter((shard) => taken.size >= loads.length || !taken.has(shard));
  return open.reduce((lightest, shard) => (loads[shard]! < loads[lightest]! ? shard : lightest));
}

/** The time a run of `paths` holds by these times: each file's weight, and the first shard's step. */
export function runMs(paths: readonly string[], times: PlatformTimes): number {
  const weight = weigher(times.files);
  return paths.reduce((sum, path) => sum + weight(path), times.firstShardExtraMs);
}

/**
 * How many of `total` shards a run of `paths` needs so that, split evenly by time, no shard holds more than a shard of
 * a run of every file the times name holds: `total` times the run's share of that whole run's time, rounded up, at
 * least one. Every shard when the times name no file, since they then say nothing of what a file takes.
 */
export function shardsByTime(paths: readonly string[], times: PlatformTimes, total: number): number {
  const named = Object.keys(times.files);
  if (named.length === 0) return total;
  return Math.min(total, Math.max(1, Math.ceil((total * runMs(paths, times)) / runMs(named, times))));
}
