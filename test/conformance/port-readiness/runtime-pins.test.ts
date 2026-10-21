/**
 * expected/upstream.json pins every runtime the generator installs, and the gates hold it to exactly those
 * (recordedRuntimes): a runtime gates.yaml names that upstream.json doesn't makes expected/ stale, because nothing would
 * otherwise check that runtime's pins, and a change to them would leave CI judging the rules against answers generated
 * under the old setup.
 */
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "vitest";

import { readGatesManifest } from "../../../conformance/port-readiness/manifest.js";
import { staleMessage, toolPackChecks } from "../../../conformance/port-readiness/recorded-adapter.js";
import { RUNTIMES_DIRECTORY } from "../../../conformance/port-readiness/upstream-files.js";
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

const PINS = { repository: "https://github.com/PyCQA/bandit", commit: "a".repeat(40), version: "1.9.4", runtime: "bandit-1.9.4" };
const BANDIT = '[project]\nname = "bandit-runtime"\ndependencies = ["bandit==1.9.4"]\n';
const PYTHON = '[project]\nname = "python-runtime"\ndependencies = []\n';
const digest = (text: string): string => createHash("sha256").update(text).digest("hex");

type Runtimes = Record<string, Record<string, string>>;
const BANDIT_PIN: Runtimes = { "bandit-1.9.4": { "pyproject.toml": digest(BANDIT) } };
const PYTHON_PIN: Runtimes = { python: { "pyproject.toml": digest(PYTHON) } };

function recording(runtimes: Runtimes) {
  return gatesRecordingSchema.parse({
    format: RECORDING_FORMAT,
    tool: "bandit",
    adapter: "bandit",
    pins: PINS,
    runtimes,
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
    absent: [],
    parsers: [{ parser: "cpython 3.14.8", grammar: true }],
    parserVerdicts: [],
  });
}

/**
 * The pins-agree and runtime-pins checks of a bandit tool-pack (which installs bandit-1.9.4 and python) whose
 * expected answers pin `runtimes`, over committed pins `committed`.
 */
function runtimeChecks(runtimes: Runtimes, committed: Record<string, string> = { "bandit-1.9.4": BANDIT, python: PYTHON }) {
  const root = mkdtempSync(join(tmpdir(), "gates-runtime-pins-"));
  try {
    mkdirSync(join(root, "packs/demo/upstream/bandit"), { recursive: true });
    writeFileSync(join(root, "packs/demo/upstream/bandit/gates.yaml"), MANIFEST);
    for (const [runtime, text] of Object.entries(committed)) {
      mkdirSync(join(root, RUNTIMES_DIRECTORY, runtime), { recursive: true });
      writeFileSync(join(root, RUNTIMES_DIRECTORY, runtime, "pyproject.toml"), text);
    }
    const checks = toolPackChecks({
      root,
      manifest: readGatesManifest(root, "demo", "bandit"),
      digest: "",
      recording: recording(runtimes),
      corpus: null,
      tools: [{ name: "bandit", version: "1.9.4" }],
      names: [],
    });
    return { pinsAgree: checks.find((check) => check.id === "pins-agree")!, runtimePins: checks.find((check) => check.id === "runtime-pins")! };
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
}

test("expected answers that pin every runtime the tool-pack installs, at its committed pins, pass", () => {
  const { pinsAgree, runtimePins } = runtimeChecks({ ...BANDIT_PIN, ...PYTHON_PIN });
  assert.equal(pinsAgree.ok, true, pinsAgree.detail);
  assert.equal(runtimePins.ok, true, runtimePins.detail);
  assert.equal(runtimePins.detail, "2 runtime(s) match their committed pins");
});

test("expected answers missing a runtime gates.yaml installs, or pinning one it doesn't, fail pins-agree as stale", () => {
  // The case that let eslint-config-next's package.json change unchecked: runtime-pins passes on what upstream.json names.
  const missing = runtimeChecks(BANDIT_PIN);
  assert.equal(missing.runtimePins.ok, true, missing.runtimePins.detail);
  assert.equal(missing.pinsAgree.ok, false);
  assert.match(missing.pinsAgree.detail, /gates\.yaml's adapter and environment install the runtimes bandit-1\.9\.4, python but expected\/upstream\.json pins bandit-1\.9\.4/u);
  assert.ok(missing.pinsAgree.detail.endsWith(staleMessage("demo", "bandit")), missing.pinsAgree.detail);

  // gates.yaml moved to another runtime directory while upstream.json still names the old one, whose pins are unchanged.
  const moved = runtimeChecks({ ...BANDIT_PIN, "python-old": PYTHON_PIN.python! }, { "bandit-1.9.4": BANDIT, "python-old": PYTHON });
  assert.equal(moved.runtimePins.ok, true, moved.runtimePins.detail);
  assert.equal(moved.pinsAgree.ok, false);
  assert.match(moved.pinsAgree.detail, /install the runtimes bandit-1\.9\.4, python but expected\/upstream\.json pins bandit-1\.9\.4, python-old/u);
});

test("a runtime pin edited after the answers were generated fails runtime-pins", () => {
  const edited = runtimeChecks({ ...BANDIT_PIN, ...PYTHON_PIN }, { "bandit-1.9.4": BANDIT, python: `${PYTHON}# edited\n` });
  assert.equal(edited.pinsAgree.ok, true, edited.pinsAgree.detail);
  assert.equal(edited.runtimePins.ok, false);
  assert.match(edited.runtimePins.detail, /^python\/pyproject\.toml is [0-9a-f]{12}, recorded [0-9a-f]{12}/u);
  assert.ok(edited.runtimePins.detail.endsWith(staleMessage("demo", "bandit")), edited.runtimePins.detail);
});
