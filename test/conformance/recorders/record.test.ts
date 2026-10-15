import assert from "node:assert/strict";
import { existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "vitest";

import type { CorpusFile } from "../../../conformance/gates/corpus.js";
import { gatesCorpusSchema, gatesRecordingSchema, spanOf } from "../../../conformance/gates/recording/schema.js";
import { readCachedRecording, recordingLockSchema, recordingLockText } from "../../../conformance/gates/recording/store.js";
import type { PackAdapter, UpstreamFile } from "../../../conformance/gates/types.js";
import { CASE_LAYOUTS, committedSources, diffRecording, recordToolPack, withUpstreamRows, writeRecording, type AdapterFactory } from "../../../conformance/recorders/gates/record.js";
import { RUNTIMES_DIRECTORY, unpinnedNpmDependencies, unpinnedPythonDependencies } from "../../../conformance/recorders/gates/runtimes.js";

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
const span = { startLine: 1, startCol: 0, endLine: null, endCol: 17 };

const fakeAdapter: AdapterFactory = async () => ({
  identity: { bandit: "1.9.4" },
  packChecks: [{ id: "upstream-commit", ok: true, detail: "fake" }],
  licenseText: "LICENSE TEXT\n",
  rule: (name) => (name === "B301" ? { name, implementation: ["bandit/blacklists/calls.py"], tests: ["examples/pickle_deserialize.py"], deprecation: null, recommended: { config: "default", options: [] }, notOnRealCode: null } : undefined),
  implementationNames: () => new Set(["loads", "pickle"]),
  sharedImplementationNames: () => new Set(["qualname"]),
  implementationRegexes: () => [/pick\w+/iu],
  fixtureWords: () => new Set(["pickle", "loads"]),
  fixtureTexts: () => [SOURCE],
  keywords: new Set(["import"]),
  commentVocabulary: new Set(["nosec"]),
  inScopeSamples: ["src/example.py"],
  cases: () => [{ fixture: "examples/pickle_deserialize.py", variant: "default", path: "pickle_deserialize.py", source: SOURCE, skipped: null, expected: [span], markup: null }],
  toSpan: (_name, _source, start) => ({ startLine: 0, startCol: start, endLine: null, endCol: 3 }),
  inCorpus: (path) => path.endsWith(".py"),
  lintCorpus: async (files: CorpusFile[]): Promise<UpstreamFile[]> => files.map((file) => ({ file, excluded: null, findings: new Map([["B301", [span]]]) })),
  parserVerdicts: async (files) => files.map(() => [{ parser: "cpython 3.14.0", syntax: [], grammar: [] }]),
}) satisfies PackAdapter;

test("a fake adapter's answers are recorded in the form recording/schema.ts reads, and a second recording finds no drift", async () => {
  const root = mkdtempSync(join(tmpdir(), "gates-record-"));
  try {
    mkdirSync(join(root, "packs/demo/upstream/bandit"), { recursive: true });
    mkdirSync(join(root, "packs/demo/licenses/bandit"), { recursive: true });
    writeFileSync(join(root, "packs/demo/upstream/bandit/gates.yaml"), MANIFEST);
    writeFileSync(join(root, "packs/demo/licenses/bandit/UPSTREAM.md"), "# Bandit\n\n| File | Upstream path | Copy |\n|---|---|---|\n| `upstream/bandit/fixtures/assert.py` | `examples/assert.py` | verbatim |\n");
    writeFileSync(join(root, "packs/demo/conformance.json"), JSON.stringify({ rules: [
      { id: "bandit-b301", upstream: { tool: "bandit", rule: "B301" }, policy: "rules/bandit-b301.json" },
      { id: "bandit-b999", upstream: { tool: "bandit", rule: "B999" }, policy: "rules/bandit-b999.json" },
      { id: "ruff-f401", upstream: { tool: "ruff", rule: "F401" }, policy: "rules/ruff-f401.json" },
    ] }));
    const corpusFile: CorpusFile = { task: "demo-task", repository: "demo/demo", path: "app.py", source: SOURCE, sha256: "b".repeat(64) };
    const options = {
      root, pack: "demo", tool: "bandit", cacheRoot: join(root, "cache"), runner: { run: () => Promise.reject(new Error("no commands")) },
      adapters: { bandit: fakeAdapter },
      upstreamBytes: () => Buffer.from(SOURCE, "utf8"),
      corpusFiles: async () => [corpusFile],
      tasksSha256: "c".repeat(64),
      runtimeDigests: () => ({ "pyproject.toml": "d".repeat(64) }),
    };
    const recorded = await recordToolPack(options);
    const written = await writeRecording(root, "demo", "bandit", recorded, { cacheRoot: join(root, "cache") });
    assert.equal(written.changes.first, true);

    // Only the lock is committed; the recording itself went into the cache, by its own digest.
    const lock = recordingLockSchema.parse(JSON.parse(readFileSync(join(root, "packs/demo/upstream/bandit/recording.lock"), "utf8")));
    assert.deepEqual(lock.pins, recorded.recording.pins);
    assert.match(lock.files["gates.json"].asset, /^demo-bandit-gates-[0-9a-f]{64}\.json\.gz$/u);
    assert.deepEqual(written.assets.map((asset) => asset.name).sort(), [lock.files["gates-corpus.json"].asset, lock.files["gates.json"].asset].sort());
    assert.equal(existsSync(join(root, "packs/demo/upstream/bandit/gates.json")), false);
    const recording = gatesRecordingSchema.parse(JSON.parse(readCachedRecording(join(root, "cache"), lock.files["gates.json"].sha256)!));
    assert.deepEqual(Object.keys(recording.rules), ["B301"]);
    assert.deepEqual(recording.absent, ["B999"]);
    const rule = recording.rules.B301!;
    assert.deepEqual(rule.spanParts, { endLine: false, endCol: true });
    assert.equal(rule.cases[0]!.file, "fixtures/pickle_deserialize.py");
    assert.deepEqual(spanOf(rule.cases[0]!.expected[0]!), span);
    const sets = [...rule.names.implementation.sets, ...rule.names.shared.sets].map((id) => recording.nameSets[id]!);
    assert.deepEqual(new Set([...rule.names.implementation.names, ...sets.flatMap((set) => set.names)]), new Set(["loads", "pickle", "qualname"]));
    assert.deepEqual(sets.flatMap((set) => set.regexes).map(({ source, flags }) => new RegExp(source, flags)), [/pick\w+/iu]);
    assert.equal(recording.parserVerdicts.length, 1);
    assert.equal(recording.corpus.extensions.includes(".py"), true);

    const corpus = gatesCorpusSchema.parse(JSON.parse(readCachedRecording(join(root, "cache"), lock.files["gates-corpus.json"].sha256)!));
    assert.deepEqual(corpus.files.map((file) => [file.task, file.path, Object.keys(file.findings)]), [["demo-task", "app.py", ["B301"]]]);
    assert.equal(readFileSync(join(root, "packs/demo/upstream/bandit/fixtures/pickle_deserialize.py"), "utf8"), SOURCE);
    assert.match(readFileSync(join(root, "packs/demo/licenses/bandit/UPSTREAM.md"), "utf8"), /\| `upstream\/bandit\/fixtures\/pickle_deserialize\.py` \| `examples\/pickle_deserialize\.py` \| verbatim \|/u);

    const again = await recordToolPack(options);
    assert.deepEqual(diffRecording(root, "demo", "bandit", again), { changed: [], missing: [], conflicting: [] });
    // The lock is what a commit pins, so a lock naming other bytes is what the stale check sees.
    writeFileSync(join(root, "packs/demo/upstream/bandit/recording.lock"), recordingLockText({ ...lock, files: { ...lock.files, "gates.json": { ...lock.files["gates.json"], sha256: "e".repeat(64) } } }));
    assert.deepEqual(diffRecording(root, "demo", "bandit", again).changed, ["gates.json"]);
    // A lock whose recording is not in the cache was recorded before: the summary says it could not compare, never
    // that this is a first recording.
    const uncompared = await writeRecording(root, "demo", "bandit", again, { cacheRoot: join(root, "cache") });
    assert.equal(uncompared.changes.first, false);
    assert.equal(uncompared.changes.unavailable, `${lock.files["gates.json"].asset} is not in the cache`);
    const compared = await writeRecording(root, "demo", "bandit", again, { cacheRoot: join(root, "cache") });
    assert.equal(compared.changes.unavailable, null);
    assert.deepEqual(compared.changes.rules, []);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("an ESLint case reuses a committed source with its bytes, its own directory and file type first, or gets one of its own", () => {
  const root = mkdtempSync(join(tmpdir(), "gates-layout-"));
  try {
    for (const [file, text] of [["fixtures/eqeqeq/001.js", "a == b"], ["fixtures/eqeqeq/002.ts", "a == b"], ["fixtures/yoda/001.js", "1 == a"]] as const) {
      mkdirSync(join(root, file, ".."), { recursive: true });
      writeFileSync(join(root, file), text);
    }
    const committed = committedSources(root);
    const layout = CASE_LAYOUTS.eslint;
    const place = (code: string, path: string) => layout.file({ fixture: "tests/lib/rules/eqeqeq.js#0/valid[3]", path, bytes: Buffer.from(code) }, committed);
    assert.equal(place("a == b", "eqeqeq/input.ts"), "fixtures/eqeqeq/002.ts");
    assert.equal(place("a == b", "eqeqeq/input.jsx"), "fixtures/eqeqeq/001.js");
    assert.equal(place("1 == a", "eqeqeq/input.js"), "fixtures/yoda/001.js");
    assert.match(place("x === y", "src/App.JS")!, /^fixtures\/eqeqeq\/[0-9a-f]{12}\.js$/u);
    assert.deepEqual(layout.row!("tests/lib/rules/eqeqeq.js#0/valid[3]"), { upstream: "tests/lib/rules/eqeqeq.js", copy: "derived" });

    const table = ["# ESLint", "", "| File | Upstream path | Copy |", "|---|---|---|",
      "| `upstream/eslint/fixtures/default-case/002.js` | `tests/lib/rules/default-case.js` | derived |",
      "| `upstream/eslint/fixtures/default-case-last/001.js` | `tests/lib/rules/default-case-last.js` | derived |", ""].join("\n");
    const copy = { file: "fixtures/default-case/0f0f0f0f0f0f.js", fixture: "tests/lib/rules/default-case.js#0/invalid[1]", bytes: Buffer.from("") };
    assert.deepEqual(withUpstreamRows(table, "eslint", [copy], layout).split("\n").slice(4, 7), [
      "| `upstream/eslint/fixtures/default-case/002.js` | `tests/lib/rules/default-case.js` | derived |",
      "| `upstream/eslint/fixtures/default-case/0f0f0f0f0f0f.js` | `tests/lib/rules/default-case.js` | derived |",
      "| `upstream/eslint/fixtures/default-case-last/001.js` | `tests/lib/rules/default-case-last.js` | derived |",
    ]);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("a runtime's manifest pins every direct dependency at an exact version", () => {
  assert.deepEqual(unpinnedNpmDependencies({ dependencies: { eslint: "10.11.0", "eslint-v9": "npm:eslint@9.39.5", "@scope/parser-v2": "npm:@typescript-eslint/parser@2.34.0", beta: "1.0.0-rc.1" } }), []);
  assert.deepEqual(unpinnedNpmDependencies({ dependencies: { a: "^1.2.3", b: "~1.2.3", c: "1.x", d: "latest", e: "npm:x@^1.0.0" } }), ["a@^1.2.3", "b@~1.2.3", "c@1.x", "d@latest", "e@npm:x@^1.0.0"]);
  assert.deepEqual(unpinnedNpmDependencies({ overrides: { acorn: "8.18.0", "@typescript-eslint/parser-v2": { eslint: "$eslint" }, "@typescript-eslint/typescript-estree": { minimatch: { ".": "10.2.5", "brace-expansion": "^5.0.5" } } } }), ["@typescript-eslint/typescript-estree > minimatch > brace-expansion@^5.0.5"]);
  assert.deepEqual(unpinnedPythonDependencies('[project]\ndependencies = ["bandit==1.9.4", "libcst==1.9.0"]\n'), []);
  assert.deepEqual(unpinnedPythonDependencies('[project]\ndependencies = ["ruff>=0.16", "libcst~=1.9", "x"]\n'), ["ruff>=0.16", "libcst~=1.9", "x"]);
});

test("every committed runtime pins its direct dependencies exactly, and has no lockfile", () => {
  for (const runtime of readdirSync(RUNTIMES_DIRECTORY, { withFileTypes: true }).filter((entry) => entry.isDirectory()).map((entry) => entry.name)) {
    const directory = join(RUNTIMES_DIRECTORY, runtime);
    for (const lockfile of ["package-lock.json", "uv.lock"]) assert.ok(!existsSync(join(directory, lockfile)), `${runtime}/${lockfile}`);
    if (existsSync(join(directory, "package.json"))) assert.deepEqual(unpinnedNpmDependencies(JSON.parse(readFileSync(join(directory, "package.json"), "utf8")) as { dependencies?: Record<string, string> }), [], runtime);
    if (existsSync(join(directory, "pyproject.toml"))) assert.deepEqual(unpinnedPythonDependencies(readFileSync(join(directory, "pyproject.toml"), "utf8")), [], runtime);
  }
});
