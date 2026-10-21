import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { afterEach, describe, it } from "vitest";

import { CACHE_FORMAT, readCached, resultPath, ruleKey, toolPackKey, writeCached, type SharedKeyInputs } from "../../../conformance/port-readiness/cache.js";
import { engineReadsGoldensCheck } from "../../../conformance/port-readiness/checks/matches-upstream-tests.js";
import { sparseDirectoryPattern } from "../../../conformance/port-readiness/recorded-adapter.js";
import type { CommandOptions, CommandResult, CommandRunner } from "../../../conformance/port-readiness/process.js";
import { renderMarkdown } from "../../../conformance/port-readiness/report.js";
import { blockingFailures, isCachedPack, isCachedRule, packDocuments, type CachedRule, type GatesReport, type RuleReport } from "../../../conformance/port-readiness/run.js";
import { CONFORMANCE_COLUMN, parsePlanned, READY_COLUMN, renderSummary, summarize } from "../../../conformance/port-readiness/summary.js";
import { ungatedChecks } from "../../../conformance/port-readiness/ungated.js";
import { CONFORMANCE_REPORT_FORMAT, type ConformanceReport, type ConformanceRuleReport } from "../../../conformance/port-readiness/conformance-gates.js";
import { BLOCKING_GATES, type BlockingGateId } from "../../../conformance/port-readiness/types.js";

const temporary: string[] = [];
afterEach(() => {
  for (const directory of temporary.splice(0)) rmSync(directory, { recursive: true, force: true });
});

function tempRoot(): string {
  const root = mkdtempSync(join(tmpdir(), "rerail-gates-"));
  temporary.push(root);
  return root;
}

function write(root: string, path: string, value: unknown): void {
  mkdirSync(join(root, path, ".."), { recursive: true });
  writeFileSync(join(root, path), typeof value === "string" ? value : `${JSON.stringify(value, null, 2)}\n`);
}

/** A runner answering git commands from a table; anything else fails. */
class FakeRunner implements CommandRunner {
  readonly calls: string[] = [];
  constructor(private readonly answers: Record<string, Partial<CommandResult>>) {}
  async run(argv: readonly string[], _options?: CommandOptions): Promise<CommandResult> {
    const command = argv.join(" ");
    this.calls.push(command);
    const answer = Object.entries(this.answers).find(([prefix]) => command.startsWith(prefix))?.[1];
    return { exitCode: answer?.exitCode ?? 1, stdout: answer?.stdout ?? "", stderr: answer?.stderr ?? "no answer" };
  }
}

function passingGates(): RuleReport["gates"] {
  const ok = { pass: true, checks: [] };
  return {
    "upstream-status": { checks: [] },
    loads: ok,
    "not-overfit": { ...ok, flagged: [], words: {}, uncheckedRegexes: [] },
    compact: { ...ok, metrics: null },
    "matches-upstream-tests": { ...ok, totals: null, cases: {}, differences: [], goldens: { count: 0, differing: { count: 0, first: [] }, engineGaps: { count: 0, first: [] } } },
    "passes-conformance": ok,
  };
}

function rule(id: string, location: RuleReport["location"], failing: Partial<Record<BlockingGateId, string>> = {}): RuleReport {
  const gates = passingGates();
  for (const [gate, check] of Object.entries(failing) as [BlockingGateId, string][]) {
    gates[gate] = { ...gates[gate], pass: false, checks: [{ id: check, ok: false, detail: `${check} failed` }] } as never;
  }
  const stoppedAt = BLOCKING_GATES.find((gate) => failing[gate.id] !== undefined)?.id ?? null;
  return {
    id, file: `packs/python/${location === "shipped" ? "rules" : "candidates/rules"}/${id}.json`, location, upstreamRule: "X1", sha256: "0", bytes: 1,
    policyDigest: null, pass: stoppedAt === null, stoppedAt, blockedBy: null, defaultOn: true, defaultOnReason: "", on: location === "shipped" ? true : null, gates,
  };
}

function report(rules: RuleReport[], unmapped: GatesReport["unmapped"] = []): GatesReport {
  const zero = { loads: 0, "not-overfit": 0, compact: 0, "matches-upstream-tests": 0, "passes-conformance": 0 };
  return {
    format: "rerail.port-readiness/v1", pack: "python", tool: "bandit", toolPack: "python/bandit",
    upstream: { name: "", repository: "", commit: "", version: "", license: "", recommended_configs: [], runtime: "", identity: {} },
    engine: { sha256: "" }, gates: [], method: {} as GatesReport["method"], toolPackChecks: { pass: true, checks: [] }, integrity: [], knownGaps: [], gapCandidates: [],
    corpus: null, unmapped, listedWithoutDocument: 0, cache: { rules: rules.length, hits: 0 },
    summary: {
      shipped: { rules: rules.filter((r) => r.location === "shipped").length, pass: rules.filter((r) => r.location === "shipped" && r.pass).length },
      candidates: { rules: rules.filter((r) => r.location === "candidate").length, pass: rules.filter((r) => r.location === "candidate" && r.pass).length },
      stoppedAt: { shipped: { ...zero }, candidate: { ...zero } }, failedGate: { ...zero },
      goldens: { count: 0, differing: 0, engineGaps: 0 },
    },
    rules,
  };
}

const noUngated = { checks: [], notGated: [], pending: [] };

describe("location-based blocking", () => {
  it("a failing shipped rule blocks; a failing candidate does not", () => {
    assert.equal(blockingFailures(report([rule("a", "shipped"), rule("b", "candidate", { loads: "schema" })])), 0);
    assert.equal(blockingFailures(report([rule("a", "shipped", { loads: "schema" }), rule("b", "candidate")])), 1);
  });

  it("an unmapped shipped document blocks; an unmapped candidate is only reported", () => {
    assert.equal(blockingFailures(report([], [{ file: "packs/python/candidates/rules/x.json", location: "candidate" }])), 0);
    assert.equal(blockingFailures(report([], [{ file: "packs/python/rules/x.json", location: "shipped" }])), 1);
  });

  it("the summary lists an unmapped candidate once as not judged, outside the blocking rows and the funnel", () => {
    const unmapped = [{ file: "packs/python/candidates/rules/x.json", location: "candidate" as const }, { file: "packs/python/rules/y.json", location: "shipped" as const }];
    // Each tool-pack report of the pack lists the pack's unmapped documents.
    const summary = summarize([report([rule("a", "candidate")], unmapped), { ...report([], unmapped), tool: "ruff", toolPack: "python/ruff" }], noUngated, [{ pack: "python", tool: "bandit" }, { pack: "python", tool: "ruff" }]);
    assert.deepEqual(summary.unmappedCandidates, [{ pack: "python", file: "packs/python/candidates/rules/x.json" }]);
    assert.deepEqual(summary.blocking.map((row) => row.rule), ["packs/python/rules/y.json"]);
    assert.deepEqual(summary.candidateRows.map((row) => row.rule), ["a"]);
    assert.match(renderSummary(summary), /^1 candidate is not judged for port readiness: no conformance\.json entry maps it to an upstream rule, so no port gate ran \(the conformance gates judge it\)\n\n- python: `packs\/python\/candidates\/rules\/x\.json`$/mu);
  });

  it("the summary lists shipped failures as blocking and candidates in the funnel, and fails on a missing report", () => {
    const summary = summarize([report([rule("a", "shipped", { loads: "schema" }), rule("b", "candidate"), rule("c", "candidate", { loads: "json" })])], noUngated, [{ pack: "python", tool: "bandit" }, { pack: "python", tool: "ruff" }]);
    // A rule that does not load is the conformance section's blocker: no port gate can judge it.
    assert.deepEqual(summary.conformance.blocking.map((row) => [row.pack, row.rule, row.check]), [["python", "a", "schema"]]);
    assert.deepEqual(summary.blocking.map((row) => [row.pack, row.rule, row.check]), [["python/ruff", "(every rule)", "report"]]);
    assert.deepEqual(summary.promotable.map(({ toolPack, rule }) => ({ toolPack, rule })), [{ toolPack: "python/bandit", rule: "b" }]);
    assert.deepEqual(summary.notJudged, []);
    assert.equal(summary.pass, false);
  });

  const gated = [{ pack: "python", tool: "bandit" }, { pack: "python", tool: "ruff" }];

  it("a tool-pack the plan left out is listed as not judged and blocks nothing", () => {
    const summary = summarize([report([rule("a", "shipped")])], noUngated, gated, [{ pack: "python", tool: "bandit" }]);
    assert.deepEqual(summary.missingReports, []);
    assert.deepEqual(summary.notJudged, ["python/ruff"]);
    assert.deepEqual(summary.blocking, []);
    assert.equal(summary.pass, true);
    const markdown = renderSummary(summary);
    assert.match(markdown, /^\*\*In the 1 of 2 tool-packs judged: 1 of 1 shipped ported rules pass;/mu);
    assert.match(markdown, /^Not judged on this change: python\/ruff \(nothing they read changed against the base, so main's verdict stands\)\.$/mu);
    assert.doesNotMatch(renderSummary(summarize([report([rule("a", "shipped")])], noUngated, [gated[0]!])), /judged/u);
  });

  it("a planned tool-pack with no report blocks", () => {
    const summary = summarize([], noUngated, gated, [{ pack: "python", tool: "ruff" }]);
    assert.deepEqual(summary.blocking.map((row) => [row.pack, row.rule, row.check]), [["python/ruff", "(every rule)", "report"]]);
    assert.deepEqual(summary.notJudged, ["python/bandit"]);
    assert.equal(summary.pass, false);
  });

  it("a planned tool-pack that is not gated blocks: the plan is wrong", () => {
    const summary = summarize([report([rule("a", "shipped")])], noUngated, gated, [...gated.slice(0, 1), { pack: "python", tool: "flake8" }]);
    assert.deepEqual(summary.blocking.map((row) => [row.pack, row.check]), [["python/flake8", "plan"]]);
    assert.deepEqual(summary.notJudged, ["python/ruff"]);
    assert.equal(summary.pass, false);
  });

  it("--planned takes a JSON array of <pack>/<tool> strings", () => {
    assert.deepEqual(parsePlanned('["python/ruff","react/eslint-plugin-react"]'), [{ pack: "python", tool: "ruff" }, { pack: "react", tool: "eslint-plugin-react" }]);
    assert.deepEqual(parsePlanned("[]"), []);
    for (const bad of ["", "python/ruff", '"python/ruff"', "{}", '["python"]', '["python/ruff/x"]', "[1]"]) {
      assert.throws(() => parsePlanned(bad), /--planned takes a JSON array of "<pack>\/<tool>" strings/u, bad);
    }
  });

  it("a candidate a failing tool-pack check blocks is not ready to promote, and says why", () => {
    const blocked = { ...rule("b", "candidate"), pass: false, blockedBy: "bundled-load: Duplicate bundled rule id: x" };
    const summary = summarize([report([blocked])], noUngated, []);
    assert.deepEqual(summary.promotable, []);
    assert.deepEqual(summary.candidateRows.map((row) => [row.rule, row.stoppedAt, row.why]), [["b", "Tool-pack checks", "bundled-load: Duplicate bundled rule id: x"]]);
  });

  it("a listed known gap that does not hold, or excused nothing, blocks", () => {
    const knownGaps: GatesReport["knownGaps"] = [
      { id: "used", proven: true, kind: "valid-source", excused: 2 },
      { id: "unused", proven: true, kind: "valid-source", excused: 0 },
      { id: "not-source", proven: false, kind: "invalid-source", excused: 0 },
    ];
    const summary = summarize([{ ...report([rule("a", "shipped")]), knownGaps }], noUngated, []);
    assert.deepEqual(summary.blocking.map((row) => [row.pack, row.stoppedAt, row.check, row.why]), [
      ["python/bandit", "Known gaps", "known-gap", "known gap unused excused no mismatch; remove it or correct its fixtures"],
      ["python/bandit", "Known gaps", "known-gap", "known gap not-source does not hold (the report's run integrity says why); remove it or correct it"],
    ]);
    assert.equal(summary.pass, false);
    assert.equal(summarize([{ ...report([rule("a", "shipped")]), knownGaps: knownGaps.slice(0, 1) }], noUngated, []).pass, true);
  });

  it("documents come from rules/ (shipped) and candidates/** without probes or conformance manifests (candidate)", () => {
    const root = tempRoot();
    for (const path of ["packs/p/rules/a.json", "packs/p/candidates/rules/b.json", "packs/p/candidates/probes/c.json", "packs/p/candidates/conformance.json"]) write(root, path, {});
    assert.deepEqual(packDocuments(root, "p"), [
      { path: "packs/p/candidates/rules/b.json", location: "candidate" },
      { path: "packs/p/rules/a.json", location: "shipped" },
    ]);
  });
});

/** A rule of the conformance report: one no gated tool-pack judges. */
function unported(id: string, pack: string, location: RuleReport["location"], failing: Partial<Record<"loads" | "compact", string>> = {}, mapped = false): ConformanceRuleReport {
  const ok = { pass: true, checks: [] };
  const failed = (check: string) => ({ pass: false, checks: [{ id: check, ok: false, detail: `${check} failed` }] });
  const gates = {
    loads: failing.loads === undefined ? ok : failed(failing.loads),
    compact: { ...(failing.compact === undefined ? ok : failed(failing.compact)), metrics: null },
    "passes-conformance": mapped ? { ...ok, applies: true } : { pass: true, applies: false, checks: [{ id: "not-applicable", ok: true, detail: `packs/${pack} has no conformance.json` }] },
  };
  const stoppedAt = failing.loads !== undefined ? "loads" : failing.compact !== undefined ? "compact" : null;
  return { id, file: `packs/${pack}/${location === "shipped" ? "rules" : "candidates/rules"}/${id}.json`, pack, location, upstream: null, pass: stoppedAt === null, stoppedAt, gates };
}

function conformanceReport(rules: ConformanceRuleReport[], packs: ConformanceReport["packs"] = []): ConformanceReport {
  return { format: CONFORMANCE_REPORT_FORMAT, engine: { sha256: "" }, packs, rules };
}

describe("conformance and port readiness sections", () => {
  it("a shipped port that fails only a conformance gate blocks in the conformance section, not in port readiness", () => {
    const summary = summarize([report([rule("a", "shipped", { compact: "expansion" })])], noUngated, []);
    assert.deepEqual(summary.conformance.blocking.map((row) => [row.pack, row.rule, row.stoppedAt, row.check]), [["python", "a", "Compact (no written-out expansion)", "expansion"]]);
    assert.deepEqual(summary.blocking, []);
    assert.deepEqual(summary.shipped, { rules: 1, pass: 1 });
    assert.equal(summary.pass, false);
  });

  it("a shipped port that fails only a port gate blocks in port readiness, and passes conformance", () => {
    const summary = summarize([report([rule("a", "shipped", { "not-overfit": "fixture-literals" })])], noUngated, []);
    assert.deepEqual(summary.blocking.map((row) => [row.pack, row.rule, row.stoppedAt]), [["python/bandit", "a", "Not overfit to upstream's tests"]]);
    assert.deepEqual(summary.conformance.blocking, []);
    assert.deepEqual(summary.conformance.shipped, { rules: 1, pass: 1 });
    assert.equal(summary.pass, false);
  });

  it("a shipped port that fails both is listed in both sections, each at its own gate", () => {
    const summary = summarize([report([rule("a", "shipped", { "not-overfit": "fixture-literals", compact: "expansion" })])], noUngated, []);
    assert.deepEqual(summary.blocking.map((row) => row.stoppedAt), ["Not overfit to upstream's tests"]);
    assert.deepEqual(summary.conformance.blocking.map((row) => row.stoppedAt), ["Compact (no written-out expansion)"]);
  });

  it("the conformance section counts the rules no tool-pack judges with the ported ones, per pack", () => {
    const summary = summarize(
      [report([rule("a", "shipped"), rule("b", "candidate", { compact: "expansion" })])],
      noUngated,
      [],
      [],
      conformanceReport([unported("complexity-x", "complexity", "shipped"), unported("y", "sql", "candidate", { loads: "schema" }), unported("z", "sql", "candidate", {}, true)]),
    );
    assert.deepEqual(summary.conformance.packs, [
      { pack: "complexity", shipped: "1/1", candidates: "0/0" },
      { pack: "python", shipped: "1/1", candidates: "0/1" },
      { pack: "sql", shipped: "0/0", candidates: "1/2" },
    ]);
    assert.deepEqual(summary.conformance.shipped, { rules: 2, pass: 2 });
    assert.deepEqual(summary.conformance.candidates, { rules: 3, pass: 1 });
    assert.deepEqual(summary.conformance.candidateRows.map((row) => [row.pack, row.rule, row.stoppedAt]), [["python", "b", "Compact (no written-out expansion)"], ["sql", "y", "Compiles and loads on this engine"]]);
    assert.deepEqual(summary.conformance.notApplicable.map((entry) => entry.file), ["packs/complexity/rules/complexity-x.json", "packs/sql/candidates/rules/y.json"]);
    // The unported rules are not port readiness's: it counts the tool-pack's own.
    assert.deepEqual(summary.shipped, { rules: 1, pass: 1 });
    assert.equal(summary.pass, true);
  });

  it("a candidate that fails only a conformance gate is not ready to promote, and is counted apart from the port gates", () => {
    const summary = summarize([report([rule("b", "candidate", { "passes-conformance": "fixtures-row" }), rule("c", "candidate"), rule("d", "candidate", { loads: "json", "matches-upstream-tests": "not-run" })])], noUngated, []);
    assert.deepEqual(summary.promotable.map((entry) => entry.rule), ["c"]);
    assert.equal(summary.candidateFunnel["python/bandit"]![CONFORMANCE_COLUMN], 2);
    assert.equal(summary.candidateFunnel["python/bandit"]![READY_COLUMN], 1);
    assert.equal(summary.candidateFunnel["python/bandit"]!["Matches upstream on its own tests"], 0);
  });

  it("a shipped unported rule that fails, a failing pack check, and a missing conformance report each block", () => {
    const failing = summarize([], noUngated, [], [], conformanceReport([unported("complexity-x", "complexity", "shipped", { compact: "expansion" })], [{ pack: "complexity", checks: { pass: false, checks: [{ id: "bundled-load", ok: false, detail: "duplicate id" }] }, judgedByToolPacks: 0 }]));
    assert.deepEqual(failing.conformance.blocking.map((row) => [row.pack, row.rule, row.check]), [["complexity", "complexity-x", "expansion"], ["complexity", "-", "bundled-load"]]);
    assert.equal(failing.pass, false);
    const missing = summarize([], noUngated, [], [], null);
    assert.deepEqual(missing.conformance.blocking.map((row) => [row.stoppedAt, row.check]), [["Run", "report"]]);
    assert.equal(missing.pass, false);
  });
});

describe("the report", () => {
  it("lists every rule matches-upstream-tests compared without an end part, by the positions compared", () => {
    const compared = (id: string, location: RuleReport["location"], parts: { endLine: boolean; endCol: boolean } | undefined): RuleReport => {
      const base = rule(id, location);
      return { ...base, gates: { ...base.gates, "matches-upstream-tests": { ...base.gates["matches-upstream-tests"], ...(parts === undefined ? {} : { compared: parts }) } } };
    };
    const markdown = renderMarkdown(report([
      compared("full", "shipped", { endLine: true, endCol: true }),
      compared("filename-case", "shipped", { endLine: false, endCol: false }),
      compared("not-run", "shipped", undefined),
      compared("b101", "candidate", { endLine: false, endCol: true }),
      compared("entities", "candidate", { endLine: false, endCol: false }),
    ]));
    assert.ok(markdown.includes([
      "Rules compared without an end part, left out for every finding because upstream does not report it on every finding (for an ESLint rule, one finding on its own tests without an end is enough):",
      "",
      "- start only: `filename-case` (shipped), `entities` (candidate)",
      "- start and end column: `b101` (candidate)",
    ].join("\n")));
    assert.ok(!renderMarkdown(report([compared("full", "shipped", { endLine: true, endCol: true })])).includes("without an end part"));
  });
});

describe("engine-reads-goldens", () => {
  it("fails on any golden the engine could not fully read, naming the files and NER-1764", () => {
    const check = engineReadsGoldensCheck([
      { task: "t", path: "a.py", status: "agree" },
      { task: "t", path: "b.py", status: "engine-gap" },
      { task: "rerail:django", path: "c.py", status: "mismatch" },
    ]);
    assert.equal(check.id, "engine-reads-goldens");
    assert.equal(check.ok, false);
    assert.match(check.detail, /^1 of 3 golden/u);
    assert.match(check.detail, /t\/b\.py/u);
    assert.match(check.detail, /NER-1764/u);
  });

  it("passes when the engine read every golden, and on a rule without goldens", () => {
    assert.equal(engineReadsGoldensCheck([{ task: "t", path: "a.py", status: "mismatch" }]).ok, true);
    assert.equal(engineReadsGoldensCheck([]).ok, true);
  });

  it("a shipped rule whose only failure is engine-reads-goldens is listed as blocked by an engine gap", () => {
    const summary = summarize([report([rule("a", "shipped", { "matches-upstream-tests": "engine-reads-goldens" })])], noUngated, []);
    assert.deepEqual(summary.engineGaps.map((row) => row.rule), ["a"]);
    assert.equal(summary.pass, false);
  });
});

describe("cache keys", () => {
  const shared: SharedKeyInputs = { gateCode: "g", dependencies: "d", gatesYaml: "y", recordingJson: "j", corpusJson: "c", corpusTasks: "t", corpora: "[]" };
  const rules = {
    a: { engine: "e", document: '{"id":"a"}', conformanceEntry: "{}", goldens: "[]", caseSources: ["fixtures/a.py\0aa"] },
    b: { engine: "e", document: '{"id":"b"}', conformanceEntry: "{}", goldens: "[]", caseSources: ["fixtures/b.py\0bb"] },
  };

  it("a changed document changes only its own key", () => {
    const before = { a: ruleKey(shared, rules.a), b: ruleKey(shared, rules.b) };
    const after = { a: ruleKey(shared, { ...rules.a, document: '{"id":"a","x":1}' }), b: ruleKey(shared, rules.b) };
    assert.notEqual(after.a, before.a);
    assert.equal(after.b, before.b);
  });

  it("a changed golden or case source changes only that rule's key", () => {
    assert.notEqual(ruleKey(shared, { ...rules.a, goldens: '[{"task":"t"}]' }), ruleKey(shared, rules.a));
    assert.notEqual(ruleKey(shared, { ...rules.a, caseSources: ["fixtures/a.py\0ab"] }), ruleKey(shared, rules.a));
    assert.equal(ruleKey(shared, rules.b), ruleKey(shared, rules.b));
  });

  it("a gate-code, dependency or recording change changes every key", () => {
    for (const part of ["gateCode", "dependencies", "recordingJson", "corpusJson"] as const) {
      const changed = { ...shared, [part]: "changed" };
      assert.notEqual(ruleKey(changed, rules.a), ruleKey(shared, rules.a));
      assert.notEqual(ruleKey(changed, rules.b), ruleKey(shared, rules.b));
    }
  });

  // The engine is each rule's own key input: it is only the part of the engine the rule's language family reads.
  it("a changed engine key changes only the rules that share it", () => {
    assert.notEqual(ruleKey(shared, { ...rules.a, engine: "changed" }), ruleKey(shared, rules.a));
    assert.equal(ruleKey(shared, rules.b), ruleKey(shared, rules.b));
  });

  // Shard 1 judges a and shard 2 judges b, but both shards' tool-pack checks read the goldens and cases of a and b.
  it("a shard's tool-pack key moves with what its checks read of another shard's rules", () => {
    const pack = { engine: "e", names: ["A", "B"], goldens: ['a\0[]', 'b\0[{"task":"t","path":"x.js","sha256":"1"}]'], caseSources: ["fixtures/a.py\0aa", "fixtures/b.py\0bb"] };
    const shardOne = (inputs: typeof pack) => toolPackKey(shared, inputs, [ruleKey(shared, rules.a)]);
    const before = shardOne(pack);
    assert.equal(shardOne({ ...pack, goldens: [...pack.goldens].reverse(), caseSources: [...pack.caseSources].reverse() }), before);
    assert.notEqual(shardOne({ ...pack, goldens: ['a\0[]', 'b\0[{"task":"t","path":"x.js","sha256":"2"}]'] }), before);
    assert.notEqual(shardOne({ ...pack, caseSources: ["fixtures/a.py\0aa", "fixtures/b.py\0bc"] }), before);
    assert.notEqual(shardOne({ ...pack, names: ["A", "B", "C"] }), before);
    assert.notEqual(shardOne({ ...pack, engine: "changed" }), before);
    assert.notEqual(toolPackKey({ ...shared, recordingJson: "changed" }, pack, [ruleKey(shared, rules.a)]), before);
    // The gap counts come from the shard's own results, so its judged rules are part of the key too.
    assert.notEqual(toolPackKey(shared, pack, [ruleKey(shared, rules.b)]), before);
  });
});

describe("cached results", () => {
  const result = {
    report: { id: "a", file: "packs/p/rules/a.json", location: "shipped", pass: true, gates: {} },
    promoted: [],
    gapUses: {},
    integrity: [],
    gapCandidates: [],
  } as unknown as CachedRule;

  it("reads back what it wrote", async () => {
    const cache = tempRoot();
    await writeCached(cache, "p", "t", "k", result);
    assert.deepEqual(await readCached(cache, "p", "t", "k", isCachedRule), result);
  });

  it("an entry of another layout, the wrong shape or not JSON is a miss, never a verdict", async () => {
    const cache = tempRoot();
    const path = resultPath(cache, "p", "t", "k");
    mkdirSync(join(path, ".."), { recursive: true });
    for (const text of [JSON.stringify(result), JSON.stringify({ format: CACHE_FORMAT, value: { report: { pass: true } } }), "{"]) {
      writeFileSync(path, text);
      assert.equal(await readCached(cache, "p", "t", "k", isCachedRule), undefined, text);
    }
  });

  it("a tool-pack entry replays the passing corpus-complete check it was cached with; one without it is a miss", () => {
    const complete = { id: "corpus-complete", ok: true, detail: "all 3 recorded corpus file(s) were fetched with their recorded sha256" };
    const pack = { integrity: [], knownGaps: [], corpus: { tasks: [], files: 3, excluded: 0 }, corpusComplete: complete };
    assert.ok(isCachedPack(pack));
    const { corpusComplete: _complete, ...older } = pack;
    assert.equal(isCachedPack(older), false);
    assert.equal(isCachedPack({ ...pack, corpusComplete: { ...complete, ok: false } }), false);
  });
});

describe("not gated and pending", () => {
  function fixture(frozen: Record<string, string[]>, shipped: string[] = ["n1", "n2"]): string {
    const root = tempRoot();
    write(root, "packs/node/pack.json", { rules: shipped.map((id) => ({ id })) });
    write(root, "packs/node/conformance.json", { tools: [{ name: "eslint-plugin-n" }] });
    write(root, "packs/javascript/pack.json", { rules: [{ id: "j1" }] });
    write(root, "packs/javascript/conformance.json", { tools: [{ name: "eslint" }] });
    write(root, "packs/python/pack.json", { rules: [{ id: "p1" }] });
    write(root, "packs/python/conformance.json", { tools: [{ name: "bandit" }] });
    write(root, "packs/python/upstream/bandit/gates.yaml", "adapter: bandit\n");
    write(root, "conformance/port-readiness/ungated-rules.json", frozen);
    write(root, "conformance/port-readiness/pending.json", { "javascript/eslint": "still validated in nera-rule-validation" });
    return root;
  }
  const failing = (checks: { id: string; ok: boolean }[]) => checks.filter((check) => !check.ok).map((check) => check.id);
  const noGit = new FakeRunner({});

  it("passes when every shipped rule of a not-gated pack is listed; gated and pending packs are not frozen", async () => {
    const result = await ungatedChecks(fixture({ node: ["n1", "n2"] }), { runner: noGit });
    assert.deepEqual(failing(result.checks), []);
    assert.deepEqual(result.notGated, [{ pack: "node", rules: 2 }]);
    assert.deepEqual(result.pending.map((entry) => entry.toolPack), ["javascript/eslint"]);
  });

  it("a new shipped rule in a not-gated pack fails", async () => {
    const result = await ungatedChecks(fixture({ node: ["n1"] }), { runner: noGit });
    assert.deepEqual(failing(result.checks), ["not-gated:node"]);
    assert.match(result.checks.find((check) => !check.ok)!.detail, /keep it in candidates\//u);
  });

  it("a listed rule no longer shipped is stale", async () => {
    const result = await ungatedChecks(fixture({ node: ["n1", "n2", "n3"] }), { runner: noGit });
    assert.deepEqual(failing(result.checks), ["frozen-list:node"]);
    assert.match(result.checks.find((check) => !check.ok)!.detail, /remove it from the list/u);
  });

  it("gating is decided per tool: a gated pack's rule of a tool with no gates.yaml must be frozen", async () => {
    const root = fixture({ node: ["n1", "n2"] });
    write(root, "packs/python/pack.json", { rules: [{ id: "p1" }, { id: "p2" }] });
    write(root, "packs/python/conformance.json", { tools: [{ name: "bandit" }, { name: "flake8" }], rules: [{ id: "p1", upstream: { tool: "bandit" } }, { id: "p2", upstream: { tool: "flake8" } }] });
    const blocked = await ungatedChecks(root, { runner: noGit });
    assert.deepEqual(failing(blocked.checks), ["not-gated:python"]);
    assert.match(blocked.checks.find((check) => !check.ok)!.detail, /p2 of flake8/u);
    write(root, "conformance/port-readiness/ungated-rules.json", { node: ["n1", "n2"], python: ["p2"] });
    const frozen = await ungatedChecks(root, { runner: noGit });
    assert.deepEqual(failing(frozen.checks), []);
    assert.deepEqual(frozen.notGated, [{ pack: "node", rules: 2 }, { pack: "python", rules: 1 }]);
  });

  it("a pending tool covers only its own rules: another tool's rule in the same pack must be frozen", async () => {
    const root = fixture({ node: ["n1", "n2"] });
    write(root, "packs/javascript/pack.json", { rules: [{ id: "j1" }, { id: "j2" }] });
    write(root, "packs/javascript/conformance.json", { tools: [{ name: "eslint" }, { name: "eslint-plugin-unicorn" }], rules: [{ id: "j1", upstream: { tool: "eslint" } }, { id: "j2", upstream: { tool: "eslint-plugin-unicorn" } }] });
    const result = await ungatedChecks(root, { runner: noGit });
    assert.deepEqual(failing(result.checks), ["not-gated:javascript"]);
    assert.deepEqual(result.pending, [{ toolPack: "javascript/eslint", reason: "still validated in nera-rule-validation", rules: 1 }]);
  });

  it("a frozen rule whose tool is gated now is stale", async () => {
    const root = fixture({ node: ["n1", "n2"], python: ["p1"] });
    write(root, "packs/python/conformance.json", { tools: [{ name: "bandit" }], rules: [{ id: "p1", upstream: { tool: "bandit" } }] });
    const result = await ungatedChecks(root, { runner: noGit });
    assert.deepEqual(failing(result.checks), ["frozen-list:python"]);
  });

  const base = "a".repeat(40);
  const present = { "git cat-file": { exitCode: 0 } };

  it("the shrink guard is skipped when the base has no list", async () => {
    const runner = new FakeRunner({ ...present, "git ls-tree": { exitCode: 0, stdout: "" } });
    const result = await ungatedChecks(fixture({ node: ["n1", "n2"] }), { base, runner });
    assert.deepEqual(failing(result.checks), []);
  });

  it("the shrink guard fails an entry the base lacks", async () => {
    const runner = new FakeRunner({
      ...present,
      "git ls-tree": { exitCode: 0, stdout: "x\n" },
      [`git show ${base}:conformance/port-readiness/ungated-rules.json`]: { exitCode: 0, stdout: JSON.stringify({ node: ["n1"] }) },
      [`git show ${base}:conformance/port-readiness/pending.json`]: { exitCode: 0, stdout: JSON.stringify({ "javascript/eslint": "r" }) },
    });
    const result = await ungatedChecks(fixture({ node: ["n1", "n2"] }), { base, runner });
    assert.deepEqual(failing(result.checks), ["shrinks:conformance/port-readiness/ungated-rules.json"]);
  });

  it("the shrink guard fetches a missing base and fails closed when the fetch fails", async () => {
    const runner = new FakeRunner({ "git cat-file": { exitCode: 1 }, "git fetch": { exitCode: 128, stderr: "fatal: no network" } });
    const result = await ungatedChecks(fixture({ node: ["n1", "n2"] }), { base, runner });
    assert.ok(runner.calls.includes(`git fetch --depth=1 origin ${base}`));
    assert.deepEqual(failing(result.checks), ["shrinks:conformance/port-readiness/ungated-rules.json", "shrinks:conformance/port-readiness/pending.json"]);
  });

  it("the shrink guard fails closed when the base's list cannot be parsed", async () => {
    const runner = new FakeRunner({ ...present, "git ls-tree": { exitCode: 0, stdout: "x\n" }, "git show": { exitCode: 0, stdout: "{not json" } });
    const result = await ungatedChecks(fixture({ node: ["n1", "n2"] }), { base, runner });
    assert.equal(failing(result.checks).length, 2);
    assert.match(result.checks.find((check) => !check.ok)!.detail, /fails closed/u);
  });
});

describe("upstream's test files", () => {
  it("are checked out by their directory, glob characters escaped", () => {
    assert.equal(sparseDirectoryPattern("test/unit/eslint-plugin-next/custom-pages/[profile]/index.tsx"), "/test/unit/eslint-plugin-next/custom-pages/\\[profile\\]/");
    assert.equal(sparseDirectoryPattern("test/unit/eslint-plugin-next/with-app-dir/app/@modal/default.tsx"), "/test/unit/eslint-plugin-next/with-app-dir/app/@modal/");
    assert.equal(sparseDirectoryPattern("a/*?/b.py"), "/a/\\*\\?/");
    assert.equal(sparseDirectoryPattern("setup.py"), "/");
  });
});
