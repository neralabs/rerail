/**
 * `npm run port-readiness:record`: run a tool-pack's live adapter and write the recording the gates read: recording.json
 * and the corpus answers archive into the cache, the recording.lock naming them, and missing case sources. The corpus
 * answers are recorded incrementally (only those the locked archive lacks): `--corpus` records nothing else, `--full`
 * asks every needed answer again, `--prune` drops answers no golden needs. `--check` asks every needed answer again,
 * writes nothing into the tree and exits 1 when the committed lock is not what a record of them names (checkRecording):
 * `--base <lock>` is the lock the commit's was recorded on (a missing file: the base has none), and `--recorded <dir>`
 * gets, when it fails, the lock and the two recordings a record would commit. A check that holds puts the recordings
 * the committed lock names into the cache, where the gates read them.
 */
import { existsSync, readFileSync } from "node:fs";
import { mkdir, writeFile } from "node:fs/promises";
import { basename, dirname, join, resolve } from "node:path";
import { parseArgs } from "node:util";

import { packFixturesWritten } from "../../../scripts/s3/pack-fixtures.mjs";
import { PACKAGE_ROOT } from "../../../src/engine/index.js";
import { DEFAULT_CACHE_ROOT } from "../../port-readiness/corpus.js";
import { ProcessCommandRunner } from "../../port-readiness/process.js";
import { cacheCorpusArchive, CORPUS_ARCHIVE, corpusArchiveCachePath } from "../../port-readiness/recording/corpus-answers.js";
import { canonicalText, RECORDING_FILE } from "../../port-readiness/recording/schema.js";
import { renderRecordingChanges, type RecordingChanges } from "../../port-readiness/recording/diff.js";
import { parseRecordingLock, RECORDING_LOCK, recordingCachePath, recordingLockText, writeCachedRecording } from "../../port-readiness/recording/store.js";
import { checkRecording, recordToolPack, writeRecording } from "./record.js";

const USAGE = `usage:
  npm run port-readiness:record -- --pack <id> --tool <tool> [--corpus] [--full] [--prune] [--cache dir] [--changes file]
  npm run port-readiness:record -- --pack <id> --tool <tool> --check [--base lock] [--recorded dir] [--cache dir] [--changes file]
`;

const megabytes = (bytes: number): string => `${(bytes / 1024 / 1024).toFixed(1)} MB`;

async function main(): Promise<number> {
  const { values, positionals } = parseArgs({
    options: {
      pack: { type: "string" },
      tool: { type: "string" },
      cache: { type: "string" },
      changes: { type: "string" },
      check: { type: "boolean", default: false },
      base: { type: "string" },
      recorded: { type: "string" },
      corpus: { type: "boolean", default: false },
      full: { type: "boolean", default: false },
      prune: { type: "boolean", default: false },
      help: { type: "boolean", default: false },
    },
    allowPositionals: true,
    strict: true,
  });
  const root = PACKAGE_ROOT;
  const cacheRoot = resolve(values.cache ?? DEFAULT_CACHE_ROOT);
  const checkOnly = values.base !== undefined || values.recorded !== undefined;
  if (values.help === true || positionals.length > 0 || values.pack === undefined || values.tool === undefined || (values.check && (values.corpus || values.prune)) || (checkOnly && !values.check)) {
    process.stderr.write(USAGE);
    return values.help === true ? 0 : 2;
  }
  const { pack, tool } = values;
  const record = `npm run port-readiness:record -- --pack ${pack} --tool ${tool}`;
  // Read before upstream runs, so a base lock that cannot be read fails first.
  const base = values.base === undefined ? undefined : existsSync(values.base) ? parseRecordingLock(readFileSync(values.base, "utf8"), values.base, record) : null;
  const started = Date.now();
  // --check asks every needed answer again, so the locked ones are compared with a fresh upstream's.
  const recorded = await recordToolPack({
    root,
    pack,
    tool,
    cacheRoot,
    runner: new ProcessCommandRunner(),
    corpus: { only: values.corpus, full: values.full || values.check, prune: values.prune },
  });
  const label = `packs/${pack}/upstream/${tool}`;
  const cases = Object.values(recorded.recording.rules).flatMap((rule) => rule.cases);
  const { corpus } = recorded;
  const summary = `${Object.keys(recorded.recording.rules).length} rules (${recorded.recording.absent.length} absent upstream), ${cases.length} cases `
    + `(${cases.filter((testCase) => testCase.skipped === "not-copied").length} not copied), ${recorded.sources.length} case sources, `
    + `${corpus.needed.answers.length} corpus answers and ${corpus.needed.verdicts.length} verdicts needed (${corpus.asked.answers} and ${corpus.asked.verdicts} asked of upstream), ${((Date.now() - started) / 1000).toFixed(0)} s`;

  // The per-rule summary replaces the recording's own diff, which is no longer in the tree to read.
  const reportChanges = async (changes: RecordingChanges): Promise<void> => {
    if (values.changes !== undefined) {
      await mkdir(dirname(resolve(values.changes)), { recursive: true });
      await writeFile(resolve(values.changes), renderRecordingChanges([changes]));
    }
    process.stdout.write(changes.first
      ? `  first recording: nothing to compare upstream's answers with\n`
      : changes.unavailable !== null
      ? `  not compared: the recording ${values.base === undefined ? RECORDING_LOCK : "the base's lock"} named could not be read (${changes.unavailable}), so what moved is unknown\n`
      : `  ${changes.rules.length} upstream rule(s) answer differently${changes.pinsChanged.length === 0 ? "" : `, pins moved (${changes.pinsChanged.join("; ")})`}\n`);
  };

  if (values.check) {
    const check = checkRecording(root, pack, tool, recorded, { cacheRoot, base });
    if (check.corpusNote !== null) process.stdout.write(`${label}: ${check.corpusNote}\n`);
    const problems = [
      ...check.changed.map((file) => `${label}/${file} differs from a fresh recording`),
      ...check.missing.slice(0, 10).map((copy) => `${label}/${copy.file} is missing`),
      ...(check.missing.length > 10 ? [`... and ${check.missing.length - 10} more missing case sources`] : []),
      ...check.conflicting.map((file) => `${label}/${file} differs from upstream's copy`),
    ];
    const text = canonicalText(recorded.recording);
    if (problems.length === 0) {
      // The committed lock names the fresh recording.json, and an archive the cache now holds or a record of the fresh
      // answers made: the gates read both from the cache.
      await writeCachedRecording(cacheRoot, text);
      if (check.committed !== null) cacheCorpusArchive(cacheRoot, check.committed);
      process.stdout.write(`${label}: the recording is current (${summary})\n`);
      await reportChanges(check.changes);
      return 0;
    }
    if (values.recorded !== undefined) {
      const directory = resolve(values.recorded);
      await mkdir(directory, { recursive: true });
      await writeFile(join(directory, RECORDING_LOCK), recordingLockText(check.lock));
      await writeFile(join(directory, basename(recordingCachePath(cacheRoot, check.lock.files[RECORDING_FILE].sha256))), text);
      await writeFile(join(directory, basename(corpusArchiveCachePath(cacheRoot, check.packed.sha256))), check.packed.compressed);
      process.stdout.write(`${label}: the lock and the two recordings a record makes are in ${directory}\n`);
    }
    // An answer the locked archive holds is never asked again by an incremental record, so one upstream now gives
    // differently (a transitive package resolved at install time moved) is fixed only by asking every answer again;
    // --corpus alone when recording.json and the case sources are current.
    const corpusOnly = !check.changed.includes(RECORDING_FILE) && check.missing.length === 0;
    const flags = !check.changed.includes(CORPUS_ARCHIVE) ? "" : corpusOnly ? " --corpus --full" : " --full";
    process.stdout.write(`${label}: the recording is stale; run ${record}${flags}\n${problems.map((problem) => `  ${problem}`).join("\n")}\n`);
    await reportChanges(check.changes);
    return 1;
  }
  const written = await writeRecording(root, pack, tool, recorded, { cacheRoot });
  process.stdout.write(`${label}: recorded ${summary}; copied ${written.missing.length} case sources\n`);
  // New case sources are pack test data, which git ignores: a contributor adds them to the pull request by hand.
  if (written.missing.length > 0) process.stdout.write(`${packFixturesWritten(written.missing.map((copy) => join(root, label, copy.file)))}\n`);
  const recording = written.lock.files[RECORDING_FILE];
  const archive = written.lock.files[CORPUS_ARCHIVE];
  process.stdout.write(`  ${recordingCachePath(cacheRoot, recording.sha256)} (${megabytes(recording.size)})\n`);
  process.stdout.write(`  ${corpusArchiveCachePath(cacheRoot, archive.sha256)} (${archive.files} files, ${megabytes(archive.bytes)} uncompressed)\n`);
  process.stdout.write(`  ${RECORDING_LOCK} names them\n`);
  await reportChanges(written.changes);
  return 0;
}

main().then((code) => {
  process.exitCode = code;
}, (error: unknown) => {
  process.stderr.write(`${error instanceof Error ? error.stack ?? error.message : String(error)}\n`);
  process.exitCode = 2;
});
