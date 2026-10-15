/**
 * Where a recording's bytes live now that they are not in the tree: a GitHub release holds them, and each tool-pack
 * commits a `recording.lock` naming exactly the one its rules are judged against.
 *
 * The recordings were 96% of the branch that added the gates — 71 MB over 16 files, every one of them generated — and
 * git keeps every version of each of them forever, so re-recording a pack after an upstream bump cost another few
 * megabytes of history that nobody can ever read. The lock is a few hundred bytes and reviews as data; the bytes it
 * names are fetched and checked against its sha256, so a commit still pins exactly one recording.
 *
 * Release assets, not Git LFS: every public clone and every fork would draw on the repository's LFS bandwidth quota,
 * and going over it blocks LFS for the whole repository, including the native engine's fixtures. Not Actions
 * artifacts either: those expire after 90 days, and a commit must stay judgeable. Release downloads have no stated
 * bandwidth limit, need no account once the repository is public, and a fork pull request's token is read-only, so a
 * fork cannot upload one (rule-gates-record.yml records instead).
 *
 * An asset is named for the sha256 of what it decompresses to, so it is write-once: the same recording is the same
 * asset, a changed recording is a new one, and nothing a commit already names is ever replaced.
 */
import { createHash } from "node:crypto";
import { existsSync, readFileSync } from "node:fs";
import { mkdir, rename, writeFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { gunzipSync, gzipSync } from "node:zlib";

import { z } from "zod";

import { toolPackDirectory } from "../manifest.js";
import { GATES_CORPUS, GATES_CORPUS_FORMAT, GATES_RECORDING, GATES_RECORDING_FORMAT } from "./schema.js";

const sha256 = (bytes: string | Buffer): string => createHash("sha256").update(bytes).digest("hex");

/** The committed manifest's file name in a tool-pack's directory. */
export const RECORDING_LOCK = "recording.lock";

export const RECORDING_LOCK_FORMAT = "rerail.gates-recording-lock/v1";

/**
 * The prerelease the recorder uploads to by default. It is a prerelease so it never shows as the repository's current
 * release, and the recorder rolls to `gate-recordings-2`, `-3` and so on before a release reaches GitHub's cap of
 * 1,000 assets; a lock names the release that holds its asset, so an older commit keeps fetching from the older one.
 */
export const DEFAULT_RELEASE = "gate-recordings";

const digestSchema = z.string().regex(/^[0-9a-f]{64}$/u, "a sha256 in lowercase hex");
const nonEmpty = z.string().min(1);

/** One recording in the release: what it is, how big, and under what name the release holds it. */
const lockedFileSchema = z.strictObject({
  /** sha256 of the recording's text, UTF-8: what the fetched asset must decompress to. */
  sha256: digestSchema,
  /** The text's length in bytes, so a fetch can report what it is about to read before it reads it. */
  size: z.number().int().positive(),
  /** The recording's own `format`, checked after the download so a v2 reader never parses a v3 recording. */
  format: z.enum([GATES_RECORDING_FORMAT, GATES_CORPUS_FORMAT]),
  /** The asset's name in the release: `<pack>-<tool>-<stem>-<sha256>.json.gz`. */
  asset: nonEmpty,
});

export type LockedFile = z.infer<typeof lockedFileSchema>;

export const recordingLockSchema = z.strictObject({
  format: z.literal(RECORDING_LOCK_FORMAT),
  /** The tool, as in gates.json: the directory under the pack's upstream/. */
  tool: nonEmpty,
  /** The release tag holding both assets. */
  release: nonEmpty,
  /**
   * The recorder that wrote it: the repository version it ran from. It dates a recording for a reader without
   * deciding anything — no check reads it, so it cannot make a gate pass or fail.
   */
  recorder: nonEmpty,
  /**
   * The upstream pins the recording was made at, copied from gates.json. The stale-recording checks read them here,
   * from the committed manifest, so a pull request whose pins moved fails before anything is downloaded.
   */
  pins: z.strictObject({ repository: nonEmpty, commit: z.string().regex(/^[0-9a-f]{40}$/u), version: nonEmpty, runtime: nonEmpty }),
  /** The runtimes' committed pin digests, as gates.json records them, for the same reason. */
  runtimes: z.record(z.string().regex(/^[a-z0-9._-]+$/u), z.record(z.string().regex(/^[A-Za-z0-9._-]+$/u), digestSchema)),
  /** Both recordings, by their file name. */
  files: z.strictObject({ [GATES_RECORDING]: lockedFileSchema, [GATES_CORPUS]: lockedFileSchema }),
});

export type RecordingLock = z.infer<typeof recordingLockSchema>;

/** A recording's asset name: content-addressed, so the same bytes are always the same asset. */
export function assetName(pack: string, tool: string, file: string, digest: string): string {
  return `${pack}-${tool}-${file.replace(/\.json$/u, "")}-${digest}.json.gz`;
}

export function recordingLockPath(root: string, pack: string, tool: string): string {
  return join(toolPackDirectory(root, pack, tool), RECORDING_LOCK);
}

/**
 * The lock a tool-pack commits. A tool-pack with a gates.yaml and no lock has never been recorded; the message says
 * so, because the gates cannot tell that from a lock that failed to read.
 */
export function readRecordingLock(root: string, pack: string, tool: string): RecordingLock {
  const path = recordingLockPath(root, pack, tool);
  if (!existsSync(path)) throw new Error(`packs/${pack}/upstream/${tool}/${RECORDING_LOCK} is missing: run npm run gates:record -- --pack ${pack} --tool ${tool}`);
  const parsed = recordingLockSchema.safeParse(JSON.parse(readFileSync(path, "utf8")));
  if (!parsed.success) throw new Error(`packs/${pack}/upstream/${tool}/${RECORDING_LOCK}: ${parsed.error.issues.slice(0, 5).map((issue) => `/${issue.path.map(String).join("/")}: ${issue.message}`).join("; ")}`);
  return parsed.data;
}

/** The lock as it is committed: two-space JSON with a final newline, keys in the order the schema declares them. */
export function recordingLockText(lock: RecordingLock): string {
  return `${JSON.stringify(lock, null, 2)}\n`;
}

/** Where a fetched recording is kept: by its own digest, so two commits naming the same recording share one copy. */
export function recordingCachePath(cacheRoot: string, digest: string): string {
  return join(cacheRoot, "recordings", `${digest}.json`);
}

/**
 * A cached recording's text, or undefined when it is not cached. Bytes whose digest is not the one asked for are
 * treated as absent and not read: a truncated download or a half-written file must not become a recording, and the
 * caller fetches again.
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

/** What a recording weighs in the release: gzip at the default level, which is what the recorder uploads. */
export function gzipRecording(text: string): Buffer {
  return gzipSync(Buffer.from(text, "utf8"));
}

/**
 * The text an asset holds, checked against the digest its name and its lock entry carry. A digest that does not match
 * fails here and nowhere else: the alternative is judging rules against bytes no commit pinned, which is exactly the
 * hole a fork pull request would reach for.
 */
export function recordingFromAsset(bytes: Buffer, locked: LockedFile, label: string): string {
  const text = gunzipSync(bytes).toString("utf8");
  const digest = sha256(Buffer.from(text, "utf8"));
  if (digest !== locked.sha256) throw new Error(`${label}: the release's ${locked.asset} has sha256 ${digest.slice(0, 12)}, and ${RECORDING_LOCK} names ${locked.sha256.slice(0, 12)}`);
  const format: unknown = (JSON.parse(text) as { format?: unknown }).format;
  if (format !== locked.format) throw new Error(`${label}: the release's ${locked.asset} is ${String(format)}, and ${RECORDING_LOCK} names ${locked.format}`);
  return text;
}

/** The lock for a tool-pack whose two recordings have just been written. */
export function buildRecordingLock(
  pack: string,
  tool: string,
  release: string,
  recorder: string,
  pins: RecordingLock["pins"],
  runtimes: RecordingLock["runtimes"],
  texts: { [GATES_RECORDING]: string; [GATES_CORPUS]: string },
): RecordingLock {
  const locked = (file: string, text: string, format: LockedFile["format"]): LockedFile => {
    const digest = sha256(Buffer.from(text, "utf8"));
    return { sha256: digest, size: Buffer.byteLength(text), format, asset: assetName(pack, tool, file, digest) };
  };
  return recordingLockSchema.parse({
    format: RECORDING_LOCK_FORMAT,
    tool,
    release,
    recorder,
    pins,
    runtimes,
    files: {
      [GATES_RECORDING]: locked(GATES_RECORDING, texts[GATES_RECORDING], GATES_RECORDING_FORMAT),
      [GATES_CORPUS]: locked(GATES_CORPUS, texts[GATES_CORPUS], GATES_CORPUS_FORMAT),
    },
  });
}
