/**
 * `npm run port-readiness:record`: record a tool-pack's recording archive (upstream's answers on its own tests) into
 * the cache, the recording.lock naming it, and missing case sources. A record asks upstream only for what the locked
 * archive lacks at this branch's config, and nothing when it lacks nothing: `--full` asks everything again, `--prune`
 * drops what no recorded rule names. `--check` asks everything again, writes nothing into the tree and exits 1 when the committed lock is
 * not what a record of it names (checkRecording): `--base <lock>` is the lock the commit's was recorded on (a missing
 * file: the base has none), and `--recorded <dir>` gets, when it fails, the lock and the archive a record would commit.
 * A check that holds puts the archive the committed lock names into the cache, where the gates read it.
 */
import { existsSync, readFileSync } from "node:fs";
import { mkdir, writeFile } from "node:fs/promises";
import { basename, dirname, join, resolve } from "node:path";
import { parseArgs } from "node:util";

import { packFixturesWritten } from "../../../scripts/s3/pack-fixtures.mjs";
import { PACKAGE_ROOT } from "../../../src/engine/index.js";
import { DEFAULT_CACHE_ROOT } from "./corpus.js";
import { ProcessCommandRunner } from "../../process.js";
import { archiveCachePath, cacheArchive, RECORDING_ARCHIVE } from "../../port-readiness/recording/archive.js";
import { renderRecordingChanges, type RecordingChanges } from "../../port-readiness/recording/diff.js";
import { parseRecordingLock, RECORDING_LOCK, recordingLockText } from "../../port-readiness/recording/store.js";
import { checkRecording, recordToolPack, writeRecording } from "./record.js";

const USAGE = `usage:
  npm run port-readiness:record -- --pack <id> --tool <tool> [--full] [--prune] [--cache dir] [--changes file]
  npm run port-readiness:record -- --pack <id> --tool <tool> --check [--base lock] [--recorded dir] [--cache dir] [--changes file]
`;

const megabytes = (bytes: number): string => `${(bytes / 1024 / 1024).toFixed(1)} MB`;

/**
 * The Node major every recording is made on: the one the port readiness workflows set up. What upstream answers can
 * depend on it (Node 22 and 25 send no `require.resolve` through the ESLint driver's `registerHooks` resolve hook, so
 * an eslint-plugin-vue test file that resolves its package root does not load), and the config id does not cover the
 * host's Node, so a record on another major is refused rather than filed under the same config.
 */
const RECORDING_NODE_MAJOR = 24;

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
  if (values.help === true || positionals.length > 0 || values.pack === undefined || values.tool === undefined || (values.check && values.prune) || (checkOnly && !values.check)) {
    process.stderr.write(USAGE);
    return values.help === true ? 0 : 2;
  }
  const nodeMajor = Number(process.versions.node.split(".")[0]);
  if (nodeMajor !== RECORDING_NODE_MAJOR) {
    process.stderr.write(`port-readiness:record runs on Node ${RECORDING_NODE_MAJOR}, as CI does, not Node ${process.versions.node}: upstream's answers can differ by Node major. Put Node ${RECORDING_NODE_MAJOR} first on PATH and record again.\n`);
    return 2;
  }
  const { pack, tool } = values;
  const record = `npm run port-readiness:record -- --pack ${pack} --tool ${tool}`;
  // Read before upstream runs, so a base lock that cannot be read fails first.
  const base = values.base === undefined ? undefined : existsSync(values.base) ? parseRecordingLock(readFileSync(values.base, "utf8"), values.base, record) : null;
  const started = Date.now();
  // --check asks everything again, so the locked answers are compared with a fresh upstream's.
  const recorded = await recordToolPack({
    root,
    pack,
    tool,
    cacheRoot,
    runner: new ProcessCommandRunner(),
    mode: { full: values.full || values.check, prune: values.prune },
  });
  const label = `packs/${pack}/upstream/${tool}`;
  const cases = Object.values(recorded.recording.rules).flatMap((rule) => rule.cases);
  const summary = `${Object.keys(recorded.recording.rules).length} rules (${recorded.recording.absent.length} absent upstream; ${recorded.asked.rules.length} asked of upstream), ${cases.length} cases `
    + `(${cases.filter((testCase) => testCase.skipped === "not-copied").length} not copied, ${recorded.deduplicated} sharing an answer), ${recorded.sources.length} case sources, `
    + `${recorded.asked.verdicts} verdicts asked, ${((Date.now() - started) / 1000).toFixed(0)} s`;

  // The per-rule summary replaces the recording's own diff, which is not in the tree to read.
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
    if (check.note !== null) process.stdout.write(`${label}: ${check.note}\n`);
    const problems = [
      ...(check.changed ? [`${label}/${RECORDING_LOCK} names a recording that differs from a fresh one`] : []),
      ...check.missing.slice(0, 10).map((copy) => `${label}/${copy.file} is missing`),
      ...(check.missing.length > 10 ? [`... and ${check.missing.length - 10} more missing case sources`] : []),
      ...check.conflicting.map((file) => `${label}/${file} differs from upstream's copy`),
    ];
    if (problems.length === 0) {
      // The committed lock names an archive the cache now holds or a record of the fresh answers made: the gates read it from the cache.
      if (check.committed !== null) cacheArchive(cacheRoot, check.committed);
      process.stdout.write(`${label}: the recording is current (${summary})\n`);
      await reportChanges(check.changes);
      return 0;
    }
    if (values.recorded !== undefined) {
      const directory = resolve(values.recorded);
      await mkdir(directory, { recursive: true });
      await writeFile(join(directory, RECORDING_LOCK), recordingLockText(check.lock));
      await writeFile(join(directory, basename(archiveCachePath(cacheRoot, check.packed.sha256))), check.packed.compressed);
      process.stdout.write(`${label}: the lock and the archive a record makes are in ${directory}\n`);
    }
    // An answer the locked archive holds is never asked again by an incremental record, so one upstream now gives
    // differently (a transitive package resolved at install time moved) is fixed only by asking everything again.
    process.stdout.write(`${label}: the recording is stale; run ${record}${check.changed ? " --full" : ""}\n${problems.map((problem) => `  ${problem}`).join("\n")}\n`);
    await reportChanges(check.changes);
    return 1;
  }
  const written = await writeRecording(root, pack, tool, recorded, { cacheRoot });
  process.stdout.write(`${label}: recorded ${summary}; copied ${written.missing.length} case sources\n`);
  // New case sources are pack test data, which git ignores: a contributor adds them to the pull request by hand.
  if (written.missing.length > 0) process.stdout.write(`${packFixturesWritten(written.missing.map((copy) => join(root, label, copy.file)))}\n`);
  const archive = written.lock.files[RECORDING_ARCHIVE];
  process.stdout.write(`  ${archiveCachePath(cacheRoot, archive.sha256)} (${archive.files} files, ${megabytes(archive.bytes)} uncompressed, ${megabytes(recorded.packed.compressed.length)} compressed)\n`);
  process.stdout.write(`  ${RECORDING_LOCK} names it\n`);
  await reportChanges(written.changes);
  return 0;
}

main().then((code) => {
  process.exitCode = code;
}, (error: unknown) => {
  process.stderr.write(`${error instanceof Error ? error.stack ?? error.message : String(error)}\n`);
  process.exitCode = 2;
});
