import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { afterEach, describe, it } from "vitest";

import { CACHE_FORMAT, readCached, resultPath, ruleKey, toolPackKey, writeCached, type SharedKeyInputs } from "../../../conformance/port-readiness/cache.js";
import { sparseDirectoryPattern } from "../../../conformance/port-readiness/recorded-adapter.js";
import type { CommandOptions, CommandResult, CommandRunner } from "../../../conformance/port-readiness/process.js";
import { renderMarkdown } from "../../../conformance/port-readiness/report.js";
import { blockingFailures, isCachedPack, isCachedRule, packDocuments, type CachedRule, type GatesReport, type RuleReport } from "../../../conformance/port-readiness/run.js";
import { parsePlanned, renderSummary, summarize } from "../../../conformance/port-readiness/summary.js";
import { ungatedChecks } from "../../../conformance/port-readiness/ungated.js";

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
    "matches-upstream-tests": { ...ok, totals: null, cases: {}, silentCases: 0, differences: [] },
    "passes-conformance": ok,
  };
}

function rule(id: string, location: RuleReport["location"], failing: Partial<Record<"matches-upstream-tests" | "loads", string>> = {}): RuleReport {
  const gates = passingGates();
  for (const [gate, check] of Object.entries(failing) as ["matches-upstream-tests" | "loads", string][]) {
    gates[gate] = { ...gates[gate], pass: false, checks: [{ id: check, ok: false, detail: `${check} failed` }] } as never;
  }
  const stoppedAt = failing.loads !== undefined ? "loads" : failing["matches-upstream-tests"] !== undefined ? "matches-upstream-tests" : null;
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
    unmapped, listedWithoutDocument: 0, cache: { rules: rules.length, hits: 0 },
    summary: {
      shipped: { rules: rules.filter((r) => r.location === "shipped").length, pass: rules.filter((r) => r.location === "shipped" && r.pass).length },
      candidates: { rules: rules.filter((r) => r.location === "candidate").length, pass: rules.filter((r) => r.location === "candidate" && r.pass).length },
      stoppedAt: { shipped: { ...zero }, candidate: { ...zero } }, failedGate: { ...zero },
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
    assert.match(renderSummary(summary), /^1 candidate is not judged: no conformance\.json entry maps it to an upstream rule, so no gate ran\n\n- python: `packs\/python\/candidates\/rules\/x\.json`$/mu);
  });

  it("the summary lists shipped failures as blocking and candidates in the funnel, and fails on a missing report", () => {
    const summary = summarize([report([rule("a", "shipped", { loads: "schema" }), rule("b", "candidate"), rule("c", "candidate", { loads: "json" })])], noUngated, [{ pack: "python", tool: "bandit" }, { pack: "python", tool: "ruff" }]);
    assert.deepEqual(summary.blocking.map((row) => [row.pack, row.rule, row.check]), [["python/bandit", "a", "schema"], ["python/ruff", "(every rule)", "report"]]);
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
    assert.match(markdown, /^\*\*In the 1 of 2 tool-packs judged: 1 of 1 shipped rules pass;/mu);
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

describe("cache keys", () => {
  const shared: SharedKeyInputs = { gateCode: "g", dependencies: "d", gatesYaml: "y", recordingArchive: "r", adapterSources: "a" };
  const rules = {
    a: { engine: "e", document: '{"id":"a"}', conformanceEntry: "{}", caseSources: ["fixtures/a.py\0aa"], recording: "ra" },
    b: { engine: "e", document: '{"id":"b"}', conformanceEntry: "{}", caseSources: ["fixtures/b.py\0bb"], recording: "rb" },
  };

  it("a changed document changes only its own key", () => {
    const before = { a: ruleKey(shared, rules.a), b: ruleKey(shared, rules.b) };
    const after = { a: ruleKey(shared, { ...rules.a, document: '{"id":"a","x":1}' }), b: ruleKey(shared, rules.b) };
    assert.notEqual(after.a, before.a);
    assert.equal(after.b, before.b);
  });

  it("a changed case source changes only that rule's key", () => {
    assert.notEqual(ruleKey(shared, { ...rules.a, caseSources: ["fixtures/a.py\0ab"] }), ruleKey(shared, rules.a));
    assert.equal(ruleKey(shared, rules.b), ruleKey(shared, rules.b));
  });

  it("a gate-code, dependency or recorder adapter change changes every key", () => {
    for (const part of ["gateCode", "dependencies", "adapterSources"] as const) {
      const changed = { ...shared, [part]: "changed" };
      assert.notEqual(ruleKey(changed, rules.a), ruleKey(shared, rules.a));
      assert.notEqual(ruleKey(changed, rules.b), ruleKey(shared, rules.b));
    }
  });

  // A rule's key takes only what it reads of the recording archive, so another rule recorded again keeps it.
  it("a re-recorded rule changes only its own key, and a new archive alone changes no rule's key", () => {
    assert.notEqual(ruleKey(shared, { ...rules.a, recording: "changed" }), ruleKey(shared, rules.a));
    assert.equal(ruleKey({ ...shared, recordingArchive: "changed" }, rules.a), ruleKey(shared, rules.a));
    assert.equal(ruleKey({ ...shared, recordingArchive: "changed" }, rules.b), ruleKey(shared, rules.b));
  });

  // The engine is each rule's own key input: it is only the part of the engine the rule's language family reads.
  it("a changed engine key changes only the rules that share it", () => {
    assert.notEqual(ruleKey(shared, { ...rules.a, engine: "changed" }), ruleKey(shared, rules.a));
    assert.equal(ruleKey(shared, rules.b), ruleKey(shared, rules.b));
  });

  // Shard 1 judges a and shard 2 judges b, but both shards' tool-pack checks read the cases of a and b.
  it("a shard's tool-pack key moves with what its checks read of another shard's rules", () => {
    const pack = { engine: "e", names: ["A", "B"], caseSources: ["fixtures/a.py\0aa", "fixtures/b.py\0bb"] };
    const shardOne = (inputs: typeof pack) => toolPackKey(shared, inputs, [ruleKey(shared, rules.a)]);
    const before = shardOne(pack);
    assert.equal(shardOne({ ...pack, caseSources: [...pack.caseSources].reverse() }), before);
    assert.notEqual(shardOne({ ...pack, caseSources: ["fixtures/a.py\0aa", "fixtures/b.py\0bc"] }), before);
    assert.notEqual(shardOne({ ...pack, names: ["A", "B", "C"] }), before);
    assert.notEqual(shardOne({ ...pack, engine: "changed" }), before);
    assert.notEqual(toolPackKey({ ...shared, recordingArchive: "changed" }, pack, [ruleKey(shared, rules.a)]), before);
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

  it("a tool-pack entry needs its integrity lines and known gaps; one without them is a miss", () => {
    assert.ok(isCachedPack({ integrity: [], knownGaps: [] }));
    assert.equal(isCachedPack({ integrity: [] }), false);
    assert.equal(isCachedPack({ integrity: "none", knownGaps: [] }), false);
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
