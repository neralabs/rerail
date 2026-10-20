import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { afterEach, describe, it } from "vitest";

import { resolveNativeBindingPath } from "../../../src/engine/index.js";
import { resultPath } from "../../../conformance/port-readiness/cache.js";
import { checkoutBinary, mergeDirectory, mergeShards } from "../../../conformance/port-readiness/merge.js";
import { blockingFailures, summarizeRules, unusedGapLine, type GatesReport, type RuleReport, type VerdictDrift } from "../../../conformance/port-readiness/run.js";
import { parseShard, shardName, shardOf, type Shard } from "../../../conformance/port-readiness/shard.js";
import { readReports } from "../../../conformance/port-readiness/summary.js";

const temporary: string[] = [];
afterEach(() => {
  for (const directory of temporary.splice(0)) rmSync(directory, { recursive: true, force: true });
});

function passingGates(): RuleReport["gates"] {
  const ok = { pass: true, checks: [] };
  return {
    "upstream-status": { checks: [] },
    loads: ok,
    "not-overfit": { ...ok, flagged: [], words: {}, uncheckedRegexes: [] },
    compact: { ...ok, metrics: null },
    "matches-upstream-tests": { ...ok, totals: null, cases: {}, differences: [], goldens: { count: 1, differing: { count: 0, first: [] }, engineGaps: { count: 0, first: [] } } },
    "passes-conformance": ok,
  };
}

function rule(id: string, location: RuleReport["location"] = "shipped", failing = false): RuleReport {
  const gates = passingGates();
  if (failing) gates.loads = { pass: false, checks: [{ id: "schema", ok: false, detail: "schema failed" }] };
  return {
    id, file: `packs/demo/${location === "shipped" ? "rules" : "candidates/rules"}/${id}.json`, location, upstreamRule: id, sha256: "0", bytes: 1,
    policyDigest: null, pass: !failing, stoppedAt: failing ? "loads" : null, blockedBy: null, defaultOn: true, defaultOnReason: "", on: location === "shipped" ? true : null, gates,
  };
}

/** A shard's report of the rules of `ids` that fall in `shard`, out of `total` rules, made by the binary `binary`. */
function shardReport(shard: Shard, ids: string[], options: { total?: number; excused?: number; integrity?: string[]; failing?: string[]; binary?: string } = {}): GatesReport {
  const rules = ids.filter((id) => shardOf(id, shard.count) === shard.index).map((id) => rule(id, id.startsWith("c-") ? "candidate" : "shipped", options.failing?.includes(id)));
  return {
    format: "rerail.port-readiness/v1", pack: "demo", tool: "tool", toolPack: "demo/tool",
    upstream: { name: "", repository: "", commit: "", version: "", license: "", recommended_configs: [], runtime: "", identity: {} },
    engine: { sha256: "e", binary: options.binary ?? "b" }, gates: [], method: {} as GatesReport["method"], toolPackChecks: { pass: true, checks: [] },
    integrity: options.integrity ?? [], knownGaps: [{ id: "gap", proven: true, kind: "valid-source", excused: options.excused ?? 0 }],
    gapCandidates: [{ gate: "matches-upstream-tests", file: "t/a.ts", engine: "parse", verdict: "valid-source", parsers: [] }],
    corpus: { tasks: [], files: 3, excluded: 0 }, unmapped: [], listedWithoutDocument: 0,
    cache: { rules: rules.length, hits: 1 },
    shard: { ...shard, rules: options.total ?? ids.length },
    summary: summarizeRules(rules),
    rules,
  };
}

const IDS = ["a", "b", "c", "d", "e", "f", "g", "h", "c-1", "c-2", "c-3"];

describe("shards", () => {
  it("--shard takes <index>/<count> with 1 <= index <= count", () => {
    assert.deepEqual(parseShard("2/4"), { index: 2, count: 4 });
    assert.deepEqual(parseShard("1/1"), { index: 1, count: 1 });
    for (const bad of ["0/4", "5/4", "1/0", "2", "a/b", "01/4", " 1/4"]) assert.throws(() => parseShard(bad), /--shard takes/u);
    assert.equal(shardName({ index: 3, count: 8 }), "shard-3of8");
  });

  it("every rule falls in exactly one shard, by its id alone, so adding rules moves none", () => {
    for (const count of [1, 2, 3, 6]) {
      const shards = IDS.map((id) => shardOf(id, count));
      assert.ok(shards.every((index) => index >= 1 && index <= count));
      const more = [...IDS, "new-1", "new-2"].map((id) => shardOf(id, count));
      assert.deepEqual(more.slice(0, IDS.length), shards);
    }
    // sha256("a") mod 2: a fixed answer, the same on every machine.
    assert.equal(shardOf("a", 2), Number(BigInt("0xca978112ca1bbdcafac231b39a23dc4da786eff8147c4e72b9807785afee48bb") % 2n) + 1);
  });

  it("each shard caches its results in its own directory", () => {
    assert.equal(resultPath("/c", "demo", "tool", "k"), join("/c", "results", "demo", "tool", "k.json"));
    assert.equal(resultPath("/c", "demo", "tool", "k", { index: 2, count: 6 }), join("/c", "results", "demo", "tool", "shard-2of6", "k.json"));
  });
});

describe("merge", () => {
  const shards = (count: number, options: Parameters<typeof shardReport>[2] = {}) => Array.from({ length: count }, (_, index) => shardReport({ index: index + 1, count }, IDS, options));

  it("joins the shards into the report a whole run gives: every rule once, sorted, counts summed and recomputed", () => {
    const parts = shards(3, { failing: ["b"] });
    const report = mergeShards(parts);
    assert.equal(report.shard, undefined);
    assert.deepEqual(new Set(report.rules.map((entry) => entry.id)), new Set(IDS));
    assert.deepEqual(report.rules.map((entry) => entry.file), report.rules.map((entry) => entry.file).sort());
    assert.deepEqual(report.summary, summarizeRules(report.rules));
    assert.deepEqual(report.summary.shipped, { rules: 8, pass: 7 });
    assert.deepEqual(report.summary.candidates, { rules: 3, pass: 3 });
    // Only a refresh compares results with the cache: shards that compared none merge to a report that says nothing.
    assert.deepEqual(report.cache, { rules: 11, hits: 3 });
    assert.equal(report.gapCandidates.length, 1);
  });

  it("adds up the results each shard compared with the cache, and lists every difference, each of which blocks", () => {
    const drift = (id: string): VerdictDrift => ({ id, file: `packs/demo/rules/${id}.json`, location: "shipped", what: "compact", cached: "fail", fresh: "pass" });
    const [first, second] = shards(2);
    const report = mergeShards([{ ...first!, cache: { ...first!.cache, compared: 2, differing: [drift("h")] } }, { ...second!, cache: { ...second!.cache, compared: 3, differing: [drift("a")] } }]);
    assert.equal(report.cache.compared, 5);
    assert.deepEqual(report.cache.differing, [drift("a"), drift("h")]);
    assert.equal(blockingFailures(report), 2);
  });

  it("a known gap counts as used when any shard used it", () => {
    const unused = unusedGapLine("gap");
    const parts = shards(2, { integrity: [unused, "other"] });
    parts[1] = { ...parts[1]!, knownGaps: [{ id: "gap", proven: true, kind: "valid-source", excused: 2 }], integrity: ["other"] };
    const report = mergeShards(parts);
    assert.deepEqual(report.knownGaps, [{ id: "gap", proven: true, kind: "valid-source", excused: 2 }]);
    assert.deepEqual(report.integrity, ["other"]);
    const unusedEverywhere = mergeShards(shards(2, { integrity: [unused] }));
    assert.deepEqual(unusedEverywhere.integrity, [unused]);
  });

  it("refuses shards that are missing, of another count, misplaced, short of the tool-pack's rules or disagreeing", () => {
    const parts = shards(3);
    assert.throws(() => mergeShards([parts[0]!, parts[2]!]), /are not 1\/3 to 3\/3/u);
    assert.throws(() => mergeShards([...parts, shardReport({ index: 2, count: 4 }, IDS)]), /are not 1\/3 to 3\/3/u);
    assert.throws(() => mergeShards(parts.map((part) => ({ ...part, shard: { ...part.shard!, rules: 12 } }))), /judged 11 of the tool-pack's 12 rules/u);
    const moved = parts.map((part) => ({ ...part, rules: [...part.rules] }));
    const from = moved.findIndex((part) => part.rules.length > 0);
    const to = (from + 1) % moved.length;
    moved[to]!.rules.push(moved[from]!.rules.pop()!);
    assert.throws(() => mergeShards(moved), /is in shard-\d+of3, not in shard \d+/u);
    assert.throws(() => mergeShards([parts[0]!, { ...parts[1]!, engine: { sha256: "other", binary: "b" } }, parts[2]!]), /disagree on engine: shard-2of3 differ from shard-1of3/u);
    assert.throws(() => mergeShards([parts[0]!, parts[1]!, { ...parts[2]!, corpus: { tasks: [], files: 2, excluded: 0 } }]), /disagree on corpus: shard-3of3 differ.*failed to fetch or apply/u);
    const { shard: _shard, ...whole } = parts[0]!;
    assert.throws(() => mergeShards([whole as GatesReport, parts[1]!, parts[2]!]), /not a shard's/u);
  });

  // No rule's key covers the native binary (engine.ts), so shards run on a stale one and merged after a rebuild agree
  // on everything else: each report's engine.binary is what tells them apart.
  it("refuses shards another native binary made, before it compares the engine sources", () => {
    const parts = shards(3);
    assert.throws(() => mergeShards([parts[0]!, { ...parts[1]!, engine: { sha256: "other", binary: "other" } }, parts[2]!]), /^Error: demo\/tool: shard-2of3 made by another native binary than shard-1of3: rebuild once and rerun every shard$/u);
    assert.throws(() => mergeShards([parts[0]!, shardReport({ index: 2, count: 3 }, IDS, { binary: "c" }), shardReport({ index: 3, count: 3 }, IDS, { binary: "c" })]), /: shard-2of3, shard-3of3 made by another native binary than shard-1of3:/u);
    // A report written before reports recorded the binary says nothing that matches one that does.
    const unrecorded = { ...parts[2]!, engine: { sha256: "e" } };
    assert.throws(() => mergeShards([parts[0]!, parts[1]!, unrecorded]), /: shard-3of3 made by another native binary than shard-1of3:/u);
  });

  it("with a native binary in the checkout, refuses shards it did not make or that do not say which made them", () => {
    const parts = shards(2);
    assert.equal(mergeShards(parts, "b").engine.binary, "b");
    assert.throws(() => mergeShards(parts, "c"), /^Error: demo\/tool: shard-1of2, shard-2of2 made by another native binary than this checkout's: rerun every shard on it$/u);
    const unrecorded = parts.map((part) => ({ ...part, engine: { sha256: "e" } }));
    assert.throws(() => mergeShards(unrecorded, "b"), /^Error: demo\/tool: shard-1of2, shard-2of2 do not say which native binary made them \(written before reports recorded it\): rerun every shard$/u);
    // Shards that disagree with each other are told so first, whatever the checkout's binary.
    assert.throws(() => mergeShards([parts[0]!, { ...parts[1]!, engine: { sha256: "e", binary: "c" } }], "c"), /: shard-2of2 made by another native binary than shard-1of2: rebuild once/u);
  });

  it("with no native binary in the checkout, as in CI's report job, merges shards that agree", () => {
    const report = mergeShards(shards(2), undefined);
    assert.deepEqual(report.engine, { sha256: "e", binary: "b" });
    assert.equal(report.rules.length, IDS.length);
  });

  it("the checkout's binary is the one RERAIL_ENGINE_PATH names, and one that names no file is an error", () => {
    const directory = mkdtempSync(join(tmpdir(), "gates-binary-"));
    temporary.push(directory);
    const binary = join(directory, "rerail_engine.node");
    writeFileSync(binary, "a build\n");
    assert.equal(checkoutBinary(binary), createHash("sha256").update("a build\n").digest("hex"));
    assert.throws(() => checkoutBinary(join(directory, "missing.node")), /RERAIL_ENGINE_PATH does not exist/u);
    assert.equal(checkoutBinary(), createHash("sha256").update(readFileSync(resolveNativeBindingPath())).digest("hex"));
  });

  it("with RERAIL_ENGINE_PATH unset and no binary in native/, as in CI's report job, the checkout has no binary", () => {
    // resolveNativeBindingPath looks in this checkout's native/, which has a binary: this answers as it does in one without.
    const unbuilt = (configured?: string): string => {
      throw new Error(configured === undefined ? "Unsupported platform or missing native addon: rerail_engine.node" : `RERAIL_ENGINE_PATH does not exist: ${configured}`);
    };
    const configured = process.env.RERAIL_ENGINE_PATH;
    delete process.env.RERAIL_ENGINE_PATH;
    try {
      assert.equal(checkoutBinary(undefined, unbuilt), undefined);
    } finally {
      if (configured !== undefined) process.env.RERAIL_ENGINE_PATH = configured;
    }
    // Only an unset RERAIL_ENGINE_PATH makes a missing binary no binary: one that names no file is still an error.
    assert.throws(() => checkoutBinary("missing.node", unbuilt), /^Error: RERAIL_ENGINE_PATH does not exist: missing\.node$/u);
  });

  it("a shard that took every result from the cache merges with one that judged: both carry the same corpus-complete", () => {
    const complete = { id: "corpus-complete", ok: true, detail: "all 3 recorded corpus file(s) were fetched with their recorded sha256" };
    const withCheck = (part: GatesReport, check: typeof complete) => ({ ...part, toolPackChecks: { pass: true, checks: [check] } });
    const [judged, cached] = shards(2);
    assert.deepEqual(mergeShards([withCheck(judged!, complete), withCheck({ ...cached!, cache: { rules: cached!.rules.length, hits: cached!.rules.length } }, complete)]).toolPackChecks.checks, [complete]);
    // Before the cached check was replayed, a fully cached shard wrote its own detail and no merge took place.
    const notFetched = { ...complete, detail: "not fetched on this run: every result is cached from a run that fetched every recorded corpus file" };
    assert.throws(() => mergeShards([withCheck(judged!, complete), withCheck(cached!, notFetched)]), /disagree on toolPackChecks: shard-2of2 differ from shard-1of2/u);
  });

  it("merge writes the tool-pack report the summary reads, and the summary never reads a shard's", async () => {
    const directory = mkdtempSync(join(tmpdir(), "gates-merge-"));
    temporary.push(directory);
    for (const part of shards(2)) {
      mkdirSync(join(directory, shardName(part.shard!)), { recursive: true });
      writeFileSync(join(directory, shardName(part.shard!), "demo--tool.json"), JSON.stringify(part));
    }
    assert.deepEqual(readReports(directory), []);
    const result = await mergeDirectory(directory);
    assert.deepEqual(result.problems, []);
    assert.deepEqual(readReports(directory).map((report) => [report.toolPack, report.rules.length]), [["demo/tool", 11]]);
    assert.match(readFileSync(join(directory, "demo--tool.md"), "utf8"), /^- Engine: this checkout's, identity `e`, native binary `b`$/mu);

    // Shards another binary made than the checkout's leave no tool-pack report either.
    rmSync(join(directory, "demo--tool.json"));
    const rebuilt = await mergeDirectory(directory, "c");
    assert.equal(rebuilt.merged.length, 0);
    assert.match(rebuilt.problems[0]!, /demo\/tool: shard-1of2, shard-2of2 made by another native binary than this checkout's/u);
    assert.equal(existsSync(join(directory, "demo--tool.json")), false);

    // A missing shard leaves no tool-pack report, and says why.
    rmSync(join(directory, "shard-2of2"), { recursive: true });
    const short = await mergeDirectory(directory);
    assert.equal(short.merged.length, 0);
    assert.match(short.problems[0]!, /demo\/tool: shards 1\/2 are not 1\/2 to 2\/2/u);
    assert.equal(existsSync(join(directory, "demo--tool.json")), false);
    assert.match(readFileSync(join(directory, "shard-1of2", "demo--tool.json"), "utf8"), /"index": ?1/u);
  });
});
