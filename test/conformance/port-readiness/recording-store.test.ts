/**
 * The recordings outside git: the committed `recording.lock`, the cache they are read from, and what happens when the
 * bytes are not the ones the lock names. That last one is the whole point of the manifest — a pull request that writes
 * its own recording must fail, because the gates take only the bytes the lock names, which CI records from the commit
 * itself on a cache miss, and cannot tell a true recording from a flattering one by anything but its digest.
 */
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, renameSync, rmSync, statSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, posix } from "node:path";
import ts from "typescript";
import { test } from "vitest";

import { CORPUS_TREE_MARKER, extractCorpusTree } from "../../../scripts/s3/pack-fixtures.mjs";
import { PACKAGE_ROOT } from "../../../src/engine/index.js";
import { diffRecordings, renderRecordingChanges, type RecordedPair } from "../../../conformance/port-readiness/recording/diff.js";
import {
  buildRecordingLock, readCachedRecording, readRecordingLock, recordingCachePath, recordingLockSchema, recordingLockText, writeCachedRecording, RECORDING_LOCK,
  type RecordingLock,
} from "../../../conformance/port-readiness/recording/store.js";
import { readGatesManifest, recordedRuntimes } from "../../../conformance/port-readiness/manifest.js";
import { canonicalText, gatesRecordingSchema, RECORDING_FORMAT } from "../../../conformance/port-readiness/recording/schema.js";
import {
  ADAPTER_SOURCES, adapterSourcesDigest, answerKey, answerPath, answerText, cacheCorpusArchive, cachedCorpusTree, checkTreePaths, configId, corpusArchiveCachePath, corpusTreeDirectory,
  CORPUS_ANSWERS_FORMAT, fetchedContext, memoryTree, missingAnswers, neededAnswers, nextTree, packCorpusTree, parseAnswer, parsersId, treeFiles, unpackCorpusArchive, verdictPath,
  type KeyContext,
} from "../../../conformance/port-readiness/recording/corpus-answers.js";
import { parseGatesManifest } from "../../../conformance/port-readiness/manifest.js";
import type { Span } from "../../../conformance/port-readiness/types.js";

const digest = (text: string): string => createHash("sha256").update(text).digest("hex");

const PARSERS = [{ parser: "cpython 3.14.8", grammar: true }];
const PINS = { repository: "https://github.com/PyCQA/bandit", commit: "a".repeat(40), version: "1.9.4", runtime: "bandit-1.9.4" };
const RUNTIMES = { "bandit-1.9.4": { "pyproject.toml": "d".repeat(64) } };

function recordingOf(rules: Record<string, unknown>): ReturnType<typeof gatesRecordingSchema.parse> {
  return gatesRecordingSchema.parse({
    format: RECORDING_FORMAT,
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
    parserVerdicts: [],
  });
}

const rule = (expected: [number, number | null, number | null, number | null][], tests: string[] = ["examples/pickle.py"], recommended: { config: string; options: unknown[] } | null = null) => ({
  facts: { name: "B301", implementation: ["bandit/blacklists/calls.py"], tests, deprecation: null, recommended, notOnRealCode: null },
  names: { implementation: { sets: [], names: [], regexes: [] }, shared: { sets: [], names: [], regexes: [] } },
  spanParts: { endLine: true, endCol: true },
  cases: [{ fixture: "examples/pickle.py", variant: "default", path: "pickle.py", skipped: null, expected, markup: null, file: "fixtures/pickle.py" }],
});

const spanList = (spans: [number, number | null, number | null, number | null][]): Span[] => spans.map(([startLine, startCol, endLine, endCol]) => ({ startLine, startCol, endLine, endCol }));

/** A corpus answers tree holding each rule's findings on demo/app.py, filed under `config`. */
function treeOf(findings: Record<string, [number, number | null, number | null, number | null][]>, config = "c".repeat(64), task = "demo"): Map<string, Buffer> {
  return new Map(Object.entries(findings).map(([name, spans]) => [answerPath(config, name, { task, path: "app.py" }, "b".repeat(64)), Buffer.from(answerText({ excluded: null, findings: spanList(spans) }), "utf8")]));
}

/** A tool-pack directory with a committed lock for recording.json and a corpus answers archive, and nothing else. */
function toolPack(recording: string, corpus: Map<string, Buffer>) {
  const root = mkdtempSync(join(tmpdir(), "port-readiness-store-"));
  mkdirSync(join(root, "packs/demo/upstream/bandit"), { recursive: true });
  const packed = packCorpusTree(corpus);
  const lock = buildRecordingLock("bandit", PINS, RUNTIMES, recording, packed);
  writeFileSync(join(root, "packs/demo/upstream/bandit", RECORDING_LOCK), recordingLockText(lock));
  return { root, lock, packed, cacheRoot: join(root, "cache"), cleanup: () => rmSync(root, { recursive: true, force: true }) };
}

const RECORDING = canonicalText(recordingOf({ B301: rule([[1, 0, 1, 17]]) }));
const TREE = treeOf({ B301: [[3, 0, 3, 9]] });

test("a lock names recording.json by its digest and the corpus answers archive by its tar's digest, file count and bytes", () => {
  const { lock, packed, cleanup } = toolPack(RECORDING, TREE);
  try {
    assert.equal(lock.files["recording.json"].sha256, digest(RECORDING));
    assert.equal(lock.files["recording.json"].format, RECORDING_FORMAT);
    assert.deepEqual(lock.files["corpus.tar.zst"], { sha256: packed.sha256, files: 1, bytes: packed.bytes, format: CORPUS_ANSWERS_FORMAT });
    // The lock is small enough to review as data, which is the point of taking 71 MB of JSON out of the tree.
    assert.ok(recordingLockText(lock).length < 2000, `${recordingLockText(lock).length} bytes`);
    // It parses back, so what the recorder writes is what the gates read.
    assert.deepEqual(recordingLockSchema.parse(JSON.parse(recordingLockText(lock))), lock);
  } finally {
    cleanup();
  }
});

test("a lock of an older format is refused, saying to record the tool-pack again", () => {
  const { root, lock, cleanup } = toolPack(RECORDING, TREE);
  try {
    const refused = /packs\/demo\/upstream\/bandit\/recording\.lock is format "rerail\.port-readiness-recording-lock\/v[12]", not rerail\.port-readiness-recording-lock\/v3, and is not read: delete it and record the tool-pack again \(npm run port-readiness:record -- --pack demo --tool bandit\)/u;
    const v1 = { ...lock, format: "rerail.port-readiness-recording-lock/v1", release: "port-readiness-recordings" };
    writeFileSync(join(root, "packs/demo/upstream/bandit", RECORDING_LOCK), recordingLockText(v1 as RecordingLock));
    assert.throws(() => readRecordingLock(root, "demo", "bandit"), refused);
    const v2 = { ...lock, format: "rerail.port-readiness-recording-lock/v2", files: { "recording.json": lock.files["recording.json"], "corpus.json": { sha256: "e".repeat(64), size: 10, format: "rerail.port-readiness-corpus/v2" } } };
    writeFileSync(join(root, "packs/demo/upstream/bandit", RECORDING_LOCK), recordingLockText(v2 as unknown as RecordingLock));
    assert.throws(() => readRecordingLock(root, "demo", "bandit"), refused);
    // A v3 lock that does not match its schema is refused by what is wrong with it.
    writeFileSync(join(root, "packs/demo/upstream/bandit", RECORDING_LOCK), recordingLockText({ ...lock, release: "x" } as RecordingLock));
    assert.throws(() => readRecordingLock(root, "demo", "bandit"), /recording\.lock: \/: Unrecognized key.*release/su);
  } finally {
    cleanup();
  }
});

test("a flattering recording cached under the digest the lock names is never read", async () => {
  const { cacheRoot, lock, cleanup } = toolPack(RECORDING, TREE);
  try {
    // What a pull request would want for a rule that finds nothing, put where the lock's recording would be: the cache
    // key names the digest, so restoring it is all a poisoned cache entry could do.
    const flattering = canonicalText(recordingOf({ B301: rule([]) }));
    const path = recordingCachePath(cacheRoot, lock.files["recording.json"].sha256);
    mkdirSync(join(cacheRoot, "recordings"), { recursive: true });
    writeFileSync(path, flattering);
    assert.equal(readCachedRecording(cacheRoot, lock.files["recording.json"].sha256), undefined);
    // The bytes the lock does name read back.
    await writeCachedRecording(cacheRoot, RECORDING);
    assert.equal(readCachedRecording(cacheRoot, lock.files["recording.json"].sha256), RECORDING);
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

test("a recording whose cached bytes were tampered with reads as absent, not as a recording", async () => {
  const { cacheRoot } = toolPack(RECORDING, TREE);
  const written = await writeCachedRecording(cacheRoot, RECORDING);
  assert.equal(readCachedRecording(cacheRoot, written), RECORDING);
  writeFileSync(join(cacheRoot, "recordings", `${written}.json`), "{}\n");
  assert.equal(readCachedRecording(cacheRoot, written), undefined);
  rmSync(cacheRoot, { recursive: true, force: true });
});

test("the per-rule summary names what moved, and says so plainly when nothing did", () => {
  const before: RecordedPair = { recording: recordingOf({ B301: rule([[1, 0, 1, 17]]), B302: rule([[2, 0, 2, 4]]) }), corpus: treeOf({ B301: [[3, 0, 3, 9]] }) };
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
  const wider: RecordedPair = { ...before, corpus: new Map([...treeOf({ B301: [[3, 0, 3, 9]] }), ...treeOf({ B301: [[1, 0, 1, 9]] }, "c".repeat(64), "dropped")]) };
  assert.deepEqual(diffRecordings("demo", "bandit", wider, before).rules, [
    { rule: "B301", status: "changed", expectedAdded: 0, expectedRemoved: 0, answersAdded: 0, answersRemoved: 1, answersChanged: 0, factsChanged: [] },
  ]);
  // A first recording has nothing to compare with, and says that rather than listing every rule as a change.
  assert.match(renderRecordingChanges([diffRecordings("demo", "bandit", null, after)]), /first recording/u);
  // A lock whose recording could not be read is neither: the summary warns that what moved is unknown.
  const uncompared = renderRecordingChanges([diffRecordings("demo", "bandit", { unavailable: "demo-bandit-recording.json.gz is not in the cache" }, after)]);
  assert.match(uncompared, /^## Port readiness recordings: ⚠️ 1 tool-pack not compared$/mu);
  assert.match(uncompared, /`demo\/bandit`: ⚠️ not compared\n\n.*\(demo-bandit-recording\.json\.gz is not in the cache\)/u);
  assert.doesNotMatch(uncompared, /: first recording|✅/u);
});

test("the per-rule summary calls a fact changed only when its value changed, whatever order its object keys are in", () => {
  // Bandit's shell_injection options, in the order its gen_config builds them, which is how the recorder holds a fresh
  // recording in memory; the nested object stands for an ESLint rule's options. The recording the lock names is read
  // back from its key-sorted text, so the same options come back with every key sorted.
  const options = (shell: string[]) => [{ subprocess: ["subprocess.Popen"], shell, no_shell: ["os.execl"] }, { allow: true, ignore: { patterns: ["^_"], names: ["x"] } }];
  const recorded = (shell: string[]): RecordedPair => ({ recording: recordingOf({ B602: rule([[1, 0, 1, 17]], ["examples/subprocess_shell.py"], { config: "default", options: options(shell) }) }), corpus: null });
  const fresh = recorded(["os.system"]);
  const locked: RecordedPair = { recording: gatesRecordingSchema.parse(JSON.parse(canonicalText(fresh.recording))), corpus: null };
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

test("an answer is keyed by the file's content, plus what else the adapter's answer reads", () => {
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
  assert.equal(answerPath("c".repeat(64), "@typescript-eslint/no-explicit-any", { task: "rerail:vscode", path: "src/a.ts" }, "k"), `answers/${"c".repeat(64)}/@typescript-eslint~no-explicit-any/rerail~vscode/src/a.ts@k.json`);
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

test("stillat/blade-parser's and PHPStan's answers are keyed by the rules recorded, since one rule asked excludes a file for all", () => {
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
  const needed = (rules: string[]) => neededAnswers({ manifest: gatesYaml("blade-parser"), runtimes: RUNTIMES, parsers: PARSERS, rules, files: [file], context: fetchedContext([file], NO_COMMIT, rules) });
  assert.deepEqual(needed(["V1", "V2"]).answers.map((answer) => answer.path.slice(answer.path.lastIndexOf("@"))), [`@${keyed("blade-parser", ["V1", "V2"])}.json`, `@${keyed("blade-parser", ["V1", "V2"])}.json`]);
  assert.equal(needed(["V1"]).answers.some((answer) => needed(["V1", "V2"]).answers.some((other) => other.path === answer.path)), false);
});

test("the config id and the parsers id name the recorder's adapter code", () => {
  const needed = neededAnswers({ manifest: gatesYaml("bandit"), runtimes: RUNTIMES, parsers: PARSERS, rules: ["B301"], files: [], context: ALONE });
  assert.equal(needed.config.sources, adapterSourcesDigest("bandit"));
  assert.equal(needed.parsersId, parsersId("bandit", PARSERS));
  // The same parsers run by other adapter code are another parsers id.
  assert.notEqual(parsersId("ruff", PARSERS), parsersId("bandit", PARSERS));
  assert.notEqual(adapterSourcesDigest("ruff"), adapterSourcesDigest("bandit"));
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

test("the answers needed are every golden file for every recorded rule at this config, and a missing one is found by its absence", () => {
  const manifest = gatesYaml("bandit");
  const files = [{ task: "demo", path: "a.py", sha256: "1".repeat(64) }, { task: "demo", path: "b.py", sha256: "2".repeat(64) }, { task: "demo", path: "a.py", sha256: "1".repeat(64) }];
  const options = { manifest, runtimes: RUNTIMES, parsers: PARSERS, rules: ["B302", "B301"], files, context: ALONE };
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

test("a record keeps the answers of its config, drops other configs', and drops answers no golden needs only when pruned", () => {
  const needed = neededAnswers({ manifest: gatesYaml("bandit"), runtimes: RUNTIMES, parsers: PARSERS, rules: ["B301"], files: [{ task: "demo", path: "a.py", sha256: "1".repeat(64) }], context: ALONE });
  const answer = Buffer.from(answerText({ excluded: null, findings: [] }), "utf8");
  const stale = answerPath(needed.configId, "B301", { task: "demo", path: "gone.py" }, "3".repeat(64));
  const otherConfig = answerPath("e".repeat(64), "B301", { task: "demo", path: "a.py" }, "1".repeat(64));
  const previous = new Map([[stale, answer], [otherConfig, answer]]);
  const written = new Map([[needed.answers[0]!.path, answer]]);
  const kept = nextTree(previous, needed, written, false);
  assert.deepEqual([...kept.keys()].sort(), [needed.answers[0]!.path, stale, `config/${needed.configId}.json`].sort());
  assert.equal(digest(kept.get(`config/${needed.configId}.json`)!.toString("utf8")), needed.configId);
  assert.deepEqual([...nextTree(previous, needed, written, true).keys()].sort(), [needed.answers[0]!.path, `config/${needed.configId}.json`].sort());
});

test("the corpus answers archive is canonical, verified before it is extracted, and refuses paths a case-insensitive file system merges", () => {
  const entries = [...TREE, ["config/x.json", Buffer.from("{}\n", "utf8")] as const];
  const packed = packCorpusTree(new Map(entries));
  // The same files make the same archive, whatever order they were added in.
  assert.equal(packCorpusTree(new Map([...entries].reverse())).sha256, packed.sha256);
  assert.deepEqual(unpackCorpusArchive(packed.compressed, packed, "demo").map((entry) => entry.path), [...entries.map(([path]) => path)].sort());
  assert.throws(() => unpackCorpusArchive(packed.compressed, { ...packed, files: packed.files + 1 }, "demo"), /the lock says/u);
  assert.throws(() => unpackCorpusArchive(packed.compressed, { ...packed, sha256: "0".repeat(64) }, "demo"), /not the locked/u);

  assert.throws(() => checkTreePaths(["answers/c/B301/demo/App.py@k.json", "answers/c/B301/demo/app.py@k.json"]), /differ only by letter case/u);
  assert.throws(() => checkTreePaths(["corpus.json"]), /holds only config\/, answers\/ and verdicts\//u);

  const cacheRoot = mkdtempSync(join(tmpdir(), "port-readiness-archive-"));
  try {
    cacheCorpusArchive(cacheRoot, packed);
    const archive = corpusArchiveCachePath(cacheRoot, packed.sha256);
    const tree = corpusTreeDirectory(cacheRoot, packed.sha256);
    assert.ok(existsSync(archive));
    // The extraction's marker names the tar, and is no file of the tree.
    assert.equal(readFileSync(join(tree, CORPUS_TREE_MARKER), "utf8"), `${packed.sha256}\n`);
    assert.deepEqual([...treeFiles(tree).keys()].sort(), [...entries.map(([path]) => path)].sort());
    // A tree that lost a file is extracted again from the cached archive.
    rmSync(join(tree, "config"), { recursive: true });
    assert.ok(existsSync(join(cachedCorpusTree(cacheRoot, packed, "demo")!, "config/x.json")));
    // A tree of the right counts whose marker names no archive is not reused: one file's bytes were swapped in place.
    writeFileSync(join(tree, "config/x.json"), "[]\n");
    writeFileSync(join(tree, CORPUS_TREE_MARKER), `${"0".repeat(64)}\n`);
    assert.equal(readFileSync(join(cachedCorpusTree(cacheRoot, packed, "demo")!, "config/x.json"), "utf8"), "{}\n");
    // Nor is one whose marker names the archive but whose files were edited, the counts and bytes kept: a finding's
    // coordinate changed, or an answer renamed. Each is extracted again from the verified archive.
    const answer = [...treeFiles(tree).keys()].find((path) => path.startsWith("answers/"))!;
    const original = readFileSync(join(tree, answer));
    const moved = Buffer.from(original.toString("utf8").replace("3", "4"), "utf8");
    assert.equal(moved.length, original.length);
    writeFileSync(join(tree, answer), moved);
    assert.deepEqual(readFileSync(join(cachedCorpusTree(cacheRoot, packed, "demo")!, answer)), original);
    renameSync(join(tree, answer), join(tree, `${answer}.renamed`));
    assert.deepEqual(treeFiles(cachedCorpusTree(cacheRoot, packed, "demo")!).get(answer), original);
    // Another process's tree of the same archive, there first, is accepted as this one.
    extractCorpusTree(tree, unpackCorpusArchive(packed.compressed, packed, "demo"), packed);
    assert.equal(cachedCorpusTree(cacheRoot, packed, "demo"), tree);
    assert.deepEqual(readdirSync(dirname(tree)), [packed.sha256]);
    // A corrupt cached archive is no copy of it: it is deleted, so the caller fetches it again.
    rmSync(tree, { recursive: true });
    writeFileSync(archive, Buffer.from("not zstd"));
    assert.equal(cachedCorpusTree(cacheRoot, packed, "demo"), undefined);
    assert.equal(existsSync(archive), false);
    // With neither the tree nor the archive, there is nothing to read.
    assert.equal(cachedCorpusTree(cacheRoot, { ...packed, sha256: "0".repeat(64) }, "demo"), undefined);
  } finally {
    rmSync(cacheRoot, { recursive: true, force: true });
  }
});
