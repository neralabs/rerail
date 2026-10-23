/**
 * Where a recording's bytes live now that they are not in the tree: each tool-pack commits a `recording.lock` naming,
 * by sha256, exactly the recording its rules are judged against — `recording.json`, and `corpus.tar.zst`, the archive
 * of upstream's answers on the golden corpus files (corpus-answers.ts) — and the bytes are objects in S3.
 *
 * The recordings were 96% of the branch that added the gates — 71 MB over 16 files, every one of them generated — and
 * git keeps every version of each of them forever, so re-recording a pack after an upstream bump cost another few
 * megabytes of history that nobody can ever read. The lock is a few hundred bytes and reviews as data; the bytes it
 * names are checked against it on every read, so a commit still pins exactly one recording.
 *
 * Locally the recorder writes them into `.rerail/port-readiness/recordings/`: `<sha256>.json`, `<sha256>.tar.zst` and
 * the archive's extracted tree `corpus/<sha256>/`, where the gates read them. Their durable store is S3, written once
 * at s3://nera-test-fixtures/rerail/recordings/<pack>/<tool>/<sha256>.json and …/corpus/<sha256>.tar.zst by
 * `node scripts/s3/pack-fixtures.mjs recordings push` or the bump's publish workflow; `node scripts/s3/pack-fixtures.mjs
 * recordings` fetches the ones a checkout's locks name into that directory. In CI the directory is restored from the
 * GitHub Actions cache, keyed by the tool-pack and the lock's two digests (port-readiness.yml), which the test-data job
 * fills from S3; a gate leg that finds them in neither records the tool-pack from its own commit's recorder, which must
 * write the lock the commit has. Whatever the source, a recording is bytes with the digest the lock names, or it is
 * absent.
 */
import { createHash } from "node:crypto";
import { existsSync, readFileSync } from "node:fs";
import { mkdir, rename, writeFile } from "node:fs/promises";
import { dirname, join } from "node:path";

import { z } from "zod";

import { toolPackDirectory } from "../manifest.js";
import { CORPUS_ANSWERS_FORMAT, CORPUS_ARCHIVE, type CorpusArchivePin } from "./corpus-answers.js";
import { RECORDING_FILE, RECORDING_FORMAT } from "./schema.js";

const sha256 = (bytes: string | Buffer): string => createHash("sha256").update(bytes).digest("hex");

/** The committed manifest's file name in a tool-pack's directory. */
export const RECORDING_LOCK = "recording.lock";

export const RECORDING_LOCK_FORMAT = "rerail.port-readiness-recording-lock/v3";

const digestSchema = z.string().regex(/^[0-9a-f]{64}$/u, "a sha256 in lowercase hex");
const nonEmpty = z.string().min(1);
const count = z.number().int().nonnegative();

/** recording.json: what it is and how big. */
const lockedRecordingSchema = z.strictObject({
  /** sha256 of the recording's text, UTF-8: the name it is cached under, and what every read checks. */
  sha256: digestSchema,
  /** The text's length in bytes. */
  size: z.number().int().positive(),
  /** The recording's own `format`, so a v2 reader never parses a v3 recording. */
  format: z.literal(RECORDING_FORMAT),
});

/** corpus.tar.zst: its tar's sha256, which it is cached and stored under, and what the tar holds. */
const lockedCorpusSchema = z.strictObject({
  sha256: digestSchema,
  /** How many answer, verdict and config files the tar holds: none for a tool-pack without goldens. */
  files: count,
  /** Their total size in bytes. */
  bytes: count,
  format: z.literal(CORPUS_ANSWERS_FORMAT),
});

export type LockedRecording = z.infer<typeof lockedRecordingSchema>;
export type LockedCorpus = z.infer<typeof lockedCorpusSchema>;

export const recordingLockSchema = z.strictObject({
  format: z.literal(RECORDING_LOCK_FORMAT),
  /**
   * The tool, as in recording.json: the directory under the pack's upstream/. The lock names no recorder version: a
   * re-record of the same upstream writes the same lock whatever version of rerail runs it, which is what
   * port-readiness.yml's recording leg holds a commit to.
   */
  tool: nonEmpty,
  /**
   * The upstream pins the recording was made at, copied from recording.json. The stale-recording checks read them here,
   * from the committed manifest, so a pull request whose pins moved fails before any recording is read.
   */
  pins: z.strictObject({ repository: nonEmpty, commit: z.string().regex(/^[0-9a-f]{40}$/u), version: nonEmpty, runtime: nonEmpty }),
  /** The runtimes' committed pin digests, as recording.json records them, for the same reason. */
  runtimes: z.record(z.string().regex(/^[a-z0-9._-]+$/u), z.record(z.string().regex(/^[A-Za-z0-9._-]+$/u), digestSchema)),
  /** recording.json and the corpus answers archive, by their file name. */
  files: z.strictObject({ [RECORDING_FILE]: lockedRecordingSchema, [CORPUS_ARCHIVE]: lockedCorpusSchema }),
});

export type RecordingLock = z.infer<typeof recordingLockSchema>;

export function recordingLockPath(root: string, pack: string, tool: string): string {
  return join(toolPackDirectory(root, pack, tool), RECORDING_LOCK);
}

/**
 * The lock a tool-pack commits. A tool-pack with a gates.yaml and no lock has never been recorded; the message says
 * so, because the gates cannot tell that from a lock that failed to read. A lock of an older format (a v2 one names
 * corpus.json, which the corpus answers archive replaced) is refused: nothing reads it, and the tool-pack is recorded
 * again.
 */
export function readRecordingLock(root: string, pack: string, tool: string): RecordingLock {
  const path = recordingLockPath(root, pack, tool);
  const label = `packs/${pack}/upstream/${tool}/${RECORDING_LOCK}`;
  if (!existsSync(path)) throw new Error(`${label} is missing: run npm run port-readiness:record -- --pack ${pack} --tool ${tool}`);
  return parseRecordingLock(readFileSync(path, "utf8"), label, `npm run port-readiness:record -- --pack ${pack} --tool ${tool}`);
}

/** A lock's text, named `label` in errors; one of another format is refused, saying to delete it and run `record`. */
export function parseRecordingLock(text: string, label: string, record: string): RecordingLock {
  const raw: unknown = JSON.parse(text);
  const format = raw !== null && typeof raw === "object" && "format" in raw ? raw.format : undefined;
  if (format !== RECORDING_LOCK_FORMAT) {
    throw new Error(`${label} is format ${JSON.stringify(format)}, not ${RECORDING_LOCK_FORMAT}, and is not read: delete it and record the tool-pack again (${record})`);
  }
  const parsed = recordingLockSchema.safeParse(raw);
  if (!parsed.success) throw new Error(`${label}: ${parsed.error.issues.slice(0, 5).map((issue) => `/${issue.path.map(String).join("/")}: ${issue.message}`).join("; ")}`);
  return parsed.data;
}

/** The lock as it is committed: two-space JSON with a final newline, keys in the order the schema declares them. */
export function recordingLockText(lock: RecordingLock): string {
  return `${JSON.stringify(lock, null, 2)}\n`;
}

/** Where a recording is cached: by its own digest, so two commits naming the same recording share one copy. */
export function recordingCachePath(cacheRoot: string, digest: string): string {
  return join(cacheRoot, "recordings", `${digest}.json`);
}

/**
 * A cached recording's text, or undefined when it is not cached. Bytes whose digest is not the one asked for are
 * treated as absent and not read: a truncated cache entry or a half-written file must not become a recording, and the
 * gates report it missing.
 */
export function readCachedRecording(cacheRoot: string, digest: string): string | undefined {
  const path = recordingCachePath(cacheRoot, digest);
  if (!existsSync(path)) return undefined;
  const bytes = readFileSync(path);
  return sha256(bytes) === digest ? bytes.toString("utf8") : undefined;
}

/** Put a recording in the cache under its own digest, written through a temporary name so a reader never sees half of it. */
export async function writeCachedRecording(cacheRoot: string, text: string): Promise<string> {
  const digest = sha256(Buffer.from(text, "utf8"));
  const path = recordingCachePath(cacheRoot, digest);
  await mkdir(dirname(path), { recursive: true });
  const temporary = `${path}.${process.pid}.tmp`;
  await writeFile(temporary, text);
  await rename(temporary, path);
  return digest;
}

/** The lock for a tool-pack whose recording.json and corpus answers archive have just been written. */
export function buildRecordingLock(
  tool: string,
  pins: RecordingLock["pins"],
  runtimes: RecordingLock["runtimes"],
  recording: string,
  corpus: CorpusArchivePin,
): RecordingLock {
  return recordingLockSchema.parse({
    format: RECORDING_LOCK_FORMAT,
    tool,
    pins,
    runtimes,
    files: {
      [RECORDING_FILE]: { sha256: sha256(Buffer.from(recording, "utf8")), size: Buffer.byteLength(recording), format: RECORDING_FORMAT },
      [CORPUS_ARCHIVE]: { sha256: corpus.sha256, files: corpus.files, bytes: corpus.bytes, format: CORPUS_ANSWERS_FORMAT },
    },
  });
}
