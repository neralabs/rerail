/**
 * The committed case sources are pinned by the expected answers as upstream's answers are: a case source with other
 * bytes than the ones upstream was run on fails `case-sources`, so a pull request cannot trim the construct behind a
 * finding out of a fixture and have the rule agree with upstream's unchanged answer.
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

/** The case-sources check on a tool-pack directory whose fixtures/pickle.py holds `committed`, or nothing. */
function caseSources(committed: string | null, caseDigest: string | undefined) {
  const root = mkdtempSync(join(tmpdir(), "gates-case-sources-"));
  try {
    mkdirSync(join(root, "packs/demo/upstream/bandit/fixtures"), { recursive: true });
    writeFileSync(join(root, "packs/demo/upstream/bandit/gates.yaml"), MANIFEST);
    if (committed !== null) writeFileSync(join(root, "packs/demo/upstream/bandit/fixtures/pickle.py"), committed);
    const checks = toolPackChecks({
      root,
      manifest: readGatesManifest(root, "demo", "bandit"),
      digest: "",
      recording: recording(caseDigest),
      corpus: null,
      tools: [{ name: "bandit", version: "1.9.4" }],
      names: ["B301"],
    });
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

  // Answers recorded without a digest per case pin nothing, so they are stale rather than trusted.
  const unpinned = caseSources(SOURCE, undefined);
  assert.equal(unpinned.ok, false);
  assert.match(unpinned.detail, /fixtures\/pickle\.py has no recorded sha256/u);
});
