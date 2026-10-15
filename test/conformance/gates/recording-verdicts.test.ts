/**
 * The v2 recordings' parser verdicts: a recording names its independent parsers once and leaves `verdicts` off every
 * entry none of them objected to. What the gates must keep telling apart is an entry that is there without `verdicts`
 * (every parser silent) from an entry that is not there at all (a source nobody recorded, on which the gates draw no
 * conclusion) — the second is what a golden whose text changed looks like.
 */
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "vitest";

import { readGatesManifest } from "../../../conformance/gates/manifest.js";
import { createRecordedAdapter } from "../../../conformance/gates/recorded-adapter.js";
import {
  cleanVerdicts, gatesCorpusSchema, gatesRecordingSchema, isCleanVerdict, recordedParsers, recordingText,
  GATES_CORPUS_FORMAT, GATES_RECORDING_FORMAT,
} from "../../../conformance/gates/recording/schema.js";
import { migrateToV2 } from "../../../conformance/recorders/gates/migrate-v2.js";

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

function recording(parserVerdicts: unknown[]): unknown {
  return {
    format: GATES_RECORDING_FORMAT,
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
    rules: {},
    absent: [],
    parsers: PARSERS,
    parserVerdicts,
  };
}

function corpus(files: unknown[]): unknown {
  return {
    format: GATES_CORPUS_FORMAT,
    tool: "bandit",
    pins: { tasks: "c".repeat(64), corpora: [] },
    rules: [],
    parsers: PARSERS,
    files,
  };
}

/** A recorded adapter over the two recordings, with a tool-pack directory the checks can look at. */
function adapterOver(parserVerdicts: unknown[], files: unknown[]) {
  const root = mkdtempSync(join(tmpdir(), "gates-verdicts-"));
  mkdirSync(join(root, "packs/demo/upstream/bandit"), { recursive: true });
  writeFileSync(join(root, "packs/demo/upstream/bandit/gates.yaml"), MANIFEST);
  writeFileSync(join(root, "packs/demo/conformance.json"), JSON.stringify({ tools: [{ name: "bandit", version: "1.9.4" }] }));
  const manifest = readGatesManifest(root, "demo", "bandit");
  const adapter = createRecordedAdapter({
    root,
    manifest,
    lock: null,
    recording: gatesRecordingSchema.parse(recording(parserVerdicts)),
    corpus: gatesCorpusSchema.parse(corpus(files)),
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

test("an entry without verdicts reads as every parser silent; no entry at all reads as no verdict", async () => {
  const { adapter, cleanup } = adapterOver(
    [
      { path: "broken.py", script: false, sha256: digest(BROKEN), verdicts: OBJECTING },
      { path: "silent.py", script: false, sha256: digest(SILENT) },
    ],
    [{ task: "demo", path: "app.py", sha256: digest(SILENT), excluded: null, findings: {} }],
  );
  try {
    const answers = await adapter.parserVerdicts([
      { path: "silent.py", source: SILENT },
      { path: "broken.py", source: BROKEN },
      // Recorded at this path, but with other bytes: the entry is not found, as for a golden whose text changed.
      { path: "silent.py", source: "import sys\n" },
      { path: "never-recorded.py", source: SILENT },
      // A corpus file's verdicts are looked up by path and sha256 alone, and are dropped when silent just the same.
      { path: "app.py", source: SILENT },
    ]);
    assert.deepEqual(answers[0], cleanVerdicts(PARSERS));
    assert.deepEqual(answers[1], OBJECTING);
    assert.deepEqual(answers[2], []);
    assert.deepEqual(answers[3], []);
    assert.deepEqual(answers[4], cleanVerdicts(PARSERS));
  } finally {
    cleanup();
  }
});

test("a verdict list that is written out must be one a parser objected to, and must name the recording's parsers", () => {
  const silentWritten = gatesRecordingSchema.safeParse(recording([{ path: "silent.py", script: false, sha256: digest(SILENT), verdicts: cleanVerdicts(PARSERS) }]));
  assert.equal(silentWritten.success, false);
  assert.match(silentWritten.error!.issues.map((issue) => issue.message).join("; "), /every parser is silent: leave verdicts off/u);

  const otherParsers = gatesRecordingSchema.safeParse(recording([{ path: "broken.py", script: false, sha256: digest(BROKEN), verdicts: [OBJECTING[1]!, OBJECTING[0]!] }]));
  assert.equal(otherParsers.success, false);
  assert.match(otherParsers.error!.issues.map((issue) => issue.message).join("; "), /names other parsers than the recording's parsers, or in another order/u);

  const corpusSilent = gatesCorpusSchema.safeParse(corpus([{ task: "demo", path: "app.py", sha256: digest(SILENT), excluded: null, findings: {}, verdicts: cleanVerdicts(PARSERS) }]));
  assert.equal(corpusSilent.success, false);
});

test("the recorder's parser list is the one every source was asked about, and a source asked of fewer parsers is an error", () => {
  assert.deepEqual(recordedParsers([OBJECTING, cleanVerdicts(PARSERS)]), PARSERS);
  // An empty list is a source no parser answered about; it says nothing about which parsers ran.
  assert.deepEqual(recordedParsers([[], OBJECTING]), PARSERS);
  assert.throws(() => recordedParsers([OBJECTING, [OBJECTING[0]!]]), /independent parsers differ between sources/u);
  assert.throws(() => recordedParsers([]), /no independent parser ran/u);
});

test("the v1 to v2 migration drops only the silent lists, keeps every entry's identity, and a second run is a no-op", () => {
  // A v1 recording has no `parsers`: the migration reads the parser list off the verdict lists themselves.
  const withoutParsers = (value: unknown): Record<string, unknown> => {
    const { parsers: _declared, ...rest } = value as Record<string, unknown>;
    return rest;
  };
  const v1Recording = {
    ...withoutParsers(recording([])),
    format: "rerail.gates-recording/v1",
    parserVerdicts: [
      { path: "broken.py", script: false, sha256: digest(BROKEN), verdicts: OBJECTING },
      { path: "silent.py", script: false, sha256: digest(SILENT), verdicts: cleanVerdicts(PARSERS) },
    ],
  };
  const v1Corpus = {
    ...withoutParsers(corpus([])),
    format: "rerail.gates-corpus/v1",
    files: [{ task: "demo", path: "app.py", sha256: digest(SILENT), excluded: null, findings: {}, verdicts: cleanVerdicts(PARSERS) }],
  };

  const first = migrateToV2(v1Recording, v1Corpus);
  assert.deepEqual(first.recording.parsers, PARSERS);
  assert.deepEqual(first.corpus.parsers, PARSERS);
  // Both entries survive, sorted as before; only the silent one lost its list.
  assert.deepEqual(first.recording.parserVerdicts.map((entry) => [entry.path, entry.verdicts === undefined]), [["broken.py", false], ["silent.py", true]]);
  assert.deepEqual(first.corpus.files.map((file) => [file.path, file.sha256, file.verdicts === undefined]), [["app.py", digest(SILENT), true]]);

  const again = migrateToV2(JSON.parse(recordingText(first.recording)), JSON.parse(recordingText(first.corpus)));
  assert.equal(recordingText(again.recording), recordingText(first.recording));
  assert.equal(recordingText(again.corpus), recordingText(first.corpus));
});
