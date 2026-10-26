import { existsSync } from "node:fs";
import { mkdir, readFile, rm } from "node:fs/promises";
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
    // commit. Widened, never narrowed, so tool-packs that share a corpus keep each other's files.
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

/** One corpus a rerail pack pins in its conformance manifest: a repository at a commit, the paths of it to read. */
export interface RerailCorpus {
  name: string;
  repository: string;
  commit: string;
  paths: string[];
  exclude: string[];
}

/** The conformance manifests of a pack that pin corpora: the pack's own, and its candidates'. */
export function conformancePaths(pack: string): string[] {
  return [`packs/${pack}/conformance.json`, `packs/${pack}/candidates/conformance.json`];
}

/**
 * Every corpus a pack pins, by name: the `corpora` of its conformance manifests under `root`. A name pinned twice must
 * name the same repository, commit and paths.
 */
export async function readRerailCorpora(root: string, pack: string): Promise<RerailCorpus[]> {
  const byName = new Map<string, RerailCorpus>();
  for (const path of conformancePaths(pack)) {
    const text = await readFile(join(root, path), "utf8").catch(() => null);
    if (text === null) continue;
    const corpora = (JSON.parse(text) as { corpora?: unknown }).corpora;
    if (!Array.isArray(corpora)) continue;
    for (const entry of corpora as Partial<RerailCorpus>[]) {
      const valid = typeof entry.name === "string" && /^[a-z0-9][a-z0-9._-]*$/u.test(entry.name)
        && typeof entry.repository === "string" && /^https:\/\/github\.com\/[A-Za-z0-9._-]+\/[A-Za-z0-9._-]+$/u.test(entry.repository)
        && typeof entry.commit === "string" && /^[0-9a-f]{40}$/u.test(entry.commit)
        && Array.isArray(entry.paths) && entry.paths.length > 0 && entry.paths.every((item) => typeof item === "string" && item !== "" && !item.split("/").includes(".."))
        && (entry.exclude === undefined || (Array.isArray(entry.exclude) && entry.exclude.every((item) => typeof item === "string")));
      if (!valid) throw new Error(`${path} pins a corpus the gates cannot read: ${JSON.stringify(entry)}`);
      const corpus: RerailCorpus = { name: entry.name!, repository: entry.repository!, commit: entry.commit!, paths: [...entry.paths!].sort(), exclude: [...(entry.exclude ?? [])].sort() };
      const pinned = byName.get(corpus.name);
      if (pinned !== undefined && JSON.stringify(pinned) !== JSON.stringify(corpus)) throw new Error(`pack ${pack} pins two different corpora named ${corpus.name}`);
      byName.set(corpus.name, corpus);
    }
  }
  return [...byName.values()].sort((a, b) => (a.name < b.name ? -1 : a.name > b.name ? 1 : 0));
}
