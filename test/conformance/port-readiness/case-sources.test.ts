/**
 * The committed case sources are pinned by the recording as upstream's answers are: a case source with other bytes
 * than the ones upstream was recorded on fails `case-sources`, so a pull request cannot trim the construct behind a
 * finding out of a fixture and have the rule agree with upstream's unchanged answer. A recording uploaded by hand
 * before cases carried a sha256 is held to one digest over all its case sources instead (LIFTED_CASE_SOURCES).
 */
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "vitest";

import { readGatesManifest } from "../../../conformance/port-readiness/manifest.js";
import { staleMessage, toolPackChecks } from "../../../conformance/port-readiness/recorded-adapter.js";
import { gatesRecordingSchema, RECORDING_FORMAT } from "../../../conformance/port-readiness/recording/schema.js";
import { assetName, caseSourcesDigest, type RecordingLock } from "../../../conformance/port-readiness/recording/store.js";

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

const SOURCE = "import pickle\npickle.loads(b'')\n";
const digest = (text: string): string => createHash("sha256").update(text).digest("hex");

function recording(caseDigest: string | undefined) {
  const testCase = { fixture: "examples/pickle.py", variant: "default", path: "pickle.py", skipped: null, expected: [[1, 0, 1, 17]], markup: null, file: "fixtures/pickle.py" };
  return gatesRecordingSchema.parse({
    format: RECORDING_FORMAT,
    tool: "bandit",
    adapter: "bandit",
    pins: { repository: "https://github.com/PyCQA/bandit", commit: "a".repeat(40), version: "1.9.4", runtime: "bandit-1.9.4" },
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
    rules: {
      B301: {
        facts: { name: "B301", implementation: ["bandit/blacklists/calls.py"], tests: ["examples/pickle.py"], deprecation: null, recommended: null, notOnRealCode: null },
        names: { implementation: { sets: [], names: [], regexes: [] }, shared: { sets: [], names: [], regexes: [] } },
        spanParts: { endLine: true, endCol: true },
        cases: [caseDigest === undefined ? testCase : { ...testCase, sha256: caseDigest }],
      },
    },
    absent: [],
    parsers: [{ parser: "cpython 3.14.8", grammar: true }],
    parserVerdicts: [],
  });
}

/** A lock naming the recording's recording.json by the asset name readRecordingLock requires; only the name is read here. */
const RECORDING_SHA = "b".repeat(64);
const LOCK = { tool: "bandit", files: { "recording.json": { sha256: RECORDING_SHA, asset: assetName("demo", "bandit", "recording.json", RECORDING_SHA) } } } as unknown as RecordingLock;

/**
 * The case-sources check on a tool-pack directory whose fixtures/pickle.py holds `committed`, or nothing; with `lifted`,
 * under a lock naming the recording, and LIFTED_CASE_SOURCES replaced by `lifted`.
 */
function caseSources(committed: string | null, caseDigest: string | undefined, lifted?: ReadonlyMap<string, string>) {
  const root = mkdtempSync(join(tmpdir(), "gates-case-sources-"));
  try {
    mkdirSync(join(root, "packs/demo/upstream/bandit/fixtures"), { recursive: true });
    writeFileSync(join(root, "packs/demo/upstream/bandit/gates.yaml"), MANIFEST);
    if (committed !== null) writeFileSync(join(root, "packs/demo/upstream/bandit/fixtures/pickle.py"), committed);
    const checks = toolPackChecks({
      root,
      manifest: readGatesManifest(root, "demo", "bandit"),
      lock: lifted === undefined ? null : LOCK,
      recording: recording(caseDigest),
      corpus: null,
      tools: [{ name: "bandit", version: "1.9.4" }],
      names: ["B301"],
    }, lifted);
    return checks.find((check) => check.id === "case-sources")!;
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
}

test("a committed case source with its recorded bytes passes case-sources", () => {
  const check = caseSources(SOURCE, digest(SOURCE));
  assert.equal(check.ok, true, check.detail);
});

test("an edited, missing or unpinned case source fails case-sources as a stale recording", () => {
  // What a pull request would commit to make the rule agree: the construct behind upstream's finding trimmed out.
  const edited = caseSources("import pickle\n", digest(SOURCE));
  assert.equal(edited.ok, false);
  assert.match(edited.detail, /fixtures\/pickle\.py is [0-9a-f]{12}, recorded [0-9a-f]{12}/u);
  assert.ok(edited.detail.endsWith(staleMessage("demo", "bandit")), edited.detail);

  const missing = caseSources(null, digest(SOURCE));
  assert.equal(missing.ok, false);
  assert.match(missing.detail, /fixtures\/pickle\.py is not committed/u);

  // A recording made before cases carried a digest pins nothing, so it is stale rather than trusted.
  const unpinned = caseSources(SOURCE, undefined);
  assert.equal(unpinned.ok, false);
  assert.match(unpinned.detail, /fixtures\/pickle\.py has no recorded sha256/u);
});

test("a hand-uploaded recording without per-case sha256 passes only while its case sources are the ones pinned when it was recorded", () => {
  const asset = LOCK.files["recording.json"].asset;
  const pinned = new Map([[asset, caseSourcesDigest([["fixtures/pickle.py", digest(SOURCE)]])]]);

  const unchanged = caseSources(SOURCE, undefined, pinned);
  assert.equal(unchanged.ok, true, unchanged.detail);
  assert.match(unchanged.detail, /1 of them pinned together/u);

  // The same trim as above, on a recording that has no per-case digest to catch it: the pinned digest does.
  const edited = caseSources("import pickle\n", undefined, pinned);
  assert.equal(edited.ok, false);
  assert.match(edited.detail, /the 1 case source\(s\) demo-bandit-recording-b{64}\.json\.gz records no sha256 for are [0-9a-f]{12} together, pinned [0-9a-f]{12}/u);
  assert.ok(edited.detail.endsWith(staleMessage("demo", "bandit")), edited.detail);

  const missing = caseSources(null, undefined, pinned);
  assert.equal(missing.ok, false);
  assert.match(missing.detail, /fixtures\/pickle\.py is not committed/u);

  // Only the recording an entry names: another hand-uploaded recording's digest does not excuse this one.
  const other = caseSources(SOURCE, undefined, new Map([[assetName("demo", "bandit", "recording.json", "c".repeat(64)), pinned.get(asset)!]]));
  assert.equal(other.ok, false);
  assert.match(other.detail, /fixtures\/pickle\.py has no recorded sha256/u);
});
