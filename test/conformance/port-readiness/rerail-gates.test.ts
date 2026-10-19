import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { afterEach, describe, it } from "vitest";

import { CACHE_FORMAT, readCached, resultPath, ruleKey, toolPackKey, writeCached, type SharedKeyInputs } from "../../../conformance/port-readiness/cache.js";
import { engineReadsGoldensCheck } from "../../../conformance/port-readiness/checks/matches-upstream-tests.js";
import { renderComment } from "../../../conformance/port-readiness/comment.js";
import { readGatesManifest } from "../../../conformance/port-readiness/manifest.js";
import { RUNTIMES_DIRECTORY, sparseDirectoryPattern, staleMessage, staleRuntimesMessage, toolPackChecks, unansweredGoldenMessage } from "../../../conformance/port-readiness/recorded-adapter.js";
import { gatesRecordingSchema, RECORDING_FORMAT, type GatesCorpusRecording } from "../../../conformance/port-readiness/recording/schema.js";
import { assetName, type RecordingLock } from "../../../conformance/port-readiness/recording/store.js";
import type { CommandOptions, CommandResult, CommandRunner } from "../../../conformance/port-readiness/process.js";
import { renderMarkdown } from "../../../conformance/port-readiness/report.js";
import { blockingFailures, corpusPinsCheck, isCachedPack, isCachedRule, packDocuments, type CachedRule, type GatesReport, type RuleReport } from "../../../conformance/port-readiness/run.js";
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
    "matches-upstream-tests": { ...ok, totals: null, cases: {}, differences: [], goldens: { count: 0, differing: { count: 0, first: [] }, engineGaps: { count: 0, first: [] } } },
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

/** How python/bandit gets recorded, as a message that asks for a recording says it. */
const ROUTES = "in a pull request, a maintainer adds the record-port-readiness label (or runs port-readiness-record.yml by hand with the pull request's number and head sha); locally, npm run port-readiness:record -- --pack python --tool bandit";

/**
 * The comment's and the job summary's line for a tool-pack a recording of the pull request fixes, for one whose
 * runtimes changed, which only a recording of main fixes, and what follows either: where a record run's result shows.
 */
const LABEL_LINE = "`python/bandit` needs recording again: in a pull request, a maintainer adds the `record-port-readiness` label (or runs `port-readiness-record.yml` by hand with the pull request's number and head sha); on `main`, a maintainer runs `port-readiness-record.yml` from `main` with `pr` and `sha` empty.";
const MAIN_FIRST_LINE = "`python/bandit` needs recording on `main`: the record run takes the recorder's runtimes from `main`, so a change to them lands on `main` first, and a maintainer then runs `port-readiness-record.yml` from `main` with `pr` and `sha` empty.";
const RESULT_LINE = "A record run that works comments on the pull request with what it recorded (one for `main` writes that, and a link to open a pull request, to its job summary); one that fails says why in its log.";
const escaped = (text: string): string => text.replace(/[.*+?^${}()|[\]\\]/gu, "\\$&");

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

  it("a planned tool-pack with no report blocks, and says the logs have why", () => {
    const summary = summarize([], noUngated, gated, [{ pack: "python", tool: "ruff" }]);
    assert.deepEqual(summary.blocking.map((row) => [row.pack, row.rule, row.check]), [["python/ruff", "(every rule)", "report"]]);
    // A leg failed (in its fetch, its gates or a timeout) or its shards disagree in the merge; a recording the record
    // workflow never made is one way a leg fails, before it judges anything.
    assert.equal(summary.blocking[0]!.why, "no report: a leg failed or its shards disagree; the logs say why (one cause: its lock names a recording port-readiness-record.yml never made)");
    // The comment cuts a why to 160 characters, and this one has nothing to cut.
    assert.ok(summary.blocking[0]!.why.length <= 160);
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

  it("a known gap with no recorded parser verdict on its probe says the tool-pack needs recording again, and how", () => {
    const knownGaps: GatesReport["knownGaps"] = [{ id: "unrecorded", proven: false, kind: "not-recorded", excused: 0 }];
    const summary = summarize([{ ...report([rule("a", "shipped")]), knownGaps }], noUngated, []);
    assert.deepEqual(summary.blocking.map((row) => [row.check, row.why]), [["known-gap", `known gap unrecorded does not hold: no independent parser verdict on its probe is recorded, so the tool-pack needs recording again: ${ROUTES}`]]);
    assert.deepEqual(summary.toolPacks.map((entry) => entry.needsRecording), ["pull-request"]);
  });

  it("a tool-pack whose recording is stale needs recording again, and says how; one failing another tool-pack check does not", () => {
    const failing = (id: string, detail: string): GatesReport => ({ ...report([rule("a", "shipped")]), toolPackChecks: { pass: false, checks: [{ id, ok: false, detail }] } });
    const stale = summarize([failing("pins-agree", `gates.yaml upstream.version 1.9.5 but recording.lock pins.version 1.9.4: ${staleMessage("python", "bandit")}`)], noUngated, []);
    assert.deepEqual(stale.toolPacks.map((entry) => entry.needsRecording), ["pull-request"]);
    // The job summary is all a fork pull request gets, so the line is there as well as in the comment.
    // The job summary of a push to main and of the nightly has it too, where there is no pull request to label.
    // Neither promises a comment: a record run that fails before it publishes posts none.
    assert.match(renderSummary(stale), new RegExp(`^${escaped(`${LABEL_LINE} ${RESULT_LINE}`)}$`, "mu"));
    assert.doesNotMatch(renderSummary(stale), /what comes next/u);
    const duplicate = summarize([failing("bundled-load", "Duplicate bundled rule id: x")], noUngated, []);
    assert.deepEqual(duplicate.toolPacks.map((entry) => entry.needsRecording), [null]);
    assert.doesNotMatch(renderSummary(duplicate), /needs recording/u);
    // An edited case source is restored, not recorded again: recording refuses one that differs from upstream's.
    const edited = summarize([failing("case-sources", "1 case source(s) differ: fixtures/x.py is 0123456789ab, recorded ba9876543210: restore each edited one to the bytes it was recorded with (git log -p -- packs/python/upstream/bandit/fixtures shows the edits), or delete it so recording copies upstream's again")], noUngated, []);
    assert.deepEqual(edited.toolPacks.map((entry) => entry.needsRecording), [null]);
  });

  it("a corpus recorded on other corpus pins needs recording again, in the pull request", () => {
    // corpus-pins is runToolPack's own check: its detail is what needsRecording reads.
    const pinned = corpusPinsCheck("python", "bandit", { tasks: "0".repeat(64), corpora: [] }, "the tasks\n", []);
    const summary = summarize([{ ...report([rule("a", "shipped")]), toolPackChecks: { pass: false, checks: [pinned] } }], noUngated, []);
    assert.deepEqual(summary.toolPacks.map((entry) => entry.needsRecording), ["pull-request"]);
  });

  it("a tool-pack whose runtimes changed needs recording on main, since the record run takes the runtimes from there", () => {
    // Labelling the pull request would record with main's runtimes, and runtime-pins would fail again.
    const runtimes = (also: GatesReport["toolPackChecks"]["checks"] = []): GatesReport => ({
      ...report([rule("a", "shipped")]),
      toolPackChecks: { pass: false, checks: [{ id: "runtime-pins", ok: false, detail: `bandit-1.9.4/pyproject.toml is 0123456789ab, recorded ba9876543210: ${staleRuntimesMessage("python", "bandit")}` }, ...also] },
    });
    const summary = summarize([runtimes()], noUngated, []);
    assert.deepEqual(summary.toolPacks.map((entry) => entry.needsRecording), ["main-first"]);
    const text = renderSummary(summary);
    assert.match(text, new RegExp(`^${escaped(`${MAIN_FIRST_LINE} ${RESULT_LINE}`)}$`, "mu"));
    assert.doesNotMatch(text, /record-port-readiness/u);
    // A recording of main fixes the rest as well, so a tool-pack stale both ways is sent to main alone.
    const both = summarize([runtimes([{ id: "license-sha256", ok: false, detail: `LICENSE is missing: ${staleMessage("python", "bandit")}` }])], noUngated, []);
    assert.deepEqual(both.toolPacks.map((entry) => entry.needsRecording), ["main-first"]);
    // Each route names its own tool-packs.
    const ruff = { ...report([rule("b", "shipped")]), pack: "python", tool: "ruff", toolPack: "python/ruff", toolPackChecks: { pass: false, checks: [{ id: "pins-agree", ok: false, detail: `x: ${staleMessage("python", "ruff")}` }] } };
    const mixed = renderSummary(summarize([runtimes(), ruff], noUngated, []));
    assert.match(mixed, new RegExp(`^${escaped(`${LABEL_LINE.replace("python/bandit", "python/ruff")} ${MAIN_FIRST_LINE} ${RESULT_LINE}`)}$`, "mu"));
  });

  it("a tool-pack whose gates.yaml moved to a runtime its lock does not pin needs recording on main, though runtime-pins passes", () => {
    // A pull request moving python/bandit to bandit-1.9.5 adds that directory and keeps bandit-1.9.4 for any tool-pack
    // still on it (as next, react and react-hooks share eslint-config-next's), so runtime-pins, which checks what the lock
    // pins, passes. The record run installs the runtimes from main, which has no bandit-1.9.5, so labelling the pull
    // request would fail its leg: only pins-agree says so, and needsRecording reads its suffix.
    const root = tempRoot();
    const pins = { repository: "https://github.com/PyCQA/bandit", commit: "a".repeat(40), version: "1.9.4", runtime: "bandit-1.9.4" };
    write(root, "packs/python/upstream/bandit/gates.yaml", `adapter: bandit
rerail:
  pack: python
  project: bandit
upstream:
  name: PyCQA/bandit
  repository: ${pins.repository}
  commit: ${pins.commit}
  version: 1.9.4
  license: Apache-2.0
  recommended_configs:
    - default
  runtime: bandit-1.9.5
popularity:
  observed_on: 2026-10-06
  evidence:
    - a fake upstream
known_gaps: []
`);
    const digest = (text: string): string => createHash("sha256").update(text).digest("hex");
    const pinFiles = { "bandit-1.9.4": "bandit==1.9.4\n", "bandit-1.9.5": "bandit==1.9.5\n", python: "python\n" };
    for (const [runtime, text] of Object.entries(pinFiles)) write(root, join(RUNTIMES_DIRECTORY, runtime, "pyproject.toml"), text);
    write(root, "packs/python/licenses/bandit/LICENSE", "the license\n");
    const runtimes: RecordingLock["runtimes"] = { "bandit-1.9.4": { "pyproject.toml": digest(pinFiles["bandit-1.9.4"]) }, python: { "pyproject.toml": digest(pinFiles.python) } };
    const recording = gatesRecordingSchema.parse({
      format: RECORDING_FORMAT, tool: "bandit", adapter: "bandit", pins, runtimes, identity: { bandit: "1.9.4" }, packChecks: [], licenseSha256: digest("the license\n"),
      inScopeSamples: ["src/example.py"], corpus: { extensions: [".py"], caseSensitive: true }, keywords: [], commentVocabulary: [], fixtureWords: [], nameSets: {}, rules: {}, absent: [],
      parsers: [{ parser: "cpython 3.14.8", grammar: true }], parserVerdicts: [],
    });
    const lock = { tool: "bandit", pins, runtimes, files: { "recording.json": { sha256: "b".repeat(64), asset: assetName("python", "bandit", "recording.json", "b".repeat(64)) } } } as unknown as RecordingLock;
    const checks = toolPackChecks({ root, manifest: readGatesManifest(root, "python", "bandit"), lock, recording, corpus: { rules: [] } as unknown as GatesCorpusRecording, tools: [{ name: "bandit", version: "1.9.4" }], names: [] }, new Map(), new Map());
    assert.deepEqual(checks.filter((check) => !check.ok).map((check) => check.id), ["pins-agree"]);
    const summary = summarize([{ ...report([rule("a", "shipped")]), toolPackChecks: { pass: false, checks } }], noUngated, []);
    assert.deepEqual(summary.toolPacks.map((entry) => entry.needsRecording), ["main-first"]);
    assert.match(renderSummary(summary), new RegExp(`^${escaped(`${MAIN_FIRST_LINE} ${RESULT_LINE}`)}$`, "mu"));
  });

  it("a golden corpus.json does not answer needs recording again, in the pull request", () => {
    // A goldens.tsv the pull request extended, before the record run answered its new files.
    const unanswered = rule("a", "shipped");
    const reason = `it has no expected answer: ${unansweredGoldenMessage("python", "bandit")}`;
    const golden = { task: "t1", path: "src/new.py", expected: 0, status: "mismatch" as const, reason, actual: 0, missing: [], extra: [], failures: [] };
    const matches = { ...unanswered.gates["matches-upstream-tests"], pass: false, checks: [{ id: "identical-on-goldens", ok: false, detail: `1 of 1 golden(s) differ, e.g. t1/src/new.py: ${reason}` }], goldens: { count: 1, differing: { count: 1, first: [golden] }, engineGaps: { count: 0, first: [] } } };
    const failing: RuleReport = { ...unanswered, pass: false, stoppedAt: "matches-upstream-tests", gates: { ...unanswered.gates, "matches-upstream-tests": matches } };
    const summary = summarize([report([failing])], noUngated, []);
    assert.deepEqual(summary.toolPacks.map((entry) => entry.needsRecording), ["pull-request"]);
    // A candidate's unanswered golden holds it back the same way.
    const candidate = summarize([report([{ ...failing, location: "candidate" }])], noUngated, []);
    assert.deepEqual(candidate.toolPacks.map((entry) => entry.needsRecording), ["pull-request"]);
  });

  it("the comment shortens a why to a table cell, and points a route it would cut at the line that names it whole", () => {
    // A shipped rule blocked by stale tool-pack checks: its why is the failing checks, each ending with a route.
    const blockedBy = `pins-agree: 1.9.5, locked 1.9.4: ${staleMessage("python", "bandit")}; license-sha256: LICENSE is missing: ${staleMessage("python", "bandit")}`;
    const stale = (why: string) => ({
      ...report([{ ...rule("a", "shipped"), pass: false, blockedBy: why }]),
      toolPackChecks: { pass: false, checks: [{ id: "license-sha256", ok: false, detail: `LICENSE is missing: ${staleMessage("python", "bandit")}` }] },
    });
    const comment = renderComment(summarize([stale(blockedBy)], noUngated, []));
    assert.ok(comment.includes(`${LABEL_LINE} ${RESULT_LINE}`), comment);
    const row = (text: string) => text.split("\n").find((line) => line.includes("| `a` |"))!;
    assert.ok(row(comment).includes("| pins-agree: 1.9.5, locked 1.9.4: the recording is stale (how: see above); license-sha256: LICENSE is missing: the recording is stale (how: see above) |"), row(comment));
    // A why still too long once its routes are swapped is cut, and no half route is left in it.
    const long = renderComment(summarize([stale(`pins-agree: ${"x".repeat(200)}: ${staleMessage("python", "bandit")}`)], noUngated, []));
    assert.match(row(long), /\| pins-agree: x+… \|$/u);
    assert.doesNotMatch(row(long), /a maintainer|npm run/u);
    // The job summary has room for the whole why, routes and all.
    assert.ok(renderSummary(summarize([stale(blockedBy)], noUngated, [])).includes(blockedBy));
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
  it("says a failing tool-pack check fails every shipped rule, and leaves what is wrong to the check", () => {
    // bundled-load, case-sources with an edited source and corpus-complete fail for reasons a recording does not fix.
    const failing = { ...report([rule("a", "shipped")]), toolPackChecks: { pass: false, checks: [{ id: "bundled-load", ok: false, detail: "Duplicate bundled rule id: x" }] } };
    const markdown = renderMarkdown(failing);
    assert.ok(markdown.includes("A tool-pack check fails, so every shipped rule of this tool-pack fails and none of its candidates is ready to promote: the failing check's detail says what is wrong, and how to record the tool-pack again when a recording fixes it."), markdown);
    assert.doesNotMatch(markdown, /recording is stale/u);
  });

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
