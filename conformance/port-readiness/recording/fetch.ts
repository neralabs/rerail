/**
 * Downloading the recordings a commit's `recording.lock`s name, into the cache they are read from
 * (store.ts). `npm run port-readiness -- fetch --pack <id> --tool <tool>` runs it, once per gate shard, so a shard
 * downloads its own tool-pack and nothing else, and a job that runs no gate downloads nothing at all.
 *
 * It goes through the releases API rather than a `releases/download/` URL because the same call works before and
 * after the repository is public: with the job's token while it is private, and with no token at all once it is not.
 * Nothing here is trusted on its word — the asset is checked against the lock's sha256 and must carry the record
 * workflow's attestation before it is cached, and an asset the release does not have is a failure, never an empty
 * recording. That one failure is a MissingRecordingError, which the fetch command exits 3 on: it is the one a gate leg
 * answers by recording the tool-pack itself (port-readiness.yml), where any other fails the leg. Checking the
 * attestation runs `gh attestation verify`, which needs a signed-in GitHub CLI or a token whatever the repository's
 * visibility.
 */
import { existsSync } from "node:fs";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { ProcessCommandRunner, runChecked } from "../process.js";
import { CORPUS_FILE, RECORDING_FILE } from "./schema.js";
import { LIFTED_ASSETS, readCachedRecording, readRecordingLock, recordingCachePath, recordingFromAsset, writeCachedRecording, type LockedFile, type RecordingLock } from "./store.js";

/** Kept here too, where the fetch takes them without an attestation; they are defined beside the lock in store.ts. */
export { LIFTED_ASSETS };

/** Where the recordings are published: this repository, unless a fork's CI says otherwise. */
export const DEFAULT_REPOSITORY = "neralabs/rerail";

/**
 * The one workflow that records, and the ref it must have run from: an asset is taken only with its attestation.
 * A release accepts an upload from anyone who can write to the repository, and from any workflow with a write token,
 * so an asset's digest says only that it is the bytes the lock names, not that the recorder made them.
 */
export const RECORD_WORKFLOW = ".github/workflows/port-readiness-record.yml";
export const RECORD_REF = "refs/heads/main";

export interface FetchOptions {
  /** `<owner>/<repo>` holding the release; GITHUB_REPOSITORY in CI. */
  repository?: string;
  /**
   * A token: the download needs one while the repository is private, and `gh attestation verify` needs one (or a
   * signed-in GitHub CLI) always. The job's GITHUB_TOKEN is enough (contents: read, attestations: read).
   */
  token?: string;
  /** Injected in tests: the asset's gzipped bytes, by asset name. */
  download?: (asset: LockedFile, lock: RecordingLock) => Promise<Buffer>;
  /** Injected in tests: fail unless RECORD_WORKFLOW, run from RECORD_REF, attested this recording's text. */
  verify?: (text: string, asset: LockedFile) => Promise<void>;
  /** Injected in tests: the assets taken without an attestation, LIFTED_ASSETS unless given. */
  lifted?: ReadonlySet<string>;
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

/**
 * The lock names a recording the release does not hold: the release exists and has no asset of that name, because the
 * recording was never uploaded. Not a release the lookup cannot find (a missing tag, a repository the token cannot read,
 * a wrong repository: GitHub answers 404 to all three), nor a download, API or attestation failure, which are thrown
 * as plain errors.
 */
export class MissingRecordingError extends Error {
  constructor(release: string, repository: string, asset: string) {
    super(`the release ${release} of ${repository} has no asset ${asset}: the recording this commit names was never uploaded (a gate leg records it from this commit instead; port-readiness-record.yml uploads it, so later runs fetch it)`);
    this.name = "MissingRecordingError";
  }
}

/** GET a GitHub API url as JSON; any answer outside 2xx, 404 included, is an error. */
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
  const url = `https://api.github.com/repos/${repository}/releases/tags/${encodeURIComponent(lock.release)}`;
  let release: { assets?: ReleaseAsset[] };
  try {
    release = await githubJson<{ assets?: ReleaseAsset[] }>(url, options.token);
  } catch (error) {
    // A 404 here is not a missing recording: the release itself is not found, or the token cannot read the repository.
    throw new Error(`the release ${lock.release} of ${repository} cannot be read (a missing tag, a repository this token cannot read, or the wrong repository): ${error instanceof Error ? error.message : String(error)}`);
  }
  const found = (release.assets ?? []).find((entry) => entry.name === asset.asset);
  if (found === undefined) throw new MissingRecordingError(lock.release, repository, asset.asset);
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
 * Fail unless port-readiness-record.yml, run from main, attested this recording. The subject is the recording's text,
 * whose sha256 the lock and the asset's name carry, so an asset uploaded before the attestation was made is covered
 * by it too. `gh attestation verify` checks the Sigstore signature, the signer workflow and the ref it ran from.
 */
async function verifyAttestation(text: string, asset: LockedFile, options: FetchOptions): Promise<void> {
  const repository = options.repository ?? DEFAULT_REPOSITORY;
  const directory = await mkdtemp(join(tmpdir(), "gates-attestation-"));
  try {
    const path = join(directory, `${asset.sha256}.json`);
    await writeFile(path, text);
    const env = { ...process.env, ...(options.token === undefined || options.token === "" ? {} : { GH_TOKEN: options.token }) };
    await runChecked(new ProcessCommandRunner(), [
      "gh", "attestation", "verify", path, "--repo", repository,
      "--signer-workflow", `${repository}/${RECORD_WORKFLOW}`, "--source-ref", RECORD_REF, "--deny-self-hosted-runners",
    ], { env });
  } catch (error) {
    throw new Error(`the release's ${asset.asset} is not verified as recorded by ${RECORD_WORKFLOW} on ${RECORD_REF}, and no other asset is taken (not one uploaded by hand or by another workflow; verifying needs the GitHub CLI, signed in or given GITHUB_TOKEN): ${error instanceof Error ? error.message : String(error)}`);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
}

/**
 * Both of a tool-pack's recordings in the cache, downloaded only where they are not there already. A recording is
 * cached by its own digest, so a second run on the same commit — a re-run, another shard on a warm cache — reads it
 * and downloads nothing.
 */
export async function fetchToolPackRecordings(root: string, cacheRoot: string, pack: string, tool: string, options: FetchOptions = {}): Promise<FetchedRecording[]> {
  const lock = readRecordingLock(root, pack, tool);
  const download = options.download ?? ((asset: LockedFile, forLock: RecordingLock) => downloadAsset(asset, forLock, options));
  const verify = options.verify ?? ((text: string, asset: LockedFile) => verifyAttestation(text, asset, options));
  const lifted = options.lifted ?? LIFTED_ASSETS;
  const fetched: FetchedRecording[] = [];
  for (const file of [RECORDING_FILE, CORPUS_FILE] as const) {
    const locked = lock.files[file];
    if (readCachedRecording(cacheRoot, locked.sha256) !== undefined) {
      fetched.push({ file, asset: locked.asset, size: locked.size, cached: true });
      continue;
    }
    const label = `packs/${pack}/upstream/${tool}/${file}`;
    const text = recordingFromAsset(await download(locked, lock), locked, label);
    if (!lifted.has(locked.asset)) await verify(text, locked);
    await writeCachedRecording(cacheRoot, text);
    fetched.push({ file, asset: locked.asset, size: locked.size, cached: false });
  }
  return fetched;
}

/** True when both of a tool-pack's recordings are already cached: the gates can run offline. */
export function recordingsCached(root: string, cacheRoot: string, pack: string, tool: string): boolean {
  const lock = readRecordingLock(root, pack, tool);
  return [RECORDING_FILE, CORPUS_FILE].every((file) => existsSync(recordingCachePath(cacheRoot, lock.files[file as typeof RECORDING_FILE].sha256)));
}
