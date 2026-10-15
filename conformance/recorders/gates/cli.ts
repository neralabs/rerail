/**
 * `npm run gates:record`: run a tool-pack's live adapter and write the recording the gates read
 * (packs/<pack>/upstream/<tool>/gates.json, gates-corpus.json and missing case sources). `--check` writes nothing and
 * exits 1 when a fresh recording differs from the committed one.
 *
 * `npm run gates:record -- migrate-v2` rewrites every committed recording in the v2 format instead of recording
 * anything, which needs no upstream runtime (migrate-v2.ts).
 */
import { mkdir, writeFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { parseArgs } from "node:util";

import { PACKAGE_ROOT } from "../../../src/engine/index.js";
import { DEFAULT_CACHE_ROOT } from "../../gates/corpus.js";
import { ProcessCommandRunner } from "../../gates/process.js";
import { renderRecordingChanges } from "../../gates/recording-diff.js";
import { RECORDING_LOCK } from "../../gates/recording-store.js";
import { liftRecordings, migrateToolPacks } from "./migrate-v2.js";
import { diffRecording, recordToolPack, writeRecording } from "./record.js";

const USAGE = `usage:
  npm run gates:record -- --pack <id> --tool <tool> [--cache dir] [--release tag] [--changes file] [--check]
  npm run gates:record -- migrate-v2 [--check]
  npm run gates:record -- lift [--cache dir] [--release tag]
`;

/** `migrate-v2`: rewrite every committed recording in the v2 format; `--check` writes nothing and reports what differs. */
async function migrate(root: string, check: boolean): Promise<number> {
  const results = await migrateToolPacks(root, !check);
  const stale = results.filter((result) => !result.unchanged);
  const mb = (bytes: number) => `${(bytes / 1024 / 1024).toFixed(1)} MB`;
  for (const result of results) {
    const label = `packs/${result.pack}/upstream/${result.tool}`;
    process.stdout.write(result.unchanged
      ? `${label}: already v2 (${mb(result.after)})\n`
      : `${label}: ${mb(result.before)} → ${mb(result.after)} (${check ? "would rewrite" : "rewritten"})\n`);
  }
  const before = results.reduce((total, result) => total + result.before, 0);
  const after = results.reduce((total, result) => total + result.after, 0);
  process.stdout.write(`${results.length} tool-pack(s), ${stale.length} to rewrite: ${mb(before)} → ${mb(after)}\n`);
  return check && stale.length > 0 ? 1 : 0;
}

/**
 * `lift`: seed the release from the recordings that were committed before it existed. It writes each tool-pack's
 * `recording.lock`, puts both recordings in the cache and the gzipped assets in `<cache>/recordings/upload/`; the
 * in-tree files are then removed from the tree and the assets uploaded once.
 */
async function lift(root: string, cacheRoot: string, release: string | undefined): Promise<number> {
  const lifted = await liftRecordings(root, cacheRoot, release);
  let total = 0;
  for (const entry of lifted) {
    process.stdout.write(`packs/${entry.pack}/upstream/${entry.tool}/${RECORDING_LOCK} names ${entry.lock.release}\n`);
    for (const asset of entry.assets) {
      total += asset.bytes;
      process.stdout.write(`  ${asset.name} (${(asset.bytes / 1024 / 1024).toFixed(1)} MB gzipped)\n`);
    }
  }
  process.stdout.write(`${lifted.length} tool-pack(s), ${lifted.length * 2} assets, ${(total / 1024 / 1024).toFixed(1)} MB gzipped in ${cacheRoot}/recordings/upload/\n`);
  return 0;
}

async function main(): Promise<number> {
  const { values, positionals } = parseArgs({
    options: {
      pack: { type: "string" },
      tool: { type: "string" },
      cache: { type: "string" },
      release: { type: "string" },
      changes: { type: "string" },
      check: { type: "boolean", default: false },
      help: { type: "boolean", default: false },
    },
    allowPositionals: true,
    strict: true,
  });
  if (positionals[0] === "migrate-v2") return migrate(PACKAGE_ROOT, values.check === true);
  if (positionals[0] === "lift") return lift(PACKAGE_ROOT, resolve(values.cache ?? DEFAULT_CACHE_ROOT), values.release);
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
    const diff = diffRecording(root, values.pack, values.tool, recorded);
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
    process.stdout.write(`${label}: the recording is stale; run npm run gates:record -- --pack ${values.pack} --tool ${values.tool}\n${problems.map((problem) => `  ${problem}`).join("\n")}\n`);
    return 1;
  }
  const written = await writeRecording(root, values.pack, values.tool, recorded, { cacheRoot, ...(values.release === undefined ? {} : { release: values.release }) });
  process.stdout.write(`${label}: recorded ${summary}; copied ${written.missing.length} case sources\n`);
  for (const asset of written.assets) process.stdout.write(`  ${asset.name} (${(asset.bytes / 1024 / 1024).toFixed(1)} MB gzipped)\n`);
  process.stdout.write(`  ${RECORDING_LOCK} names them; the recordings are in ${cacheRoot}/recordings/ and the assets to upload in ${cacheRoot}/recordings/upload/\n`);

  // The per-rule summary replaces the recording's own diff, which is no longer in the tree to read.
  const changes = [written.changes];
  if (values.changes !== undefined) {
    await mkdir(dirname(resolve(values.changes)), { recursive: true });
    await writeFile(resolve(values.changes), renderRecordingChanges(changes));
  }
  process.stdout.write(written.changes.first
    ? `  first recording: nothing to compare upstream's answers with\n`
    : `  ${written.changes.rules.length} upstream rule(s) answer differently${written.changes.pinsChanged.length === 0 ? "" : `, pins moved (${written.changes.pinsChanged.join("; ")})`}\n`);
  return 0;
}

main().then((code) => {
  process.exitCode = code;
}, (error: unknown) => {
  process.stderr.write(`${error instanceof Error ? error.stack ?? error.message : String(error)}\n`);
  process.exitCode = 2;
});
