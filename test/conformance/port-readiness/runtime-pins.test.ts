/**
 * A lock pins every runtime the recorder installs, and the gates hold it to exactly those (recordedRuntimes): a runtime
 * gates.yaml names that the lock doesn't is a stale recording, because nothing would otherwise check that runtime's
 * pins, and a change to them would leave CI judging the rules against answers recorded under the old setup. A
 * recording uploaded by hand before the recorder pinned them all names the missing ones in LIFTED_RUNTIMES instead,
 * and runtime-pins checks their pins as if the lock named them.
 */
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "vitest";

import { readGatesManifest } from "../../../conformance/port-readiness/manifest.js";
import { RUNTIMES_DIRECTORY, staleMessage, toolPackChecks } from "../../../conformance/port-readiness/recorded-adapter.js";
import { gatesRecordingSchema, RECORDING_FORMAT } from "../../../conformance/port-readiness/recording/schema.js";
import { assetName, type RecordingLock } from "../../../conformance/port-readiness/recording/store.js";

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

type Runtimes = RecordingLock["runtimes"];
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

const RECORDING_SHA = "b".repeat(64);
const ASSET = assetName("demo", "bandit", "recording.json", RECORDING_SHA);

/** A lock naming the recording by the asset name readRecordingLock requires; the gates read its pins, runtimes and asset. */
function lockOf(runtimes: Runtimes): RecordingLock {
  return { tool: "bandit", pins: PINS, runtimes, files: { "recording.json": { sha256: RECORDING_SHA, asset: ASSET } } } as unknown as RecordingLock;
}

/**
 * The pins-agree and runtime-pins checks of a bandit tool-pack (which installs bandit-1.9.4 and python) whose recording
 * and lock pin `runtimes`, over committed pins `committed`, with LIFTED_RUNTIMES replaced by `lifted`.
 */
function runtimeChecks(runtimes: Runtimes, lifted: ReadonlyMap<string, Runtimes> = new Map(), committed: Record<string, string> = { "bandit-1.9.4": BANDIT, python: PYTHON }) {
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
      lock: lockOf(runtimes),
      recording: recording(runtimes),
      corpus: null,
      tools: [{ name: "bandit", version: "1.9.4" }],
      names: [],
    }, undefined, lifted);
    return { pinsAgree: checks.find((check) => check.id === "pins-agree")!, runtimePins: checks.find((check) => check.id === "runtime-pins")! };
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
}

test("a lock that pins every runtime the tool-pack installs, at its committed pins, passes", () => {
  const { pinsAgree, runtimePins } = runtimeChecks({ ...BANDIT_PIN, ...PYTHON_PIN });
  assert.equal(pinsAgree.ok, true, pinsAgree.detail);
  assert.equal(runtimePins.ok, true, runtimePins.detail);
  assert.equal(runtimePins.detail, "2 runtime(s) match their committed pins");
});

test("a lock missing a runtime gates.yaml installs, or pinning one it doesn't, fails pins-agree as a stale recording", () => {
  // The case that let eslint-config-next's package.json change unchecked: runtime-pins passes on what the lock names.
  const missing = runtimeChecks(BANDIT_PIN);
  assert.equal(missing.runtimePins.ok, true, missing.runtimePins.detail);
  assert.equal(missing.pinsAgree.ok, false);
  assert.match(missing.pinsAgree.detail, /gates\.yaml's adapter and environment install the runtimes bandit-1\.9\.4, python but recording\.lock pins bandit-1\.9\.4/u);
  assert.ok(missing.pinsAgree.detail.endsWith(staleMessage("demo", "bandit")), missing.pinsAgree.detail);

  // gates.yaml moved to another runtime directory while the lock still names the old one, whose pins are unchanged.
  const moved = runtimeChecks({ ...BANDIT_PIN, "python-old": PYTHON_PIN.python! }, new Map(), { "bandit-1.9.4": BANDIT, "python-old": PYTHON });
  assert.equal(moved.runtimePins.ok, true, moved.runtimePins.detail);
  assert.equal(moved.pinsAgree.ok, false);
  assert.match(moved.pinsAgree.detail, /install the runtimes bandit-1\.9\.4, python but recording\.lock pins bandit-1\.9\.4, python-old/u);
});

test("a hand-uploaded recording's LIFTED_RUNTIMES entry counts as part of its lock, and its pins are checked", () => {
  const lifted = new Map([[ASSET, PYTHON_PIN]]);

  const unchanged = runtimeChecks(BANDIT_PIN, lifted);
  assert.equal(unchanged.pinsAgree.ok, true, unchanged.pinsAgree.detail);
  assert.equal(unchanged.runtimePins.ok, true, unchanged.runtimePins.detail);
  assert.match(unchanged.runtimePins.detail, /^2 runtime\(s\) match their committed pins \(python pinned in recording\/store\.ts's LIFTED_RUNTIMES/u);

  // The edit the exemption must not let through: the lifted runtime's pin changes after the recording was made.
  const edited = runtimeChecks(BANDIT_PIN, lifted, { "bandit-1.9.4": BANDIT, python: `${PYTHON}# edited\n` });
  assert.equal(edited.pinsAgree.ok, true, edited.pinsAgree.detail);
  assert.equal(edited.runtimePins.ok, false);
  assert.match(edited.runtimePins.detail, /^python\/pyproject\.toml is [0-9a-f]{12}, recorded [0-9a-f]{12}/u);
  assert.ok(edited.runtimePins.detail.endsWith(staleMessage("demo", "bandit")), edited.runtimePins.detail);

  // Only the recording an entry names: another hand-uploaded recording's entry does not excuse this one.
  const other = runtimeChecks(BANDIT_PIN, new Map([[assetName("demo", "bandit", "recording.json", "c".repeat(64)), PYTHON_PIN]]));
  assert.equal(other.pinsAgree.ok, false);
  assert.match(other.pinsAgree.detail, /install the runtimes bandit-1\.9\.4, python but recording\.lock pins bandit-1\.9\.4/u);
});
