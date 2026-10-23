/**
 * The recording outside git: the committed `recording.lock`, the archive it names and the cache it is read from, the
 * tree inside it (one file per answer, named by what the answer depends on), and what happens when the bytes are not
 * the ones the lock names. That last one is the whole point of the manifest — a pull request that writes its own
 * recording must fail, because the gates take only the bytes the lock names, which CI records from the commit itself
 * on a cache miss, and cannot tell a true recording from a flattering one by anything but its digest.
 */
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, renameSync, rmSync, statSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, posix } from "node:path";
import ts from "typescript";
import { test } from "vitest";

import { extractRecordingTree, RECORDING_TREE_MARKER } from "../../../scripts/s3/pack-fixtures.mjs";
import { PACKAGE_ROOT } from "../../../src/engine/index.js";
import { diffRecordings, renderRecordingChanges, type RecordedPair } from "../../../conformance/port-readiness/recording/diff.js";
import { buildRecordingLock, readRecordingLock, recordingLockSchema, recordingLockText, RECORDING_LOCK, type RecordingLock } from "../../../conformance/port-readiness/recording/store.js";
import { readGatesManifest, recordedRuntimes, type GatesManifest } from "../../../conformance/port-readiness/manifest.js";
import { canonicalText, gatesRecordingSchema, type GatesRecording } from "../../../conformance/port-readiness/recording/schema.js";
import {
  ADAPTER_SOURCES, adapterSourcesDigest, answerConfig, archiveCachePath, cacheArchive, cachedTree, checkTreePaths, configId, memoryTree, packTree, parsersId, RECORDER_SOURCES,
  recorderSourcesDigest, RECORDING_ARCHIVE_FORMAT, treeDirectory, treeFiles, unpackArchive,
} from "../../../conformance/port-readiness/recording/archive.js";
import { answerKey, answerPath, answerText, fetchedContext, missingAnswers, neededAnswers, parseAnswer, verdictPath, type KeyContext } from "../../../conformance/port-readiness/recording/corpus-answers.js";
import { caseAnswerPath, caseKey, loadRecording, loadRule, recordingFiles, ruleTreeDigest, textDigest, textPath } from "../../../conformance/port-readiness/recording/tree.js";
import { parseGatesManifest } from "../../../conformance/port-readiness/manifest.js";
import { wantedPlaces } from "../../../conformance/port-readiness/corpus.js";
import { goldenBatches } from "../../../conformance/port-readiness/run.js";
import type { Span } from "../../../conformance/port-readiness/types.js";

const digest = (text: string): string => createHash("sha256").update(text).digest("hex");

const PARSERS = [{ parser: "cpython 3.14.8", grammar: true }];
const PINS = { repository: "https://github.com/PyCQA/bandit", commit: "a".repeat(40), version: "1.9.4", runtime: "bandit-1.9.4" };
const RUNTIMES = { "bandit-1.9.4": { "pyproject.toml": "d".repeat(64) } };
const SOURCE = "5".repeat(64);
const CONFIG = "c".repeat(64);
const RULES = "7".repeat(64);

function recordingOf(rules: Record<string, unknown>, fields: Record<string, unknown> = {}): GatesRecording {
  return gatesRecordingSchema.parse({
    tool: "bandit",
    adapter: "bandit",
    pins: PINS,
    runtimes: RUNTIMES,
    identity: { bandit: "1.9.4" },
    packChecks: [],
    licenseSha256: "0".repeat(64),
    inScopeSamples: ["src/example.py"],
    corpus: { extensions: [".py"], caseSensitive: true },
    keywords: [],
    commentVocabulary: [],
    fixtureWords: [],
    nameSets: {},
    rules,
    absent: [],
    parsers: PARSERS,
    ...fields,
  });
}

type Tuple = [number, number | null, number | null, number | null];
const testCase = (expected: Tuple[], fields: Record<string, unknown> = {}) => ({ fixture: "examples/pickle.py", variant: "default", path: "pickle.py", skipped: null, expected, markup: null, file: "fixtures/pickle.py", sha256: SOURCE, ...fields });
const rule = (expected: Tuple[], tests: string[] = ["examples/pickle.py"], recommended: { config: string; options: unknown[] } | null = null, name = "B301") => ({
  facts: { name, implementation: ["bandit/blacklists/calls.py"], tests, deprecation: null, recommended, notOnRealCode: null },
  names: { implementation: { sets: [], names: [], regexes: [] }, shared: { sets: [], names: [], regexes: [] } },
  spanParts: { endLine: true, endCol: true },
  cases: [testCase(expected)],
});

const spanList = (spans: Tuple[]): Span[] => spans.map(([startLine, startCol, endLine, endCol]) => ({ startLine, startCol, endLine, endCol }));

/** A corpus answers tree holding each rule's findings on demo/app.py, filed under `config`. */
function treeOf(findings: Record<string, Tuple[]>, config = CONFIG, task = "demo"): Map<string, Buffer> {
  return new Map(Object.entries(findings).map(([name, spans]) => [answerPath(config, name, { task, path: "app.py" }, "b".repeat(64)), Buffer.from(answerText({ excluded: null, findings: spanList(spans) }), "utf8")]));
}

/** A tool-pack directory with a committed lock naming the archive of `files`, and nothing else. */
function toolPack(files: Map<string, Buffer>) {
  const root = mkdtempSync(join(tmpdir(), "port-readiness-store-"));
  mkdirSync(join(root, "packs/demo/upstream/bandit"), { recursive: true });
  const packed = packTree(files);
  const lock = buildRecordingLock("bandit", PINS, RUNTIMES, packed);
  writeFileSync(join(root, "packs/demo/upstream/bandit", RECORDING_LOCK), recordingLockText(lock));
  return { root, lock, packed, cacheRoot: join(root, "cache"), cleanup: () => rmSync(root, { recursive: true, force: true }) };
}

/** The tree of a recording at CONFIG, with its corpus answers. */
const filesOf = (recording: GatesRecording, corpus: Map<string, Buffer> = new Map()) => new Map([...recordingFiles({ demo: true }, CONFIG, recording, RULES).files, ...corpus]);
const TREE = filesOf(recordingOf({ B301: rule([[1, 0, 1, 17]]) }), treeOf({ B301: [[3, 0, 3, 9]] }));

test("a lock names the one archive by its tar's digest, file count and bytes", () => {
  const { lock, packed, cleanup } = toolPack(TREE);
  try {
    assert.deepEqual(lock.files["recording.tar.zst"], { sha256: packed.sha256, files: TREE.size, bytes: packed.bytes, format: RECORDING_ARCHIVE_FORMAT });
    // The lock is small enough to review as data, which is the point of taking 71 MB of JSON out of the tree.
    assert.ok(recordingLockText(lock).length < 2000, `${recordingLockText(lock).length} bytes`);
    // It parses back, so what the recorder writes is what the gates read.
    assert.deepEqual(recordingLockSchema.parse(JSON.parse(recordingLockText(lock))), lock);
  } finally {
    cleanup();
  }
});

test("a lock of an older format is refused, saying to record the tool-pack again", () => {
  const { root, lock, cleanup } = toolPack(TREE);
  try {
    const refused = /packs\/demo\/upstream\/bandit\/recording\.lock is format "rerail\.port-readiness-recording-lock\/v[123]", not rerail\.port-readiness-recording-lock\/v4, and is not read: delete it and record the tool-pack again \(npm run port-readiness:record -- --pack demo --tool bandit\)/u;
    const entry = { sha256: "e".repeat(64), size: 10 };
    const older = [
      { ...lock, format: "rerail.port-readiness-recording-lock/v1", release: "port-readiness-recordings" },
      { ...lock, format: "rerail.port-readiness-recording-lock/v2", files: { "recording.json": entry, "corpus.json": entry } },
      { ...lock, format: "rerail.port-readiness-recording-lock/v3", files: { "recording.json": entry, "corpus.tar.zst": { sha256: "e".repeat(64), files: 1, bytes: 1 } } },
    ];
    for (const old of older) {
      writeFileSync(join(root, "packs/demo/upstream/bandit", RECORDING_LOCK), recordingLockText(old as unknown as RecordingLock));
      assert.throws(() => readRecordingLock(root, "demo", "bandit"), refused);
    }
    // A v4 lock that does not match its schema is refused by what is wrong with it.
    writeFileSync(join(root, "packs/demo/upstream/bandit", RECORDING_LOCK), recordingLockText({ ...lock, files: { ...lock.files, "recording.json": entry } } as unknown as RecordingLock));
    assert.throws(() => readRecordingLock(root, "demo", "bandit"), /recording\.lock: \/files: Unrecognized key.*recording\.json/su);
  } finally {
    cleanup();
  }
});

test("a flattering archive cached under the digest the lock names is never read", () => {
  const { cacheRoot, lock, packed, cleanup } = toolPack(TREE);
  try {
    // What a pull request would want for a rule that finds nothing, put where the lock's archive would be: the cache
    // key names the digest, so restoring it is all a poisoned cache entry could do.
    const flattering = packTree(filesOf(recordingOf({ B301: rule([]) })));
    const pin = lock.files["recording.tar.zst"];
    mkdirSync(dirname(archiveCachePath(cacheRoot, pin.sha256)), { recursive: true });
    writeFileSync(archiveCachePath(cacheRoot, pin.sha256), flattering.compressed);
    assert.equal(cachedTree(cacheRoot, pin, "demo"), undefined);
    assert.equal(existsSync(archiveCachePath(cacheRoot, pin.sha256)), false);
    // The bytes the lock does name read back.
    cacheArchive(cacheRoot, packed);
    const tree = memoryTree(treeFiles(cachedTree(cacheRoot, pin, "demo")!));
    assert.deepEqual(loadRecording(tree, CONFIG, "demo").rules.B301!.cases[0]!.expected, [[1, 0, 1, 17]]);
  } finally {
    cleanup();
  }
});

/** Every committed lock, with the runtimes the gates.yaml beside it installs. */
function committedLocks(): { label: string; lock: RecordingLock; runtimes: string[] }[] {
  const locks: { label: string; lock: RecordingLock; runtimes: string[] }[] = [];
  for (const pack of readdirSync(join(PACKAGE_ROOT, "packs"))) {
    const upstream = join(PACKAGE_ROOT, "packs", pack, "upstream");
    if (!existsSync(upstream)) continue;
    for (const tool of readdirSync(upstream)) {
      if (!existsSync(join(upstream, tool, RECORDING_LOCK))) continue;
      locks.push({ label: `${pack}/${tool}`, lock: readRecordingLock(PACKAGE_ROOT, pack, tool), runtimes: recordedRuntimes(readGatesManifest(PACKAGE_ROOT, pack, tool)) });
    }
  }
  return locks;
}

test("every committed lock pins exactly the runtimes its gates.yaml installs", () => {
  // What pins-agree checks on every run, here without reading a recording: a gates.yaml that names a runtime its lock
  // doesn't (a new environment, or one moved to another runtime directory) needs the tool-pack recorded again.
  const mismatched = committedLocks().flatMap(({ label, lock, runtimes }) => {
    const pinned = Object.keys(lock.runtimes).sort();
    return pinned.join(", ") === runtimes.join(", ") ? [] : [`${label}: gates.yaml installs ${runtimes.join(", ")}, the lock pins ${pinned.join(", ")}`];
  });
  assert.deepEqual(mismatched, []);
});

test("a recording's tree reads back as the recording, with ESLint's span parts said by its answers", () => {
  const sets = { "regexes:0123456789abcdef": { names: [], regexes: [{ source: "^on[A-Z]", flags: "u" }] } };
  const named = { ...rule([[1, 0, 1, 17]]), names: { implementation: { sets: ["regexes:0123456789abcdef"], names: ["loads"], regexes: [] }, shared: { sets: [], names: [], regexes: [] } } };
  // A not-copied case has no source to key an answer by, so its answer is in the index; a lone surrogate survives as a text.
  const notCopied = testCase([[2, 0, null, null]], { fixture: "examples/gpl.py", file: null, skipped: "not-copied" });
  delete (notCopied as { sha256?: string }).sha256;
  const recording = recordingOf({ B301: { ...named, cases: [...named.cases, notCopied] }, "plugin/B302": rule([], undefined, null, "plugin/B302") }, {
    nameSets: sets,
    absent: ["B999"],
    licenseText: "LICENSE\n",
    licenseSha256: digest("LICENSE\n"),
    fixtureTexts: { files: [{ path: "examples/pickle.py", sha256: "1".repeat(64) }], inline: [{ text: "a" }, { text: "a" }, { text: "\ud800 lone" }] },
  });
  const { files } = recordingFiles({ demo: true }, CONFIG, recording, RULES);
  assert.equal(canonicalText(loadRecording(memoryTree(files), CONFIG, "demo")), canonicalText(recording));
  // A rule's directory writes its `/` as `~`; a text is named by its file's digest.
  assert.ok(files.has(`rules/${CONFIG}/plugin~B302/rule.json`));
  assert.ok(files.has(textPath(textDigest("\ud800 lone"))));

  // ESLint's spanParts is no file: one finding of the rule without an end compares every finding on its start alone.
  const eslint = (expected: Tuple[][]) => {
    const written = recordingFiles({ demo: true }, CONFIG, { ...recordingOf({ R: { ...rule([]), cases: expected.map((spans, index) => testCase(spans, { fixture: `t.js#0/valid[${index}]`, sha256: digest(String(index)) })) } }), adapter: "eslint" }, RULES);
    assert.equal(JSON.parse(written.files.get(`rules/${CONFIG}/R/rule.json`)!.toString("utf8")).spanParts, undefined);
    return loadRule(memoryTree(written.files), CONFIG, "eslint", "R")!.spanParts;
  };
  assert.deepEqual(eslint([[[0, 1, 0, 4]], []]), { endLine: true, endCol: true });
  assert.deepEqual(eslint([[[0, 1, 0, 4]], [[1, 2, null, null]]]), { endLine: false, endCol: false });
  assert.deepEqual(eslint([[]]), { endLine: true, endCol: true });
});

test("a missing answer is a rule the recorder asks about again, and a tree the gates cannot read", () => {
  const recording = recordingOf({ B301: rule([[1, 0, 1, 17]]) });
  const files = filesOf(recording);
  const answer = caseAnswerPath(CONFIG, "B301", caseKey("bandit", recording.rules.B301!.cases[0]!, SOURCE, RULES));
  assert.ok(files.delete(answer));
  assert.equal(loadRule(memoryTree(files), CONFIG, "bandit", "B301"), undefined);
  assert.throws(() => loadRecording(memoryTree(files), CONFIG, "demo"), /lacks an answer the case index of B301 names/u);
  // Without the whole-tool facts there is no recording at that config at all.
  assert.throws(() => loadRecording(memoryTree(new Map()), CONFIG, "demo"), /has no complete tool facts at config cccccccccccc/u);
});

test("a case's answer is keyed by its source and the inputs its adapter feeds upstream", () => {
  const base = { fixture: "examples/pickle.py", variant: "default", path: "pickle.py" };
  const key = (adapter: Parameters<typeof caseKey>[0], fields: Record<string, unknown> = {}, rules = RULES) => caseKey(adapter, { ...base, ...fields }, SOURCE, rules);
  assert.match(key("bandit"), new RegExp(`^${SOURCE}\\+[0-9a-f]{12}$`, "u"));
  // Bandit lints each example on its own: the same bytes at another example are the same answer, another file type is not.
  assert.equal(key("bandit", { fixture: "examples/other.py", path: "other.py" }), key("bandit"));
  assert.notEqual(key("bandit", { path: "pickle.pyi" }), key("bandit"));
  assert.notEqual(caseKey("bandit", base, "t".repeat(64), RULES), key("bandit"));
  // Ruff reads the path (.pyi, and the __init__.py files above it).
  assert.notEqual(key("ruff", { path: "sub/pickle.py" }), key("ruff"));
  // ESLint's case configuration is no recorded field, so its key takes the case's place in upstream's tests.
  assert.notEqual(key("eslint", { fixture: "t.js#0/valid[1]" }), key("eslint", { fixture: "t.js#0/valid[0]" }));
  assert.notEqual(key("eslint", { script: true }), key("eslint", { script: false }));
  // The recorded rules key the answers of the adapters that lint them together, and no other.
  assert.notEqual(key("blade-parser", {}, "o".repeat(64)), key("blade-parser"));
  assert.notEqual(key("sqlfluff", {}, "o".repeat(64)), key("sqlfluff"));
  assert.equal(key("psalm", {}, "o".repeat(64)), key("psalm"));
  // tlint and stillat/blade-parser read a template's type as the engine does: a Blade template is not plain PHP.
  assert.notEqual(key("tlint", { path: "a.blade.php" }), key("tlint", { path: "a.php" }));
  assert.notEqual(key("blade-parser", { path: "a.blade.php" }), key("blade-parser", { path: "a.php" }));
  assert.equal(key("tlint", { path: "a.blade.php" }), key("tlint", { path: "b.blade.php" }));
});

test("two cases of a rule under one key must have the same answer, which is then filed once", () => {
  const twin = (expected: Tuple[]) => testCase(expected, { fixture: "examples/twin.py", path: "twin.py" });
  const same = recordingFiles({ demo: true }, CONFIG, recordingOf({ B301: { ...rule([[1, 0, 1, 17]]), cases: [testCase([[1, 0, 1, 17]]), twin([[1, 0, 1, 17]])] } }), RULES);
  assert.equal(same.deduplicated, 1);
  assert.throws(
    () => recordingFiles({ demo: true }, CONFIG, recordingOf({ B301: { ...rule([[1, 0, 1, 17]]), cases: [testCase([[1, 0, 1, 17]]), twin([])] } }), RULES),
    /B301: two cases are filed under the key [0-9a-f+]+ with different answers \(examples\/twin\.py among them\): the key lacks an input/u,
  );
});

test("a rule's digest moves with what it reads of the tree, and not with another rule", () => {
  const recording = recordingOf({ B301: rule([[1, 0, 1, 17]]), B302: rule([], undefined, null, "B302") });
  const files = filesOf(recording, treeOf({ B301: [[3, 0, 3, 9]], B302: [] }));
  const digestOf = (tree: Map<string, Buffer>, name: string, extra: string[] = []) => ruleTreeDigest(memoryTree(tree), CONFIG, recording, name, extra);
  const before = digestOf(files, "B301");
  const edited = (path: string) => new Map([...files, [path, Buffer.from("{}\n", "utf8")]]);
  // Another rule's index or corpus answers: no change.
  assert.equal(digestOf(edited(`rules/${CONFIG}/B302/rule.json`), "B301"), before);
  assert.equal(digestOf(edited(answerPath(CONFIG, "B302", { task: "demo", path: "app.py" }, "b".repeat(64))), "B301"), before);
  // Its own, a whole-tool fact or a verdict it reads: a change.
  assert.notEqual(digestOf(edited(`rules/${CONFIG}/B301/rule.json`), "B301"), before);
  assert.notEqual(digestOf(edited(answerPath(CONFIG, "B301", { task: "demo", path: "app.py" }, "b".repeat(64))), "B301"), before);
  assert.notEqual(digestOf(edited(`tool/${CONFIG}/keywords.json`), "B301"), before);
  const verdict = verdictPath("p".repeat(64), { path: "pickle.py", sha256: SOURCE, script: false });
  assert.notEqual(digestOf(edited(verdict), "B301", [verdict]), digestOf(files, "B301", [verdict]));
});

test("the per-rule summary names what moved, and says so plainly when nothing did", () => {
  const before: RecordedPair = { recording: recordingOf({ B301: rule([[1, 0, 1, 17]]), B302: rule([[2, 0, 2, 4]], undefined, null, "B302") }), corpus: treeOf({ B301: [[3, 0, 3, 9]] }) };
  const after: RecordedPair = { recording: recordingOf({ B301: rule([[1, 0, 1, 17], [5, 0, 5, 8]], ["examples/pickle.py", "examples/more.py"]) }), corpus: treeOf({ B301: [[4, 0, 4, 9]] }) };

  const unchanged = diffRecordings("demo", "bandit", before, before);
  assert.deepEqual(unchanged.rules, []);
  assert.deepEqual(unchanged.pinsChanged, []);
  assert.match(renderRecordingChanges([unchanged]), /re-recorded, and upstream answers the same/u);
  assert.match(renderRecordingChanges([unchanged]), /`demo\/bandit`: unchanged/u);
  // The Composer packages a recording resolved are listed, so a transitive drift shows, without making it a change.
  const resolved = renderRecordingChanges([diffRecordings("demo", "bandit", before, before, [{ runtime: "php", packages: ["nikic/php-parser@v5.9.0"] }])]);
  assert.match(resolved, /re-recorded, and upstream answers the same/u);
  assert.match(resolved, /^- `runtimes\/php`: `nikic\/php-parser@v5\.9\.0`$/mu);
  assert.doesNotMatch(renderRecordingChanges([unchanged]), /Composer packages/u);

  const changes = diffRecordings("demo", "bandit", before, after);
  assert.deepEqual(changes.rules, [
    { rule: "B301", status: "changed", expectedAdded: 1, expectedRemoved: 0, answersAdded: 0, answersRemoved: 0, answersChanged: 1, factsChanged: ["tests"] },
    { rule: "B302", status: "removed", expectedAdded: 0, expectedRemoved: 0, answersAdded: 0, answersRemoved: 0, answersChanged: 0, factsChanged: [] },
  ]);
  const rendered = renderRecordingChanges([changes]);
  assert.match(rendered, /1 tool-pack moved/u);
  assert.match(rendered, /\| `B301` \| changed \| \+1 \/ −0 \| \+0 \/ −0 \/ ~1 \| `tests` \|/u);
  assert.match(rendered, /\| `B302` \| removed \| — \| — \| — \|/u);
  // Answers are compared by rule, place and key, whatever config they are filed under: a bump that answers the same is
  // no change. An answer file one tree has and the other lacks is added or removed.
  assert.deepEqual(diffRecordings("demo", "bandit", before, { ...before, corpus: treeOf({ B301: [[3, 0, 3, 9]] }, "e".repeat(64)) }).rules, []);
  const wider: RecordedPair = { ...before, corpus: new Map([...treeOf({ B301: [[3, 0, 3, 9]] }), ...treeOf({ B301: [[1, 0, 1, 9]] }, CONFIG, "dropped")]) };
  assert.deepEqual(diffRecordings("demo", "bandit", wider, before).rules, [
    { rule: "B301", status: "changed", expectedAdded: 0, expectedRemoved: 0, answersAdded: 0, answersRemoved: 1, answersChanged: 0, factsChanged: [] },
  ]);
  // A first recording has nothing to compare with, and says that rather than listing every rule as a change.
  assert.match(renderRecordingChanges([diffRecordings("demo", "bandit", null, after)]), /first recording/u);
  // A lock whose recording could not be read is neither: the summary warns that what moved is unknown.
  const uncompared = renderRecordingChanges([diffRecordings("demo", "bandit", { unavailable: "the locked recording.tar.zst is not in the cache" }, after)]);
  assert.match(uncompared, /^## Port readiness recordings: ⚠️ 1 tool-pack not compared$/mu);
  assert.match(uncompared, /`demo\/bandit`: ⚠️ not compared\n\n.*\(the locked recording\.tar\.zst is not in the cache\)/u);
  assert.doesNotMatch(uncompared, /: first recording|✅/u);
});

test("the per-rule summary calls a fact changed only when its value changed, whatever order its object keys are in", () => {
  // Bandit's shell_injection options, in the order its gen_config builds them, which is how the recorder holds a fresh
  // recording in memory; the nested object stands for an ESLint rule's options. The recording the lock names is read
  // back from its key-sorted files, so the same options come back with every key sorted.
  const options = (shell: string[]) => [{ subprocess: ["subprocess.Popen"], shell, no_shell: ["os.execl"] }, { allow: true, ignore: { patterns: ["^_"], names: ["x"] } }];
  const recorded = (shell: string[]): RecordedPair => ({ recording: recordingOf({ B602: rule([[1, 0, 1, 17]], ["examples/subprocess_shell.py"], { config: "default", options: options(shell) }, "B602") }), corpus: null });
  const fresh = recorded(["os.system"]);
  const locked: RecordedPair = { recording: loadRecording(memoryTree(filesOf(fresh.recording)), CONFIG, "demo"), corpus: null };
  // The two hold the same options in different key orders, so the comparison below is not vacuous.
  assert.notEqual(JSON.stringify(locked.recording.rules.B602!.facts.recommended), JSON.stringify(fresh.recording.rules.B602!.facts.recommended));

  // Run 39319650531 listed `recommended` as changed for nine Bandit rules whose recordings were identical.
  assert.deepEqual(diffRecordings("demo", "bandit", locked, fresh).rules, []);
  assert.deepEqual(diffRecordings("demo", "bandit", fresh, locked).rules, []);
  assert.match(renderRecordingChanges([diffRecordings("demo", "bandit", locked, fresh)]), /`demo\/bandit`: unchanged/u);

  // A value that did change still does, and a list is a value whose order counts: only object keys are unordered.
  const changed = [{ rule: "B602", status: "changed", expectedAdded: 0, expectedRemoved: 0, answersAdded: 0, answersRemoved: 0, answersChanged: 0, factsChanged: ["recommended"] }];
  assert.deepEqual(diffRecordings("demo", "bandit", locked, recorded(["os.system", "os.popen"])).rules, changed);
  assert.deepEqual(diffRecordings("demo", "bandit", recorded(["os.popen", "os.system"]), recorded(["os.system", "os.popen"])).rules, changed);
});

test("the per-rule summary calls a fact the older recording lacks changed, instead of failing the record", () => {
  // The schema requires every fact today, but a field added later would be optional for the recordings made before it,
  // so a re-record would compare a fresh recording that has it with a locked one that does not.
  const fresh: RecordedPair = { recording: recordingOf({ B301: rule([[1, 0, 1, 17]]) }), corpus: null };
  const older = gatesRecordingSchema.parse(JSON.parse(canonicalText(fresh.recording)));
  delete (older.rules.B301!.facts as Record<string, unknown>).notOnRealCode;

  assert.deepEqual(diffRecordings("demo", "bandit", { recording: older, corpus: null }, fresh).rules, [
    { rule: "B301", status: "changed", expectedAdded: 0, expectedRemoved: 0, answersAdded: 0, answersRemoved: 0, answersChanged: 0, factsChanged: ["notOnRealCode"] },
  ]);
});

const gatesYaml = (adapter: string, commit = "a".repeat(40)) => parseGatesManifest(`adapter: ${adapter}
rerail:
  pack: demo
  project: upstream
upstream:
  name: demo/upstream
  repository: https://github.com/demo/upstream
  commit: ${commit}
  version: 1.0.0
  license: MIT
  recommended_configs:
    - default
  runtime: upstream-1.0.0
popularity:
  observed_on: 2026-10-06
  evidence:
    - a fake upstream
known_gaps: []
`, "gates.yaml");

const NO_COMMIT: KeyContext["commitOf"] = (task) => {
  throw new Error(`no commit for ${task}`);
};
const NO_BATCH: KeyContext["batchOf"] = (task) => {
  throw new Error(`no batch for ${task}`);
};
const ALONE: KeyContext = { sha256Of: () => undefined, commitOf: NO_COMMIT, batchOf: NO_BATCH, rules: "e".repeat(64) };
const placesOf = (files: Record<string, string>): KeyContext => ({ ...ALONE, sha256Of: (task, path) => files[`${task}/${path}`] });

test("a corpus answer is keyed by the file's content, plus what else the adapter's answer reads", () => {
  const file = { task: "rerail:corpus", path: "pkg/sub/mod.py", sha256: "b".repeat(64) };
  // Every adapter that reads a file on its own keys its answer by the content alone.
  assert.equal(answerKey("bandit", file, ALONE), "b".repeat(64));
  // Ruff's module name comes from the __init__.py files above the file: an __init__.py appearing is another answer.
  const bare = answerKey("ruff", file, placesOf({}));
  const packaged = answerKey("ruff", file, placesOf({ "rerail:corpus/pkg/__init__.py": "1".repeat(64) }));
  assert.match(bare, /^b{64}\+[0-9a-f]{12}$/u);
  assert.notEqual(packaged, bare);
  // A file beside it that is no context file changes nothing.
  assert.equal(answerKey("ruff", file, placesOf({ "rerail:corpus/pkg/sub/other.py": "1".repeat(64) })), bare);
  // Clippy builds whole crates, so its answer is keyed by the corpus's commit.
  assert.equal(answerKey("clippy", file, { ...ALONE, commitOf: () => "f".repeat(40) }), `${"b".repeat(64)}+${"f".repeat(12)}`);
  assert.equal(answerPath(CONFIG, "@typescript-eslint/no-explicit-any", { task: "rerail:vscode", path: "src/a.ts" }, "k"), `answers/${CONFIG}/@typescript-eslint~no-explicit-any/rerail~vscode/src/a.ts@k.json`);
  assert.equal(verdictPath("p", { path: "views/a.blade.php", sha256: "b".repeat(64), script: true }), `verdicts/p/${"b".repeat(64)}.blade.php.script.json`);
});

test("PHPStan's and Psalm's answers are keyed by every fetched file of their task, which upstream analyses as one project", () => {
  const file = { task: "demo", path: "src/A.php", sha256: "a".repeat(64) };
  const sibling = { task: "demo", path: "src/B.php", sha256: "b".repeat(64) };
  const elsewhere = { task: "other", path: "src/C.php", sha256: "c".repeat(64) };
  const alone = answerKey("phpstan", file, fetchedContext([file], NO_COMMIT, ["R1"]));
  assert.match(alone, /^a{64}\+[0-9a-f]{12}$/u);
  // Another file of the task in the batch is another answer; one of another task is not in the batch.
  assert.notEqual(answerKey("phpstan", file, fetchedContext([file, sibling], NO_COMMIT, ["R1"])), alone);
  assert.equal(answerKey("phpstan", file, fetchedContext([file, elsewhere], NO_COMMIT, ["R1"])), alone);
  assert.equal(answerKey("psalm", file, fetchedContext([sibling, file], NO_COMMIT, ["R1"])), answerKey("psalm", file, fetchedContext([file, sibling], NO_COMMIT, ["R1"])));
  // A tool that reads each file on its own is not keyed by the batch.
  assert.equal(answerKey("phpcs", file, fetchedContext([file, sibling], NO_COMMIT, ["R1"])), "a".repeat(64));
});

test("a PHPStan or Psalm rule's key reads the batch every rule's goldens make in its tasks, so cached equals uncached", () => {
  const x = { task: "demo", path: "src/A.php", sha256: "a".repeat(64) };
  const y = { task: "demo", path: "src/B.php", sha256: "b".repeat(64) };
  const elsewhere = { task: "other", path: "src/C.php", sha256: "c".repeat(64) };
  for (const adapter of ["phpstan", "psalm"] as const) {
    // With another rule's goldens `others`: what X's cached result is keyed by (run.ts builds `wanted` from every
    // golden, as it fetches them), and the answer an uncached run reads for X on its golden.
    const state = (others: (typeof x)[]) => {
      const named = [x, ...others];
      return {
        key: goldenBatches(adapter, wantedPlaces(named), [x]),
        answer: answerPath(CONFIG, "X", x, answerKey(adapter, x, fetchedContext(named, NO_COMMIT, ["X", "Y"]))),
      };
    };
    const alone = state([]);
    const added = state([y]);
    const removed = state([]);
    // Another rule's golden in X's task is another batch: another answer read, another key.
    assert.notEqual(added.answer, alone.answer, adapter);
    assert.notEqual(added.key, alone.key, adapter);
    // Removed again, the answer read and the key are the first ones.
    assert.deepEqual(removed, alone, adapter);
    // A golden of another task is no part of the batch, and moves neither.
    assert.deepEqual(state([elsewhere]), alone, adapter);
    // An archive recorded at both batches keeps both answers, and X's tree digest (ruleTreeDigest) reads every file
    // under its answers: the same digest whichever the run reads, so the batch is what tells the two apart.
    const both = memoryTree(new Map([alone.answer, added.answer].map((path) => [path, Buffer.from("{}\n", "utf8")])));
    assert.deepEqual(both.list(`answers/${CONFIG}/X/`), [alone.answer, added.answer].sort(), adapter);
  }
  // An adapter that reads each file alone: another rule's golden is no part of a rule's key.
  assert.equal(goldenBatches("bandit", wantedPlaces([x, y]), [x]), "");
  assert.equal(goldenBatches("phpcs", wantedPlaces([x, y]), [x]), "");
});

test("stillat/blade-parser's and PHPStan's corpus answers are keyed by the rules recorded, since one rule asked excludes a file for all", () => {
  const file = { task: "demo", path: "views/a.blade.php", sha256: "a".repeat(64) };
  const keyed = (adapter: "blade-parser" | "phpstan" | "psalm" | "tlint", rules: string[]) => answerKey(adapter, file, fetchedContext([file], NO_COMMIT, rules));
  assert.match(keyed("blade-parser", ["V1", "V2"]), /^a{64}\+[0-9a-f]{12}$/u);
  // The set, however listed, is the key; a validator or level added or removed is another answer, which is asked again.
  assert.equal(keyed("blade-parser", ["V2", "V1", "V2"]), keyed("blade-parser", ["V1", "V2"]));
  assert.notEqual(keyed("blade-parser", ["V1"]), keyed("blade-parser", ["V1", "V2"]));
  assert.notEqual(keyed("phpstan", ["L1"]), keyed("phpstan", ["L1", "L2"]));
  // PHPStan's key keeps the batch too, and an adapter that answers a rule whatever else is asked is not keyed by them.
  assert.notEqual(answerKey("phpstan", file, fetchedContext([file, { ...file, path: "b.php" }], NO_COMMIT, ["L1"])), keyed("phpstan", ["L1"]));
  assert.equal(keyed("psalm", ["P1"]), keyed("psalm", ["P1", "P2"]));
  assert.equal(keyed("tlint", ["T1"]), "a".repeat(64));
  // The gates and the recorder build the context alike: the needed answers' names are those keys.
  const needed = (rules: string[]) => neededAnswers({ root: PACKAGE_ROOT, manifest: gatesYaml("blade-parser"), runtimes: RUNTIMES, parsers: PARSERS, rules, files: [file], context: fetchedContext([file], NO_COMMIT, rules) });
  assert.deepEqual(needed(["V1", "V2"]).answers.map((answer) => answer.path.slice(answer.path.lastIndexOf("@"))), [`@${keyed("blade-parser", ["V1", "V2"])}.json`, `@${keyed("blade-parser", ["V1", "V2"])}.json`]);
  assert.equal(needed(["V1"]).answers.some((answer) => needed(["V1", "V2"]).answers.some((other) => other.path === answer.path)), false);
});

test("the config id and the parsers id name the recorder's adapter code and the code that files its answers", () => {
  const needed = neededAnswers({ root: PACKAGE_ROOT, manifest: gatesYaml("bandit"), runtimes: RUNTIMES, parsers: PARSERS, rules: ["B301"], files: [], context: ALONE });
  assert.equal(needed.config.sources, adapterSourcesDigest("bandit"));
  assert.equal(needed.config.recorder, recorderSourcesDigest());
  assert.equal(needed.parsersId, parsersId("bandit", PARSERS));
  // The same parsers run by other adapter code are another parsers id.
  assert.notEqual(parsersId("ruff", PARSERS), parsersId("bandit", PARSERS));
  assert.notEqual(adapterSourcesDigest("ruff"), adapterSourcesDigest("bandit"));
  // The recorder code every adapter files its answers through: the rule records, the keys, spans and canonical text.
  for (const path of ["conformance/recorders/port-readiness/record.ts", "conformance/port-readiness/recording/tree.ts", "conformance/port-readiness/recording/schema.ts"]) {
    assert.ok(RECORDER_SOURCES.includes(path), `RECORDER_SOURCES leaves out ${path}`);
  }
});

test("the config id covers every gates.yaml setting the adapter reads and the pack's not-copied prefixes", () => {
  const eslint = (block: string, scope = "") => parseGatesManifest(`adapter: eslint
rerail:
  pack: demo
  project: upstream
${scope}upstream:
  name: demo/upstream
  repository: https://github.com/demo/upstream
  commit: ${"a".repeat(40)}
  version: 1.0.0
  license: MIT
  recommended_configs:
    - recommended
  runtime: upstream-1.0.0
popularity:
  observed_on: 2026-10-06
  evidence:
    - a fake upstream
known_gaps: []
eslint:
  plugin: null
  release:
    tag: v1.0.0
    path: lib
    installed: eslint/lib
    verbatim: true
  implementation: lib/rules/{rule}.js
  helpers: []
  checkout:
    - /lib/
${block}`, "gates.yaml");
  const root = mkdtempSync(join(tmpdir(), "port-readiness-config-"));
  try {
    const id = (manifest: GatesManifest) => configId(answerConfig(root, manifest, RUNTIMES));
    const base = id(eslint("  tests:\n    - tests/lib/rules/*.js\n  parsers:\n    espree: javascript\n"));
    // Which tests are read, how a parser's tree is read, and which file types the pack covers: each a new config.
    assert.notEqual(id(eslint("  tests:\n    - tests/lib/rules/a*.js\n  parsers:\n    espree: javascript\n")), base);
    assert.notEqual(id(eslint("  tests:\n    - tests/lib/rules/*.js\n  parsers:\n    espree: typescript\n")), base);
    assert.notEqual(id(eslint("  tests:\n    - tests/lib/rules/*.js\n  parsers:\n    espree: javascript\n  fixture_parsers:\n    tests/fixtures/parsers/: .ts\n")), base);
    assert.notEqual(id(eslint("  tests:\n    - tests/lib/rules/*.js\n  parsers:\n    espree: javascript\n", "  in_scope:\n    - .js\n")), base);
    // An UPSTREAM.md "Not copied" line skips the cases under it.
    mkdirSync(join(root, "packs/demo/licenses/upstream"), { recursive: true });
    writeFileSync(join(root, "packs/demo/licenses/upstream/UPSTREAM.md"), "# Upstream\n\n- Not copied: `tests/lib/rules/gpl/`\n");
    assert.notEqual(id(eslint("  tests:\n    - tests/lib/rules/*.js\n  parsers:\n    espree: javascript\n")), base);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

/**
 * Every file under the recorder or the span units an adapter reaches: relative imports, `new URL("./x", import.meta.url)`
 * drivers and PHP `require __DIR__ . '/x'`. A directory one names is runtimes.ts's runtimes/ of pin files, which the
 * config id holds by their digests (recording.lock `runtimes`).
 */
function adapterGraph(entry: string): Set<string> {
  const reached = new Set<string>();
  const queue = [entry];
  while (queue.length > 0) {
    const file = queue.pop()!;
    if (reached.has(file) || !(file.startsWith("conformance/recorders/port-readiness/") || file.startsWith("conformance/port-readiness/span-units/"))) continue;
    reached.add(file);
    const text = readFileSync(join(PACKAGE_ROOT, file), "utf8");
    const specifiers = [
      ...(/\.(?:[cm]?[jt]s)$/u.test(file) ? ts.preProcessFile(text, true, true).importedFiles.map(({ fileName }) => fileName) : []),
      ...[...text.matchAll(/new URL\("(\.\.?\/[^"]+)", import\.meta\.url\)/gu)].map((match) => match[1]!),
      ...[...text.matchAll(/^\s*(?:require|include)(?:_once)?\s+__DIR__\s*\.\s*'(\/[^']+)'/gmu)].map((match) => `.${match[1]!}`),
    ].filter((specifier) => specifier.startsWith("./") || specifier.startsWith("../"));
    for (const specifier of specifiers) {
      const target = posix.join(posix.dirname(file), specifier);
      const typed = target.endsWith(".js") && existsSync(join(PACKAGE_ROOT, `${target.slice(0, -3)}.ts`)) ? `${target.slice(0, -3)}.ts` : target;
      assert.ok(existsSync(join(PACKAGE_ROOT, typed)), `${file} names ${specifier}, which resolves to no file`);
      if (statSync(join(PACKAGE_ROOT, typed)).isFile()) queue.push(typed);
    }
  }
  return reached;
}

test("ADAPTER_SOURCES covers every recorder file and span unit each adapter reaches", () => {
  const tracked = (pathspec: readonly string[]) => new Set(execFileSync("git", ["ls-files", "-z", "--", ...pathspec], { cwd: PACKAGE_ROOT, encoding: "utf8" }).split("\0").filter((path) => path !== ""));
  for (const [adapter, pathspec] of Object.entries(ADAPTER_SOURCES)) {
    const keyed = tracked(pathspec);
    for (const entry of pathspec) assert.ok(tracked([entry]).size > 0, `${adapter}: ${entry} names no tracked file`);
    const entry = adapter === "eslint" ? "conformance/recorders/port-readiness/adapters/eslint/eslint.ts" : `conformance/recorders/port-readiness/adapters/${adapter}.ts`;
    const graph = adapterGraph(entry);
    assert.ok(graph.has("conformance/recorders/port-readiness/runtimes.ts"), `${adapter}'s graph misses runtimes.ts`);
    for (const path of graph) assert.ok(keyed.has(path), `${adapter} reaches ${path}, which ADAPTER_SOURCES leaves out`);
  }
});

test("the corpus answers needed are every golden file for every recorded rule at this config, and a missing one is found by its absence", () => {
  const manifest = gatesYaml("bandit");
  const files = [{ task: "demo", path: "a.py", sha256: "1".repeat(64) }, { task: "demo", path: "b.py", sha256: "2".repeat(64) }, { task: "demo", path: "a.py", sha256: "1".repeat(64) }];
  const options = { root: PACKAGE_ROOT, manifest, runtimes: RUNTIMES, parsers: PARSERS, rules: ["B302", "B301"], files, context: ALONE };
  const needed = neededAnswers(options);
  // A golden listed under two rule ids is one file to answer.
  assert.equal(needed.answers.length, 4);
  assert.equal(needed.verdicts.length, 2);
  assert.deepEqual(needed.answers.map((answer) => `${answer.rule} ${answer.file.path}`), ["B301 a.py", "B301 b.py", "B302 a.py", "B302 b.py"]);
  assert.equal(needed.configId, configId(needed.config));

  const tree = new Map(needed.answers.slice(1).map((answer) => [answer.path, Buffer.from(answerText({ excluded: null, findings: [] }), "utf8")]));
  tree.set(needed.verdicts[0]!.path, Buffer.from("[]\n", "utf8"));
  const missing = missingAnswers(memoryTree(tree), needed);
  assert.deepEqual(missing.answers.map((answer) => `${answer.rule} ${answer.file.path}`), ["B301 a.py"]);
  assert.deepEqual(missing.verdicts.map((verdict) => verdict.file.path), ["b.py"]);
  // Upstream asked and silent is an answer; it is not the absence of one.
  assert.deepEqual(parseAnswer(tree.get(needed.answers[1]!.path)!.toString("utf8"), "b.py"), { findings: [] });

  // Moving upstream's pins is a new config, under which nothing is answered yet.
  const bumped = neededAnswers({ ...options, manifest: gatesYaml("bandit", "e".repeat(40)) });
  assert.notEqual(bumped.configId, needed.configId);
  assert.equal(missingAnswers(memoryTree(tree), bumped).answers.length, 4);
});

test("the archive is canonical, verified before it is extracted, and refuses paths a case-insensitive file system merges", () => {
  const entries = [...TREE];
  const packed = packTree(new Map(entries));
  // The same files make the same archive, whatever order they were added in.
  assert.equal(packTree(new Map([...entries].reverse())).sha256, packed.sha256);
  assert.deepEqual(unpackArchive(packed.compressed, packed, "demo").map((entry) => entry.path), [...entries.map(([path]) => path)].sort());
  assert.throws(() => unpackArchive(packed.compressed, { ...packed, files: packed.files + 1 }, "demo"), /the lock says/u);
  assert.throws(() => unpackArchive(packed.compressed, { ...packed, sha256: "0".repeat(64) }, "demo"), /not the locked/u);

  assert.throws(() => checkTreePaths(["answers/c/B301/demo/App.py@k.json", "answers/c/B301/demo/app.py@k.json"]), /differ only by letter case/u);
  assert.throws(() => checkTreePaths(["recording.json"]), /holds only answers\/, config\/, rules\/, texts\/, tool\/, verdicts\/, not recording\.json/u);
  // A bare name is no directory, even one a top-level directory's name starts with.
  for (const bare of ["tools", "config", "/config/a.json"]) assert.throws(() => checkTreePaths([bare]), /holds only/u);

  const cacheRoot = mkdtempSync(join(tmpdir(), "port-readiness-archive-"));
  try {
    cacheArchive(cacheRoot, packed);
    const archive = archiveCachePath(cacheRoot, packed.sha256);
    const tree = treeDirectory(cacheRoot, packed.sha256);
    assert.ok(existsSync(archive));
    // The extraction's marker names the tar, and is no file of the tree.
    assert.equal(readFileSync(join(tree, RECORDING_TREE_MARKER), "utf8"), `${packed.sha256}\n`);
    assert.deepEqual([...treeFiles(tree).keys()].sort(), [...entries.map(([path]) => path)].sort());
    const config = `config/${CONFIG}.json`;
    // A tree that lost a file is extracted again from the cached archive.
    rmSync(join(tree, "config"), { recursive: true });
    assert.ok(existsSync(join(cachedTree(cacheRoot, packed, "demo")!, config)));
    // A tree of the right counts whose marker names no archive is not reused: one file's bytes were swapped in place.
    const original = readFileSync(join(tree, config));
    writeFileSync(join(tree, config), Buffer.from(original.toString("utf8").replace("true", "nope"), "utf8"));
    writeFileSync(join(tree, RECORDING_TREE_MARKER), `${"0".repeat(64)}\n`);
    assert.deepEqual(readFileSync(join(cachedTree(cacheRoot, packed, "demo")!, config)), original);
    // Nor is one whose marker names the archive but whose files were edited, the counts and bytes kept: a finding's
    // coordinate changed, or an answer renamed. Each is extracted again from the verified archive.
    const answer = [...treeFiles(tree).keys()].find((path) => path.startsWith("answers/"))!;
    const kept = readFileSync(join(tree, answer));
    const moved = Buffer.from(kept.toString("utf8").replace("3", "4"), "utf8");
    assert.equal(moved.length, kept.length);
    writeFileSync(join(tree, answer), moved);
    assert.deepEqual(readFileSync(join(cachedTree(cacheRoot, packed, "demo")!, answer)), kept);
    renameSync(join(tree, answer), join(tree, `${answer}.renamed`));
    assert.deepEqual(treeFiles(cachedTree(cacheRoot, packed, "demo")!).get(answer), kept);
    // Another process's tree of the same archive, there first, is accepted as this one.
    extractRecordingTree(tree, unpackArchive(packed.compressed, packed, "demo"), packed);
    assert.equal(cachedTree(cacheRoot, packed, "demo"), tree);
    assert.deepEqual(readdirSync(dirname(tree)), [packed.sha256]);
    // A corrupt cached archive is no copy of it: it is deleted, so the caller fetches it again.
    rmSync(tree, { recursive: true });
    writeFileSync(archive, Buffer.from("not zstd"));
    assert.equal(cachedTree(cacheRoot, packed, "demo"), undefined);
    assert.equal(existsSync(archive), false);
    // With neither the tree nor the archive, there is nothing to read.
    assert.equal(cachedTree(cacheRoot, { ...packed, sha256: "0".repeat(64) }, "demo"), undefined);
  } finally {
    rmSync(cacheRoot, { recursive: true, force: true });
  }
});
