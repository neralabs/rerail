/**
 * The generator key: what ties a tree to its generated answers. Every input the generator reads moves it, so answers for
 * another tree are never judged against this one; nothing else does, so a change the generator never reads regenerates
 * nothing.
 */
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { afterAll, test } from "vitest";

import { generatorKey } from "../../../conformance/port-readiness/recording/generator-key.js";
import { PACKAGE_ROOT } from "../../../src/engine/index.js";

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

const temporary: string[] = [];
afterAll(() => {
  for (const directory of temporary) rmSync(directory, { recursive: true, force: true });
});

/** A repository with a generator of two modules, a driver and three runtimes, and one tool-pack, every file tracked. */
function repository(): { root: string; write(path: string, text: string): void } {
  const root = mkdtempSync(join(tmpdir(), "generator-key-"));
  temporary.push(root);
  const write = (path: string, text: string) => {
    mkdirSync(dirname(join(root, path)), { recursive: true });
    writeFileSync(join(root, path), text);
  };
  const files: Record<string, string> = {
    "conformance/recorders/port-readiness/expected-cli.ts": `import { z } from "zod";\nimport { helper } from "./helper.js";\nconsole.log(z, helper);\n`,
    "conformance/recorders/port-readiness/helper.ts": "export const helper = 1;\n",
    "conformance/recorders/port-readiness/adapters/driver.py": "print('driver')\n",
    "conformance/recorders/port-readiness/runtimes/bandit-1.9.4/pyproject.toml": "bandit==1.9.4\n",
    "conformance/recorders/port-readiness/runtimes/python/pyproject.toml": "python\n",
    "conformance/recorders/port-readiness/runtimes/ruff-0.16.9/pyproject.toml": "ruff==0.16.9\n",
    "conformance/port-readiness/corpus-tasks.json": "[]\n",
    "package.json": `{"type":"module"}\n`,
    "package-lock.json": JSON.stringify({ lockfileVersion: 3, packages: { "": {}, "node_modules/zod": { version: "4.0.0", integrity: "sha512-zod" } } }),
    "packs/demo/upstream/bandit/gates.yaml": MANIFEST,
    "packs/demo/upstream/bandit/goldens.tsv": "# goldens\n",
    "packs/demo/upstream/bandit/fixtures/pickle.py": "import pickle\n",
    "packs/demo/conformance.json": `{"rules":[]}\n`,
    "packs/demo/licenses/bandit/UPSTREAM.md": "# Bandit\n",
    "packs/demo/rules/bandit-b301.json": "{}\n",
    "README.md": "# Demo\n",
  };
  for (const [path, text] of Object.entries(files)) write(path, text);
  execFileSync("git", ["init", "-q"], { cwd: root });
  execFileSync("git", ["add", "-A"], { cwd: root });
  return { root, write };
}

test("the key is the upstream version and a digest of every input the generator reads, and lists them", async () => {
  const { root } = repository();
  const key = await generatorKey(root, "demo", "bandit");
  assert.equal(key.version, "1.9.4");
  assert.match(key.key, /^demo\/bandit\/1\.9\.4\/[0-9a-f]{64}$/u);
  assert.equal(key.key, `demo/bandit/1.9.4/${key.digest}`);
  assert.deepEqual(key.inputs, [
    "conformance/port-readiness/corpus-tasks.json",
    "conformance/recorders/port-readiness/adapters/driver.py",
    "conformance/recorders/port-readiness/expected-cli.ts",
    "conformance/recorders/port-readiness/helper.ts",
    "conformance/recorders/port-readiness/runtimes/bandit-1.9.4/pyproject.toml",
    "conformance/recorders/port-readiness/runtimes/python/pyproject.toml",
    "npm:node_modules/zod",
    "package.json#type",
    "packs/demo/conformance.json",
    "packs/demo/licenses/bandit/UPSTREAM.md",
    "packs/demo/upstream/bandit/fixtures/pickle.py",
    "packs/demo/upstream/bandit/gates.yaml",
    "packs/demo/upstream/bandit/goldens.tsv",
  ]);
  assert.deepEqual(await generatorKey(root, "demo", "bandit"), key);
});

test("the key moves with gates.yaml, a runtime pin and the generator's code and data, and with nothing else", async () => {
  const { root, write } = repository();
  const { key } = await generatorKey(root, "demo", "bandit");
  const keyAfter = async (path: string, text: string) => {
    write(path, text);
    return (await generatorKey(root, "demo", "bandit")).key;
  };

  // Nothing the generator reads: another runtime, a rule document, the README, and an untracked file among its inputs.
  assert.equal(await keyAfter("conformance/recorders/port-readiness/runtimes/ruff-0.16.9/pyproject.toml", "ruff==0.17.0\n"), key);
  assert.equal(await keyAfter("packs/demo/rules/bandit-b301.json", `{"id":"edited"}\n`), key);
  assert.equal(await keyAfter("README.md", "# Edited\n"), key);
  assert.equal(await keyAfter("packs/demo/upstream/bandit/fixtures/scratch.py", "scratch\n"), key);

  const moved = new Set([key]);
  for (const [path, text] of [
    ["packs/demo/upstream/bandit/gates.yaml", `${MANIFEST}# edited\n`],
    ["conformance/recorders/port-readiness/runtimes/bandit-1.9.4/pyproject.toml", "bandit==1.9.4\nextra==1.0.0\n"],
    ["conformance/recorders/port-readiness/helper.ts", "export const helper = 2;\n"],
    ["conformance/recorders/port-readiness/adapters/driver.py", "print('edited')\n"],
    ["packs/demo/upstream/bandit/fixtures/pickle.py", "import pickle  # edited\n"],
    ["package-lock.json", JSON.stringify({ lockfileVersion: 3, packages: { "": {}, "node_modules/zod": { version: "4.0.1", integrity: "sha512-zod-2" } } })],
  ] as const) {
    const next = await keyAfter(path, text);
    assert.ok(!moved.has(next), `editing ${path} left the key where it was`);
    moved.add(next);
  }
  // A new upstream release is a new version segment.
  assert.match(await keyAfter("packs/demo/upstream/bandit/gates.yaml", MANIFEST.replace("version: 1.9.4", "version: 1.9.5")), /^demo\/bandit\/1\.9\.5\//u);
});

test("a version that cannot be a key segment is refused", async () => {
  const { root, write } = repository();
  write("packs/demo/upstream/bandit/gates.yaml", MANIFEST.replace("version: 1.9.4", "version: 1.9/4"));
  await assert.rejects(generatorKey(root, "demo", "bandit"), /the upstream\.version "1\.9\/4" cannot name a key segment/u);
});

test("the repository's own generator key reads its whole import graph and the adapter's repository files", async () => {
  const key = await generatorKey(PACKAGE_ROOT, "python", "ruff");
  assert.equal(key.version, "0.16.9");
  for (const input of [
    "conformance/recorders/port-readiness/expected-cli.ts",
    "conformance/recorders/port-readiness/adapters/ruff.ts",
    "conformance/port-readiness/recording/expected.ts",
    "native/src/code_rule_dsl/scope/options.rs",
    "packs/python/upstream/ruff/gates.yaml",
    "npm:node_modules/zod",
  ]) assert.ok(key.inputs.includes(input), `the key misses ${input}`);
  assert.ok(key.inputs.some((input) => input.startsWith("conformance/recorders/port-readiness/runtimes/ruff-0.16.9/")));
  assert.ok(!key.inputs.some((input) => input.startsWith("conformance/recorders/port-readiness/runtimes/php/")), "the Ruff key hashes the PHP runtime");
}, 60_000);

test("the repository's generator reads nothing of the engine's TypeScript, so an engine edit regenerates no answers", async () => {
  const key = await generatorKey(PACKAGE_ROOT, "python", "bandit");
  assert.ok(key.inputs.includes("conformance/recorders/port-readiness/adapters/bandit.ts"), "the key misses the Bandit adapter");
  assert.deepEqual(key.inputs.filter((input) => input.startsWith("src/")), []);
}, 60_000);
