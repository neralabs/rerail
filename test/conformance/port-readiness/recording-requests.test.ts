/**
 * Every message that asks for a recording names both ways to get one. Pull request CI takes only a recording
 * port-readiness-record.yml made and attested, and only someone who can write here starts that run, so in a pull
 * request a maintainer does it; a recording made locally is enough for the gates on that machine and never for CI.
 * The record run takes the recorder's runtimes and its list of tool-packs from main, so a change to either is
 * recorded once it is on main, never through the pull request that makes it.
 */
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, test, vi } from "vitest";

import { readGatesManifest } from "../../../conformance/port-readiness/manifest.js";
import { createRecordedAdapter, rebuildFixtureTexts, staleMessage, staleRuntimesMessage, toolPackChecks, unansweredGoldenMessage } from "../../../conformance/port-readiness/recorded-adapter.js";
import { fetchToolPackRecordings } from "../../../conformance/port-readiness/recording/fetch.js";
import { mainRecordingRoutes } from "../../../conformance/port-readiness/recording/routes.js";
import { corpusPinsCheck } from "../../../conformance/port-readiness/run.js";
import { CORPUS_FORMAT, gatesCorpusSchema, gatesRecordingSchema, RECORDING_FORMAT, recordingText } from "../../../conformance/port-readiness/recording/schema.js";
import { buildRecordingLock, gzipRecording, readRecordingLock, RECORDING_LOCK, recordingLockText } from "../../../conformance/port-readiness/recording/store.js";

/** The two routes, written out here so a change to their wording is a change to this test. */
const ROUTES = "in a pull request, a maintainer adds the record-port-readiness label (or runs port-readiness-record.yml by hand with the pull request's number and head sha); locally, npm run port-readiness:record -- --pack demo --tool bandit";
/** The route for a change to the recorder's runtimes, which the record run takes from main. */
const RUNTIME_ROUTES = "a change to the recorder's runtimes is recorded once it is on main, since the record run takes the runtimes from main: it lands on main first, and a maintainer then runs port-readiness-record.yml from main with pr and sha empty; locally, npm run port-readiness:record -- --pack demo --tool bandit";
/** The route for a new tool-pack, which the record run records only once main's TOOL_PACKS lists it. */
const NEW_TOOL_PACK_ROUTES = "a new tool-pack is recorded once it is on main, since the record run records only the tool-packs main's port-readiness-record.yml lists in TOOL_PACKS: it lands on main first, with its entry there, and a maintainer then runs port-readiness-record.yml from main with pr and sha empty; locally, npm run port-readiness:record -- --pack demo --tool bandit";

const PINS = { repository: "https://github.com/PyCQA/bandit", commit: "a".repeat(40), version: "1.9.4", runtime: "bandit-1.9.4" };
const PARSERS = [{ parser: "cpython 3.14.8", grammar: true }];

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

const RECORDING = gatesRecordingSchema.parse({
  format: RECORDING_FORMAT,
  tool: "bandit",
  adapter: "bandit",
  pins: PINS,
  runtimes: {},
  identity: { bandit: "1.9.4" },
  packChecks: [],
  licenseSha256: "0".repeat(64),
  inScopeSamples: ["src/example.py"],
  corpus: { extensions: [".py"], caseSensitive: true },
  keywords: [],
  commentVocabulary: [],
  fixtureWords: [],
  nameSets: {},
  rules: {},
  absent: ["B301"],
  parsers: PARSERS,
  parserVerdicts: [],
});

const CORPUS = gatesCorpusSchema.parse({
  format: CORPUS_FORMAT,
  tool: "bandit",
  pins: { tasks: "c".repeat(64), corpora: [] },
  rules: [],
  parsers: PARSERS,
  files: [{ task: "demo", path: "app.py", sha256: "b".repeat(64), excluded: null, findings: {} }],
});

const TEXTS = { "recording.json": recordingText(RECORDING), "corpus.json": recordingText(CORPUS) };

const temporary: string[] = [];
afterEach(() => {
  vi.unstubAllGlobals();
  for (const directory of temporary.splice(0)) rmSync(directory, { recursive: true, force: true });
});

/** A gated tool-pack directory with its gates.yaml and, unless `locked` is false, a lock naming TEXTS. */
function toolPack(locked = true) {
  const root = mkdtempSync(join(tmpdir(), "port-readiness-requests-"));
  temporary.push(root);
  mkdirSync(join(root, "packs/demo/upstream/bandit"), { recursive: true });
  writeFileSync(join(root, "packs/demo/upstream/bandit/gates.yaml"), MANIFEST);
  const lock = buildRecordingLock("demo", "bandit", "port-readiness-recordings", "9.9.9", PINS, {}, TEXTS);
  if (locked) writeFileSync(join(root, "packs/demo/upstream/bandit", RECORDING_LOCK), recordingLockText(lock));
  return { root, lock, cacheRoot: join(root, "cache") };
}

test("a stale recording says how to record the tool-pack again, in a pull request and locally", () => {
  assert.equal(staleMessage("demo", "bandit"), `the recording is stale: ${ROUTES}`);
});

test("a recording stale for a change to the recorder's runtimes says it is recorded once the change is on main", () => {
  // The record run checks out the recorder and its runtimes from main, so labelling the pull request records nothing new.
  assert.equal(staleRuntimesMessage("demo", "bandit"), `the recording is stale: ${RUNTIME_ROUTES}`);
});

test("every recording check that fails says the recording is stale, and how to record it again", () => {
  // The comment's "needs recording again" line reads these suffixes (summary.ts needsRecording), so each check keeps
  // one: runtime-pins, and pins-agree on a runtime the lock does not pin, the one for a change to the runtimes, which a
  // pull request's recording cannot pick up.
  const { root } = toolPack(false);
  const recording = gatesRecordingSchema.parse({
    ...RECORDING,
    // A runtime pin the tree does not commit (runtime-pins), and not python, which gates.yaml installs (pins-agree).
    runtimes: { "bandit-1.9.4": { "pyproject.toml": "d".repeat(64) } },
    // A case source the tree does not commit (case-sources); no LICENSE is committed either (license-sha256).
    rules: { B301: { facts: { name: "B301", implementation: [], tests: [], deprecation: null, recommended: null, notOnRealCode: null }, names: { implementation: { sets: [], names: [], regexes: [] }, shared: { sets: [], names: [], regexes: [] } }, spanParts: { endLine: true, endCol: true }, cases: [{ fixture: "examples/pickle.py", variant: "default", path: "pickle.py", skipped: null, expected: [], markup: null, file: "fixtures/pickle.py", sha256: "e".repeat(64) }] } },
    absent: [],
  });
  // B302 is recorded nowhere (recording-covers-rules), and there is no corpus.json (corpus-covers-rules).
  const checks = toolPackChecks({ root, manifest: readGatesManifest(root, "demo", "bandit"), lock: null, recording, corpus: null, tools: [{ name: "bandit", version: "1.9.4" }], names: ["B301", "B302"] });
  assert.deepEqual(checks.filter((check) => !check.ok).map((check) => check.id), ["pins-agree", "runtime-pins", "license-sha256", "case-sources", "recording-covers-rules", "corpus-covers-rules"]);
  for (const check of checks.filter((entry) => !entry.ok)) {
    const stale = check.id === "runtime-pins" || check.id === "pins-agree" ? staleRuntimesMessage("demo", "bandit") : staleMessage("demo", "bandit");
    assert.ok(check.detail.endsWith(stale), `${check.id}: ${check.detail}`);
  }
});

test("corpus-pins fails on other corpus pins, or none, and says the recording is stale and how to record it again", () => {
  // runToolPack makes this check itself, and the comment's "needs recording again" line reads its suffix.
  const tasks = "the corpus tasks\n";
  const corpora = [{ name: "demo", repository: "https://github.com/demo/demo", commit: "a".repeat(40), paths: ["src"], exclude: [] }];
  const recorded = { tasks: createHash("sha256").update(tasks).digest("hex"), corpora };
  assert.deepEqual(corpusPinsCheck("demo", "bandit", recorded, tasks, corpora), { id: "corpus-pins", ok: true, detail: "corpus.json was recorded on this branch's pinned tasks and corpora" });
  for (const pins of [{ ...recorded, tasks: "0".repeat(64) }, { ...recorded, corpora: [] }, null]) {
    assert.deepEqual(corpusPinsCheck("demo", "bandit", pins, tasks, corpora), { id: "corpus-pins", ok: false, detail: `corpus.json was recorded on other corpus pins: ${staleMessage("demo", "bandit")}` });
  }
});

test("a stale recording of main says how to record main again, and locally", () => {
  assert.equal(mainRecordingRoutes("demo", "bandit"), "on main, a maintainer runs port-readiness-record.yml from main with pr and sha empty; locally, npm run port-readiness:record -- --pack demo --tool bandit");
});

test("a gated tool-pack without a lock says so, and that a new tool-pack is recorded once it is on main", () => {
  // The record run's legs come from main's TOOL_PACKS, so the label on the pull request that adds one records nothing.
  const { root } = toolPack(false);
  assert.throws(() => readRecordingLock(root, "demo", "bandit"), { message: `packs/demo/upstream/bandit/${RECORDING_LOCK} is missing: ${NEW_TOOL_PACK_ROUTES}` });
});

test("a lock naming an asset the release lacks says CI never takes a local recording, and how to get one recorded", async () => {
  const { root, cacheRoot, lock } = toolPack();
  // The release as the API answers for a lock a contributor recorded locally and committed: none of its assets.
  vi.stubGlobal("fetch", async () => new Response(JSON.stringify({ assets: [] }), { status: 200, headers: { "content-type": "application/json" } }));
  await assert.rejects(fetchToolPackRecordings(root, cacheRoot, "demo", "bandit", { repository: "neralabs/rerail" }), {
    message: `the release port-readiness-recordings of neralabs/rerail has no asset ${lock.files["recording.json"].asset}: the recording this commit names was never uploaded (CI takes only a recording port-readiness-record.yml uploaded and attested, never a local one): ${ROUTES}`,
  });
});

test("an asset the record workflow did not attest says how to get the tool-pack recorded", async () => {
  const { root, cacheRoot, lock } = toolPack();
  const download = async (asset: { asset: string }) => gzipRecording(asset.asset === lock.files["recording.json"].asset ? TEXTS["recording.json"] : TEXTS["corpus.json"]);
  // No GitHub CLI to verify with, so the attestation check fails as it does for an asset uploaded by hand.
  const path = process.env.PATH;
  process.env.PATH = join(root, "no-bin");
  try {
    await assert.rejects(fetchToolPackRecordings(root, cacheRoot, "demo", "bandit", { download }), (error: Error) => {
      assert.ok(error.message.startsWith(`the release's ${lock.files["recording.json"].asset} is not verified as recorded by .github/workflows/port-readiness-record.yml on refs/heads/main`), error.message);
      // The route ends the message, so the command copies without a full stop on its last argument.
      assert.ok(error.message.endsWith(`If that workflow did not record it, the tool-pack needs recording again: ${ROUTES}`), error.message);
      return true;
    });
  } finally {
    process.env.PATH = path;
  }
});

test("a golden corpus.json does not answer says the tool-pack needs recording again, and how", async () => {
  const { root } = toolPack(false);
  const adapter = createRecordedAdapter({ root, manifest: readGatesManifest(root, "demo", "bandit"), lock: null, recording: RECORDING, corpus: CORPUS, tools: [{ name: "bandit", version: "1.9.4" }], names: [] });
  const [answer] = await adapter.lintCorpus([{ task: "demo", repository: "https://example.test/demo", path: "new.py", source: "x = 1\n", sha256: "e".repeat(64) }], []);
  assert.equal(answer!.excluded, `corpus.json has no answer for it with this sha256, so the tool-pack needs recording again: ${ROUTES}`);
  // summary.ts reads it by this message, to name the tool-pack in the comment's "needs recording again" line.
  assert.equal(answer!.excluded, unansweredGoldenMessage("demo", "bandit"));
});

test("upstream's test files the recording names but its pinned commit does not hold say the tool-pack needs recording again, and how", () => {
  const checkout = mkdtempSync(join(tmpdir(), "port-readiness-requests-checkout-"));
  temporary.push(checkout);
  mkdirSync(join(checkout, "tests"), { recursive: true });
  writeFileSync(join(checkout, "tests/a.py"), "x = 1\n");
  const recorded = (sha256: string) => ({ files: [{ path: "tests/a.py", sha256 }], inline: [] });
  assert.throws(() => rebuildFixtureTexts(checkout, recorded("f".repeat(64)), { pack: "demo", tool: "bandit" }), {
    message: `upstream's tests/a.py has sha256 ${createHash("sha256").update("x = 1\n").digest("hex").slice(0, 12)} at the pinned commit, but recording.json recorded ffffffffffff, so the tool-pack needs recording again: ${ROUTES}`,
  });
  assert.throws(() => rebuildFixtureTexts(checkout, { files: [{ path: "tests/gone.py", sha256: "f".repeat(64) }], inline: [] }, { pack: "demo", tool: "bandit" }), {
    message: `upstream's tests/gone.py is not in its checkout at the pinned commit (${checkout}), though recording.json names it, so the tool-pack needs recording again: ${ROUTES}`,
  });
  // The recorder rebuilds what it just recorded, where a difference is its own bug and no recording fixes it.
  assert.throws(() => rebuildFixtureTexts(checkout, recorded("f".repeat(64))), { message: /recording\.json recorded ffffffffffff$/u });
});
