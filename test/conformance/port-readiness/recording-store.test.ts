/**
 * The recordings outside git: the committed `recording.lock`, the cache they are read from, and what happens when the
 * bytes are not the ones the lock names. That last one is the whole point of the manifest — a pull request that writes
 * its own recording must fail, because the gates take only the bytes the lock names, which CI records from the commit
 * itself on a cache miss, and cannot tell a true recording from a flattering one by anything but its digest.
 */
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, mkdtempSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "vitest";

import { PACKAGE_ROOT } from "../../../src/engine/index.js";
import { diffRecordings, renderRecordingChanges, type RecordedPair } from "../../../conformance/port-readiness/recording/diff.js";
import {
  buildRecordingLock, readCachedRecording, readRecordingLock, recordingCachePath, recordingLockSchema, recordingLockText, writeCachedRecording, RECORDING_LOCK,
  type RecordingLock,
} from "../../../conformance/port-readiness/recording/store.js";
import { readGatesManifest, recordedRuntimes } from "../../../conformance/port-readiness/manifest.js";
import { gatesCorpusSchema, gatesRecordingSchema, CORPUS_FORMAT, RECORDING_FORMAT, recordingText } from "../../../conformance/port-readiness/recording/schema.js";

const digest = (text: string): string => createHash("sha256").update(text).digest("hex");

const PARSERS = [{ parser: "cpython 3.14.8", grammar: true }];
const PINS = { repository: "https://github.com/PyCQA/bandit", commit: "a".repeat(40), version: "1.9.4", runtime: "bandit-1.9.4" };

function recordingOf(rules: Record<string, unknown>): ReturnType<typeof gatesRecordingSchema.parse> {
  return gatesRecordingSchema.parse({
    format: RECORDING_FORMAT,
    tool: "bandit",
    adapter: "bandit",
    pins: PINS,
    runtimes: { "bandit-1.9.4": { "pyproject.toml": "d".repeat(64) } },
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

function corpusOf(findings: Record<string, [number, number | null, number | null, number | null][]>): ReturnType<typeof gatesCorpusSchema.parse> {
  return gatesCorpusSchema.parse({
    format: CORPUS_FORMAT,
    tool: "bandit",
    pins: { tasks: "c".repeat(64), corpora: [] },
    rules: Object.keys(findings).sort(),
    parsers: PARSERS,
    files: [{ task: "demo", path: "app.py", sha256: "b".repeat(64), excluded: null, findings }],
  });
}

/** A tool-pack directory with a committed lock for the two recordings, and nothing else. */
function toolPack(texts: { "recording.json": string; "corpus.json": string }) {
  const root = mkdtempSync(join(tmpdir(), "port-readiness-store-"));
  mkdirSync(join(root, "packs/demo/upstream/bandit"), { recursive: true });
  const lock = buildRecordingLock("bandit", PINS, { "bandit-1.9.4": { "pyproject.toml": "d".repeat(64) } }, texts);
  writeFileSync(join(root, "packs/demo/upstream/bandit", RECORDING_LOCK), recordingLockText(lock));
  return { root, lock, cacheRoot: join(root, "cache"), cleanup: () => rmSync(root, { recursive: true, force: true }) };
}

const TEXTS = { "recording.json": recordingText(recordingOf({ B301: rule([[1, 0, 1, 17]]) })), "corpus.json": recordingText(corpusOf({ B301: [[3, 0, 3, 9]] })) };

test("a lock names each recording by its own digest", () => {
  const { lock, cleanup } = toolPack(TEXTS);
  try {
    assert.equal(lock.files["recording.json"].sha256, digest(TEXTS["recording.json"]));
    assert.equal(lock.files["recording.json"].format, RECORDING_FORMAT);
    assert.equal(lock.files["corpus.json"].format, CORPUS_FORMAT);
    // The lock is small enough to review as data, which is the point of taking 71 MB of JSON out of the tree.
    assert.ok(recordingLockText(lock).length < 2000, `${recordingLockText(lock).length} bytes`);
    // It parses back, so what the recorder writes is what the gates read.
    assert.deepEqual(recordingLockSchema.parse(JSON.parse(recordingLockText(lock))), lock);
  } finally {
    cleanup();
  }
});

test("a lock still naming a release and its assets is refused: the recordings come from the cache or the recorder", () => {
  const { root, lock, cleanup } = toolPack(TEXTS);
  try {
    const v1 = { ...lock, format: "rerail.port-readiness-recording-lock/v1", release: "port-readiness-recordings" };
    writeFileSync(join(root, "packs/demo/upstream/bandit", RECORDING_LOCK), recordingLockText(v1 as RecordingLock));
    assert.throws(() => readRecordingLock(root, "demo", "bandit"), /recording\.lock: \/format: .*\/: Unrecognized key.*release/su);
  } finally {
    cleanup();
  }
});

test("a flattering recording cached under the digest the lock names is never read", async () => {
  const { cacheRoot, lock, cleanup } = toolPack(TEXTS);
  try {
    // What a pull request would want for a rule that finds nothing, put where the lock's recording would be: the cache
    // key names the digest, so restoring it is all a poisoned cache entry could do.
    const flattering = recordingText(recordingOf({ B301: rule([]) }));
    const path = recordingCachePath(cacheRoot, lock.files["recording.json"].sha256);
    mkdirSync(join(cacheRoot, "recordings"), { recursive: true });
    writeFileSync(path, flattering);
    assert.equal(readCachedRecording(cacheRoot, lock.files["recording.json"].sha256), undefined);
    // The bytes the lock does name read back.
    await writeCachedRecording(cacheRoot, TEXTS["recording.json"]);
    assert.equal(readCachedRecording(cacheRoot, lock.files["recording.json"].sha256), TEXTS["recording.json"]);
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
  const { cacheRoot } = toolPack(TEXTS);
  const written = await writeCachedRecording(cacheRoot, TEXTS["recording.json"]);
  assert.equal(readCachedRecording(cacheRoot, written), TEXTS["recording.json"]);
  writeFileSync(join(cacheRoot, "recordings", `${written}.json`), "{}\n");
  assert.equal(readCachedRecording(cacheRoot, written), undefined);
  rmSync(cacheRoot, { recursive: true, force: true });
});

test("the per-rule summary names what moved, and says so plainly when nothing did", () => {
  const before: RecordedPair = { recording: recordingOf({ B301: rule([[1, 0, 1, 17]]), B302: rule([[2, 0, 2, 4]]) }), corpus: corpusOf({ B301: [[3, 0, 3, 9]] }) };
  const after: RecordedPair = { recording: recordingOf({ B301: rule([[1, 0, 1, 17], [5, 0, 5, 8]], ["examples/pickle.py", "examples/more.py"]) }), corpus: corpusOf({ B301: [[4, 0, 4, 9]] }) };

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
    { rule: "B301", status: "changed", expectedAdded: 1, expectedRemoved: 0, corpusFilesChanged: 1, factsChanged: ["tests"] },
    { rule: "B302", status: "removed", expectedAdded: 0, expectedRemoved: 0, corpusFilesChanged: 0, factsChanged: [] },
  ]);
  const rendered = renderRecordingChanges([changes]);
  assert.match(rendered, /1 tool-pack moved/u);
  assert.match(rendered, /\| `B301` \| changed \| \+1 \/ −0 \| 1 \| `tests` \|/u);
  assert.match(rendered, /\| `B302` \| removed \| — \| — \| — \|/u);
  // A re-record that answers fewer files (the recorder answers only the files goldens name) is not a change in
  // upstream's answers: only files both recordings answer are compared.
  const whole = corpusOf({ B301: [[3, 0, 3, 9]] });
  const wider: RecordedPair = { ...before, corpus: { ...whole, files: [...whole.files, { ...whole.files[0]!, task: "dropped", findings: { B301: [[1, 0, 1, 9]] } }] } };
  assert.deepEqual(diffRecordings("demo", "bandit", wider, before).rules, []);
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
  const locked: RecordedPair = { recording: gatesRecordingSchema.parse(JSON.parse(recordingText(fresh.recording))), corpus: null };
  // The two hold the same options in different key orders, so the comparison below is not vacuous.
  assert.notEqual(JSON.stringify(locked.recording.rules.B602!.facts.recommended), JSON.stringify(fresh.recording.rules.B602!.facts.recommended));

  // Run 39319650531 listed `recommended` as changed for nine Bandit rules whose recordings were identical.
  assert.deepEqual(diffRecordings("demo", "bandit", locked, fresh).rules, []);
  assert.deepEqual(diffRecordings("demo", "bandit", fresh, locked).rules, []);
  assert.match(renderRecordingChanges([diffRecordings("demo", "bandit", locked, fresh)]), /`demo\/bandit`: unchanged/u);

  // A value that did change still does, and a list is a value whose order counts: only object keys are unordered.
  const changed = [{ rule: "B602", status: "changed", expectedAdded: 0, expectedRemoved: 0, corpusFilesChanged: 0, factsChanged: ["recommended"] }];
  assert.deepEqual(diffRecordings("demo", "bandit", locked, recorded(["os.system", "os.popen"])).rules, changed);
  assert.deepEqual(diffRecordings("demo", "bandit", recorded(["os.popen", "os.system"]), recorded(["os.system", "os.popen"])).rules, changed);
});

test("the per-rule summary calls a fact the older recording lacks changed, instead of failing the record", () => {
  // The schema requires every fact today, but a field added later would be optional for the recordings made before it,
  // so a re-record would compare a fresh recording that has it with a locked one that does not.
  const fresh: RecordedPair = { recording: recordingOf({ B301: rule([[1, 0, 1, 17]]) }), corpus: null };
  const older = gatesRecordingSchema.parse(JSON.parse(recordingText(fresh.recording)));
  delete (older.rules.B301!.facts as Record<string, unknown>).notOnRealCode;

  assert.deepEqual(diffRecordings("demo", "bandit", { recording: older, corpus: null }, fresh).rules, [
    { rule: "B301", status: "changed", expectedAdded: 0, expectedRemoved: 0, corpusFilesChanged: 0, factsChanged: ["notOnRealCode"] },
  ]);
});
