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
  "javascript-eslint-gates-6b2c8c972842da71225cd69fc6db0ad98c23b35d1e2cbd67dff34d768d7c5e57.json.gz",
  "javascript-eslint-gates-corpus-4a0005ebc7096ebdd43bfce65187a919ad7bb40dc98f453fc4b51245123061aa.json.gz",
  "javascript-eslint-plugin-unicorn-gates-corpus-2c1af5f587d2bc7d34e6cb1b89e0d5e5267b2a84445acba29172f7fa0869efd0.json.gz",
  "javascript-eslint-plugin-unicorn-gates-eca17ffaf1c7c589938f30129cd667617c29dc4cf66193f2a93fdf1e7fc18217.json.gz",
  "next-eslint-plugin-next-gates-57b494f4aa66a021048bacf610abe2d2702e3f71ff4f24d47d920615bda28a10.json.gz",
  "next-eslint-plugin-next-gates-corpus-43f24383610a797418dae21ee0d1c64483d274ee19754af30544284a4a781447.json.gz",
  "python-bandit-gates-corpus-6bf91179fe4e9347b56b3d94ce91ab1ff7888f52baf4df316eb257bbe3199f3d.json.gz",
  "python-bandit-gates-ec6b29fce85a03c292dabbfbe9438e76078a082af3409c1d7a0faa7aa8efecaa.json.gz",
  "python-ruff-gates-8f6038d965cf0cf26193f61e8b69963a7bfb35c480ed0ce02cd2bf30c04c2746.json.gz",
  "python-ruff-gates-corpus-c4cfcbb4345494ca185899b903d8b8ee282bac98698bcd92bb04d6adbe29bcab.json.gz",
  "react-eslint-plugin-react-gates-17f4eeabca71c32a612062f80f60dd9d3126360d6944c3cacb788665830eef55.json.gz",
  "react-eslint-plugin-react-gates-corpus-6257ec57d71616eab398281ae57bd7d47bca946c29fb603ee607809fc87cfcbc.json.gz",
  "react-eslint-plugin-react-hooks-gates-corpus-f98cafeb13f98fd8918469c8acfbcff07fc52fdc01f4d394b4dfb92199acad8c.json.gz",
  "react-eslint-plugin-react-hooks-gates-ec9af105936015a25c2d67b97126d3c142ca2066d1e0a1167e3f56e125255a2f.json.gz",
  "typescript-eslint-plugin-vitest-gates-corpus-9909aa03f587bbeaed1e34ad3ec57293b6733161c0e3778eb9265acee45f67c4.json.gz",
  "typescript-eslint-plugin-vitest-gates-e2f09bcaebc4d0e7518a0c03ed2d75b802ea4af9cdeaaab0575ea543a207a4f9.json.gz",
  "typescript-typescript-eslint-gates-302d0de625111bb16d158a2198c53119ab47229ecb744b7e8d3b9cab443de309.json.gz",
  "typescript-typescript-eslint-gates-corpus-c7ee7614b5b99b8d92789b88d89e307654bddb18471c1025234e2fe3bec00c7b.json.gz",
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
export const LIFTED_CASE_SOURCES: ReadonlyMap<string, string> = new Map([
  ["javascript-eslint-gates-6b2c8c972842da71225cd69fc6db0ad98c23b35d1e2cbd67dff34d768d7c5e57.json.gz", "55ed6950b41dda8a69dc39b1f0e2db6ccc7bc844c8fc8b8115ece451ab4aad01"],
  ["javascript-eslint-plugin-unicorn-gates-eca17ffaf1c7c589938f30129cd667617c29dc4cf66193f2a93fdf1e7fc18217.json.gz", "2cf9555df128696126f4de3e11379ec20ed154f9264df91f37b0321901409d58"],
  ["next-eslint-plugin-next-gates-57b494f4aa66a021048bacf610abe2d2702e3f71ff4f24d47d920615bda28a10.json.gz", "c05906a98e7288249ba7010d76e4e08d0e104d66463735a3694e9e649ff026ff"],
  ["python-bandit-gates-ec6b29fce85a03c292dabbfbe9438e76078a082af3409c1d7a0faa7aa8efecaa.json.gz", "1919d65b918095429612693c3e7ce43e258124b4db8b7d4be28ab8ef73c3f3e9"],
  ["python-ruff-gates-8f6038d965cf0cf26193f61e8b69963a7bfb35c480ed0ce02cd2bf30c04c2746.json.gz", "04b0f9cb79fb696d4df4d182ccf12be69df22d0dc2d37e283692649f8f77f056"],
  ["react-eslint-plugin-react-gates-17f4eeabca71c32a612062f80f60dd9d3126360d6944c3cacb788665830eef55.json.gz", "6f883431014f89bede712e3ad20d5267fb82a5250f819cba393c162c2e35b581"],
  ["react-eslint-plugin-react-hooks-gates-ec9af105936015a25c2d67b97126d3c142ca2066d1e0a1167e3f56e125255a2f.json.gz", "88ca837f731ebd9e289c2f01c224fa69eac13707a77d279e29c4a68393e27ed6"],
  ["typescript-eslint-plugin-vitest-gates-e2f09bcaebc4d0e7518a0c03ed2d75b802ea4af9cdeaaab0575ea543a207a4f9.json.gz", "f52c697f90ff40d28936f94654448848876a41704c7d66c3ee3cd9a9f82a4e41"],
  ["typescript-typescript-eslint-gates-302d0de625111bb16d158a2198c53119ab47229ecb744b7e8d3b9cab443de309.json.gz", "4ac07a2a450f23eba43f9ad3b6bb55e89e19b4aafd56e700d4b22f4a86aa5dfd"],
]);

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
