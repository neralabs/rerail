/**
 * Where a recording's bytes live now that they are not in the tree: each tool-pack commits a `recording.lock` naming
 * exactly the recording its rules are judged against — `recording.tar.zst`, the archive of every answer upstream gave
 * the recorder (archive.ts), by its tar's sha256, file count and bytes — and the bytes are an object in S3.
 *
 * The recordings were 96% of the branch that added the gates — 71 MB over 16 files, every one of them generated — and
 * git keeps every version of each of them forever, so re-recording a pack after an upstream bump cost another few
 * megabytes of history that nobody can ever read. The lock is a few hundred bytes and reviews as data; the bytes it
 * names are checked against it on every read, so a commit still pins exactly one recording.
 *
 * Locally the recorder writes the archive into `.rerail/port-readiness/recordings/<sha256>.tar.zst` and its extracted
 * tree into `tree/<sha256>/` beside it, where the gates read them. Their durable store is S3, written once at
 * s3://nera-test-fixtures/rerail/recordings/<pack>/<tool>/<sha256>.tar.zst by `node scripts/s3/pack-fixtures.mjs
 * recordings push` or the bump's publish workflow; `node scripts/s3/pack-fixtures.mjs recordings` fetches the ones a
 * checkout's locks name into that directory. In CI the directory is restored from the GitHub Actions cache, keyed by
 * the tool-pack and the lock's digest (port-readiness.yml), which the test-data job fills from S3; a gate leg that
 * finds it in neither records the tool-pack from its own commit's recorder, which must write the lock the commit has.
 * Whatever the source, a recording is bytes with the digest the lock names, or it is absent.
 */
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";

import { z } from "zod";

import { toolPackDirectory } from "../manifest.js";
import { RECORDING_ARCHIVE, RECORDING_ARCHIVE_FORMAT, type ArchivePin } from "./archive.js";

/** The committed manifest's file name in a tool-pack's directory. */
export const RECORDING_LOCK = "recording.lock";

export const RECORDING_LOCK_FORMAT = "rerail.port-readiness-recording-lock/v4";

const digestSchema = z.string().regex(/^[0-9a-f]{64}$/u, "a sha256 in lowercase hex");
const nonEmpty = z.string().min(1);
const count = z.number().int().nonnegative();

/** recording.tar.zst: its tar's sha256, which it is cached and stored under, and what the tar holds. */
const lockedArchiveSchema = z.strictObject({
  sha256: digestSchema,
  /** How many files the tar holds. */
  files: count,
  /** Their total size in bytes. */
  bytes: count,
  format: z.literal(RECORDING_ARCHIVE_FORMAT),
});

export type LockedArchive = z.infer<typeof lockedArchiveSchema>;

export const recordingLockSchema = z.strictObject({
  format: z.literal(RECORDING_LOCK_FORMAT),
  /**
   * The tool: the directory under the pack's upstream/. The lock names no recorder version: a re-record of the same
   * upstream writes the same lock whatever version of rerail runs it, which is what port-readiness.yml's recording leg
   * holds a commit to.
   */
  tool: nonEmpty,
  /**
   * The upstream pins the recording was made at. The stale-recording checks read them here, from the committed
   * manifest, so a pull request whose pins moved fails before any recording is read.
   */
  pins: z.strictObject({ repository: nonEmpty, commit: z.string().regex(/^[0-9a-f]{40}$/u), version: nonEmpty, runtime: nonEmpty }),
  /** The runtimes' committed pin digests, as the recording records them, for the same reason. */
  runtimes: z.record(z.string().regex(/^[a-z0-9._-]+$/u), z.record(z.string().regex(/^[A-Za-z0-9._-]+$/u), digestSchema)),
  /** The archive, by its file name. */
  files: z.strictObject({ [RECORDING_ARCHIVE]: lockedArchiveSchema }),
});

export type RecordingLock = z.infer<typeof recordingLockSchema>;

export function recordingLockPath(root: string, pack: string, tool: string): string {
  return join(toolPackDirectory(root, pack, tool), RECORDING_LOCK);
}

/**
 * The lock a tool-pack commits. A tool-pack with a gates.yaml and no lock has never been recorded; the message says
 * so, because the gates cannot tell that from a lock that failed to read. A lock of an older format (v2 named
 * recording.json and corpus.json, v3 recording.json and corpus.tar.zst, which the one archive replaced) is refused:
 * nothing reads it, and the tool-pack is recorded again.
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

/** The lock for a tool-pack whose recording archive has just been written. */
export function buildRecordingLock(tool: string, pins: RecordingLock["pins"], runtimes: RecordingLock["runtimes"], archive: ArchivePin): RecordingLock {
  return recordingLockSchema.parse({
    format: RECORDING_LOCK_FORMAT,
    tool,
    pins,
    runtimes,
    files: { [RECORDING_ARCHIVE]: { sha256: archive.sha256, files: archive.files, bytes: archive.bytes, format: RECORDING_ARCHIVE_FORMAT } },
  });
}
