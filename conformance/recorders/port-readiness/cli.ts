/**
 * `npm run port-readiness:record`: run a tool-pack's live adapter and write the recording the gates read: recording.json
 * and corpus.json into the cache, the recording.lock naming them, and missing case sources. `--check` writes nothing
 * and exits 1 when a fresh recording differs from what the committed lock names.
 */
import { mkdir, writeFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { parseArgs } from "node:util";

import { packFixturesWritten } from "../../../scripts/s3/pack-fixtures.mjs";
import { PACKAGE_ROOT } from "../../../src/engine/index.js";
import { DEFAULT_CACHE_ROOT } from "../../port-readiness/corpus.js";
import { ProcessCommandRunner } from "../../port-readiness/process.js";
import { renderRecordingChanges } from "../../port-readiness/recording/diff.js";
import { RECORDING_LOCK } from "../../port-readiness/recording/store.js";
import { diffRecording, recordToolPack, writeRecording } from "./record.js";

const USAGE = `usage:
  npm run port-readiness:record -- --pack <id> --tool <tool> [--cache dir] [--changes file] [--check]
`;

async function main(): Promise<number> {
  const { values, positionals } = parseArgs({
    options: {
      pack: { type: "string" },
      tool: { type: "string" },
      cache: { type: "string" },
      changes: { type: "string" },
      check: { type: "boolean", default: false },
      help: { type: "boolean", default: false },
    },
    allowPositionals: true,
    strict: true,
  });
  if (values.help === true || positionals.length > 0 || values.pack === undefined || values.tool === undefined) {
    process.stderr.write(USAGE);
    return values.help === true ? 0 : 2;
  }
  const root = PACKAGE_ROOT;
  const cacheRoot = resolve(values.cache ?? DEFAULT_CACHE_ROOT);
  const started = Date.now();
  const recorded = await recordToolPack({
    root,
    pack: values.pack,
    tool: values.tool,
    cacheRoot,
    runner: new ProcessCommandRunner(),
  });
  const label = `packs/${values.pack}/upstream/${values.tool}`;
  const cases = Object.values(recorded.recording.rules).flatMap((rule) => rule.cases);
  const summary = `${Object.keys(recorded.recording.rules).length} rules (${recorded.recording.absent.length} absent upstream), ${cases.length} cases `
    + `(${cases.filter((testCase) => testCase.skipped === "not-copied").length} not copied), ${recorded.sources.length} case sources, `
    + `${recorded.corpus.files.length} corpus files, ${((Date.now() - started) / 1000).toFixed(0)} s`;
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
    process.stdout.write(`${label}: the recording is stale; run npm run port-readiness:record -- --pack ${values.pack} --tool ${values.tool}\n${problems.map((problem) => `  ${problem}`).join("\n")}\n`);
    return 1;
  }
  const written = await writeRecording(root, values.pack, values.tool, recorded, { cacheRoot });
  process.stdout.write(`${label}: recorded ${summary}; copied ${written.missing.length} case sources\n`);
  // New case sources are pack test data, which git ignores: a contributor adds them to the pull request by hand.
  if (written.missing.length > 0) process.stdout.write(`${packFixturesWritten(written.missing.map((copy) => join(root, label, copy.file)))}\n`);
  for (const file of Object.values(written.lock.files)) process.stdout.write(`  ${cacheRoot}/recordings/${file.sha256}.json (${(file.size / 1024 / 1024).toFixed(1)} MB)\n`);
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
