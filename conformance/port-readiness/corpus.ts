/**
 * Pinned upstream checkouts and the gates' cache root. The file keeps the name of the corpus tier it once served
 * (replaced by the real-code samples) because recorded-adapter.ts imports it, and that file is part of the bandit
 * recording's config id (ADAPTER_SOURCES): renaming the import would re-key the recordings.
 */
import { existsSync } from "node:fs";
import { mkdir, rm } from "node:fs/promises";
import { join } from "node:path";

import { PACKAGE_ROOT } from "../../src/engine/index.js";
import { runChecked, type CommandRunner } from "./process.js";

/** Where fetched checkouts, results and scratch files are cached unless a run names another directory (gitignored). */
export const DEFAULT_CACHE_ROOT = join(PACKAGE_ROOT, ".rerail", "port-readiness");

/** True when a cached checkout is exactly the commit with an unmodified working tree (ignored build outputs aside). */
async function isPristine(directory: string, commit: string, runner: CommandRunner): Promise<boolean> {
  if (!existsSync(join(directory, ".git"))) return false;
  const head = await runChecked(runner, ["git", "-C", directory, "rev-parse", "HEAD"]).catch(() => "");
  if (head.trim() !== commit) return false;
  const status = await runChecked(runner, ["git", "-C", directory, "status", "--porcelain"]).catch(() => "?");
  return status.trim() === "";
}

/**
 * A sparse checkout of `paths` of a repository at one exact commit, in `directory`. A cached checkout is reused only
 * while it is pristine; anything else is fetched again.
 */
export async function checkoutPinned(repository: string, commit: string, directory: string, paths: string[], runner: CommandRunner, options: { blobless?: boolean } = {}): Promise<string> {
  if (await isPristine(directory, commit, runner)) {
    // A checkout cached with fewer paths only needs its sparse set widened: the shallow fetch holds every file of the
    // commit. Widened, never narrowed, so tool-packs that share an upstream checkout keep each other's files.
    const sparse = (await runChecked(runner, ["git", "-C", directory, "sparse-checkout", "list"]).catch(() => "")).trim().split("\n");
    if (paths.every((path) => sparse.includes(path))) return directory;
    await runChecked(runner, ["git", "-C", directory, "sparse-checkout", "add", ...paths.filter((path) => !sparse.includes(path))]);
    if (await isPristine(directory, commit, runner)) return directory;
  }
  await rm(directory, { recursive: true, force: true });
  await mkdir(directory, { recursive: true });
  const git = (...args: string[]) => runChecked(runner, ["git", "-C", directory, ...args]);
  await git("init", "-q");
  await git("remote", "add", "origin", `${repository}.git`);
  await git("sparse-checkout", "set", "--no-cone", ...paths);
  // Blobless, the checkout fetches only the blobs of the sparse paths, and a widened sparse set fetches the rest on demand.
  await git("fetch", "-q", "--depth", "1", ...(options.blobless === true ? ["--filter=blob:none"] : []), "origin", commit);
  await git("checkout", "-q", "--force", "FETCH_HEAD");
  if (!(await isPristine(directory, commit, runner))) throw new Error(`checkout of ${repository} ${commit} is not clean after checkout`);
  return directory;
}

/** The conformance manifests of a pack: the pack's own, and its candidates'. */
export function conformancePaths(pack: string): string[] {
  return [`packs/${pack}/conformance.json`, `packs/${pack}/candidates/conformance.json`];
}
