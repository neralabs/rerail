/**
 * gates.yaml's `watch:` block (conformance/languages/manifest.ts) and its expansion over the ported rules
 * (conformance/languages/watch.ts), on a scratch checkout.
 */
import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, test } from "vitest";

import { parseGatesManifest, type GatesManifest } from "../../conformance/languages/manifest.js";
import { expandWatch, watchMatcher } from "../../conformance/languages/watch.js";

const roots: string[] = [];
afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

const gates = (watch: string) => parseGatesManifest(`adapter: bandit
rerail:
  pack: python
  project: bandit
upstream:
  name: PyCQA/bandit
  repository: https://github.com/PyCQA/bandit
  commit: ${"a".repeat(40)}
  version: 1.9.4
  license: Apache-2.0
  recommended_configs: [default]
  runtime: bandit-1.9.4
  lints: [".py"]
popularity:
  observed_on: 2026-10-06
  evidence: [a fake upstream]
rules: {}
${watch}`, "gates.yaml");

/** A scratch checkout whose packs/python/conformance.json ports `rules` (tool, upstream rule), with `files` beside the generator. */
function checkout(rules: [string, string][], files: Record<string, unknown> = {}): string {
  const root = mkdtempSync(join(tmpdir(), "rerail-watch-"));
  roots.push(root);
  const tool = join(root, "packs", "python", "upstream", "bandit");
  mkdirSync(tool, { recursive: true });
  writeFileSync(join(root, "packs", "python", "conformance.json"), JSON.stringify({ rules: rules.map(([name, rule]) => ({ id: rule, upstream: { tool: name, rule } })) }));
  for (const [name, content] of Object.entries(files)) writeFileSync(join(tool, name), JSON.stringify(content));
  return root;
}

const expand = (root: string, manifest: GatesManifest) => expandWatch(root, "python", "bandit", manifest);

test("{rule} expands over the tool's ports without rule-prefix, plus extra-rules; a rules: entry takes its own paths", () => {
  const root = checkout([["bandit", "x/b"], ["bandit", "x/a"], ["bandit", "x/a"], ["ruff", "x/c"]]);
  const watch = expand(root, gates(`watch:
  rule-prefix: "x/"
  extra-rules: [z]
  tests: ["tests/{rule}.py", "tests/conftest.py"]
  code: ["src/{rule}.py", "src/*/{rule}.py"]
  shared: ["src/util/**", "setup.cfg"]
  new-tests: ["tests/{rule}/**"]
  rules:
    a: { code: ["src/alpha.py"] }
`));
  assert.deepEqual(watch, {
    rules: ["a", "b", "z"],
    tests: {
      paths: ["tests/a.py", "tests/b.py", "tests/conftest.py", "tests/z.py"],
      globs: [],
      byRule: { a: ["tests/a.py"], b: ["tests/b.py"], z: ["tests/z.py"] },
    },
    code: {
      paths: ["src/alpha.py", "src/b.py", "src/z.py"],
      globs: ["src/*/b.py", "src/*/z.py"],
      byRule: { a: ["src/alpha.py"], b: ["src/*/b.py", "src/b.py"], z: ["src/*/z.py", "src/z.py"] },
      indexes: [],
    },
    shared: { paths: ["setup.cfg"], globs: ["src/util/**"] },
    newTests: { paths: [], globs: ["tests/a/**", "tests/b/**", "tests/z/**"], byRule: { a: ["tests/a/**"], b: ["tests/b/**"], z: ["tests/z/**"] } },
  });
  assert.equal(expand(root, gates("")), undefined);
});

test("a { file } entry watches each rule's paths from the data file beside the generator; { index } is kept by name", () => {
  const root = checkout([["bandit", "B101"], ["bandit", "B102"]], {
    "fixtures.json": { rules: { B102: ["examples/exec.py"], B101: ["examples/assert.py", "examples/shared.py"] } },
  });
  const watch = expand(root, gates(`watch:
  tests: [{ file: fixtures.json }, "examples/auxiliary/**"]
  code: [{ index: ruff-codes }]
  shared: []
`))!;
  assert.deepEqual(watch.tests, {
    paths: ["examples/assert.py", "examples/exec.py", "examples/shared.py"],
    globs: ["examples/auxiliary/**"],
    byRule: { B101: ["examples/assert.py", "examples/shared.py"], B102: ["examples/exec.py"] },
  });
  assert.deepEqual(watch.code.indexes, ["ruff-codes"]);
  const watched = watchMatcher(watch.tests);
  assert.deepEqual(["examples/exec.py", "examples/auxiliary/a/b.rs", "examples/other.py"].map(watched), [true, true, false]);
});

test("a rule the watch names must be watched, and a port must carry rule-prefix", () => {
  const root = checkout([["bandit", "x/a"]], { "fixtures.json": { rules: { b: ["t/b.py"] } } });
  const watch = (body: string) => gates(`watch:\n  rule-prefix: "x/"\n  code: []\n  shared: []\n${body}`);
  assert.throws(() => expand(root, watch("  tests: []\n  rules:\n    b: { tests: [t.py] }\n")), /watch\.rules names b, which is no watched rule/u);
  assert.throws(() => expand(root, watch("  tests: [{ file: fixtures.json }]\n")), /fixtures\.json lists b, which is no watched rule/u);
  assert.throws(() => expand(root, watch("  extra-rules: [a]\n  tests: []\n")), /extra-rules lists a, which conformance\.json already ports/u);
  assert.throws(() => expand(root, gates("watch:\n  rule-prefix: \"y/\"\n  tests: []\n  code: []\n  shared: []\n")), /rule-prefix is y\/, but conformance\.json ports x\/a/u);
});

test("watch patterns are upstream-relative, take {rule} only where it expands, and the block is strict", () => {
  const bad = (body: string, message: RegExp) => assert.throws(() => gates(`watch:\n  tests: []\n  code: []\n${body}`), message);
  bad("  shared: [\"lib/{rule}.js\"]\n", /\/watch\/shared\/0: takes no \{rule\}/u);
  bad("  shared: [\"/lib/a.js\"]\n", /\/watch\/shared\/0: a path relative/u);
  bad("  shared: [\"lib/../a.js\"]\n", /\/watch\/shared\/0: a path relative/u);
  bad("  shared: [\"lib/{a,b}.js\"]\n", /\/watch\/shared\/0: holds a brace other than \{rule\}/u);
  bad("  shared: []\n  rules:\n    a: { tests: [\"t/{rule}.js\"] }\n", /\/watch\/rules\/a\/tests\/0: takes no \{rule\}/u);
  bad("  shared: []\n  extra: []\n", /\/watch: /u);
  assert.throws(() => gates("watch:\n  tests: []\n  code: [{ index: eslint-rules }]\n  shared: []\n"), /\/watch\/code\/0/u);
  assert.equal(gates("watch:\n  tests: [\".github/x.yml\"]\n  code: []\n  shared: []\n").watch?.tests[0], ".github/x.yml");
});
