/**
 * The recordings outside git: the committed `recording.lock`, the cache they are fetched into, and what happens when
 * the bytes are not the ones the lock names. That last one is the whole point of the manifest — a pull request that
 * writes its own recording must fail, because the gates take only an attested recording or one CI records from the
 * commit itself, and cannot tell a true recording from a flattering one by its bytes.
 */
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, mkdtempSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, test, vi } from "vitest";

import { PACKAGE_ROOT } from "../../../src/engine/index.js";
import { fetchToolPackRecordings, LIFTED_ASSETS, MissingRecordingError } from "../../../conformance/port-readiness/recording/fetch.js";
import { diffRecordings, renderRecordingChanges, type RecordedPair } from "../../../conformance/port-readiness/recording/diff.js";
import {
  assetName, buildRecordingLock, gzipRecording, LIFTED_CASE_SOURCES, LIFTED_RUNTIMES, readCachedRecording, readRecordingLock, recordingFromAsset,
  recordingLockSchema, recordingLockText, writeCachedRecording, RECORDING_LOCK, type RecordingLock,
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
  const lock = buildRecordingLock("demo", "bandit", "port-readiness-recordings", PINS, { "bandit-1.9.4": { "pyproject.toml": "d".repeat(64) } }, texts);
  writeFileSync(join(root, "packs/demo/upstream/bandit", RECORDING_LOCK), recordingLockText(lock));
  return { root, lock, cacheRoot: join(root, "cache"), cleanup: () => rmSync(root, { recursive: true, force: true }) };
}

const TEXTS = { "recording.json": recordingText(recordingOf({ B301: rule([[1, 0, 1, 17]]) })), "corpus.json": recordingText(corpusOf({ B301: [[3, 0, 3, 9]] })) };

test("a lock names each recording by its own digest, under a content-addressed asset name", () => {
  const { lock, cleanup } = toolPack(TEXTS);
  try {
    assert.equal(lock.files["recording.json"].sha256, digest(TEXTS["recording.json"]));
    assert.equal(lock.files["recording.json"].asset, assetName("demo", "bandit", "recording.json", digest(TEXTS["recording.json"])));
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

test("a fetch caches by digest, and a second fetch on the same commit downloads nothing", async () => {
  const { root, cacheRoot, lock, cleanup } = toolPack(TEXTS);
  try {
    let downloads = 0;
    const download = async (asset: { asset: string }) => {
      downloads += 1;
      return gzipRecording(asset.asset === lock.files["recording.json"].asset ? TEXTS["recording.json"] : TEXTS["corpus.json"]);
    };
    // Every downloaded recording's attestation is checked, on the text whose sha256 the lock names.
    const verified: string[] = [];
    const verify = async (text: string, asset: { sha256: string }) => {
      assert.equal(digest(text), asset.sha256);
      verified.push(asset.sha256);
    };
    const first = await fetchToolPackRecordings(root, cacheRoot, "demo", "bandit", { download, verify });
    assert.deepEqual(first.map((entry) => entry.cached), [false, false]);
    assert.equal(downloads, 2);
    assert.deepEqual(verified, [lock.files["recording.json"].sha256, lock.files["corpus.json"].sha256]);
    assert.equal(readCachedRecording(cacheRoot, lock.files["recording.json"].sha256), TEXTS["recording.json"]);

    const second = await fetchToolPackRecordings(root, cacheRoot, "demo", "bandit", { download, verify });
    assert.deepEqual(second.map((entry) => entry.cached), [true, true]);
    assert.equal(downloads, 2);
  } finally {
    cleanup();
  }
});

test("an asset whose sha256 is not the one the manifest names fails the fetch, and is not cached", async () => {
  const { root, cacheRoot, lock, cleanup } = toolPack(TEXTS);
  try {
    // A recording that says upstream expects nothing: the shape a pull request would want for a rule that finds nothing.
    const flattering = recordingText(recordingOf({ B301: rule([]) }));
    await assert.rejects(
      fetchToolPackRecordings(root, cacheRoot, "demo", "bandit", { download: async () => gzipRecording(flattering) }),
      /has sha256 [0-9a-f]{12}, and recording\.lock names [0-9a-f]{12}/u,
    );
    assert.equal(existsSync(join(cacheRoot, "recordings")), false);
    // And the bytes the lock does name still read back.
    assert.equal(recordingFromAsset(gzipRecording(TEXTS["recording.json"]), lock.files["recording.json"], "demo/bandit"), TEXTS["recording.json"]);
  } finally {
    cleanup();
  }
});

test("a hand-edited manifest naming a hash the release does not hold fails as a missing recording", async () => {
  const { root, cacheRoot, lock, cleanup } = toolPack(TEXTS);
  try {
    // What a fork pull request would commit: a lock pointing at a recording nobody uploaded.
    const forged = { ...lock, files: { ...lock.files, "recording.json": { ...lock.files["recording.json"], sha256: "f".repeat(64), asset: assetName("demo", "bandit", "recording.json", "f".repeat(64)) } } };
    writeFileSync(join(root, "packs/demo/upstream/bandit", RECORDING_LOCK), recordingLockText(forged));
    const release = new Set([lock.files["recording.json"].asset, lock.files["corpus.json"].asset]);
    const download = async (asset: { asset: string }) => {
      if (!release.has(asset.asset)) throw new MissingRecordingError("port-readiness-recordings", "neralabs/rerail", asset.asset);
      return gzipRecording(TEXTS["recording.json"]);
    };
    // A missing recording is told apart from every other failure: it is the one a gate leg answers by recording.
    await assert.rejects(fetchToolPackRecordings(root, cacheRoot, "demo", "bandit", { download }), (error: unknown) =>
      error instanceof MissingRecordingError && /has no asset demo-bandit-recording-f{64}\.json\.gz: the recording this commit names was never uploaded/u.test(error.message));
  } finally {
    cleanup();
  }
});

afterEach(() => {
  vi.unstubAllGlobals();
});

test("only a release that lacks the asset, or does not exist, is a missing recording; an API error is not", async () => {
  const { root, cacheRoot, cleanup } = toolPack(TEXTS);
  try {
    const answering = (status: number, body: unknown) => vi.stubGlobal("fetch", async () => new Response(JSON.stringify(body), { status }));
    answering(200, { assets: [{ name: "another-asset.json.gz", url: "https://api.github.com/assets/1", size: 1 }] });
    await assert.rejects(fetchToolPackRecordings(root, cacheRoot, "demo", "bandit"), MissingRecordingError);
    answering(404, { message: "Not Found" });
    await assert.rejects(fetchToolPackRecordings(root, cacheRoot, "demo", "bandit"), MissingRecordingError);
    answering(502, { message: "Bad Gateway" });
    await assert.rejects(fetchToolPackRecordings(root, cacheRoot, "demo", "bandit"), (error: unknown) =>
      error instanceof Error && !(error instanceof MissingRecordingError) && /answered 502/u.test(error.message));
  } finally {
    cleanup();
  }
});

test("an asset with the right bytes that the record workflow did not attest fails the fetch, and is not cached", async () => {
  const { root, cacheRoot, lock, cleanup } = toolPack(TEXTS);
  try {
    // What anyone who can write here could upload by hand: the digest the lock names, and no attestation.
    const download = async (asset: { asset: string }) => gzipRecording(asset.asset === lock.files["recording.json"].asset ? TEXTS["recording.json"] : TEXTS["corpus.json"]);
    const verify = async () => {
      throw new Error("no attestation from port-readiness-record.yml");
    };
    await assert.rejects(fetchToolPackRecordings(root, cacheRoot, "demo", "bandit", { download, verify }), /no attestation from port-readiness-record\.yml/u);
    assert.equal(existsSync(join(cacheRoot, "recordings")), false);
  } finally {
    cleanup();
  }
});

test("an asset uploaded by hand before the record workflow ran from main is taken without an attestation, and only that one", async () => {
  const { root, cacheRoot, lock, cleanup } = toolPack(TEXTS);
  try {
    const download = async (asset: { asset: string }) => gzipRecording(asset.asset === lock.files["recording.json"].asset ? TEXTS["recording.json"] : TEXTS["corpus.json"]);
    const verified: string[] = [];
    const verify = async (_text: string, asset: { asset: string }) => {
      verified.push(asset.asset);
      throw new Error("no attestation from port-readiness-record.yml");
    };
    // Only the recording was uploaded by hand: it is cached unverified, and the corpus beside it still has to be attested.
    const lifted = new Set([lock.files["recording.json"].asset]);
    await assert.rejects(fetchToolPackRecordings(root, cacheRoot, "demo", "bandit", { download, verify, lifted }), /no attestation from port-readiness-record\.yml/u);
    assert.deepEqual(verified, [lock.files["corpus.json"].asset]);
    assert.equal(readCachedRecording(cacheRoot, lock.files["recording.json"].sha256), TEXTS["recording.json"]);
    assert.equal(readCachedRecording(cacheRoot, lock.files["corpus.json"].sha256), undefined);
  } finally {
    cleanup();
  }
});

test("a lock that names an asset other than its own tool-pack's recording of that digest is refused before any download", async () => {
  const { root, cacheRoot, lock, cleanup } = toolPack(TEXTS);
  try {
    // The same bytes under another tool-pack's name: the digest matches, so only the name gives it away.
    const misnamed = { ...lock, files: { ...lock.files, "recording.json": { ...lock.files["recording.json"], asset: assetName("demo", "ruff", "recording.json", lock.files["recording.json"].sha256) } } };
    writeFileSync(join(root, "packs/demo/upstream/bandit", RECORDING_LOCK), recordingLockText(misnamed));
    let downloads = 0;
    const download = async () => {
      downloads += 1;
      return gzipRecording(TEXTS["recording.json"]);
    };
    await assert.rejects(
      fetchToolPackRecordings(root, cacheRoot, "demo", "bandit", { download, verify: async () => undefined }),
      /\/files\/recording\.json\/asset is demo-ruff-recording-[0-9a-f]{64}\.json\.gz, and this tool-pack's recording of sha256 [0-9a-f]{12} is demo-bandit-recording-[0-9a-f]{64}\.json\.gz/u,
    );
    assert.equal(downloads, 0);
  } finally {
    cleanup();
  }
});

test("every asset taken without an attestation is one a committed lock still names", () => {
  // The hand-uploaded assets: an entry whose tool-pack has an attested recording since must go.
  const named = new Set<string>();
  for (const pack of readdirSync(join(PACKAGE_ROOT, "packs"))) {
    const upstream = join(PACKAGE_ROOT, "packs", pack, "upstream");
    if (!existsSync(upstream)) continue;
    for (const tool of readdirSync(upstream)) {
      if (!existsSync(join(upstream, tool, RECORDING_LOCK))) continue;
      for (const locked of Object.values(readRecordingLock(PACKAGE_ROOT, pack, tool).files)) named.add(locked.asset);
    }
  }
  assert.deepEqual([...LIFTED_ASSETS].filter((asset) => !named.has(asset)), []);
});

test("every hand-uploaded recording whose case sources are pinned together is a hand-uploaded asset, so its pin goes with it", () => {
  assert.deepEqual([...LIFTED_CASE_SOURCES.keys()].filter((asset) => !LIFTED_ASSETS.has(asset) || /-corpus-[0-9a-f]{64}\.json\.gz$/u.test(asset)), []);
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

test("every committed lock pins exactly the runtimes its gates.yaml installs, a hand-uploaded recording's LIFTED_RUNTIMES included", () => {
  // What pins-agree checks on every run, here without fetching a recording: a gates.yaml that names a runtime its lock
  // doesn't (a new environment, or one moved to another runtime directory) needs the tool-pack recorded again.
  const mismatched = committedLocks().flatMap(({ label, lock, runtimes }) => {
    const pinned = Object.keys({ ...LIFTED_RUNTIMES.get(lock.files["recording.json"].asset), ...lock.runtimes }).sort();
    return pinned.join(", ") === runtimes.join(", ") ? [] : [`${label}: gates.yaml installs ${runtimes.join(", ")}, the lock pins ${pinned.join(", ")}`];
  });
  assert.deepEqual(mismatched, []);
});

test("every LIFTED_RUNTIMES entry is a hand-uploaded recording a committed lock names, and pins only runtimes that lock doesn't", () => {
  const byAsset = new Map(committedLocks().map(({ lock }) => [lock.files["recording.json"].asset, lock]));
  const wrong = [...LIFTED_RUNTIMES].flatMap(([asset, runtimes]) => {
    if (!LIFTED_ASSETS.has(asset) || !/-recording-[0-9a-f]{64}\.json\.gz$/u.test(asset)) return [`${asset} is not a hand-uploaded recording.json`];
    const lock = byAsset.get(asset);
    if (lock === undefined) return [`${asset} is named by no committed lock: its tool-pack was recorded again, so the entry goes`];
    return Object.keys(runtimes).filter((runtime) => runtime in lock.runtimes).map((runtime) => `${asset}: its lock already pins ${runtime}`);
  });
  assert.deepEqual(wrong, []);
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
