import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, realpathSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "vitest";

import type { CorpusFile } from "../../../conformance/port-readiness/corpus.js";
import { readGatesManifest, recordedRuntimes } from "../../../conformance/port-readiness/manifest.js";
import { gatesCorpusSchema, gatesRecordingSchema, recordingText, spanOf } from "../../../conformance/port-readiness/recording/schema.js";
import { readCachedRecording, recordingLockSchema, recordingLockText, writeCachedRecording } from "../../../conformance/port-readiness/recording/store.js";
import type { PackAdapter, UpstreamFile } from "../../../conformance/port-readiness/types.js";
import { CASE_LAYOUTS, committedSources, CONTEXT_FILES, diffRecording, notCopiedPrefixes, recordToolPack, withUpstreamRows, writeRecording, type AdapterFactory } from "../../../conformance/recorders/port-readiness/record.js";
import { extensionList, RUNTIMES_DIRECTORY, runtimePinDigests, unpinnedComposerDependencies, unpinnedNpmDependencies, unpinnedPythonDependencies, withoutMachinePaths } from "../../../conformance/recorders/port-readiness/runtimes.js";
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

const SOURCE = "import pickle\npickle.loads(b'')\n";
const span = { startLine: 1, startCol: 0, endLine: null, endCol: 17 };

// Options in the order Bandit's shell_injection gen_config builds them, not sorted: a re-record that reads the locked
// recording back from its key-sorted text must still find nothing moved.
const OPTIONS = [{ subprocess: ["subprocess.Popen"], shell: ["os.system"], no_shell: ["os.execl"] }];

const fakeAdapter: AdapterFactory = async () => ({
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

test("a fake adapter's answers are recorded in the form recording/schema.ts reads, and a second recording finds no drift", async () => {
  const root = mkdtempSync(join(tmpdir(), "port-readiness-record-"));
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

    // The lock names the recording; the recording itself is not in the tree but in the cache, by its own digest.
    const lock = recordingLockSchema.parse(JSON.parse(readFileSync(join(root, "packs/demo/upstream/bandit/recording.lock"), "utf8")));
    assert.deepEqual(lock.pins, recorded.recording.pins);
    assert.deepEqual(written.lock, lock);
    assert.equal(existsSync(join(root, "packs/demo/upstream/bandit/recording.json")), false);
    const recording = gatesRecordingSchema.parse(JSON.parse(readCachedRecording(join(root, "cache"), lock.files["recording.json"].sha256)!));
    assert.deepEqual(Object.keys(recording.rules), ["B301"]);
    assert.deepEqual(recording.absent, ["B999"]);
    // Upstream's licence travels with the excerpts the asset holds, and must be the text its digest names.
    assert.equal(recording.licenseText, "LICENSE TEXT\n");
    assert.equal(recording.licenseSha256, createHash("sha256").update("LICENSE TEXT\n").digest("hex"));
    assert.throws(() => gatesRecordingSchema.parse({ ...recording, licenseText: "ANOTHER LICENSE\n" }), /its sha256 is not licenseSha256/u);
    const older = { ...recording };
    delete older.licenseText;
    assert.equal(gatesRecordingSchema.parse(older).licenseText, undefined);
    const rule = recording.rules.B301!;
    assert.deepEqual(rule.spanParts, { endLine: false, endCol: true });
    assert.equal(rule.cases[0]!.file, "fixtures/pickle_deserialize.py");
    // The bytes the case was recorded on, which case-sources checks the committed copy against.
    assert.equal(rule.cases[0]!.sha256, createHash("sha256").update(SOURCE).digest("hex"));
    assert.deepEqual(spanOf(rule.cases[0]!.expected[0]!), span);
    const sets = [...rule.names.implementation.sets, ...rule.names.shared.sets].map((id) => recording.nameSets[id]!);
    assert.deepEqual(new Set([...rule.names.implementation.names, ...sets.flatMap((set) => set.names)]), new Set(["loads", "pickle", "qualname"]));
    assert.deepEqual(sets.flatMap((set) => set.regexes).map(({ source, flags }) => new RegExp(source, flags)), [/pick\w+/iu]);
    assert.equal(recording.parserVerdicts.length, 1);
    assert.equal(recording.corpus.extensions.includes(".py"), true);

    const corpus = gatesCorpusSchema.parse(JSON.parse(readCachedRecording(join(root, "cache"), lock.files["corpus.json"].sha256)!));
    assert.deepEqual(corpus.files.map((file) => [file.task, file.path, Object.keys(file.findings)]), [["demo-task", "app.py", ["B301"]]]);
    assert.equal(readFileSync(join(root, "packs/demo/upstream/bandit/fixtures/pickle_deserialize.py"), "utf8"), SOURCE);
    assert.match(readFileSync(join(root, "packs/demo/licenses/bandit/UPSTREAM.md"), "utf8"), /\| `upstream\/bandit\/fixtures\/pickle_deserialize\.py` \| `examples\/pickle_deserialize\.py` \| verbatim \|/u);

    const again = await recordToolPack(options);
    assert.deepEqual(diffRecording(root, "demo", "bandit", again, join(root, "cache")), { changed: [], corpusNote: null, missing: [], conflicting: [] });
    // The lock is what a commit pins, so a lock naming other bytes is what the stale check sees.
    writeFileSync(join(root, "packs/demo/upstream/bandit/recording.lock"), recordingLockText({ ...lock, files: { ...lock.files, "recording.json": { ...lock.files["recording.json"], sha256: "e".repeat(64) } } }));
    assert.deepEqual(diffRecording(root, "demo", "bandit", again, join(root, "cache")).changed, ["recording.json"]);
    // A lock whose recording is not in the cache was recorded before: the summary says it could not compare, never
    // that this is a first recording.
    const uncompared = await writeRecording(root, "demo", "bandit", again, { cacheRoot: join(root, "cache") });
    assert.equal(uncompared.changes.first, false);
    assert.equal(uncompared.changes.unavailable, `the locked recording.json (sha256 eeeeeeeeeeee) is not in the cache`);
    const compared = await writeRecording(root, "demo", "bandit", again, { cacheRoot: join(root, "cache") });
    assert.equal(compared.changes.unavailable, null);
    assert.deepEqual(compared.changes.rules, []);

    // A locked corpus.json recorded over more files than the goldens name (before the recorder narrowed to them) still
    // holds while it answers the goldens' files as a fresh recording does; --check reads it from the cache.
    const current = recordingLockSchema.parse(JSON.parse(readFileSync(join(root, "packs/demo/upstream/bandit/recording.lock"), "utf8")));
    const lockCorpus = async (value: typeof corpus) => {
      const digest = await writeCachedRecording(join(root, "cache"), recordingText(value));
      writeFileSync(join(root, "packs/demo/upstream/bandit/recording.lock"), recordingLockText({ ...current, files: { ...current.files, "corpus.json": { ...current.files["corpus.json"], sha256: digest } } }));
    };
    const extra = { ...corpus.files[0]!, task: "other-task", findings: {} };
    await lockCorpus({ ...corpus, files: [...corpus.files, extra] });
    const superset = diffRecording(root, "demo", "bandit", again, join(root, "cache"));
    assert.deepEqual(superset.changed, []);
    assert.match(superset.corpusNote!, /answers all 1 file\(s\) the goldens name, as a fresh recording does, and 1 more/u);
    await lockCorpus({ ...corpus, files: [{ ...corpus.files[0]!, findings: {} }, extra] });
    const differs = diffRecording(root, "demo", "bandit", again, join(root, "cache"));
    assert.deepEqual(differs.changed, ["corpus.json"]);
    assert.match(differs.corpusNote!, /no longer holds: 1 of the 1 file\(s\) the goldens name are answered differently or not at all, e\.g\. demo-task\/app\.py/u);
    // Without the locked recording in the cache nothing is compared, and the check never calls it current.
    writeFileSync(join(root, "packs/demo/upstream/bandit/recording.lock"), recordingLockText({ ...current, files: { ...current.files, "corpus.json": { ...current.files["corpus.json"], sha256: "f".repeat(64) } } }));
    const unread = diffRecording(root, "demo", "bandit", again, join(root, "cache"));
    assert.deepEqual(unread.changed, ["corpus.json"]);
    assert.match(unread.corpusNote!, /could not be compared: .* not in the cache/u);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("a case from an upstream path the pack's UPSTREAM.md lists as not copied is skipped and never committed", async () => {
  const root = mkdtempSync(join(tmpdir(), "port-readiness-record-"));
  try {
    mkdirSync(join(root, "packs/demo/upstream/bandit"), { recursive: true });
    mkdirSync(join(root, "packs/demo/licenses/bandit"), { recursive: true });
    writeFileSync(join(root, "packs/demo/upstream/bandit/gates.yaml"), MANIFEST);
    writeFileSync(join(root, "packs/demo/licenses/bandit/UPSTREAM.md"), "# Bandit\n\n- Not copied: `examples/pickle` — a copyleft project, GPL-3.0-only\n\n| File | Upstream path | Copy |\n|---|---|---|\n");
    writeFileSync(join(root, "packs/demo/conformance.json"), JSON.stringify({ rules: [{ id: "bandit-b301", upstream: { tool: "bandit", rule: "B301" }, policy: "rules/bandit-b301.json" }] }));
    assert.deepEqual(notCopiedPrefixes(root, "demo"), ["examples/pickle"]);
    // One case under the not-copied path and one beside it, which the recorder copies and parses as usual.
    const twoCases: AdapterFactory = async (...args) => ({
      ...(await fakeAdapter(...args)),
      cases: () => [
        { fixture: "examples/pickle_deserialize.py", variant: "default", path: "pickle_deserialize.py", source: SOURCE, skipped: null, expected: [span], markup: null },
        { fixture: "examples/assert.py", variant: "default", path: "assert.py", source: SOURCE, skipped: null, expected: [span], markup: null },
      ],
    });
    const recorded = await recordToolPack({
      root, pack: "demo", tool: "bandit", cacheRoot: join(root, "cache"), runner: { run: () => Promise.reject(new Error("no commands")) },
      adapters: { bandit: twoCases },
      upstreamBytes: () => Buffer.from(SOURCE, "utf8"),
      corpusFiles: async () => [],
      tasksSha256: "c".repeat(64),
      runtimeDigests: () => ({ "pyproject.toml": "d".repeat(64) }),
    });
    const [refused, copied] = recorded.recording.rules.B301!.cases;
    assert.equal(refused!.file, null);
    assert.equal(refused!.skipped, "not-copied");
    assert.equal(copied!.file, "fixtures/assert.py");
    assert.deepEqual(recorded.sources.map((source) => source.fixture), ["examples/assert.py"]);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("Ruff is handed the __init__ files above a golden's file, which decide its package and module name; the others read a file alone", () => {
  // django/conf/locale/ar_DZ/__init__.py is N999 only when Ruff cannot see the packages above it.
  assert.deepEqual(CONTEXT_FILES.ruff("django/conf/locale/ar_DZ/__init__.py"), [
    "django/conf/locale/ar_DZ/__init__.py", "django/conf/locale/ar_DZ/__init__.pyi",
    "django/conf/locale/__init__.py", "django/conf/locale/__init__.pyi",
    "django/conf/__init__.py", "django/conf/__init__.pyi",
    "django/__init__.py", "django/__init__.pyi",
  ]);
  assert.deepEqual(CONTEXT_FILES.ruff("setup.py"), []);
  assert.deepEqual(CONTEXT_FILES.bandit("django/conf/__init__.py"), []);
  assert.deepEqual(CONTEXT_FILES.eslint("src/vs/base/common/strings.ts"), []);
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

// The versions come from the manifests, so a bump of the pins (conformance/port-readiness/upstream-bump.ts) keeps this
// test meaning what it says: a runtime directory is named for the release it installs.
test("a lock pins every runtime the adapter installs, the environment the pack is linted in among them", () => {
  const next = readGatesManifest(PACKAGE_ROOT, "next", "eslint-plugin-next");
  assert.deepEqual(recordedRuntimes(next), [`eslint-config-next-${next.eslint!.environment!.version}`, `eslint-plugin-next-${next.upstream.version}`, "parsers"]);
  const ruff = readGatesManifest(PACKAGE_ROOT, "python", "ruff");
  assert.deepEqual(recordedRuntimes(ruff), ["python", `ruff-${ruff.upstream.version}`]);
  const phpcs = readGatesManifest(PACKAGE_ROOT, "php", "phpcs");
  assert.deepEqual(recordedRuntimes(phpcs), ["php", `phpcs-${phpcs.upstream.version}`]);
  const tlint = readGatesManifest(PACKAGE_ROOT, "blade", "tlint");
  assert.deepEqual(recordedRuntimes(tlint), ["blade", "php", `tlint-${tlint.upstream.version}`]);
});

test("a runtime's manifest pins every direct dependency at an exact version", () => {
  assert.deepEqual(unpinnedNpmDependencies({ dependencies: { eslint: "10.11.0", "eslint-v9": "npm:eslint@9.39.5", "@scope/parser-v2": "npm:@typescript-eslint/parser@2.34.0", beta: "1.0.0-rc.1" } }), []);
  assert.deepEqual(unpinnedNpmDependencies({ dependencies: { a: "^1.2.3", b: "~1.2.3", c: "1.x", d: "latest", e: "npm:x@^1.0.0" } }), ["a@^1.2.3", "b@~1.2.3", "c@1.x", "d@latest", "e@npm:x@^1.0.0"]);
  assert.deepEqual(unpinnedNpmDependencies({ overrides: { acorn: "8.18.0", "@typescript-eslint/parser-v2": { eslint: "$eslint" }, "@typescript-eslint/typescript-estree": { minimatch: { ".": "10.2.5", "brace-expansion": "^5.0.5" } } } }), ["@typescript-eslint/typescript-estree > minimatch > brace-expansion@^5.0.5"]);
  assert.deepEqual(unpinnedPythonDependencies('[project]\ndependencies = ["bandit==1.9.4", "libcst==1.9.0"]\n'), []);
  assert.deepEqual(unpinnedPythonDependencies('[project]\ndependencies = ["ruff>=0.16", "libcst~=1.9", "x"]\n'), ["ruff>=0.16", "libcst~=1.9", "x"]);
  // Single quotes, and requirements after an extras entry's `]`, are read: the list is parsed as TOML.
  assert.deepEqual(unpinnedPythonDependencies("[project]\ndependencies = ['ruff>=0.16']\n"), ["ruff>=0.16"]);
  assert.deepEqual(unpinnedPythonDependencies('[project]\ndependencies = [\n  "bandit[toml]==1.9.4",\n  "ruff>=0.16",  # a comment ]\n]\n'), ["ruff>=0.16"]);
  assert.deepEqual(unpinnedPythonDependencies('[project]\ndependencies = ["bandit[toml,sarif]>=1.9"]\n'), ["bandit[toml,sarif]>=1.9"]);
  // uv sync installs the dev group by default, and tool.uv can add requirements or change where packages come from.
  assert.deepEqual(unpinnedPythonDependencies('[project]\ndependencies = ["ruff==0.16.9"]\n[dependency-groups]\ndev = ["pytest>=8", { include-group = "lint" }]\nlint = ["ruff==0.16.9"]\n'), ["dependency-groups.dev: pytest>=8"]);
  assert.deepEqual(unpinnedPythonDependencies('[project]\ndependencies = ["ruff==0.16.9"]\n[tool.uv]\npackage = false\noverride-dependencies = ["libcst>=1"]\nsources = { ruff = { git = "https://example.com/ruff" } }\n'), ["tool.uv.override-dependencies: libcst>=1", "tool.uv.sources (not read by the pin check)"]);
  assert.deepEqual(unpinnedPythonDependencies('[project]\ndependencies = "ruff==0.16.9"\n'), ["(not a list)"]);
  assert.match(unpinnedPythonDependencies('[project]\ndependencies = ["ruff==0.16.9"\n')[0]!, /^\(not TOML: /u);
  // Composer: every requirement at an exact release, platform requirements included (`.php-version` pins the
  // interpreter), and no key that can change what is installed or run.
  assert.deepEqual(unpinnedComposerDependencies('{"name": "x", "require": {"squizlabs/php_codesniffer": "4.0.4", "nikic/php-parser": "v5.9.0", "a/b": "4.0.0-RC1"}}'), []);
  assert.deepEqual(unpinnedComposerDependencies('{"require": {"a/b": "^4.0", "c/d": "4.*", "e/f": ">=1.0 <2", "g/h": "dev-main", "php": ">=8.2", "ext-iconv": "*"}}'), ["a/b@^4.0", "c/d@4.*", "e/f@>=1.0 <2", "g/h@dev-main", "php@>=8.2", "ext-iconv@*"]);
  assert.deepEqual(unpinnedComposerDependencies('{"require": {"a/b": "1.0.0"}, "require-dev": {"c/d": "^1"}, "repositories": [], "config": {"platform": {"php": "8.1.0"}}}'), ["config (not read by the pin check)", "repositories (not read by the pin check)", "require-dev (not read by the pin check)"]);
  assert.deepEqual(unpinnedComposerDependencies('{"require": {"a/b": 1}}'), ["a/b@1"]);
  assert.deepEqual(unpinnedComposerDependencies('{"require": []}'), ["require (not an object)"]);
  assert.deepEqual(unpinnedComposerDependencies("[]"), ["(not a JSON object)"]);
  assert.match(unpinnedComposerDependencies("{")[0]!, /^\(not JSON: /u);
});

test("every committed runtime pins its direct dependencies exactly, and has no lockfile", () => {
  for (const runtime of readdirSync(RUNTIMES_DIRECTORY, { withFileTypes: true }).filter((entry) => entry.isDirectory()).map((entry) => entry.name)) {
    const directory = join(RUNTIMES_DIRECTORY, runtime);
    for (const lockfile of ["package-lock.json", "uv.lock", "composer.lock"]) assert.ok(!existsSync(join(directory, lockfile)), `${runtime}/${lockfile}`);
    if (existsSync(join(directory, "package.json"))) assert.deepEqual(unpinnedNpmDependencies(JSON.parse(readFileSync(join(directory, "package.json"), "utf8")) as { dependencies?: Record<string, string> }), [], runtime);
    if (existsSync(join(directory, "pyproject.toml"))) assert.deepEqual(unpinnedPythonDependencies(readFileSync(join(directory, "pyproject.toml"), "utf8")), [], runtime);
    if (existsSync(join(directory, "composer.json"))) {
      assert.deepEqual(unpinnedComposerDependencies(readFileSync(join(directory, "composer.json"), "utf8")), [], runtime);
      // Composer installs no interpreter, so a PHP runtime names the exact one it must run on.
      assert.match(readFileSync(join(directory, ".php-version"), "utf8"), /^\d+\.\d+\.\d+\n$/u, runtime);
    }
  }
});

test("a runtime's pin files are its manifest and, for Python and PHP, the exact interpreter and PHP's extensions", () => {
  const directory = mkdtempSync(join(tmpdir(), "gates-runtime-pins-"));
  try {
    mkdirSync(join(directory, "php"));
    writeFileSync(join(directory, "php", "composer.json"), "{}\n");
    writeFileSync(join(directory, "php", ".php-version"), "8.5.11\n");
    writeFileSync(join(directory, "php", "extensions.txt"), "Core\nmbstring\n");
    writeFileSync(join(directory, "php", "build.sh"), "not a pin\n");
    const digest = (text: string) => createHash("sha256").update(text).digest("hex");
    assert.deepEqual(runtimePinDigests("php", directory), { ".php-version": digest("8.5.11\n"), "composer.json": digest("{}\n"), "extensions.txt": digest("Core\nmbstring\n") });
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
});

test("PHP's extensions are read from `php -m` as extensions.txt lists them: headers left out, sorted, distinct", () => {
  assert.deepEqual(extensionList("[PHP Modules]\nmbstring\nCore\nZend OPcache\n\n[Zend Modules]\nZend OPcache\n\n"), ["Core", "Zend OPcache", "mbstring"]);
  // The committed list is already in that form, so a byte-identical `php -n -m` is what pinnedPhp accepts.
  const committed = readFileSync(join(RUNTIMES_DIRECTORY, "php", "extensions.txt"), "utf8");
  assert.equal(committed, `${extensionList(committed).join("\n")}\n`);
});

test("an upstream message names no directory of this machine's: each as given and resolved, the deepest first", () => {
  const root = mkdtempSync(join(tmpdir(), "rerail-machine-paths-"));
  try {
    const cache = join(root, "cache");
    const scratch = join(cache, "tmp", "tlint-abc");
    mkdirSync(scratch, { recursive: true });
    const real = realpathSync(scratch);
    const message = `TypeError: f(): Argument #1 called in ${cache}/runtimes/tlint/vendor/a.php on line 3, for ${scratch}/x.blade.php and ${real}/y.blade.php`;
    assert.equal(
      withoutMachinePaths(message, [scratch, cache]),
      "TypeError: f(): Argument #1 called in runtimes/tlint/vendor/a.php on line 3, for x.blade.php and y.blade.php",
    );
    // A directory already removed is still taken out as given.
    rmSync(scratch, { recursive: true });
    assert.equal(withoutMachinePaths(`${scratch}/x.blade.php`, [scratch, cache]), "x.blade.php");
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("PHP_CodeSniffer's unit test files are committed as fixtures/<standard>/<category>/<file>.php, as generate.py copied them", () => {
  const layout = CASE_LAYOUTS.phpcs;
  const place = (fixture: string) => layout.file({ fixture, path: `${fixture}.php`, bytes: Buffer.from("") }, () => []);
  assert.equal(place("src/Standards/Generic/Tests/PHP/LowerCaseConstantUnitTest.1.inc"), "fixtures/Generic/PHP/LowerCaseConstantUnitTest.1.inc.php");
  assert.equal(place("src/Standards/Squiz/Tests/PHP/EvalUnitTest.inc"), "fixtures/Squiz/PHP/EvalUnitTest.inc.php");
  assert.equal(place("src/Util/Tokens.php"), null);
  assert.deepEqual(CONTEXT_FILES.phpcs("src/a/b.php"), []);
});

test("a tlint case reuses the committed copy of its template, its test's own directory first, or gets one of its own", () => {
  const root = mkdtempSync(join(tmpdir(), "gates-layout-"));
  try {
    for (const [file, text] of [["fixtures/NoRayDirectiveTest/1.blade.php", "@ray($a)"], ["fixtures/NoDumpDirectivesTest/1.blade.php", "@ray($a)"], ["fixtures/NoDumpDirectivesTest/2.blade.php", "@dd($a)"]] as const) {
      mkdirSync(join(root, file, ".."), { recursive: true });
      writeFileSync(join(root, file), text);
    }
    const committed = committedSources(root);
    const layout = CASE_LAYOUTS.tlint;
    const place = (fixture: string, template: string) => layout.file({ fixture, path: "NoRayDirectiveTest/catches_ray_directive_usage.blade.php", bytes: Buffer.from(template) }, committed);
    assert.equal(place("tests/Linting/Linters/NoRayDirectiveTest.php#catches_ray_directive_usage", "@ray($a)"), "fixtures/NoRayDirectiveTest/1.blade.php");
    assert.equal(place("tests/Linting/Linters/NoRayDirectiveTest.php#catches_ray_directive_usage", "@dd($a)"), "fixtures/NoDumpDirectivesTest/2.blade.php");
    assert.match(place("tests/Linting/Linters/NoRayDirectiveTest.php#catches_ray_directive_usage", "{{$a}}")!, /^fixtures\/NoRayDirectiveTest\/[0-9a-f]{12}\.blade\.php$/u);
    assert.deepEqual(layout.row!("tests/Linting/Linters/NoRayDirectiveTest.php#catches_ray_directive_usage"), { upstream: "tests/Linting/Linters/NoRayDirectiveTest.php", copy: "derived" });
    assert.deepEqual(CONTEXT_FILES.tlint("resources/views/welcome.blade.php"), []);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});
