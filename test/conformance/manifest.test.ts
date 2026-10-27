import assert from "node:assert/strict";
import { test } from "vitest";

import { lintedPaths, parseGatesManifest } from "../../conformance/languages/manifest.js";

const gates = (rerail: string, lints: string, rules = "rules:\n  B101:\n    deprecated: null\n    recommended: { config: default, options: [] }\n") => `adapter: bandit
rerail:
  pack: python
  project: bandit
${rerail}upstream:
  name: PyCQA/bandit
  repository: https://github.com/PyCQA/bandit
  commit: ${"a".repeat(40)}
  version: 1.9.4
  license: Apache-2.0
  recommended_configs: [default]
  runtime: bandit-1.9.4
  lints: ${lints}
popularity:
  observed_on: 2026-10-06
  evidence: [a fake upstream]
${rules}known_gaps: []
`;

test("gates.yaml declares the file types upstream lints and each rule's upstream facts", () => {
  const manifest = parseGatesManifest(gates("", '[".py", ".pyi"]'), "gates.yaml");
  assert.deepEqual(manifest.upstream.lints, [".py", ".pyi"]);
  assert.deepEqual(manifest.rules, { B101: { deprecated: null, recommended: { config: "default", options: [] } } });
  assert.deepEqual(lintedPaths(manifest), ["src/example.py", "src/example.pyi"]);
  // A dialect's longer suffix is a file type too.
  assert.deepEqual(lintedPaths(parseGatesManifest(gates("", '[".blade.php"]'), "gates.yaml")), ["src/example.blade.php"]);
});

test("rerail.in_scope narrows the declared file types, and names only types upstream lints", () => {
  assert.deepEqual(lintedPaths(parseGatesManifest(gates("  in_scope: [.py]\n", '[".py", ".pyi"]'), "gates.yaml")), ["src/example.py"]);
  assert.throws(() => parseGatesManifest(gates("  in_scope: [.pyi]\n", '[".py"]'), "gates.yaml"), {
    message: "gates.yaml: /rerail/in_scope: names .pyi, which upstream does not lint (upstream.lints: .py)",
  });
});

test("upstream.lints and rules are required, and a rule's facts take only deprecated and recommended", () => {
  assert.throws(() => parseGatesManifest(gates("", "[]"), "gates.yaml"), /\/upstream\/lints: /u);
  assert.throws(() => parseGatesManifest(gates("", '[".py"]', ""), "gates.yaml"), /\/rules: /u);
  assert.throws(() => parseGatesManifest(gates("", '[".py"]', "rules:\n  B101:\n    deprecated: null\n    recommended: null\n    tests: []\n"), "gates.yaml"), /\/rules\/B101: /u);
});
