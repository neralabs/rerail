import assert from "node:assert/strict";
import { cpSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "vitest";

import type { CorpusFile } from "../../../conformance/gates/corpus.js";
import { gatesCorpusSchema, gatesRecordingSchema, spanOf } from "../../../conformance/gates/recording.js";
import type { PackAdapter, UpstreamFile } from "../../../conformance/gates/types.js";
import { diffRecording, recordToolPack, writeRecording, type AdapterFactory } from "../../../conformance/recorders/gates/record.js";

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

test("a fake adapter's answers are recorded in the form recording.ts reads, and a second recording finds no drift", async () => {
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
      runtimeDigests: () => ({ "uv.lock": "d".repeat(64) }),
    };
    const recorded = await recordToolPack(options);
    await writeRecording(root, "demo", "bandit", recorded);

    const recording = gatesRecordingSchema.parse(JSON.parse(readFileSync(join(root, "packs/demo/upstream/bandit/gates.json"), "utf8")));
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

    const corpus = gatesCorpusSchema.parse(JSON.parse(readFileSync(join(root, "packs/demo/upstream/bandit/gates-corpus.json"), "utf8")));
    assert.deepEqual(corpus.files.map((file) => [file.task, file.path, Object.keys(file.findings)]), [["demo-task", "app.py", ["B301"]]]);
    assert.equal(readFileSync(join(root, "packs/demo/upstream/bandit/fixtures/pickle_deserialize.py"), "utf8"), SOURCE);
    assert.match(readFileSync(join(root, "packs/demo/licenses/bandit/UPSTREAM.md"), "utf8"), /\| `upstream\/bandit\/fixtures\/pickle_deserialize\.py` \| `examples\/pickle_deserialize\.py` \| verbatim \|/u);

    const again = await recordToolPack(options);
    assert.deepEqual(diffRecording(join(root, "packs/demo/upstream/bandit"), again), { changed: [], missing: [], conflicting: [] });
    cpSync(join(root, "packs/demo/upstream/bandit/gates.json"), join(root, "gates.json.bak"));
    writeFileSync(join(root, "packs/demo/upstream/bandit/gates.json"), "{}\n");
    assert.deepEqual(diffRecording(join(root, "packs/demo/upstream/bandit"), again).changed, ["gates.json"]);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});
