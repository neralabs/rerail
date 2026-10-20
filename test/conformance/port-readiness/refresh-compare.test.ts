/**
 * `--refresh-cache`, the nightly's run: every rule is judged again, and each fresh result is compared with the cached one
 * under the same key before it overwrites it. A verdict that differs means the key leaves out something the verdict
 * depends on, so it fails the run, a candidate's as well as a shipped rule's, and the fresh result is saved.
 */
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";

import { afterAll, describe, it } from "vitest";
import { parse } from "yaml";

import { PACKAGE_ROOT } from "../../../src/engine/index.js";
import { ProcessCommandRunner } from "../../../conformance/port-readiness/process.js";
import { CORPUS_FORMAT, RECORDING_FORMAT } from "../../../conformance/port-readiness/recording/schema.js";
import { buildRecordingLock, recordingLockText, writeCachedRecording } from "../../../conformance/port-readiness/recording/store.js";
import { driftLine, renderMarkdown } from "../../../conformance/port-readiness/report.js";
import { blockingFailures, runToolPack, verdictDrift, type CachedRule, type RuleReport } from "../../../conformance/port-readiness/run.js";
import { renderSummary, summarize } from "../../../conformance/port-readiness/summary.js";

const temporary: string[] = [];
afterAll(() => {
  for (const directory of temporary.splice(0)) rmSync(directory, { recursive: true, force: true });
});

const PINS = { repository: "https://github.com/PyCQA/bandit", commit: "a".repeat(40), version: "1.9.4", runtime: "bandit-1.9.4" };

const MANIFEST = `adapter: bandit
rerail:
  pack: demo
  project: bandit
upstream:
  name: PyCQA/bandit
  repository: ${PINS.repository}
  commit: ${PINS.commit}
  version: ${PINS.version}
  license: Apache-2.0
  recommended_configs:
    - default
  runtime: ${PINS.runtime}
popularity:
  observed_on: 2026-10-06
  evidence:
    - a fake upstream
known_gaps: []
`;

const ID = "bandit-b101-assert-used";
const FILE = `packs/demo/candidates/rules/${ID}.json`;

/**
 * A checkout of its own holding one gated tool-pack, demo/bandit, with one candidate (python's B101 document) and its
 * recordings in the cache. The recording does not cover B101, so the tool-pack checks fail and the candidate stops at
 * not-overfit, which blocks nothing: a candidate's verdict, as the gates reach it from the recordings.
 */
async function toolPack(): Promise<{ root: string; cacheRoot: string }> {
  const root = mkdtempSync(join(tmpdir(), "gates-refresh-"));
  temporary.push(root);
  const cacheRoot = join(root, ".cache");
  const write = (path: string, text: string) => {
    mkdirSync(join(root, dirname(path)), { recursive: true });
    writeFileSync(join(root, path), text);
  };
  // The gate code and the engine are keyed on what git tracks, and the npm side on the package files.
  execFileSync("git", ["init", "-q"], { cwd: root });
  copyFileSync(join(PACKAGE_ROOT, "package.json"), join(root, "package.json"));
  copyFileSync(join(PACKAGE_ROOT, "package-lock.json"), join(root, "package-lock.json"));
  write("packs/demo/upstream/bandit/gates.yaml", MANIFEST);
  write(FILE, readFileSync(join(PACKAGE_ROOT, "packs/python/rules/bandit-b101-assert-used.json"), "utf8"));
  write("packs/demo/pack.json", `${JSON.stringify({ rules: [] })}\n`);
  write("packs/demo/conformance.json", `${JSON.stringify({
    pack: "demo", family: "python", capabilities: ["syntax"], corpora: [], tools: [{ name: "bandit", version: PINS.version }],
    rules: [{ id: ID, upstream: { tool: "bandit", rule: "B101" }, policy: `candidates/rules/${ID}.json`, capabilities: ["syntax"], match: "line" }],
  })}\n`);
  const parsers = [{ parser: "cpython 3.14.8", grammar: true }];
  const recording = JSON.stringify({
    format: RECORDING_FORMAT, tool: "bandit", adapter: "bandit", pins: PINS, runtimes: {}, identity: { bandit: PINS.version }, packChecks: [],
    licenseSha256: "0".repeat(64), inScopeSamples: ["src/example.py"], corpus: { extensions: [".py"], caseSensitive: true }, keywords: ["import"],
    commentVocabulary: ["nosec"], fixtureWords: ["os"], nameSets: {}, rules: {}, absent: [], parsers, parserVerdicts: [],
  });
  const corpus = JSON.stringify({ format: CORPUS_FORMAT, tool: "bandit", pins: { tasks: "c".repeat(64), corpora: [] }, rules: [], parsers, files: [] });
  await writeCachedRecording(cacheRoot, recording);
  await writeCachedRecording(cacheRoot, corpus);
  write("packs/demo/upstream/bandit/recording.lock", recordingLockText(buildRecordingLock("demo", "bandit", "port-readiness-recordings", "test", PINS, {}, { "recording.json": recording, "corpus.json": corpus })));
  return { root, cacheRoot };
}

type Mode = "cached" | "refresh" | "no-cache";

/** One run of the tool-pack, with the cache options cli.ts sets for `mode` on a binary that is not stale. */
function judge(where: { root: string; cacheRoot: string }, mode: Mode) {
  const cache = { cached: { readCache: true, writeCache: true, compareCache: false }, refresh: { readCache: false, writeCache: true, compareCache: true }, "no-cache": { readCache: false, writeCache: false, compareCache: false } }[mode];
  return runToolPack({ ...where, pack: "demo", tool: "bandit", ...cache, runner: new ProcessCommandRunner() });
}

/** The rule's cached result file: the one entry under the tool-pack that is not the tool-pack checks'. */
function cachedFile(cacheRoot: string): string {
  const directory = join(cacheRoot, "results", "demo", "bandit");
  const entries = readdirSync(directory).filter((name) => name.endsWith(".json") && !name.startsWith("pack-"));
  assert.equal(entries.length, 1);
  return join(directory, entries[0]!);
}

/** Edit the rule's cached result in place, under the key it is cached by. */
function editCached(cacheRoot: string, edit: (value: CachedRule) => void): void {
  const path = cachedFile(cacheRoot);
  const entry = JSON.parse(readFileSync(path, "utf8")) as { format: string; value: CachedRule };
  edit(entry.value);
  writeFileSync(path, `${JSON.stringify(entry)}\n`);
}

const readCachedValue = (cacheRoot: string): CachedRule => (JSON.parse(readFileSync(cachedFile(cacheRoot), "utf8")) as { value: CachedRule }).value;

describe("a refresh run compares each fresh verdict with the cached one under its key", () => {
  it("fails, naming the rule, its file and the gate, when a cached gate differs, and saves the fresh result", async () => {
    const where = await toolPack();
    const seeded = await judge(where, "cached");
    assert.deepEqual(seeded.cache, { rules: 1, hits: 0, compared: 0, differing: [] });
    assert.equal(seeded.rules[0]!.stoppedAt, "not-overfit");
    assert.equal(seeded.rules[0]!.gates.compact.pass, true);
    // A candidate that stops at a gate blocks nothing.
    assert.equal(blockingFailures(seeded), 0);

    // A stale failure under the rule's current key: what the cache holds if the key missed an input that moved.
    editCached(where.cacheRoot, (value) => {
      value.report.gates.compact = { ...value.report.gates.compact, pass: false, checks: [{ id: "size", ok: false, detail: "an earlier engine's measure" }] };
    });
    const refreshed = await judge(where, "refresh");
    const drift = { id: ID, file: FILE, location: "candidate", what: "compact", cached: "fail", fresh: "pass" };
    assert.deepEqual(refreshed.cache, { rules: 1, hits: 0, compared: 1, differing: [drift] });
    // The candidate's verdict is unchanged, and the difference alone fails the run.
    assert.equal(refreshed.rules[0]!.stoppedAt, "not-overfit");
    assert.equal(blockingFailures(refreshed), 1);
    assert.equal(driftLine(refreshed.cache.differing![0]!), `${ID} (${FILE}) compact: cached fail, fresh pass`);
    assert.match(renderMarkdown(refreshed), /^- Cache: 0 of 1 rule result\(s\) from the cache; compared 1; 1 differ$/mu);
    assert.match(renderMarkdown(refreshed), new RegExp(`^  - ${ID} \\(${FILE.replaceAll(".", "\\.")}\\) compact: cached fail, fresh pass$`, "mu"));

    // The fresh result replaced the stale one, so the next refresh compares it and finds nothing.
    assert.equal(readCachedValue(where.cacheRoot).report.gates.compact.pass, true);
    const again = await judge(where, "refresh");
    assert.deepEqual(again.cache, { rules: 1, hits: 0, compared: 1, differing: [] });
    assert.equal(blockingFailures(again), 0);
  });

  it("ignores free-text details and the passes-conformance placeholder", async () => {
    const where = await toolPack();
    await judge(where, "cached");
    editCached(where.cacheRoot, (value) => {
      value.report.gates.loads.checks[0]!.detail = "/tmp/gates-old/packs/demo: a path of another run";
      value.report.gates["passes-conformance"] = { pass: false, checks: [{ id: "scored", ok: false, detail: "judged fresh on every run" }] };
      value.report.defaultOnReason = "another wording";
    });
    const refreshed = await judge(where, "refresh");
    assert.deepEqual(refreshed.cache, { rules: 1, hits: 0, compared: 1, differing: [] });
  });

  it("compares no entry that names another document, and none at all without --refresh-cache", async () => {
    const where = await toolPack();
    await judge(where, "cached");
    editCached(where.cacheRoot, (value) => {
      value.report.gates.compact = { ...value.report.gates.compact, pass: false };
    });
    // --no-cache neither reads, saves nor compares; a cached run reads the entry as a hit.
    assert.deepEqual((await judge(where, "no-cache")).cache, { rules: 1, hits: 0, compared: 0, differing: [] });
    const hit = await judge(where, "cached");
    assert.deepEqual(hit.cache, { rules: 1, hits: 1, compared: 0, differing: [] });
    assert.equal(hit.rules[0]!.gates.compact.pass, false);
    // An entry for another document under this key is a miss when read, and is not compared either.
    editCached(where.cacheRoot, (value) => {
      value.report.file = "packs/demo/candidates/rules/other.json";
    });
    assert.deepEqual((await judge(where, "refresh")).cache, { rules: 1, hits: 0, compared: 0, differing: [] });
  });

  it("puts one blocking row per difference in the summary, a candidate's included", async () => {
    const where = await toolPack();
    await judge(where, "cached");
    editCached(where.cacheRoot, (value) => {
      value.report.gates.compact = { ...value.report.gates.compact, pass: false };
      value.report.defaultOn = true;
    });
    const report = await judge(where, "refresh");
    const summary = summarize([report], { checks: [], notGated: [], pending: [] }, [{ pack: "demo", tool: "bandit" }]);
    assert.deepEqual(summary.blocking.map((row) => [row.pack, row.rule, row.stoppedAt, row.check]), [
      ["demo/bandit", ID, "Cache", "compact"],
      ["demo/bandit", ID, "Cache", "defaultOn"],
    ]);
    assert.match(summary.blocking[0]!.why, new RegExp(`^${FILE.replaceAll(".", "\\.")}: cached fail, fresh pass under the same key`, "u"));
    assert.equal(summary.pass, false);
    assert.match(renderSummary(summary), /\*\*0 of 0 shipped rules pass; 2 blocking failure\(s\);/u);
  });
});

function gates(): RuleReport["gates"] {
  const ok = { pass: true, checks: [{ id: "check", ok: true, detail: "holds" }] };
  return {
    "upstream-status": { checks: [{ id: "recommended", ok: true, detail: "on in default" }] },
    loads: ok,
    "not-overfit": { ...ok, flagged: [], words: {}, uncheckedRegexes: [] },
    compact: { ...ok, metrics: null },
    "matches-upstream-tests": { ...ok, totals: null, cases: {}, differences: [], goldens: { count: 0, differing: { count: 0, first: [] }, engineGaps: { count: 0, first: [] } } },
    "passes-conformance": { pass: true, checks: [] },
  };
}

/** A shipped rule's cached result that passes every gate and starts on. */
function cachedRule(): CachedRule {
  return {
    report: {
      id: "r", file: "packs/demo/rules/r.json", location: "shipped", upstreamRule: "R1", sha256: "0", bytes: 1, policyDigest: null,
      pass: true, stoppedAt: null, blockedBy: null, defaultOn: true, defaultOnReason: "upstream turns it on", on: true, gates: gates(),
    },
    gapUses: {},
    integrity: [],
    gapCandidates: [],
  };
}

describe("verdictDrift", () => {
  const where = { id: "r", file: "packs/demo/rules/r.json", location: "shipped" };

  it("reports a flipped gate, and the verdict it moves", () => {
    const fresh = cachedRule();
    fresh.report.gates.loads = { pass: false, checks: [{ id: "schema", ok: false, detail: "schema failed" }] };
    Object.assign(fresh.report, { pass: false, stoppedAt: "loads" });
    assert.deepEqual(verdictDrift(cachedRule(), fresh), [
      { ...where, what: "verdict", cached: "passes", fresh: "stops at loads" },
      { ...where, what: "loads", cached: "pass", fresh: "fail" },
    ]);
    // A gate behind the one the rule stops at moves no verdict.
    const later = structuredClone(fresh);
    later.report.gates.compact = { ...later.report.gates.compact, pass: false };
    assert.deepEqual(verdictDrift(fresh, later), [{ ...where, what: "compact", cached: "pass", fresh: "fail" }]);
    // A cached entry whose pass and stoppedAt disagree, which no verdict writes, differs from the fresh verdict.
    const torn = cachedRule();
    torn.report.pass = false;
    assert.deepEqual(verdictDrift(torn, cachedRule()), [{ ...where, what: "verdict", cached: "pass false, stoppedAt null", fresh: "passes" }]);
  });

  it("reports nothing when only a detail differs", () => {
    const fresh = cachedRule();
    fresh.report.gates.loads.checks[0]!.detail = "/tmp/gates-x/packs/demo/rules/r.json compiles";
    fresh.report.gates["upstream-status"].checks[0]!.detail = "on in another wording";
    fresh.report.defaultOnReason = "upstream has it";
    assert.deepEqual(verdictDrift(cachedRule(), fresh), []);
  });

  it("reports a defaultOn flip", () => {
    const fresh = cachedRule();
    fresh.report.gates["upstream-status"].checks[0]!.ok = false;
    fresh.report.defaultOn = false;
    assert.deepEqual(verdictDrift(cachedRule(), fresh), [{ ...where, what: "defaultOn", cached: "on", fresh: "off" }]);
  });

  it("ignores passes-conformance, which the cached form holds as a placeholder", () => {
    const fresh = cachedRule();
    fresh.report.gates["passes-conformance"] = { pass: false, checks: [{ id: "scored", ok: false, detail: "judged on every run" }] };
    assert.deepEqual(verdictDrift(cachedRule(), fresh), []);
  });
});

describe("the workflow", () => {
  it("restores main's results for a no-cache run too, so the refresh has something to compare with", () => {
    const workflow = parse(readFileSync(join(PACKAGE_ROOT, ".github", "workflows", "port-readiness.yml"), "utf8")) as { jobs: { gates: { steps: { name?: string; if?: unknown }[] } } };
    const restore = workflow.jobs.gates.steps.filter((step) => step.name === "Restore earlier rule results");
    assert.equal(restore.length, 1);
    assert.equal(restore[0]!.if, undefined);
  });
});
