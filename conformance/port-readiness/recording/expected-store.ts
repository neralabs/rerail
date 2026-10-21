/**
 * Where the generated answers are kept between machines: Azure Blob Storage, through the `az` CLI (no SDK), one blob
 * per generator key (generator-key.ts), in the format recording/expected.ts encodes. Two containers, so that what a
 * pull request may write is apart from what main reads:
 *
 * - `port-readiness-expected`: `<key>/expected.json.gz`, written only by main's identity, and `latest/<pack>/<tool>.json`
 *   (`{key, commit}`), the answers main last promoted for each tool-pack, which a pull request's answers are compared
 *   with;
 * - `port-readiness-expected-pr`: `pr-<n>/<key>/expected.json.gz`, written by the pull request identity.
 *
 * Every blob carries metadata `sha256` (of the JSON inside the gzip), `commit` and `run`; a download whose JSON does
 * not have its `sha256`, or that holds another key's answers, is rejected. A blob that is not there is null; any other
 * failure (no login, no network, no permission) throws, so a store that cannot be read is never taken for one that
 * lacks the answers.
 *
 * The store is reached with `--connection-string` when AZURE_STORAGE_CONNECTION_STRING is set (Azurite, in tests and a
 * local end-to-end run), else as the signed-in identity (`az login`, or azure/login in CI) on the account
 * RERAIL_EXPECTED_ACCOUNT names. With neither there is no store: the generator then only generates.
 */
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

import type { CommandRunner } from "../process.js";
import { decodeExpected, EXPECTED_BLOB, type EncodedExpected } from "./expected.js";

export const MAIN_CONTAINER = "port-readiness-expected";
export const PR_CONTAINER = "port-readiness-expected-pr";

/** What a blob of answers records about where it was made: the commit, and the GitHub run (its URL) or `local`. */
export interface Provenance {
  commit: string;
  run: string;
}

/** The answers main last promoted for a tool-pack. */
export interface Latest {
  key: string;
  commit: string;
}

export interface ExpectedStore {
  runner: CommandRunner;
  /** The arguments that reach the account. */
  auth: string[];
  /** Values an error message must not repeat. */
  secrets: string[];
}

/** The store the environment reaches, or why there is none. */
export function expectedStore(runner: CommandRunner, env: NodeJS.ProcessEnv = process.env): ExpectedStore | { unavailable: string } {
  const connection = env.AZURE_STORAGE_CONNECTION_STRING ?? "";
  if (connection !== "") return { runner, auth: ["--connection-string", connection], secrets: [connection] };
  const account = env.RERAIL_EXPECTED_ACCOUNT ?? "";
  if (account !== "") return { runner, auth: ["--auth-mode", "login", "--account-name", account], secrets: [] };
  return { unavailable: "no answer store is configured (set RERAIL_EXPECTED_ACCOUNT and sign in with az login, or AZURE_STORAGE_CONNECTION_STRING)" };
}

/** A key's answers in main's container. */
export function mainBlobName(key: string): string {
  return `${key}/${EXPECTED_BLOB}`;
}

/** A key's answers in the pull request container, under the pull request that generated them. */
export function prBlobName(pr: number, key: string): string {
  return `pr-${pr}/${key}/${EXPECTED_BLOB}`;
}

/** The pointer to the answers main last promoted for a tool-pack, in main's container. */
export function latestBlobName(pack: string, tool: string): string {
  return `latest/${pack}/${tool}.json`;
}

/** Run `az storage blob <command>` against the store and parse its JSON output; a failure throws, its secrets masked. */
async function az(store: ExpectedStore, command: readonly string[]): Promise<unknown> {
  const argv = ["az", "storage", "blob", ...command, ...store.auth, "--only-show-errors", "--output", "json"];
  const result = await store.runner.run(argv);
  if (result.exitCode !== 0) {
    let message = `${argv.slice(0, 4).join(" ")} ${command.slice(1).join(" ")} exited ${result.exitCode}: ${(result.stderr.trim() || result.stdout.trim()).split("\n").slice(-10).join("\n")}`;
    for (const secret of store.secrets) message = message.replaceAll(secret, "***");
    throw new Error(message);
  }
  return result.stdout.trim() === "" ? null : JSON.parse(result.stdout) as unknown;
}

/** Whether the container holds the blob; a store that cannot say throws. */
export async function blobExists(store: ExpectedStore, container: string, name: string): Promise<boolean> {
  const answer = await az(store, ["exists", "--container-name", container, "--name", name]) as { exists?: unknown } | null;
  if (typeof answer?.exists !== "boolean") throw new Error(`az storage blob exists gave no answer for ${container}/${name}`);
  return answer.exists;
}

/** A blob's bytes and metadata, or null when the container does not hold it. */
export async function downloadBlob(store: ExpectedStore, container: string, name: string): Promise<{ bytes: Buffer; metadata: Record<string, string> } | null> {
  if (!await blobExists(store, container, name)) return null;
  const directory = await mkdtemp(join(tmpdir(), "rerail-expected-"));
  try {
    const file = join(directory, "blob");
    await az(store, ["download", "--container-name", container, "--name", name, "--file", file, "--no-progress"]);
    const metadata = await az(store, ["metadata", "show", "--container-name", container, "--name", name]);
    return { bytes: await readFile(file), metadata: Object.fromEntries(Object.entries(metadata ?? {}).filter((entry): entry is [string, string] => typeof entry[1] === "string")) };
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
}

/** Write a blob, replacing any there, with its metadata. */
export async function uploadBlob(store: ExpectedStore, container: string, name: string, bytes: Buffer, contentType: string, metadata: Record<string, string>): Promise<void> {
  const directory = await mkdtemp(join(tmpdir(), "rerail-expected-"));
  try {
    const file = join(directory, "blob");
    await writeFile(file, bytes);
    await az(store, [
      "upload", "--container-name", container, "--name", name, "--file", file, "--overwrite", "true", "--content-type", contentType, "--no-progress",
      ...(Object.keys(metadata).length === 0 ? [] : ["--metadata", ...Object.entries(metadata).map(([field, value]) => `${field}=${value}`)]),
    ]);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
}

/** A key's answers in a container, checked against the sha256 its metadata carries and the key; null when absent. */
export async function downloadExpected(store: ExpectedStore, container: string, name: string, key: string): Promise<{ files: Map<string, string>; encoded: EncodedExpected; provenance: Provenance } | null> {
  const blob = await downloadBlob(store, container, name);
  if (blob === null) return null;
  const { sha256, commit, run } = blob.metadata;
  if (sha256 === undefined) throw new Error(`${container}/${name} carries no sha256 metadata`);
  return { files: decodeExpected(blob.bytes, key, sha256), encoded: { bytes: blob.bytes, sha256 }, provenance: { commit: commit ?? "", run: run ?? "" } };
}

/** Store a key's answers under `name` in a container, with their sha256 and where they were made. */
export async function uploadExpected(store: ExpectedStore, container: string, name: string, encoded: EncodedExpected, provenance: Provenance): Promise<void> {
  await uploadBlob(store, container, name, encoded.bytes, "application/gzip", { sha256: encoded.sha256, commit: provenance.commit, run: provenance.run });
}

/**
 * Copy a pull request's answers for a key into main's container: downloaded, checked (sha256, key) and uploaded with
 * the metadata of where they were made, never a server-side copy, so main's identity needs only to read the pull
 * request container. False when the pull request container does not hold them.
 */
export async function promoteExpected(store: ExpectedStore, pr: number, key: string): Promise<boolean> {
  const found = await downloadExpected(store, PR_CONTAINER, prBlobName(pr, key), key);
  if (found === null) return false;
  await uploadExpected(store, MAIN_CONTAINER, mainBlobName(key), found.encoded, found.provenance);
  return true;
}

/** The answers main last promoted for a tool-pack, or null when it has promoted none. */
export async function readLatest(store: ExpectedStore, pack: string, tool: string): Promise<Latest | null> {
  const blob = await downloadBlob(store, MAIN_CONTAINER, latestBlobName(pack, tool));
  if (blob === null) return null;
  const value = JSON.parse(blob.bytes.toString("utf8")) as Partial<Latest> | null;
  if (typeof value?.key !== "string" || typeof value.commit !== "string") throw new Error(`${MAIN_CONTAINER}/${latestBlobName(pack, tool)} is not {key, commit}`);
  return { key: value.key, commit: value.commit };
}

export async function writeLatest(store: ExpectedStore, pack: string, tool: string, latest: Latest): Promise<void> {
  await uploadBlob(store, MAIN_CONTAINER, latestBlobName(pack, tool), Buffer.from(`${JSON.stringify({ key: latest.key, commit: latest.commit })}\n`, "utf8"), "application/json", {});
}
