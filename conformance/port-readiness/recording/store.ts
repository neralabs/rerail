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
 * fork cannot upload one (port-readiness-record.yml records instead).
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
import { CORPUS_FILE, CORPUS_FORMAT, RECORDING_FILE, RECORDING_FORMAT } from "./schema.js";

const sha256 = (bytes: string | Buffer): string => createHash("sha256").update(bytes).digest("hex");

/** The committed manifest's file name in a tool-pack's directory. */
export const RECORDING_LOCK = "recording.lock";

export const RECORDING_LOCK_FORMAT = "rerail.port-readiness-recording-lock/v1";

/**
 * The prerelease a lock names when `port-readiness:record` is given no `--release`. It is a prerelease so it never shows as the
 * repository's current release. GitHub caps a release at 1,000 assets, so port-readiness-record.yml, the only job that
 * uploads, passes `--release` itself: the newest of `port-readiness-recordings`, `port-readiness-recordings-2`, `-3`
 * and so on, or the next once the newest holds 900, and it refuses an upload that would pass the cap. A lock names the
 * release that holds its assets, so an older commit keeps fetching from the older one.
 */
export const DEFAULT_RELEASE = "port-readiness-recordings";

const digestSchema = z.string().regex(/^[0-9a-f]{64}$/u, "a sha256 in lowercase hex");
const nonEmpty = z.string().min(1);

/** One recording in the release: what it is, how big, and under what name the release holds it. */
const lockedFileSchema = z.strictObject({
  /** sha256 of the recording's text, UTF-8: what the fetched asset must decompress to. */
  sha256: digestSchema,
  /** The text's length in bytes, so a fetch can report what it is about to read before it reads it. */
  size: z.number().int().positive(),
  /** The recording's own `format`, checked after the download so a v2 reader never parses a v3 recording. */
  format: z.enum([RECORDING_FORMAT, CORPUS_FORMAT]),
  /**
   * The asset's name in the release: `<pack>-<tool>-<stem>-<sha256>.json.gz`. readRecordingLock refuses any other,
   * so a lock cannot name another tool-pack's recording, or an asset under a name of its own choosing.
   */
  asset: nonEmpty,
});

export type LockedFile = z.infer<typeof lockedFileSchema>;

export const recordingLockSchema = z.strictObject({
  format: z.literal(RECORDING_LOCK_FORMAT),
  /** The tool, as in recording.json: the directory under the pack's upstream/. */
  tool: nonEmpty,
  /** The release tag holding both assets. */
  release: nonEmpty,
  /**
   * The recorder that wrote it: the repository version it ran from. It dates a recording for a reader without
   * deciding anything — no check reads it, so it cannot make a gate pass or fail.
   */
  recorder: nonEmpty,
  /**
   * The upstream pins the recording was made at, copied from recording.json. The stale-recording checks read them here,
   * from the committed manifest, so a pull request whose pins moved fails before anything is downloaded.
   */
  pins: z.strictObject({ repository: nonEmpty, commit: z.string().regex(/^[0-9a-f]{40}$/u), version: nonEmpty, runtime: nonEmpty }),
  /** The runtimes' committed pin digests, as recording.json records them, for the same reason. */
  runtimes: z.record(z.string().regex(/^[a-z0-9._-]+$/u), z.record(z.string().regex(/^[A-Za-z0-9._-]+$/u), digestSchema)),
  /** Both recordings, by their file name. */
  files: z.strictObject({ [RECORDING_FILE]: lockedFileSchema, [CORPUS_FILE]: lockedFileSchema }),
});

export type RecordingLock = z.infer<typeof recordingLockSchema>;

/**
 * The assets recorded on a maintainer's machine and uploaded by hand before port-readiness-record.yml could run from
 * main, the only ref whose recordings it attests: the only ones fetch.ts takes without its attestation. Each is the
 * recording a reviewed commit's lock names. The list only shrinks: an entry goes once its tool-pack's lock names an
 * attested recording, nothing joins it once the record workflow runs from main, and a test fails while an entry is
 * named by no committed lock.
 */
export const LIFTED_ASSETS: ReadonlySet<string> = new Set([
  "javascript-eslint-corpus-4e955069b420e258c368e99c0cba326b987ade51c0232c19fca8cbef2bb2cee4.json.gz",
  "javascript-eslint-plugin-unicorn-corpus-58f3d9e9b7a813b10c921c110ead51ccb2443e8caa08b18b1d2f62c05d9b5267.json.gz",
  "javascript-eslint-plugin-unicorn-recording-030ae504d4d81216fadb2759ee942ec5a9604f4de75a92621fbea8375337228a.json.gz",
  "javascript-eslint-recording-e91788d9a22310304f9f913971a5ca224b981a825a965c05cf75865207c544a4.json.gz",
  "next-eslint-plugin-next-corpus-220df27c30349a333b333cc17bf70af7213f44f063c570406ac53ad330bd0f0b.json.gz",
  "next-eslint-plugin-next-recording-824360574a54f5a3e9b42c76b507ee87522169e9502d88ebdaadb5802483fced.json.gz",
  "python-bandit-corpus-ead66a1ebf52f6daa1a94556eea9e4443f675f275ae79fe6ab7af9f152b1d949.json.gz",
  "python-bandit-recording-2baa8fc4fc3fdcde02845a367113b58861d05ea897a6edb8ad8e8acb455fa802.json.gz",
  "python-ruff-corpus-941b451583ab220d51a50cd883fa9c47e80e606bc62bdcbde0332ce2438fb5f5.json.gz",
  "python-ruff-recording-34a2fac3e70ef1fb025ccc1beaedcc6f9207a5128d5030caf7a1a39e90e565a1.json.gz",
  "react-eslint-plugin-react-corpus-1c9eaeebcd65f8b67b37f3859e6827a8d7942702f0491e5531d23e8e64f8eb91.json.gz",
  "react-eslint-plugin-react-hooks-corpus-b85afe644972c47e475a26ff186273468bb7317e7211863d6f0c8a2626222e1c.json.gz",
  "react-eslint-plugin-react-hooks-recording-11e50fbaf44ab74565931616b6c90fa69db7b8303d11f6a87b251dcc3b88710e.json.gz",
  "react-eslint-plugin-react-recording-e171b74355708619ec757af1570fb5db03db8bf8675d34f3fb9b7c5e83d25d6e.json.gz",
  "typescript-eslint-plugin-vitest-corpus-2062b527fbdb0816a05de3ddd8cb88f5dc8bea720acdf757a1c9011d693d0084.json.gz",
  "typescript-eslint-plugin-vitest-recording-2cf5e774987e23a649aa5d6b8375dec162f293da8396cb53553ba7ec4ac3318f.json.gz",
  "typescript-typescript-eslint-corpus-a722b4540559768f80e7d7502952e74b5aa74a909126c3d7b3a305b15cd1d4a5.json.gz",
  "typescript-typescript-eslint-recording-aece5f8587bf60f4e561124a0746b5f0a4673b5406776e084b7af5e9c1ad88df.json.gz",
]);

/**
 * The case sources of each hand-uploaded `recording.json` (LIFTED_ASSETS) made before a case carried the sha256 of its
 * committed source (`cases[].sha256`), pinned together: the gates' `case-sources` check has nothing per case to check
 * them against, and passing them unchecked would let an edited fixture be judged against upstream's unchanged answers.
 * Each value is caseSourcesDigest over every file the recording's cases name, taken from the tree the recording was
 * made from. Keyed by asset name, which readRecordingLock ties to the recording's sha256 and tool-pack, so an entry
 * covers that one recording and no other. A recording whose cases carry their own sha256 needs no entry. The list only
 * shrinks: an entry goes with the asset's entry in LIFTED_ASSETS, when the tool-pack gets an attested recording.
 */
// Empty: every hand-uploaded recording now carries a sha256 per case.
export const LIFTED_CASE_SOURCES: ReadonlyMap<string, string> = new Map();

/**
 * One digest over a set of case sources: sha256 of `<sha256 of the file's bytes>  <file>\n` per file (sha256sum's
 * lines), the files in code-unit order. Any file added, dropped, renamed or edited changes it.
 */
export function caseSourcesDigest(files: Iterable<readonly [file: string, digest: string]>): string {
  const lines = [...files].sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0)).map(([file, digest]) => `${digest}  ${file}\n`);
  return sha256(lines.join(""));
}

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
  if (!existsSync(path)) throw new Error(`packs/${pack}/upstream/${tool}/${RECORDING_LOCK} is missing: run npm run port-readiness:record -- --pack ${pack} --tool ${tool}`);
  const parsed = recordingLockSchema.safeParse(JSON.parse(readFileSync(path, "utf8")));
  if (!parsed.success) throw new Error(`packs/${pack}/upstream/${tool}/${RECORDING_LOCK}: ${parsed.error.issues.slice(0, 5).map((issue) => `/${issue.path.map(String).join("/")}: ${issue.message}`).join("; ")}`);
  for (const [file, locked] of Object.entries(parsed.data.files)) {
    const expected = assetName(pack, tool, file, locked.sha256);
    if (locked.asset !== expected) throw new Error(`packs/${pack}/upstream/${tool}/${RECORDING_LOCK}: /files/${file}/asset is ${locked.asset}, and this tool-pack's recording of sha256 ${locked.sha256.slice(0, 12)} is ${expected}`);
  }
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
  texts: { [RECORDING_FILE]: string; [CORPUS_FILE]: string },
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
      [RECORDING_FILE]: locked(RECORDING_FILE, texts[RECORDING_FILE], RECORDING_FORMAT),
      [CORPUS_FILE]: locked(CORPUS_FILE, texts[CORPUS_FILE], CORPUS_FORMAT),
    },
  });
}
