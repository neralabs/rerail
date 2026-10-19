/**
 * `loads`'s in-scope check holds a port to every file type upstream lints, unless gates.yaml's `rerail.in_scope`
 * narrows it: the typescript pack covers TypeScript files only, though typescript-eslint also lints JavaScript. A
 * narrowing may only leave types out, which the `in-scope-narrowing` tool-pack check enforces.
 */
import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "vitest";

import { readGatesManifest } from "../../../conformance/port-readiness/manifest.js";
import { inScopeSamples, toolPackChecks } from "../../../conformance/port-readiness/recorded-adapter.js";
import { gatesRecordingSchema, RECORDING_FORMAT } from "../../../conformance/port-readiness/recording/schema.js";

const SAMPLES = ["src/example.js", "src/example.mjs", "src/example.cjs", "src/example.ts", "src/example.mts", "src/example.cts", "src/example.tsx"];

/** A typescript-eslint-like gates.yaml, with `in_scope` under rerail when given. */
const manifest = (inScope: string[] | undefined) => `adapter: bandit
rerail:
  pack: demo
  project: demo
${inScope === undefined ? "" : `  in_scope:\n${inScope.map((type) => `    - ${type}\n`).join("")}`}upstream:
  name: demo
  repository: https://github.com/demo/demo
  commit: ${"a".repeat(40)}
  version: 1.0.0
  license: MIT
  recommended_configs:
    - default
  runtime: demo-1.0.0
popularity:
  observed_on: 2026-10-06
  evidence:
    - a fake upstream
known_gaps: []
`;

const recording = gatesRecordingSchema.parse({
  format: RECORDING_FORMAT,
  tool: "bandit",
  adapter: "bandit",
  pins: { repository: "https://github.com/demo/demo", commit: "a".repeat(40), version: "1.0.0", runtime: "demo-1.0.0" },
  runtimes: {},
  identity: {},
  packChecks: [],
  licenseSha256: "0".repeat(64),
  inScopeSamples: SAMPLES,
  corpus: { extensions: [".ts"], caseSensitive: true },
  keywords: [],
  commentVocabulary: [],
  fixtureWords: [],
  nameSets: {},
  rules: {},
  absent: [],
  parsers: [{ parser: "typescript 6.0.3", grammar: true }],
  parserVerdicts: [],
});

/** The `in-scope-narrowing` check for a gates.yaml with this `rerail.in_scope`, or undefined when there is none. */
function narrowing(inScope: string[] | undefined) {
  const root = mkdtempSync(join(tmpdir(), "gates-in-scope-"));
  try {
    mkdirSync(join(root, "packs/demo/upstream/bandit"), { recursive: true });
    writeFileSync(join(root, "packs/demo/upstream/bandit/gates.yaml"), manifest(inScope));
    const parsed = readGatesManifest(root, "demo", "bandit");
    const checks = toolPackChecks({ root, manifest: parsed, lock: null, recording, corpus: null, tools: [{ name: "bandit", version: "1.0.0" }], names: [] });
    return { inScope: parsed.rerail.in_scope, check: checks.find((check) => check.id === "in-scope-narrowing") };
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
}

test("without rerail.in_scope a port is held to every type upstream lints", () => {
  assert.deepEqual(inScopeSamples(SAMPLES, undefined), SAMPLES);
  assert.equal(narrowing(undefined).check, undefined);
});

test("rerail.in_scope narrows the samples to the types it names, and passes when upstream lints them all", () => {
  const { inScope, check } = narrowing([".ts", ".mts", ".cts", ".tsx"]);
  assert.deepEqual(inScopeSamples(SAMPLES, inScope), ["src/example.ts", "src/example.mts", "src/example.cts", "src/example.tsx"]);
  assert.equal(check?.ok, true, check!.detail);
});

test("rerail.in_scope naming a type upstream does not lint fails in-scope-narrowing", () => {
  const { check } = narrowing([".ts", ".vue"]);
  assert.equal(check?.ok, false);
  assert.match(check!.detail, /names \.vue, which upstream does not lint/u);
});
