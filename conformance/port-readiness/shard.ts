import { createHash } from "node:crypto";

/**
 * One shard of a tool-pack: `--shard <index>/<count>` judges only the rules whose sha256(id) mod count is index - 1,
 * so a rule keeps its shard when other rules are added or removed. `gates merge` joins the shards' reports.
 */
export interface Shard {
  /** 1-based. */
  index: number;
  count: number;
}

/** `<index>/<count>`, both positive integers, index at most count. */
export function parseShard(text: string): Shard {
  const match = /^(?<index>[1-9]\d*)\/(?<count>[1-9]\d*)$/u.exec(text);
  const index = Number(match?.groups?.index);
  const count = Number(match?.groups?.count);
  if (match === null || index > count) throw new Error(`--shard takes <index>/<count>, 1 <= index <= count (got ${JSON.stringify(text)})`);
  return { index, count };
}

/** The 1-based shard of `count` that judges the rule with this document id. */
export function shardOf(id: string, count: number): number {
  return Number(BigInt(`0x${createHash("sha256").update(id).digest("hex")}`) % BigInt(count)) + 1;
}

/** The shard's directory name, under the report directory and under the results cache: `shard-<index>of<count>`. */
export function shardName(shard: Shard): string {
  return `shard-${shard.index}of${shard.count}`;
}

export const SHARD_DIRECTORY = /^shard-(?<index>[1-9]\d*)of(?<count>[1-9]\d*)$/u;
