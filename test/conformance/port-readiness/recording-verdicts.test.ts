/**
 * The recording's parser verdicts: one file per source the recorder asked the independent parsers about, named by the
 * source's sha256, file type and script (a case source and a golden with the same text share it), `[]` when every parser
 * was silent. What the gates must keep telling apart is a file that says every parser was silent from no file at all
 * (a source nobody recorded, on which the gates draw no conclusion) — the second is what a golden whose text changed
 * looks like.
 */
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "vitest";

import { readGatesManifest } from "../../../conformance/port-readiness/manifest.js";
import { createRecordedAdapter } from "../../../conformance/port-readiness/recorded-adapter.js";
import { answerConfig, configId, memoryTree, parsersId } from "../../../conformance/port-readiness/recording/archive.js";
import { answerPath, answerText, parseVerdicts, verdictPath, verdictText } from "../../../conformance/port-readiness/recording/corpus-answers.js";
import { canonicalText, cleanVerdicts, gatesRecordingSchema, isCleanVerdict, recordedParsers } from "../../../conformance/port-readiness/recording/schema.js";

const MANIFEST = `adapter: bandit
rerail:
  pack: demo
  project: bandit
upstream:
  name: PyCQA/bandit
  repository: https://github.com/PyCQA/bandit
  commit: ${"a".repeat(40)}
  version: 1.9.4
  license: Apache-2.0
  recommended_configs:
    - default
  runtime: bandit-1.9.4
popularity:
  observed_on: 2026-10-06
  evidence:
    - a fake upstream
known_gaps: []
`;

const SILENT = "import os\n";
const BROKEN = "def (:\n";
const digest = (text: string): string => createHash("sha256").update(text).digest("hex");

/** cpython checks the grammar; libcst makes no grammar checks at all, so its clean verdict has `grammar: null`. */
const PARSERS = [{ parser: "cpython 3.14.8", grammar: true }, { parser: "libcst 1.9.0", grammar: false }];
const OBJECTING = [{ parser: "cpython 3.14.8", syntax: ["invalid syntax"], grammar: [] }, { parser: "libcst 1.9.0", syntax: ["parse error"], grammar: null }];

function recording(rules: Record<string, unknown> = {}): unknown {
  return {
    tool: "bandit",
    adapter: "bandit",
    pins: { repository: "https://github.com/PyCQA/bandit", commit: "a".repeat(40), version: "1.9.4", runtime: "bandit-1.9.4" },
    runtimes: {},
    identity: { bandit: "1.9.4" },
    packChecks: [],
    licenseSha256: "0".repeat(64),
    inScopeSamples: ["src/example.py"],
    corpus: { extensions: [".py"], caseSensitive: true },
    keywords: ["import"],
    commentVocabulary: ["nosec"],
    fixtureWords: ["os"],
    nameSets: {},
    rules,
    absent: [],
    parsers: PARSERS,
  };
}

/** A tree holding the parsers' verdict on each of `files`' content, read at its path. */
function corpus(files: { path: string; source: string; script?: boolean; verdicts: Parameters<typeof verdictText>[0] }[]) {
  const tree = new Map(files.map((file) => [verdictPath(parsersId("bandit", PARSERS), { path: file.path, sha256: digest(file.source), script: file.script === true }), Buffer.from(verdictText(file.verdicts), "utf8")]));
  return { tree: memoryTree(tree), commitOf: (task: string): string => { throw new Error(`no commit for ${task}`); } };
}

/** A recorded adapter over the recording and the verdict files, with a tool-pack directory the checks can look at. */
function adapterOver(files: Parameters<typeof corpus>[0]) {
  const root = mkdtempSync(join(tmpdir(), "gates-verdicts-"));
  mkdirSync(join(root, "packs/demo/upstream/bandit"), { recursive: true });
  writeFileSync(join(root, "packs/demo/upstream/bandit/gates.yaml"), MANIFEST);
  writeFileSync(join(root, "packs/demo/conformance.json"), JSON.stringify({ tools: [{ name: "bandit", version: "1.9.4" }] }));
  const manifest = readGatesManifest(root, "demo", "bandit");
  const adapter = createRecordedAdapter({
    root,
    manifest,
    lock: null,
    recording: gatesRecordingSchema.parse(recording()),
    config: configId(answerConfig(root, manifest, {})),
    corpus: corpus(files),
    tools: [{ name: "bandit", version: "1.9.4" }],
    names: [],
  });
  return { adapter, cleanup: () => rmSync(root, { recursive: true, force: true }) };
}

test("a verdict is clean when no parser objected, and a parser that makes no grammar checks is not a grammar error", () => {
  assert.equal(isCleanVerdict({ parser: "cpython", syntax: [], grammar: [] }), true);
  assert.equal(isCleanVerdict({ parser: "libcst", syntax: [], grammar: null }), true);
  assert.equal(isCleanVerdict({ parser: "cpython", syntax: [], grammar: ["return outside function"] }), false);
  assert.equal(isCleanVerdict({ parser: "cpython", syntax: ["invalid syntax"], grammar: [] }), false);
  assert.deepEqual(cleanVerdicts(PARSERS), [
    { parser: "cpython 3.14.8", syntax: [], grammar: [] },
    { parser: "libcst 1.9.0", syntax: [], grammar: null },
  ]);
});

test("a verdict file written [] reads as every parser silent; no file reads as no verdict", async () => {
  const { adapter, cleanup } = adapterOver([
    { path: "broken.py", source: BROKEN, verdicts: OBJECTING },
    { path: "silent.py", source: SILENT, verdicts: cleanVerdicts(PARSERS) },
    { path: "probe.js", source: SILENT, script: true, verdicts: OBJECTING },
  ]);
  try {
    const answers = await adapter.parserVerdicts([
      { path: "silent.py", source: SILENT },
      { path: "broken.py", source: BROKEN },
      // The same bytes, never recorded: no verdict, as for a golden whose text changed.
      { path: "silent.py", source: "import sys\n" },
      { path: "never-recorded.py", source: "x = 1\n" },
      // A verdict is looked up by content and file type, wherever the source is: a case source and a golden share one.
      { path: "lib/app.py", source: SILENT },
      { path: "elsewhere/broken.py", source: BROKEN },
      // Another file type, or the same bytes read as a script, is another read: its own verdict, or none.
      { path: "app.pyi", source: SILENT },
      { path: "probe.js", source: SILENT, script: true },
      { path: "probe.js", source: SILENT },
    ]);
    assert.deepEqual(answers[0], cleanVerdicts(PARSERS));
    assert.deepEqual(answers[1], OBJECTING);
    assert.deepEqual(answers[2], []);
    assert.deepEqual(answers[3], []);
    assert.deepEqual(answers[4], cleanVerdicts(PARSERS));
    assert.deepEqual(answers[5], OBJECTING);
    assert.deepEqual(answers[6], []);
    assert.deepEqual(answers[7], OBJECTING);
    assert.deepEqual(answers[8], []);
  } finally {
    cleanup();
  }
});

test("a verdict file has one form: [] for every parser silent, and a written list one a parser objected to", () => {
  assert.equal(verdictText(cleanVerdicts(PARSERS)), "[]\n");
  assert.deepEqual(parseVerdicts("[]\n", "app.py", PARSERS), cleanVerdicts(PARSERS));
  assert.throws(() => parseVerdicts(JSON.stringify(cleanVerdicts(PARSERS)), "app.py", PARSERS), /not in canonical form/u);
  assert.throws(() => parseVerdicts(canonicalText(cleanVerdicts(PARSERS)), "app.py", PARSERS), /every parser is silent, which is written \[\]/u);
});

test("the recorder's parser list is the one every source was asked about, and a source asked of fewer parsers is an error", () => {
  assert.deepEqual(recordedParsers([OBJECTING, cleanVerdicts(PARSERS)]), PARSERS);
  // An empty list is a source no parser answered about; it says nothing about which parsers ran.
  assert.deepEqual(recordedParsers([[], OBJECTING]), PARSERS);
  assert.throws(() => recordedParsers([OBJECTING, [OBJECTING[0]!]]), /independent parsers differ between sources/u);
  assert.throws(() => recordedParsers([]), /no independent parser ran/u);
});

const recordedRule = (name: string) => ({
  facts: { name, implementation: ["bandit/blacklists/calls.py"], tests: ["examples/pickle.py"], deprecation: null, recommended: null, notOnRealCode: null },
  names: { implementation: { sets: [], names: [], regexes: [] }, shared: { sets: [], names: [], regexes: [] } },
  spanParts: { endLine: true, endCol: true },
  cases: [],
});

test("a rule whose recorded answer excludes a corpus file hides no other rule's answer on it", async () => {
  const root = mkdtempSync(join(tmpdir(), "gates-excluded-"));
  try {
    mkdirSync(join(root, "packs/demo/upstream/bandit"), { recursive: true });
    writeFileSync(join(root, "packs/demo/upstream/bandit/gates.yaml"), MANIFEST);
    const manifest = readGatesManifest(root, "demo", "bandit");
    const config = configId(answerConfig(root, manifest, {}));
    const file = { task: "demo", repository: "demo/demo", path: "app.py", source: SILENT, sha256: digest(SILENT) };
    const answer = (rule: string, text: string): [string, Buffer] => [answerPath(config, rule, file, file.sha256), Buffer.from(text, "utf8")];
    const adapterWith = (answers: [string, Buffer][]) => createRecordedAdapter({
      root,
      manifest,
      lock: null,
      recording: gatesRecordingSchema.parse(recording({ B301: recordedRule("B301"), B302: recordedRule("B302"), B303: recordedRule("B303") })),
      config,
      corpus: { tree: memoryTree(new Map(answers)), commitOf: () => "0".repeat(40) },
      tools: [{ name: "bandit", version: "1.9.4" }],
      names: [],
    });
    const excluded = answerText({ excluded: "upstream could not analyse it", findings: [] });
    const mixed = adapterWith([answer("B301", excluded), answer("B302", answerText({ excluded: null, findings: [{ startLine: 1, startCol: 0, endLine: null, endCol: 3 }] }))]);
    const [split] = await mixed.lintCorpus([file], ["B301", "B302", "B303"]);
    assert.equal(split!.excluded, null);
    assert.deepEqual([...split!.excludedFor!], [["B301", "upstream could not analyse it"]]);
    assert.deepEqual(split!.findings.get("B302"), [{ startLine: 1, startCol: 0, endLine: null, endCol: 3 }]);
    assert.deepEqual([...split!.unanswered!], ["B303"]);
    // Every answered rule excluding it, for one reason, is the file excluded.
    const [whole] = await adapterWith([answer("B301", excluded), answer("B302", excluded)]).lintCorpus([file], ["B301", "B302"]);
    assert.equal(whole!.excluded, "upstream could not analyse it");
    assert.equal(whole!.excludedFor, undefined);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});
