/**
 * `gates merge <dir>`: join each sharded tool-pack's shard reports (`<dir>/shard-<i>of<n>/<pack>--<tool>.json`) into
 * the one tool-pack report the summary reads (`<dir>/<pack>--<tool>.json`). A tool-pack whose shards are missing or
 * disagree gets no merged report, so the summary fails it as a tool-pack without a report.
 */
import { readdirSync, readFileSync, statSync } from "node:fs";
import { writeFile } from "node:fs/promises";
import { join } from "node:path";

import { resolveNativeBindingPath } from "../../src/engine/index.js";
import { nativeBinarySha256 } from "./engine.js";
import { byText } from "./goldens.js";
import { renderMarkdown } from "./report.js";
import { REPORT_FORMAT, summarizeRules, unusedGapLine, type GatesReport } from "./run.js";
import { SHARD_DIRECTORY, shardName, shardOf } from "./shard.js";

/** A tool-pack report's file name, as the summary reads it. */
const REPORT_FILE = /^(?<name>[a-z0-9-]+--[A-Za-z0-9._-]+)\.json$/u;

const same = (a: unknown, b: unknown) => JSON.stringify(a) === JSON.stringify(b);

/**
 * The tool-pack report of every shard together. Throws when the shards are not exactly 1..n of one count, were not all
 * made by one native binary (and, given the checkout's sha256 as `binary`, by that one), disagree on what they share
 * (engine, upstream, tool-pack checks, corpus, unmapped documents), or do not add up to the tool-pack's rules.
 *
 * The binary is checked on its own, before the engine: no rule's key covers it (engine.ts), so shards run on a binary
 * older than a source edit and shards run after the rebuild agree on everything else. Without a binary in the
 * checkout, as in CI's report job, the shards answer to each other's alone; CI's all unpack the build job's.
 */
export function mergeShards(reports: GatesReport[], binary?: string): GatesReport {
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
  // A report written before reports recorded the binary has none, which matches no binary.
  const otherBinary = reports.filter((report) => report.engine.binary !== first.engine.binary).map((report) => shardName(report.shard!));
  if (otherBinary.length > 0) throw new Error(`${label}: ${otherBinary.join(", ")} made by another native binary than ${shardName(first.shard!)}: rebuild once and rerun every shard`);
  if (binary !== undefined && first.engine.binary !== binary) {
    const all = reports.map((report) => shardName(report.shard!)).join(", ");
    throw new Error(first.engine.binary === undefined
      ? `${label}: ${all} do not say which native binary made them (written before reports recorded it): rerun every shard`
      : `${label}: ${all} made by another native binary than this checkout's: rerun every shard on it`);
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
    cache: {
      rules: reports.reduce((sum, entry) => sum + entry.cache.rules, 0),
      hits: reports.reduce((sum, entry) => sum + entry.cache.hits, 0),
      // Only a refresh compares results (run.ts): shards that compared none merge to a report that says nothing of it.
      ...(reports.every((entry) => entry.cache.compared === undefined) ? {} : {
        compared: reports.reduce((sum, entry) => sum + (entry.cache.compared ?? 0), 0),
        differing: reports.flatMap((entry) => entry.cache.differing ?? []).sort((a, b) => byText(a.file, b.file)),
      }),
    },
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

/**
 * The sha256 of this checkout's native binary, the one the gates would load (`configured`, by default
 * RERAIL_ENGINE_PATH, or else native/'s), or undefined when there is none, as in CI's report job. A RERAIL_ENGINE_PATH
 * that names no file is an error. `resolve` finds the binary; a test passes its own for a checkout with none in native/.
 */
export function checkoutBinary(configured: string | undefined = process.env.RERAIL_ENGINE_PATH, resolve = resolveNativeBindingPath): string | undefined {
  let path: string;
  try {
    path = resolve(configured);
  } catch (error) {
    // Without RERAIL_ENGINE_PATH, the one way to fail is a checkout with no binary in native/.
    if (configured === undefined) return undefined;
    throw error;
  }
  return nativeBinarySha256(path);
}

/**
 * Merge every sharded tool-pack under `directory` into `<name>.json` and `<name>.md` there. `binary`, the sha256 of the
 * checkout's native binary when it has one (`checkoutBinary`), is the binary every shard must have been made by.
 */
export async function mergeDirectory(directory: string, binary?: string): Promise<MergeResult> {
  const result: MergeResult = { merged: [], problems: [] };
  for (const [name, reports] of readShards(directory)) {
    let report: GatesReport;
    try {
      report = mergeShards(reports, binary);
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
