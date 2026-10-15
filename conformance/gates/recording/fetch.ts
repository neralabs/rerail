/**
 * Downloading the recordings a commit's `recording.lock`s name, into the cache they are read from
 * (recording-store.ts). `npm run gates -- fetch --pack <id> --tool <tool>` runs it, once per gate shard, so a shard
 * downloads its own tool-pack and nothing else, and a job that runs no gate downloads nothing at all.
 *
 * It goes through the releases API rather than a `releases/download/` URL because the same call works before and
 * after the repository is public: with the job's token while it is private, and with no token at all once it is not.
 * Nothing here is trusted on its word — the asset is checked against the lock's sha256 before it is cached, and an
 * asset the release does not have is a failure, never an empty recording.
 */
import { existsSync } from "node:fs";

import { GATES_CORPUS, GATES_RECORDING } from "./schema.js";
import { readCachedRecording, readRecordingLock, recordingCachePath, recordingFromAsset, writeCachedRecording, type LockedFile, type RecordingLock } from "./store.js";

/** Where the recordings are published: this repository, unless a fork's CI says otherwise. */
export const DEFAULT_REPOSITORY = "neralabs/rerail";

export interface FetchOptions {
  /** `<owner>/<repo>` holding the release; GITHUB_REPOSITORY in CI. */
  repository?: string;
  /** A token for a repository that is not public yet; the job's GITHUB_TOKEN is enough (contents: read). */
  token?: string;
  /** Injected in tests: the asset's gzipped bytes, by asset name. */
  download?: (asset: LockedFile, lock: RecordingLock) => Promise<Buffer>;
}

/** What one fetch did, for the job log: a cache hit downloads nothing, which is what a second run on a commit does. */
export interface FetchedRecording {
  file: string;
  asset: string;
  size: number;
  cached: boolean;
}

interface ReleaseAsset {
  name: string;
  url: string;
  size: number;
}

async function githubJson<T>(url: string, token: string | undefined): Promise<T> {
  const response = await fetch(url, {
    headers: {
      accept: "application/vnd.github+json",
      "x-github-api-version": "2022-11-28",
      ...(token === undefined || token === "" ? {} : { authorization: `Bearer ${token}` }),
    },
  });
  if (!response.ok) throw new Error(`GET ${url} answered ${response.status} ${response.statusText}`);
  return (await response.json()) as T;
}

/**
 * One asset's bytes from the release the lock names. The asset is found by name in the release, then read through its
 * own API url with `accept: application/octet-stream`, which is the one form that works whether or not the repository
 * is public.
 */
async function downloadAsset(asset: LockedFile, lock: RecordingLock, options: FetchOptions): Promise<Buffer> {
  const repository = options.repository ?? DEFAULT_REPOSITORY;
  const release = await githubJson<{ assets?: ReleaseAsset[] }>(`https://api.github.com/repos/${repository}/releases/tags/${encodeURIComponent(lock.release)}`, options.token);
  const found = (release.assets ?? []).find((entry) => entry.name === asset.asset);
  if (found === undefined) {
    throw new Error(`the release ${lock.release} of ${repository} has no asset ${asset.asset}: the recording this commit names was never uploaded (a pull request cannot supply its own; see rule-gates-record.yml)`);
  }
  const response = await fetch(found.url, {
    headers: {
      accept: "application/octet-stream",
      ...(options.token === undefined || options.token === "" ? {} : { authorization: `Bearer ${options.token}` }),
    },
  });
  if (!response.ok) throw new Error(`GET ${found.url} answered ${response.status} ${response.statusText}`);
  return Buffer.from(await response.arrayBuffer());
}

/**
 * Both of a tool-pack's recordings in the cache, downloaded only where they are not there already. A recording is
 * cached by its own digest, so a second run on the same commit — a re-run, another shard on a warm cache — reads it
 * and downloads nothing.
 */
export async function fetchToolPackRecordings(root: string, cacheRoot: string, pack: string, tool: string, options: FetchOptions = {}): Promise<FetchedRecording[]> {
  const lock = readRecordingLock(root, pack, tool);
  const download = options.download ?? ((asset: LockedFile, forLock: RecordingLock) => downloadAsset(asset, forLock, options));
  const fetched: FetchedRecording[] = [];
  for (const file of [GATES_RECORDING, GATES_CORPUS] as const) {
    const locked = lock.files[file];
    if (readCachedRecording(cacheRoot, locked.sha256) !== undefined) {
      fetched.push({ file, asset: locked.asset, size: locked.size, cached: true });
      continue;
    }
    const label = `packs/${pack}/upstream/${tool}/${file}`;
    const text = recordingFromAsset(await download(locked, lock), locked, label);
    await writeCachedRecording(cacheRoot, text);
    fetched.push({ file, asset: locked.asset, size: locked.size, cached: false });
  }
  return fetched;
}

/** True when both of a tool-pack's recordings are already cached: the gates can run offline. */
export function recordingsCached(root: string, cacheRoot: string, pack: string, tool: string): boolean {
  const lock = readRecordingLock(root, pack, tool);
  return [GATES_RECORDING, GATES_CORPUS].every((file) => existsSync(recordingCachePath(cacheRoot, lock.files[file as typeof GATES_RECORDING].sha256)));
}
