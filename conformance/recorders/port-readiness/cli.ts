/**
 * `npm run port-readiness:record`: run a tool-pack's live adapter and write the recording the gates read: recording.json
 * and the corpus answers archive into the cache, the recording.lock naming them, and missing case sources. The corpus
 * answers are recorded incrementally (only those the locked archive lacks): `--corpus` records nothing else, `--full`
 * asks every needed answer again, `--prune` drops answers no golden needs. `--check` writes nothing and exits 1 when a
 * fresh recording differs from what the committed lock names. `--migrate-corpus` rewrites v2 locks to v3 from their
 * corpus.json, without running upstream.
 */
import { existsSync } from "node:fs";
import { mkdir, writeFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { parseArgs } from "node:util";

import { packFixturesWritten, selectToolPacks } from "../../../scripts/s3/pack-fixtures.mjs";
import { PACKAGE_ROOT } from "../../../src/engine/index.js";
import { DEFAULT_CACHE_ROOT } from "../../port-readiness/corpus.js";
import { ProcessCommandRunner } from "../../port-readiness/process.js";
import { CORPUS_ARCHIVE, corpusArchiveCachePath } from "../../port-readiness/recording/corpus-answers.js";
import { RECORDING_FILE } from "../../port-readiness/recording/schema.js";
import { renderRecordingChanges } from "../../port-readiness/recording/diff.js";
import { RECORDING_LOCK, recordingCachePath } from "../../port-readiness/recording/store.js";
import { migrateCorpus } from "./migrate-corpus.js";
import { diffRecording, recordToolPack, writeRecording } from "./record.js";

const USAGE = `usage:
  npm run port-readiness:record -- --pack <id> --tool <tool> [--corpus] [--full] [--prune] [--cache dir] [--changes file] [--check]
  npm run port-readiness:record -- --migrate-corpus [--pack <id> [--tool <tool>]] [--cache dir]
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
      corpus: { type: "boolean", default: false },
      full: { type: "boolean", default: false },
      prune: { type: "boolean", default: false },
      "migrate-corpus": { type: "boolean", default: false },
      help: { type: "boolean", default: false },
    },
    allowPositionals: true,
    strict: true,
  });
  const root = PACKAGE_ROOT;
  const cacheRoot = resolve(values.cache ?? DEFAULT_CACHE_ROOT);
  if (values["migrate-corpus"]) {
    if (positionals.length > 0 || (values.tool !== undefined && values.pack === undefined)) {
      process.stderr.write(USAGE);
      return 2;
    }
    const runner = new ProcessCommandRunner();
    for (const { pack, tool } of selectToolPacks(root, values.pack, values.tool)) {
      if (!existsSync(join(root, "packs", pack, "upstream", tool, RECORDING_LOCK))) continue;
      const migrated = await migrateCorpus(root, cacheRoot, pack, tool, runner);
      if (migrated === null) {
        process.stdout.write(`packs/${pack}/upstream/${tool}: ${RECORDING_LOCK} is v3 already\n`);
        continue;
      }
      process.stdout.write(`packs/${pack}/upstream/${tool}: ${migrated.answers} answer(s), ${migrated.verdicts} verdict(s); corpus.tar.zst sha256 ${migrated.packed.sha256}, ${migrated.packed.files} file(s), ${migrated.packed.bytes} byte(s), ${megabytes(migrated.packed.compressed.length)} compressed\n`);
    }
    return 0;
  }
  if (values.help === true || positionals.length > 0 || values.pack === undefined || values.tool === undefined || (values.check && (values.corpus || values.prune))) {
    process.stderr.write(USAGE);
    return values.help === true ? 0 : 2;
  }
  const started = Date.now();
  // --check asks every needed answer again, so the locked ones are compared with a fresh upstream's.
  const recorded = await recordToolPack({
    root,
    pack: values.pack,
    tool: values.tool,
    cacheRoot,
    runner: new ProcessCommandRunner(),
    corpus: { only: values.corpus, full: values.full || values.check, prune: values.prune },
  });
  const label = `packs/${values.pack}/upstream/${values.tool}`;
  const cases = Object.values(recorded.recording.rules).flatMap((rule) => rule.cases);
  const { corpus } = recorded;
  const summary = `${Object.keys(recorded.recording.rules).length} rules (${recorded.recording.absent.length} absent upstream), ${cases.length} cases `
    + `(${cases.filter((testCase) => testCase.skipped === "not-copied").length} not copied), ${recorded.sources.length} case sources, `
    + `${corpus.needed.answers.length} corpus answers and ${corpus.needed.verdicts.length} verdicts needed (${corpus.asked.answers} and ${corpus.asked.verdicts} asked of upstream), ${((Date.now() - started) / 1000).toFixed(0)} s`;
  if (values.check) {
    const diff = diffRecording(root, values.pack, values.tool, recorded, cacheRoot);
    if (diff.corpusNote !== null) process.stdout.write(`${label}: ${diff.corpusNote}\n`);
    const problems = [
      ...diff.changed.map((file) => `${label}/${file} differs from a fresh recording`),
      ...diff.missing.slice(0, 10).map((copy) => `${label}/${copy.file} is missing`),
      ...(diff.missing.length > 10 ? [`... and ${diff.missing.length - 10} more missing case sources`] : []),
      ...diff.conflicting.map((file) => `${label}/${file} differs from upstream's copy`),
    ];
    if (problems.length === 0) {
      process.stdout.write(`${label}: the recording is current (${summary})\n`);
      return 0;
    }
    // An answer the locked archive holds is never asked again by an incremental record, so one upstream now gives
    // differently (a transitive package resolved at install time moved) is fixed only by asking every answer again;
    // --corpus alone when recording.json and the case sources are current.
    const corpusOnly = !diff.changed.includes(RECORDING_FILE) && diff.missing.length === 0;
    const flags = !diff.changed.includes(CORPUS_ARCHIVE) ? "" : corpusOnly ? " --corpus --full" : " --full";
    process.stdout.write(`${label}: the recording is stale; run npm run port-readiness:record -- --pack ${values.pack} --tool ${values.tool}${flags}\n${problems.map((problem) => `  ${problem}`).join("\n")}\n`);
    return 1;
  }
  const written = await writeRecording(root, values.pack, values.tool, recorded, { cacheRoot });
  process.stdout.write(`${label}: recorded ${summary}; copied ${written.missing.length} case sources\n`);
  // New case sources are pack test data, which git ignores: a contributor adds them to the pull request by hand.
  if (written.missing.length > 0) process.stdout.write(`${packFixturesWritten(written.missing.map((copy) => join(root, label, copy.file)))}\n`);
  const recording = written.lock.files["recording.json"];
  const archive = written.lock.files["corpus.tar.zst"];
  process.stdout.write(`  ${recordingCachePath(cacheRoot, recording.sha256)} (${megabytes(recording.size)})\n`);
  process.stdout.write(`  ${corpusArchiveCachePath(cacheRoot, archive.sha256)} (${archive.files} files, ${megabytes(archive.bytes)} uncompressed)\n`);
  process.stdout.write(`  ${RECORDING_LOCK} names them\n`);

  // The per-rule summary replaces the recording's own diff, which is no longer in the tree to read.
  const changes = [written.changes];
  if (values.changes !== undefined) {
    await mkdir(dirname(resolve(values.changes)), { recursive: true });
    await writeFile(resolve(values.changes), renderRecordingChanges(changes));
  }
  process.stdout.write(written.changes.first
    ? `  first recording: nothing to compare upstream's answers with\n`
    : written.changes.unavailable !== null
    ? `  not compared: the recording ${RECORDING_LOCK} named could not be read (${written.changes.unavailable}), so what moved is unknown\n`
    : `  ${written.changes.rules.length} upstream rule(s) answer differently${written.changes.pinsChanged.length === 0 ? "" : `, pins moved (${written.changes.pinsChanged.join("; ")})`}\n`);
  return 0;
}

main().then((code) => {
  process.exitCode = code;
}, (error: unknown) => {
  process.stderr.write(`${error instanceof Error ? error.stack ?? error.message : String(error)}\n`);
  process.exitCode = 2;
});
