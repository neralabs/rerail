import assert from "node:assert/strict";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { afterEach, describe, it } from "vitest";

import { resultPath } from "../../../conformance/gates/cache.js";
import { serializeGoldens } from "../../../conformance/gates/goldens.js";
import { mergeDirectory, mergeShards } from "../../../conformance/gates/merge.js";
import { summarizeRules, unusedGapLine, type GatesReport, type RuleReport } from "../../../conformance/gates/run.js";
import { parseShard, shardName, shardOf, type Shard } from "../../../conformance/gates/shard.js";
import { readReports } from "../../../conformance/gates/summary.js";

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
    "matches-real-code": { checks: [], eligible: 0, positives: 0, cases: [], disagreementsElsewhere: { count: 0, first: [] } },
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

/** A shard's report of the rules of `ids` that fall in `shard`, out of `total` rules. */
function shardReport(shard: Shard, ids: string[], options: { total?: number; excused?: number; integrity?: string[]; failing?: string[] } = {}): GatesReport {
  const rules = ids.filter((id) => shardOf(id, shard.count) === shard.index).map((id) => rule(id, id.startsWith("c-") ? "candidate" : "shipped", options.failing?.includes(id)));
  return {
    format: "rerail.rule-gates/v1", pack: "demo", tool: "tool", toolPack: "demo/tool",
    upstream: { name: "", repository: "", commit: "", version: "", license: "", recommended_configs: [], runtime: "", identity: {} },
    engine: { sha256: "e" }, gates: [], method: {} as GatesReport["method"], toolPackChecks: { pass: true, checks: [] },
    integrity: options.integrity ?? [], knownGaps: [{ id: "gap", proven: true, kind: "valid-source", excused: options.excused ?? 0 }],
    gapCandidates: [{ gate: "matches-real-code", file: "t/a.ts", engine: "parse", verdict: "valid-source", parsers: [] }],
    corpus: { tasks: [], files: 3, excluded: 0 }, unmapped: [], listedWithoutDocument: 0,
    cache: { rules: rules.length, hits: 1 }, uncommittedPromotions: rules.length > 0 ? 1 : 0,
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
    const { report, goldens } = mergeShards(parts.map((part) => ({ report: part, goldens: null })));
    assert.equal(report.shard, undefined);
    assert.deepEqual(new Set(report.rules.map((entry) => entry.id)), new Set(IDS));
    assert.deepEqual(report.rules.map((entry) => entry.file), report.rules.map((entry) => entry.file).sort());
    assert.deepEqual(report.summary, summarizeRules(report.rules));
    assert.deepEqual(report.summary.shipped, { rules: 8, pass: 7 });
    assert.deepEqual(report.summary.candidates, { rules: 3, pass: 3 });
    assert.deepEqual(report.cache, { rules: 11, hits: 3 });
    assert.equal(report.uncommittedPromotions, parts.filter((part) => part.rules.length > 0).length);
    assert.equal(report.gapCandidates.length, 1);
    assert.equal(goldens, null);
  });

  it("a known gap counts as used when any shard used it", () => {
    const unused = unusedGapLine("gap");
    const parts = shards(2, { integrity: [unused, "other"] });
    parts[1] = { ...parts[1]!, knownGaps: [{ id: "gap", proven: true, kind: "valid-source", excused: 2 }], integrity: ["other"] };
    const { report } = mergeShards(parts.map((part) => ({ report: part, goldens: null })));
    assert.deepEqual(report.knownGaps, [{ id: "gap", proven: true, kind: "valid-source", excused: 2 }]);
    assert.deepEqual(report.integrity, ["other"]);
    const unusedEverywhere = mergeShards(shards(2, { integrity: [unused] }).map((part) => ({ report: part, goldens: null })));
    assert.deepEqual(unusedEverywhere.report.integrity, [unused]);
  });

  it("the goldens are every shard's promotions together", () => {
    const committed = { task: "t", path: "x.ts", sha256: "1".repeat(64) };
    const one = serializeGoldens(new Map([["a", [committed, { task: "t", path: "y.ts", sha256: "2".repeat(64) }]]]));
    const two = serializeGoldens(new Map([["a", [committed]], ["b", [{ task: "t", path: "z.ts", sha256: "3".repeat(64) }]]]));
    const parts = shards(2);
    const { goldens } = mergeShards([{ report: parts[0]!, goldens: one }, { report: parts[1]!, goldens: two }]);
    assert.equal(goldens, serializeGoldens(new Map([
      ["a", [committed, { task: "t", path: "y.ts", sha256: "2".repeat(64) }]],
      ["b", [{ task: "t", path: "z.ts", sha256: "3".repeat(64) }]],
    ])));
  });

  it("refuses shards that are missing, of another count, misplaced, short of the tool-pack's rules or disagreeing", () => {
    const parts = shards(3);
    const merge = (reports: GatesReport[]) => mergeShards(reports.map((report) => ({ report, goldens: null })));
    assert.throws(() => merge([parts[0]!, parts[2]!]), /are not 1\/3 to 3\/3/u);
    assert.throws(() => merge([...parts, shardReport({ index: 2, count: 4 }, IDS)]), /are not 1\/3 to 3\/3/u);
    assert.throws(() => merge(parts.map((part) => ({ ...part, shard: { ...part.shard!, rules: 12 } }))), /judged 11 of the tool-pack's 12 rules/u);
    const moved = parts.map((part) => ({ ...part, rules: [...part.rules] }));
    const from = moved.findIndex((part) => part.rules.length > 0);
    const to = (from + 1) % moved.length;
    moved[to]!.rules.push(moved[from]!.rules.pop()!);
    assert.throws(() => merge(moved), /is in shard-\d+of3, not in shard \d+/u);
    assert.throws(() => merge([parts[0]!, { ...parts[1]!, engine: { sha256: "other" } }, parts[2]!]), /disagree on engine: shard-2of3 differ from shard-1of3/u);
    assert.throws(() => merge([parts[0]!, parts[1]!, { ...parts[2]!, corpus: { tasks: [], files: 2, excluded: 0 } }]), /disagree on corpus: shard-3of3 differ.*failed to fetch or apply/u);
    const { shard: _shard, ...whole } = parts[0]!;
    assert.throws(() => merge([whole as GatesReport, parts[1]!, parts[2]!]), /not a shard's/u);
  });

  it("merge writes the tool-pack report the summary reads, and the summary never reads a shard's", async () => {
    const directory = mkdtempSync(join(tmpdir(), "gates-merge-"));
    temporary.push(directory);
    for (const part of shards(2)) {
      mkdirSync(join(directory, shardName(part.shard!)), { recursive: true });
      writeFileSync(join(directory, shardName(part.shard!), "demo--tool.json"), JSON.stringify(part));
    }
    assert.deepEqual(readReports(directory), []);
    const result = await mergeDirectory(directory, directory, false);
    assert.deepEqual(result.problems, []);
    assert.deepEqual(readReports(directory).map((report) => [report.toolPack, report.rules.length]), [["demo/tool", 11]]);
    assert.ok(existsSync(join(directory, "demo--tool.md")));
    assert.equal(existsSync(join(directory, "demo--tool.goldens.tsv")), false);

    // A missing shard leaves no tool-pack report, and says why.
    rmSync(join(directory, "demo--tool.json"));
    rmSync(join(directory, "shard-2of2"), { recursive: true });
    const short = await mergeDirectory(directory, directory, false);
    assert.equal(short.merged.length, 0);
    assert.match(short.problems[0]!, /demo\/tool: shards 1\/2 are not 1\/2 to 2\/2/u);
    assert.equal(existsSync(join(directory, "demo--tool.json")), false);
    assert.match(readFileSync(join(directory, "shard-1of2", "demo--tool.json"), "utf8"), /"index": ?1/u);
  });
});
