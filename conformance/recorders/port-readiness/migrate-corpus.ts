/**
 * The one-off migration from corpus.json (`rerail.port-readiness-corpus/v2`, a v2 recording.lock's second recording)
 * to the corpus answers archive (`rerail.port-readiness-corpus-answers/v1`, a v3 lock). It is the only reader of the
 * old format. It runs no upstream: every answer corpus.json holds — each of its files for each of its rules, and the
 * parsers' verdicts on each — is written into the tree under the name the gates look it up by, the tree is packed into
 * the cache, and the lock is rewritten to v3 with the same recording.json.
 *
 * Naming an answer needs what its key reads beyond the file: for Ruff the context files present beside it, which are
 * fetched from the pinned corpus as the gates fetch them; for PHPStan and Psalm the batch, every file of the task
 * corpus.json answered, which the recorder handed upstream at once; for Clippy the corpus's commit, from the pins.
 */
import { createHash } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

import { z } from "zod";

import { buildCorpus, buildRerailCorpora, readCorpusTasks, readRerailCorpora, wantedPlaces } from "../../port-readiness/corpus.js";
import { readGatesManifest, toolPackDirectory } from "../../port-readiness/manifest.js";
import type { CommandRunner } from "../../port-readiness/process.js";
import { recordedInCorpus } from "../../port-readiness/recorded-adapter.js";
import {
  answerText, cacheCorpusArchive, CONTEXT_FILES, corpusCommits, fetchedContext, neededAnswers, nextTree, packCorpusTree, verdictPath, verdictText, type PackedCorpus,
} from "../../port-readiness/recording/corpus-answers.js";
import { cleanVerdicts, gatesRecordingSchema, parserVerdictSchema, recordedSpanSchema, RECORDING_FILE, spanOf } from "../../port-readiness/recording/schema.js";
import { buildRecordingLock, readCachedRecording, RECORDING_LOCK, recordingLockSchema, recordingLockText } from "../../port-readiness/recording/store.js";

const digest = z.string().regex(/^[0-9a-f]{64}$/u);

/** The v2 lock: what it pinned, and the two recordings by name. */
const v2LockSchema = z.object({
  format: z.literal("rerail.port-readiness-recording-lock/v2"),
  tool: z.string().min(1),
  pins: z.strictObject({ repository: z.string(), commit: z.string(), version: z.string(), runtime: z.string() }),
  runtimes: z.record(z.string(), z.record(z.string(), digest)),
  files: z.object({ "recording.json": z.object({ sha256: digest }), "corpus.json": z.object({ sha256: digest }) }),
});

/** What the migration reads of a v2 corpus.json: its rules, and every file's answer and verdicts. */
const v2CorpusSchema = z.object({
  format: z.literal("rerail.port-readiness-corpus/v2"),
  rules: z.array(z.string().min(1)),
  files: z.array(z.object({
    task: z.string().min(1),
    path: z.string().min(1),
    sha256: digest,
    excluded: z.string().nullable(),
    findings: z.record(z.string(), z.array(recordedSpanSchema)),
    verdicts: z.array(parserVerdictSchema).optional(),
  })),
});

/** What a migrated tool-pack's archive is. */
export interface MigratedCorpus {
  pack: string;
  tool: string;
  packed: PackedCorpus;
  answers: number;
  verdicts: number;
}

/** Throws when two corpus.json files' paths in one task differ only by letter case: a case-insensitive tree cannot hold both. */
function checkCaseCollisions(files: readonly { task: string; path: string }[], label: string): void {
  const seen = new Map<string, string>();
  for (const { task, path } of files) {
    const key = `${task}\0${path.toLowerCase()}`;
    const other = seen.get(key);
    if (other !== undefined && other !== path) throw new Error(`${label}: ${task}/${other} and ${task}/${path} differ only by letter case, which a case-insensitive file system cannot hold as two answers`);
    seen.set(key, path);
  }
}

/** Migrate one tool-pack's locked corpus.json into its archive in the cache, and rewrite its lock to v3; null when it is v3 already. */
export async function migrateCorpus(root: string, cacheRoot: string, pack: string, tool: string, runner: CommandRunner): Promise<MigratedCorpus | null> {
  const label = `packs/${pack}/upstream/${tool}`;
  const lockPath = join(toolPackDirectory(root, pack, tool), RECORDING_LOCK);
  const raw: unknown = JSON.parse(readFileSync(lockPath, "utf8"));
  if (recordingLockSchema.safeParse(raw).success) return null;
  const lock = v2LockSchema.parse(raw);
  const read = (name: string, sha: string): string => {
    const text = readCachedRecording(cacheRoot, sha);
    if (text === undefined) throw new Error(`${label}: the locked ${name} (sha256 ${sha.slice(0, 12)}) is not in ${cacheRoot}/recordings: fetch it with the pre-migration pack-fixtures.mjs recordings`);
    return text;
  };
  const recordingText = read(RECORDING_FILE, lock.files["recording.json"].sha256);
  const recording = gatesRecordingSchema.parse(JSON.parse(recordingText));
  const corpus = v2CorpusSchema.parse(JSON.parse(read("corpus.json", lock.files["corpus.json"].sha256)));
  checkCaseCollisions(corpus.files, label);
  const manifest = readGatesManifest(root, pack, tool);

  // The context files beside each answered file, fetched from the pinned corpus as the gates fetch them.
  const corpora = await readRerailCorpora(root, pack);
  const contextPlaces = corpus.files.flatMap((file) => CONTEXT_FILES[manifest.adapter](file.path).map((path) => ({ task: file.task, path })));
  const inCorpus = recordedInCorpus(recording);
  const wanted = wantedPlaces(contextPlaces);
  const fetched = contextPlaces.length === 0 ? [] : [
    ...(await buildCorpus(cacheRoot, inCorpus, runner, wanted)).files,
    ...(await buildRerailCorpora(corpora, cacheRoot, inCorpus, runner, wanted)).files,
  ];
  // The files corpus.json answered were every golden's file, recorded at once: for an adapter whose answers depend on
  // the batch (BATCHED_ADAPTERS) each task's of them is the batch its answers are keyed by.
  const needed = neededAnswers({
    manifest,
    runtimes: lock.runtimes,
    parsers: recording.parsers,
    rules: corpus.rules,
    files: corpus.files,
    context: fetchedContext([...fetched, ...corpus.files], corpusCommits(corpora, readCorpusTasks())),
  });

  const byPlace = new Map(corpus.files.map((file) => [`${file.task}\0${file.path}\0${file.sha256}`, file]));
  const written = new Map<string, Buffer>();
  for (const { rule, file, path } of needed.answers) {
    const recorded = byPlace.get(`${file.task}\0${file.path}\0${file.sha256}`)!;
    written.set(path, Buffer.from(answerText({ excluded: recorded.excluded, findings: (recorded.findings[rule] ?? []).map(spanOf) }), "utf8"));
  }
  // Every corpus file's verdicts, so files that share a verdict name (one content, one file type) must agree.
  const silent = cleanVerdicts(recording.parsers);
  for (const file of corpus.files) {
    const path = verdictPath(needed.parsersId, { path: file.path, sha256: file.sha256, script: false });
    const text = Buffer.from(verdictText(file.verdicts ?? silent), "utf8");
    const other = written.get(path);
    if (other !== undefined && !other.equals(text)) throw new Error(`${label}: two corpus files with one content and file type have different parser verdicts (${path})`);
    written.set(path, text);
  }

  const tree = nextTree(new Map(), needed, written, false);
  const packed = packCorpusTree(tree);
  cacheCorpusArchive(cacheRoot, packed);
  const v3 = buildRecordingLock(lock.tool, lock.pins, lock.runtimes, recordingText, packed);
  if (createHash("sha256").update(recordingText).digest("hex") !== v3.files[RECORDING_FILE].sha256) throw new Error(`${label}: recording.json changed in the migration`);
  writeFileSync(lockPath, recordingLockText(v3));
  return { pack, tool, packed, answers: needed.answers.length, verdicts: needed.verdicts.length };
}
