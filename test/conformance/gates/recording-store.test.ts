/**
 * The recordings outside git: the committed `recording.lock`, the cache they are fetched into, and what happens when
 * the bytes are not the ones the lock names. That last one is the whole point of the manifest — a pull request that
 * writes its own recording must fail, because pull request CI never runs upstream and cannot tell a true recording
 * from a flattering one.
 */
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, mkdtempSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "vitest";

import { PACKAGE_ROOT } from "../../../src/engine/index.js";
import { fetchToolPackRecordings, LIFTED_ASSETS } from "../../../conformance/gates/recording/fetch.js";
import { diffRecordings, renderRecordingChanges, type RecordedPair } from "../../../conformance/gates/recording/diff.js";
import {
  assetName, buildRecordingLock, gzipRecording, LIFTED_CASE_SOURCES, readCachedRecording, readRecordingLock, recordingFromAsset, recordingLockSchema,
  recordingLockText, writeCachedRecording, RECORDING_LOCK,
} from "../../../conformance/gates/recording/store.js";
import { gatesCorpusSchema, gatesRecordingSchema, GATES_CORPUS_FORMAT, GATES_RECORDING_FORMAT, recordingText } from "../../../conformance/gates/recording/schema.js";

const digest = (text: string): string => createHash("sha256").update(text).digest("hex");

const PARSERS = [{ parser: "cpython 3.14.8", grammar: true }];
const PINS = { repository: "https://github.com/PyCQA/bandit", commit: "a".repeat(40), version: "1.9.4", runtime: "bandit-1.9.4" };

function recordingOf(rules: Record<string, unknown>): ReturnType<typeof gatesRecordingSchema.parse> {
  return gatesRecordingSchema.parse({
    format: GATES_RECORDING_FORMAT,
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

const rule = (expected: [number, number | null, number | null, number | null][], tests: string[] = ["examples/pickle.py"]) => ({
  facts: { name: "B301", implementation: ["bandit/blacklists/calls.py"], tests, deprecation: null, recommended: null, notOnRealCode: null },
  names: { implementation: { sets: [], names: [], regexes: [] }, shared: { sets: [], names: [], regexes: [] } },
  spanParts: { endLine: true, endCol: true },
  cases: [{ fixture: "examples/pickle.py", variant: "default", path: "pickle.py", skipped: null, expected, markup: null, file: "fixtures/pickle.py" }],
});

function corpusOf(findings: Record<string, [number, number | null, number | null, number | null][]>): ReturnType<typeof gatesCorpusSchema.parse> {
  return gatesCorpusSchema.parse({
    format: GATES_CORPUS_FORMAT,
    tool: "bandit",
    pins: { tasks: "c".repeat(64), corpora: [] },
    rules: Object.keys(findings).sort(),
    parsers: PARSERS,
    files: [{ task: "demo", path: "app.py", sha256: "b".repeat(64), excluded: null, findings }],
  });
}

/** A tool-pack directory with a committed lock for the two recordings, and nothing else. */
function toolPack(texts: { "gates.json": string; "gates-corpus.json": string }) {
  const root = mkdtempSync(join(tmpdir(), "gates-store-"));
  mkdirSync(join(root, "packs/demo/upstream/bandit"), { recursive: true });
  writeFileSync(join(root, "package.json"), JSON.stringify({ version: "9.9.9" }));
  const lock = buildRecordingLock("demo", "bandit", "gate-recordings", "9.9.9", PINS, { "bandit-1.9.4": { "pyproject.toml": "d".repeat(64) } }, texts);
  writeFileSync(join(root, "packs/demo/upstream/bandit", RECORDING_LOCK), recordingLockText(lock));
  return { root, lock, cacheRoot: join(root, "cache"), cleanup: () => rmSync(root, { recursive: true, force: true }) };
}

const TEXTS = { "gates.json": recordingText(recordingOf({ B301: rule([[1, 0, 1, 17]]) })), "gates-corpus.json": recordingText(corpusOf({ B301: [[3, 0, 3, 9]] })) };

test("a lock names each recording by its own digest, under a content-addressed asset name", () => {
  const { lock, cleanup } = toolPack(TEXTS);
  try {
    assert.equal(lock.files["gates.json"].sha256, digest(TEXTS["gates.json"]));
    assert.equal(lock.files["gates.json"].asset, assetName("demo", "bandit", "gates.json", digest(TEXTS["gates.json"])));
    assert.equal(lock.files["gates.json"].format, GATES_RECORDING_FORMAT);
    assert.equal(lock.files["gates-corpus.json"].format, GATES_CORPUS_FORMAT);
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
      return gzipRecording(asset.asset === lock.files["gates.json"].asset ? TEXTS["gates.json"] : TEXTS["gates-corpus.json"]);
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
    assert.deepEqual(verified, [lock.files["gates.json"].sha256, lock.files["gates-corpus.json"].sha256]);
    assert.equal(readCachedRecording(cacheRoot, lock.files["gates.json"].sha256), TEXTS["gates.json"]);

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
    assert.equal(recordingFromAsset(gzipRecording(TEXTS["gates.json"]), lock.files["gates.json"], "demo/bandit"), TEXTS["gates.json"]);
  } finally {
    cleanup();
  }
});

test("a hand-edited manifest naming a hash the release does not hold fails, and says a pull request cannot supply its own", async () => {
  const { root, cacheRoot, lock, cleanup } = toolPack(TEXTS);
  try {
    // What a fork pull request would commit: a lock pointing at a recording nobody uploaded.
    const forged = { ...lock, files: { ...lock.files, "gates.json": { ...lock.files["gates.json"], sha256: "f".repeat(64), asset: assetName("demo", "bandit", "gates.json", "f".repeat(64)) } } };
    writeFileSync(join(root, "packs/demo/upstream/bandit", RECORDING_LOCK), recordingLockText(forged));
    const release = new Set([lock.files["gates.json"].asset, lock.files["gates-corpus.json"].asset]);
    const download = async (asset: { asset: string }) => {
      if (!release.has(asset.asset)) throw new Error(`the release gate-recordings of neralabs/rerail has no asset ${asset.asset}: the recording this commit names was never uploaded (a pull request cannot supply its own; see rule-gates-record.yml)`);
      return gzipRecording(TEXTS["gates.json"]);
    };
    await assert.rejects(fetchToolPackRecordings(root, cacheRoot, "demo", "bandit", { download }), /has no asset demo-bandit-gates-f{64}\.json\.gz: the recording this commit names was never uploaded/u);
  } finally {
    cleanup();
  }
});

test("an asset with the right bytes that the record workflow did not attest fails the fetch, and is not cached", async () => {
  const { root, cacheRoot, lock, cleanup } = toolPack(TEXTS);
  try {
    // What anyone who can write here could upload by hand: the digest the lock names, and no attestation.
    const download = async (asset: { asset: string }) => gzipRecording(asset.asset === lock.files["gates.json"].asset ? TEXTS["gates.json"] : TEXTS["gates-corpus.json"]);
    const verify = async () => {
      throw new Error("no attestation from rule-gates-record.yml");
    };
    await assert.rejects(fetchToolPackRecordings(root, cacheRoot, "demo", "bandit", { download, verify }), /no attestation from rule-gates-record\.yml/u);
    assert.equal(existsSync(join(cacheRoot, "recordings")), false);
  } finally {
    cleanup();
  }
});

test("an asset uploaded by hand when the recordings were lifted is taken without an attestation, and only that one", async () => {
  const { root, cacheRoot, lock, cleanup } = toolPack(TEXTS);
  try {
    const download = async (asset: { asset: string }) => gzipRecording(asset.asset === lock.files["gates.json"].asset ? TEXTS["gates.json"] : TEXTS["gates-corpus.json"]);
    const verified: string[] = [];
    const verify = async (_text: string, asset: { asset: string }) => {
      verified.push(asset.asset);
      throw new Error("no attestation from rule-gates-record.yml");
    };
    // Only the recording is lifted: it is cached unverified, and the corpus beside it still has to be attested.
    const lifted = new Set([lock.files["gates.json"].asset]);
    await assert.rejects(fetchToolPackRecordings(root, cacheRoot, "demo", "bandit", { download, verify, lifted }), /no attestation from rule-gates-record\.yml/u);
    assert.deepEqual(verified, [lock.files["gates-corpus.json"].asset]);
    assert.equal(readCachedRecording(cacheRoot, lock.files["gates.json"].sha256), TEXTS["gates.json"]);
    assert.equal(readCachedRecording(cacheRoot, lock.files["gates-corpus.json"].sha256), undefined);
  } finally {
    cleanup();
  }
});

test("a lock that names an asset other than its own tool-pack's recording of that digest is refused before any download", async () => {
  const { root, cacheRoot, lock, cleanup } = toolPack(TEXTS);
  try {
    // The same bytes under another tool-pack's name: the digest matches, so only the name gives it away.
    const misnamed = { ...lock, files: { ...lock.files, "gates.json": { ...lock.files["gates.json"], asset: assetName("demo", "ruff", "gates.json", lock.files["gates.json"].sha256) } } };
    writeFileSync(join(root, "packs/demo/upstream/bandit", RECORDING_LOCK), recordingLockText(misnamed));
    let downloads = 0;
    const download = async () => {
      downloads += 1;
      return gzipRecording(TEXTS["gates.json"]);
    };
    await assert.rejects(
      fetchToolPackRecordings(root, cacheRoot, "demo", "bandit", { download, verify: async () => undefined }),
      /\/files\/gates\.json\/asset is demo-ruff-gates-[0-9a-f]{64}\.json\.gz, and this tool-pack's recording of sha256 [0-9a-f]{12} is demo-bandit-gates-[0-9a-f]{64}\.json\.gz/u,
    );
    assert.equal(downloads, 0);
  } finally {
    cleanup();
  }
});

test("every asset taken without an attestation is one a committed lock still names", () => {
  // The hand-uploaded assets the recordings were lifted into: an entry whose tool-pack has been recorded since must go.
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

test("every lifted recording whose case sources are pinned together is a lifted asset, so its pin goes with it", () => {
  assert.deepEqual([...LIFTED_CASE_SOURCES.keys()].filter((asset) => !LIFTED_ASSETS.has(asset) || asset.includes("-gates-corpus-")), []);
});

test("a recording whose cached bytes were tampered with reads as absent, not as a recording", async () => {
  const { cacheRoot } = toolPack(TEXTS);
  const written = await writeCachedRecording(cacheRoot, TEXTS["gates.json"]);
  assert.equal(readCachedRecording(cacheRoot, written), TEXTS["gates.json"]);
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
  // A first recording has nothing to compare with, and says that rather than listing every rule as a change.
  assert.match(renderRecordingChanges([diffRecordings("demo", "bandit", null, after)]), /first recording/u);
  // A lock whose recording could not be read is neither: the summary warns that what moved is unknown.
  const uncompared = renderRecordingChanges([diffRecordings("demo", "bandit", { unavailable: "demo-bandit-gates.json.gz is not in the cache" }, after)]);
  assert.match(uncompared, /^## Gate recordings: ⚠️ 1 tool-pack not compared$/mu);
  assert.match(uncompared, /`demo\/bandit`: ⚠️ not compared\n\n.*\(demo-bandit-gates\.json\.gz is not in the cache\)/u);
  assert.doesNotMatch(uncompared, /: first recording|✅/u);
});
