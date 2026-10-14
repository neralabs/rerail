import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { afterEach, describe, it } from "vitest";

import { ruleKey, type SharedKeyInputs } from "../../../conformance/gates/cache.js";
import { engineReadsGoldensCheck } from "../../../conformance/gates/checks/matches-upstream-tests.js";
import { sparseDirectoryPattern } from "../../../conformance/gates/recorded-adapter.js";
import type { CommandOptions, CommandResult, CommandRunner } from "../../../conformance/gates/process.js";
import { blockingFailures, packDocuments, type GatesReport, type RuleReport } from "../../../conformance/gates/run.js";
import { summarize } from "../../../conformance/gates/summary.js";
import { ungatedChecks } from "../../../conformance/gates/ungated.js";

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
    "matches-real-code": { checks: [], eligible: 0, positives: 0, cases: [], disagreementsElsewhere: { count: 0, first: [] } },
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
  const zero = { loads: 0, "not-overfit": 0, compact: 0, "matches-upstream-tests": 0 };
  return {
    format: "rerail.rule-gates/v1", pack: "python", tool: "bandit", toolPack: "python/bandit",
    upstream: { name: "", repository: "", commit: "", version: "", license: "", recommended_configs: [], runtime: "", identity: {} },
    engine: { sha256: "" }, gates: [], method: {} as GatesReport["method"], toolPackChecks: { pass: true, checks: [] }, integrity: [], knownGaps: [], gapCandidates: [],
    corpus: null, unmapped, listedWithoutDocument: 0, cache: { rules: rules.length, hits: 0 }, uncommittedPromotions: 0,
    summary: {
      shipped: { rules: rules.filter((r) => r.location === "shipped").length, pass: rules.filter((r) => r.location === "shipped" && r.pass).length },
      candidates: { rules: rules.filter((r) => r.location === "candidate").length, pass: rules.filter((r) => r.location === "candidate" && r.pass).length },
      stoppedAt: { shipped: { ...zero }, candidate: { ...zero } }, failedGate: { ...zero },
      endToEnd: { cases: 0, mismatches: 0, goldens: 0, goldensDiffering: 0, goldensEngineGaps: 0 },
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

  it("the summary lists shipped failures as blocking and candidates in the funnel, and fails on a missing report", () => {
    const summary = summarize([report([rule("a", "shipped", { loads: "schema" }), rule("b", "candidate"), rule("c", "candidate", { loads: "json" })])], noUngated, [{ pack: "python", tool: "bandit" }, { pack: "python", tool: "ruff" }]);
    assert.deepEqual(summary.blocking.map((row) => [row.pack, row.rule, row.check]), [["python/bandit", "a", "schema"], ["python/ruff", "(every rule)", "report"]]);
    assert.deepEqual(summary.promotable.map(({ toolPack, rule }) => ({ toolPack, rule })), [{ toolPack: "python/bandit", rule: "b" }]);
    assert.equal(summary.pass, false);
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
  const shared: SharedKeyInputs = { gateCode: "g", engine: "e", gatesYaml: "y", gatesJson: "j", gatesCorpus: "c", corpusTasks: "t", corpora: "[]" };
  const rules = {
    a: { document: '{"id":"a"}', conformanceEntry: "{}", goldens: "[]", caseSources: ["fixtures/a.py\0aa"] },
    b: { document: '{"id":"b"}', conformanceEntry: "{}", goldens: "[]", caseSources: ["fixtures/b.py\0bb"] },
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

  it("an engine, gate-code or recording change changes every key", () => {
    for (const part of ["engine", "gateCode", "gatesJson", "gatesCorpus"] as const) {
      const changed = { ...shared, [part]: "changed" };
      assert.notEqual(ruleKey(changed, rules.a), ruleKey(shared, rules.a));
      assert.notEqual(ruleKey(changed, rules.b), ruleKey(shared, rules.b));
    }
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
    write(root, "conformance/gates/ungated-rules.json", frozen);
    write(root, "conformance/gates/pending.json", { "javascript/eslint": "still validated in nera-rule-validation" });
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
      [`git show ${base}:conformance/gates/ungated-rules.json`]: { exitCode: 0, stdout: JSON.stringify({ node: ["n1"] }) },
      [`git show ${base}:conformance/gates/pending.json`]: { exitCode: 0, stdout: JSON.stringify({ "javascript/eslint": "r" }) },
    });
    const result = await ungatedChecks(fixture({ node: ["n1", "n2"] }), { base, runner });
    assert.deepEqual(failing(result.checks), ["shrinks:conformance/gates/ungated-rules.json"]);
  });

  it("the shrink guard fetches a missing base and fails closed when the fetch fails", async () => {
    const runner = new FakeRunner({ "git cat-file": { exitCode: 1 }, "git fetch": { exitCode: 128, stderr: "fatal: no network" } });
    const result = await ungatedChecks(fixture({ node: ["n1", "n2"] }), { base, runner });
    assert.ok(runner.calls.includes(`git fetch --depth=1 origin ${base}`));
    assert.deepEqual(failing(result.checks), ["shrinks:conformance/gates/ungated-rules.json", "shrinks:conformance/gates/pending.json"]);
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
