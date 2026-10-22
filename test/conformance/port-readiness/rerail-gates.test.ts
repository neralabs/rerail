import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { afterEach, describe, it } from "vitest";

import { CACHE_FORMAT, readCached, resultPath, ruleKey, toolPackKey, writeCached, type SharedKeyInputs } from "../../../conformance/port-readiness/cache.js";
import { engineReadsGoldensCheck } from "../../../conformance/port-readiness/checks/matches-upstream-tests.js";
import { sparseDirectoryPattern } from "../../../conformance/port-readiness/recorded-adapter.js";
import { renderMarkdown } from "../../../conformance/port-readiness/report.js";
import { blockingFailures, isCachedPack, isCachedRule, packDocuments, type CachedRule, type GatesReport, type RuleReport } from "../../../conformance/port-readiness/run.js";
import { renderJobSummary } from "../../../conformance/port-readiness/comment.js";
import { parsePlanned, stepLabel, summarize } from "../../../conformance/port-readiness/summary.js";
import { CONFORMANCE_REPORT_FORMAT, conformanceBlockingFailures, runConformanceGates, type ConformanceReport, type ConformanceRuleReport } from "../../../conformance/port-readiness/conformance-gates.js";
import { loadEngine } from "../../../conformance/port-readiness/engine.js";
import { BLOCKING_GATES, type BlockingGateId } from "../../../conformance/port-readiness/types.js";
import { PACKAGE_ROOT } from "../../../src/engine/index.js";

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

const bandit = { pack: "python", tool: "bandit" };
const gated = [bandit, { pack: "python", tool: "ruff" }];

describe("location-based blocking", () => {
  it("a failing shipped rule blocks; a failing candidate does not", () => {
    assert.equal(blockingFailures(report([rule("a", "shipped"), rule("b", "candidate", { loads: "schema" })])), 0);
    assert.equal(blockingFailures(report([rule("a", "shipped", { loads: "schema" }), rule("b", "candidate")])), 1);
  });

  it("an unmapped shipped document blocks; an unmapped candidate is only reported", () => {
    assert.equal(blockingFailures(report([], [{ file: "packs/python/candidates/rules/x.json", location: "candidate" }])), 0);
    assert.equal(blockingFailures(report([], [{ file: "packs/python/rules/x.json", location: "shipped" }])), 1);
  });

  it("an unmapped document of a pack with a conformance.json is a port that stops at the mapping step; a shipped one blocks", () => {
    const summary = summarize([report([rule("a", "candidate")])], [bandit], undefined, conformanceReport([unported("x", "python", "candidate"), unported("y", "python", "shipped")], [pythonPack]));
    assert.deepEqual(summary.rules.map((entry) => [entry.id, entry.port, entry.stoppedAt]), [["a", true, null], ["x", true, "mapping"], ["y", true, "mapping"]]);
    assert.deepEqual(summary.failing.map((entry) => entry.id), ["y"]);
    assert.equal(summary.pass, false);
  });

  it("the summary lists failing shipped rules, and fails on a missing report", () => {
    const summary = summarize([report([rule("a", "shipped", { loads: "schema" }), rule("b", "candidate"), rule("c", "candidate", { loads: "json" })])], gated);
    assert.deepEqual(summary.failing.map((entry) => [entry.id, entry.stoppedAt, entry.check]), [["a", "loads", "schema"]]);
    assert.deepEqual(summary.otherFailures.map((entry) => [entry.what, entry.check]), [["python/ruff", "report"]]);
    assert.deepEqual(summary.promotable.map((entry) => [entry.id, entry.upstream, entry.onByDefault]), [["b", { tool: "bandit", rule: "X1" }, true]]);
    assert.deepEqual(summary.notJudged, []);
    assert.equal(summary.pass, false);
  });

  it("an upstream linter the plan left out is listed as not re-run and blocks nothing", () => {
    const summary = summarize([report([rule("a", "shipped")])], gated, [bandit]);
    assert.deepEqual(summary.notJudged, ["python/ruff"]);
    assert.deepEqual([summary.failing, summary.otherFailures], [[], []]);
    assert.equal(summary.pass, true);
    assert.match(renderJobSummary(summary), /^Not re-run on this change, since nothing their ports read changed: python\/ruff\. Main's results stand for them, and the counts leave them out\.$/mu);
    assert.doesNotMatch(renderJobSummary(summarize([report([rule("a", "shipped")])], [bandit])), /Not re-run/u);
  });

  it("a planned upstream linter with no report blocks", () => {
    const summary = summarize([], gated, [{ pack: "python", tool: "ruff" }]);
    assert.deepEqual(summary.otherFailures.map((entry) => [entry.what, entry.check]), [["python/ruff", "report"]]);
    assert.deepEqual(summary.notJudged, ["python/bandit"]);
    assert.equal(summary.pass, false);
  });

  it("a planned upstream linter with no gates.yaml blocks: the plan is wrong", () => {
    const summary = summarize([report([rule("a", "shipped")])], gated, [bandit, { pack: "python", tool: "flake8" }]);
    assert.deepEqual(summary.otherFailures.map((entry) => [entry.what, entry.check]), [["python/flake8", "plan"]]);
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

  it("a candidate a failing upstream linter check blocks is not ready to promote, and says why", () => {
    const blocked = { ...rule("b", "candidate"), pass: false, blockedBy: "bundled-load: Duplicate bundled rule id: x" };
    const summary = summarize([report([blocked])], []);
    assert.deepEqual(summary.promotable, []);
    assert.deepEqual(summary.rules.map((entry) => [entry.id, entry.stoppedAt, entry.why]), [["b", "linter-checks", "bundled-load: Duplicate bundled rule id: x"]]);
  });

  it("a listed known gap that does not hold, or excused nothing, blocks", () => {
    const knownGaps: GatesReport["knownGaps"] = [
      { id: "used", proven: true, kind: "valid-source", excused: 2 },
      { id: "unused", proven: true, kind: "valid-source", excused: 0 },
      { id: "not-source", proven: false, kind: "invalid-source", excused: 0 },
    ];
    const summary = summarize([{ ...report([rule("a", "shipped")]), knownGaps }], []);
    assert.deepEqual(summary.otherFailures.map((entry) => [entry.what, entry.check, entry.why]), [
      ["python/bandit", "known-gap", "known gap unused excused no mismatch; remove it or correct its fixtures"],
      ["python/bandit", "known-gap", "known gap not-source does not hold (the report's run integrity says why); remove it or correct it"],
    ]);
    assert.equal(summary.pass, false);
    assert.equal(summarize([{ ...report([rule("a", "shipped")]), knownGaps: knownGaps.slice(0, 1) }], []).pass, true);
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

/** A rule of the conformance report: one no gated upstream linter judges, of `tool` when conformance.json maps it. */
function unported(id: string, pack: string, location: RuleReport["location"], failing: Partial<Record<"loads" | "compact", string>> = {}, tool?: string): ConformanceRuleReport {
  const ok = { pass: true, checks: [] };
  const failed = (check: string) => ({ pass: false, checks: [{ id: check, ok: false, detail: `${check} failed` }] });
  const gates = {
    loads: failing.loads === undefined ? ok : failed(failing.loads),
    compact: { ...(failing.compact === undefined ? ok : failed(failing.compact)), metrics: null },
    "passes-conformance": tool !== undefined ? { ...ok, applies: true } : { pass: true, applies: false, checks: [{ id: "not-applicable", ok: true, detail: "nothing to score it against" }] },
  };
  const stoppedAt = failing.loads !== undefined ? "loads" : failing.compact !== undefined ? "compact" : null;
  return { id, file: `packs/${pack}/${location === "shipped" ? "rules" : "candidates/rules"}/${id}.json`, pack, location, upstream: tool === undefined ? null : { tool, rule: id }, pass: stoppedAt === null, stoppedAt, gates };
}

function conformanceReport(rules: ConformanceRuleReport[], packs: ConformanceReport["packs"] = []): ConformanceReport {
  return { format: CONFORMANCE_REPORT_FORMAT, engine: { sha256: "" }, packs, rules };
}

const packEntry = (pack: string, hasConformanceJson: boolean): ConformanceReport["packs"][number] => ({ pack, checks: { pass: true, checks: [] }, judgedByToolPacks: 0, hasConformanceJson });
const pythonPack = packEntry("python", true);

describe("one verdict per rule", () => {
  it("a rule is counted once, at the first step it fails: the conformance gates come before the port readiness gates", () => {
    const summary = summarize([report([rule("a", "shipped", { "not-overfit": "fixture-literals", compact: "expansion" })])], []);
    assert.deepEqual(summary.rules.map((entry) => [entry.id, entry.stoppedAt, entry.check]), [["a", "compact", "expansion"]]);
    assert.equal(stepLabel("compact"), "Conformance > Compact (no written-out expansion)");
    assert.deepEqual(summary.shipped, { rules: 1, pass: 0 });
    assert.equal(summary.pass, false);
  });

  it("a port that passes conformance stops at the first port readiness gate it fails", () => {
    const summary = summarize([report([rule("a", "shipped", { "not-overfit": "fixture-literals" })])], []);
    assert.deepEqual(summary.failing.map((entry) => [entry.id, entry.stoppedAt]), [["a", "not-overfit"]]);
    assert.equal(stepLabel("not-overfit"), "Port readiness > Not overfit to upstream's tests");
  });

  it("a port whose only failing loads check is in-scope stops at the in-scope port readiness step, not at loads", () => {
    const outOfScope = rule("a", "shipped");
    outOfScope.gates.loads = { pass: false, checks: [{ id: "schema", ok: true, detail: "" }, { id: "in-scope", ok: false, detail: "does not apply to src/example.pyi" }] };
    Object.assign(outOfScope, { pass: false, stoppedAt: "loads" });
    const summary = summarize([report([outOfScope, { ...structuredClone(outOfScope), id: "b", location: "candidate", on: null }])], []);
    assert.deepEqual(summary.rules.map((entry) => [entry.id, entry.stoppedAt, entry.check]), [["a", "in-scope", "in-scope"], ["b", "in-scope", "in-scope"]]);
  });

  it("a rule that is not a port is done after conformance; a port of a linter with no gates.yaml stops at set-up", () => {
    const summary = summarize([report([rule("a", "shipped"), rule("c", "candidate")])], [bandit], undefined, conformanceReport([
      unported("complexity-x", "complexity", "shipped"),
      unported("complexity-y", "complexity", "candidate"),
      unported("complexity-z", "complexity", "candidate", { compact: "expansion" }),
      unported("n1", "node", "candidate", {}, "eslint-plugin-n"),
      unported("n2", "node", "shipped", {}, "eslint-plugin-n"),
      unported("n3", "node", "candidate", { loads: "schema" }, "eslint-plugin-n"),
    ], [packEntry("complexity", false), packEntry("node", true)]));
    assert.deepEqual(summary.rules.filter((entry) => !entry.file.startsWith("packs/python/")).map((entry) => [entry.id, entry.port, entry.stoppedAt]), [
      ["complexity-y", false, null],
      ["complexity-z", false, "compact"],
      ["complexity-x", false, null],
      ["n1", true, "set-up"],
      ["n3", true, "loads"],
      ["n2", true, "set-up"],
    ]);
    assert.deepEqual(summary.promotable.map((entry) => entry.id), ["complexity-y", "c"]);
    assert.deepEqual(summary.failing.map((entry) => entry.id), ["n2"]);
    assert.match(summary.failing[0]!.why, /^eslint-plugin-n has no packs\/node\/upstream\/eslint-plugin-n\/gates\.yaml yet/u);
    assert.deepEqual(summary.shipped, { rules: 3, pass: 2 });
    assert.equal(summary.pass, false);
  });

  it("a shipped rule that fails conformance, a failing pack check, and a missing conformance report each block", () => {
    const failing = summarize([], [], [], conformanceReport([unported("complexity-x", "complexity", "shipped", { compact: "expansion" })], [{ ...packEntry("complexity", false), checks: { pass: false, checks: [{ id: "bundled-load", ok: false, detail: "duplicate id" }] } }]));
    assert.deepEqual(failing.failing.map((entry) => [entry.id, entry.check]), [["complexity-x", "expansion"]]);
    assert.deepEqual(failing.otherFailures.map((entry) => [entry.what, entry.check]), [["complexity", "bundled-load"]]);
    assert.equal(failing.pass, false);
    const missing = summarize([], [], [], null);
    assert.deepEqual(missing.otherFailures.map((entry) => [entry.what, entry.check]), [["(every pack)", "report"]]);
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
  it("fails on any golden the engine could not fully read, naming the files and sending the fix to the engine", () => {
    const check = engineReadsGoldensCheck([
      { task: "t", path: "a.py", status: "agree" },
      { task: "t", path: "b.py", status: "engine-gap" },
      { task: "rerail:django", path: "c.py", status: "mismatch" },
    ]);
    assert.equal(check.id, "engine-reads-goldens");
    assert.equal(check.ok, false);
    assert.match(check.detail, /^1 of 3 golden/u);
    assert.match(check.detail, /t\/b\.py/u);
    assert.match(check.detail, /the fix belongs in the engine/u);
  });

  it("passes when the engine read every golden, and on a rule without goldens", () => {
    assert.equal(engineReadsGoldensCheck([{ task: "t", path: "a.py", status: "mismatch" }]).ok, true);
    assert.equal(engineReadsGoldensCheck([]).ok, true);
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

describe("upstream's test files", () => {
  it("are checked out by their directory, glob characters escaped", () => {
    assert.equal(sparseDirectoryPattern("test/unit/eslint-plugin-next/custom-pages/[profile]/index.tsx"), "/test/unit/eslint-plugin-next/custom-pages/\\[profile\\]/");
    assert.equal(sparseDirectoryPattern("test/unit/eslint-plugin-next/with-app-dir/app/@modal/default.tsx"), "/test/unit/eslint-plugin-next/with-app-dir/app/@modal/");
    assert.equal(sparseDirectoryPattern("a/*?/b.py"), "/a/\\*\\?/");
    assert.equal(sparseDirectoryPattern("setup.py"), "/");
  });
});

/** A tree of two packs: `demo`, with no conformance.json, and `ported`, whose one tool has a gates.yaml. */
function conformanceTree(): string {
  const root = tempRoot();
  const complexity = readFileSync(join(PACKAGE_ROOT, "packs/complexity/rules/complexity-cyclomatic-python.json"), "utf8");
  write(root, "packs/demo/pack.json", { rules: [{ id: "complexity-cyclomatic-python", document: "rules/complexity-cyclomatic-python.json" }] });
  write(root, "packs/demo/rules/complexity-cyclomatic-python.json", complexity);
  write(root, "packs/demo/candidates/rules/broken.json", "{");
  write(root, "packs/ported/pack.json", { rules: [] });
  write(root, "packs/ported/conformance.json", { tools: [{ name: "lint" }], rules: [{ id: "x", policy: "rules/x.json", upstream: { tool: "lint", rule: "x" } }] });
  write(root, "packs/ported/upstream/lint/gates.yaml", "");
  write(root, "packs/ported/rules/x.json", complexity);
  return root;
}

describe("the conformance gates", () => {
  // Alone it takes under a second, but as the file's first case, with the whole suite sharing the machine, it has taken
  // 14 s, past vitest's 5 s default.
  it("judge the rules no gated tool-pack judges, and leave the gated tool-pack's to its own run", async () => {
    const root = conformanceTree();
    const report = await runConformanceGates(root, { engine: loadEngine(), cacheRoot: join(root, ".cache") });
    assert.deepEqual(report.rules.map((rule) => [rule.file, rule.location, rule.pass, rule.stoppedAt]), [
      ["packs/demo/candidates/rules/broken.json", "candidate", false, "loads"],
      ["packs/demo/rules/complexity-cyclomatic-python.json", "shipped", true, null],
    ]);
    assert.deepEqual(report.packs.map((entry) => [entry.pack, entry.checks.pass, entry.judgedByToolPacks]), [["demo", true, 0], ["ported", true, 1]]);
    assert.equal(conformanceBlockingFailures(report), 0);
  }, 30_000);

  it("make no in-scope check, and say passes-conformance does not apply to a pack with no conformance.json", async () => {
    const root = conformanceTree();
    const report = await runConformanceGates(root, { engine: loadEngine(), cacheRoot: join(root, ".cache") });
    const shipped = report.rules.find((rule) => rule.location === "shipped")!;
    assert.match(shipped.gates.loads.checks.find((check) => check.id === "in-scope")!.detail, /^not checked: /u);
    assert.equal(shipped.gates["passes-conformance"].applies, false);
    assert.match(shipped.gates["passes-conformance"].checks[0]!.detail, /packs\/demo has no conformance\.json/u);
    assert.equal(shipped.gates.compact.pass, true);
  });

  it("block on a shipped rule that fails, and on a pack whose shipped rules do not load together", async () => {
    const root = conformanceTree();
    write(root, "packs/demo/rules/broken.json", "{");
    write(root, "packs/demo/pack.json", { rules: [{ id: "complexity-cyclomatic-python", document: "rules/complexity-cyclomatic-python.json" }, { id: "broken", document: "rules/broken.json" }] });
    const report = await runConformanceGates(root, { engine: loadEngine(), cacheRoot: join(root, ".cache") });
    assert.equal(report.rules.find((rule) => rule.file === "packs/demo/rules/broken.json")?.pass, false);
    assert.equal(report.packs.find((entry) => entry.pack === "demo")?.checks.pass, false);
    assert.equal(conformanceBlockingFailures(report), 2);
  });
});
