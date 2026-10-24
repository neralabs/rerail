/**
 * A fake Bandit tool-pack the recorder tests record: its gates.yaml, its live adapter, the directory it lives in and
 * the options a record of it takes. Another adapter is the same tool-pack with `adapter:` replaced in MANIFEST.
 */
import { mkdirSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import type { CorpusFile } from "../../../conformance/port-readiness/corpus.js";
import { recordingLockSchema } from "../../../conformance/port-readiness/recording/store.js";
import type { PackAdapter, UpstreamFile } from "../../../conformance/port-readiness/types.js";
import type { AdapterFactory, RecordOptions } from "../../../conformance/recorders/port-readiness/record.js";

export const MANIFEST = `adapter: bandit
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

export const SOURCE = "import pickle\npickle.loads(b'')\n";
export const span = { startLine: 1, startCol: 0, endLine: null, endCol: 17 };

// Options in the order Bandit's shell_injection gen_config builds them, not sorted: a re-record that reads the locked
// recording back from its key-sorted files must still find nothing moved.
const OPTIONS = [{ subprocess: ["subprocess.Popen"], shell: ["os.system"], no_shell: ["os.execl"] }];

export const fakeAdapter: AdapterFactory = async () => ({
  identity: { bandit: "1.9.4" },
  packChecks: [{ id: "upstream-commit", ok: true, detail: "fake" }],
  licenseText: "LICENSE TEXT\n",
  rule: (name) => (name === "B301" ? { name, implementation: ["bandit/blacklists/calls.py"], tests: ["examples/pickle_deserialize.py"], deprecation: null, recommended: { config: "default", options: OPTIONS }, notOnRealCode: null } : undefined),
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

/** A factory that records the rules each live adapter is made for: making one is running upstream. */
export function counted(factory: AdapterFactory): { factory: AdapterFactory; made: string[][] } {
  const made: string[][] = [];
  return { made, factory: async (pack, names, cacheRoot, runner) => {
    made.push([...names]);
    return factory(pack, names, cacheRoot, runner);
  } };
}

/** A tool-pack directory for the fake Bandit, naming `rules`. */
export function demoToolPack(rules: string[] = ["B301"]): string {
  const root = mkdtempSync(join(tmpdir(), "port-readiness-record-"));
  mkdirSync(join(root, "packs/demo/upstream/bandit"), { recursive: true });
  mkdirSync(join(root, "packs/demo/licenses/bandit"), { recursive: true });
  writeFileSync(join(root, "packs/demo/upstream/bandit/gates.yaml"), MANIFEST);
  writeFileSync(join(root, "packs/demo/licenses/bandit/UPSTREAM.md"), "# Bandit\n\n| File | Upstream path | Copy |\n|---|---|---|\n| `upstream/bandit/fixtures/assert.py` | `examples/assert.py` | verbatim |\n");
  nameRules(root, rules);
  return root;
}

export function nameRules(root: string, rules: string[]): void {
  writeFileSync(join(root, "packs/demo/conformance.json"), JSON.stringify({ rules: rules.map((rule) => ({ id: `bandit-${rule}`, upstream: { tool: "bandit", rule }, policy: `rules/${rule}.json` })) }));
}

export const corpusFile = (path: string, sha: string, task = "demo-task"): CorpusFile => ({ task, repository: "demo/demo", path, source: SOURCE, sha256: sha.repeat(64) });

export function optionsFor(root: string, factory: AdapterFactory, goldens: () => CorpusFile[], adapter = "bandit"): RecordOptions {
  return {
    root, pack: "demo", tool: "bandit", cacheRoot: join(root, "cache"), runner: { run: () => Promise.reject(new Error("no commands")) },
    adapters: { [adapter]: factory },
    // Upstream's bytes of a test file: twoRules' cases name the rule in their source.
    upstreamBytes: (fixture: string) => Buffer.from(/B30\d/u.test(fixture) ? `${SOURCE}# ${/B30\d/u.exec(fixture)![0]}\n` : SOURCE, "utf8"),
    corpusFiles: async () => goldens(),
    runtimeDigests: () => ({ "pyproject.toml": "d".repeat(64) }),
  };
}

export const lockOf = (root: string) => recordingLockSchema.parse(JSON.parse(readFileSync(join(root, "packs/demo/upstream/bandit/recording.lock"), "utf8")));
