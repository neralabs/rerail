import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { afterEach, describe, it } from "vitest";

import { CACHE_FORMAT, readCached, resultPath, ruleKey, toolPackKey, writeCached, type SharedKeyInputs } from "../../../conformance/port-readiness/cache.js";
import { engineReadsGoldensCheck } from "../../../conformance/port-readiness/checks/matches-upstream-tests.js";
import { LEFT_TO_CONFORMANCE_JOB, type PackConformance } from "../../../conformance/port-readiness/checks/passes-conformance.js";
import type { RuleRow } from "../../../conformance/languages/report.js";
import { sparseDirectoryPattern } from "../../../conformance/port-readiness/recorded-adapter.js";
import { renderMarkdown } from "../../../conformance/port-readiness/report.js";
import { blockingFailures, isCachedPack, isCachedRule, packDocuments, type CachedRule, type GatesReport, type RuleReport } from "../../../conformance/port-readiness/run.js";
import { renderJobSummary } from "../../../conformance/port-readiness/comment.js";
import { parsePlanned, stepLabel, summarize } from "../../../conformance/port-readiness/summary.js";
import { CONFORMANCE_REPORT_FORMAT, conformanceBlockingFailures, runConformanceGates, type ConformanceReport, type ConformanceRuleReport } from "../../../conformance/port-readiness/conformance-gates.js";
import { loadEngine } from "../../../conformance/port-readiness/engine.js";
import { BLOCKING_GATES, type BlockingGateId } from "../../../conformance/port-readiness/types.js";
import type { Origin } from "../../../conformance/origins.js";
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

  it("a port origins.json declares, which no conformance.json entry maps, has no fixtures, so it fails conformance and says why; a shipped one blocks", () => {
    const unmapped: Origin = { kind: "port", upstream: { tool: "ruff", rule: "A005" }, unmapped: "Ruff's only fixtures for it are GPL-derived" };
    const summary = summarize([report([rule("a", "candidate")])], [bandit], undefined, conformanceReport([unported("x", "python", "candidate", unmapped), unported("y", "python", "shipped", unmapped)], [pythonPack]));
    assert.deepEqual(summary.rules.map((entry) => [entry.id, entry.origin, entry.upstream?.rule ?? null, entry.stoppedAt, entry.check]), [["a", "port", "X1", null, ""], ["x", "port", "A005", "fixtures", "has-fixtures"], ["y", "port", "A005", "fixtures", "has-fixtures"]]);
    // The reason carries the origins.json entry's own, so the comment can show it as it is.
    assert.equal(summary.rules[1]!.why, "no conformance.json entry maps it (Ruff's only fixtures for it are GPL-derived)");
    assert.equal(stepLabel("fixtures"), "Conformance > Passes its fixtures");
    assert.deepEqual(summary.failing.map((entry) => entry.id), ["y"]);
    assert.equal(summary.pass, false);
  });

  it("a rule document that declares no origin stops at the first conformance step, whatever else it fails; a shipped one blocks", () => {
    const summary = summarize([], [], [], conformanceReport([unported("x", "python", "candidate", null), unported("y", "complexity", "shipped", null, { loads: "schema" })], [pythonPack, packEntry("complexity")]));
    assert.deepEqual(summary.rules.map((entry) => [entry.id, entry.origin, entry.stoppedAt, entry.check]), [["y", null, "origin", "origin"], ["x", null, "origin", "origin"]]);
    assert.match(summary.rules[0]!.why, /^declares no origin: map it in conformance\.json if the conformance runner can score it, or declare it in origins\.json/u);
    assert.equal(stepLabel("origin"), "Conformance > Declares its origin: a port or an original");
    assert.deepEqual(summary.failing.map((entry) => entry.id), ["y"]);
    assert.deepEqual(summary.promotable, []);
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
    assert.deepEqual(summary.rules.map((entry) => [entry.id, entry.stoppedAt, entry.why]), [["b", "fixture-data", "bundled-load: Duplicate bundled rule id: x"]]);
  });

  it("a failing tool-pack check that vouches for the recorded answers stops every port of the linter in conformance; a licence check at the licence step", () => {
    const toolPackChecks = { pass: false, checks: [
      { id: "corpus-pins", ok: false, detail: "corpus.json was recorded on other corpus pins" },
      { id: "license-allowed", ok: false, detail: "upstream's licence GPL-3.0-only names GPL-3.0-only, which is not on scripts/check/licence-allowlist.json" },
    ] };
    const blocked = { ...rule("a", "shipped"), pass: false, blockedBy: "corpus-pins: ...; license-allowed: ..." };
    const summary = summarize([{ ...report([blocked]), toolPackChecks }], []);
    // The stale recording is the first failure: the fixtures upstream answered are judged against it, in conformance.
    assert.deepEqual(summary.failing.map((entry) => [entry.id, entry.stoppedAt, entry.check, entry.why]), [["a", "fixture-data", "corpus-pins", "corpus-pins: corpus.json was recorded on other corpus pins"]]);
    assert.equal(stepLabel("fixture-data"), "Conformance > Its fixtures' recorded answers are current");
    const licence = summarize([{ ...report([blocked]), toolPackChecks: { pass: false, checks: toolPackChecks.checks.slice(1) } }], []);
    assert.deepEqual(licence.failing.map((entry) => [entry.stoppedAt, entry.check]), [["licence", "license-allowed"]]);
    assert.equal(stepLabel("licence"), "Port readiness > Upstream licence kept and allowed");
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

const original: Origin = { kind: "original", why: "Rerail's own rule, modelled on lizard" };
/** A port of `tool` a conformance.json entry maps. */
const mapped = (tool: string, rule: string): Origin => ({ kind: "port", upstream: { tool, rule } });

/**
 * A rule of the conformance report, one no gated upstream linter judges, with the origin its pack declares (null: none).
 * As the conformance job does, a port a conformance.json entry maps has fixtures and any other rule none, unless
 * `fixtures` says otherwise (an original with fixtures, which this pass cannot give one yet).
 */
function unported(id: string, pack: string, location: RuleReport["location"], origin: Origin | null, failing: Partial<Record<"loads" | "compact", string>> = {}, fixtures = origin?.kind === "port" && origin.unmapped === undefined): ConformanceRuleReport {
  const ok = { pass: true, checks: [] };
  const failed = (check: string, detail = `${check} failed`) => ({ pass: false, checks: [{ id: check, ok: false, detail }] });
  const noFixtures = origin?.kind === "port" && origin.unmapped !== undefined ? `no conformance.json entry maps it (${origin.unmapped})` : "nothing gives this original input files with expected findings to be scored on";
  const gates = {
    loads: failing.loads === undefined ? ok : failed(failing.loads),
    compact: { ...(failing.compact === undefined ? ok : failed(failing.compact)), metrics: null },
    "passes-conformance": fixtures ? ok : failed("has-fixtures", noFixtures),
  };
  const stoppedAt = origin === null ? "origin" : failing.loads !== undefined ? "loads" : failing.compact !== undefined ? "compact" : fixtures ? null : "passes-conformance";
  return { id, file: `packs/${pack}/${location === "shipped" ? "rules" : "candidates/rules"}/${id}.json`, pack, location, origin, pass: stoppedAt === null, stoppedAt, gates };
}

function conformanceReport(rules: ConformanceRuleReport[], packs: ConformanceReport["packs"] = [], passesConformance: ConformanceReport["passesConformance"] = []): ConformanceReport {
  return { format: CONFORMANCE_REPORT_FORMAT, engine: { sha256: "" }, packs, rules, passesConformance };
}

const packEntry = (pack: string): ConformanceReport["packs"][number] => ({ pack, checks: { pass: true, checks: [] }, judgedByToolPacks: 0 });
const pythonPack = packEntry("python");

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

  it("an original with no fixtures fails conformance; one with fixtures is done after it, in any pack, never compared with an upstream; a port of a linter with no gates.yaml stops at set-up", () => {
    const summary = summarize([report([rule("a", "shipped"), rule("c", "candidate")])], [bandit], undefined, conformanceReport([
      unported("complexity-x", "complexity", "shipped", original),
      unported("complexity-y", "complexity", "candidate", original, {}, true),
      unported("complexity-z", "complexity", "candidate", original, { compact: "expansion" }),
      unported("n1", "node", "candidate", mapped("eslint-plugin-n", "n/n1")),
      unported("n2", "node", "shipped", mapped("eslint-plugin-n", "n/n2")),
      unported("n3", "node", "candidate", mapped("eslint-plugin-n", "n/n3"), { loads: "schema" }),
      unported("n4", "node", "candidate", original, {}, true),
    ], [packEntry("complexity"), packEntry("node")]));
    assert.deepEqual(summary.rules.filter((entry) => !entry.file.startsWith("packs/python/")).map((entry) => [entry.id, entry.origin, entry.stoppedAt, entry.check]), [
      ["complexity-y", "original", null, ""],
      ["complexity-z", "original", "compact", "expansion"],
      ["complexity-x", "original", "fixtures", "has-fixtures"],
      ["n1", "port", "set-up", "gates-yaml"],
      ["n3", "port", "loads", "schema"],
      ["n4", "original", null, ""],
      ["n2", "port", "set-up", "gates-yaml"],
    ]);
    // An original has no port readiness verdict and no real-code count: nothing holds it to an upstream.
    assert.equal(summary.rules.find((entry) => entry.id === "complexity-y")!.realCodeGoldens, undefined);
    assert.deepEqual(summary.promotable.map((entry) => entry.id), ["complexity-y", "n4", "c"]);
    assert.deepEqual(summary.failing.map((entry) => entry.id), ["complexity-x", "n2"]);
    assert.match(summary.failing[0]!.why, /^nothing gives this original input files/u);
    assert.match(summary.failing[1]!.why, /^eslint-plugin-n has no packs\/node\/upstream\/eslint-plugin-n\/gates\.yaml yet/u);
    assert.deepEqual(summary.shipped, { rules: 3, pass: 1 });
    assert.equal(summary.pass, false);
  });

  it("a gated port's differences from the answers upstream recorded fail its fixtures, in conformance; its evidence from upstream is port readiness's", () => {
    const differs = rule("a", "shipped", { "matches-upstream-tests": "identical-results" });
    const silentNowhere = rule("b", "shipped", { "matches-upstream-tests": "stays-silent" });
    const overfitToo = rule("c", "shipped", { "not-overfit": "no-fixture-only-names", "matches-upstream-tests": "fires" });
    const summary = summarize([report([differs, silentNowhere, overfitToo])], []);
    assert.deepEqual(summary.failing.map((entry) => [entry.id, entry.stoppedAt, entry.check]), [
      ["a", "fixtures", "identical-results"],
      ["b", "upstream-evidence", "stays-silent"],
      ["c", "not-overfit", "no-fixture-only-names"],
    ]);
    assert.equal(stepLabel("upstream-evidence"), "Port readiness > Has evidence from upstream");
  });

  it("a divergence the pack's fixtures excuse never excuses a difference on the fixtures upstream answered", () => {
    // passes-conformance passes (its divergences.json explains the difference there); upstream's test files still differ.
    const excusedThere = rule("a", "shipped", { "matches-upstream-tests": "identical-results" });
    excusedThere.gates["passes-conformance"] = { pass: true, checks: [{ id: "no-unexplained-false-positives", ok: true, detail: "1 finding explained by a divergence" }] };
    assert.deepEqual(summarize([report([excusedThere])], []).failing.map((entry) => [entry.stoppedAt, entry.check]), [["fixtures", "identical-results"]]);
  });

  it("a stale recording stops a port at its fixtures' recorded answers, before the differences judged against them", () => {
    const stale = { ...rule("a", "shipped", { "matches-upstream-tests": "identical-results" }), pass: false, blockedBy: "pins-agree: ..." };
    const toolPackChecks = { pass: false, checks: [{ id: "pins-agree", ok: false, detail: "gates.yaml and recording.json pin other versions" }] };
    assert.deepEqual(summarize([{ ...report([stale]), toolPackChecks }], []).failing.map((entry) => [entry.stoppedAt, entry.check]), [["fixture-data", "pins-agree"]]);
  });

  it("a gated port carries how many real-code goldens its fixtures hold", () => {
    const withGoldens = rule("a", "shipped");
    withGoldens.gates["matches-upstream-tests"].goldens.count = 3;
    assert.deepEqual(summarize([report([withGoldens, rule("b", "shipped")])], []).rules.map((entry) => [entry.id, entry.realCodeGoldens]), [["a", 3], ["b", 0]]);
  });

  it("a shipped rule that fails conformance, a failing pack check, and a missing conformance report each block", () => {
    const failing = summarize([], [], [], conformanceReport([unported("complexity-x", "complexity", "shipped", original, { compact: "expansion" })], [{ ...packEntry("complexity"), checks: { pass: false, checks: [{ id: "bundled-load", ok: false, detail: "duplicate id" }] } }]));
    assert.deepEqual(failing.failing.map((entry) => [entry.id, entry.check]), [["complexity-x", "expansion"]]);
    assert.deepEqual(failing.otherFailures.map((entry) => [entry.what, entry.check]), [["complexity", "bundled-load"]]);
    assert.equal(failing.pass, false);
    const missing = summarize([], [], [], null);
    assert.deepEqual(missing.otherFailures.map((entry) => [entry.what, entry.check]), [["(every pack)", "report"]]);
    assert.equal(missing.pass, false);
  });
});

describe("the pack's conformance check, run once in the conformance job", () => {
  const leftToJob = (id: string, location: RuleReport["location"]): RuleReport => {
    const base = rule(id, location);
    return { ...base, gates: { ...base.gates, "passes-conformance": { pass: true, checks: [{ id: LEFT_TO_CONFORMANCE_JOB, ok: true, detail: "" }] } } };
  };
  const fromJob = (file: string, location: RuleReport["location"], pass: boolean): ConformanceReport["passesConformance"][number] => ({ file, location, gate: { pass, checks: [{ id: "catches-something", ok: pass, detail: pass ? "" : "0 true positives on the fixtures" }] } });

  it("a port's passes-conformance comes from the conformance job's report, not its tool-pack run", () => {
    const summary = summarize([report([leftToJob("a", "shipped"), leftToJob("b", "candidate")])], [], [], conformanceReport([], [pythonPack], [
      fromJob("packs/python/rules/a.json", "shipped", false),
      fromJob("packs/python/candidates/rules/b.json", "candidate", true),
    ]));
    assert.deepEqual(summary.rules.map((entry) => [entry.id, entry.stoppedAt, entry.check]), [["b", null, ""], ["a", "fixtures", "catches-something"]]);
    assert.deepEqual(summary.failing.map((entry) => entry.id), ["a"]);
    assert.equal(summary.pass, false);
  });

  it("a rule its tool-pack run left to the conformance job, which the job did not report, fails closed", () => {
    const summary = summarize([report([leftToJob("a", "shipped")])], [], [], conformanceReport([], [pythonPack]));
    assert.deepEqual(summary.failing.map((entry) => [entry.id, entry.stoppedAt, entry.check]), [["a", "fixtures", "conformance-job"]]);
    // A tool-pack run that ran the check itself keeps its own result.
    assert.equal(summarize([report([rule("a", "shipped")])], [], [], conformanceReport([], [pythonPack])).pass, true);
  });

  it("a shipped port of a linter the plan left out that fails the conformance job blocks; a candidate does not", () => {
    const job = (passes: ConformanceReport["passesConformance"]) => conformanceReport([], [pythonPack], passes);
    const summary = summarize([], [bandit], [], job([
      fromJob("packs/python/rules/a.json", "shipped", false),
      fromJob("packs/python/rules/b.json", "shipped", true),
      fromJob("packs/python/candidates/rules/c.json", "candidate", false),
    ]));
    assert.deepEqual(summary.notJudged, ["python/bandit"]);
    assert.deepEqual(summary.otherFailures.map((entry) => [entry.what, entry.check]), [["packs/python/rules/a.json", "catches-something"]]);
    assert.match(summary.otherFailures[0]!.why, /not re-run on this change/u);
    assert.equal(summary.pass, false);
    // A port its tool-pack's report holds is judged with that report, not counted again.
    const judged = summarize([report([leftToJob("a", "shipped")])], [bandit], [bandit], job([fromJob("packs/python/rules/a.json", "shipped", false)]));
    assert.deepEqual(judged.failing.map((entry) => entry.id), ["a"]);
    assert.deepEqual(judged.otherFailures, []);
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

/** A stand-in for the pack's conformance check, which scores every rule in `passing` as a clean fixtures row. */
function packCheck(passing: string[]): (root: string, pack: string) => Promise<PackConformance> {
  const row = (id: string) => ({ rule: id, candidate: false, examples: false, upstream: { tool: "lint", rule: id }, capabilities: [], source: "fixtures", tp: 1, fp: 0, fn: 0, precision: 1, recall: 1, unanalysed: 0, engineFailures: 0, partialFiles: 0, syntaxErrorFiles: { count: 0, files: [] }, divergences: [], falsePositives: [], falseNegatives: [] }) as unknown as RuleRow;
  return async () => ({ rows: new Map(passing.map((id) => [id, row(id)])), pinsCorpora: false, stale: new Map(), uncovered: new Map(), ruleErrors: new Map(), errors: [] });
}

/**
 * A tree of two packs: `demo`, with no conformance.json, whose origins.json declares its two documents originals, and
 * `ported`, whose one tool has a gates.yaml.
 */
function conformanceTree(): string {
  const root = tempRoot();
  const complexity = readFileSync(join(PACKAGE_ROOT, "packs/complexity/rules/complexity-cyclomatic-python.json"), "utf8");
  write(root, "packs/demo/pack.json", { rules: [{ id: "complexity-cyclomatic-python", document: "rules/complexity-cyclomatic-python.json" }] });
  write(root, "packs/demo/rules/complexity-cyclomatic-python.json", complexity);
  write(root, "packs/demo/candidates/rules/broken.json", "{");
  write(root, "packs/demo/origins.json", ["rules/complexity-cyclomatic-python.json", "candidates/rules/broken.json"].map((document) => ({ document, original: "Rerail's own rule, for the conformance gates' tests" })));
  write(root, "packs/ported/pack.json", { rules: [] });
  write(root, "packs/ported/conformance.json", { tools: [{ name: "lint" }], rules: [{ id: "x", policy: "rules/x.json", upstream: { tool: "lint", rule: "x" } }] });
  write(root, "packs/ported/upstream/lint/gates.yaml", "");
  write(root, "packs/ported/rules/x.json", complexity);
  return root;
}

describe("the conformance gates", () => {
  // Alone it takes under a second, but as the file's first case, with the whole suite sharing the machine, it has taken
  // 14 s, past vitest's 5 s default.
  it("judge the rules no gated tool-pack judges, and leave the gated tool-pack's loads and compact to its own run", async () => {
    const root = conformanceTree();
    const report = await runConformanceGates(root, { engine: loadEngine(), cacheRoot: join(root, ".cache"), packConformance: packCheck(["x"]) });
    // The shipped original loads and is compact, but nothing gives it fixtures, so it fails, and blocks.
    assert.deepEqual(report.rules.map((rule) => [rule.file, rule.location, rule.origin?.kind, rule.pass, rule.stoppedAt]), [
      ["packs/demo/candidates/rules/broken.json", "candidate", "original", false, "loads"],
      ["packs/demo/rules/complexity-cyclomatic-python.json", "shipped", "original", false, "passes-conformance"],
    ]);
    assert.deepEqual(report.packs.map((entry) => [entry.pack, entry.checks.pass, entry.judgedByToolPacks]), [["demo", true, 0], ["ported", true, 1]]);
    assert.equal(conformanceBlockingFailures(report), 1);
  }, 30_000);

  it("run each pack's conformance check once, and report passes-conformance for the rules a gated tool-pack judges", async () => {
    const root = conformanceTree();
    const runs: string[] = [];
    const counting = (pack: string) => {
      runs.push(pack);
      return packCheck([])(root, pack);
    };
    const report = await runConformanceGates(root, { engine: loadEngine(), cacheRoot: join(root, ".cache"), packConformance: (_root, pack) => counting(pack) });
    assert.deepEqual(runs, ["ported"]);
    assert.deepEqual(report.passesConformance.map((entry) => [entry.file, entry.location, entry.gate.pass, entry.gate.checks.find((check) => !check.ok)?.id]), [["packs/ported/rules/x.json", "shipped", false, "fixtures-row"]]);
    // x, and demo's shipped original, which has no fixtures.
    assert.equal(conformanceBlockingFailures(report), 2);
  });

  it("fail the pack, and every rule it maps, when its conformance check does not run", async () => {
    const root = conformanceTree();
    const report = await runConformanceGates(root, { engine: loadEngine(), cacheRoot: join(root, ".cache"), packConformance: async () => { throw new Error("bad conformance.json"); } });
    assert.match(report.packs.find((entry) => entry.pack === "ported")!.checks.checks.find((check) => check.id === "conformance-runs")!.detail, /failed to run: bad conformance\.json/u);
    assert.deepEqual(report.passesConformance.map((entry) => entry.gate.pass), [false]);
  });

  it("make no in-scope check, and fail passes-conformance for a rule with no fixtures: an original, or a port no entry maps, with why", async () => {
    const root = conformanceTree();
    write(root, "packs/ported/candidates/rules/later.json", readFileSync(join(PACKAGE_ROOT, "packs/complexity/rules/complexity-cyclomatic-python.json"), "utf8"));
    write(root, "packs/ported/origins.json", [{ document: "candidates/rules/later.json", upstream: { tool: "lint", rule: "later" }, unmapped: "lint's only tests for it are GPL-derived" }]);
    const report = await runConformanceGates(root, { engine: loadEngine(), cacheRoot: join(root, ".cache"), packConformance: packCheck(["x"]) });
    const shipped = report.rules.find((rule) => rule.location === "shipped")!;
    assert.match(shipped.gates.loads.checks.find((check) => check.id === "in-scope")!.detail, /^not checked: /u);
    assert.deepEqual(shipped.gates["passes-conformance"].checks.map((check) => [check.id, check.ok, check.detail]), [["has-fixtures", false, "nothing gives this original input files with expected findings to be scored on"]]);
    assert.equal(shipped.gates.compact.pass, true);
    const later = report.rules.find((rule) => rule.file === "packs/ported/candidates/rules/later.json")!;
    assert.deepEqual([later.stoppedAt, later.gates["passes-conformance"].checks[0]!.detail], ["passes-conformance", "no conformance.json entry maps it (lint's only tests for it are GPL-derived)"]);
  });

  it("block on a shipped rule that fails, and on a pack whose shipped rules do not load together", async () => {
    const root = conformanceTree();
    write(root, "packs/demo/rules/broken.json", "{");
    write(root, "packs/demo/pack.json", { rules: [{ id: "complexity-cyclomatic-python", document: "rules/complexity-cyclomatic-python.json" }, { id: "broken", document: "rules/broken.json" }] });
    write(root, "packs/demo/origins.json", ["rules/complexity-cyclomatic-python.json", "rules/broken.json", "candidates/rules/broken.json"].map((document) => ({ document, original: "Rerail's own rule, for the conformance gates' tests" })));
    const report = await runConformanceGates(root, { engine: loadEngine(), cacheRoot: join(root, ".cache"), packConformance: packCheck(["x"]) });
    assert.deepEqual(report.rules.filter((rule) => rule.file === "packs/demo/rules/broken.json").map((rule) => [rule.pass, rule.stoppedAt]), [[false, "loads"]]);
    assert.equal(report.packs.find((entry) => entry.pack === "demo")?.checks.pass, false);
    // broken, the pack's bundled load, and the shipped original with no fixtures.
    assert.equal(conformanceBlockingFailures(report), 3);
  });

  it("stop a rule document that declares no origin at the origin step, and fail a pack whose origins.json is wrong", async () => {
    const root = conformanceTree();
    write(root, "packs/demo/origins.json", [{ document: "rules/gone.json", original: "declared, though no such document exists" }]);
    const report = await runConformanceGates(root, { engine: loadEngine(), cacheRoot: join(root, ".cache"), packConformance: packCheck(["x"]) });
    assert.deepEqual(report.rules.map((rule) => [rule.file, rule.origin, rule.pass, rule.stoppedAt]), [
      ["packs/demo/candidates/rules/broken.json", null, false, "origin"],
      ["packs/demo/rules/complexity-cyclomatic-python.json", null, false, "origin"],
    ]);
    const demo = report.packs.find((entry) => entry.pack === "demo")!;
    assert.deepEqual(demo.checks.checks.filter((check) => !check.ok).map((check) => [check.id, check.detail]), [["origins", "origins.json: rules/gone.json is not a rule document of the pack"]]);
    assert.equal(conformanceBlockingFailures(report), 2);
  });
});
