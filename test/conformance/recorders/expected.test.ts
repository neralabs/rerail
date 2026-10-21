/**
 * The generator's answers: split by upstream test file and joined back into exactly the recording they came from, with
 * each finding once per test file and keyed by rule, so a regeneration changes only the files whose answers moved; and
 * the blob they are stored as, which reads back only as the answers for its own key.
 */
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { gunzipSync, gzipSync } from "node:zlib";
import { test } from "vitest";

import {
  decodeExpected, encodeExpected, EXPECTED_BLOB_FORMAT, expectedCachePath, expectedDigest, expectedFiles, joinExpected, parseTextsFile, readCachedExpected, readExpected, textsFileText,
  writeCachedExpected, type ExpectedSource,
} from "../../../conformance/port-readiness/recording/expected.js";
import { CORPUS_FORMAT, RECORDING_FORMAT, gatesCorpusSchema, gatesRecordingSchema, recordingText, type GatesRecording } from "../../../conformance/port-readiness/recording/schema.js";

const digest = (text: string): string => createHash("sha256").update(text).digest("hex");
const PARSERS = [{ parser: "cpython 3.14.8", grammar: true }];
const SOURCES: Record<string, string> = { "fixtures/a.py": "import os\n", "fixtures/b.py": "def (:\n" };
const SHARED = { names: ["os", "sys"], regexes: [] };
const OWN = { names: [], regexes: [{ source: "^B1\\d\\d$", flags: "u" }] };

/** A case of `fixtures/<name>.py` from upstream's examples/<name>.py, with `findings` for the rule that reads it. */
function caseOf(name: "a" | "b", findings: [number, number | null, number | null, number | null][], markup: typeof findings | null = null) {
  return {
    fixture: `examples/${name}.py`,
    variant: ".",
    path: `examples/${name}.py`,
    skipped: null,
    expected: findings,
    markup,
    file: `fixtures/${name}.py`,
    sha256: digest(SOURCES[`fixtures/${name}.py`]!),
  };
}

function rule(name: string, sets: { implementation: string[]; shared: string[] }, cases: unknown[]) {
  return {
    facts: { name, implementation: [`bandit/plugins/${name}.py`], tests: [`examples/${name}.py`], deprecation: null, recommended: { config: "default", options: [] }, notOnRealCode: null },
    names: { implementation: { sets: sets.implementation, names: [name], regexes: [] }, shared: { sets: sets.shared, names: [], regexes: [] } },
    spanParts: { endLine: false, endCol: true },
    cases,
  };
}

/** Two Bandit-style rules that both read every example, sharing one name set, one with a set of its own. */
function source(overrides: { b101?: unknown[] } = {}): ExpectedSource {
  const recording = gatesRecordingSchema.parse({
    format: RECORDING_FORMAT,
    tool: "bandit",
    adapter: "bandit",
    pins: { repository: "https://github.com/PyCQA/bandit", commit: "a".repeat(40), version: "1.9.4", runtime: "bandit-1.9.4" },
    runtimes: { "bandit-1.9.4": { "pyproject.toml": "d".repeat(64) } },
    identity: { bandit: "1.9.4" },
    packChecks: [],
    licenseSha256: digest("licence"),
    licenseText: "licence",
    inScopeSamples: ["src/example.py"],
    corpus: { extensions: [".py"], caseSensitive: true },
    keywords: ["import"],
    commentVocabulary: ["nosec"],
    fixtureWords: ["os"],
    fixtureTexts: {
      files: [{ path: "examples/a.py", sha256: digest(SOURCES["fixtures/a.py"]!) }],
      // A lone surrogate, a NUL and a CR, which a text file cannot hold as they are, and a line that looks like a header.
      inline: [{ text: "### text 3\nabc" }, { text: "\0" }, { text: "a\rb" }, { text: "x = '\ud800'" }, { text: "y = 1\n" }].sort((a, b) => (a.text < b.text ? -1 : 1)),
    },
    nameSets: { "shared-names:1": SHARED, "regexes:1": OWN },
    rules: {
      B101: rule("B101", { implementation: ["regexes:1"], shared: ["shared-names:1"] }, overrides.b101 ?? [caseOf("a", [[0, 0, null, 6]], []), caseOf("b", [])]),
      B102: rule("B102", { implementation: [], shared: ["shared-names:1"] }, [caseOf("a", []), caseOf("b", [[0, 4, null, 5]])]),
    },
    absent: ["B999"],
    parsers: PARSERS,
    parserVerdicts: [
      { path: "examples/a.py", script: false, sha256: digest(SOURCES["fixtures/a.py"]!) },
      { path: "examples/b.py", script: false, sha256: digest(SOURCES["fixtures/b.py"]!), verdicts: [{ parser: "cpython 3.14.8", syntax: ["invalid syntax"], grammar: [] }] },
      { path: "probe.py", script: false, sha256: digest("probe") },
    ],
  });
  const corpus = gatesCorpusSchema.parse({
    format: CORPUS_FORMAT,
    tool: "bandit",
    pins: { tasks: "c".repeat(64), corpora: [{ name: "demo", repository: "https://github.com/x/demo", commit: "b".repeat(40), paths: ["src"], exclude: [] }] },
    rules: ["B101", "B102"],
    parsers: PARSERS,
    files: [
      { task: "rerail:demo", path: "src/x.py", sha256: digest("x"), excluded: null, findings: { B101: [[1, 0, null, 6]] } },
      { task: "task-1", path: "a/b.py", sha256: digest("b"), excluded: "upstream's parser rejects it", findings: {} },
    ],
  });
  // The inline texts come from two test modules: each goes in its own texts/ file.
  const inline = recording.fixtureTexts!.inline;
  return {
    recording,
    corpus,
    inlineTests: inline.map(({ text }) => (text.startsWith("y") ? "tests/functional/test_runtime.py" : "tests/unit/test_texts.py")),
    testFileDigest: (test) => (test === "examples/b.py" ? null : digest(test)),
    sourceText: (file) => SOURCES[file]!,
  };
}

test("expected/ splits a recording by test file and joins back into exactly that recording", () => {
  const input = source();
  const files = expectedFiles(input);
  assert.deepEqual([...files.keys()].sort(), [
    "cases/examples/a.py.json",
    "cases/examples/b.py.json",
    "corpus/rerail/demo.json",
    "corpus/task-1.json",
    "rules/B101.json",
    "rules/B102.json",
    "texts/tests/functional/test_runtime.py.txt",
    "texts/tests/unit/test_texts.py.txt",
    "upstream.json",
  ]);
  const joined = joinExpected(files, "expected");
  const { licenseText: _licenseText, ...recording } = input.recording;
  assert.equal(recordingText(joined.recording), recordingText(recording));
  assert.equal(recordingText(joined.corpus), recordingText(input.corpus));

  // Each finding once per test file, keyed by rule; markup only where a rule has some; a set two rules share stays in
  // upstream.json and a set one rule names moves into its file.
  const a = JSON.parse(files.get("cases/examples/a.py.json")!);
  assert.equal(a.cases.length, 1);
  assert.deepEqual(a.cases[0].expected, { B101: [[0, 0, null, 6]], B102: [] });
  assert.deepEqual(a.cases[0].markup, { B101: [] });
  assert.equal(a.sha256, digest("examples/a.py"));
  assert.deepEqual(a.parserVerdicts.map((entry: { path: string }) => entry.path), ["examples/a.py"]);
  assert.equal(JSON.parse(files.get("cases/examples/b.py.json")!).sha256, null);
  const upstream = JSON.parse(files.get("upstream.json")!);
  assert.deepEqual(Object.keys(upstream.nameSets), ["shared-names:1"]);
  assert.deepEqual(upstream.parserVerdicts.map((entry: { path: string }) => entry.path), ["probe.py"]);
  assert.equal("licenseText" in upstream, false);
  assert.deepEqual(Object.keys(JSON.parse(files.get("rules/B101.json")!).nameSets), ["regexes:1"]);
});

test("a texts/ file holds any text, one after another, and reads back the same", () => {
  const texts = ["", "### text 3\nabc", "\0", "a\rb", "x = '\ud800'", "line\n### json\n", "😀"];
  const text = textsFileText(texts);
  assert.equal(text.includes("\0"), false);
  assert.equal(text.includes("\r"), false);
  assert.equal(text.isWellFormed(), true);
  assert.deepEqual(parseTextsFile(text, "t"), texts);
  assert.throws(() => parseTextsFile("### text 9\nabc\n", "t"), /not 9 code units long/u);
  assert.throws(() => parseTextsFile("abc\n", "t"), /no "### text <n>" or "### json" header/u);
});

test("a rule whose answers move changes its own file and the test files whose answers moved, nothing else", () => {
  const first = expectedFiles(source());
  assert.equal(first.size, 9);
  const differing = (a: ReadonlyMap<string, string>, b: ReadonlyMap<string, string>) => [...new Set([...a.keys(), ...b.keys()])].filter((path) => a.get(path) !== b.get(path)).sort();

  // Unchanged: the same files, to the byte.
  assert.deepEqual(differing(first, expectedFiles(source())), []);

  // B101 gains a finding on b.py: b.py's cases move, and nothing else does (B101's own file lists test files only).
  const moved = source({ b101: [caseOf("a", [[0, 0, null, 6]], []), caseOf("b", [[0, 0, null, 1]])] });
  assert.deepEqual(differing(first, expectedFiles(moved)), ["cases/examples/b.py.json"]);

  // A rule and a test file no rule reads any more go; the set B101 now names alone moves into its file.
  const dropped = source({ b101: [caseOf("a", [[0, 0, null, 6]], [])] });
  const third = expectedFiles({
    ...dropped,
    recording: { ...dropped.recording, rules: { B101: dropped.recording.rules.B101! } as GatesRecording["rules"] },
    corpus: { ...dropped.corpus, rules: ["B101"] },
  });
  assert.deepEqual([...first.keys()].filter((path) => !third.has(path)).sort(), ["cases/examples/b.py.json", "rules/B102.json"]);
  assert.deepEqual(differing(first, third).filter((path) => third.has(path)), ["cases/examples/a.py.json", "rules/B101.json", "upstream.json"]);
});

test("joining refuses a directory it cannot read as the generator wrote it", () => {
  const files = expectedFiles(source());
  assert.throws(() => joinExpected(new Map([...files, ["notes.md", "hand-written"]]), "expected"), /expected\/notes\.md is no file of the expected\/ layout/u);
  const withoutB = new Map(files);
  withoutB.delete("cases/examples/b.py.json");
  assert.throws(() => joinExpected(withoutB, "expected"), /rules\/B101\.json reads the cases of examples\/b\.py, and cases\/examples\/b\.py\.json is missing/u);
  const withoutUpstream = new Map(files);
  withoutUpstream.delete("upstream.json");
  assert.throws(() => joinExpected(withoutUpstream, "expected"), /has no upstream\.json/u);
});

const KEY = `demo/bandit/1.9.4/${"e".repeat(64)}`;

test("a blob holds the answers as the same bytes for the same answers, and reads back only as its own key's", () => {
  const files = expectedFiles(source());
  const encoded = encodeExpected("demo", "bandit", KEY, files);
  // The files in path order, whatever order the map holds them in: identical answers are identical bytes.
  assert.deepEqual(encodeExpected("demo", "bandit", KEY, new Map([...files].reverse())), encoded);
  const text = gunzipSync(encoded.bytes).toString("utf8");
  assert.equal(encoded.sha256, createHash("sha256").update(text).digest("hex"));
  assert.ok(text.startsWith(`{"format":"${EXPECTED_BLOB_FORMAT}","pack":"demo","tool":"bandit","key":"${KEY}","files":{"cases/examples/a.py.json":`), text.slice(0, 200));
  assert.deepEqual(decodeExpected(encoded.bytes, KEY, encoded.sha256), files);
  assert.deepEqual(decodeExpected(encoded.bytes, KEY), files);

  // Another key's answers, or answers whose JSON is not the one the store's metadata names, are never read as these.
  const other = `demo/bandit/1.9.4/${"f".repeat(64)}`;
  assert.throws(() => decodeExpected(encoded.bytes, other), /the answers stored for demo\/bandit\/1\.9\.4\/f+ are those for "demo\/bandit\/1\.9\.4\/e+"/u);
  assert.throws(() => decodeExpected(encoded.bytes, KEY, "0".repeat(64)), /have sha256 [0-9a-f]{12}, the store says 000000000000/u);
  const blob = JSON.parse(text) as Record<string, unknown>;
  assert.throws(() => decodeExpected(gzipSync(JSON.stringify({ ...blob, format: "rerail.port-readiness-expected/v0" })), KEY), /are in the layout "rerail\.port-readiness-expected\/v0"/u);
  assert.throws(() => decodeExpected(gzipSync(JSON.stringify({ ...blob, tool: "ruff" })), KEY), /name the tool-pack "demo"\/"ruff"/u);
  assert.throws(() => decodeExpected(gzipSync(JSON.stringify({ ...blob, files: { "../upstream.json": "{}" } })), KEY), /cannot name a file under expected\//u);
  assert.throws(() => decodeExpected(encoded.bytes.subarray(0, encoded.bytes.length - 8), KEY), /are not gzip/u);
});

test("the gates read the answers the local cache holds under the tree's key, and a miss names the command that makes them", async () => {
  const cacheRoot = mkdtempSync(join(tmpdir(), "expected-cache-"));
  try {
    assert.equal(expectedCachePath(cacheRoot, KEY), join(cacheRoot, "expected", "demo", "bandit", "1.9.4", "e".repeat(64), "expected.json.gz"));
    assert.equal(readCachedExpected(cacheRoot, KEY), undefined);
    assert.throws(() => readExpected(cacheRoot, "demo", "bandit", KEY), new RegExp(`^Error: answers for ${KEY} are not available: run npm run generate -- --pack demo --tool bandit$`, "u"));

    const input = source();
    const files = expectedFiles(input);
    await writeCachedExpected(cacheRoot, KEY, encodeExpected("demo", "bandit", KEY, files).bytes);
    const read = readExpected(cacheRoot, "demo", "bandit", KEY);
    assert.equal(recordingText(read.corpus), recordingText(input.corpus));
    assert.equal(read.digest, expectedDigest(files));

    // Answers cached under a key that were written for another are refused, not judged against.
    const other = `demo/bandit/1.9.4/${"f".repeat(64)}`;
    await writeCachedExpected(cacheRoot, other, encodeExpected("demo", "bandit", KEY, files).bytes);
    assert.throws(() => readExpected(cacheRoot, "demo", "bandit", other), /are those for/u);
    writeFileSync(expectedCachePath(cacheRoot, KEY), "not gzip");
    assert.throws(() => readExpected(cacheRoot, "demo", "bandit", KEY), /are not gzip/u);
  } finally {
    rmSync(cacheRoot, { recursive: true, force: true });
  }
});
