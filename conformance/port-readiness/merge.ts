/**
 * `gates merge <dir>`: join each sharded tool-pack's shard reports (`<dir>/shard-<i>of<n>/<pack>--<tool>.json`) into
 * the one tool-pack report the summary reads (`<dir>/<pack>--<tool>.json`). A tool-pack whose shards are missing or
 * disagree gets no merged report, so the summary fails it as a tool-pack without a report.
 */
import { readdirSync, readFileSync, statSync } from "node:fs";
import { writeFile } from "node:fs/promises";
import { join } from "node:path";

import { byText } from "./goldens.js";
import { renderMarkdown } from "./report.js";
import { REPORT_FORMAT, summarizeRules, unusedGapLine, type GatesReport } from "./run.js";
import { SHARD_DIRECTORY, shardName, shardOf } from "./shard.js";

/** A tool-pack report's file name, as the summary reads it. */
const REPORT_FILE = /^(?<name>[a-z0-9-]+--[A-Za-z0-9._-]+)\.json$/u;

const same = (a: unknown, b: unknown) => JSON.stringify(a) === JSON.stringify(b);

/**
 * The tool-pack report of every shard together. Throws when the shards are not exactly 1..n of one count, disagree on
 * what they share (engine, upstream, tool-pack checks, corpus, unmapped documents), or do not add up to the tool-pack's
 * rules.
 */
export function mergeShards(reports: GatesReport[]): GatesReport {
  if (reports.length === 0) throw new Error("no shard reports to merge");
  const first = reports[0]!;
  const label = first.toolPack;
  const count = first.shard?.count;
  if (count === undefined || reports.some((report) => report.shard === undefined)) throw new Error(`${label}: a report to merge is not a shard's`);
  const indices = reports.map((report) => report.shard!.index).sort((a, b) => a - b);
  const expected = Array.from({ length: count }, (_, index) => index + 1);
  if (reports.some((report) => report.shard!.count !== count) || !same(indices, expected)) {
    throw new Error(`${label}: shards ${indices.map((index) => `${index}/${reports.find((report) => report.shard!.index === index)!.shard!.count}`).join(", ")} are not 1/${count} to ${count}/${count}`);
  }
  for (const key of ["format", "pack", "tool", "upstream", "engine", "gates", "method", "toolPackChecks", "corpus", "unmapped", "listedWithoutDocument"] as const) {
    const differing = reports.filter((report) => !same(report[key], first[key])).map((report) => shardName(report.shard!));
    if (differing.length > 0) throw new Error(`${label}: the shards disagree on ${key}: ${differing.join(", ")} differ from ${shardName(first.shard!)}${key === "corpus" ? " (a corpus task failed to fetch or apply in one of them; see each report's corpus.tasks)" : ""}`);
  }
  if (reports.some((report) => report.shard!.rules !== first.shard!.rules)) throw new Error(`${label}: the shards disagree on how many rules the tool-pack has`);
  const rules = reports.flatMap((report) => report.rules).sort((a, b) => byText(a.file, b.file));
  if (new Set(rules.map((rule) => rule.file)).size !== rules.length) throw new Error(`${label}: a rule is in two shards`);
  for (const report of reports) {
    const stray = report.rules.find((rule) => shardOf(rule.id, count) !== report.shard!.index);
    if (stray !== undefined) throw new Error(`${label}: ${stray.id} is in ${shardName(report.shard!)}, not in shard ${shardOf(stray.id, count)}`);
  }
  if (rules.length !== first.shard!.rules) throw new Error(`${label}: the shards judged ${rules.length} of the tool-pack's ${first.shard!.rules} rules`);

  // A known gap is used across shards: one that excused nothing in one shard may excuse mismatches in another.
  const knownGaps = first.knownGaps.map((gap) => ({ ...gap, excused: reports.reduce((sum, report) => sum + (report.knownGaps.find((entry) => entry.id === gap.id)?.excused ?? 0), 0) }));
  const unused = new Set(first.knownGaps.map((gap) => unusedGapLine(gap.id)));
  const integrity = new Set(reports.flatMap((report) => report.integrity).filter((line) => !unused.has(line)));
  for (const gap of knownGaps) if (gap.proven && gap.excused === 0) integrity.add(unusedGapLine(gap.id));
  const gapCandidates = new Map(reports.flatMap((report) => report.gapCandidates).map((candidate) => [`${candidate.gate}\0${candidate.file}`, candidate]));

  const { shard: _shard, ...shared } = first;
  return {
    ...shared,
    integrity: [...integrity].sort(byText),
    knownGaps,
    gapCandidates: [...gapCandidates.values()].sort((a, b) => byText(a.gate, b.gate) || byText(a.file, b.file)),
    cache: { rules: reports.reduce((sum, entry) => sum + entry.cache.rules, 0), hits: reports.reduce((sum, entry) => sum + entry.cache.hits, 0) },
    summary: summarizeRules(rules),
    rules,
  };
}

/** Each sharded tool-pack under `directory`: its report name and every shard's report, by shard directory. */
export function readShards(directory: string): Map<string, GatesReport[]> {
  const found = new Map<string, GatesReport[]>();
  const shards = readdirSync(directory).filter((entry) => SHARD_DIRECTORY.test(entry) && statSync(join(directory, entry)).isDirectory()).sort(byText);
  for (const shard of shards) {
    for (const file of readdirSync(join(directory, shard)).sort(byText)) {
      const name = REPORT_FILE.exec(file)?.groups?.name;
      if (name === undefined) continue;
      const report = JSON.parse(readFileSync(join(directory, shard, file), "utf8")) as GatesReport;
      if (report.format !== REPORT_FORMAT) continue;
      found.set(name, [...(found.get(name) ?? []), report]);
    }
  }
  return found;
}

export interface MergeResult {
  /** Merged tool-pack reports written to the directory. */
  merged: GatesReport[];
  /** Why a tool-pack's shards could not be merged; that tool-pack gets no report. */
  problems: string[];
}

/** Merge every sharded tool-pack under `directory` into `<name>.json` and `<name>.md` there. */
export async function mergeDirectory(directory: string): Promise<MergeResult> {
  const result: MergeResult = { merged: [], problems: [] };
  for (const [name, reports] of readShards(directory)) {
    let report: GatesReport;
    try {
      report = mergeShards(reports);
    } catch (error) {
      result.problems.push(error instanceof Error ? error.message : String(error));
      continue;
    }
    await writeFile(join(directory, `${name}.json`), `${JSON.stringify(report, null, 2)}\n`);
    await writeFile(join(directory, `${name}.md`), renderMarkdown(report));
    result.merged.push(report);
  }
  return result;
}
