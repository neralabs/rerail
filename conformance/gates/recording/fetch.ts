/**
 * Downloading the recordings a commit's `recording.lock`s name, into the cache they are read from
 * (store.ts). `npm run gates -- fetch --pack <id> --tool <tool>` runs it, once per gate shard, so a shard
 * downloads its own tool-pack and nothing else, and a job that runs no gate downloads nothing at all.
 *
 * It goes through the releases API rather than a `releases/download/` URL because the same call works before and
 * after the repository is public: with the job's token while it is private, and with no token at all once it is not.
 * Nothing here is trusted on its word — the asset is checked against the lock's sha256 and must carry the record
 * workflow's attestation before it is cached, and an asset the release does not have is a failure, never an empty
 * recording. Checking the attestation runs `gh attestation verify`, which needs a signed-in GitHub CLI or a token
 * whatever the repository's visibility.
 */
import { existsSync } from "node:fs";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { ProcessCommandRunner, runChecked } from "../process.js";
import { GATES_CORPUS, GATES_RECORDING } from "./schema.js";
import { readCachedRecording, readRecordingLock, recordingCachePath, recordingFromAsset, writeCachedRecording, type LockedFile, type RecordingLock } from "./store.js";

/** Where the recordings are published: this repository, unless a fork's CI says otherwise. */
export const DEFAULT_REPOSITORY = "neralabs/rerail";

/**
 * The one workflow that records, and the ref it must have run from: an asset is taken only with its attestation.
 * A release accepts an upload from anyone who can write to the repository, and from any workflow with a write token,
 * so an asset's digest says only that it is the bytes the lock names, not that the recorder made them.
 */
export const RECORD_WORKFLOW = ".github/workflows/rule-gates-record.yml";
export const RECORD_REF = "refs/heads/main";

/**
 * The assets uploaded by hand when the recordings were lifted out of the tree (migrate-v2.ts), before
 * rule-gates-record.yml could run from main to attest anything: the only ones taken without its attestation. Each
 * holds the bytes the reviewed in-tree recording held. The list only shrinks: an entry goes once its tool-pack's lock
 * names another recording, and a test fails while an entry is named by no committed lock.
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
 * Fail unless rule-gates-record.yml, run from main, attested this recording. The subject is the recording's text,
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
  for (const file of [GATES_RECORDING, GATES_CORPUS] as const) {
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
  return [GATES_RECORDING, GATES_CORPUS].every((file) => existsSync(recordingCachePath(cacheRoot, lock.files[file as typeof GATES_RECORDING].sha256)));
}
